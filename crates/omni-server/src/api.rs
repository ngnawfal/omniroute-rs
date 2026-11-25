// Management REST API (/api/*), Self-Service API (/v1/me/*), and MCP HTTP/stdio context.

use crate::chat::{authorize_request, expand_and_order_candidates, extract_api_key};
use crate::state::{hash_api_key, now_ms, SharedState};
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::Response;
use omni_gami::XpAction;
use omni_guard::{add_custom_guard_pattern, custom_patterns, Severity};
use omni_mcp::{handle_http_rpc, McpContext};
use omni_router::{normalize_strategy, PUBLIC_STRATEGIES};
use omni_store::{ComboRow, ComboStep, CredentialRow};
use serde::Deserialize;
use serde_json::{json, Value};

fn json_ok(body: Value) -> Response {
    json_status(StatusCode::OK, body)
}

fn json_status(status: StatusCode, body: Value) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

// ── Health ──────────────────────────────────────────────────────────

pub async fn api_health(State(state): State<SharedState>) -> Response {
    json_ok(build_health_json(&state))
}

pub fn build_health_json(state: &SharedState) -> Value {
    let catalog = state.catalog.read().unwrap();
    let breakers: Vec<Value> = omni_exec::resilience::all_provider_states()
        .into_iter()
        .map(|(p, s, streak)| json!({"provider": p, "state": s, "failure_streak": streak}))
        .collect();
    let lockouts: Vec<Value> = omni_exec::resilience::all_lockouts()
        .into_iter()
        .map(|l| {
            json!({
                "provider": l.provider,
                "connection_id": l.connection_id,
                "model": l.model,
                "reason": l.reason,
                "failure_count": l.failure_count
            })
        })
        .collect();
    let usage = state.store.usage_summary().unwrap_or_else(|_| json!({}));
    let combos_count = state.store.list_combos().map(|c| c.len()).unwrap_or(0);
    let creds_count = state.store.list_credentials().map(|c| c.len()).unwrap_or(0);
    json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "upstream_sync_version": omni_catalog::upstream_version(),
        "uptime_secs": state.started_at.elapsed().as_secs(),
        "provider_count": catalog.provider_count(),
        "model_count": catalog.model_count(),
        "routing_strategies": PUBLIC_STRATEGIES.len(),
        "default_strategy": state.default_strategy().as_str(),
        "configured_credentials": creds_count,
        "combos_count": combos_count,
        "dashboard_locked": state.is_dashboard_locked(),
        "alerting_enabled": state.alerting_config().enabled,
        "upstream_proxy_configured": state.proxy_config().global.is_some()
            || !state.proxy_config().providers.is_empty()
            || !state.proxy_config().pool.is_empty(),
        "guardrails_enabled": state.guardrails_enabled(),
        "agent_guard_mode": state.agent_guard_mode().as_str(),
        "compression": state.compression_summary(),
        "cache": state.cache_summary(),
        "oauth": state.oauth_summary(),
        "breakers": breakers,
        "active_lockouts": lockouts,
        "usage": usage,
    })
}

// ── Providers & Credentials CRUD ────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
pub struct ProviderQuery {
    pub q: Option<String>,
    pub free_only: Option<bool>,
    pub limit: Option<usize>,
}

pub async fn list_providers(
    State(state): State<SharedState>,
    Query(query): Query<ProviderQuery>,
) -> Response {
    json_ok(build_providers_json(
        &state,
        query.q.as_deref(),
        query.free_only.unwrap_or(false),
        query.limit.unwrap_or(500).min(1000),
    ))
}

fn build_providers_json(
    state: &SharedState,
    q: Option<&str>,
    free_only: bool,
    limit: usize,
) -> Value {
    let catalog = state.catalog.read().unwrap();
    let creds = state.store.list_credentials().unwrap_or_default();
    let ql = q.map(|s| s.to_ascii_lowercase());
    let mut items = Vec::new();
    for p in &catalog.providers {
        if free_only && !p.is_free_tier {
            continue;
        }
        if let Some(ref s) = ql {
            let hit = p.id.to_ascii_lowercase().contains(s)
                || p.alias.as_deref().unwrap_or("").to_ascii_lowercase().contains(s)
                || p.name.as_deref().unwrap_or("").to_ascii_lowercase().contains(s);
            if !hit {
                continue;
            }
        }
        let configured_accounts = creds.iter().filter(|c| c.enabled && c.provider == p.id).count();
        items.push(json!({
            "id": p.id,
            "alias": p.alias,
            "name": p.name,
            "format": p.format,
            "executor": p.executor,
            "auth_type": p.auth_type,
            "auth_header": p.effective_auth_header(),
            "base_urls": p.base_urls,
            "passthrough_models": p.passthrough_models,
            "is_free_tier": p.is_free_tier,
            "model_count": p.model_specs.len(),
            "configured_accounts": configured_accounts,
        }));
        if items.len() >= limit {
            break;
        }
    }
    json!({
        "upstream_version": omni_catalog::upstream_version(),
        "total": catalog.provider_count(),
        "count": items.len(),
        "providers": items
    })
}

pub async fn get_provider(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Response {
    let catalog = state.catalog.read().unwrap();
    match catalog.find(&id) {
        Some(p) => json_ok(serde_json::to_value(p).unwrap_or(json!({}))),
        None => json_status(StatusCode::NOT_FOUND, json!({"error": "provider not found"})),
    }
}

#[derive(Debug, Deserialize)]
pub struct UpsertProviderBody {
    pub id: String,
    pub base_urls: Vec<String>,
    #[serde(default = "default_bearer")]
    pub auth_type: String,
    #[serde(default)]
    pub chat_path: String,
    #[serde(default = "default_openai")]
    pub format: String,
    #[serde(default)]
    pub models: Vec<String>,
}

fn default_bearer() -> String {
    "bearer".into()
}
fn default_openai() -> String {
    "openai".into()
}

pub async fn upsert_custom_provider(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<UpsertProviderBody>,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    if let Err(e) = state.store.upsert_custom_provider(
        &body.id,
        &body.base_urls,
        &body.auth_type,
        &body.chat_path,
        &body.format,
        &body.models,
    ) {
        return json_status(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e.to_string()}));
    }
    let _ = state.reload_catalog();
    json_status(StatusCode::CREATED, json!({"ok": true, "id": body.id}))
}

pub async fn delete_custom_provider(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    let deleted = state.store.delete_custom_provider(&id).unwrap_or(false);
    let _ = state.reload_catalog();
    json_ok(json!({"deleted": deleted, "id": id}))
}

pub async fn list_credentials(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    let rows = state.store.list_credentials().unwrap_or_default();
    let masked: Vec<Value> = rows
        .into_iter()
        .map(|c| {
            let preview = if c.api_key.len() > 8 {
                format!("{}...{}", &c.api_key[..4], &c.api_key[c.api_key.len() - 4..])
            } else if c.api_key.is_empty() {
                String::new()
            } else {
                "***".into()
            };
            json!({
                "provider": c.provider,
                "connection_id": c.connection_id,
                "auth_type": c.auth_type,
                "api_key_preview": preview,
                "base_urls": c.base_urls,
                "models": c.models,
                "enabled": c.enabled,
                "priority": c.priority,
                "weight": c.weight,
                "label": c.label,
                "has_refresh_token": c.refresh_token.as_deref().map(|s| !s.is_empty()).unwrap_or(false),
                "expires_at": c.expires_at,
                "health_status": c.health_status,
                "last_refreshed_at": c.last_refreshed_at,
                "health_check_interval_min": c.health_check_interval_min,
                "proxy_url": c.provider_specific_json.get("proxy_url").or_else(|| c.provider_specific_json.get("proxyUrl")).and_then(Value::as_str),
                "synced_models_count": c.provider_specific_json.get("synced_models").and_then(Value::as_array).map(|a| a.len()).unwrap_or(c.models.len()),
            })
        })
        .collect();
    json_ok(json!({"credentials": masked}))
}

pub async fn upsert_credential(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<CredentialRow>,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    match state.store.upsert_credential_full(&body) {
        Ok(()) => json_status(
            StatusCode::CREATED,
            json!({"ok": true, "provider": body.provider, "connection_id": body.connection_id}),
        ),
        Err(e) => json_status(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e.to_string()})),
    }
}

pub async fn delete_credential(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((provider, connection_id)): Path<(String, String)>,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    let deleted = state
        .store
        .delete_credential(&provider, &connection_id)
        .unwrap_or(false);
    json_ok(json!({"deleted": deleted, "provider": provider, "connection_id": connection_id}))
}

// ── Combos CRUD + Dry-Run Simulation ────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct UpsertComboBody {
    pub id: Option<String>,
    pub name: String,
    #[serde(default = "default_priority")]
    pub strategy: String,
    #[serde(default = "default_rules")]
    pub auto_strategy: String,
    #[serde(default)]
    pub models: Vec<ComboStep>,
    #[serde(default)]
    pub config: Value,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_priority() -> String {
    "priority".into()
}
fn default_rules() -> String {
    "rules".into()
}
fn default_true() -> bool {
    true
}

pub async fn list_combos(State(state): State<SharedState>) -> Response {
    let combos = state.store.list_combos().unwrap_or_default();
    json_ok(json!({
        "strategies": PUBLIC_STRATEGIES.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        "combos": combos
    }))
}

pub async fn get_combo(State(state): State<SharedState>, Path(id): Path<String>) -> Response {
    match state.store.get_combo(&id) {
        Ok(Some(c)) => json_ok(serde_json::to_value(c).unwrap_or(json!({}))),
        _ => json_status(StatusCode::NOT_FOUND, json!({"error": "combo not found"})),
    }
}

pub async fn upsert_combo(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<UpsertComboBody>,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    let now = now_ms();
    let id = body
        .id
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let norm_strat = normalize_strategy(&body.strategy).as_str().to_string();
    let row = ComboRow {
        id: id.clone(),
        name: body.name,
        strategy: norm_strat,
        auto_strategy: body.auto_strategy,
        models: body.models,
        config: if body.config.is_null() { json!({}) } else { body.config },
        enabled: body.enabled,
        created_at: now,
        updated_at: now,
    };
    match state.store.upsert_combo(&row) {
        Ok(()) => {
            state.record_gamification_action(XpAction::ComboCreate, None, Some(&row.name));
            json_status(StatusCode::CREATED, serde_json::to_value(row).unwrap())
        }
        Err(e) => json_status(StatusCode::BAD_REQUEST, json!({"error": e.to_string()})),
    }
}

pub async fn delete_combo(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    let deleted = state.store.delete_combo(&id).unwrap_or(false);
    json_ok(json!({"deleted": deleted, "id": id}))
}

#[derive(Debug, Deserialize)]
pub struct SimulateBody {
    pub model: String,
    pub strategy: Option<String>,
    pub prompt_tokens: Option<i64>,
}

pub async fn simulate_route(
    State(state): State<SharedState>,
    axum::Json(body): axum::Json<SimulateBody>,
) -> Response {
    let strat_override = body.strategy.as_deref().map(normalize_strategy);
    let (used_strategy, is_combo, ordered) = expand_and_order_candidates(
        &state,
        &body.model,
        strat_override,
        body.prompt_tokens.unwrap_or(1_000),
    );
    json_ok(json!({
        "model": body.model,
        "is_combo": is_combo,
        "strategy": used_strategy.as_str(),
        "candidates": ordered.iter().map(|t| json!({
            "provider": t.provider,
            "connection_id": t.connection_id,
            "model": t.model,
            "weight": t.weight,
            "fallback_only": t.fallback_only,
            "breaker_open": t.breaker_open,
            "context_len": t.context_len,
            "success_rate": t.success_rate,
            "avg_latency_ms": t.avg_latency_ms,
        })).collect::<Vec<_>>()
    }))
}

// ── API Keys CRUD ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateKeyBody {
    pub key: Option<String>,
    #[serde(default)]
    pub label: String,
    pub expires_in_days: Option<i64>,
    pub budget_5h_usd: Option<f64>,
    pub rpm_limit: Option<u32>,
}

pub async fn list_keys(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    if let Err(resp) = crate::part_a::authorize_management(&state, &headers) {
        return resp;
    }
    let rows = state.store.list_api_key_rows().unwrap_or_default();
    let items: Vec<Value> = rows
        .into_iter()
        .map(|k| {
            let preview = if k.key.len() > 8 {
                format!("{}...{}", &k.key[..5], &k.key[k.key.len() - 4..])
            } else {
                "***".into()
            };
            json!({
                "key": k.key,
                "key_preview": preview,
                "key_hash": hash_api_key(&k.key),
                "label": k.label,
                "created_at": k.created_at,
                "disabled": k.disabled,
                "expires_at": k.expires_at,
                "budget_5h_usd": k.budget_5h_usd,
                "rpm_limit": k.rpm_limit,
            })
        })
        .collect();
    json_ok(json!({"keys": items}))
}

pub async fn create_key(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<CreateKeyBody>,
) -> Response {
    if let Err(resp) = crate::part_a::authorize_management(&state, &headers) {
        return resp;
    }
    let now = now_ms();
    let key = body
        .key
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("sk-omni-{}", uuid::Uuid::new_v4().simple()));
    let expires_at = body.expires_in_days.map(|d| now + d * 86_400_000);
    match state.store.add_api_key_with_limits(
        &key,
        &body.label,
        now,
        expires_at,
        body.budget_5h_usd,
        body.rpm_limit,
    ) {
        Ok(()) => json_status(
            StatusCode::CREATED,
            json!({"key": key, "label": body.label, "expires_at": expires_at, "rpm_limit": body.rpm_limit}),
        ),
        Err(e) => json_status(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e.to_string()})),
    }
}

pub async fn delete_key(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Response {
    if let Err(resp) = crate::part_a::authorize_management(&state, &headers) {
        return resp;
    }
    let deleted = state.store.delete_api_key(&key).unwrap_or(false);
    json_ok(json!({"deleted": deleted}))
}

// ── Settings, Usage, Guardrails, Gamification, MITM ─────────────────

pub async fn get_settings(State(state): State<SharedState>) -> Response {
    let map = state.store.list_settings().unwrap_or_default();
    json_ok(json!({
        "default_strategy": state.default_strategy().as_str(),
        "guardrails_enabled": state.guardrails_enabled(),
        "pii_redaction_enabled": state.pii_redaction_enabled(),
        "pii_vault_enabled": state.pii_vault_enabled(),
        "agent_guard_mode": state.agent_guard_mode().as_str(),
        "raw": map
    }))
}

pub async fn put_settings(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    let now = now_ms();
    if let Some(obj) = body.as_object() {
        for (k, v) in obj {
            let val_str = match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            let _ = state.store.set_setting(k, &val_str, now);
        }
    }
    get_settings(State(state)).await
}

pub async fn get_usage(State(state): State<SharedState>) -> Response {
    let summary = state.store.usage_summary().unwrap_or_else(|_| json!({}));
    json_ok(summary)
}

pub async fn guardrails_status(State(state): State<SharedState>) -> Response {
    let events = state.store.list_guardrail_events(25).unwrap_or_default();
    json_ok(json!({
        "enabled": state.guardrails_enabled(),
        "pii_redaction_enabled": state.pii_redaction_enabled(),
        "pii_vault_enabled": state.pii_vault_enabled(),
        "agent_guard_mode": state.agent_guard_mode().as_str(),
        "builtin_injection_patterns": omni_guard::DEFAULT_PATTERNS.len(),
        "builtin_pii_patterns": omni_guard::PII_PATTERNS.len(),
        "custom_patterns": custom_patterns(),
        "recent_events": events
    }))
}

#[derive(Debug, Deserialize)]
pub struct AddGuardPatternBody {
    pub name: String,
    pub regex: String,
    #[serde(default = "default_high")]
    pub severity: String,
}

fn default_high() -> String {
    "high".into()
}

pub async fn add_guardrail_pattern(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<AddGuardPatternBody>,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    let ok = add_custom_guard_pattern(&body.name, &body.regex, Severity::from_str_lossy(&body.severity));
    if !ok {
        return json_status(StatusCode::BAD_REQUEST, json!({"error": "invalid regex"}));
    }
    json_status(StatusCode::CREATED, json!({"ok": true, "name": body.name}))
}

pub async fn gamification_status(State(state): State<SharedState>) -> Response {
    json_ok(state.gamification_summary())
}

pub async fn mitm_targets(State(state): State<SharedState>) -> Response {
    json_ok(state.mitm_status_json())
}

pub async fn mitm_ca_cert(State(state): State<SharedState>) -> Response {
    let pem = state.mitm_ca.read().unwrap().ca_cert_pem();
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/x-pem-file")
        .header(
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"omniroute-mitm-ca.crt\"",
        )
        .body(Body::from(pem))
        .unwrap()
}

#[derive(Debug, Deserialize, Default)]
pub struct StartMitmBody {
    pub port: Option<u16>,
    pub bind_addr: Option<String>,
    pub gateway_base_url: Option<String>,
}

pub async fn mitm_start(
    State(state): State<SharedState>,
    headers: HeaderMap,
    body: Option<axum::Json<StartMitmBody>>,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    let req = body.map(|b| b.0).unwrap_or_default();
    let bind_addr = req
        .bind_addr
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("127.0.0.1:{}", req.port.unwrap_or(20443)));
    let gw_port: u16 = std::env::var("OMNIROUTE_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20128);
    let gateway_base_url = req
        .gateway_base_url
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("http://127.0.0.1:{gw_port}"));

    // Stop any previous instance cleanly before starting a new listener
    if let Some(prev) = state.mitm_handle.lock().unwrap().take() {
        prev.stop();
    }

    let ca = state.mitm_ca.read().unwrap().clone();
    let bypass = state.mitm_bypass.read().unwrap().clone();
    match omni_mitm::proxy::MitmProxyHandle::start(
        ca,
        omni_mitm::proxy::MitmProxyConfig {
            bind_addr,
            gateway_base_url,
            bypass_patterns: bypass,
            extra_targets: vec![],
        },
    )
    .await
    {
        Ok(handle) => {
            *state.mitm_handle.lock().unwrap() = Some(handle);
            json_ok(state.mitm_status_json())
        }
        Err(e) => json_status(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e})),
    }
}

pub async fn mitm_stop(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    if let Some(prev) = state.mitm_handle.lock().unwrap().take() {
        prev.stop();
    }
    json_ok(state.mitm_status_json())
}

pub async fn mitm_regenerate_cert(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    match omni_mitm::cert::CertAuthority::generate() {
        Ok(new_ca) => {
            let mitm_dir = std::path::Path::new(&state.data_dir).join("mitm");
            let _ = std::fs::create_dir_all(&mitm_dir);
            let _ = std::fs::write(mitm_dir.join("ca.crt"), new_ca.ca_cert_pem());
            let _ = std::fs::write(mitm_dir.join("ca.key"), new_ca.ca_key_pem());
            let arc_ca = std::sync::Arc::new(new_ca);
            *state.mitm_ca.write().unwrap() = arc_ca.clone();
            if let Some(ref handle) = *state.mitm_handle.lock().unwrap() {
                handle.replace_ca(arc_ca);
            }
            json_ok(state.mitm_status_json())
        }
        Err(e) => json_status(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e})),
    }
}

#[derive(Debug, Deserialize)]
pub struct BypassBody {
    pub bypass_patterns: Vec<String>,
}

pub async fn mitm_set_bypass(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<BypassBody>,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    *state.mitm_bypass.write().unwrap() = body.bypass_patterns.clone();
    if let Some(ref handle) = *state.mitm_handle.lock().unwrap() {
        handle.set_bypass_patterns(body.bypass_patterns);
    }
    json_ok(state.mitm_status_json())
}

// ── Compression & Semantic Cache Management APIs ────────────────────

pub async fn get_compression_settings(State(state): State<SharedState>) -> Response {
    json_ok(state.compression_summary())
}

pub async fn put_compression_settings(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<omni_compress::CompressionConfig>,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    let serialized = serde_json::to_string(&body).unwrap_or_else(|_| "{}".into());
    let _ = state
        .store
        .set_setting("compression_config", &serialized, now_ms());
    json_ok(state.compression_summary())
}

pub async fn get_cache_stats(State(state): State<SharedState>) -> Response {
    json_ok(state.cache_summary())
}

pub async fn put_cache_settings(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<omni_cache::SemanticCacheConfig>,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    let serialized = serde_json::to_string(&body).unwrap_or_else(|_| "{}".into());
    let _ = state
        .store
        .set_setting("semantic_cache_config", &serialized, now_ms());
    json_ok(state.cache_summary())
}

pub async fn clear_cache(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    let cleared = state.semantic_cache.clear();
    json_ok(json!({"ok": true, "cleared_entries": cleared}))
}

// ── Database Diagnostics, Upstream Import/Export & Hot Backup (/api/db/*) ─

pub async fn db_status(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    match state.store.db_diagnostics() {
        Ok(v) => json_ok(v),
        Err(e) => json_status(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e.to_string()})),
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct ExportQuery {
    pub include_secrets: Option<bool>,
}

pub async fn db_export(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(q): Query<ExportQuery>,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    match state.store.export_snapshot(q.include_secrets.unwrap_or(false)) {
        Ok(v) => json_ok(v),
        Err(e) => json_status(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e.to_string()})),
    }
}

pub async fn db_import_json(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    match state.store.import_upstream_json(&body) {
        Ok(report) => {
            let _ = state.reload_catalog();
            json_ok(serde_json::to_value(report).unwrap_or(json!({"ok": true})))
        }
        Err(e) => json_status(StatusCode::BAD_REQUEST, json!({"error": e.to_string()})),
    }
}

#[derive(Debug, Deserialize)]
pub struct ImportSqliteBody {
    pub path: String,
}

pub async fn db_import_upstream_sqlite(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<ImportSqliteBody>,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    if body.path.trim().is_empty() {
        return json_status(StatusCode::BAD_REQUEST, json!({"error": "path is required"}));
    }
    match state.store.import_upstream_sqlite(body.path.trim()) {
        Ok(report) => {
            let _ = state.reload_catalog();
            json_ok(serde_json::to_value(report).unwrap_or(json!({"ok": true})))
        }
        Err(e) => json_status(StatusCode::BAD_REQUEST, json!({"error": e.to_string()})),
    }
}

pub async fn db_backup(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let dest = format!("{}/backups/omniroute-rs-{ts}.db", state.data_dir);
    match state.store.backup_to_file(&dest) {
        Ok(bytes) => json_status(
            StatusCode::CREATED,
            json!({"ok": true, "path": dest, "size_bytes": bytes}),
        ),
        Err(e) => json_status(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e.to_string()})),
    }
}

// ── Self-Service Endpoints (/v1/me/status, /v1/me/usage, /v1/me/quota) ──

pub async fn me_status(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    let key = match authorize_request(&state, &headers, false) {
        Ok(k) => k,
        Err(resp) => return resp,
    };
    let kh = key.as_deref().map(hash_api_key).unwrap_or_default();
    let now = now_ms();
    let today_usage = state.store.usage_for_key(&kh, now - 86_400_000).unwrap_or(json!({}));
    let week_usage = state.store.usage_for_key(&kh, now - 7 * 86_400_000).unwrap_or(json!({}));
    let key_row = key.as_deref().and_then(|k| state.store.get_api_key_row(k).ok().flatten());
    json_ok(json!({
        "authenticated": key.is_some(),
        "label": key_row.as_ref().map(|r| r.label.as_str()).unwrap_or("open-door"),
        "limits": {
            "expires_at": key_row.as_ref().and_then(|r| r.expires_at),
            "budget_5h_usd": key_row.as_ref().and_then(|r| r.budget_5h_usd),
            "rpm_limit": key_row.as_ref().and_then(|r| r.rpm_limit),
        },
        "usage_24h": today_usage,
        "usage_7d": week_usage
    }))
}

#[derive(Debug, Deserialize, Default)]
pub struct UsageRangeQuery {
    pub range: Option<String>,
}

pub async fn me_usage(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(query): Query<UsageRangeQuery>,
) -> Response {
    let key = match authorize_request(&state, &headers, false) {
        Ok(k) => k,
        Err(resp) => return resp,
    };
    let range = query.range.as_deref().unwrap_or("7d");
    let days: i64 = match range {
        "1d" => 1,
        "30d" => 30,
        "90d" => 90,
        _ => 7,
    };
    let kh = key.as_deref().map(hash_api_key).unwrap_or_default();
    let since = now_ms() - days * 86_400_000;
    let usage = state.store.usage_for_key(&kh, since).unwrap_or(json!({}));
    json_ok(json!({"range": range, "usage": usage}))
}

pub async fn me_quota(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    let key = extract_api_key(&headers);
    let catalog = state.catalog.read().unwrap();
    let free_providers: Vec<&str> = catalog
        .providers
        .iter()
        .filter(|p| p.is_free_tier)
        .map(|p| p.id.as_str())
        .collect();
    let lockouts: Vec<Value> = omni_exec::resilience::all_lockouts()
        .into_iter()
        .map(|l| json!({"provider": l.provider, "model": l.model, "reason": l.reason}))
        .collect();
    json_ok(json!({
        "authenticated": key.is_some(),
        "free_tier_provider_count": free_providers.len(),
        "active_lockouts": lockouts
    }))
}

// ── MCP HTTP endpoint (POST /api/mcp) & Shared McpContext ───────────

pub struct ServerMcpContext {
    pub state: SharedState,
}

impl McpContext for ServerMcpContext {
    fn health(&self) -> Value {
        build_health_json(&self.state)
    }

    fn list_providers(&self, query: Option<&str>, free_only: bool, limit: usize) -> Value {
        build_providers_json(&self.state, query, free_only, limit)
    }

    fn list_combos(&self) -> Value {
        let combos = self.state.store.list_combos().unwrap_or_default();
        json!({"combos": combos})
    }

    fn create_combo(&self, args: &Value) -> Value {
        let name = args.get("name").and_then(Value::as_str).unwrap_or("unnamed");
        let strategy = normalize_strategy(
            args.get("strategy").and_then(Value::as_str).unwrap_or("priority"),
        )
        .as_str()
        .to_string();
        let models: Vec<ComboStep> = args
            .get("models")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let now = now_ms();
        let row = ComboRow {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            strategy,
            auto_strategy: "rules".into(),
            models,
            config: json!({}),
            enabled: true,
            created_at: now,
            updated_at: now,
        };
        match self.state.store.upsert_combo(&row) {
            Ok(()) => {
                self.state
                    .record_gamification_action(XpAction::ComboCreate, None, Some(name));
                json!({"ok": true, "combo": row})
            }
            Err(e) => json!({"ok": false, "error": e.to_string()}),
        }
    }

    fn switch_combo(&self, alias: &str, target: &str) -> Value {
        if alias.is_empty() || target.is_empty() {
            return json!({"ok": false, "error": "alias and target required"});
        }
        match self.state.store.set_model_alias(alias, target) {
            Ok(()) => json!({"ok": true, "alias": alias, "target": target}),
            Err(e) => json!({"ok": false, "error": e.to_string()}),
        }
    }

    fn combo_metrics(&self, combo_id: &str) -> Value {
        let metrics = self.state.target_metrics.read().unwrap();
        let entries: Vec<Value> = metrics
            .iter()
            .filter(|(k, _)| combo_id.is_empty() || k.contains(combo_id))
            .map(|(k, m)| {
                json!({
                    "target": k,
                    "requests": m.requests,
                    "success_rate": m.success_rate,
                    "avg_latency_ms": m.avg_latency_ms
                })
            })
            .collect();
        json!({"filter": combo_id, "metrics": entries})
    }

    fn check_quota(&self, provider: Option<&str>) -> Value {
        let creds = self.state.store.list_credentials().unwrap_or_default();
        let filtered_creds: Vec<Value> = creds
            .into_iter()
            .filter(|c| provider.map(|p| c.provider == p).unwrap_or(true))
            .map(|c| json!({"provider": c.provider, "connection_id": c.connection_id, "enabled": c.enabled}))
            .collect();
        let lockouts: Vec<Value> = omni_exec::resilience::all_lockouts()
            .into_iter()
            .filter(|l| provider.map(|p| l.provider == p).unwrap_or(true))
            .map(|l| json!({"provider": l.provider, "model": l.model, "reason": l.reason}))
            .collect();
        json!({"credentials": filtered_creds, "lockouts": lockouts})
    }

    fn route(&self, model: &str, strategy: Option<&str>) -> Value {
        self.simulate_route(model, strategy)
    }

    fn simulate_route(&self, model: &str, strategy: Option<&str>) -> Value {
        let strat = strategy.map(normalize_strategy);
        let (used, is_combo, ordered) =
            expand_and_order_candidates(&self.state, model, strat, 1_000);
        json!({
            "model": model,
            "is_combo": is_combo,
            "strategy": used.as_str(),
            "targets": ordered.iter().map(|t| json!({
                "provider": t.provider,
                "connection_id": t.connection_id,
                "model": t.model,
                "breaker_open": t.breaker_open
            })).collect::<Vec<_>>()
        })
    }

    fn cost_report(&self) -> Value {
        self.state.store.usage_summary().unwrap_or_else(|_| json!({}))
    }

    fn set_strategy(&self, strategy: &str) -> Value {
        let norm = normalize_strategy(strategy);
        let _ = self
            .state
            .store
            .set_setting("default_strategy", norm.as_str(), now_ms());
        json!({"ok": true, "default_strategy": norm.as_str()})
    }

    fn set_resilience_profile(&self, provider: &str, profile: &str) -> Value {
        let ok = omni_exec::resilience::set_provider_profile(provider, profile);
        json!({"ok": ok, "provider": provider, "profile": profile})
    }

    fn reset_breaker(&self, provider: &str) -> Value {
        omni_exec::resilience::reset_provider_breaker(provider);
        json!({"ok": true, "provider": provider})
    }

    fn pick_faster(&self, candidates: &[String]) -> Value {
        let metrics = self.state.target_metrics.read().unwrap();
        let mut scored: Vec<(String, f64)> = candidates
            .iter()
            .map(|c| {
                let lat = metrics
                    .iter()
                    .find(|(k, _)| k.contains(c))
                    .map(|(_, m)| m.avg_latency_ms)
                    .unwrap_or(1_000.0);
                (c.clone(), lat)
            })
            .collect();
        scored.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        json!({
            "ranked": scored.into_iter().map(|(m, lat)| json!({"model": m, "avg_latency_ms": lat})).collect::<Vec<_>>()
        })
    }

    fn models(&self, provider: Option<&str>, limit: usize) -> Value {
        let catalog = self.state.catalog.read().unwrap();
        let mut out = Vec::new();
        for p in &catalog.providers {
            if let Some(pf) = provider {
                if !p.matches_id_or_alias(pf) {
                    continue;
                }
            }
            for spec in &p.model_specs {
                out.push(json!({
                    "id": format!("{}/{}", p.id, spec.id),
                    "provider": p.id,
                    "name": spec.name,
                    "context_length": spec.context_length
                }));
                if out.len() >= limit {
                    break;
                }
            }
            if out.len() >= limit {
                break;
            }
        }
        json!({"total_models": catalog.model_count(), "count": out.len(), "data": out})
    }

    fn guardrail_status(&self) -> Value {
        json!({
            "enabled": self.state.guardrails_enabled(),
            "pii_redaction_enabled": self.state.pii_redaction_enabled(),
            "agent_guard_mode": self.state.agent_guard_mode().as_str(),
            "custom_patterns": custom_patterns()
        })
    }

    fn gamification_status(&self) -> Value {
        self.state.gamification_summary()
    }

    fn compression_stats(&self) -> Value {
        self.state.compression_summary()
    }

    fn cache_stats(&self) -> Value {
        self.state.cache_summary()
    }

    fn oauth_status(&self) -> Value {
        self.state.oauth_summary()
    }

    fn memory_search(&self, query: &str, limit: usize) -> Value {
        let now = now_ms();
        if query.trim().is_empty() {
            let (rows, total, tokens) = self
                .state
                .store
                .list_memories(None, None, None, None, None, now, limit, 0)
                .unwrap_or((vec![], 0, 0));
            return json!({"total": total, "tokens_used": tokens, "memories": rows});
        }
        let hits = self
            .state
            .store
            .retrieve_memories(query, None, None, now, limit)
            .unwrap_or_default();
        json!({
            "query": query,
            "matches": hits.into_iter().map(|(m, s)| json!({"score": s, "memory": m})).collect::<Vec<_>>()
        })
    }

    fn skills_list(&self, query: Option<&str>) -> Value {
        let ql = query.map(|s| s.to_ascii_lowercase());
        let skills: Vec<_> = self
            .state
            .store
            .list_skills()
            .unwrap_or_default()
            .into_iter()
            .filter(|s| {
                ql.as_ref()
                    .map(|q| {
                        s.name.to_ascii_lowercase().contains(q)
                            || s.description.to_ascii_lowercase().contains(q)
                    })
                    .unwrap_or(true)
            })
            .collect();
        json!({"count": skills.len(), "skills": skills})
    }

    fn mitm_status(&self) -> Value {
        self.state.mitm_status_json()
    }
}

pub async fn mcp_http(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    let ctx = ServerMcpContext { state };
    json_ok(handle_http_rpc(&ctx, &body))
}
