// Gateway inference pipeline supporting 5 wire surfaces:
//   - OpenAI Chat Completions (/v1/chat/completions)
//   - Anthropic Messages (/v1/messages + /v1/messages/count_tokens)
//   - OpenAI Responses (/v1/responses + /v1/responses/input_tokens)
//   - Native Gemini (/v1beta/models/{model}:generateContent|:streamGenerateContent)
//   - Legacy OpenAI Completions (/v1/completions)
//
// Pipeline stages:
//   1. Strict API key auth + per-key RPM limit
//   2. Guardrails (prompt injection, PII masking / reversible PII vault, v3.8.55 agent-guard)
//   3. RTK + Caveman compression + Adaptive Context Compaction (omni-compress)
//   4. Dual-layer Semantic Cache lookup & store (omni-cache, key-isolated)
//   5. Combo/catalog resolution + 22-strategy routing + retry/backoff/lockout failover
//   6. Usage analytics (honoring x-omniroute-no-log) + Gamification XP/streak/badges

use crate::oauth_api::{ensure_fresh_oauth_credential, refresh_credential_now};
use crate::state::{hash_api_key, now_ms, SharedState, TargetMetric};
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::Response;
use futures_util::StreamExt;
use omni_cache::normalize_messages_for_cache;
use omni_catalog::{ProviderDescriptor, WireFormat};
use omni_compress::compress_chat_request;
use omni_exec::dispatch::{
    dispatch_once_detailed, AuthHeader, AuthScheme, ConnectionEndpoint, DispatchFailure,
};
use omni_exec::executors::{
    bedrock_converse_to_openai, prepare_provider_request, UpstreamResponseFormat,
};
use omni_gami::XpAction;
use omni_guard::{evaluate_agent_guard, process_request, PiiVault};
use omni_router::{
    normalize_auto_strategy, normalize_strategy, order_targets_with_auto, AutoRoutingStrategy,
    ComboTarget, Strategy,
};
use omni_store::{CredentialRow, UsageEntry};
use omni_translate::{
    claude_to_openai_request, collect_sse_chunks_to_openai, extract_usage_tokens,
    normalize_non_stream_to_openai, normalize_stream_chunk_to_openai,
    openai_chat_to_legacy_completion, openai_chunk_to_gemini_sse, openai_to_claude_response,
    openai_to_gemini_response, openai_to_responses_response, OpenAiToClaudeStreamState,
    OpenAiToResponsesStreamState,
};
use omni_types::errors::{
    build_anthropic_error_body, build_error_body, build_error_body_with_retry,
    cap_client_retry_after_secs,
};
use omni_types::sse::SseAssembler;
use omni_types::{ChatRequest, OmniError};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseSurface {
    OpenAiChat,
    AnthropicMessages,
    OpenAiResponses,
    GeminiNative,
    LegacyCompletion,
}

impl ResponseSurface {
    pub fn is_anthropic(self) -> bool {
        self == Self::AnthropicMessages
    }
}

pub fn json_response(status: StatusCode, body: Value) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

pub fn format_error_response(
    status: StatusCode,
    message: &str,
    is_anthropic: bool,
    retry_after_ms: Option<u64>,
) -> Response {
    let retry_secs = retry_after_ms.map(|ms| cap_client_retry_after_secs(ms as f64 / 1000.0));
    let body = if is_anthropic {
        build_anthropic_error_body(status.as_u16(), message)
    } else {
        serde_json::to_value(build_error_body_with_retry(
            status.as_u16(),
            message,
            retry_secs,
            None,
        ))
        .unwrap_or_else(|_| json!({"error": {"message": message}}))
    };
    let mut builder = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(secs) = retry_secs {
        if secs > 0.0 {
            builder = builder.header("Retry-After", format!("{}", secs.ceil() as u64));
        }
    }
    builder.body(Body::from(body.to_string())).unwrap()
}

fn failure_to_response(f: &DispatchFailure, is_anthropic: bool) -> Response {
    let status = StatusCode::from_u16(f.error.status()).unwrap_or(StatusCode::BAD_GATEWAY);
    let msg = match &f.error {
        OmniError::Upstream { body, .. } => body.clone().unwrap_or_default(),
        OmniError::Network(m) | OmniError::BadRequest(m) | OmniError::Translate(m) => m.clone(),
    };
    format_error_response(status, &msg, is_anthropic, f.retry_after_ms)
}

pub fn extract_api_key(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            headers
                .get("x-api-key")
                .or_else(|| headers.get("x-goog-api-key"))
                .and_then(|v| v.to_str().ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
}

pub fn authorize_request(
    state: &SharedState,
    headers: &HeaderMap,
    is_anthropic: bool,
) -> Result<Option<String>, Response> {
    // Allow authenticated dashboard sessions (signed HttpOnly cookie) when password lock is active
    if state.is_dashboard_locked() && crate::part_a::has_valid_dashboard_session(state, headers) {
        return Ok(extract_api_key(headers));
    }
    let key = extract_api_key(headers);
    let valid = state
        .store
        .validate_api_key(key.as_deref(), now_ms())
        .unwrap_or(false);
    if !valid {
        return Err(format_error_response(
            StatusCode::UNAUTHORIZED,
            "Invalid or missing API key",
            is_anthropic,
            None,
        ));
    }
    if let Some(ref k) = key {
        if let Ok(Some(row)) = state.store.get_api_key_row(k) {
            if let Some(rpm) = row.rpm_limit {
                if !state.check_rpm_limit(k, rpm) {
                    return Err(format_error_response(
                        StatusCode::TOO_MANY_REQUESTS,
                        "API key RPM rate limit exceeded",
                        is_anthropic,
                        Some(60_000),
                    ));
                }
            }
        }
    }
    Ok(key)
}

pub fn no_log_requested(headers: &HeaderMap) -> bool {
    headers
        .get("x-omniroute-no-log")
        .and_then(|v| v.to_str().ok())
        .map(|s| matches!(s.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

fn no_cache_requested(headers: &HeaderMap) -> bool {
    headers
        .get("x-omniroute-no-cache")
        .and_then(|v| v.to_str().ok())
        .map(|s| matches!(s.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

fn build_connection(
    state: &SharedState,
    provider: &ProviderDescriptor,
    model: &str,
    stream: bool,
    cred: Option<&CredentialRow>,
) -> Option<ConnectionEndpoint> {
    let auth_header = match cred
        .map(|c| c.auth_type.as_str())
        .filter(|s| {
            matches!(
                *s,
                "x-api-key" | "x-goog-api-key" | "api-key" | "authorization-key" | "none"
            )
        }) {
        Some(h) => AuthHeader::from_str_lossy(h),
        None => AuthHeader::from_str_lossy(provider.effective_auth_header()),
    };

    let key = cred
        .map(|c| c.api_key.clone())
        .or_else(|| provider.anonymous_api_key.clone())
        .unwrap_or_default();

    let raw_urls = match cred {
        Some(c) if !c.base_urls.is_empty() => c.base_urls.clone(),
        _ => provider.base_urls.clone(),
    };
    let resolved_urls: Vec<String> = raw_urls
        .iter()
        .map(|u| provider.resolve_chat_url(u, model, stream))
        .filter(|u| !u.is_empty())
        .collect();
    if resolved_urls.is_empty() {
        return None;
    }

    let mut extra_headers: Vec<(String, String)> = provider
        .headers
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    if provider.effective_wire_format(model) == WireFormat::Claude
        && !extra_headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("anthropic-version"))
    {
        extra_headers.push(("anthropic-version".into(), "2023-06-01".into()));
    }
    if provider.id == "pollinations"
        && !extra_headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("referer"))
    {
        extra_headers.push(("Referer".into(), "https://omniroute-rs.local/".into()));
    }

    let auth_scheme = if provider.auth_type == "oauth"
        || cred.map(|c| c.auth_type == "oauth").unwrap_or(false)
    {
        AuthScheme::OAuth
    } else if key.is_empty() {
        AuthScheme::Anonymous
    } else {
        AuthScheme::ApiKey
    };

    Some(ConnectionEndpoint {
        provider: provider.id.clone(),
        connection_id: cred
            .map(|c| c.connection_id.clone())
            .unwrap_or_else(|| "default".into()),
        model: model.to_string(),
        auth_type: auth_scheme,
        base_urls: resolved_urls,
        api_key: key,
        auth_header,
        auth_prefix: provider.auth_prefix.clone(),
        extra_headers,
        timeout_ms: provider.timeout_ms,
        raw_body_override: None,
        force_gzip: false,
        proxy_url: state.resolve_proxy_for(&provider.id, cred),
    })
}

pub fn expand_and_order_candidates(
    state: &SharedState,
    requested_model: &str,
    strategy_override: Option<Strategy>,
    prompt_tokens_est: i64,
) -> (Strategy, bool, Vec<ComboTarget>) {
    let resolved_name = state
        .store
        .resolve_model_alias(requested_model)
        .ok()
        .flatten()
        .unwrap_or_else(|| requested_model.to_string());

    let all_creds = state.store.list_credentials().unwrap_or_default();
    let metrics = state.target_metrics.read().unwrap().clone();
    let catalog = state.catalog.read().unwrap();

    if let Ok(Some(combo)) = state.store.get_combo(&resolved_name) {
        if combo.enabled && !combo.models.is_empty() {
            let strategy = strategy_override.unwrap_or_else(|| normalize_strategy(&combo.strategy));
            let auto_strat = normalize_auto_strategy(&combo.auto_strategy);
            let mut targets = Vec::new();
            for step in &combo.models {
                let prov_id = catalog
                    .find(&step.provider)
                    .map(|p| p.id.clone())
                    .unwrap_or_else(|| step.provider.clone());
                let ctx_len = catalog
                    .find(&prov_id)
                    .map(|p| p.context_length_for(&step.model) as i64)
                    .unwrap_or(128_000);
                let matching_creds: Vec<&CredentialRow> = all_creds
                    .iter()
                    .filter(|c| {
                        c.enabled
                            && c.provider == prov_id
                            && step
                                .connection_id
                                .as_deref()
                                .map(|cid| cid == c.connection_id)
                                .unwrap_or(true)
                    })
                    .collect();

                if matching_creds.is_empty() {
                    let exec_key = format!("{prov_id}::default::{}", step.model);
                    let m = metrics.get(&exec_key).cloned().unwrap_or(TargetMetric {
                        requests: 0,
                        success_rate: 100.0,
                        avg_latency_ms: 100.0,
                    });
                    targets.push(ComboTarget {
                        provider: prov_id.clone(),
                        connection_id: "default".into(),
                        model: step.model.clone(),
                        weight: step.weight,
                        fallback_only: step.fallback_only,
                        cost_per_m_out: 0.0,
                        quota_remaining_hint: 1_000_000,
                        reset_in_ms: 0,
                        request_count: m.requests,
                        success_rate: m.success_rate,
                        avg_latency_ms: m.avg_latency_ms,
                        headroom_bytes: 0,
                        context_len: ctx_len,
                        prompt_tokens: prompt_tokens_est,
                        breaker_open: omni_exec::resilience::is_provider_in_cooldown(&prov_id),
                    });
                } else {
                    for c in matching_creds {
                        let exec_key = format!("{prov_id}::{}::{}", c.connection_id, step.model);
                        let m = metrics.get(&exec_key).cloned().unwrap_or(TargetMetric {
                            requests: 0,
                            success_rate: 100.0,
                            avg_latency_ms: 100.0,
                        });
                        targets.push(ComboTarget {
                            provider: prov_id.clone(),
                            connection_id: c.connection_id.clone(),
                            model: step.model.clone(),
                            weight: step.weight * c.weight.max(0.1),
                            fallback_only: step.fallback_only,
                            cost_per_m_out: 0.0,
                            quota_remaining_hint: 1_000_000,
                            reset_in_ms: 0,
                            request_count: m.requests,
                            success_rate: m.success_rate,
                            avg_latency_ms: m.avg_latency_ms,
                            headroom_bytes: 0,
                            context_len: ctx_len,
                            prompt_tokens: prompt_tokens_est,
                            breaker_open: omni_exec::resilience::is_provider_in_cooldown(&prov_id)
                                || omni_exec::resilience::is_connection_in_cooldown(
                                    &prov_id,
                                    &c.connection_id,
                                ),
                        });
                    }
                }
            }
            let mut rng = rand::rngs::OsRng;
            let mut rr_slot = state.next_rr();
            let lkgp = state.get_last_good(&resolved_name);
            let ordered = order_targets_with_auto(
                strategy,
                auto_strat,
                &targets,
                &mut rng,
                Some(&mut rr_slot),
                lkgp.as_deref(),
            );
            return (strategy, true, ordered);
        }
    }

    let strategy = strategy_override.unwrap_or_else(|| state.default_strategy());
    let resolved = catalog.resolve_model(&resolved_name);
    let mut configured_targets = Vec::new();
    let mut unconfigured_targets = Vec::new();

    for rm in resolved {
        let p = rm.provider;
        let ctx_len = p.context_length_for(&rm.model) as i64;
        let p_creds: Vec<&CredentialRow> = all_creds
            .iter()
            .filter(|c| c.enabled && c.provider == p.id)
            .collect();

        if !p_creds.is_empty() {
            for c in p_creds {
                let exec_key = format!("{}::{}::{}", p.id, c.connection_id, rm.model);
                let m = metrics.get(&exec_key).cloned().unwrap_or(TargetMetric {
                    requests: 0,
                    success_rate: 100.0,
                    avg_latency_ms: 100.0,
                });
                configured_targets.push(ComboTarget {
                    provider: p.id.clone(),
                    connection_id: c.connection_id.clone(),
                    model: rm.model.clone(),
                    weight: c.weight.max(0.1),
                    fallback_only: false,
                    cost_per_m_out: if p.is_free_tier { 0.0 } else { 1.0 },
                    quota_remaining_hint: 1_000_000,
                    reset_in_ms: 0,
                    request_count: m.requests,
                    success_rate: m.success_rate,
                    avg_latency_ms: m.avg_latency_ms,
                    headroom_bytes: 0,
                    context_len: ctx_len,
                    prompt_tokens: prompt_tokens_est,
                    breaker_open: omni_exec::resilience::is_provider_in_cooldown(&p.id)
                        || omni_exec::resilience::is_connection_in_cooldown(&p.id, &c.connection_id),
                });
            }
        } else if rm.explicit_provider
            || p.anonymous_api_key.is_some()
            || p.auth_type == "none"
            || p.auth_type == "optional"
            || !p.passthrough_models
        {
            let exec_key = format!("{}::default::{}", p.id, rm.model);
            let m = metrics.get(&exec_key).cloned().unwrap_or(TargetMetric {
                requests: 0,
                success_rate: 100.0,
                avg_latency_ms: 100.0,
            });
            unconfigured_targets.push(ComboTarget {
                provider: p.id.clone(),
                connection_id: "default".into(),
                model: rm.model.clone(),
                weight: 1.0,
                fallback_only: p.passthrough_models,
                cost_per_m_out: if p.is_free_tier { 0.0 } else { 1.0 },
                quota_remaining_hint: 1_000_000,
                reset_in_ms: 0,
                request_count: m.requests,
                success_rate: m.success_rate,
                avg_latency_ms: m.avg_latency_ms,
                headroom_bytes: 0,
                context_len: ctx_len,
                prompt_tokens: prompt_tokens_est,
                breaker_open: omni_exec::resilience::is_provider_in_cooldown(&p.id),
            });
        }
    }

    let targets = if !configured_targets.is_empty() {
        configured_targets
    } else {
        unconfigured_targets
    };

    let mut rng = rand::rngs::OsRng;
    let mut rr_slot = state.next_rr();
    let lkgp = state.get_last_good(&resolved_name);
    let ordered = order_targets_with_auto(
        strategy,
        AutoRoutingStrategy::Rules,
        &targets,
        &mut rng,
        Some(&mut rr_slot),
        lkgp.as_deref(),
    );
    (strategy, false, ordered)
}

// ── POST /v1/chat/completions ───────────────────────────────────────

pub async fn chat_completions(
    State(state): State<SharedState>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let api_key = match authorize_request(&state, &headers, false) {
        Ok(k) => k,
        Err(resp) => return resp,
    };
    let req: ChatRequest = match serde_json::from_str(&body) {
        Ok(r) => r,
        Err(e) => {
            return format_error_response(StatusCode::BAD_REQUEST, &e.to_string(), false, None);
        }
    };
    execute_chat_pipeline(state, headers, api_key, req, ResponseSurface::OpenAiChat).await
}

// ── POST /v1/messages & /v1/messages/count_tokens ───────────────────

pub async fn messages(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let api_key = match authorize_request(&state, &headers, true) {
        Ok(k) => k,
        Err(resp) => return resp,
    };
    let req = claude_to_openai_request(&body);
    if req.model.is_empty() {
        return format_error_response(
            StatusCode::BAD_REQUEST,
            "missing required field: model",
            true,
            None,
        );
    }
    execute_chat_pipeline(state, headers, api_key, req, ResponseSurface::AnthropicMessages).await
}

pub async fn count_tokens(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, true) {
        return resp;
    }
    let serialized = body.to_string();
    let est = ((serialized.len() as f64) / 4.0).ceil().max(1.0) as u64;
    json_response(StatusCode::OK, json!({"input_tokens": est}))
}

fn format_non_stream_for_surface(surface: ResponseSurface, oai: &Value, model: &str) -> Value {
    match surface {
        ResponseSurface::OpenAiChat => oai.clone(),
        ResponseSurface::AnthropicMessages => openai_to_claude_response(oai, model),
        ResponseSurface::OpenAiResponses => openai_to_responses_response(oai, model),
        ResponseSurface::GeminiNative => openai_to_gemini_response(oai, model),
        ResponseSurface::LegacyCompletion => openai_chat_to_legacy_completion(oai, model),
    }
}

// ── Unified pipeline across all 5 surfaces ──────────────────────────

pub(crate) async fn execute_chat_pipeline(
    state: SharedState,
    headers: HeaderMap,
    api_key: Option<String>,
    mut req: ChatRequest,
    surface: ResponseSurface,
) -> Response {
    let is_anthropic = surface.is_anthropic();
    let stream = req.stream.unwrap_or(false);
    let requested_model = req.model.clone();
    let no_log = no_log_requested(&headers);
    let no_cache = no_cache_requested(&headers);
    let key_hash = api_key.as_deref().map(hash_api_key).unwrap_or_default();
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let surface_path = match surface {
        ResponseSurface::OpenAiChat => "/v1/chat/completions",
        ResponseSurface::AnthropicMessages => "/v1/messages",
        ResponseSurface::OpenAiResponses => "/v1/responses",
        ResponseSurface::GeminiNative => "/v1beta/models",
        ResponseSurface::LegacyCompletion => "/v1/completions",
    };

    // 0. Hard Budget Cap Enforcement (daily/weekly/monthly per-key & global caps)
    if let Err((status, msg)) = crate::part_a::check_budget_enforcement(&state, &key_hash) {
        if !no_log {
            crate::part_a::write_call_log(
                &state,
                surface_path,
                status.as_u16(),
                "gateway",
                "",
                &requested_model,
                0,
                0,
                0,
                Some(msg.clone()),
            );
        }
        return format_error_response(status, &msg, is_anthropic, None);
    }

    // 1. Guardrails: Prompt Injection + PII Masking / Reversible PII Vault
    let mut pii_vault = PiiVault::default();
    let raw_payload = serde_json::to_value(&req).unwrap_or_else(|_| json!({}));
    if state.guardrails_enabled() {
        let outcome = process_request(&raw_payload, None);
        if outcome.blocked {
            let _ = state.store.record_guardrail_event(
                now_ms(),
                "prompt_injection",
                "block",
                "",
                &requested_model,
                &json!({"matches": outcome.injection_matches}),
            );
            return format_error_response(
                StatusCode::BAD_REQUEST,
                "Request blocked by prompt-injection guardrail",
                is_anthropic,
                None,
            );
        }
        if state.pii_redaction_enabled() && !outcome.detections.is_empty() {
            let _ = state.store.record_guardrail_event(
                now_ms(),
                "pii_redaction",
                if state.pii_vault_enabled() { "vault_mask" } else { "redact" },
                "",
                &requested_model,
                &json!({"count": outcome.detections.len()}),
            );
            if state.pii_vault_enabled() {
                for msg in &mut req.messages {
                    if let Some(Value::String(s)) = msg.content.as_mut() {
                        let (masked, _) = pii_vault.mask(s, None);
                        *s = masked;
                    }
                }
            } else if let Ok(masked_req) = serde_json::from_value::<ChatRequest>(outcome.masked) {
                req = masked_req;
            }
        }
    }

    // 2. RTK + Caveman Compression + Adaptive Context Compaction
    let comp_cfg = state.compression_config();
    let mut comp_saved_tokens: u64 = 0;
    if comp_cfg.enabled {
        let ctx_hint = {
            let cat = state.catalog.read().unwrap();
            cat.resolve_model(&requested_model)
                .first()
                .map(|r| r.provider.context_length_for(&r.model))
                .unwrap_or(128_000)
        };
        let report = compress_chat_request(&mut req, &comp_cfg, ctx_hint);
        if report.applied {
            comp_saved_tokens = report.saved_tokens;
            state.record_compression_savings(report.saved_tokens, report.compacted_turns);
        }
    }

    // 3. Semantic Cache lookup (non-stream, no tools, respects require_zero_temperature & key isolation)
    let cache_cfg = state.semantic_cache_config();
    let cacheable = cache_cfg.enabled
        && !stream
        && !no_cache
        && req.tools.is_none()
        && (!cache_cfg.require_zero_temperature
            || req.temperature.map(|t| t <= f64::EPSILON).unwrap_or(true));
    let cache_prompt = if cacheable {
        let pairs: Vec<(String, String)> = req
            .messages
            .iter()
            .filter_map(|m| {
                let text = match &m.content {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => other.to_string(),
                    None => return None,
                };
                Some((m.role.clone(), text))
            })
            .collect();
        normalize_messages_for_cache(&pairs, &cache_cfg)
    } else {
        None
    };

    if let Some(ref norm_prompt) = cache_prompt {
        if let Some(hit) =
            state
                .semantic_cache
                .lookup(&cache_cfg, &key_hash, &requested_model, norm_prompt)
        {
            let formatted = format_non_stream_for_surface(surface, &hit.response, &requested_model);
            let mut b = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/json")
                .header("X-OmniRoute-Cache", hit.kind.header_value())
                .header("X-OmniRoute-Cache-Similarity", format!("{:.3}", hit.similarity));
            if comp_saved_tokens > 0 {
                b = b.header("X-OmniRoute-Compression-Saved", comp_saved_tokens.to_string());
            }
            return b.body(Body::from(formatted.to_string())).unwrap();
        }
    }

    // 4. Resolve candidates via combo/catalog + 22-strategy router
    let strategy_override = req
        .extra
        .get("_strategy")
        .and_then(Value::as_str)
        .map(normalize_strategy);
    let prompt_tokens_est = (serde_json::to_string(&req.messages)
        .map(|s| s.len())
        .unwrap_or(0)
        / 4) as i64;
    let (_strategy, is_combo, ordered) =
        expand_and_order_candidates(&state, &requested_model, strategy_override, prompt_tokens_est);

    if ordered.is_empty() {
        return format_error_response(
            StatusCode::NOT_FOUND,
            &format!("no provider serves model {requested_model}"),
            is_anthropic,
            None,
        );
    }

    let all_creds = state.store.list_credentials().unwrap_or_default();
    let agent_guard_mode = state.agent_guard_mode();

    let mut last_failure: Option<DispatchFailure> = None;
    let max_passes: u32 = 2;

    for pass in 0..max_passes {
        let mut shortest_rate_wait_ms: Option<u64> = None;

        for candidate in &ordered {
            let provider_desc = {
                let catalog = state.catalog.read().unwrap();
                match catalog.find(&candidate.provider) {
                    Some(p) => p.clone(),
                    None => continue,
                }
            };

            if omni_exec::resilience::is_model_locked(
                &candidate.provider,
                &candidate.connection_id,
                &candidate.model,
            ) {
                if let Some(rem) = omni_exec::resilience::model_lockout_remaining_ms(
                    &candidate.provider,
                    &candidate.connection_id,
                    &candidate.model,
                ) {
                    shortest_rate_wait_ms =
                        Some(shortest_rate_wait_ms.map(|a| a.min(rem)).unwrap_or(rem));
                }
                continue;
            }
            if omni_exec::resilience::is_provider_in_cooldown(&candidate.provider)
                || omni_exec::resilience::is_connection_in_cooldown(
                    &candidate.provider,
                    &candidate.connection_id,
                )
            {
                if let Some(rem) =
                    omni_exec::resilience::provider_cooldown_remaining_ms(&candidate.provider)
                {
                    shortest_rate_wait_ms =
                        Some(shortest_rate_wait_ms.map(|a| a.min(rem)).unwrap_or(rem));
                }
                continue;
            }

            let ag = evaluate_agent_guard(
                agent_guard_mode,
                &provider_desc.id,
                user_agent.as_deref(),
                &raw_payload,
            );
            if ag.flagged {
                let _ = state.store.record_guardrail_event(
                    now_ms(),
                    "agent_guard",
                    if ag.blocked { "block" } else { "monitor" },
                    &provider_desc.id,
                    &candidate.model,
                    &json!({"mode": ag.mode.as_str(), "reason": ag.reason}),
                );
                if ag.blocked {
                    return format_error_response(
                        StatusCode::FORBIDDEN,
                        "Request blocked by agent-guard guardrail",
                        is_anthropic,
                        None,
                    );
                }
            }

            let cred_opt = all_creds.iter().find(|c| {
                c.enabled
                    && c.provider == provider_desc.id
                    && c.connection_id == candidate.connection_id
            });
            let active_cred = match cred_opt {
                Some(c) => Some(ensure_fresh_oauth_credential(&state, c).await),
                None => None,
            };
            let Some(base_conn) =
                build_connection(&state, &provider_desc, &candidate.model, stream, active_cred.as_ref())
            else {
                continue;
            };

            let upstream_model = provider_desc.upstream_model_id(&candidate.model);
            let unsupported = provider_desc.unsupported_params_for(&candidate.model);
            let empty_psj = json!({});
            let psj = active_cred
                .as_ref()
                .map(|c| &c.provider_specific_json)
                .unwrap_or(&empty_psj);

            let mut prepared = prepare_provider_request(
                &provider_desc,
                base_conn,
                &req,
                &upstream_model,
                stream,
                psj,
                &unsupported,
            );

            let started = std::time::Instant::now();
            let mut result = dispatch_once_detailed(
                &state.dispatcher,
                &prepared.endpoint,
                &prepared.body,
                stream,
                pass,
            )
            .await;

            // Reactive 401 OAuth token refresh retry once
            if let Err(ref fail) = result {
                if fail.error.status() == 401 {
                    if let Some(ref c) = active_cred {
                        if c.refresh_token.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
                            || matches!(c.provider.as_str(), "github" | "ghe-copilot")
                        {
                            if let Ok(fresh) = refresh_credential_now(&state, c, None, None).await {
                                if let Some(retry_conn) = build_connection(
                                    &state,
                                    &provider_desc,
                                    &candidate.model,
                                    stream,
                                    Some(&fresh),
                                ) {
                                    prepared = prepare_provider_request(
                                        &provider_desc,
                                        retry_conn,
                                        &req,
                                        &upstream_model,
                                        stream,
                                        &fresh.provider_specific_json,
                                        &unsupported,
                                    );
                                    result = dispatch_once_detailed(
                                        &state.dispatcher,
                                        &prepared.endpoint,
                                        &prepared.body,
                                        stream,
                                        pass,
                                    )
                                    .await;
                                }
                            }
                        }
                    }
                }
            }

            let conn = prepared.endpoint;
            let tool_name_map = prepared.tool_name_map;
            let upstream_resp_fmt = prepared.response_format;
            let extra_resp_headers = prepared.extra_response_headers;
            let latency_ms = started.elapsed().as_millis() as i64;
            let exec_key = candidate.execution_key();

            match result {
                Ok(resp) => {
                    state.bump_metric(&exec_key, true, latency_ms as f64);
                    state.set_last_good(&requested_model, &exec_key);
                    let status = resp.status();

                    if stream {
                        if !no_log {
                            let est_cost = crate::part_a::estimate_cost_usd(
                                &conn.provider,
                                &candidate.model,
                                prompt_tokens_est,
                                0,
                                provider_desc.is_free_tier,
                            );
                            let _ = state.store.record_usage_full(&UsageEntry {
                                ts_ms: now_ms(),
                                provider: conn.provider.clone(),
                                connection_id: conn.connection_id.clone(),
                                model: candidate.model.clone(),
                                status: status.as_u16(),
                                prompt_tokens: prompt_tokens_est,
                                completion_tokens: 0,
                                total_tokens: prompt_tokens_est,
                                cache_read_tokens: 0,
                                cache_write_tokens: 0,
                                latency_ms,
                                cost_usd: est_cost,
                                api_key_hash: key_hash.clone(),
                                session_id: String::new(),
                            });
                            crate::part_a::write_call_log(
                                &state,
                                surface_path,
                                status.as_u16(),
                                &conn.provider,
                                &conn.connection_id,
                                &candidate.model,
                                latency_ms,
                                prompt_tokens_est,
                                0,
                                None,
                            );
                        }
                        state.record_gamification_action(
                            if is_combo { XpAction::ComboUse } else { XpAction::Request },
                            Some(&conn.provider),
                            Some(&candidate.model),
                        );
                        return sse_passthrough(
                            &conn,
                            &candidate.model,
                            status,
                            resp,
                            surface,
                            tool_name_map,
                            pii_vault,
                            comp_saved_tokens,
                            extra_resp_headers,
                        )
                        .await;
                    }

                    let resp_is_sse = resp
                        .headers()
                        .get(header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .map(|s| s.to_ascii_lowercase().contains("text/event-stream"))
                        .unwrap_or(false);

                    match resp.bytes().await {
                        Ok(bytes) => {
                            let looks_like_sse = resp_is_sse || {
                                let trimmed = String::from_utf8_lossy(&bytes);
                                let s = trimmed.trim_start();
                                s.starts_with("data:") || s.starts_with("event:")
                            };
                            let v: Value = if looks_like_sse {
                                collect_sse_chunks_to_openai(
                                    &conn.provider,
                                    &candidate.model,
                                    &bytes,
                                    &tool_name_map,
                                )
                            } else {
                                serde_json::from_slice(&bytes).unwrap_or_else(|_| {
                                    json!({"raw": String::from_utf8_lossy(&bytes).to_string()})
                                })
                            };
                            let (pt, ct, tt, cr, cw) = if upstream_resp_fmt
                                == UpstreamResponseFormat::BedrockConverse
                            {
                                let ipt = v.pointer("/usage/inputTokens").and_then(Value::as_i64).unwrap_or(0);
                                let oct = v.pointer("/usage/outputTokens").and_then(Value::as_i64).unwrap_or(0);
                                let ott = v.pointer("/usage/totalTokens").and_then(Value::as_i64).unwrap_or(ipt + oct);
                                (ipt, oct, ott, 0, 0)
                            } else {
                                extract_usage_tokens(&v)
                            };
                            if !no_log {
                                let est_cost = crate::part_a::estimate_cost_usd(
                                    &conn.provider,
                                    &candidate.model,
                                    pt,
                                    ct,
                                    provider_desc.is_free_tier,
                                );
                                let _ = state.store.record_usage_full(&UsageEntry {
                                    ts_ms: now_ms(),
                                    provider: conn.provider.clone(),
                                    connection_id: conn.connection_id.clone(),
                                    model: candidate.model.clone(),
                                    status: status.as_u16(),
                                    prompt_tokens: pt,
                                    completion_tokens: ct,
                                    total_tokens: tt,
                                    cache_read_tokens: cr,
                                    cache_write_tokens: cw,
                                    latency_ms,
                                    cost_usd: est_cost,
                                    api_key_hash: key_hash.clone(),
                                    session_id: String::new(),
                                });
                                crate::part_a::write_call_log(
                                    &state,
                                    surface_path,
                                    status.as_u16(),
                                    &conn.provider,
                                    &conn.connection_id,
                                    &candidate.model,
                                    latency_ms,
                                    pt,
                                    ct,
                                    None,
                                );
                            }
                            state.record_gamification_action(
                                if is_combo { XpAction::ComboUse } else { XpAction::Request },
                                Some(&conn.provider),
                                Some(&candidate.model),
                            );

                            let mut oai = if upstream_resp_fmt
                                == UpstreamResponseFormat::BedrockConverse
                            {
                                bedrock_converse_to_openai(&candidate.model, &v)
                            } else {
                                normalize_non_stream_to_openai(&candidate.model, v, &tool_name_map)
                            };
                            if !pii_vault.is_empty() {
                                if let Some(text) = oai
                                    .pointer_mut("/choices/0/message/content")
                                    .and_then(|x| x.as_str().map(String::from))
                                {
                                    oai["choices"][0]["message"]["content"] =
                                        Value::String(pii_vault.restore(&text));
                                }
                            }

                            if let Some(ref norm_prompt) = cache_prompt {
                                state.semantic_cache.store(
                                    &cache_cfg,
                                    &key_hash,
                                    &requested_model,
                                    norm_prompt,
                                    &conn.provider,
                                    oai.clone(),
                                    tt,
                                );
                            }

                            let final_body =
                                format_non_stream_for_surface(surface, &oai, &candidate.model);
                            let mut b = Response::builder()
                                .status(StatusCode::OK)
                                .header(header::CONTENT_TYPE, "application/json")
                                .header("X-OmniRoute-Provider", &conn.provider)
                                .header(
                                    "X-OmniRoute-Cache",
                                    if cacheable { "MISS" } else { "BYPASS" },
                                );
                            if comp_saved_tokens > 0 {
                                b = b.header(
                                    "X-OmniRoute-Compression-Saved",
                                    comp_saved_tokens.to_string(),
                                );
                            }
                            for (hk, hv) in &extra_resp_headers {
                                b = b.header(hk.as_str(), hv.as_str());
                            }
                            return b.body(Body::from(final_body.to_string())).unwrap();
                        }
                        Err(e) => {
                            last_failure = Some(DispatchFailure {
                                error: OmniError::Network(e.to_string()),
                                retry_after_ms: None,
                                cooldown_ms: 2_000,
                                is_terminal: false,
                                is_rate_limit: false,
                                is_local_refusal_replay: false,
                            });
                            continue;
                        }
                    }
                }
                Err(fail) => {
                    state.bump_metric(&exec_key, false, latency_ms as f64);
                    if omni_exec::resilience::is_provider_in_cooldown(&conn.provider) {
                        crate::part_a::dispatch_alert_async(
                            state.clone(),
                            "breaker_open",
                            format!("Circuit breaker OPEN: {}", conn.provider),
                            format!(
                                "Provider '{}' entered cooldown after HTTP {} on model '{}'",
                                conn.provider,
                                fail.error.status(),
                                candidate.model
                            ),
                            json!({
                                "provider": conn.provider,
                                "connection_id": conn.connection_id,
                                "model": candidate.model,
                                "status": fail.error.status()
                            }),
                        );
                    }
                    if !no_log {
                        let _ = state.store.record_usage_full(&UsageEntry {
                            ts_ms: now_ms(),
                            provider: conn.provider.clone(),
                            connection_id: conn.connection_id.clone(),
                            model: candidate.model.clone(),
                            status: fail.error.status(),
                            prompt_tokens: 0,
                            completion_tokens: 0,
                            total_tokens: 0,
                            cache_read_tokens: 0,
                            cache_write_tokens: 0,
                            latency_ms,
                            cost_usd: 0.0,
                            api_key_hash: key_hash.clone(),
                            session_id: String::new(),
                        });
                        let err_msg = match &fail.error {
                            OmniError::Upstream { body, .. } => body.clone().unwrap_or_else(|| format!("HTTP {}", fail.error.status())),
                            OmniError::Network(m) | OmniError::BadRequest(m) | OmniError::Translate(m) => m.clone(),
                        };
                        crate::part_a::write_call_log(
                            &state,
                            surface_path,
                            fail.error.status(),
                            &conn.provider,
                            &conn.connection_id,
                            &candidate.model,
                            latency_ms,
                            0,
                            0,
                            Some(err_msg),
                        );
                    }
                    // Skip outer cooldown-aware retry loop for Antigravity/agy 429s & local
                    // refusal replays so we never multiply upstream 429 attempts (antigravity429Guard.ts).
                    if fail.is_rate_limit
                        && !fail.is_local_refusal_replay
                        && !omni_exec::antigravity::is_antigravity_provider(&conn.provider)
                    {
                        let wait = fail.retry_after_ms.unwrap_or(fail.cooldown_ms);
                        shortest_rate_wait_ms =
                            Some(shortest_rate_wait_ms.map(|a| a.min(wait)).unwrap_or(wait));
                    }
                    last_failure = Some(fail);
                    continue;
                }
            }
        }

        if pass + 1 < max_passes {
            if let Some(wait_ms) = shortest_rate_wait_ms {
                if wait_ms > 0 && wait_ms <= 5_000 {
                    tokio::time::sleep(Duration::from_millis(wait_ms)).await;
                    continue;
                }
            }
        }
        break;
    }

    match last_failure {
        Some(f) => failure_to_response(&f, is_anthropic),
        None => format_error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "All candidate providers are currently in cooldown",
            is_anthropic,
            Some(5_000),
        ),
    }
}

// ── SSE passthrough & multi-surface streaming translation ───────────

#[allow(clippy::too_many_arguments)]
async fn sse_passthrough(
    conn: &ConnectionEndpoint,
    model: &str,
    status: reqwest::StatusCode,
    resp: reqwest::Response,
    surface: ResponseSurface,
    tool_name_map: Vec<(String, String)>,
    pii_vault: PiiVault,
    comp_saved_tokens: u64,
    extra_resp_headers: Vec<(String, String)>,
) -> Response {
    let stream = resp.bytes_stream();
    let provider = conn.provider.clone();
    let model_owned = model.to_string();
    let conn_key = conn.connection_id.clone();

    let s = async_stream::stream! {
        let mut assembler = SseAssembler::new();
        let mut claude_out = if surface == ResponseSurface::AnthropicMessages {
            Some(OpenAiToClaudeStreamState::new(&model_owned))
        } else {
            None
        };
        let mut responses_out = if surface == ResponseSurface::OpenAiResponses {
            Some(OpenAiToResponsesStreamState::new(&model_owned))
        } else {
            None
        };
        let mut pin = stream;
        let mut line_buf = String::new();

        while let Some(chunk) = pin.next().await {
            match chunk {
                Ok(bytes) => {
                    line_buf.push_str(&String::from_utf8_lossy(&bytes));
                    while let Some(pos) = line_buf.find('\n') {
                        let raw_line: String = line_buf.drain(..=pos).collect();
                        if let Some(frame) = assembler.feed_line(raw_line.trim_end()) {
                            if frame.done { continue; }
                            let event = frame.event.as_deref().unwrap_or("");
                            let upstream = serde_json::from_str::<Value>(&frame.data).ok();
                            if let Some(mut oai_chunk) = normalize_stream_chunk_to_openai(
                                &provider, &model_owned, event, upstream, &tool_name_map
                            ) {
                                if !pii_vault.is_empty() {
                                    if let Some(t) = oai_chunk
                                        .pointer("/choices/0/delta/content")
                                        .and_then(Value::as_str)
                                        .map(String::from)
                                    {
                                        oai_chunk["choices"][0]["delta"]["content"] =
                                            Value::String(pii_vault.restore(&t));
                                    }
                                }
                                if let Some(st) = claude_out.as_mut() {
                                    for sse_frame in st.push_openai_chunk(&oai_chunk) {
                                        yield Ok::<_, std::io::Error>(sse_frame.into_bytes());
                                    }
                                } else if let Some(st) = responses_out.as_mut() {
                                    for sse_frame in st.push_openai_chunk(&oai_chunk) {
                                        yield Ok::<_, std::io::Error>(sse_frame.into_bytes());
                                    }
                                } else if surface == ResponseSurface::GeminiNative {
                                    if let Some(gem_frame) = openai_chunk_to_gemini_sse(&oai_chunk, &model_owned) {
                                        yield Ok::<_, std::io::Error>(gem_frame.into_bytes());
                                    }
                                } else {
                                    let payload = format!("data: {}\n\n", oai_chunk);
                                    yield Ok::<_, std::io::Error>(payload.into_bytes());
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("stream error provider={provider}: {e}");
                    omni_exec::resilience::record_connection_failure(&provider, &conn_key);
                    break;
                }
            }
        }
        if let Some(st) = claude_out.as_mut() {
            for sse_frame in st.finish() {
                yield Ok::<_, std::io::Error>(sse_frame.into_bytes());
            }
        } else if let Some(st) = responses_out.as_mut() {
            for sse_frame in st.finish() {
                yield Ok::<_, std::io::Error>(sse_frame.into_bytes());
            }
        } else if surface != ResponseSurface::GeminiNative {
            yield Ok::<_, std::io::Error>(b"data: [DONE]\n\n".to_vec());
        }
    };

    let mut b = Response::builder()
        .status(StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::OK))
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::CONNECTION, "keep-alive")
        .header("X-OmniRoute-Provider", &conn.provider);
    if comp_saved_tokens > 0 {
        b = b.header("X-OmniRoute-Compression-Saved", comp_saved_tokens.to_string());
    }
    for (hk, hv) in &extra_resp_headers {
        b = b.header(hk.as_str(), hv.as_str());
    }
    b.body(Body::from_stream(s)).unwrap()
}

// ── GET /v1/models & GET /v1/models/{*model} ────────────────────────

#[derive(Debug, Deserialize, Default)]
pub struct ModelsQuery {
    pub provider: Option<String>,
    pub q: Option<String>,
    pub limit: Option<usize>,
}

pub async fn list_models(
    State(state): State<SharedState>,
    Query(query): Query<ModelsQuery>,
) -> Response {
    let catalog = state.catalog.read().unwrap();
    let now = now_ms() / 1000;
    let q_lower = query.q.as_deref().map(|s| s.to_ascii_lowercase());
    let limit = query.limit.unwrap_or(500).min(5_000);

    let mut data: Vec<Value> = Vec::new();
    if query.provider.is_none() {
        if let Ok(combos) = state.store.list_combos() {
            for c in combos.into_iter().filter(|c| c.enabled) {
                if let Some(ref ql) = q_lower {
                    if !c.name.to_ascii_lowercase().contains(ql) {
                        continue;
                    }
                }
                data.push(json!({
                    "id": c.name,
                    "object": "model",
                    "created": now,
                    "owned_by": "omniroute-combo",
                    "strategy": c.strategy
                }));
            }
        }
    }

    for p in &catalog.providers {
        if let Some(ref pf) = query.provider {
            if !p.matches_id_or_alias(pf) {
                continue;
            }
        }
        for spec in &p.model_specs {
            let full_id = format!("{}/{}", p.id, spec.id);
            if let Some(ref ql) = q_lower {
                if !full_id.to_ascii_lowercase().contains(ql)
                    && !spec.name.to_ascii_lowercase().contains(ql)
                {
                    continue;
                }
            }
            data.push(json!({
                "id": full_id,
                "object": "model",
                "created": now,
                "owned_by": p.id,
                "name": spec.name,
                "context_length": spec.context_length.or(p.default_context_length),
                "max_output_tokens": spec.max_output_tokens,
                "supports_reasoning": spec.supports_reasoning,
                "supports_vision": spec.supports_vision,
                "free_tier": p.is_free_tier
            }));
            if data.len() >= limit {
                break;
            }
        }
        if data.len() >= limit {
            break;
        }
    }

    json_response(
        StatusCode::OK,
        json!({
            "object": "list",
            "upstream_version": omni_catalog::upstream_version(),
            "provider_count": catalog.provider_count(),
            "total_catalog_models": catalog.model_count(),
            "data": data
        }),
    )
}

pub async fn get_model(
    State(state): State<SharedState>,
    Path(model_id): Path<String>,
) -> Response {
    let catalog = state.catalog.read().unwrap();
    let hits = catalog.resolve_model(&model_id);
    if hits.is_empty() {
        return json_response(
            StatusCode::NOT_FOUND,
            serde_json::to_value(build_error_body(404, &format!("model {model_id} not found")))
                .unwrap(),
        );
    }
    let first = &hits[0];
    let spec = first.provider.find_model_spec(&first.model);
    json_response(
        StatusCode::OK,
        json!({
            "id": model_id,
            "object": "model",
            "owned_by": first.provider.id,
            "format": first.provider.format,
            "context_length": first.provider.context_length_for(&first.model),
            "spec": spec,
            "providers": hits.iter().map(|h| h.provider.id.as_str()).collect::<Vec<_>>()
        }),
    )
}
