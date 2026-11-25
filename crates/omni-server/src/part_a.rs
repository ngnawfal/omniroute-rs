// Part A Priority Features (Upstream OmniRoute v3.8.55 Parity — Ultra-Low-RAM Native Rust):
//   1. Per-Account & Global HTTP/SOCKS5 Upstream Proxy + Pool + Live Proxy Tester (/api/proxies*)
//   2. Call Logs Inspector, Daily Analytics, Hard Cost/Budget Caps & CSV/JSONL Export (/api/logs*, /api/analytics, /api/budgets*)
//   3. Live Quota Monitor & OAuth Live Model Auto-Sync (/api/quota, /api/providers/{p}/{c}/sync-models)
//   4. Model Aliases CRUD, CLI Tools Auto-Configurator & OpenAPI 3.1 Spec (/api/aliases*, /api/cli-tools, /api/openapi)
//   5. Dashboard Password Auth with Signed HttpOnly Cookie (/api/auth/*)
//   6. Webhook + Telegram Bot Alerting (/api/alerts/*)
//   7. OpenAI /v1/files & Async /v1/batches Execution Engine

use crate::chat::{
    authorize_request, execute_chat_pipeline, format_error_response, json_response, ResponseSurface,
};
use crate::oauth_api::ensure_fresh_oauth_credential;
use crate::state::{
    now_ms, AlertingConfig, AppState, DashboardAuthConfig, SharedState, UpstreamProxyConfig,
};
use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::Response;
use chrono::Utc;
use omni_exec::antigravity::{
    antigravity_content_user_agent, is_antigravity_provider, scrub_proxy_and_fingerprint_headers,
};
use omni_exec::dispatch::validate_proxy_url;
use omni_exec::executors::{hmac_sha256, ANTIGRAVITY_DISCOVERY_ENDPOINTS};
use omni_oauth::{
    exchange_copilot_token, GITHUB_COPILOT_API_VERSION, GITHUB_COPILOT_EDITOR_VERSION,
    GITHUB_COPILOT_PLUGIN_VERSION, GITHUB_COPILOT_USER_AGENT,
};
use omni_store::{CallLogRow, CredentialRow, DomainBudgetRow, OpenAiBatchRow, OpenAiFileRow};
use omni_types::ChatRequest;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::{Duration, Instant};

const SESSION_COOKIE_NAME: &str = "omniroute_session";
const MAX_UPLOAD_FILE_BYTES: usize = 8 * 1024 * 1024; // 8 MiB cap for STB RAM safety
const ALERT_DEDUP_WINDOW_MS: i64 = 5 * 60 * 1000; // 5 minutes

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

// ── Dashboard Session Cookie & Management Authorization ─────────────

pub fn extract_session_cookie(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        let trimmed = part.trim();
        if let Some(val) = trimmed.strip_prefix(&format!("{SESSION_COOKIE_NAME}=")) {
            let v = val.trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

pub fn has_valid_dashboard_session(state: &SharedState, headers: &HeaderMap) -> bool {
    if !state.is_dashboard_locked() {
        return true;
    }
    extract_session_cookie(headers)
        .map(|tok| state.verify_dashboard_session_token(&tok))
        .unwrap_or(false)
}

/// Authorizes `/api/*` dashboard management calls:
/// - If dashboard password lock is active: accepts valid signed HttpOnly session cookie OR valid gateway API key.
/// - Otherwise (unlocked local/LAN dashboard): allows management UI access directly even after `/v1/*` API keys
///   have been created for external tools (Hermes, Cursor, CLI), so creating a key never blanks out the Dashboard.
pub fn authorize_management(state: &SharedState, headers: &HeaderMap) -> Result<(), Response> {
    if state.is_dashboard_locked() {
        if has_valid_dashboard_session(state, headers) {
            return Ok(());
        }
        // Also allow programmatic automation with a valid gateway API key when keys exist
        if state.store.has_api_keys().unwrap_or(false) {
            if authorize_request(state, headers, false).is_ok() {
                return Ok(());
            }
        }
        return Err(json_status(
            StatusCode::UNAUTHORIZED,
            json!({
                "error": "Dashboard authentication required",
                "dashboard_locked": true
            }),
        ));
    }
    Ok(())
}

// ── 1. Upstream Proxies (HTTP/HTTPS/SOCKS5) & Live Tester ───────────

pub async fn get_proxies(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    let cfg = state.proxy_config();
    let creds = state.store.list_credentials().unwrap_or_default();
    let account_overrides: Vec<Value> = creds
        .into_iter()
        .filter_map(|c| {
            let p = c
                .provider_specific_json
                .get("proxy_url")
                .or_else(|| c.provider_specific_json.get("proxyUrl"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())?
                .to_string();
            Some(json!({
                "provider": c.provider,
                "connection_id": c.connection_id,
                "label": c.label,
                "proxy_url": p,
            }))
        })
        .collect();
    json_ok(json!({
        "config": cfg,
        "cached_clients": state.dispatcher.cached_proxy_count(),
        "account_overrides": account_overrides,
        "supported_schemes": ["http", "https", "socks5", "socks5h"],
    }))
}

pub async fn put_proxies(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(mut body): axum::Json<UpstreamProxyConfig>,
) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    // Validate global proxy
    if let Some(ref g) = body.global {
        let t = g.trim();
        if t.is_empty() || t.eq_ignore_ascii_case("direct") || t.eq_ignore_ascii_case("none") {
            body.global = None;
        } else if let Err(e) = validate_proxy_url(t) {
            return json_status(StatusCode::BAD_REQUEST, json!({"error": format!("global proxy: {e}")}));
        } else {
            body.global = Some(t.to_string());
        }
    }
    // Validate per-provider overrides
    let mut clean_providers = HashMap::new();
    for (prov, url) in &body.providers {
        let t = url.trim();
        if t.is_empty() {
            continue;
        }
        if t.eq_ignore_ascii_case("direct") || t.eq_ignore_ascii_case("none") {
            clean_providers.insert(prov.clone(), "direct".to_string());
            continue;
        }
        if let Err(e) = validate_proxy_url(t) {
            return json_status(
                StatusCode::BAD_REQUEST,
                json!({"error": format!("provider '{prov}' proxy: {e}")}),
            );
        }
        clean_providers.insert(prov.clone(), t.to_string());
    }
    body.providers = clean_providers;
    // Validate pool entries
    let mut clean_pool = Vec::new();
    for item in &body.pool {
        let t = item.trim();
        if t.is_empty() {
            continue;
        }
        if let Err(e) = validate_proxy_url(t) {
            return json_status(
                StatusCode::BAD_REQUEST,
                json!({"error": format!("pool entry '{t}': {e}")}),
            );
        }
        clean_pool.push(t.to_string());
    }
    body.pool = clean_pool;

    let serialized = serde_json::to_string(&body).unwrap_or_else(|_| "{}".into());
    if let Err(e) = state
        .store
        .set_setting("upstream_proxy_config", &serialized, now_ms())
    {
        return json_status(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e.to_string()}));
    }
    get_proxies(State(state), headers).await
}

#[derive(Debug, Deserialize)]
pub struct TestProxyBody {
    pub proxy_url: String,
    pub target_url: Option<String>,
}

pub async fn test_proxy(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<TestProxyBody>,
) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    let validated = match validate_proxy_url(&body.proxy_url) {
        Ok(u) => u,
        Err(e) => return json_status(StatusCode::BAD_REQUEST, json!({"ok": false, "error": e})),
    };
    let client = match state.dispatcher.client_for_proxy(Some(&validated)) {
        Ok(c) => c,
        Err(e) => {
            return json_status(
                StatusCode::BAD_REQUEST,
                json!({"ok": false, "error": e.to_string()}),
            )
        }
    };
    let probe_url = body
        .target_url
        .as_deref()
        .map(str::trim)
        .filter(|s| s.starts_with("http://") || s.starts_with("https://"))
        .unwrap_or("https://api.ipify.org?format=json");

    let started = Instant::now();
    match client
        .get(probe_url)
        .timeout(Duration::from_secs(12))
        .send()
        .await
    {
        Ok(resp) => {
            let latency_ms = started.elapsed().as_millis() as u64;
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            let exit_ip = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v.get("ip").and_then(Value::as_str).map(String::from));
            json_ok(json!({
                "ok": (200..400).contains(&status),
                "proxy_url": validated,
                "target_url": probe_url,
                "status": status,
                "latency_ms": latency_ms,
                "exit_ip": exit_ip,
                "body_preview": text.chars().take(200).collect::<String>(),
            }))
        }
        Err(e) => json_ok(json!({
            "ok": false,
            "proxy_url": validated,
            "target_url": probe_url,
            "latency_ms": started.elapsed().as_millis() as u64,
            "error": e.to_string(),
        })),
    }
}

// ── 2. Cost Estimation, Budget Enforcement, Call Logs & Analytics ───

/// Lightweight per-1M token cost estimator (USD) so budgets and analytics reflect real spend.
pub fn estimate_cost_usd(
    provider: &str,
    model: &str,
    prompt_tokens: i64,
    completion_tokens: i64,
    is_free_tier: bool,
) -> f64 {
    if is_free_tier
        || matches!(
            provider,
            "ollama" | "lmstudio" | "vllm" | "mlx" | "pollinations" | "groq" | "cerebras"
        )
    {
        return 0.0;
    }
    let m = model.to_ascii_lowercase();
    let (in_per_m, out_per_m) = if m.contains("opus") {
        (15.0, 75.0)
    } else if m.contains("sonnet") {
        (3.0, 15.0)
    } else if m.contains("haiku") {
        (0.8, 4.0)
    } else if m.contains("gpt-5") || m.contains("gpt-6") || m.contains("o3") || m.contains("o1") {
        (5.0, 15.0)
    } else if m.contains("gpt-4o-mini") || m.contains("mini") || m.contains("flash") || m.contains("nano") {
        (0.15, 0.6)
    } else if m.contains("gpt-4") || m.contains("codex") {
        (2.5, 10.0)
    } else if m.contains("gemini-2.5-pro") || m.contains("gemini-3") {
        (1.25, 10.0)
    } else if m.contains("deepseek") {
        (0.27, 1.10)
    } else if m.contains("qwen") || m.contains("llama") || m.contains("mistral") {
        (0.3, 0.9)
    } else {
        (1.0, 3.0)
    };
    let in_cost = (prompt_tokens.max(0) as f64) * in_per_m / 1_000_000.0;
    let out_cost = (completion_tokens.max(0) as f64) * out_per_m / 1_000_000.0;
    ((in_cost + out_cost) * 1_000_000.0).round() / 1_000_000.0
}

/// Checks daily/weekly/monthly hard cost caps for the calling API key hash AND `"global"`.
/// Returns `Err((402, reason))` when a cap is breached and triggers an async alert.
pub fn check_budget_enforcement(
    state: &SharedState,
    key_hash: &str,
) -> Result<(), (StatusCode, String)> {
    let now = now_ms();
    let mut scopes: Vec<(&str, &str)> = vec![("global", "global")];
    if !key_hash.is_empty() {
        scopes.push((key_hash, key_hash));
    }
    for (budget_id, spend_scope) in scopes {
        let Ok(Some(budget)) = state.store.get_domain_budget(budget_id) else {
            continue;
        };
        let checks = [
            ("daily", budget.daily_limit_usd, 86_400_000i64),
            ("weekly", budget.weekly_limit_usd, 7 * 86_400_000i64),
            ("monthly", budget.monthly_limit_usd, 30 * 86_400_000i64),
        ];
        for (period, limit, window_ms) in checks {
            if limit <= 0.0 {
                continue;
            }
            let spent = state
                .store
                .spend_usd_since(spend_scope, now - window_ms)
                .unwrap_or(0.0);
            if spent >= limit {
                let msg = format!(
                    "Budget cap exceeded for '{budget_id}' ({period}): ${spent:.4} >= ${limit:.2}"
                );
                dispatch_alert_async(
                    state.clone(),
                    "budget_exceeded",
                    format!("Budget cap exceeded ({budget_id}/{period})"),
                    msg.clone(),
                    json!({ "scope": budget_id, "period": period, "spent_usd": spent, "limit_usd": limit }),
                );
                return Err((StatusCode::PAYMENT_REQUIRED, msg));
            }
            if budget.warning_threshold > 0.0 && spent >= limit * budget.warning_threshold {
                dispatch_alert_async(
                    state.clone(),
                    "budget_warning",
                    format!("Budget warning ({budget_id}/{period})"),
                    format!(
                        "Spend for '{budget_id}' ({period}) reached ${spent:.4} / ${limit:.2} ({:.0}%)",
                        (spent / limit) * 100.0
                    ),
                    json!({ "scope": budget_id, "period": period, "spent_usd": spent, "limit_usd": limit }),
                );
            }
        }
    }
    Ok(())
}

#[derive(Debug, Deserialize, Default)]
pub struct LogsQuery {
    pub limit: Option<usize>,
    pub provider: Option<String>,
    pub status: Option<String>,
    pub q: Option<String>,
    pub format: Option<String>,
}

pub async fn list_logs(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(q): Query<LogsQuery>,
) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    let limit = q.limit.unwrap_or(100).clamp(1, 2000);
    let rows = state
        .store
        .list_call_logs(limit, q.provider.as_deref(), q.status.as_deref(), q.q.as_deref())
        .unwrap_or_default();
    json_ok(json!({
        "count": rows.len(),
        "logs": rows,
    }))
}

pub async fn clear_logs(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    let deleted = state.store.clear_call_logs().unwrap_or(0);
    json_ok(json!({ "ok": true, "deleted": deleted }))
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

pub async fn export_logs(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(q): Query<LogsQuery>,
) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    let rows = state
        .store
        .list_call_logs(
            q.limit.unwrap_or(2000).clamp(1, 2000),
            q.provider.as_deref(),
            q.status.as_deref(),
            q.q.as_deref(),
        )
        .unwrap_or_default();
    let fmt = q.format.as_deref().unwrap_or("jsonl").to_ascii_lowercase();
    if fmt == "csv" {
        let mut out = String::from(
            "id,timestamp,method,path,status,provider,connection_id,model,duration_ms,tokens_in,tokens_out,error_summary\n",
        );
        for r in rows {
            out.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{},{},{}\n",
                csv_escape(&r.id),
                csv_escape(&r.timestamp),
                csv_escape(&r.method),
                csv_escape(&r.path),
                r.status,
                csv_escape(&r.provider),
                csv_escape(&r.connection_id),
                csv_escape(&r.model),
                r.duration,
                r.tokens_in,
                r.tokens_out,
                csv_escape(r.error_summary.as_deref().unwrap_or("")),
            ));
        }
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/csv; charset=utf-8")
            .header(
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"omniroute-call-logs.csv\"",
            )
            .body(Body::from(out))
            .unwrap()
    } else {
        let mut out = String::new();
        for r in rows {
            if let Ok(line) = serde_json::to_string(&r) {
                out.push_str(&line);
                out.push('\n');
            }
        }
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/x-ndjson; charset=utf-8")
            .header(
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"omniroute-call-logs.jsonl\"",
            )
            .body(Body::from(out))
            .unwrap()
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct AnalyticsQuery {
    pub days: Option<i64>,
}

pub async fn get_analytics(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(q): Query<AnalyticsQuery>,
) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    let days = q.days.unwrap_or(14).clamp(1, 90);
    match state.store.usage_analytics(days, now_ms()) {
        Ok(v) => json_ok(v),
        Err(e) => json_status(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e.to_string()})),
    }
}

pub async fn list_budgets(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    let now = now_ms();
    let rows = state.store.list_domain_budgets().unwrap_or_default();
    let enriched: Vec<Value> = rows
        .into_iter()
        .map(|b| {
            let daily_spent = state
                .store
                .spend_usd_since(&b.api_key_id, now - 86_400_000)
                .unwrap_or(0.0);
            let weekly_spent = state
                .store
                .spend_usd_since(&b.api_key_id, now - 7 * 86_400_000)
                .unwrap_or(0.0);
            let monthly_spent = state
                .store
                .spend_usd_since(&b.api_key_id, now - 30 * 86_400_000)
                .unwrap_or(0.0);
            json!({
                "api_key_id": b.api_key_id,
                "daily_limit_usd": b.daily_limit_usd,
                "weekly_limit_usd": b.weekly_limit_usd,
                "monthly_limit_usd": b.monthly_limit_usd,
                "warning_threshold": b.warning_threshold,
                "daily_spent_usd": daily_spent,
                "weekly_spent_usd": weekly_spent,
                "monthly_spent_usd": monthly_spent,
            })
        })
        .collect();
    json_ok(json!({ "budgets": enriched }))
}

pub async fn upsert_budget(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<DomainBudgetRow>,
) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    if body.api_key_id.trim().is_empty() {
        return json_status(
            StatusCode::BAD_REQUEST,
            json!({"error": "api_key_id is required (use 'global' for gateway-wide cap)"}),
        );
    }
    match state.store.upsert_domain_budget(&body) {
        Ok(()) => json_status(StatusCode::CREATED, json!({ "ok": true, "budget": body })),
        Err(e) => json_status(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e.to_string()})),
    }
}

pub async fn delete_budget(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    let deleted = state.store.delete_domain_budget(&id).unwrap_or(false);
    json_ok(json!({ "deleted": deleted, "api_key_id": id }))
}

// ── 3. Live Quota Monitor & OAuth Model Auto-Sync ───────────────────

#[derive(Debug, Deserialize, Default)]
pub struct QuotaQuery {
    pub refresh: Option<bool>,
}

async fn probe_antigravity_models_and_quota(
    client: &reqwest::Client,
    cred: &CredentialRow,
) -> Result<(Vec<String>, Vec<Value>), String> {
    let project_id = cred
        .provider_specific_json
        .get("project_id")
        .or_else(|| cred.provider_specific_json.get("projectId"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let ua = antigravity_content_user_agent(&cred.provider, &cred.provider_specific_json);
    let is_cli = cred.provider.eq_ignore_ascii_case("agy")
        || cred
            .provider_specific_json
            .get("clientProfile")
            .and_then(Value::as_str)
            .map(|s| s.eq_ignore_ascii_case("cli"))
            .unwrap_or(false);
    let mut hdrs = vec![("User-Agent".to_string(), ua)];
    if !project_id.is_empty() && !is_cli {
        hdrs.push(("x-goog-user-project".to_string(), project_id.to_string()));
    }
    scrub_proxy_and_fingerprint_headers(&mut hdrs);

    let payload = if project_id.is_empty() {
        json!({})
    } else {
        json!({ "project": project_id })
    };

    let mut last_err = String::from("no Antigravity endpoint succeeded");
    for base in ANTIGRAVITY_DISCOVERY_ENDPOINTS {
        let url = format!("{base}/v1internal:fetchAvailableModels");
        let mut req = client
            .post(&url)
            .timeout(Duration::from_secs(12))
            .header("Content-Type", "application/json");
        for (k, v) in &hdrs {
            req = req.header(k.as_str(), v.as_str());
        }
        req = req.header("Authorization", format!("Bearer {}", cred.api_key));

        match req.json(&payload).send().await {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await.unwrap_or_else(|_| json!({}));
                let mut models = Vec::new();
                let mut quotas = Vec::new();
                if let Some(map) = body.get("models").and_then(Value::as_object) {
                    for (mid, info) in map {
                        models.push(mid.clone());
                        if let Some(qi) = info.get("quotaInfo") {
                            quotas.push(json!({
                                "model": mid,
                                "family": omni_exec::antigravity::get_antigravity_quota_family(mid).as_str(),
                                "remaining_fraction": qi.get("remainingFraction").and_then(Value::as_f64),
                                "reset_time": qi.get("resetTime").and_then(Value::as_str),
                            }));
                        }
                    }
                }
                models.sort();
                return Ok((models, quotas));
            }
            Ok(resp) => {
                let st = resp.status().as_u16();
                let txt = resp.text().await.unwrap_or_default();
                last_err = format!("HTTP {st}: {}", txt.chars().take(180).collect::<String>());
                if st == 401 || st == 403 {
                    break;
                }
            }
            Err(e) => {
                last_err = e.to_string();
            }
        }
    }
    Err(last_err)
}

async fn probe_provider_live_models(
    state: &SharedState,
    cred: &CredentialRow,
) -> Result<(Vec<String>, Vec<Value>), String> {
    let fresh = ensure_fresh_oauth_credential(state, cred).await;
    let client = state.http_client_for(&fresh.provider, Some(&fresh));

    if is_antigravity_provider(&fresh.provider) {
        return probe_antigravity_models_and_quota(&client, &fresh).await;
    }

    if matches!(fresh.provider.as_str(), "github" | "ghe-copilot") {
        let mut ctok = fresh
            .provider_specific_json
            .get("copilot_token")
            .and_then(Value::as_str)
            .map(String::from);
        let mut api_base = fresh
            .provider_specific_json
            .get("copilot_api_endpoint")
            .and_then(Value::as_str)
            .unwrap_or("https://api.githubcopilot.com")
            .trim_end_matches('/')
            .to_string();
        if ctok.is_none() && !fresh.api_key.is_empty() {
            if let Ok(ex) = exchange_copilot_token(&client, &fresh.api_key, None).await {
                ctok = Some(ex.token);
                if let Some(ep) = ex.api_endpoint {
                    api_base = ep.trim_end_matches('/').to_string();
                }
            }
        }
        let bearer = ctok.unwrap_or_else(|| fresh.api_key.clone());
        let resp = client
            .get(format!("{api_base}/models"))
            .timeout(Duration::from_secs(12))
            .header("Authorization", format!("Bearer {bearer}"))
            .header("Editor-Version", GITHUB_COPILOT_EDITOR_VERSION)
            .header("Editor-Plugin-Version", GITHUB_COPILOT_PLUGIN_VERSION)
            .header("User-Agent", GITHUB_COPILOT_USER_AGENT)
            .header("X-GitHub-Api-Version", GITHUB_COPILOT_API_VERSION)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("Copilot /models HTTP {}", resp.status().as_u16()));
        }
        let v: Value = resp.json().await.unwrap_or_else(|_| json!({}));
        let mut models: Vec<String> = v
            .get("data")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m.get("id").and_then(Value::as_str).map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        models.sort();
        models.dedup();
        return Ok((models, vec![]));
    }

    // Generic OpenAI-compatible `/models` discovery
    let base_url = {
        let cat = state.catalog.read().unwrap();
        fresh
            .base_urls
            .first()
            .cloned()
            .or_else(|| cat.find(&fresh.provider).and_then(|p| p.base_urls.first().cloned()))
            .unwrap_or_default()
    };
    if base_url.is_empty() {
        return Err("no base_url available for model sync".into());
    }
    let models_url = {
        let trimmed = base_url
            .trim_end_matches('/')
            .trim_end_matches("/chat/completions")
            .trim_end_matches("/messages")
            .trim_end_matches("/responses")
            .trim_end_matches('/');
        format!("{trimmed}/models")
    };
    let mut req = client.get(&models_url).timeout(Duration::from_secs(12));
    if !fresh.api_key.is_empty() {
        if fresh.auth_type == "x-api-key" || fresh.provider == "anthropic" {
            req = req
                .header("x-api-key", &fresh.api_key)
                .header("anthropic-version", "2023-06-01");
        } else {
            req = req.header("Authorization", format!("Bearer {}", fresh.api_key));
        }
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("GET {models_url} returned HTTP {}", resp.status().as_u16()));
    }
    let v: Value = resp.json().await.unwrap_or_else(|_| json!({}));
    let mut models: Vec<String> = v
        .get("data")
        .or_else(|| v.get("models"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    m.get("id")
                        .or_else(|| m.get("name"))
                        .and_then(Value::as_str)
                        .map(String::from)
                })
                .collect()
        })
        .unwrap_or_default();
    models.sort();
    models.dedup();
    Ok((models, vec![]))
}

pub async fn get_quota_monitor(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(q): Query<QuotaQuery>,
) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    let now = now_ms();
    let creds = state.store.list_credentials().unwrap_or_default();
    let lockouts = omni_exec::resilience::all_lockouts();
    let do_refresh = q.refresh.unwrap_or(false);

    let mut accounts = Vec::new();
    for mut cred in creds {
        let mut live_error: Option<String> = None;
        if do_refresh && cred.enabled && !cred.api_key.is_empty() {
            match probe_provider_live_models(&state, &cred).await {
                Ok((models, quotas)) => {
                    let mut psj = cred.provider_specific_json.clone();
                    if !psj.is_object() {
                        psj = json!({});
                    }
                    psj["synced_models"] = json!(models);
                    psj["synced_at"] = json!(now);
                    if !quotas.is_empty() {
                        psj["quota_snapshot"] = json!(quotas);
                    }
                    cred.models = models;
                    cred.provider_specific_json = psj;
                    let _ = state.store.upsert_credential_full(&cred);
                }
                Err(e) => {
                    live_error = Some(e);
                }
            }
        }

        let active_locks: Vec<Value> = lockouts
            .iter()
            .filter(|l| l.provider == cred.provider && l.connection_id == cred.connection_id)
            .map(|l| {
                let rem = l
                    .until
                    .checked_duration_since(Instant::now())
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                json!({
                    "scope": l.model,
                    "reason": l.reason,
                    "remaining_ms": rem,
                    "failure_count": l.failure_count,
                })
            })
            .collect();

        // Compute family quota summary for Antigravity / agy
        let family_status = if is_antigravity_provider(&cred.provider) {
            let mut fams = Vec::new();
            for fam in ["gemini", "claude", "argon"] {
                let scope_key = format!("family:{fam}");
                let locked_ms = active_locks
                    .iter()
                    .find(|l| l.get("scope").and_then(Value::as_str) == Some(scope_key.as_str()))
                    .and_then(|l| l.get("remaining_ms").and_then(Value::as_u64));
                let live_frac = cred
                    .provider_specific_json
                    .get("quota_snapshot")
                    .and_then(Value::as_array)
                    .and_then(|arr| {
                        let vals: Vec<f64> = arr
                            .iter()
                            .filter(|q| q.get("family").and_then(Value::as_str) == Some(fam))
                            .filter_map(|q| q.get("remaining_fraction").and_then(Value::as_f64))
                            .collect();
                        if vals.is_empty() {
                            None
                        } else {
                            Some(vals.iter().sum::<f64>() / (vals.len() as f64))
                        }
                    });
                fams.push(json!({
                    "family": fam,
                    "locked_remaining_ms": locked_ms.unwrap_or(0),
                    "remaining_fraction": live_frac,
                    "status": if locked_ms.unwrap_or(0) > 0 { "cooldown" } else { "available" },
                }));
            }
            Some(fams)
        } else {
            None
        };

        let expires_in_ms = cred.expires_at.map(|e| e - now);
        let proxy = state.resolve_proxy_for(&cred.provider, Some(&cred));
        accounts.push(json!({
            "provider": cred.provider,
            "connection_id": cred.connection_id,
            "label": cred.label,
            "auth_type": cred.auth_type,
            "enabled": cred.enabled,
            "health_status": cred.health_status,
            "expires_at": cred.expires_at,
            "expires_in_ms": expires_in_ms,
            "proxy_url": proxy,
            "synced_models_count": cred.provider_specific_json.get("synced_models").and_then(Value::as_array).map(|a| a.len()).unwrap_or(cred.models.len()),
            "synced_models": cred.provider_specific_json.get("synced_models").cloned().unwrap_or_else(|| json!(cred.models)),
            "synced_at": cred.provider_specific_json.get("synced_at"),
            "quota_snapshot": cred.provider_specific_json.get("quota_snapshot").cloned().unwrap_or_else(|| json!([])),
            "families": family_status,
            "active_lockouts": active_locks,
            "live_error": live_error,
        }));
    }

    json_ok(json!({
        "timestamp_ms": now,
        "accounts": accounts,
    }))
}

pub async fn sync_provider_models(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((provider, connection_id)): Path<(String, String)>,
) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    let Ok(Some(mut cred)) = state.store.get_credential(&provider, &connection_id) else {
        return json_status(StatusCode::NOT_FOUND, json!({"error": "credential not found"}));
    };
    let now = now_ms();
    match probe_provider_live_models(&state, &cred).await {
        Ok((models, quotas)) => {
            let mut psj = cred.provider_specific_json.clone();
            if !psj.is_object() {
                psj = json!({});
            }
            psj["synced_models"] = json!(models);
            psj["synced_at"] = json!(now);
            if !quotas.is_empty() {
                psj["quota_snapshot"] = json!(quotas);
            }
            cred.models = models.clone();
            cred.provider_specific_json = psj;
            let _ = state.store.upsert_credential_full(&cred);
            json_ok(json!({
                "ok": true,
                "provider": provider,
                "connection_id": connection_id,
                "synced_at": now,
                "models_count": models.len(),
                "models": models,
                "quota_snapshot": quotas,
            }))
        }
        Err(e) => json_status(
            StatusCode::BAD_GATEWAY,
            json!({
                "ok": false,
                "provider": provider,
                "connection_id": connection_id,
                "error": e,
            }),
        ),
    }
}

// ── 4. Model Aliases, CLI Tools Auto-Configurator & OpenAPI 3.1 ─────

pub async fn list_aliases(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    let mut rows: Vec<(String, String)> = state
        .store
        .list_model_aliases()
        .unwrap_or_default()
        .into_iter()
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    let aliases: Vec<Value> = rows
        .into_iter()
        .map(|(alias, target)| json!({ "alias": alias, "target": target }))
        .collect();
    json_ok(json!({ "count": aliases.len(), "aliases": aliases }))
}

#[derive(Debug, Deserialize)]
pub struct UpsertAliasBody {
    pub alias: String,
    pub target: String,
}

pub async fn upsert_alias(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<UpsertAliasBody>,
) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    let alias = body.alias.trim();
    let target = body.target.trim();
    if alias.is_empty() || target.is_empty() {
        return json_status(
            StatusCode::BAD_REQUEST,
            json!({"error": "both alias and target are required"}),
        );
    }
    if alias.eq_ignore_ascii_case(target) {
        return json_status(
            StatusCode::BAD_REQUEST,
            json!({"error": "alias cannot point to itself"}),
        );
    }
    match state.store.set_model_alias(alias, target) {
        Ok(()) => json_status(StatusCode::CREATED, json!({"ok": true, "alias": alias, "target": target})),
        Err(e) => json_status(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e.to_string()})),
    }
}

pub async fn delete_alias(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(alias): Path<String>,
) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    let deleted = state.store.delete_model_alias(&alias).unwrap_or(false);
    json_ok(json!({ "deleted": deleted, "alias": alias }))
}

pub async fn get_cli_tools_config(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Response {
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("127.0.0.1:20128");
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("http");
    let base_url = format!("{scheme}://{host}");
    let v1_url = format!("{base_url}/v1");

    let has_keys = state.store.has_api_keys().unwrap_or(false);
    let caller_authed = authorize_management(&state, &headers).is_ok();
    let active_key = if !has_keys {
        "omniroute-local".to_string()
    } else if caller_authed {
        crate::chat::extract_api_key(&headers)
            .or_else(|| {
                state
                    .store
                    .list_api_key_rows()
                    .ok()
                    .and_then(|rows| rows.into_iter().find(|r| !r.disabled).map(|r| r.key))
            })
            .unwrap_or_else(|| "YOUR_OMNIROUTE_API_KEY".to_string())
    } else {
        "YOUR_OMNIROUTE_API_KEY".to_string()
    };
    let key_placeholder = active_key.as_str();

    // Pick a smart default model from configured credentials or combos
    let default_model = state
        .store
        .list_combos()
        .ok()
        .and_then(|c| c.into_iter().find(|x| x.enabled).map(|x| x.name))
        .or_else(|| {
            state.store.list_credentials().ok().and_then(|creds| {
                creds.into_iter().find(|c| c.enabled).map(|c| {
                    let m = c
                        .models
                        .first()
                        .cloned()
                        .unwrap_or_else(|| match c.provider.as_str() {
                            "antigravity" | "agy" => "claude-sonnet-4-6".into(),
                            "claude" | "anthropic" => "claude-sonnet-4-5".into(),
                            "codex" | "openai" => "gpt-5.3-codex".into(),
                            "github" => "claude-sonnet-4-5".into(),
                            _ => "gpt-4o".into(),
                        });
                    format!("{}/{}", c.provider, m)
                })
            })
        })
        .unwrap_or_else(|| "antigravity/claude-sonnet-4-6".into());

    let tools = vec![
        json!({
            "id": "claude-code",
            "name": "Claude Code",
            "provider_icon": "claude",
            "description": "Route Anthropic Claude Code CLI through OmniRoute (/v1/messages native surface)",
            "config_path": "~/.claude/settings.json",
            "format": "bash",
            "snippet": format!(
                "export ANTHROPIC_BASE_URL=\"{base_url}\"\nexport ANTHROPIC_AUTH_TOKEN=\"{key_placeholder}\"\nexport ANTHROPIC_MODEL=\"{default_model}\"\nclaude"
            ),
            "json_snippet": json!({
                "env": {
                    "ANTHROPIC_BASE_URL": base_url,
                    "ANTHROPIC_AUTH_TOKEN": key_placeholder,
                    "ANTHROPIC_MODEL": default_model
                }
            }),
        }),
        json!({
            "id": "codex-cli",
            "name": "OpenAI Codex CLI",
            "provider_icon": "codex",
            "description": "Use OpenAI Codex CLI with OmniRoute Responses API (/v1/responses)",
            "config_path": "~/.codex/config.toml",
            "format": "toml",
            "snippet": format!(
                "model = \"{default_model}\"\nmodel_provider = \"omniroute\"\n\n[model_providers.omniroute]\nname = \"OmniRoute RS\"\nbase_url = \"{v1_url}\"\nenv_key = \"OMNIROUTE_API_KEY\"\nwire_api = \"responses\""
            ),
        }),
        json!({
            "id": "cline",
            "name": "Cline / RooCode",
            "provider_icon": "cline",
            "description": "VS Code OpenAI-Compatible API provider preset for Cline & Roo Code",
            "config_path": "VS Code -> Cline Settings -> API Provider: OpenAI Compatible",
            "format": "json",
            "snippet": serde_json::to_string_pretty(&json!({
                "apiProvider": "openai",
                "openAiBaseUrl": v1_url,
                "openAiApiKey": key_placeholder,
                "openAiModelId": default_model
            })).unwrap_or_default(),
        }),
        json!({
            "id": "continue",
            "name": "Continue.dev",
            "provider_icon": "openai",
            "description": "Drop-in config.yaml model block for Continue IDE extension",
            "config_path": "~/.continue/config.yaml",
            "format": "yaml",
            "snippet": format!(
                "models:\n  - name: OmniRoute ({default_model})\n    provider: openai\n    model: {default_model}\n    apiBase: {v1_url}\n    apiKey: {key_placeholder}"
            ),
        }),
        json!({
            "id": "aider",
            "name": "Aider",
            "provider_icon": "openai",
            "description": "Run Aider AI pair programmer pointing directly to OmniRoute",
            "config_path": "Shell / .aider.conf.yml",
            "format": "bash",
            "snippet": format!(
                "export OPENAI_API_BASE=\"{v1_url}\"\nexport OPENAI_API_KEY=\"{key_placeholder}\"\naider --model \"openai/{default_model}\""
            ),
        }),
        json!({
            "id": "opencode",
            "name": "OpenCode",
            "provider_icon": "opencode",
            "description": "OpenCode provider configuration pointing at OmniRoute gateway",
            "config_path": "~/.config/opencode/opencode.json",
            "format": "json",
            "snippet": serde_json::to_string_pretty(&json!({
                "$schema": "https://opencode.ai/config.json",
                "provider": {
                    "omniroute": {
                        "npm": "@ai-sdk/openai-compatible",
                        "options": {
                            "baseURL": v1_url,
                            "apiKey": key_placeholder
                        },
                        "models": {
                            default_model.clone(): { "name": default_model }
                        }
                    }
                }
            })).unwrap_or_default(),
        }),
        json!({
            "id": "cursor",
            "name": "Cursor IDE",
            "provider_icon": "cursor",
            "description": "Override OpenAI Base URL inside Cursor Settings -> Models",
            "config_path": "Cursor Settings -> Models -> Override OpenAI Base URL",
            "format": "text",
            "snippet": format!(
                "Override OpenAI Base URL: {v1_url}\nOpenAI API Key:           {key_placeholder}\nCustom Model Name:        {default_model}"
            ),
        }),
    ];

    json_ok(json!({
        "base_url": base_url,
        "v1_url": v1_url,
        "default_model": default_model,
        "strict_auth_enabled": has_keys,
        "tools": tools,
    }))
}

pub async fn get_openapi_spec(headers: HeaderMap) -> Response {
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("127.0.0.1:20128");
    let spec = json!({
        "openapi": "3.1.0",
        "info": {
            "title": "OmniRoute RS AI Gateway API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Native Rust multi-provider AI gateway (upstream OmniRoute v3.8.55 parity) supporting OpenAI, Anthropic, Responses, Gemini, Files, Batches, A2A, MCP, OAuth, Proxies, Analytics, and Guardrails."
        },
        "servers": [{ "url": format!("http://{host}") }],
        "components": {
            "securitySchemes": {
                "BearerAuth": { "type": "http", "scheme": "bearer" },
                "SessionCookie": { "type": "apiKey", "in": "cookie", "name": SESSION_COOKIE_NAME }
            }
        },
        "paths": {
            "/v1/chat/completions": { "post": { "tags": ["Inference"], "summary": "OpenAI Chat Completions (stream & non-stream)" } },
            "/v1/messages": { "post": { "tags": ["Inference"], "summary": "Anthropic Messages API" } },
            "/v1/responses": { "post": { "tags": ["Inference"], "summary": "OpenAI Responses API" } },
            "/v1/completions": { "post": { "tags": ["Inference"], "summary": "Legacy OpenAI Completions" } },
            "/v1beta/models/{spec}": { "post": { "tags": ["Inference"], "summary": "Native Gemini generateContent / streamGenerateContent" } },
            "/v1/embeddings": { "post": { "tags": ["Multimodal"], "summary": "Embeddings" } },
            "/v1/images/generations": { "post": { "tags": ["Multimodal"], "summary": "Image Generations" } },
            "/v1/audio/speech": { "post": { "tags": ["Multimodal"], "summary": "Text-to-Speech" } },
            "/v1/audio/transcriptions": { "post": { "tags": ["Multimodal"], "summary": "Speech-to-Text" } },
            "/v1/rerank": { "post": { "tags": ["Multimodal"], "summary": "Document Reranking" } },
            "/v1/moderations": { "post": { "tags": ["Multimodal"], "summary": "Content Moderation" } },
            "/v1/decisions": { "post": { "tags": ["Inference"], "summary": "OpenAI Decisions API (gpt-6-luna)" } },
            "/v1/files": { "get": { "tags": ["Batches & Files"], "summary": "List uploaded files" }, "post": { "tags": ["Batches & Files"], "summary": "Upload JSONL/text file" } },
            "/v1/batches": { "get": { "tags": ["Batches & Files"], "summary": "List batch jobs" }, "post": { "tags": ["Batches & Files"], "summary": "Create & run async batch job" } },
            "/v1/models": { "get": { "tags": ["Discovery"], "summary": "List catalog & combo models" } },
            "/api/health": { "get": { "tags": ["System"], "summary": "Gateway health & subsystem summary" } },
            "/api/proxies": { "get": { "tags": ["Proxies"], "summary": "Get HTTP/SOCKS5 upstream proxy config" }, "put": { "tags": ["Proxies"], "summary": "Update proxy config & pool" } },
            "/api/proxies/test": { "post": { "tags": ["Proxies"], "summary": "Live-test an HTTP/SOCKS5 proxy URL" } },
            "/api/quota": { "get": { "tags": ["Quota"], "summary": "Live quota & Antigravity family cooldown monitor" } },
            "/api/providers/{provider}/{connection_id}/sync-models": { "post": { "tags": ["Quota"], "summary": "Sync live available models & quota snapshot from upstream" } },
            "/api/logs": { "get": { "tags": ["Observability"], "summary": "Query call logs" }, "delete": { "tags": ["Observability"], "summary": "Clear call logs" } },
            "/api/logs/export": { "get": { "tags": ["Observability"], "summary": "Export call logs as CSV or JSONL" } },
            "/api/analytics": { "get": { "tags": ["Observability"], "summary": "Daily timeseries, p50/p95 latency & model breakdown" } },
            "/api/budgets": { "get": { "tags": ["Budgets"], "summary": "List daily/weekly/monthly spend caps" }, "post": { "tags": ["Budgets"], "summary": "Upsert spend cap" } },
            "/api/aliases": { "get": { "tags": ["Routing"], "summary": "List model aliases" }, "post": { "tags": ["Routing"], "summary": "Create or update model alias" } },
            "/api/cli-tools": { "get": { "tags": ["DX"], "summary": "Generate ready-to-paste configs for Claude Code, Codex CLI, Cline, Continue, Aider, OpenCode, Cursor" } },
            "/api/auth/status": { "get": { "tags": ["Auth"], "summary": "Check dashboard password lock & session state" } },
            "/api/auth/setup": { "post": { "tags": ["Auth"], "summary": "Set or change dashboard password" } },
            "/api/auth/login": { "post": { "tags": ["Auth"], "summary": "Login and receive signed HttpOnly session cookie" } },
            "/api/alerts/config": { "get": { "tags": ["Alerting"], "summary": "Get Webhook & Telegram alerting config" }, "put": { "tags": ["Alerting"], "summary": "Update alerting config" } },
            "/api/alerts/test": { "post": { "tags": ["Alerting"], "summary": "Send a live test alert to Webhook & Telegram" } }
        }
    });
    json_ok(spec)
}

// ── 5. Dashboard Password Auth (/api/auth/*) ────────────────────────

pub async fn auth_status(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    let locked = state.is_dashboard_locked();
    let authenticated = has_valid_dashboard_session(&state, &headers);
    let has_api_keys = state.store.has_api_keys().unwrap_or(false);
    json_ok(json!({
        "locked": locked,
        "authenticated": authenticated,
        "has_api_keys": has_api_keys,
    }))
}

#[derive(Debug, Deserialize)]
pub struct AuthSetupBody {
    pub password: String,
    pub current_password: Option<String>,
}

fn build_session_cookie_header(token: &str) -> String {
    format!("{SESSION_COOKIE_NAME}={token}; HttpOnly; SameSite=Lax; Path=/; Max-Age=604800")
}

fn clear_session_cookie_header() -> String {
    format!("{SESSION_COOKIE_NAME}=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0")
}

pub async fn auth_setup(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<AuthSetupBody>,
) -> Response {
    if state.is_dashboard_locked() {
        let cur_ok = body
            .current_password
            .as_deref()
            .map(|p| state.verify_dashboard_password(p))
            .unwrap_or(false);
        if !cur_ok && !has_valid_dashboard_session(&state, &headers) {
            return json_status(
                StatusCode::UNAUTHORIZED,
                json!({"error": "Current password or active session required to change password"}),
            );
        }
    }
    let pw = body.password.trim();
    if pw.len() < 4 {
        return json_status(
            StatusCode::BAD_REQUEST,
            json!({"error": "Password must be at least 4 characters"}),
        );
    }
    let salt = uuid::Uuid::new_v4().simple().to_string();
    let session_secret = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let hash = AppState::hash_dashboard_password(&salt, pw);
    let cfg = DashboardAuthConfig {
        enabled: true,
        salt,
        password_hash: hash,
        session_secret,
        updated_at: now_ms(),
    };
    let serialized = serde_json::to_string(&cfg).unwrap_or_else(|_| "{}".into());
    if let Err(e) = state
        .store
        .set_setting("dashboard_auth_config", &serialized, now_ms())
    {
        return json_status(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e.to_string()}));
    }
    let token = state.mint_dashboard_session_token().unwrap_or_default();
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::SET_COOKIE, build_session_cookie_header(&token))
        .body(Body::from(
            json!({"ok": true, "locked": true, "authenticated": true}).to_string(),
        ))
        .unwrap()
}

#[derive(Debug, Deserialize)]
pub struct AuthLoginBody {
    pub password: String,
}

pub async fn auth_login(
    State(state): State<SharedState>,
    axum::Json(body): axum::Json<AuthLoginBody>,
) -> Response {
    if !state.is_dashboard_locked() {
        return json_ok(json!({"ok": true, "locked": false, "authenticated": true}));
    }
    if !state.verify_dashboard_password(&body.password) {
        return json_status(
            StatusCode::UNAUTHORIZED,
            json!({"ok": false, "error": "Invalid dashboard password"}),
        );
    }
    let token = state.mint_dashboard_session_token().unwrap_or_default();
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::SET_COOKIE, build_session_cookie_header(&token))
        .body(Body::from(
            json!({"ok": true, "locked": true, "authenticated": true}).to_string(),
        ))
        .unwrap()
}

pub async fn auth_logout() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::SET_COOKIE, clear_session_cookie_header())
        .body(Body::from(json!({"ok": true, "authenticated": false}).to_string()))
        .unwrap()
}

pub async fn auth_disable(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    let _ = state.store.delete_setting("dashboard_auth_config");
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::SET_COOKIE, clear_session_cookie_header())
        .body(Body::from(json!({"ok": true, "locked": false}).to_string()))
        .unwrap()
}

// ── 6. Webhook & Telegram Bot Alerting (/api/alerts/*) ──────────────

pub async fn get_alerts_config(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    let mut cfg = state.alerting_config();
    let has_tg_token = !cfg.telegram_bot_token.is_empty();
    let has_secret = !cfg.webhook_secret.is_empty();
    if cfg.telegram_bot_token.len() > 8 {
        cfg.telegram_bot_token = format!(
            "{}...{}",
            &cfg.telegram_bot_token[..4],
            &cfg.telegram_bot_token[cfg.telegram_bot_token.len() - 4..]
        );
    }
    if has_secret {
        cfg.webhook_secret = "***".into();
    }
    json_ok(json!({
        "config": cfg,
        "has_telegram_token": has_tg_token,
        "has_webhook_secret": has_secret,
    }))
}

pub async fn put_alerts_config(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(mut body): axum::Json<AlertingConfig>,
) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    // Preserve masked secrets if unchanged by UI
    let existing = state.alerting_config();
    if body.telegram_bot_token.contains("...") && !existing.telegram_bot_token.is_empty() {
        body.telegram_bot_token = existing.telegram_bot_token;
    }
    if body.webhook_secret == "***" && !existing.webhook_secret.is_empty() {
        body.webhook_secret = existing.webhook_secret;
    }
    if !body.webhook_url.trim().is_empty()
        && !body.webhook_url.starts_with("http://")
        && !body.webhook_url.starts_with("https://")
    {
        return json_status(
            StatusCode::BAD_REQUEST,
            json!({"error": "webhook_url must start with http:// or https://"}),
        );
    }
    let serialized = serde_json::to_string(&body).unwrap_or_else(|_| "{}".into());
    if let Err(e) = state
        .store
        .set_setting("alerting_config", &serialized, now_ms())
    {
        return json_status(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e.to_string()}));
    }
    get_alerts_config(State(state), headers).await
}

async fn send_alert_now(
    client: &reqwest::Client,
    cfg: &AlertingConfig,
    kind: &str,
    title: &str,
    message: &str,
    metadata: &Value,
) -> Value {
    let mut results = json!({
        "webhook": Value::Null,
        "telegram": Value::Null,
    });

    if !cfg.webhook_url.trim().is_empty() {
        let payload = json!({
            "source": "omniroute-rs",
            "event": kind,
            "title": title,
            "message": message,
            "timestamp": Utc::now().to_rfc3339(),
            "metadata": metadata,
        });
        let raw = payload.to_string();
        let mut req = client
            .post(cfg.webhook_url.trim())
            .timeout(Duration::from_secs(10))
            .header("Content-Type", "application/json")
            .body(raw.clone());
        if !cfg.webhook_secret.is_empty() {
            let sig = hex::encode(hmac_sha256(cfg.webhook_secret.as_bytes(), raw.as_bytes()));
            req = req.header("X-OmniRoute-Signature", format!("sha256={sig}"));
        }
        results["webhook"] = match req.send().await {
            Ok(r) => json!({ "ok": r.status().is_success(), "status": r.status().as_u16() }),
            Err(e) => json!({ "ok": false, "error": e.to_string() }),
        };
    }

    if !cfg.telegram_bot_token.trim().is_empty() && !cfg.telegram_chat_id.trim().is_empty() {
        let tg_url = format!(
            "https://api.telegram.org/bot{}/sendMessage",
            cfg.telegram_bot_token.trim()
        );
        let text = format!("[OmniRoute RS] {title}\nEvent: {kind}\n{message}");
        let res = client
            .post(&tg_url)
            .timeout(Duration::from_secs(10))
            .json(&json!({
                "chat_id": cfg.telegram_chat_id.trim(),
                "text": text,
                "disable_web_page_preview": true
            }))
            .send()
            .await;
        results["telegram"] = match res {
            Ok(r) => json!({ "ok": r.status().is_success(), "status": r.status().as_u16() }),
            Err(e) => json!({ "ok": false, "error": e.to_string() }),
        };
    }

    results
}

pub async fn test_alert(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    let cfg = state.alerting_config();
    if cfg.webhook_url.trim().is_empty()
        && (cfg.telegram_bot_token.trim().is_empty() || cfg.telegram_chat_id.trim().is_empty())
    {
        return json_status(
            StatusCode::BAD_REQUEST,
            json!({"ok": false, "error": "Configure a Webhook URL or Telegram Bot Token + Chat ID first"}),
        );
    }
    let res = send_alert_now(
        &state.dispatcher.client,
        &cfg,
        "test",
        "OmniRoute RS Test Alert",
        "Your Webhook / Telegram alerting pipeline is connected and operational.",
        &json!({"version": env!("CARGO_PKG_VERSION")}),
    )
    .await;
    json_ok(json!({ "ok": true, "results": res }))
}

/// Fire-and-forget deduplicated alert dispatcher triggered by OAuth needs_reauth,
/// circuit breaker trips, or hard budget warnings/breaches.
pub fn dispatch_alert_async(
    state: SharedState,
    kind: &'static str,
    title: String,
    message: String,
    metadata: Value,
) {
    let cfg = state.alerting_config();
    if !cfg.enabled {
        return;
    }
    let allowed = match kind {
        "needs_reauth" => cfg.on_needs_reauth,
        "breaker_open" => cfg.on_breaker_open,
        "budget_warning" | "budget_exceeded" => cfg.on_budget_alert,
        _ => true,
    };
    if !allowed {
        return;
    }
    if cfg.webhook_url.trim().is_empty()
        && (cfg.telegram_bot_token.trim().is_empty() || cfg.telegram_chat_id.trim().is_empty())
    {
        return;
    }
    // 5-minute deduplication per (kind, title)
    let now = now_ms();
    {
        let mut dedup = state.alert_dedup.lock().unwrap();
        let key = format!("{kind}::{title}");
        if let Some(&last_ts) = dedup.get(&key) {
            if now - last_ts < ALERT_DEDUP_WINDOW_MS {
                return;
            }
        }
        if dedup.len() > 256 {
            dedup.retain(|_, ts| now - *ts < ALERT_DEDUP_WINDOW_MS);
        }
        dedup.insert(key, now);
    }

    let client = state.dispatcher.client.clone();
    tokio::spawn(async move {
        let _ = send_alert_now(&client, &cfg, kind, &title, &message, &metadata).await;
    });
}

// ── 7. OpenAI /v1/files & Async /v1/batches ─────────────────────────

fn format_file_json(f: &OpenAiFileRow) -> Value {
    json!({
        "id": f.id,
        "object": "file",
        "bytes": f.bytes,
        "created_at": f.created_at / 1000,
        "filename": f.filename,
        "purpose": f.purpose,
        "status": "processed"
    })
}

fn parse_multipart_file_upload(content_type: &str, body: &[u8]) -> Option<(String, String, Vec<u8>)> {
    let boundary = content_type
        .split(';')
        .find_map(|p| p.trim().strip_prefix("boundary="))?
        .trim_matches('"');
    let delim = format!("--{boundary}");
    let raw = String::from_utf8_lossy(body);
    let mut purpose = String::from("batch");
    let mut filename = String::from("upload.jsonl");
    let mut file_bytes: Option<Vec<u8>> = None;

    for section in raw.split(&delim).skip(1) {
        if section.starts_with("--") {
            break;
        }
        let sec = section.trim_start_matches("\r\n").trim_start_matches('\n');
        let Some((hdrs, payload_raw)) = sec
            .split_once("\r\n\r\n")
            .or_else(|| sec.split_once("\n\n"))
        else {
            continue;
        };
        let payload = payload_raw
            .strip_suffix("\r\n")
            .or_else(|| payload_raw.strip_suffix('\n'))
            .unwrap_or(payload_raw);
        if hdrs.contains("name=\"purpose\"") {
            purpose = payload.trim().to_string();
        } else if hdrs.contains("name=\"file\"") {
            if let Some(fn_part) = hdrs.split("filename=\"").nth(1) {
                if let Some(name) = fn_part.split('"').next() {
                    if !name.is_empty() {
                        filename = name.to_string();
                    }
                }
            }
            file_bytes = Some(payload.as_bytes().to_vec());
        }
    }
    file_bytes.map(|b| (filename, purpose, b))
}

pub async fn upload_file(
    State(state): State<SharedState>,
    headers: HeaderMap,
    raw_body: Bytes,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    if raw_body.len() > MAX_UPLOAD_FILE_BYTES {
        return format_error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            "File exceeds 8 MiB limit",
            false,
            None,
        );
    }
    let ct = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let (filename, purpose, content) = if ct.contains("multipart/form-data") {
        match parse_multipart_file_upload(ct, &raw_body) {
            Some(parsed) => parsed,
            None => {
                return format_error_response(
                    StatusCode::BAD_REQUEST,
                    "Invalid multipart/form-data upload: missing 'file' field",
                    false,
                    None,
                );
            }
        }
    } else if let Ok(v) = serde_json::from_slice::<Value>(&raw_body) {
        let filename = v
            .get("filename")
            .and_then(Value::as_str)
            .unwrap_or("batch.jsonl")
            .to_string();
        let purpose = v
            .get("purpose")
            .and_then(Value::as_str)
            .unwrap_or("batch")
            .to_string();
        let content = v
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .as_bytes()
            .to_vec();
        (filename, purpose, content)
    } else {
        ("upload.jsonl".into(), "batch".into(), raw_body.to_vec())
    };

    let row = OpenAiFileRow {
        id: format!("file-{}", &uuid::Uuid::new_v4().simple().to_string()[..16]),
        filename,
        purpose,
        bytes: content.len() as i64,
        created_at: now_ms(),
        content,
    };
    if let Err(e) = state.store.insert_openai_file(&row) {
        return format_error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string(), false, None);
    }
    json_response(StatusCode::OK, format_file_json(&row))
}

pub async fn list_files(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    let files = state.store.list_openai_files().unwrap_or_default();
    let data: Vec<Value> = files.iter().map(format_file_json).collect();
    json_response(StatusCode::OK, json!({ "object": "list", "data": data }))
}

pub async fn get_file(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    match state.store.get_openai_file(&id) {
        Ok(Some(f)) => json_response(StatusCode::OK, format_file_json(&f)),
        _ => format_error_response(StatusCode::NOT_FOUND, "File not found", false, None),
    }
}

pub async fn get_file_content(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    match state.store.get_openai_file(&id) {
        Ok(Some(f)) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(Body::from(f.content))
            .unwrap(),
        _ => format_error_response(StatusCode::NOT_FOUND, "File not found", false, None),
    }
}

pub async fn delete_file(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    let deleted = state.store.delete_openai_file(&id).unwrap_or(false);
    json_response(
        StatusCode::OK,
        json!({ "id": id, "object": "file", "deleted": deleted }),
    )
}

fn format_batch_json(b: &OpenAiBatchRow) -> Value {
    json!({
        "id": b.id,
        "object": "batch",
        "endpoint": b.endpoint,
        "input_file_id": b.input_file_id,
        "output_file_id": b.output_file_id,
        "error_file_id": b.error_file_id,
        "status": b.status,
        "created_at": b.created_at / 1000,
        "completed_at": b.completed_at.map(|t| t / 1000),
        "request_counts": {
            "total": b.total_requests,
            "completed": b.completed_requests,
            "failed": b.failed_requests
        },
        "metadata": b.metadata
    })
}

#[derive(Debug, Deserialize)]
pub struct CreateBatchBody {
    pub input_file_id: String,
    #[serde(default = "default_batch_endpoint")]
    pub endpoint: String,
    #[serde(default)]
    pub metadata: Value,
}

fn default_batch_endpoint() -> String {
    "/v1/chat/completions".into()
}

pub async fn create_batch(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<CreateBatchBody>,
) -> Response {
    let api_key = match authorize_request(&state, &headers, false) {
        Ok(k) => k,
        Err(resp) => return resp,
    };
    let Ok(Some(input_file)) = state.store.get_openai_file(&body.input_file_id) else {
        return format_error_response(StatusCode::NOT_FOUND, "input_file_id not found", false, None);
    };
    let text = String::from_utf8_lossy(&input_file.content).to_string();
    let lines: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    if lines.is_empty() {
        return format_error_response(StatusCode::BAD_REQUEST, "input file has no JSONL requests", false, None);
    }

    let batch_id = format!("batch_{}", &uuid::Uuid::new_v4().simple().to_string()[..16]);
    let mut row = OpenAiBatchRow {
        id: batch_id.clone(),
        input_file_id: body.input_file_id,
        output_file_id: None,
        error_file_id: None,
        endpoint: body.endpoint,
        status: "in_progress".into(),
        total_requests: lines.len() as i64,
        completed_requests: 0,
        failed_requests: 0,
        created_at: now_ms(),
        completed_at: None,
        metadata: if body.metadata.is_null() { json!({}) } else { body.metadata },
    };
    if let Err(e) = state.store.upsert_openai_batch(&row) {
        return format_error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string(), false, None);
    }
    let initial_json = format_batch_json(&row);

    // Execute batch lines asynchronously in background
    let st = state.clone();
    tokio::spawn(async move {
        let mut out_lines: Vec<String> = Vec::with_capacity(lines.len());
        let mut err_lines: Vec<String> = Vec::new();

        for (idx, line) in lines.iter().enumerate() {
            // Stop early if cancelled
            if let Ok(Some(cur)) = st.store.get_openai_batch(&batch_id) {
                if cur.status == "cancelled" {
                    return;
                }
            }
            let item: Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(e) => {
                    row.failed_requests += 1;
                    err_lines.push(
                        json!({
                            "id": format!("batch_req_{idx}"),
                            "custom_id": format!("line-{idx}"),
                            "error": { "code": "invalid_jsonl", "message": e.to_string() }
                        })
                        .to_string(),
                    );
                    continue;
                }
            };
            let custom_id = item
                .get("custom_id")
                .and_then(Value::as_str)
                .map(String::from)
                .unwrap_or_else(|| format!("request-{idx}"));
            let req_body = item.get("body").cloned().unwrap_or_else(|| item.clone());
            let mut chat_req: ChatRequest = match serde_json::from_value(req_body) {
                Ok(r) => r,
                Err(e) => {
                    row.failed_requests += 1;
                    err_lines.push(
                        json!({
                            "id": format!("batch_req_{idx}"),
                            "custom_id": custom_id,
                            "error": { "code": "invalid_request", "message": e.to_string() }
                        })
                        .to_string(),
                    );
                    continue;
                }
            };
            chat_req.stream = Some(false);
            let resp = execute_chat_pipeline(
                st.clone(),
                HeaderMap::new(),
                api_key.clone(),
                chat_req,
                ResponseSurface::OpenAiChat,
            )
            .await;
            let status_code = resp.status().as_u16();
            let bytes = axum::body::to_bytes(resp.into_body(), 4 * 1024 * 1024)
                .await
                .unwrap_or_default();
            let parsed_body: Value = serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| json!({ "raw": String::from_utf8_lossy(&bytes).to_string() }));
            if (200..300).contains(&status_code) {
                row.completed_requests += 1;
                out_lines.push(
                    json!({
                        "id": format!("batch_req_{idx}"),
                        "custom_id": custom_id,
                        "response": { "status_code": status_code, "body": parsed_body },
                        "error": Value::Null
                    })
                    .to_string(),
                );
            } else {
                row.failed_requests += 1;
                err_lines.push(
                    json!({
                        "id": format!("batch_req_{idx}"),
                        "custom_id": custom_id,
                        "response": { "status_code": status_code, "body": parsed_body },
                        "error": { "code": format!("http_{status_code}"), "message": parsed_body.to_string() }
                    })
                    .to_string(),
                );
            }
            let _ = st.store.upsert_openai_batch(&row);
        }

        if !out_lines.is_empty() {
            let content = out_lines.join("\n").into_bytes();
            let out_id = format!("file-out-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
            let _ = st.store.insert_openai_file(&OpenAiFileRow {
                id: out_id.clone(),
                filename: format!("{batch_id}_output.jsonl"),
                purpose: "batch_output".into(),
                bytes: content.len() as i64,
                created_at: now_ms(),
                content,
            });
            row.output_file_id = Some(out_id);
        }
        if !err_lines.is_empty() {
            let content = err_lines.join("\n").into_bytes();
            let err_id = format!("file-err-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
            let _ = st.store.insert_openai_file(&OpenAiFileRow {
                id: err_id.clone(),
                filename: format!("{batch_id}_errors.jsonl"),
                purpose: "batch_error".into(),
                bytes: content.len() as i64,
                created_at: now_ms(),
                content,
            });
            row.error_file_id = Some(err_id);
        }
        row.status = if row.completed_requests == 0 && row.failed_requests > 0 {
            "failed".into()
        } else {
            "completed".into()
        };
        row.completed_at = Some(now_ms());
        let _ = st.store.upsert_openai_batch(&row);
    });

    json_response(StatusCode::OK, initial_json)
}

pub async fn list_batches(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    let batches = state.store.list_openai_batches().unwrap_or_default();
    let data: Vec<Value> = batches.iter().map(format_batch_json).collect();
    json_response(StatusCode::OK, json!({ "object": "list", "data": data }))
}

pub async fn get_batch(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    match state.store.get_openai_batch(&id) {
        Ok(Some(b)) => json_response(StatusCode::OK, format_batch_json(&b)),
        _ => format_error_response(StatusCode::NOT_FOUND, "Batch not found", false, None),
    }
}

pub async fn cancel_batch(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    match state.store.get_openai_batch(&id) {
        Ok(Some(mut b)) => {
            if b.status == "in_progress" || b.status == "validating" {
                b.status = "cancelled".into();
                b.completed_at = Some(now_ms());
                let _ = state.store.upsert_openai_batch(&b);
            }
            json_response(StatusCode::OK, format_batch_json(&b))
        }
        _ => format_error_response(StatusCode::NOT_FOUND, "Batch not found", false, None),
    }
}

/// Helper called from `chat.rs` to persist a structured call log entry without crashing hot path.
#[allow(clippy::too_many_arguments)]
pub fn write_call_log(
    state: &SharedState,
    path: &str,
    status: u16,
    provider: &str,
    connection_id: &str,
    model: &str,
    duration_ms: i64,
    tokens_in: i64,
    tokens_out: i64,
    error_summary: Option<String>,
) {
    let _ = state.store.record_call_log(&CallLogRow {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: Utc::now().to_rfc3339(),
        method: "POST".into(),
        path: path.to_string(),
        status: status as i64,
        model: model.to_string(),
        provider: provider.to_string(),
        connection_id: connection_id.to_string(),
        duration: duration_ms,
        tokens_in,
        tokens_out,
        error_summary: error_summary.map(|s| s.chars().take(1024).collect()),
    });
}
