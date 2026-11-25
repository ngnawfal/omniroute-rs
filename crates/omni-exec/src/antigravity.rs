// Antigravity / `agy` Anti-Ban & Upstream Parity Safeguards (aligned 1:1 with OmniRoute v3.8.55):
//   1. Native CLI/IDE User-Agent & Proxy/SDK Header Scrubbing (antigravityHeaders.ts + antigravityHeaderScrub.ts)
//   2. Native Envelope Identity: UUIDv4 requestId + FNV-1a stable sessionId (antigravityIdentity.ts)
//   3. Competing-Agent Prompt & Harness Marker Sanitizer (antigravity.ts #10420)
//   4. Anti-Hammering 429 Refusal Guard & Bounded Retry Budget (antigravity429Guard.ts)
//   5. Per-Model-Family Quota Cooldown Scoping: gemini / claude / argon (antigravityQuotaFamily.ts)

use rand::RngCore;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

// ── 1. Native User-Agent & Proxy/Fingerprint Header Scrubbing ───────

/// Upstream `ANTIGRAVITY_IDE_FALLBACK_VERSION` (open-sse/services/antigravityVersion.ts).
/// Cloud Code rejects older IDE versions with 403 SUBSCRIPTION_REQUIRED (#3501) on consumer accounts.
pub const DEFAULT_ANTIGRAVITY_IDE_VERSION: &str = "2.5.5";
pub const DEFAULT_ANTIGRAVITY_CLI_VERSION: &str = "1.4.4";
pub const DEFAULT_ANTIGRAVITY_CLI_CL: &str = "1003595037";

pub fn is_antigravity_provider(provider: &str) -> bool {
    matches!(provider.trim().to_ascii_lowercase().as_str(), "antigravity" | "agy")
}

pub fn resolve_client_profile(provider: &str, provider_specific: &Value) -> &'static str {
    if provider.eq_ignore_ascii_case("agy") {
        return "cli";
    }
    if provider_specific
        .get("clientProfile")
        .and_then(Value::as_str)
        .map(|s| s.eq_ignore_ascii_case("cli"))
        .unwrap_or(false)
    {
        return "cli";
    }
    "ide"
}

fn go_os_token() -> &'static str {
    match std::env::consts::OS {
        "windows" => "windows",
        "macos" => "darwin",
        "linux" => "linux",
        "freebsd" => "freebsd",
        "openbsd" => "openbsd",
        other => other,
    }
}

fn go_arch_token() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "x86" => "386",
        "aarch64" => "arm64",
        "arm" => "arm",
        other => other,
    }
}

/// Returns the authentic upstream `User-Agent` for Antigravity inference calls:
/// - `agy` CLI: `antigravity/cli/<ver> (aidev_client; os_type=<goos>; arch=<goarch>; cl=<cl>; auth_method=consumer)`
/// - `antigravity` IDE: `antigravity/ide/<ver> darwin/arm64` (pinned to macOS arm64 per upstream #8098)
pub fn antigravity_content_user_agent(provider: &str, provider_specific: &Value) -> String {
    let profile = resolve_client_profile(provider, provider_specific);
    if profile == "cli" {
        let ver = std::env::var("ANTIGRAVITY_CLI_VERSION")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_ANTIGRAVITY_CLI_VERSION.to_string());
        let cl = std::env::var("ANTIGRAVITY_CLI_CL")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                if ver == DEFAULT_ANTIGRAVITY_CLI_VERSION {
                    DEFAULT_ANTIGRAVITY_CLI_CL.to_string()
                } else {
                    String::new()
                }
            });
        let cl_tok = if cl.is_empty() {
            String::new()
        } else {
            format!("cl={cl}; ")
        };
        format!(
            "antigravity/cli/{ver} (aidev_client; os_type={}; arch={}; {cl_tok}auth_method=consumer)",
            go_os_token(),
            go_arch_token()
        )
    } else {
        let ver = std::env::var("ANTIGRAVITY_IDE_VERSION")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_ANTIGRAVITY_IDE_VERSION.to_string());
        format!("antigravity/ide/{ver} darwin/arm64")
    }
}

/// Proxy, SDK, and Chromium fingerprint headers stripped before dispatching to Antigravity
/// (exact port of `open-sse/services/antigravityHeaderScrub.ts`).
const SCRUB_HEADERS: &[&str] = &[
    "x-forwarded-for",
    "x-forwarded-host",
    "x-forwarded-proto",
    "x-forwarded-port",
    "x-real-ip",
    "forwarded",
    "via",
    "x-title",
    "x-stainless-lang",
    "x-stainless-package-version",
    "x-stainless-os",
    "x-stainless-arch",
    "x-stainless-runtime",
    "x-stainless-runtime-version",
    "x-stainless-timeout",
    "x-stainless-retry-count",
    "x-stainless-helper-method",
    "http-referer",
    "referer",
    "sec-ch-ua",
    "sec-ch-ua-mobile",
    "sec-ch-ua-platform",
    "sec-fetch-mode",
    "sec-fetch-site",
    "sec-fetch-dest",
    "priority",
    "accept-encoding",
];

pub fn should_scrub_header(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    lower.starts_with("x-omniroute-") || SCRUB_HEADERS.contains(&lower.as_str())
}

/// Scrubs leaking proxy/SDK/Chromium headers and sets Node.js-native `Accept-Encoding: gzip, deflate, br`.
pub fn scrub_proxy_and_fingerprint_headers(headers: &mut Vec<(String, String)>) {
    headers.retain(|(k, _)| !should_scrub_header(k));
    headers.push(("Accept-Encoding".to_string(), "gzip, deflate, br".to_string()));
}

// ── 2. Envelope Identity (UUIDv4 requestId + FNV-1a stable sessionId) ──

const FNV_OFFSET_I64: i64 = -3750763034362895579;
const FNV_PRIME_I64: i64 = 1099511628211;

pub fn generate_uuid_v4() -> String {
    let mut b = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    )
}

/// Deterministic 64-bit signed FNV-1a hash matching `deriveAntigravitySessionId()` in
/// `open-sse/services/antigravityIdentity.ts` byte-for-byte.
pub fn derive_antigravity_session_id(account_key: &str) -> Option<String> {
    let trimmed = account_key.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut hash: i64 = FNV_OFFSET_I64;
    for byte in trimmed.as_bytes() {
        hash = (hash ^ (*byte as i64)).wrapping_mul(FNV_PRIME_I64);
    }
    Some(hash.to_string())
}

pub fn generate_random_session_id() -> String {
    let mut buf = [0u8; 8];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    let val = u64::from_be_bytes(buf) % 9_000_000_000_000_000_000u64;
    format!("-{val}")
}

#[derive(Debug, Clone)]
pub struct AntigravityEnvelopeIdentity {
    pub request_id: String,
    pub session_id: String,
}

pub fn build_envelope_identity(
    provider_specific: &Value,
    connection_id: &str,
) -> AntigravityEnvelopeIdentity {
    let agent_id = generate_uuid_v4();
    let trajectory_id = generate_uuid_v4();
    let now_ms = chrono::Utc::now().timestamp_millis();
    let request_id = format!("agent/{agent_id}/{now_ms}/{trajectory_id}/2");

    let account_key = provider_specific
        .get("email")
        .or_else(|| provider_specific.get("accountId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            let cid = connection_id.trim();
            if cid.is_empty() { None } else { Some(cid) }
        });

    let session_id = account_key
        .and_then(derive_antigravity_session_id)
        .unwrap_or_else(generate_random_session_id);

    AntigravityEnvelopeIdentity {
        request_id,
        session_id,
    }
}

/// Applies upstream generation defaults (`topK = 40`, `topP = 1.0`), strips unsupported
/// `HARM_CATEGORY_CIVIC_INTEGRITY` safety settings, and removes OMP harness telemetry keys.
pub fn apply_antigravity_request_defaults(gemini_body: &mut Value, session_id: &str) {
    let Some(obj) = gemini_body.as_object_mut() else {
        return;
    };
    if !obj.contains_key("sessionId") && !session_id.is_empty() {
        obj.insert("sessionId".into(), Value::String(session_id.to_string()));
    }

    // Remove OMP harness telemetry keys if present
    obj.remove("used_claude");
    obj.remove("used_claude_conservative");
    if let Some(labels) = obj.get_mut("labels").and_then(Value::as_object_mut) {
        labels.remove("used_claude");
        labels.remove("used_claude_conservative");
    }

    // Apply upstream generation defaults
    let gen_cfg = obj
        .entry("generationConfig")
        .or_insert_with(|| json!({}));
    if let Some(gc) = gen_cfg.as_object_mut() {
        gc.entry("topK").or_insert_with(|| json!(40));
        gc.entry("topP").or_insert_with(|| json!(1.0));
        // Ensure maxOutputTokens exceeds thinkingBudget when thinkingConfig is active
        let thinking_budget = gc
            .get("thinkingConfig")
            .and_then(|t| t.get("thinkingBudget"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if thinking_budget > 0 {
            if let Some(max_out) = gc.get("maxOutputTokens").and_then(Value::as_u64) {
                if max_out <= thinking_budget {
                    gc.insert("maxOutputTokens".into(), json!(thinking_budget + 8192));
                }
            }
        }
    }

    // Strip unsupported safety category that Cloud Code rejects
    if let Some(safety) = obj.get_mut("safetySettings").and_then(Value::as_array_mut) {
        safety.retain(|entry| {
            entry
                .get("category")
                .and_then(Value::as_str)
                .map(|c| c != "HARM_CATEGORY_CIVIC_INTEGRITY")
                .unwrap_or(true)
        });
    }
}

// ── 3. Competing-Agent Prompt & Harness Marker Sanitizer ────────────

const COMPETITIVE_AGENT_MARKERS: &[&str] = &[
    "you are a claude agent",
    "built on anthropic's claude agent sdk",
    "you are claude code",
    "you are an ai assistant created by anthropic",
];

const HARNESS_REWRITES: &[(&str, &str)] = &[
    ("<system-conventions>", "<conventions>"),
    ("</system-conventions>", "</conventions>"),
    ("<system_conventions>", "<conventions>"),
    ("</system_conventions>", "</conventions>"),
    ("<system-directive>", "<instructions>"),
    ("</system-directive>", "</instructions>"),
    ("<system_directive>", "<instructions>"),
    ("</system_directive>", "</instructions>"),
    ("<critical>", "<important>"),
    ("</critical>", "</important>"),
    ("oh my pi coding harness", "AI coding assistant"),
    ("oh my pi", "coding assistant"),
    ("omp live", "coding assistant live"),
];

fn replace_case_insensitive(input: &str, needle_lower: &str, replacement: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut lower = input.to_ascii_lowercase();
    let mut src = input;
    while let Some(pos) = lower.find(needle_lower) {
        result.push_str(&src[..pos]);
        result.push_str(replacement);
        let next = pos + needle_lower.len();
        src = &src[next..];
        lower = lower[next..].to_string();
    }
    result.push_str(src);
    result
}

fn strip_marker_to_eol(input: &str, marker_lower: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut lower = input.to_ascii_lowercase();
    let mut src = input;
    while let Some(pos) = lower.find(marker_lower) {
        // Ensure word boundary before marker
        let prev_ok = pos == 0 || !src.as_bytes()[pos - 1].is_ascii_alphanumeric();
        if !prev_ok {
            let next = pos + marker_lower.len();
            out.push_str(&src[..next]);
            src = &src[next..];
            lower = lower[next..].to_string();
            continue;
        }
        out.push_str(&src[..pos]);
        let rest = &src[pos..];
        let eol = rest.find('\n').unwrap_or(rest.len());
        src = &rest[eol..];
        lower = src.to_ascii_lowercase();
    }
    out.push_str(src);
    out
}

pub fn sanitize_antigravity_prompt_text(text: &str) -> String {
    let mut cur = text.to_string();
    for marker in COMPETITIVE_AGENT_MARKERS {
        if cur.to_ascii_lowercase().contains(marker) {
            cur = strip_marker_to_eol(&cur, marker);
        }
    }
    for (from, to) in HARNESS_REWRITES {
        if cur.to_ascii_lowercase().contains(from) {
            cur = replace_case_insensitive(&cur, from, to);
        }
    }
    while cur.contains("\n\n\n") {
        cur = cur.replace("\n\n\n", "\n\n");
    }
    cur.trim_start().to_string()
}

/// Normalizes `systemInstruction.parts` inside a Gemini/Cloud Code request body before
/// dispatch to Antigravity so competing-agent identity lines and OMP harness tags do not
/// trigger Google's false `429 RESOURCE_EXHAUSTED` content filter (#10420).
pub fn normalize_antigravity_system_instruction(gemini_body: &mut Value) -> usize {
    let Some(obj) = gemini_body.as_object_mut() else {
        return 0;
    };
    let mut edits = 0usize;
    let should_remove = if let Some(sys) = obj.get_mut("systemInstruction").and_then(Value::as_object_mut) {
        if let Some(parts) = sys.get_mut("parts").and_then(Value::as_array_mut) {
            let before_len = parts.len();
            parts.retain(|p| !p.as_object().map(|m| m.is_empty()).unwrap_or(false));
            if parts.len() != before_len {
                edits += 1;
            }
            for part in parts.iter_mut() {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    if !text.is_empty() {
                        let cleaned = sanitize_antigravity_prompt_text(text);
                        if cleaned != text {
                            part["text"] = Value::String(cleaned);
                            edits += 1;
                        }
                    }
                }
            }
            parts.is_empty()
        } else {
            false
        }
    } else {
        false
    };
    if should_remove {
        obj.remove("systemInstruction");
    }
    edits
}

// ── 4. Anti-Hammering 429 Refusal Guard (antigravity429Guard.ts) ────

pub const ANTIGRAVITY_429_ATTEMPTS_DEFAULT: u32 = 2;
pub const ANTIGRAVITY_429_ATTEMPTS_MAX: u32 = 12;
pub const REFUSAL_BLOCK_BASE_MS: i64 = 2 * 60 * 1000;
pub const REFUSAL_BLOCK_MAX_MS: i64 = 30 * 60 * 1000;
const REFUSAL_STRIKE_WINDOW_MS: i64 = 60 * 60 * 1000;
const REFUSAL_MEMORY_MAX_ENTRIES: usize = 500;

pub const ANTIGRAVITY_REFUSAL_REPLAY_MARKER: &str = "OmniRoute is not resending it to Antigravity";

#[derive(Debug, Clone)]
struct RefusalEntry {
    blocked_until_ms: i64,
    strikes: u32,
}

static REFUSALS: LazyLock<Mutex<HashMap<String, RefusalEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// `ANTIGRAVITY_MAX_429_ATTEMPTS`: upstream 429s tolerated per request across all hosts
/// (clamped 1..=12, default 2).
pub fn get_antigravity_429_attempt_budget() -> u32 {
    std::env::var("ANTIGRAVITY_MAX_429_ATTEMPTS")
        .ok()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|&n| n >= 1)
        .map(|n| n.min(ANTIGRAVITY_429_ATTEMPTS_MAX))
        .unwrap_or(ANTIGRAVITY_429_ATTEMPTS_DEFAULT)
}

/// Stable SHA-256 fingerprint of `(connection_id, model, systemInstruction, contents, tools,
/// toolConfig, generationConfig)` — omitting per-call IDs (`requestId`, `sessionId`, `project`)
/// so identical client SDK retries map to the same refusal block.
pub fn fingerprint_antigravity_request(connection_id: &str, envelope: &Value) -> String {
    let req = envelope.get("request").unwrap_or(&Value::Null);
    let material = json!([
        connection_id,
        envelope.get("model").unwrap_or(&Value::Null),
        req.get("systemInstruction").unwrap_or(&Value::Null),
        req.get("contents").unwrap_or(&Value::Null),
        req.get("tools").unwrap_or(&Value::Null),
        req.get("toolConfig").unwrap_or(&Value::Null),
        req.get("generationConfig").unwrap_or(&Value::Null),
    ]);
    let mut hasher = Sha256::new();
    hasher.update(material.to_string().as_bytes());
    hex::encode(hasher.finalize())
}

pub fn get_active_refusal(key: &str, now_ms: i64) -> Option<(u64, u32)> {
    let map = REFUSALS.lock().unwrap();
    let entry = map.get(key)?;
    if entry.blocked_until_ms <= now_ms {
        return None;
    }
    Some(((entry.blocked_until_ms - now_ms) as u64, entry.strikes))
}

pub fn remember_429_refusal(key: &str, upstream_hint_ms: Option<u64>, now_ms: i64) -> (u64, u32) {
    let mut map = REFUSALS.lock().unwrap();
    let strikes = match map.get(key) {
        Some(prev) if prev.blocked_until_ms + REFUSAL_STRIKE_WINDOW_MS >= now_ms => prev.strikes + 1,
        _ => 1,
    };
    let block_ms = match upstream_hint_ms.filter(|&ms| ms > 0) {
        Some(ms) => (ms as i64).min(REFUSAL_BLOCK_MAX_MS),
        None => {
            let mult = 1i64.checked_shl((strikes - 1).min(10)).unwrap_or(1024);
            REFUSAL_BLOCK_BASE_MS.saturating_mul(mult).min(REFUSAL_BLOCK_MAX_MS)
        }
    };
    map.insert(
        key.to_string(),
        RefusalEntry {
            blocked_until_ms: now_ms + block_ms,
            strikes,
        },
    );
    // Prune expired / cap registry size
    map.retain(|_, e| e.blocked_until_ms + REFUSAL_STRIKE_WINDOW_MS >= now_ms);
    if map.len() > REFUSAL_MEMORY_MAX_ENTRIES {
        if let Some(oldest_key) = map
            .iter()
            .min_by_key(|(_, e)| e.blocked_until_ms)
            .map(|(k, _)| k.clone())
        {
            map.remove(&oldest_key);
        }
    }
    (block_ms as u64, strikes)
}

pub fn build_refusal_replay_body(remaining_ms: u64, strikes: u32) -> String {
    let secs = (remaining_ms.div_ceil(1000)).max(1);
    let msg = format!(
        "Antigravity refused this exact request with 429 RESOURCE_EXHAUSTED and {ANTIGRAVITY_REFUSAL_REPLAY_MARKER} for another {secs}s (repeat {strikes}), to avoid hammering the upstream. If the account still has quota, the upstream is refusing the request content itself (for example a third-party agent identity in the system prompt); change the request or use another provider."
    );
    json!({
        "error": {
            "code": 429,
            "message": msg,
            "status": "RESOURCE_EXHAUSTED"
        }
    })
    .to_string()
}

pub fn is_refusal_replay_error(status: u16, body: &str) -> bool {
    status == 429 && body.contains(ANTIGRAVITY_REFUSAL_REPLAY_MARKER)
}

pub fn reset_429_guard_for_tests() {
    REFUSALS.lock().unwrap().clear();
}

// ── 5. Per-Model-Family Quota Cooldown Scoping ──────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AntigravityQuotaFamily {
    Gemini,
    Claude,
    Argon,
    Other,
}

impl AntigravityQuotaFamily {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Gemini => "gemini",
            Self::Claude => "claude",
            Self::Argon => "argon",
            Self::Other => "other",
        }
    }
}

/// Classifies an Antigravity/`agy` model into its Google Cloud Code quota pool
/// (`gemini`, `claude`, `argon`, or `other`) matching `open-sse/services/antigravityQuotaFamily.ts`.
pub fn get_antigravity_quota_family(model: &str) -> AntigravityQuotaFamily {
    let norm = model.trim().to_ascii_lowercase();
    let stripped = norm
        .strip_prefix("antigravity/")
        .or_else(|| norm.strip_prefix("agy/"))
        .unwrap_or(&norm);
    let bare = stripped
        .split_once('/')
        .map(|(_, b)| b)
        .unwrap_or(stripped);

    if bare == "argon_weekly"
        || bare == "gemini_4_argon_weekly"
        || bare.contains("argon")
        || bare.starts_with("gemini-4")
    {
        return AntigravityQuotaFamily::Argon;
    }
    if bare.starts_with("gemini-") || bare.contains("/gemini-") || bare.contains("gemini") {
        return AntigravityQuotaFamily::Gemini;
    }
    if bare.starts_with("claude-")
        || bare.starts_with("cloud-")
        || bare.contains("/claude-")
        || bare.contains("/cloud-")
        || bare.contains("anthropic")
    {
        return AntigravityQuotaFamily::Claude;
    }
    AntigravityQuotaFamily::Other
}

/// Returns `"family:gemini"`, `"family:claude"`, `"family:argon"`, or exact model name for
/// `antigravity`/`agy` quota cooldowns so exhausting one Claude model cools the Claude family
/// without needlessly cooling Gemini models (and vice versa).
pub fn quota_scoped_model(provider: &str, model: &str) -> String {
    if !is_antigravity_provider(provider) {
        return model.to_string();
    }
    match get_antigravity_quota_family(model) {
        AntigravityQuotaFamily::Other => model.to_string(),
        fam => format!("family:{}", fam.as_str()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_user_agent_and_header_scrub() {
        let cli_ua = antigravity_content_user_agent("agy", &json!({}));
        assert!(cli_ua.starts_with("antigravity/cli/1.4.4 (aidev_client; os_type="));
        assert!(cli_ua.contains("cl=1003595037; auth_method=consumer)"));

        let ide_ua = antigravity_content_user_agent("antigravity", &json!({}));
        assert_eq!(ide_ua, "antigravity/ide/2.5.5 darwin/arm64");

        let mut hdrs = vec![
            ("User-Agent".into(), cli_ua),
            ("x-forwarded-for".into(), "1.2.3.4".into()),
            ("x-stainless-lang".into(), "js".into()),
            ("sec-ch-ua".into(), "Chromium".into()),
            ("x-omniroute-debug".into(), "1".into()),
            ("x-goog-user-project".into(), "proj-1".into()),
            ("accept-encoding".into(), "gzip, deflate, br, zstd".into()),
        ];
        scrub_proxy_and_fingerprint_headers(&mut hdrs);
        let keys: Vec<String> = hdrs.iter().map(|(k, _)| k.to_ascii_lowercase()).collect();
        assert!(!keys.contains(&"x-forwarded-for".into()));
        assert!(!keys.contains(&"x-stainless-lang".into()));
        assert!(!keys.contains(&"sec-ch-ua".into()));
        assert!(!keys.contains(&"x-omniroute-debug".into()));
        assert!(keys.contains(&"x-goog-user-project".into()));
        assert!(hdrs
            .iter()
            .any(|(k, v)| k == "Accept-Encoding" && v == "gzip, deflate, br"));
    }

    #[test]
    fn fnv1a_session_id_and_uuid_request_id_are_stable() {
        let s1 = derive_antigravity_session_id("alice@gmail.com").unwrap();
        let s2 = derive_antigravity_session_id("alice@gmail.com").unwrap();
        let s3 = derive_antigravity_session_id("bob@gmail.com").unwrap();
        assert_eq!(s1, s2);
        assert_ne!(s1, s3);

        let id = build_envelope_identity(&json!({"email": "alice@gmail.com"}), "conn-1");
        assert!(id.request_id.starts_with("agent/"));
        assert!(id.request_id.ends_with("/2"));
        assert_eq!(id.session_id, s1);
    }

    #[test]
    fn prompt_sanitizer_strips_claude_identity_and_rewrites_harness_tags() {
        let mut body = json!({
            "systemInstruction": {
                "parts": [
                    {},
                    {
                        "text": "You are Claude Code, Anthropic's official CLI.\n<system-conventions>Follow rules</system-conventions>\nBuilt with Oh My Pi coding harness.\n<critical>Be concise</critical>"
                    }
                ]
            }
        });
        let edits = normalize_antigravity_system_instruction(&mut body);
        assert!(edits > 0);
        let parts = body["systemInstruction"]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 1);
        let txt = parts[0]["text"].as_str().unwrap();
        assert!(!txt.to_ascii_lowercase().contains("you are claude code"));
        assert!(txt.contains("<conventions>Follow rules</conventions>"));
        assert!(txt.contains("AI coding assistant"));
        assert!(txt.contains("<important>Be concise</important>"));
    }

    #[test]
    fn refusal_429_guard_blocks_identical_retries_with_exponential_backoff() {
        reset_429_guard_for_tests();
        let env1 = json!({
            "project": "p1",
            "model": "claude-sonnet-4-6",
            "requestId": "agent/a/1/b/2",
            "request": {
                "sessionId": "111",
                "contents": [{"role": "user", "parts": [{"text": "hello"}]}]
            }
        });
        let env2 = json!({
            "project": "p1",
            "model": "claude-sonnet-4-6",
            "requestId": "agent/different/9/c/2",
            "request": {
                "sessionId": "222",
                "contents": [{"role": "user", "parts": [{"text": "hello"}]}]
            }
        });
        let fp1 = fingerprint_antigravity_request("conn-1", &env1);
        let fp2 = fingerprint_antigravity_request("conn-1", &env2);
        assert_eq!(fp1, fp2, "per-call ids must not alter refusal fingerprint");

        assert!(get_active_refusal(&fp1, 10_000).is_none());
        let (b1, s1) = remember_429_refusal(&fp1, None, 10_000);
        assert_eq!(s1, 1);
        assert_eq!(b1, REFUSAL_BLOCK_BASE_MS as u64);
        assert!(get_active_refusal(&fp1, 20_000).is_some());

        // Repeat within strike window doubles block duration
        let (b2, s2) = remember_429_refusal(&fp1, None, 10_000 + REFUSAL_BLOCK_BASE_MS + 1000);
        assert_eq!(s2, 2);
        assert_eq!(b2, (REFUSAL_BLOCK_BASE_MS * 2) as u64);

        let replay = build_refusal_replay_body(60_000, 2);
        assert!(is_refusal_replay_error(429, &replay));
    }

    #[test]
    fn quota_family_scoping_separates_claude_gemini_argon() {
        assert_eq!(
            quota_scoped_model("agy", "claude-sonnet-4-6"),
            "family:claude"
        );
        assert_eq!(
            quota_scoped_model("antigravity", "claude-opus-4-6"),
            "family:claude"
        );
        assert_eq!(
            quota_scoped_model("agy", "gemini-2.5-pro"),
            "family:gemini"
        );
        assert_eq!(
            quota_scoped_model("antigravity", "gemini-4-preview"),
            "family:argon"
        );
        assert_eq!(
            quota_scoped_model("openai", "gpt-4o"),
            "gpt-4o"
        );
    }
}
