// OAuth 2.0 Management API (/api/oauth/*) + Background Token Refresh Scheduler
// + On-Demand Token & Copilot Minting helpers.

use crate::chat::authorize_request;
use crate::state::{now_ms, SharedState};
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::Response;
use omni_oauth::{
    build_authorize_url, exchange_authorization_code, exchange_copilot_token, find_oauth_provider,
    generate_pkce, healthcheck_per_provider_concurrency, healthcheck_skip_providers,
    is_healthcheck_disabled, poll_device_code, refresh_with_retry, start_device_code,
    DevicePollOutcome, OAuthFlowKind, PendingOAuthSession, RefreshOutcome, OAUTH_PROVIDERS,
};
use omni_store::CredentialRow;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;

const PROACTIVE_REFRESH_WINDOW_MS: i64 = 5 * 60 * 1000; // 5 minutes
const ON_DEMAND_EXPIRY_BUFFER_MS: i64 = 60 * 1000; // 60 seconds
const BG_SWEEP_INTERVAL_SECS: u64 = 60;

fn json_status(status: StatusCode, body: Value) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

// ── GET /api/oauth/providers ────────────────────────────────────────

pub async fn list_oauth_providers(State(state): State<SharedState>) -> Response {
    let items: Vec<Value> = OAUTH_PROVIDERS
        .iter()
        .map(|p| {
            json!({
                "id": p.id,
                "display_name": p.display_name,
                "flow": p.flow,
                "uses_pkce": p.uses_pkce,
                "authorize_url": p.authorize_url,
                "device_code_url": p.device_code_url,
                "default_redirect_uri": p.default_redirect_uri(),
                "scopes": p.scopes,
                "client_id_configured": !p.resolve_client_id().is_empty(),
            })
        })
        .collect();
    json_status(
        StatusCode::OK,
        json!({
            "providers": items,
            "status": state.oauth_summary(),
        }),
    )
}

// ── POST /api/oauth/{provider}/authorize ────────────────────────────

#[derive(Debug, Deserialize, Default)]
pub struct AuthorizeBody {
    pub connection_id: Option<String>,
    pub redirect_uri: Option<String>,
    pub client_id: Option<String>,
    pub token_url: Option<String>,
    pub device_code_url: Option<String>,
}

pub async fn start_oauth_authorize(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(provider): Path<String>,
    body: Option<axum::Json<AuthorizeBody>>,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    let Some(cfg) = find_oauth_provider(&provider) else {
        return json_status(
            StatusCode::NOT_FOUND,
            json!({"error": format!("unsupported OAuth provider: {provider}")}),
        );
    };
    let req = body.map(|b| b.0).unwrap_or_default();
    let conn_id = req
        .connection_id
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("{}-{}", cfg.id, &uuid::Uuid::new_v4().simple().to_string()[..8]));
    let now = now_ms();

    match cfg.flow {
        OAuthFlowKind::Pkce => {
            let pkce = generate_pkce();
            let redirect_uri = req
                .redirect_uri
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| cfg.default_redirect_uri());
            let authorize_url =
                build_authorize_url(cfg, &redirect_uri, &pkce, req.client_id.as_deref(), &[]);
            state.oauth_sessions.insert(
                PendingOAuthSession {
                    state: pkce.state.clone(),
                    provider: cfg.id.to_string(),
                    connection_id: conn_id.clone(),
                    redirect_uri: redirect_uri.clone(),
                    code_verifier: pkce.code_verifier.clone(),
                    client_id: req.client_id.unwrap_or_else(|| cfg.resolve_client_id()),
                    token_url_override: req.token_url,
                    device_code: None,
                    created_at_ms: now,
                },
                now,
            );
            json_status(
                StatusCode::OK,
                json!({
                    "flow": "pkce",
                    "provider": cfg.id,
                    "connection_id": conn_id,
                    "authorize_url": authorize_url,
                    "state": pkce.state,
                    "code_challenge": pkce.code_challenge,
                    "code_challenge_method": pkce.code_challenge_method,
                    "redirect_uri": redirect_uri,
                }),
            )
        }
        OAuthFlowKind::DeviceCode => {
            match start_device_code(
                &state.dispatcher.client,
                cfg,
                req.client_id.as_deref(),
                req.device_code_url.as_deref(),
            )
            .await
            {
                Ok(dc) => {
                    let state_tok = uuid::Uuid::new_v4().simple().to_string();
                    state.oauth_sessions.insert(
                        PendingOAuthSession {
                            state: state_tok.clone(),
                            provider: cfg.id.to_string(),
                            connection_id: conn_id.clone(),
                            redirect_uri: String::new(),
                            code_verifier: String::new(),
                            client_id: req.client_id.unwrap_or_else(|| cfg.resolve_client_id()),
                            token_url_override: req.token_url,
                            device_code: Some(dc.device_code.clone()),
                            created_at_ms: now,
                        },
                        now,
                    );
                    json_status(
                        StatusCode::OK,
                        json!({
                            "flow": "device_code",
                            "provider": cfg.id,
                            "connection_id": conn_id,
                            "state": state_tok,
                            "device_code": dc.device_code,
                            "user_code": dc.user_code,
                            "verification_uri": dc.verification_uri,
                            "verification_uri_complete": dc.verification_uri_complete,
                            "expires_in": dc.expires_in,
                            "interval": dc.interval,
                        }),
                    )
                }
                Err(e) => json_status(StatusCode::BAD_GATEWAY, json!({"error": e})),
            }
        }
        OAuthFlowKind::ImportOnly => json_status(
            StatusCode::OK,
            json!({
                "flow": "import_only",
                "provider": cfg.id,
                "connection_id": conn_id,
                "authorize_url": cfg.authorize_url,
                "import_endpoint": format!("/api/oauth/{}/import", cfg.id),
            }),
        ),
    }
}

// ── POST /api/oauth/{provider}/callback ─────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CallbackBody {
    pub code: String,
    pub state: Option<String>,
    pub connection_id: Option<String>,
    pub redirect_uri: Option<String>,
    pub code_verifier: Option<String>,
    pub client_id: Option<String>,
    pub token_url: Option<String>,
    pub label: Option<String>,
    pub provider_specific_json: Option<Value>,
}

pub async fn oauth_callback(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(provider): Path<String>,
    axum::Json(body): axum::Json<CallbackBody>,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    let Some(cfg) = find_oauth_provider(&provider) else {
        return json_status(
            StatusCode::NOT_FOUND,
            json!({"error": format!("unsupported OAuth provider: {provider}")}),
        );
    };
    let now = now_ms();
    let session = body
        .state
        .as_deref()
        .and_then(|s| state.oauth_sessions.take(s, now));

    let redirect_uri = body
        .redirect_uri
        .or_else(|| session.as_ref().map(|s| s.redirect_uri.clone()))
        .unwrap_or_else(|| cfg.default_redirect_uri());
    let code_verifier = body
        .code_verifier
        .or_else(|| session.as_ref().map(|s| s.code_verifier.clone()));
    let client_id = body
        .client_id
        .or_else(|| session.as_ref().map(|s| s.client_id.clone()));
    let token_url = body
        .token_url
        .or_else(|| session.as_ref().and_then(|s| s.token_url_override.clone()));
    let connection_id = body
        .connection_id
        .or_else(|| session.as_ref().map(|s| s.connection_id.clone()))
        .unwrap_or_else(|| format!("{}-default", cfg.id));

    match exchange_authorization_code(
        &state.dispatcher.client,
        cfg,
        &body.code,
        &redirect_uri,
        code_verifier.as_deref(),
        client_id.as_deref(),
        token_url.as_deref(),
    )
    .await
    {
        Ok(tok) => {
            let mut psj = body.provider_specific_json.unwrap_or_else(|| json!({}));
            if let Some(ref tu) = token_url {
                psj["token_url"] = Value::String(tu.clone());
            }
            if let Some(ref cid) = client_id {
                if !cid.is_empty() {
                    psj["client_id"] = Value::String(cid.clone());
                }
            }
            let expires_at = tok.expires_at_ms(now);
            let row = CredentialRow {
                provider: cfg.id.to_string(),
                connection_id: connection_id.clone(),
                auth_type: "oauth".into(),
                api_key: tok.access_token,
                base_urls: vec![],
                models: vec![],
                enabled: true,
                priority: 0,
                weight: 1.0,
                label: body.label.unwrap_or_else(|| cfg.display_name.to_string()),
                refresh_token: tok.refresh_token,
                expires_at,
                provider_specific_json: psj,
                health_status: "healthy".into(),
                last_refreshed_at: Some(now),
                health_check_interval_min: 60,
            };
            if let Err(e) = state.store.upsert_credential_full(&row) {
                return json_status(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    json!({"error": e.to_string()}),
                );
            }
            json_status(
                StatusCode::CREATED,
                json!({
                    "ok": true,
                    "provider": cfg.id,
                    "connection_id": connection_id,
                    "expires_at": expires_at,
                    "has_refresh_token": row.refresh_token.is_some(),
                    "health_status": "healthy",
                }),
            )
        }
        Err(e) => json_status(StatusCode::BAD_GATEWAY, json!({"error": e})),
    }
}

// ── POST /api/oauth/{provider}/poll (Device Code Flow) ──────────────

#[derive(Debug, Deserialize)]
pub struct PollBody {
    pub state: Option<String>,
    pub device_code: Option<String>,
    pub connection_id: Option<String>,
    pub client_id: Option<String>,
    pub token_url: Option<String>,
    pub copilot_token_url: Option<String>,
    pub label: Option<String>,
}

pub async fn oauth_device_poll(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(provider): Path<String>,
    axum::Json(body): axum::Json<PollBody>,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    let Some(cfg) = find_oauth_provider(&provider) else {
        return json_status(
            StatusCode::NOT_FOUND,
            json!({"error": format!("unsupported OAuth provider: {provider}")}),
        );
    };
    let now = now_ms();
    let session = body
        .state
        .as_deref()
        .and_then(|s| state.oauth_sessions.get(s, now));
    let Some(device_code) = body
        .device_code
        .or_else(|| session.as_ref().and_then(|s| s.device_code.clone()))
    else {
        return json_status(
            StatusCode::BAD_REQUEST,
            json!({"error": "missing device_code or valid state"}),
        );
    };
    let client_id = body
        .client_id
        .or_else(|| session.as_ref().map(|s| s.client_id.clone()));
    let token_url = body
        .token_url
        .or_else(|| session.as_ref().and_then(|s| s.token_url_override.clone()));
    let connection_id = body
        .connection_id
        .or_else(|| session.as_ref().map(|s| s.connection_id.clone()))
        .unwrap_or_else(|| format!("{}-default", cfg.id));

    match poll_device_code(
        &state.dispatcher.client,
        cfg,
        &device_code,
        client_id.as_deref(),
        token_url.as_deref(),
    )
    .await
    {
        Ok(DevicePollOutcome::Pending) => {
            json_status(StatusCode::ACCEPTED, json!({"status": "pending"}))
        }
        Ok(DevicePollOutcome::SlowDown { next_interval_secs }) => json_status(
            StatusCode::ACCEPTED,
            json!({"status": "slow_down", "interval": next_interval_secs}),
        ),
        Ok(DevicePollOutcome::Expired) => {
            if let Some(ref s) = body.state {
                let _ = state.oauth_sessions.take(s, now);
            }
            json_status(StatusCode::GONE, json!({"status": "expired"}))
        }
        Ok(DevicePollOutcome::Denied(reason)) => {
            if let Some(ref s) = body.state {
                let _ = state.oauth_sessions.take(s, now);
            }
            json_status(StatusCode::FORBIDDEN, json!({"status": "denied", "error": reason}))
        }
        Ok(DevicePollOutcome::Authorized(tok)) => {
            if let Some(ref s) = body.state {
                let _ = state.oauth_sessions.take(s, now);
            }
            let mut psj = json!({});
            if matches!(cfg.id, "github" | "ghe-copilot") {
                if let Ok(ct) = exchange_copilot_token(
                    &state.dispatcher.client,
                    &tok.access_token,
                    body.copilot_token_url.as_deref(),
                )
                .await
                {
                    psj["copilot_token"] = Value::String(ct.token);
                    psj["copilot_token_expires_at"] = json!(ct.expires_at_ms);
                    if let Some(ep) = ct.api_endpoint {
                        psj["copilot_api_endpoint"] = Value::String(ep);
                    }
                }
                if let Some(ref cu) = body.copilot_token_url {
                    psj["copilot_token_url"] = Value::String(cu.clone());
                }
            }
            let expires_at = tok.expires_at_ms(now);
            let row = CredentialRow {
                provider: cfg.id.to_string(),
                connection_id: connection_id.clone(),
                auth_type: "oauth".into(),
                api_key: tok.access_token,
                base_urls: vec![],
                models: vec![],
                enabled: true,
                priority: 0,
                weight: 1.0,
                label: body.label.unwrap_or_else(|| cfg.display_name.to_string()),
                refresh_token: tok.refresh_token,
                expires_at,
                provider_specific_json: psj,
                health_status: "healthy".into(),
                last_refreshed_at: Some(now),
                health_check_interval_min: 60,
            };
            if let Err(e) = state.store.upsert_credential_full(&row) {
                return json_status(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    json!({"error": e.to_string()}),
                );
            }
            json_status(
                StatusCode::CREATED,
                json!({
                    "status": "authorized",
                    "provider": cfg.id,
                    "connection_id": connection_id,
                    "expires_at": expires_at,
                    "has_copilot_token": row.provider_specific_json.get("copilot_token").is_some(),
                }),
            )
        }
        Err(e) => json_status(StatusCode::BAD_GATEWAY, json!({"error": e})),
    }
}

// ── POST /api/oauth/{provider}/import ───────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ImportOAuthBody {
    pub connection_id: Option<String>,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>,
    pub expires_in: Option<i64>,
    #[serde(default)]
    pub base_urls: Vec<String>,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub provider_specific_json: Value,
    pub health_check_interval_min: Option<i64>,
}

pub async fn import_oauth_credential(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(provider): Path<String>,
    axum::Json(body): axum::Json<ImportOAuthBody>,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    let now = now_ms();
    let canonical = find_oauth_provider(&provider)
        .map(|p| p.id.to_string())
        .unwrap_or(provider);
    let conn_id = body
        .connection_id
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{canonical}-imported"));
    let expires_at = body
        .expires_at
        .or_else(|| body.expires_in.map(|s| now + s * 1000));
    let psj = if body.provider_specific_json.is_null() {
        json!({})
    } else {
        body.provider_specific_json
    };
    let row = CredentialRow {
        provider: canonical.clone(),
        connection_id: conn_id.clone(),
        auth_type: "oauth".into(),
        api_key: body.access_token,
        base_urls: body.base_urls,
        models: vec![],
        enabled: true,
        priority: 0,
        weight: 1.0,
        label: body.label,
        refresh_token: body.refresh_token,
        expires_at,
        provider_specific_json: psj,
        health_status: "healthy".into(),
        last_refreshed_at: Some(now),
        health_check_interval_min: body.health_check_interval_min.unwrap_or(60),
    };
    match state.store.upsert_credential_full(&row) {
        Ok(()) => json_status(
            StatusCode::CREATED,
            json!({"ok": true, "provider": canonical, "connection_id": conn_id}),
        ),
        Err(e) => json_status(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error": e.to_string()}),
        ),
    }
}

// ── POST /api/oauth/{provider}/{connection_id}/refresh ──────────────

#[derive(Debug, Deserialize, Default)]
pub struct RefreshBody {
    pub token_url: Option<String>,
    pub copilot_token_url: Option<String>,
}

pub async fn manual_refresh_oauth(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((provider, connection_id)): Path<(String, String)>,
    body: Option<axum::Json<RefreshBody>>,
) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    let canonical = find_oauth_provider(&provider)
        .map(|p| p.id)
        .unwrap_or(provider.as_str());
    let Ok(Some(cred)) = state.store.get_credential(canonical, &connection_id) else {
        return json_status(
            StatusCode::NOT_FOUND,
            json!({"error": "credential not found"}),
        );
    };
    let req = body.map(|b| b.0).unwrap_or_default();
    match refresh_credential_now(
        &state,
        &cred,
        req.token_url.as_deref(),
        req.copilot_token_url.as_deref(),
    )
    .await
    {
        Ok(updated) => json_status(
            StatusCode::OK,
            json!({
                "ok": true,
                "provider": updated.provider,
                "connection_id": updated.connection_id,
                "health_status": updated.health_status,
                "expires_at": updated.expires_at,
                "last_refreshed_at": updated.last_refreshed_at,
            }),
        ),
        Err((status, msg)) => json_status(
            status,
            json!({
                "ok": false,
                "provider": cred.provider,
                "connection_id": cred.connection_id,
                "error": msg,
            }),
        ),
    }
}

// ── Core Refresh Logic & On-Demand Helper ───────────────────────────

pub async fn refresh_credential_now(
    state: &SharedState,
    cred: &CredentialRow,
    token_url_override: Option<&str>,
    copilot_url_override: Option<&str>,
) -> Result<CredentialRow, (StatusCode, String)> {
    let now = now_ms();
    // GitHub Copilot short-lived token refresh path
    if matches!(cred.provider.as_str(), "github" | "ghe-copilot") {
        let cu = copilot_url_override.or_else(|| {
            cred.provider_specific_json
                .get("copilot_token_url")
                .and_then(Value::as_str)
        });
        match exchange_copilot_token(&state.dispatcher.client, &cred.api_key, cu).await {
            Ok(ct) => {
                let mut psj = cred.provider_specific_json.clone();
                psj["copilot_token"] = Value::String(ct.token);
                psj["copilot_token_expires_at"] = json!(ct.expires_at_ms);
                if let Some(ep) = ct.api_endpoint {
                    psj["copilot_api_endpoint"] = Value::String(ep);
                }
                let _ = state.store.update_oauth_tokens(
                    &cred.provider,
                    &cred.connection_id,
                    &cred.api_key,
                    cred.refresh_token.as_deref(),
                    Some(ct.expires_at_ms),
                    Some(&psj),
                    now,
                );
                let mut next = cred.clone();
                next.provider_specific_json = psj;
                next.expires_at = Some(ct.expires_at_ms);
                next.health_status = "healthy".into();
                next.last_refreshed_at = Some(now);
                return Ok(next);
            }
            Err(e) => {
                if e.contains("401") || e.contains("403") {
                    let _ = state.store.mark_credential_health(
                        &cred.provider,
                        &cred.connection_id,
                        "needs_reauth",
                        now,
                    );
                }
                return Err((StatusCode::BAD_GATEWAY, e));
            }
        }
    }

    let Some(cfg) = find_oauth_provider(&cred.provider) else {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("no OAuth refresh config for provider {}", cred.provider),
        ));
    };
    let Some(rt) = cred.refresh_token.as_deref().filter(|s| !s.is_empty()) else {
        return Err((StatusCode::BAD_REQUEST, "credential has no refresh_token".into()));
    };

    let tu = token_url_override.or_else(|| {
        cred.provider_specific_json
            .get("token_url")
            .and_then(Value::as_str)
    });
    let cid = cred
        .provider_specific_json
        .get("client_id")
        .and_then(Value::as_str);

    match refresh_with_retry(
        &state.dispatcher.client,
        &state.oauth_breaker,
        cfg,
        &cred.connection_id,
        rt,
        cid,
        tu,
        3,
    )
    .await
    {
        RefreshOutcome::Refreshed(tok) => {
            let expires_at = tok.expires_at_ms(now);
            let _ = state.store.update_oauth_tokens(
                &cred.provider,
                &cred.connection_id,
                &tok.access_token,
                tok.refresh_token.as_deref(),
                expires_at,
                None,
                now,
            );
            let mut next = cred.clone();
            next.api_key = tok.access_token;
            if let Some(new_rt) = tok.refresh_token {
                next.refresh_token = Some(new_rt);
            }
            next.expires_at = expires_at;
            next.health_status = "healthy".into();
            next.last_refreshed_at = Some(now);
            Ok(next)
        }
        RefreshOutcome::Unrecoverable(msg) => {
            let _ = state.store.mark_credential_health(
                &cred.provider,
                &cred.connection_id,
                "needs_reauth",
                now,
            );
            Err((StatusCode::UNAUTHORIZED, msg))
        }
        RefreshOutcome::Transient(msg) => Err((StatusCode::BAD_GATEWAY, msg)),
    }
}

/// Called on the hot request path before dispatch: refreshes an OAuth or Copilot token
/// if it is within 60 seconds of expiry.
pub async fn ensure_fresh_oauth_credential(
    state: &SharedState,
    cred: &CredentialRow,
) -> CredentialRow {
    let now = now_ms();
    if matches!(cred.provider.as_str(), "github" | "ghe-copilot") {
        let has_valid_ctok = cred
            .provider_specific_json
            .get("copilot_token")
            .and_then(Value::as_str)
            .map(|s| !s.is_empty())
            .unwrap_or(false)
            && cred
                .provider_specific_json
                .get("copilot_token_expires_at")
                .and_then(Value::as_i64)
                .map(|exp| exp > now + ON_DEMAND_EXPIRY_BUFFER_MS)
                .unwrap_or(true);
        if !has_valid_ctok && !cred.api_key.starts_with("tid=") {
            if let Ok(fresh) = refresh_credential_now(state, cred, None, None).await {
                return fresh;
            }
        }
        return cred.clone();
    }

    let expiring = cred
        .expires_at
        .map(|exp| exp <= now + ON_DEMAND_EXPIRY_BUFFER_MS)
        .unwrap_or(false);
    if expiring && cred.refresh_token.as_deref().map(|s| !s.is_empty()).unwrap_or(false) {
        if let Ok(fresh) = refresh_credential_now(state, cred, None, None).await {
            return fresh;
        }
    }
    cred.clone()
}

// ── Background Refresh Sweep + Scheduler ────────────────────────────

pub async fn run_oauth_refresh_sweep(state: &SharedState) -> Value {
    let now = now_ms();
    if is_healthcheck_disabled() {
        return json!({"skipped": true, "reason": "OMNIROUTE_DISABLE_TOKEN_HEALTHCHECK"});
    }
    let skip_providers = healthcheck_skip_providers();
    let cap = healthcheck_per_provider_concurrency();
    let expiring = state
        .store
        .list_expiring_oauth_credentials(now, PROACTIVE_REFRESH_WINDOW_MS)
        .unwrap_or_default();

    let mut semaphores: HashMap<String, Arc<Semaphore>> = HashMap::new();
    let mut handles = Vec::new();
    let mut skipped_count = 0usize;

    for cred in expiring {
        if skip_providers.contains(&cred.provider.to_ascii_lowercase()) {
            skipped_count += 1;
            continue;
        }
        let sem = if cap > 0 {
            Some(
                semaphores
                    .entry(cred.provider.clone())
                    .or_insert_with(|| Arc::new(Semaphore::new(cap)))
                    .clone(),
            )
        } else {
            None
        };
        let st = state.clone();
        handles.push(tokio::spawn(async move {
            let _permit = match sem {
                Some(ref s) => s.acquire().await.ok(),
                None => None,
            };
            match refresh_credential_now(&st, &cred, None, None).await {
                Ok(_) => ("refreshed", cred.provider, cred.connection_id),
                Err((StatusCode::UNAUTHORIZED, _)) => ("needs_reauth", cred.provider, cred.connection_id),
                Err(_) => ("transient_error", cred.provider, cred.connection_id),
            }
        }));
    }

    let mut refreshed = 0usize;
    let mut needs_reauth = 0usize;
    let mut transient = 0usize;
    for h in handles {
        if let Ok((outcome, _, _)) = h.await {
            match outcome {
                "refreshed" => refreshed += 1,
                "needs_reauth" => needs_reauth += 1,
                _ => transient += 1,
            }
        }
    }

    json!({
        "skipped": false,
        "refreshed": refreshed,
        "needs_reauth": needs_reauth,
        "transient_errors": transient,
        "skipped_providers": skipped_count,
    })
}

pub async fn trigger_oauth_sweep(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    if let Err(resp) = authorize_request(&state, &headers, false) {
        return resp;
    }
    json_status(StatusCode::OK, run_oauth_refresh_sweep(&state).await)
}

pub fn spawn_oauth_refresh_scheduler(state: SharedState) {
    if is_healthcheck_disabled() {
        tracing::info!("OAuth background token refresh scheduler disabled via env");
        return;
    }
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(BG_SWEEP_INTERVAL_SECS));
        interval.tick().await; // skip immediate tick at boot
        loop {
            interval.tick().await;
            let report = run_oauth_refresh_sweep(&state).await;
            let refreshed = report.get("refreshed").and_then(Value::as_u64).unwrap_or(0);
            let reauth = report.get("needs_reauth").and_then(Value::as_u64).unwrap_or(0);
            if refreshed > 0 || reauth > 0 {
                tracing::info!(
                    refreshed,
                    needs_reauth = reauth,
                    "OAuth background refresh sweep completed"
                );
            }
        }
    });
}
