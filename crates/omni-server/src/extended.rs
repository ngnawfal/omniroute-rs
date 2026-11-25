// Extended Phase 3 gateway endpoints:
//   POST /v1/responses, POST /v1/responses/input_tokens
//   POST /v1/completions
//   POST /v1beta/models/{*spec}   (native Gemini generateContent / streamGenerateContent)
//   POST /v1/embeddings
//   POST /v1/images/generations, POST /v1/images/edits
//   POST /v1/audio/speech, POST /v1/audio/transcriptions
//   POST /v1/rerank, POST /v1/moderations
//   POST /v1/decisions            (v3.8.55 #18838 native OpenAI Decisions API)
//   GET/POST /v1/search, POST /v1/web/fetch (SSRF-guarded)

use crate::chat::{
    authorize_request, execute_chat_pipeline, format_error_response, json_response,
    no_log_requested, ResponseSurface,
};
use crate::state::{hash_api_key, now_ms, SharedState};
use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::Response;
use omni_exec::dispatch::AuthHeader;
use omni_guard::process_request;
use omni_store::{CredentialRow, UsageEntry};
use omni_translate::{
    gemini_to_openai_request, legacy_completion_to_chat, responses_to_chat_request,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::net::IpAddr;

const MAX_DECISIONS_REQUEST_BYTES: usize = 262_144; // 256 KiB

// ── POST /v1/responses & /v1/responses/input_tokens ─────────────────

pub async fn responses(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let api_key = match authorize_request(&state, &headers, false) {
        Ok(k) => k,
        Err(resp) => return resp,
    };
    let req = responses_to_chat_request(&body);
    if req.model.is_empty() {
        return format_error_response(
            StatusCode::BAD_REQUEST,
            "missing required field: model",
            false,
            None,
        );
    }
    execute_chat_pipeline(state, headers, api_key, req, ResponseSurface::OpenAiResponses).await
}

pub async fn responses_input_tokens(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    let est = ((body.to_string().len() as f64) / 4.0).ceil().max(1.0) as u64;
    json_response(StatusCode::OK, json!({"object": "response.input_tokens", "input_tokens": est}))
}

// ── POST /v1/completions ────────────────────────────────────────────

pub async fn completions(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let api_key = match authorize_request(&state, &headers, false) {
        Ok(k) => k,
        Err(resp) => return resp,
    };
    let req = legacy_completion_to_chat(&body);
    if req.model.is_empty() {
        return format_error_response(
            StatusCode::BAD_REQUEST,
            "missing required field: model",
            false,
            None,
        );
    }
    execute_chat_pipeline(state, headers, api_key, req, ResponseSurface::LegacyCompletion).await
}

// ── POST /v1beta/models/{*spec} (Native Gemini API) ─────────────────

#[derive(Debug, Deserialize, Default)]
pub struct GeminiQuery {
    pub alt: Option<String>,
}

pub async fn gemini_v1beta_models(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(spec): Path<String>,
    Query(query): Query<GeminiQuery>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let api_key = match authorize_request(&state, &headers, false) {
        Ok(k) => k,
        Err(resp) => return resp,
    };
    let (model_part, action) = match spec.rsplit_once(':') {
        Some((m, a)) => (m, a),
        None => (spec.as_str(), "generateContent"),
    };
    let stream = action.eq_ignore_ascii_case("streamGenerateContent")
        || query.alt.as_deref() == Some("sse");
    let req = gemini_to_openai_request(model_part, &body, stream);
    execute_chat_pipeline(state, headers, api_key, req, ResponseSurface::GeminiNative).await
}

// ── Generic resilient JSON proxy helper for non-chat OpenAI endpoints ─

async fn proxy_json_endpoint(
    state: SharedState,
    headers: HeaderMap,
    endpoint_path: &str,
    default_provider: &str,
    mut body: Value,
) -> Response {
    let api_key = match authorize_request(&state, &headers, false) {
        Ok(k) => k,
        Err(resp) => return resp,
    };
    let no_log = no_log_requested(&headers);
    let key_hash = api_key.as_deref().map(hash_api_key).unwrap_or_default();

    let raw_model = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    // Determine target provider + stripped model id
    let (provider_desc, upstream_model) = {
        let catalog = state.catalog.read().unwrap();
        if let Some((prefix, rest)) = raw_model.split_once('/') {
            if let Some(p) = catalog.find(prefix) {
                (p.clone(), rest.to_string())
            } else if let Some(p) = catalog.find(default_provider) {
                (p.clone(), raw_model.clone())
            } else {
                return format_error_response(
                    StatusCode::NOT_FOUND,
                    &format!("no provider found for {raw_model}"),
                    false,
                    None,
                );
            }
        } else if let Some(hit) = catalog.resolve_model(&raw_model).into_iter().next() {
            (hit.provider.clone(), hit.model)
        } else if let Some(p) = catalog.find(default_provider) {
            (p.clone(), raw_model.clone())
        } else {
            return format_error_response(
                StatusCode::NOT_FOUND,
                &format!("no provider found for {endpoint_path}"),
                false,
                None,
            );
        }
    };

    if !upstream_model.is_empty() && body.is_object() {
        body["model"] = Value::String(upstream_model.clone());
    }

    let creds: Vec<CredentialRow> = state
        .store
        .list_credentials_for_provider(&provider_desc.id)
        .unwrap_or_default();
    let attempts: Vec<Option<CredentialRow>> = if creds.is_empty() {
        vec![None]
    } else {
        creds.into_iter().map(Some).collect()
    };

    let mut last_status = StatusCode::BAD_GATEWAY;
    let mut last_msg = String::from("upstream call failed");

    for cred in attempts {
        let raw_urls = cred
            .as_ref()
            .filter(|c| !c.base_urls.is_empty())
            .map(|c| c.base_urls.clone())
            .unwrap_or_else(|| provider_desc.base_urls.clone());
        let Some(base) = raw_urls.first() else {
            continue;
        };
        let url = provider_desc.resolve_endpoint_url(base, endpoint_path);
        let auth_hdr = AuthHeader::from_str_lossy(provider_desc.effective_auth_header());
        let key = cred
            .as_ref()
            .map(|c| c.api_key.clone())
            .or_else(|| provider_desc.anonymous_api_key.clone())
            .unwrap_or_default();

        let started = std::time::Instant::now();
        let http_client = state.http_client_for(&provider_desc.id, cred.as_ref());
        let mut req = http_client
            .post(&url)
            .header("Content-Type", "application/json");
        if let Some(hname) = auth_hdr.header_name() {
            if !key.is_empty() {
                req = req.header(
                    hname,
                    auth_hdr.render_value(&key, provider_desc.auth_prefix.as_deref()),
                );
            }
        }
        for (k, v) in &provider_desc.headers {
            req = req.header(k.as_str(), v.as_str());
        }

        match req.json(&body).send().await {
            Ok(resp) => {
                let status = resp.status();
                let latency_ms = started.elapsed().as_millis() as i64;
                let ct = resp
                    .headers()
                    .get(header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("application/json")
                    .to_string();
                let bytes = resp.bytes().await.unwrap_or_default();
                if status.is_success() {
                    omni_exec::resilience::record_provider_success(&provider_desc.id);
                    if !no_log {
                        let _ = state.store.record_usage_full(&UsageEntry {
                            ts_ms: now_ms(),
                            provider: provider_desc.id.clone(),
                            connection_id: cred
                                .as_ref()
                                .map(|c| c.connection_id.clone())
                                .unwrap_or_else(|| "default".into()),
                            model: if upstream_model.is_empty() {
                                endpoint_path.to_string()
                            } else {
                                upstream_model.clone()
                            },
                            status: status.as_u16(),
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
                    }
                    return Response::builder()
                        .status(StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::OK))
                        .header(header::CONTENT_TYPE, ct)
                        .header("X-OmniRoute-Provider", &provider_desc.id)
                        .body(Body::from(bytes))
                        .unwrap();
                }
                omni_exec::resilience::record_provider_failure(&provider_desc.id);
                last_status = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
                last_msg = String::from_utf8_lossy(&bytes).to_string();
            }
            Err(e) => {
                omni_exec::resilience::record_provider_failure(&provider_desc.id);
                last_status = StatusCode::BAD_GATEWAY;
                last_msg = e.to_string();
            }
        }
    }

    format_error_response(last_status, &last_msg, false, None)
}

// ── Embeddings, Images, Rerank, Moderations, Audio ──────────────────

pub async fn embeddings(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    proxy_json_endpoint(state, headers, "/embeddings", "openai", body).await
}

pub async fn images_generations(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    proxy_json_endpoint(state, headers, "/images/generations", "openai", body).await
}

pub async fn images_edits(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    proxy_json_endpoint(state, headers, "/images/edits", "openai", body).await
}

pub async fn rerank(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    proxy_json_endpoint(state, headers, "/rerank", "cohere", body).await
}

pub async fn moderations(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    proxy_json_endpoint(state, headers, "/moderations", "openai", body).await
}

pub async fn audio_speech(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    proxy_json_endpoint(state, headers, "/audio/speech", "openai", body).await
}

pub async fn audio_transcriptions(
    State(state): State<SharedState>,
    headers: HeaderMap,
    raw_body: Bytes,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    let provider_id = headers
        .get("x-omniroute-provider")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("openai")
        .to_string();
    let provider_desc = {
        let cat = state.catalog.read().unwrap();
        match cat.find(&provider_id) {
            Some(p) => p.clone(),
            None => {
                return format_error_response(
                    StatusCode::NOT_FOUND,
                    &format!("unknown provider {provider_id}"),
                    false,
                    None,
                );
            }
        }
    };
    let cred = state
        .store
        .list_credentials_for_provider(&provider_desc.id)
        .ok()
        .and_then(|v| v.into_iter().next());
    let base = cred
        .as_ref()
        .and_then(|c| c.base_urls.first().cloned())
        .or_else(|| provider_desc.base_urls.first().cloned())
        .unwrap_or_default();
    let url = provider_desc.resolve_endpoint_url(&base, "/audio/transcriptions");
    let ct = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream");
    let auth_hdr = AuthHeader::from_str_lossy(provider_desc.effective_auth_header());
    let key = cred.map(|c| c.api_key).unwrap_or_default();

    let mut req = state
        .dispatcher
        .client
        .post(&url)
        .header(header::CONTENT_TYPE, ct)
        .body(raw_body);
    if let Some(hname) = auth_hdr.header_name() {
        if !key.is_empty() {
            req = req.header(
                hname,
                auth_hdr.render_value(&key, provider_desc.auth_prefix.as_deref()),
            );
        }
    }
    match req.send().await {
        Ok(resp) => {
            let st = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::OK);
            let out_ct = resp
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/json")
                .to_string();
            let bytes = resp.bytes().await.unwrap_or_default();
            Response::builder()
                .status(st)
                .header(header::CONTENT_TYPE, out_ct)
                .body(Body::from(bytes))
                .unwrap()
        }
        Err(e) => format_error_response(StatusCode::BAD_GATEWAY, &e.to_string(), false, None),
    }
}

// ── POST /v1/decisions (v3.8.55 #18838 native OpenAI Decisions API) ─

pub async fn decisions(
    State(state): State<SharedState>,
    headers: HeaderMap,
    raw_body: Bytes,
) -> Response {
    if raw_body.len() > MAX_DECISIONS_REQUEST_BYTES {
        return format_error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            &format!("Decisions request body exceeds {MAX_DECISIONS_REQUEST_BYTES} bytes"),
            false,
            None,
        );
    }
    let body: Value = match serde_json::from_slice(&raw_body) {
        Ok(v) => v,
        Err(_) => {
            return format_error_response(StatusCode::BAD_REQUEST, "Invalid JSON body", false, None);
        }
    };
    let model = body.get("model").and_then(Value::as_str).unwrap_or("");
    let valid_model = matches!(model, "openai/gpt-6-luna" | "gpt-6-luna");
    if !valid_model {
        return format_error_response(
            StatusCode::BAD_REQUEST,
            &format!(
                "Unknown Decisions model: {model}. POST /v1/decisions serves \"openai/gpt-6-luna\""
            ),
            false,
            None,
        );
    }
    if body.get("questions").and_then(Value::as_array).map(|a| a.is_empty()).unwrap_or(true) {
        return format_error_response(
            StatusCode::BAD_REQUEST,
            "missing required non-empty array: questions",
            false,
            None,
        );
    }
    if state.guardrails_enabled() {
        let outcome = process_request(&body, None);
        if outcome.blocked {
            return format_error_response(
                StatusCode::BAD_REQUEST,
                "Request blocked by prompt-injection guardrail",
                false,
                None,
            );
        }
    }
    proxy_json_endpoint(state, headers, "/decisions", "openai", body).await
}

// ── Search Registry (GET/POST /v1/search) & Web Fetch (/v1/web/fetch) ─

struct SearchProviderDef {
    id: &'static str,
    name: &'static str,
    base_url: &'static str,
    auth_header: &'static str,
    keyless: bool,
    search_types: &'static [&'static str],
}

static SEARCH_PROVIDERS: &[SearchProviderDef] = &[
    SearchProviderDef { id: "serper-search", name: "Serper Search", base_url: "https://google.serper.dev/search", auth_header: "x-api-key", keyless: false, search_types: &["web", "news"] },
    SearchProviderDef { id: "brave-search", name: "Brave Search", base_url: "https://api.search.brave.com/res/v1/web/search", auth_header: "x-subscription-token", keyless: false, search_types: &["web", "news"] },
    SearchProviderDef { id: "perplexity-search", name: "Perplexity Search", base_url: "https://api.perplexity.ai/search", auth_header: "bearer", keyless: false, search_types: &["web"] },
    SearchProviderDef { id: "exa-search", name: "Exa Search", base_url: "https://api.exa.ai/search", auth_header: "x-api-key", keyless: false, search_types: &["web", "news"] },
    SearchProviderDef { id: "tavily-search", name: "Tavily Search", base_url: "https://api.tavily.com/search", auth_header: "bearer", keyless: false, search_types: &["web", "news", "finance"] },
    SearchProviderDef { id: "nimble-search", name: "Nimble Search", base_url: "https://sdk.nimbleway.com/v1/search", auth_header: "bearer", keyless: false, search_types: &["web"] },
    SearchProviderDef { id: "keenable-search", name: "Keenable Search", base_url: "https://api.keenable.ai/v1/search", auth_header: "none", keyless: true, search_types: &["web"] },
    SearchProviderDef { id: "duckduckgo-search", name: "DuckDuckGo Instant", base_url: "https://api.duckduckgo.com/", auth_header: "none", keyless: true, search_types: &["web"] },
];

pub async fn list_search_providers() -> Response {
    let ts = now_ms() / 1000;
    let data: Vec<Value> = SEARCH_PROVIDERS
        .iter()
        .map(|p| {
            json!({
                "id": p.id,
                "object": "search_provider",
                "created": ts,
                "name": p.name,
                "keyless": p.keyless,
                "search_types": p.search_types
            })
        })
        .collect();
    json_response(StatusCode::OK, json!({"object": "list", "data": data}))
}

#[derive(Debug, Deserialize)]
pub struct SearchRequestBody {
    pub query: String,
    pub provider: Option<String>,
    pub max_results: Option<usize>,
}

pub async fn search(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<SearchRequestBody>,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    let query = body.query.trim();
    if query.is_empty() {
        return format_error_response(StatusCode::BAD_REQUEST, "query must not be empty", false, None);
    }

    let max_results = body.max_results.unwrap_or(5).clamp(1, 50);
    let selected = if let Some(ref pid) = body.provider {
        SEARCH_PROVIDERS.iter().find(|p| p.id == pid)
    } else {
        // Prefer first configured search provider, fall back to keyless duckduckgo-search
        let creds = state.store.list_credentials().unwrap_or_default();
        SEARCH_PROVIDERS
            .iter()
            .find(|p| creds.iter().any(|c| c.enabled && c.provider == p.id))
            .or_else(|| SEARCH_PROVIDERS.iter().find(|p| p.id == "duckduckgo-search"))
    };
    let Some(prov) = selected else {
        return format_error_response(StatusCode::NOT_FOUND, "unknown search provider", false, None);
    };

    let cred = state
        .store
        .list_credentials_for_provider(prov.id)
        .ok()
        .and_then(|v| v.into_iter().next());
    if !prov.keyless && cred.is_none() {
        return format_error_response(
            StatusCode::BAD_REQUEST,
            &format!("no credentials configured for search provider {}", prov.id),
            false,
            None,
        );
    }

    let resp_res = if prov.id == "duckduckgo-search" {
        state
            .dispatcher
            .client
            .get(prov.base_url)
            .query(&[("q", query), ("format", "json"), ("no_html", "1")])
            .send()
            .await
    } else {
        let mut req = state.dispatcher.client.post(prov.base_url).json(&json!({
            "q": query,
            "query": query,
            "num": max_results,
            "max_results": max_results
        }));
        if let Some(c) = cred {
            if prov.auth_header == "bearer" {
                req = req.header("Authorization", format!("Bearer {}", c.api_key));
            } else if prov.auth_header != "none" {
                req = req.header(prov.auth_header, c.api_key);
            }
        }
        req.send().await
    };

    match resp_res {
        Ok(r) => {
            let st = StatusCode::from_u16(r.status().as_u16()).unwrap_or(StatusCode::OK);
            let v: Value = r.json().await.unwrap_or_else(|_| json!({}));
            json_response(
                st,
                json!({
                    "provider": prov.id,
                    "query": query,
                    "max_results": max_results,
                    "data": v
                }),
            )
        }
        Err(e) => format_error_response(StatusCode::BAD_GATEWAY, &e.to_string(), false, None),
    }
}

/// SSRF guard: blocks loopback, RFC1918 private, link-local (AWS/GCP metadata 169.254.169.254),
/// and non-HTTP(S) schemes before any outbound web fetch.
pub fn is_safe_public_url(raw: &str) -> Result<reqwest::Url, &'static str> {
    let parsed = reqwest::Url::parse(raw).map_err(|_| "invalid URL")?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("only http and https schemes are allowed");
    }
    let host = parsed.host_str().ok_or("missing host")?.to_ascii_lowercase();
    if host == "localhost"
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host == "metadata.google.internal"
    {
        return Err("blocked private/loopback hostname");
    }
    if let Ok(ip) = host.trim_matches(|c| c == '[' || c == ']').parse::<IpAddr>() {
        let blocked = match ip {
            IpAddr::V4(v4) => {
                v4.is_loopback()
                    || v4.is_private()
                    || v4.is_link_local()
                    || v4.is_broadcast()
                    || v4.is_unspecified()
            }
            IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified(),
        };
        if blocked {
            return Err("blocked private/loopback/metadata IP");
        }
    }
    Ok(parsed)
}

#[derive(Debug, Deserialize)]
pub struct WebFetchBody {
    pub url: String,
    pub max_chars: Option<usize>,
}

pub async fn web_fetch(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<WebFetchBody>,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    let url = match is_safe_public_url(&body.url) {
        Ok(u) => u,
        Err(reason) => {
            return format_error_response(StatusCode::BAD_REQUEST, reason, false, None);
        }
    };
    let max_chars = body.max_chars.unwrap_or(32_000).min(200_000);
    match state.dispatcher.client.get(url.clone()).send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let ct = resp
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let raw = resp.text().await.unwrap_or_default();
            let text: String = raw.chars().take(max_chars).collect();
            json_response(
                StatusCode::OK,
                json!({
                    "url": url.as_str(),
                    "status": status,
                    "content_type": ct,
                    "length": raw.len(),
                    "content": text
                }),
            )
        }
        Err(e) => format_error_response(StatusCode::BAD_GATEWAY, &e.to_string(), false, None),
    }
}
