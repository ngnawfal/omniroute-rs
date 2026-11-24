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
}

pub struct Dispatcher {
    pub client: reqwest::Client,
}

impl Default for Dispatcher {
    fn default() -> Self {
        Self::new()
    }
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
        }
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
) -> reqwest::RequestBuilder {
    let mut req = dispatcher
        .client
        .post(url)
        .header("Content-Type", "application/json");
    if let Some(ms) = conn.timeout_ms {
        req = req.timeout(Duration::from_millis(ms));
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
    if stream && !conn.extra_headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("accept")) {
        req = req.header("Accept", "text/event-stream");
    }
    for (k, v) in &conn.extra_headers {
        req = req.header(k.as_str(), v.as_str());
    }
    req
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
            let resp = build_post_request(dispatcher, url, conn, stream)
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
                // Peek body to detect gzip JSON parse refusal; if not a refusal, reconstruct response?
                // Since reqwest::Response is consumed on `.text()`, only consume when status == 400
                // and resend uncompressed if refusal detected; otherwise return Upstream error directly.
                let text = resp.text().await.unwrap_or_default();
                let not_json = text.contains("The request body is not valid JSON")
                    || text.contains("Failed to parse request: could not parse request body as JSON");
                if not_json {
                    latch_origin_gzip(&origin);
                    tracing::warn!(origin = %origin, status, "origin refused gzip body; resending uncompressed");
                    return build_post_request(dispatcher, url, conn, stream)
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
                return build_post_request(dispatcher, url, conn, stream)
                    .body(serialized)
                    .send()
                    .await
                    .map_err(|e| OmniError::Network(e.to_string()));
            }
            return Ok(resp);
        }
    }

    build_post_request(dispatcher, url, conn, stream)
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

    for url in &conn.base_urls {
        if url.is_empty() {
            continue;
        }
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
                });
                continue;
            }
        };

        let status = resp.status().as_u16();
        if (200..300).contains(&status) {
            resilience::record_connection_success(&conn.provider, &conn.connection_id);
            resilience::record_provider_success(&conn.provider);
            resilience::clear_model_lock(&conn.provider, &conn.connection_id, &conn.model);
            return Ok(resp);
        }

        let header_retry_ms = retry_after_from_header(&resp).map(|d| d.as_millis() as u64);
        let payload = resp.text().await.unwrap_or_default();
        let body_json = serde_json::from_str::<serde_json::Value>(&payload).ok();
        let body_retry_ms = parse_retry_after_json(body_json.as_ref());
        let retry_after_ms = match (header_retry_ms, body_retry_ms) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };

        let (base_cd, use_backoff) = classify_cooldown(status, &payload);
        let mut cooldown_ms =
            bounded_cooldown(base_cd, use_backoff, profile.max_backoff_level, attempt_level);
        if let Some(ra) = retry_after_ms {
            cooldown_ms = cooldown_ms.max(ra);
        }

        let is_terminal = resilience::is_terminal_status(status);
        let is_rate_limit = status == 429 || payload.to_ascii_lowercase().contains("rate limit");

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

        last = Some(DispatchFailure {
            error: OmniError::Upstream {
                status,
                body: Some(payload),
            },
            retry_after_ms,
            cooldown_ms,
            is_terminal,
            is_rate_limit,
        });
    }

    Err(last.unwrap_or(DispatchFailure {
        error: OmniError::Network("no urls configured".into()),
        retry_after_ms: None,
        cooldown_ms: TRANSIENT_INITIAL_MS,
        is_terminal: false,
        is_rate_limit: false,
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
}
