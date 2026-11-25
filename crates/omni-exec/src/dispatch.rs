// HTTP dispatch + retry-with-cooldown classification + v3.8.54 Anthropic gzip request bodies.
// Port of open-sse/executors/base.ts + open-sse/executors/base/gzipRequestBody.ts.

use crate::resilience;
use flate2::write::GzEncoder;
use flate2::Compression;
use omni_types::errors::{
    backoff_cooldown, find_matching_rule, COOLDOWN_MS, TRANSIENT_INITIAL_MS, TRANSIENT_MAX_MS,
};
use omni_types::OmniError;
use rand::RngCore;
use std::collections::HashMap;
use std::io::Write;
use std::str::FromStr;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

pub const GZIP_REQUEST_BODY_MIN_BYTES: usize = 4096;
const GZIP_LATCH_TTL: Duration = Duration::from_secs(3600);
const GZIP_MAX_PADDING: usize = 256;

static GZIP_LATCHED_ORIGINS: LazyLock<Mutex<HashMap<String, Instant>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone)]
pub struct ConnectionEndpoint {
    pub provider: String,
    pub connection_id: String,
    pub model: String,
    pub auth_type: AuthScheme,
    pub base_urls: Vec<String>,
    pub api_key: String,
    pub auth_header: AuthHeader,
    pub auth_prefix: Option<String>,
    pub extra_headers: Vec<(String, String)>,
    pub timeout_ms: Option<u64>,
    /// Optional exact pre-serialized JSON body (used by AWS SigV4 so transmitted bytes
    /// match `x-amz-content-sha256` byte-for-byte).
    pub raw_body_override: Option<String>,
    /// Allow gzipping large bodies (>= 4096 bytes) even when host != `api.anthropic.com`
    /// (useful for explicit endpoints and automated tests).
    pub force_gzip: bool,
    /// Optional per-request upstream proxy (`http://`, `https://`, `socks5://`, `socks5h://`).
    pub proxy_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthScheme {
    ApiKey,
    OAuth,
    Anonymous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthHeader {
    Bearer,
    XApiKey,
    XGoogApiKey,
    AzureApiKey,
    AuthorizationKey,
    None,
}

#[derive(Debug, Clone, Copy)]
pub enum WireFormat {
    OpenAI,
    Claude,
    Gemini,
}

impl AuthHeader {
    pub fn from_str_lossy(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "x-api-key" => Self::XApiKey,
            "x-goog-api-key" => Self::XGoogApiKey,
            "api-key" => Self::AzureApiKey,
            "authorization-key" => Self::AuthorizationKey,
            "none" => Self::None,
            _ => Self::Bearer,
        }
    }

    pub fn header_name(&self) -> Option<&'static str> {
        match self {
            Self::Bearer => Some("Authorization"),
            Self::XApiKey => Some("x-api-key"),
            Self::XGoogApiKey => Some("x-goog-api-key"),
            Self::AzureApiKey => Some("api-key"),
            Self::AuthorizationKey => Some("Authorization"),
            Self::None => None,
        }
    }

    pub fn render_value(&self, key: &str, custom_prefix: Option<&str>) -> String {
        if let Some(pre) = custom_prefix {
            if !pre.is_empty() {
                return format!("{pre}{key}");
            }
        }
        match self {
            Self::Bearer => format!("Bearer {key}"),
            Self::AuthorizationKey => format!("Key {key}"),
            _ => key.to_string(),
        }
    }
}

/// Failure metadata returned by `dispatch_once_detailed` for retry/lockout decisions.
#[derive(Debug)]
pub struct DispatchFailure {
    pub error: OmniError,
    pub retry_after_ms: Option<u64>,
    pub cooldown_ms: u64,
    pub is_terminal: bool,
    pub is_rate_limit: bool,
    /// True when Antigravity's anti-hammering 429 refusal guard answered locally without
    /// hitting upstream (so the caller must not re-try or cool the whole account).
    pub is_local_refusal_replay: bool,
}

const MAX_CACHED_PROXY_CLIENTS: usize = 64;

pub struct Dispatcher {
    pub client: reqwest::Client,
    proxy_clients: Mutex<HashMap<String, reqwest::Client>>,
}

impl Default for Dispatcher {
    fn default() -> Self {
        Self::new()
    }
}

/// Validates and normalizes an upstream proxy URL (`http://`, `https://`, `socks5://`, `socks5h://`).
pub fn validate_proxy_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("proxy URL is empty".into());
    }
    let parsed = reqwest::Url::parse(trimmed).map_err(|e| format!("invalid proxy URL: {e}"))?;
    match parsed.scheme() {
        "http" | "https" | "socks5" | "socks5h" => {}
        other => {
            return Err(format!(
                "unsupported proxy scheme '{other}' (expected http, https, socks5, or socks5h)"
            ));
        }
    }
    if parsed.host_str().map(|h| h.is_empty()).unwrap_or(true) {
        return Err("proxy URL is missing host".into());
    }
    Ok(trimmed.to_string())
}

impl Dispatcher {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(600))
                .pool_idle_timeout(Duration::from_secs(90))
                .user_agent("omniroute-rs/0.2")
                .build()
                .expect("static reqwest config"),
            proxy_clients: Mutex::new(HashMap::new()),
        }
    }

    /// Returns a pooled `reqwest::Client` configured for `proxy_url` (or the default direct client).
    /// Clients are cached by proxy URL (capped at 64 entries) to keep memory footprint minimal.
    pub fn client_for_proxy(&self, proxy_url: Option<&str>) -> Result<reqwest::Client, OmniError> {
        let Some(raw) = proxy_url.map(str::trim).filter(|s| !s.is_empty()) else {
            return Ok(self.client.clone());
        };
        if raw.eq_ignore_ascii_case("direct") || raw.eq_ignore_ascii_case("none") {
            return Ok(self.client.clone());
        }
        let normalized = validate_proxy_url(raw).map_err(OmniError::BadRequest)?;
        {
            let cache = self.proxy_clients.lock().unwrap();
            if let Some(c) = cache.get(&normalized) {
                return Ok(c.clone());
            }
        }
        let proxy = reqwest::Proxy::all(&normalized)
            .map_err(|e| OmniError::BadRequest(format!("proxy setup failed: {e}")))?;
        let built = reqwest::Client::builder()
            .proxy(proxy)
            .timeout(Duration::from_secs(600))
            .pool_idle_timeout(Duration::from_secs(60))
            .user_agent("omniroute-rs/0.2")
            .build()
            .map_err(|e| OmniError::Network(format!("proxy client build failed: {e}")))?;
        let mut cache = self.proxy_clients.lock().unwrap();
        if cache.len() >= MAX_CACHED_PROXY_CLIENTS {
            cache.clear();
        }
        cache.insert(normalized, built.clone());
        Ok(built)
    }

    pub fn cached_proxy_count(&self) -> usize {
        self.proxy_clients.lock().unwrap().len()
    }
}

// ── v3.8.54 Gzip request body transport (Anthropic api.anthropic.com) ─

pub fn gzip_disabled_by_env() -> bool {
    std::env::var("OMNIROUTE_GZIP_REQUEST_BODIES")
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "off" | "no"))
        .unwrap_or(false)
}

pub fn reset_gzip_latch_for_test() {
    GZIP_LATCHED_ORIGINS.lock().unwrap().clear();
}

fn extract_origin_and_host(url: &str) -> Option<(String, String)> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    let origin = match parsed.port() {
        Some(p) => format!("{}://{host}:{p}", parsed.scheme()),
        None => format!("{}://{host}", parsed.scheme()),
    };
    Some((origin, host))
}

fn is_origin_gzip_latched(origin: &str) -> bool {
    let mut map = GZIP_LATCHED_ORIGINS.lock().unwrap();
    if let Some(until) = map.get(origin).copied() {
        if Instant::now() < until {
            return true;
        }
        map.remove(origin);
    }
    false
}

fn latch_origin_gzip(origin: &str) {
    GZIP_LATCHED_ORIGINS
        .lock()
        .unwrap()
        .insert(origin.to_string(), Instant::now() + GZIP_LATCH_TTL);
}

fn has_signed_cch_token(body: &str) -> bool {
    let bytes = body.as_bytes();
    let mut i = 0;
    while i + 10 <= bytes.len() {
        if &bytes[i..i + 4] == b"cch="
            && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric())
            && bytes[i + 4..i + 9].iter().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            && bytes[i + 9] == b';'
        {
            return true;
        }
        i += 1;
    }
    false
}

fn random_length_padding() -> String {
    let mut rng = rand::rngs::OsRng;
    let count = (rng.next_u32() as usize) % (GZIP_MAX_PADDING + 1);
    let mut out = String::with_capacity(count + 1);
    out.push('\n');
    for _ in 0..count {
        out.push(if rng.next_u32() & 1 == 0 { ' ' } else { '\t' });
    }
    out
}

pub fn gzip_encode_json_body(body_str: &str) -> Result<Vec<u8>, std::io::Error> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::new(3));
    encoder.write_all(body_str.as_bytes())?;
    if !has_signed_cch_token(body_str) {
        let pad = random_length_padding();
        encoder.write_all(pad.as_bytes())?;
    }
    encoder.finish()
}

fn should_gzip_request(url: &str, conn: &ConnectionEndpoint, body_len: usize) -> Option<String> {
    if body_len < GZIP_REQUEST_BODY_MIN_BYTES || gzip_disabled_by_env() {
        return None;
    }
    if conn
        .extra_headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("content-encoding"))
    {
        return None;
    }
    // Never mutate bytes when a pre-signed raw body (e.g., AWS SigV4) is active.
    if conn.raw_body_override.is_some() {
        return None;
    }
    let (origin, host) = extract_origin_and_host(url)?;
    if !conn.force_gzip && host != "api.anthropic.com" {
        return None;
    }
    if is_origin_gzip_latched(&origin) {
        return None;
    }
    Some(origin)
}

fn build_post_request(
    dispatcher: &Dispatcher,
    url: &str,
    conn: &ConnectionEndpoint,
    stream: bool,
) -> Result<reqwest::RequestBuilder, OmniError> {
    let is_ag = crate::antigravity::is_antigravity_provider(&conn.provider);
    let is_agy_cli = conn.provider.eq_ignore_ascii_case("agy");

    let client = dispatcher.client_for_proxy(conn.proxy_url.as_deref())?;
    let mut req = client
        .post(url)
        .header("Content-Type", "application/json");
    if let Some(ms) = conn.timeout_ms {
        req = req.timeout(Duration::from_millis(ms));
    }
    // Upstream `antigravity.ts`: native Go CLI (`agy`) requests SSE via `?alt=sse` alone and
    // sends no `Accept: text/event-stream` header.
    if stream
        && !is_agy_cli
        && !conn
            .extra_headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("accept"))
    {
        req = req.header("Accept", "text/event-stream");
    }

    if is_ag {
        // Place scrubbed headers first and defer `Authorization` to the final header
        // (exact wire order matching native Antigravity fingerprint in antigravityHeaderScrub.ts).
        for (k, v) in &conn.extra_headers {
            if !k.eq_ignore_ascii_case("authorization") {
                req = req.header(k.as_str(), v.as_str());
            }
        }
        if let Some(hname) = conn.auth_header.header_name() {
            if !conn.api_key.is_empty() {
                req = req.header(
                    hname,
                    conn.auth_header
                        .render_value(&conn.api_key, conn.auth_prefix.as_deref()),
                );
            }
        }
    } else {
        if let Some(hname) = conn.auth_header.header_name() {
            if !conn.api_key.is_empty() {
                req = req.header(
                    hname,
                    conn.auth_header
                        .render_value(&conn.api_key, conn.auth_prefix.as_deref()),
                );
            }
        }
        for (k, v) in &conn.extra_headers {
            req = req.header(k.as_str(), v.as_str());
        }
    }
    Ok(req)
}

async fn attempt(
    dispatcher: &Dispatcher,
    url: &str,
    conn: &ConnectionEndpoint,
    body: &serde_json::Value,
    stream: bool,
) -> Result<reqwest::Response, OmniError> {
    let serialized = match &conn.raw_body_override {
        Some(raw) => raw.clone(),
        None => serde_json::to_string(body).map_err(|e| OmniError::BadRequest(e.to_string()))?,
    };

    if let Some(origin) = should_gzip_request(url, conn, serialized.len()) {
        if let Ok(compressed) = gzip_encode_json_body(&serialized) {
            let resp = build_post_request(dispatcher, url, conn, stream)?
                .header("Content-Encoding", "gzip")
                .body(compressed)
                .send()
                .await
                .map_err(|e| OmniError::Network(e.to_string()))?;

            let status = resp.status().as_u16();
            let from_anthropic = resp
                .headers()
                .get("request-id")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.starts_with("req_"))
                .unwrap_or(false)
                || resp.headers().contains_key("cf-ray");

            let is_rejection = if status == 415 {
                true
            } else if (status == 400 || status == 403) && !from_anthropic {
                true
            } else if status == 400 {
                let text = resp.text().await.unwrap_or_default();
                let not_json = text.contains("The request body is not valid JSON")
                    || text.contains("Failed to parse request: could not parse request body as JSON");
                if not_json {
                    latch_origin_gzip(&origin);
                    tracing::warn!(origin = %origin, status, "origin refused gzip body; resending uncompressed");
                    return build_post_request(dispatcher, url, conn, stream)?
                        .body(serialized)
                        .send()
                        .await
                        .map_err(|e| OmniError::Network(e.to_string()));
                }
                return Err(OmniError::Upstream {
                    status,
                    body: Some(text),
                });
            } else {
                false
            };

            if is_rejection {
                latch_origin_gzip(&origin);
                tracing::warn!(origin = %origin, status, "origin refused gzip body; resending uncompressed");
                return build_post_request(dispatcher, url, conn, stream)?
                    .body(serialized)
                    .send()
                    .await
                    .map_err(|e| OmniError::Network(e.to_string()));
            }
            return Ok(resp);
        }
    }

    build_post_request(dispatcher, url, conn, stream)?
        .body(serialized)
        .send()
        .await
        .map_err(|e| OmniError::Network(e.to_string()))
}

pub fn retry_after_from_header(resp: &reqwest::Response) -> Option<Duration> {
    resp.headers()
        .get("retry-after")
        .or_else(|| resp.headers().get("x-ratelimit-reset-requests"))
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            let trimmed = v.trim();
            if let Ok(secs) = f64::from_str(trimmed) {
                Some(Duration::from_secs_f64(secs.max(0.0)))
            } else if let Some(ms_str) = trimmed.strip_suffix("ms") {
                ms_str.parse::<u64>().ok().map(Duration::from_millis)
            } else if let Some(s_str) = trimmed.strip_suffix('s') {
                s_str.parse::<f64>().ok().map(|s| Duration::from_secs_f64(s.max(0.0)))
            } else {
                chrono::DateTime::parse_from_rfc2822(trimmed)
                    .ok()
                    .map(|dt| {
                        (dt.with_timezone(&chrono::Utc) - chrono::Utc::now())
                            .max(chrono::Duration::zero())
                            .to_std()
                            .unwrap_or_default()
                    })
            }
        })
}

/// Dispatch across `conn.base_urls`, classifying cooldowns, reading `Retry-After`
/// header & JSON body, locking broken `(provider, conn, model)` tuples, and
/// updating both provider and connection circuit breakers.
pub async fn dispatch_once_detailed(
    dispatcher: &Dispatcher,
    conn: &ConnectionEndpoint,
    body: &serde_json::Value,
    stream: bool,
    attempt_level: u32,
) -> Result<reqwest::Response, DispatchFailure> {
    let mut last: Option<DispatchFailure> = None;
    let profile = resilience::profile_for(&conn.provider);
    let is_ag = crate::antigravity::is_antigravity_provider(&conn.provider);

    // Protection 4 (Anti-Hammering 429 Refusal Guard): check whether this exact request body
    // was already refused by Antigravity. If active, answer 429 locally with ZERO upstream calls.
    let ag_fingerprint = if is_ag {
        let fp = crate::antigravity::fingerprint_antigravity_request(&conn.connection_id, body);
        let now_ms = chrono::Utc::now().timestamp_millis();
        if let Some((remaining_ms, strikes)) = crate::antigravity::get_active_refusal(&fp, now_ms) {
            return Err(DispatchFailure {
                error: OmniError::Upstream {
                    status: 429,
                    body: Some(crate::antigravity::build_refusal_replay_body(
                        remaining_ms,
                        strikes,
                    )),
                },
                retry_after_ms: Some(remaining_ms),
                cooldown_ms: remaining_ms,
                is_terminal: false,
                is_rate_limit: true,
                is_local_refusal_replay: true,
            });
        }
        Some(fp)
    } else {
        None
    };
    let ag_429_budget = if is_ag {
        crate::antigravity::get_antigravity_429_attempt_budget()
    } else {
        u32::MAX
    };
    let mut ag_429_seen: u32 = 0;

    let total_urls = conn.base_urls.iter().filter(|u| !u.is_empty()).count();
    let mut url_idx: usize = 0;

    for url in &conn.base_urls {
        if url.is_empty() {
            continue;
        }
        url_idx += 1;
        let resp = match attempt(dispatcher, url, conn, body, stream).await {
            Ok(r) => r,
            Err(OmniError::Upstream { status, body: payload_opt }) => {
                let payload = payload_opt.unwrap_or_default();
                let (base_cd, use_backoff) = classify_cooldown(status, &payload);
                let cooldown_ms =
                    bounded_cooldown(base_cd, use_backoff, profile.max_backoff_level, attempt_level);
                last = Some(DispatchFailure {
                    error: OmniError::Upstream {
                        status,
                        body: Some(payload),
                    },
                    retry_after_ms: None,
                    cooldown_ms,
                    is_terminal: resilience::is_terminal_status(status),
                    is_rate_limit: status == 429,
                    is_local_refusal_replay: false,
                });
                continue;
            }
            Err(e) => {
                resilience::record_connection_failure(&conn.provider, &conn.connection_id);
                resilience::record_provider_failure(&conn.provider);
                let cd = bounded_cooldown(
                    profile.transient_cooldown,
                    true,
                    profile.max_backoff_level,
                    attempt_level,
                );
                resilience::lock_model(
                    &conn.provider,
                    &conn.connection_id,
                    &conn.model,
                    "network",
                    cd,
                    attempt_level + 1,
                );
                last = Some(DispatchFailure {
                    error: e,
                    retry_after_ms: None,
                    cooldown_ms: cd,
                    is_terminal: false,
                    is_rate_limit: false,
                    is_local_refusal_replay: false,
                });
                continue;
            }
        };

        let mut status = resp.status().as_u16();
        if (200..300).contains(&status) {
            resilience::record_connection_success(&conn.provider, &conn.connection_id);
            resilience::record_provider_success(&conn.provider);
            resilience::clear_model_lock(&conn.provider, &conn.connection_id, &conn.model);
            return Ok(resp);
        }

        // Upstream `sendAntigravityRequest` (executeAttempt.ts:454): on 403 when
        // `x-goog-user-project` was sent, transparently retry once without it.
        let resp = if is_ag
            && status == 403
            && conn
                .extra_headers
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("x-goog-user-project"))
        {
            let mut stripped_conn = conn.clone();
            stripped_conn
                .extra_headers
                .retain(|(k, _)| !k.eq_ignore_ascii_case("x-goog-user-project"));
            match attempt(dispatcher, url, &stripped_conn, body, stream).await {
                Ok(retry_resp) => {
                    status = retry_resp.status().as_u16();
                    if (200..300).contains(&status) {
                        resilience::record_connection_success(&conn.provider, &conn.connection_id);
                        resilience::record_provider_success(&conn.provider);
                        resilience::clear_model_lock(&conn.provider, &conn.connection_id, &conn.model);
                        return Ok(retry_resp);
                    }
                    retry_resp
                }
                Err(_) => resp,
            }
        } else {
            resp
        };

        let header_retry_ms = retry_after_from_header(&resp).map(|d| d.as_millis() as u64);
        let payload = resp.text().await.unwrap_or_default();
        let body_json = serde_json::from_str::<serde_json::Value>(&payload).ok();
        let body_retry_ms = parse_retry_after_json(body_json.as_ref());
        let retry_after_ms = match (header_retry_ms, body_retry_ms) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };

        let verification_required = is_account_verification_required(status, &payload);
        let ag_actionable_403 = is_ag && is_antigravity_actionable_403(status, &payload);
        let non_terminal_403 = verification_required || ag_actionable_403;
        let (base_cd, use_backoff) = if non_terminal_403 {
            (TRANSIENT_INITIAL_MS, false)
        } else {
            classify_cooldown(status, &payload)
        };
        let mut cooldown_ms =
            bounded_cooldown(base_cd, use_backoff, profile.max_backoff_level, attempt_level);
        if let Some(ra) = retry_after_ms {
            cooldown_ms = cooldown_ms.max(ra);
        }

        // Upstream #14848: Google Cloud Code / Antigravity `403 VALIDATION_REQUIRED`
        // ("Verify your account to continue") and `403 SUBSCRIPTION_REQUIRED` are
        // operator-actionable and should not lock out the entire provider breaker.
        let is_terminal = !non_terminal_403 && resilience::is_terminal_status(status);
        let is_rate_limit = status == 429 || payload.to_ascii_lowercase().contains("rate limit");

        if non_terminal_403 {
            // Brief model-scope cooldown (5s) without tripping the provider breaker so the user
            // can retry immediately after completing browser verification or updating projectId.
            resilience::lock_model(
                &conn.provider,
                &conn.connection_id,
                &conn.model,
                if verification_required {
                    "verification_required"
                } else {
                    "subscription_required"
                },
                TRANSIENT_INITIAL_MS,
                1,
            );
        } else {
            resilience::record_connection_failure(&conn.provider, &conn.connection_id);
            if status == 429 || status >= 500 || is_terminal {
                resilience::record_provider_failure(&conn.provider);
            }
            if status != 400 {
                resilience::lock_model(
                    &conn.provider,
                    &conn.connection_id,
                    &conn.model,
                    resilience::classify_lockout_reason(status),
                    cooldown_ms,
                    attempt_level + 1,
                );
            }
        }

        last = Some(DispatchFailure {
            error: OmniError::Upstream {
                status,
                body: Some(payload),
            },
            retry_after_ms,
            cooldown_ms,
            is_terminal,
            is_rate_limit,
            is_local_refusal_replay: false,
        });

        // Bounded per-request 429 budget for Antigravity / `agy`: once exhausted (or after
        // exhausting all configured hosts with 429), remember this request fingerprint locally
        // and stop retrying remaining fallback hosts.
        if is_ag && status == 429 {
            ag_429_seen += 1;
            if ag_429_seen >= ag_429_budget || url_idx >= total_urls {
                if let Some(ref fp) = ag_fingerprint {
                    let now_ms = chrono::Utc::now().timestamp_millis();
                    let _ = crate::antigravity::remember_429_refusal(fp, retry_after_ms, now_ms);
                }
                break;
            }
        }
    }

    Err(last.unwrap_or(DispatchFailure {
        error: OmniError::Network("no urls configured".into()),
        retry_after_ms: None,
        cooldown_ms: TRANSIENT_INITIAL_MS,
        is_terminal: false,
        is_rate_limit: false,
        is_local_refusal_replay: false,
    }))
}

/// Backward-compatible wrapper around `dispatch_once_detailed`.
pub async fn dispatch_once(
    dispatcher: &Dispatcher,
    conn: &ConnectionEndpoint,
    body: &serde_json::Value,
    stream: bool,
) -> Result<reqwest::Response, OmniError> {
    dispatch_once_detailed(dispatcher, conn, body, stream, 0)
        .await
        .map_err(|f| f.error)
}

/// Upstream #14848: Google Cloud Code / Antigravity `403 VALIDATION_REQUIRED`
/// ("Verify your account to continue") is operator-actionable and non-terminal,
/// unless the body also carries a genuine ban phrase.
pub fn is_account_verification_required(status: u16, body: &str) -> bool {
    if status != 403 {
        return false;
    }
    let lower = body.to_ascii_lowercase();
    if lower.contains("this service has been disabled in this account for violation") {
        return false;
    }
    lower.contains("verify your account to continue") || lower.contains("validation_required")
}

/// Google Cloud Code `403 SUBSCRIPTION_REQUIRED` (#3501) on Antigravity is account/project-scoped
/// and should not trip the provider-wide circuit breaker for other accounts or immediate retries.
pub fn is_antigravity_actionable_403(status: u16, body: &str) -> bool {
    if status != 403 {
        return false;
    }
    let lower = body.to_ascii_lowercase();
    if lower.contains("this service has been disabled in this account for violation") {
        return false;
    }
    lower.contains("subscription_required") || lower.contains("valid license of this product")
}

pub fn classify_cooldown(status: u16, error_text: &str) -> (u64, bool) {
    if let Some(rule) = find_matching_rule(status, error_text) {
        match rule.cooldown_ms {
            Some(ms) if ms > 0 => return (ms, rule.backoff),
            Some(_) => {}
            None => {
                if rule.backoff {
                    return (backoff_cooldown(1), true);
                }
            }
        }
    }
    match status {
        429 => (COOLDOWN_MS.rate_limit, true),
        401 => (COOLDOWN_MS.unauthorized, false),
        402 => (COOLDOWN_MS.payment_required, false),
        403 => (COOLDOWN_MS.unauthorized, false),
        404 => (COOLDOWN_MS.not_found, false),
        503 => (COOLDOWN_MS.service_unavailable, false),
        502 | 504 => (TRANSIENT_INITIAL_MS, true),
        s if s >= 500 => (TRANSIENT_INITIAL_MS, true),
        _ => (COOLDOWN_MS.transient, false),
    }
}

pub fn bounded_cooldown(ms: u64, backoff: bool, max_backoff_level: u32, level: u32) -> u64 {
    if !backoff {
        return ms;
    }
    let lvl = level.min(max_backoff_level);
    let cap = TRANSIENT_MAX_MS.min(COOLDOWN_MS.rate_limit);
    backoff_cooldown(lvl).min(cap).max(ms.min(cap))
}

pub fn parse_retry_after_json(body: Option<&serde_json::Value>) -> Option<u64> {
    let b = body?;
    let inner = b.get("error").unwrap_or(b);
    for candidate in [b, inner] {
        let hit = candidate
            .get("retry_after")
            .and_then(|v| {
                if let Some(sec) = v.as_f64() {
                    Some((sec * 1000.0) as u64)
                } else {
                    v.get("seconds")
                        .and_then(|s| s.as_f64())
                        .map(|s| (s * 1000.0) as u64)
                }
            })
            .or_else(|| candidate.get("reset_after_ms").and_then(|v| v.as_u64()))
            .or_else(|| {
                candidate
                    .get("resetAt")
                    .and_then(|v| v.as_str())
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| {
                        let ms = (dt.with_timezone(&chrono::Utc) - chrono::Utc::now())
                            .num_milliseconds();
                        ms.max(0) as u64
                    })
            });
        if hit.is_some() {
            return hit;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::GzDecoder;
    use std::io::Read;

    #[test]
    fn classify_texts_then_status() {
        let (ms, backoff) = classify_cooldown(500, "quota exhausted");
        assert_eq!(ms, backoff_cooldown(1));
        assert!(backoff);
        let (ms2, backoff2) = classify_cooldown(429, "too many requests");
        assert_eq!(ms2, backoff_cooldown(1));
        assert!(backoff2);
        let (ms3, backoff3) = classify_cooldown(429, "generic message but no rule text");
        assert_eq!(ms3, backoff_cooldown(1));
        assert!(backoff3);
        let (ms4, _) = classify_cooldown(409, "some other body");
        assert_eq!(ms4, COOLDOWN_MS.transient);
    }

    #[test]
    fn terminal_statuses_immediate() {
        let (ms, _) = classify_cooldown(401, "");
        assert_eq!(ms, COOLDOWN_MS.unauthorized);
        let (ms2, _) = classify_cooldown(402, "");
        assert_eq!(ms2, COOLDOWN_MS.payment_required);
        assert!(is_account_verification_required(
            403,
            r#"{"error":{"code":403,"message":"Verify your account to continue.","details":[{"reason":"VALIDATION_REQUIRED"}]}}"#
        ));
        assert!(!is_account_verification_required(
            403,
            "Verify your account to continue. This service has been disabled in this account for violation."
        ));
    }

    #[test]
    fn auth_header_render() {
        assert_eq!(AuthHeader::Bearer.render_value("k", None), "Bearer k");
        assert_eq!(AuthHeader::XApiKey.render_value("k", None), "k");
        assert_eq!(AuthHeader::AuthorizationKey.render_value("k", None), "Key k");
        assert_eq!(AuthHeader::XGoogApiKey.header_name(), Some("x-goog-api-key"));
        assert_eq!(AuthHeader::AzureApiKey.header_name(), Some("api-key"));
        assert_eq!(AuthHeader::None.header_name(), None);
        assert_eq!(AuthHeader::Bearer.render_value("k", Some("Token ")), "Token k");
    }

    #[test]
    fn bounded_cap() {
        let c = bounded_cooldown(1_000, true, 8, 20);
        assert!(c <= TRANSIENT_MAX_MS.min(COOLDOWN_MS.rate_limit));
    }

    #[test]
    fn retry_after_json_shapes() {
        let body: serde_json::Value = serde_json::json!({"retry_after": {"seconds": 5}});
        assert_eq!(parse_retry_after_json(Some(&body)), Some(5_000));
        let nested: serde_json::Value =
            serde_json::json!({"error": {"reset_after_ms": 12_345}});
        assert_eq!(parse_retry_after_json(Some(&nested)), Some(12_345));
        assert_eq!(parse_retry_after_json(None), None);
    }

    #[test]
    fn gzip_roundtrip_and_cch_no_padding() {
        let big = serde_json::json!({"model": "claude-opus-4-7", "text": "a".repeat(6000)}).to_string();
        let gz = gzip_encode_json_body(&big).unwrap();
        assert!(gz.len() < big.len() / 5, "should compress significantly");
        let mut dec = GzDecoder::new(&gz[..]);
        let mut out = String::new();
        dec.read_to_string(&mut out).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed["model"], "claude-opus-4-7");

        // Signed cch= body must NOT receive trailing whitespace padding
        let signed = format!("{{\"billing\":\"cch=a1b2c;\",\"text\":\"{}\"}}", "b".repeat(5000));
        let gz_signed = gzip_encode_json_body(&signed).unwrap();
        let mut dec2 = GzDecoder::new(&gz_signed[..]);
        let mut out2 = String::new();
        dec2.read_to_string(&mut out2).unwrap();
        assert_eq!(out2, signed, "signed cch= body must stay byte-identical");
    }

    #[test]
    fn antigravity_family_lockout_isolates_claude_from_gemini_and_mirrors_agy() {
        resilience::clear_all_lockouts();
        resilience::lock_model("agy", "acct-1", "claude-sonnet-4-6", "rate_limit", 60_000, 1);

        // Same family (Claude) must be locked across both `agy` and `antigravity`
        assert!(resilience::is_model_locked("agy", "acct-1", "claude-opus-4-6"));
        assert!(resilience::is_model_locked("antigravity", "acct-1", "claude-sonnet-4-6"));

        // Different family (Gemini) on the SAME account must stay completely open
        assert!(!resilience::is_model_locked("agy", "acct-1", "gemini-2.5-pro"));
        assert!(!resilience::is_model_locked("antigravity", "acct-1", "gemini-2.5-flash"));

        resilience::clear_model_lock("antigravity", "acct-1", "claude-sonnet-4-6");
        assert!(!resilience::is_model_locked("agy", "acct-1", "claude-opus-4-6"));
    }

    #[tokio::test]
    async fn antigravity_429_refusal_guard_caps_upstream_hits_and_replays_locally() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        crate::antigravity::reset_429_guard_for_tests();
        resilience::clear_all_lockouts();

        let hits = Arc::new(AtomicUsize::new(0));
        let hits_srv = hits.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { break };
                hits_srv.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf).await;
                let body = r#"{"error":{"code":429,"message":"Resource has been exhausted","status":"RESOURCE_EXHAUSTED"}}"#;
                let resp = format!(
                    "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });

        let dispatcher = Dispatcher::new();
        let url = format!("http://127.0.0.1:{port}/v1internal:generateContent");
        // Provide 3 fallback hosts: default budget (2) must stop after 2 hits, NOT 3!
        let conn = ConnectionEndpoint {
            provider: "agy".into(),
            connection_id: "ag-test-conn".into(),
            model: "claude-sonnet-4-6".into(),
            auth_type: AuthScheme::OAuth,
            base_urls: vec![url.clone(), url.clone(), url],
            api_key: "tok_test".into(),
            auth_header: AuthHeader::Bearer,
            auth_prefix: None,
            extra_headers: vec![],
            timeout_ms: Some(5_000),
            raw_body_override: None,
            force_gzip: false,
            proxy_url: None,
        };
        let env_body = serde_json::json!({
            "project": "proj-1",
            "model": "claude-sonnet-4-6",
            "requestId": "agent/a/1/b/2",
            "request": {
                "sessionId": "123",
                "contents": [{"role": "user", "parts": [{"text": "refuse me"}]}]
            }
        });

        let err1 = dispatch_once_detailed(&dispatcher, &conn, &env_body, false, 0)
            .await
            .unwrap_err();
        assert_eq!(err1.error.status(), 429);
        assert!(!err1.is_local_refusal_replay);
        assert_eq!(hits.load(Ordering::SeqCst), 2, "must cap 429 retries at budget=2 across 3 hosts");

        // Second identical request (even with different requestId/sessionId) must replay locally with ZERO upstream calls!
        let env_body_retry = serde_json::json!({
            "project": "proj-1",
            "model": "claude-sonnet-4-6",
            "requestId": "agent/new/999/z/2",
            "request": {
                "sessionId": "999",
                "contents": [{"role": "user", "parts": [{"text": "refuse me"}]}]
            }
        });
        let err2 = dispatch_once_detailed(&dispatcher, &conn, &env_body_retry, false, 0)
            .await
            .unwrap_err();
        assert_eq!(err2.error.status(), 429);
        assert!(err2.is_local_refusal_replay);
        assert_eq!(hits.load(Ordering::SeqCst), 2, "identical retry must hit 0 upstream requests");
    }

    #[tokio::test]
    async fn antigravity_403_retries_once_without_x_goog_user_project() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        resilience::clear_all_lockouts();
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_srv = hits.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { break };
                hits_srv.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req_text = String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase();
                let resp = if req_text.contains("x-goog-user-project:") {
                    let body = r#"{"error":{"code":403,"message":"SUBSCRIPTION_REQUIRED","status":"PERMISSION_DENIED"}}"#;
                    format!(
                        "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                } else {
                    let body = r#"{"response":{"candidates":[{"content":{"parts":[{"text":"ok"}]}}]}}"#;
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                };
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });

        let dispatcher = Dispatcher::new();
        let conn = ConnectionEndpoint {
            provider: "antigravity".into(),
            connection_id: "ag-403-retry".into(),
            model: "gemini-2.5-flash".into(),
            auth_type: AuthScheme::OAuth,
            base_urls: vec![format!("http://127.0.0.1:{port}/v1internal:streamGenerateContent?alt=sse")],
            api_key: "tok_test".into(),
            auth_header: AuthHeader::Bearer,
            auth_prefix: None,
            extra_headers: vec![("x-goog-user-project".into(), "stale-proj".into())],
            timeout_ms: Some(5_000),
            raw_body_override: None,
            force_gzip: false,
            proxy_url: None,
        };

        let resp = dispatch_once_detailed(&dispatcher, &conn, &serde_json::json!({}), false, 0)
            .await
            .expect("retry without x-goog-user-project should succeed");
        assert_eq!(resp.status().as_u16(), 200);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        assert!(is_antigravity_actionable_403(
            403,
            r#"{"error":{"message":"You do not have a valid license of this product (#3501)","details":[{"reason":"SUBSCRIPTION_REQUIRED"}]}}"#
        ));
    }
}
