use super::{ErrorBody, ErrorInfo, ErrorReason};

pub const TRANSIENT_COOLDOWN_MS: u64 = 5_000;

pub const TRANSIENT_INITIAL_MS: u64 = 5_000;
pub const TRANSIENT_MAX_MS: u64 = 60_000;

pub const BACKOFF_BASE_MS: u64 = 1_000;
pub const BACKOFF_MAX_MS: u64 = 120_000;
pub const BACKOFF_MAX_LEVEL: u32 = 15;

// COOLDOWN_MS errorConfig.ts
pub const COOLDOWN_MS: &Cooldowns = &Cooldowns {
    unauthorized: 120_000,
    payment_required: 120_000,
    not_found: 120_000,
    not_found_local: 5_000,
    transient_initial: 5_000,
    transient_max: 60_000,
    transient: 5_000,
    request_not_allowed: 5_000,
    request_rejected: 300_000,
    request_rejected_repeat: 900_000,
    rate_limit: 120_000,
    service_unavailable: 2_000,
    auth_expired: 120_000,
    geo_blocked: 86_400_000,
    gcp_project_required: 86_400_000,
};

pub struct Cooldowns {
    pub unauthorized: u64,
    pub payment_required: u64,
    pub not_found: u64,
    pub not_found_local: u64,
    pub transient_initial: u64,
    pub transient_max: u64,
    pub transient: u64,
    pub request_not_allowed: u64,
    pub request_rejected: u64,
    pub request_rejected_repeat: u64,
    pub rate_limit: u64,
    pub service_unavailable: u64,
    pub auth_expired: u64,
    pub geo_blocked: u64,
    pub gcp_project_required: u64,
}

pub fn error_info_for_status(status: u16) -> (&'static str, &'static str) {
    match status {
        400 => ("invalid_request_error", "bad_request"),
        401 => ("authentication_error", "invalid_api_key"),
        402 => ("billing_error", "payment_required"),
        403 => ("permission_error", "permission_denied"),
        404 => ("invalid_request_error", "model_not_found"),
        406 => ("invalid_request_error", "model_not_supported"),
        410 => ("invalid_request_error", "model_shutdown"),
        429 => ("rate_limit_error", "rate_limit_exceeded"),
        499 => ("client_disconnected", "client_disconnected"),
        500 => ("server_error", "internal_server_error"),
        502 => ("server_error", "bad_gateway"),
        503 => ("server_error", "service_unavailable"),
        504 => ("server_error", "gateway_timeout"),
        s if s >= 500 => ("server_error", "internal_server_error"),
        _ => ("invalid_request_error", ""),
    }
}

pub fn default_error_message(status: u16) -> &'static str {
    match status {
        400 => "Bad request",
        401 => "Invalid API key provided",
        402 => "Payment required",
        403 => "Permission denied",
        404 => "Model not found",
        406 => "Model not supported",
        410 => "Model has been shut down",
        429 => "Rate limit exceeded",
        499 => "Client disconnected",
        500 => "Internal server error",
        502 => "Bad gateway - upstream provider error",
        503 => "Service temporarily unavailable",
        504 => "Gateway timeout",
        _ => "An error occurred",
    }
}

pub fn backoff_cooldown(level: u32) -> u64 {
    let lvl = level.min(BACKOFF_MAX_LEVEL);
    (BACKOFF_BASE_MS.saturating_mul(1u64 << lvl.min(32)) ).min(BACKOFF_MAX_MS)
}

// ── Error rules (errorConfig.ts ERROR_RULES) ────────────────────────

pub struct ErrorRule {
    pub id: &'static str,
    /// None → match only by status.
    pub text: Option<&'static str>,
    pub status: Option<u16>,
    pub cooldown_ms: Option<u64>,
    pub backoff: bool,
    pub reason: ErrorReason,
}

macro_rules! rule {
    ($id:expr, $text:expr, $reason:expr) => {
        ErrorRule {
            id: $id,
            text: $text,
            status: None,
            cooldown_ms: None,
            backoff: true,
            reason: $reason,
        }
    };
    ($id:expr, $text:expr, $reason:expr, $backoff:expr) => {
        ErrorRule {
            id: $id,
            text: $text,
            status: None,
            cooldown_ms: None,
            backoff: $backoff,
            reason: $reason,
        }
    };
}

pub static ERROR_RULES: &[ErrorRule] = &[
    ErrorRule {
        id: "no_credentials",
        text: Some("no credentials"),
        status: None,
        cooldown_ms: Some(120_000),
        backoff: false,
        reason: ErrorReason::AuthError,
    },
    ErrorRule {
        id: "request_not_allowed",
        text: Some("request not allowed"),
        status: None,
        cooldown_ms: Some(5_000),
        backoff: false,
        reason: ErrorReason::RateLimitExceeded,
    },
    ErrorRule {
        id: "improperly_formed_request",
        text: Some("improperly formed request"),
        status: None,
        cooldown_ms: Some(0),
        backoff: false,
        reason: ErrorReason::ModelCapacity,
    },
    rule!("rate_limit", Some("rate limit"), ErrorReason::RateLimitExceeded),
    rule!("too_many_requests", Some("too many requests"), ErrorReason::RateLimitExceeded),
    rule!("hour_quota_exceeded", Some("hour quota"), ErrorReason::QuotaExhausted),
    rule!("quota_has_been_exceeded", Some("quota has been exceeded"), ErrorReason::QuotaExhausted),
    rule!("quota_exceeded", Some("quota exceeded"), ErrorReason::QuotaExhausted),
    rule!("quota_will_reset", Some("quota will reset"), ErrorReason::QuotaExhausted),
    rule!("capacity_exhausted", Some("exhausted your capacity"), ErrorReason::QuotaExhausted),
    rule!("quota_exhausted", Some("quota exhausted"), ErrorReason::QuotaExhausted),
    rule!(
        "free_tier_exhausted",
        Some("free tier of the model has been exhausted"),
        ErrorReason::QuotaExhausted
    ),
    rule!("out_of_extra_usage", Some("out of extra usage"), ErrorReason::QuotaExhausted),
    rule!("extra_usage_required", Some("extra usage required"), ErrorReason::QuotaExhausted),
    rule!("capacity", Some("capacity"), ErrorReason::ModelCapacity),
    rule!("overloaded", Some("overloaded"), ErrorReason::ModelCapacity),
    rule!("high_demand", Some("high demand"), ErrorReason::ModelCapacity),
    ErrorRule {
        id: "status_401",
        text: None,
        status: Some(401),
        cooldown_ms: Some(0),
        backoff: false,
        reason: ErrorReason::AuthError,
    },
    ErrorRule {
        id: "status_402",
        text: None,
        status: Some(402),
        cooldown_ms: Some(0),
        backoff: true,
        reason: ErrorReason::QuotaExhausted,
    },
    ErrorRule {
        id: "status_403",
        text: None,
        status: Some(403),
        cooldown_ms: Some(0),
        backoff: false,
        reason: ErrorReason::Unknown,
    },
    ErrorRule {
        id: "status_404",
        text: None,
        status: Some(404),
        cooldown_ms: Some(120_000),
        backoff: false,
        reason: ErrorReason::Unknown,
    },
    ErrorRule {
        id: "status_406",
        text: None,
        status: Some(406),
        cooldown_ms: None,
        backoff: true,
        reason: ErrorReason::ServerError,
    },
    ErrorRule {
        id: "status_408",
        text: None,
        status: Some(408),
        cooldown_ms: None,
        backoff: true,
        reason: ErrorReason::ServerError,
    },
    ErrorRule {
        id: "status_429",
        text: None,
        status: Some(429),
        cooldown_ms: None,
        backoff: true,
        reason: ErrorReason::RateLimitExceeded,
    },
    ErrorRule {
        id: "status_500",
        text: None,
        status: Some(500),
        cooldown_ms: None,
        backoff: true,
        reason: ErrorReason::ServerError,
    },
    ErrorRule {
        id: "status_502",
        text: None,
        status: Some(502),
        cooldown_ms: None,
        backoff: true,
        reason: ErrorReason::ServerError,
    },
    ErrorRule {
        id: "status_503",
        text: None,
        status: Some(503),
        cooldown_ms: None,
        backoff: true,
        reason: ErrorReason::ServerError,
    },
    ErrorRule {
        id: "status_504",
        text: None,
        status: Some(504),
        cooldown_ms: None,
        backoff: true,
        reason: ErrorReason::ServerError,
    },
];

pub fn match_by_text(message: &str) -> Option<&'static ErrorRule> {
    let lower = message.to_lowercase();
    if lower.is_empty() {
        return None;
    }
    ERROR_RULES
        .iter()
        .find(|r| r.text.map(|t| lower.contains(t)).unwrap_or(false))
}

pub fn match_by_status(status: u16) -> Option<&'static ErrorRule> {
    ERROR_RULES.iter().find(|r| r.status == Some(status))
}

pub fn find_matching_rule(status: u16, message: &str) -> Option<&'static ErrorRule> {
    match_by_text(message).or_else(|| match_by_status(status))
}

// ── Sanitization port (errorSanitization.ts, load-bearing subset) ───

pub const MAX_ERROR_LEN: usize = 512;

/// Collapse credential-bearing URL segments.
fn redact_url_credentials(v: &str) -> String {
    // http(s)://user:pass@host → strip userinfo
    let mut out = v.to_string();
    if let Some(scheme_end) = out.find("://") {
        let after = scheme_end + 3;
        if let Some(rel_end) = out[after..].find('/').map(|i| after + i) {
            let userinfo = &out[after..rel_end];
            if userinfo.contains('@') {
                let at = after + userinfo.find('@').unwrap();
                out = format!("{}{}", &out[..after], &out[at + 1..]);
            }
        }
    }
    out
}

fn redact_labeled(v: &str) -> String {
    // api_key=… / api-key: … / access_token: … / bearer/Basic — cut value.
    let mut out = v.to_string();
    const LABELS: [&str; 8] = [
        "api_key", "api-key", "apikey", "access_token", "refresh_token", "password", "secret",
        "authorization",
    ];
    for label in LABELS {
        let mut search_from = 0;
        loop {
            let lower = out.to_ascii_lowercase();
            let Some(rel) = lower[search_from..].find(label) else {
                break;
            };
            let pos = search_from + rel;
            let after_label = pos + label.len();
            let rest = &out[after_label..];
            let mut offset = 0usize;
            for ch in rest.chars() {
                if ch == ' ' || ch == '=' || ch == ':' || ch == '"' || ch == '\'' {
                    offset += ch.len_utf8();
                } else {
                    break;
                }
            }
            if offset == 0 || !rest[..offset].contains(|c| c == '=' || c == ':') {
                search_from = after_label;
                continue;
            }
            let vstart = after_label + offset;
            if out[vstart..].starts_with("[REDACTED]") {
                search_from = vstart + 10;
                continue;
            }
            let vend = out[vstart..]
                .find(|c: char| {
                    c.is_whitespace() || c == '&' || c == '"' || c == '\'' || c == ',' || c == ';' || c == ')' || c == '}'
                })
                .map(|i| vstart + i)
                .unwrap_or(out.len());
            if vend > vstart {
                out = format!("{}[REDACTED]{}", &out[..vstart], &out[vend..]);
                search_from = vstart + 10;
            } else {
                search_from = vstart;
            }
        }
    }
    // Bearer / Basic
    for scheme in ["Bearer ", "bearer ", "Basic ", "basic "] {
        let mut idx = 0;
        while let Some(pos) = out[idx..].find(scheme) {
            let start = idx + pos;
            let vstart = start + scheme.len();
            if out[vstart..].starts_with("[REDACTED]") {
                idx = vstart + 10;
                continue;
            }
            let vend = out[vstart..]
                .find(|c: char| {
                    !c.is_ascii_alphanumeric()
                        && c != '.'
                        && c != '_'
                        && c != '~'
                        && c != '/'
                        && c != '+'
                        && c != '-'
                        && c != '='
                })
                .map(|i| vstart + i)
                .unwrap_or(out.len());
            if vend > vstart {
                out = format!("{}[REDACTED]{}", &out[..vstart], &out[vend..]);
                idx = vstart + 10;
            } else {
                break;
            }
        }
    }
    out
}

fn strip_stack_tail(v: &str) -> String {
    // Cut at "    at " (V8 stack frames) or "at line " markers.
    let mut out = v.to_string();
    if let Some(pos) = out.find("\n    at ") {
        out.truncate(pos);
    }
    if let Some(pos) = out.find(" at line ") {
        let cut = out[..pos].ends_with(|c: char| c.is_ascii_digit());
        if cut {
            out.truncate(pos);
        }
    }
    out
}

pub fn sanitize_error_message(message: &str) -> String {
    let mut s: String = message.chars().take(MAX_ERROR_LEN * 2).collect();
    s = strip_stack_tail(&s);
    s = redact_url_credentials(&s);
    s = redact_labeled(&s);
    truncate_sanitized(&s)
}

fn truncate_sanitized(v: &str) -> String {
    if v.len() <= MAX_ERROR_LEN {
        return v.to_string();
    }
    let cut = v.char_indices().nth(MAX_ERROR_LEN).map(|(i, _)| i).unwrap_or(MAX_ERROR_LEN);
    v[..cut].to_string()
}

/// Cap client-facing Retry-After seconds via CLIENT_RETRY_AFTER_MAX_SECONDS (v3.8.55).
pub fn cap_client_retry_after_secs(secs: f64) -> f64 {
    let clamped = secs.max(0.0);
    if let Some(max) = std::env::var("CLIENT_RETRY_AFTER_MAX_SECONDS")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|&v| v > 0.0)
    {
        clamped.min(max)
    } else {
        clamped
    }
}

/// Build canonical OpenAI-shaped error body. Message sanitized.
pub fn build_error_body(status: u16, message: &str) -> ErrorBody {
    build_error_body_with_retry(status, message, None, None)
}

pub fn build_error_body_with_retry(
    status: u16,
    message: &str,
    retry_after_secs: Option<f64>,
    reset_at: Option<String>,
) -> ErrorBody {
    let (kind, code) = error_info_for_status(status);
    let safe = sanitize_error_message(message);
    let msg = if safe.is_empty() {
        default_error_message(status).to_string()
    } else {
        safe
    };
    let reason = find_matching_rule(status, message)
        .map(|r| r.reason)
        .unwrap_or(match status {
            401 | 403 => ErrorReason::AuthError,
            429 => ErrorReason::RateLimitExceeded,
            402 => ErrorReason::QuotaExhausted,
            s if s >= 500 => ErrorReason::ServerError,
            _ => ErrorReason::Unknown,
        });
    ErrorBody {
        error: ErrorInfo {
            message: msg,
            kind: kind.to_string(),
            code: code.to_string(),
            reason: Some(reason.as_str().to_string()),
            retry_after: retry_after_secs.map(cap_client_retry_after_secs),
            reset_at,
        },
        upstream_details: None,
    }
}

/// Build Anthropic Messages API canonical error JSON envelope.
pub fn build_anthropic_error_body(status: u16, message: &str) -> serde_json::Value {
    let safe = sanitize_error_message(message);
    let msg = if safe.is_empty() {
        default_error_message(status).to_string()
    } else {
        safe
    };
    let err_type = match status {
        400 => "invalid_request_error",
        401 => "authentication_error",
        403 => "permission_error",
        404 => "not_found_error",
        429 => "rate_limit_error",
        529 => "overloaded_error",
        _ => "api_error",
    };
    serde_json::json!({
        "type": "error",
        "error": {
            "type": err_type,
            "message": msg
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_redacted() {
        let s = sanitize_error_message("auth failed with Bearer sk-abcdef1234567890");
        assert!(s.contains("[REDACTED]"), "{s}");
        assert!(!s.contains("sk-abcdef"), "{s}");
    }

    #[test]
    fn stack_tail_stripped() {
        let s = sanitize_error_message("boom\n    at foo (/src/x.ts:1:1)\n    at bar");
        assert!(!s.contains("/src/x.ts"), "{s}");
        assert!(s.starts_with("boom"), "{s}");
    }

    #[test]
    fn truncated() {
        let long = "x".repeat(2000);
        let s = sanitize_error_message(&long);
        assert!(s.chars().count() <= MAX_ERROR_LEN, "{}", s.chars().count());
    }

    #[test]
    fn error_body_shape() {
        let b = build_error_body(429, "rate limit hit");
        assert_eq!(b.error.kind, "rate_limit_error");
        assert_eq!(b.error.code, "rate_limit_exceeded");
        assert_eq!(b.error.reason.as_deref(), Some("rate_limit_exceeded"));
    }

    #[test]
    fn backoff_caps() {
        assert_eq!(backoff_cooldown(0), 1_000);
        assert_eq!(backoff_cooldown(4), 16_000);
        assert_eq!(backoff_cooldown(30), BACKOFF_MAX_MS);
    }

    #[test]
    fn rule_match_text_wins() {
        assert_eq!(find_matching_rule(500, "quota exhausted").unwrap().id, "quota_exhausted");
        assert_eq!(match_by_status(429).unwrap().id, "status_429");
    }
}
