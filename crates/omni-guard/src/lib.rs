// Guardrails — port of src/lib/guardrails + src/lib/piiSanitizer + v3.8.54 reversible
// PII vault + v3.8.55 agent-guard (monitor / block mode).
// Precompiles regexes with LazyLock for low-overhead request/output scanning.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

// ── Prompt injection (promptInjection.ts) ───────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Low,
    Medium,
    High,
}

impl Severity {
    pub fn score(&self) -> f64 {
        match self {
            Self::Low => 0.35,
            Self::Medium => 0.7,
            Self::High => 1.0,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
    pub fn from_str_lossy(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "low" => Self::Low,
            "medium" => Self::Medium,
            _ => Self::High,
        }
    }
}

pub static DEFAULT_PATTERNS: &[(&str, &str, Severity)] = &[
    ("system_override_inline", r"(?i)\bsystem\s*:\s*override\b", Severity::High),
    ("markdown_system_block", r"(?i)```+\s*system\b", Severity::High),
    ("ignore_instructions", r"(?i)\bignore\s+(all\s+|any\s+)?(previous|prior|above)\s+instructions\b", Severity::High),
    ("reveal_system_prompt", r"(?i)\b(print|show|reveal|output)\s+(your\s+)?(system\s+prompt|instructions)\b", Severity::High),
    ("jailbreak_dan", r"(?i)\bDAN\s*(mode|jailbreak)\b", Severity::High),
    ("developer_mode", r"(?i)developer\s+mode\s+(enabled|on)", Severity::Medium),
    ("roleplay_bypass", r"(?i)\bpretend\s+(you\s+are|to\s+be)\s+(an?\s+)?(ai\s+)?(without|unchecked|unfiltered)\b", Severity::Medium),
    ("credential_probe", r"(?i)\b(list|show|print|dump)\s+(all\s+)?(api\s+keys?|tokens?|passwords?|credentials?)\b", Severity::High),
];

struct CompiledInjection {
    name: &'static str,
    regex: regex::Regex,
    severity: Severity,
}

static COMPILED_INJECTIONS: LazyLock<Vec<CompiledInjection>> = LazyLock::new(|| {
    DEFAULT_PATTERNS
        .iter()
        .map(|(n, re, sev)| CompiledInjection {
            name: n,
            regex: regex::Regex::new(re).expect("static injection regex"),
            severity: *sev,
        })
        .collect()
});

#[derive(Debug, Clone, Serialize)]
pub struct InjectionMatch {
    pub pattern_name: String,
    pub severity: &'static str,
    pub offset: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct InjectionEval {
    pub blocked: bool,
    pub score: f64,
    pub matches: Vec<InjectionMatch>,
}

pub fn evaluate_prompt_injection(text: &str, custom: &[(&str, &str, Severity)]) -> InjectionEval {
    let mut matches = Vec::new();
    let mut score = 0.0;
    for pat in COMPILED_INJECTIONS.iter() {
        if let Some(m) = pat.regex.find(text) {
            score += pat.severity.score();
            matches.push(InjectionMatch {
                pattern_name: pat.name.to_string(),
                severity: pat.severity.as_str(),
                offset: m.start(),
            });
        }
    }
    for (name, re, sev) in custom {
        if let Ok(re) = regex::Regex::new(re) {
            if let Some(m) = re.find(text) {
                score += sev.score();
                matches.push(InjectionMatch {
                    pattern_name: (*name).to_string(),
                    severity: sev.as_str(),
                    offset: m.start(),
                });
            }
        }
    }
    let blocked = matches.iter().any(|m| m.severity == "high") || score >= 1.0;
    InjectionEval {
        blocked,
        score,
        matches,
    }
}

// ── Agent-Guard (v3.8.55 #20761 monitor + #20833 block) ─────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentGuardMode {
    Off,
    Monitor,
    Block,
}

impl AgentGuardMode {
    pub fn from_str_lossy(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "monitor" => Self::Monitor,
            "block" => Self::Block,
            _ => Self::Off,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Monitor => "monitor",
            Self::Block => "block",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentGuardVerdict {
    pub mode: AgentGuardMode,
    pub flagged: bool,
    pub blocked: bool,
    pub reason: Option<String>,
}

/// Evaluate v3.8.55 `agent-guard` policy (detects spoofed Claude/Anthropic agent identity
/// or suspicious automated abuse signatures when AGENT_GUARD_MODE is monitor/block).
pub fn evaluate_agent_guard(
    mode: AgentGuardMode,
    provider: &str,
    user_agent: Option<&str>,
    payload: &Value,
) -> AgentGuardVerdict {
    if mode == AgentGuardMode::Off {
        return AgentGuardVerdict {
            mode,
            flagged: false,
            blocked: false,
            reason: None,
        };
    }
    let is_claude_family = matches!(provider, "claude" | "anthropic");
    let mut reason: Option<String> = None;

    if is_claude_family {
        let ua = user_agent.unwrap_or("").to_ascii_lowercase();
        let has_tools = payload
            .get("tools")
            .and_then(Value::as_array)
            .map(|a| !a.is_empty())
            .unwrap_or(false);
        // Flag obvious non-interactive bulk scraping / credential exfil markers
        if ua.contains("sqlmap") || ua.contains("nikto") || ua.contains("masscan") {
            reason = Some("disallowed_automated_scanner_ua".into());
        } else if has_tools {
            if let Some(tools) = payload.get("tools").and_then(Value::as_array) {
                for t in tools {
                    let name = t
                        .pointer("/function/name")
                        .or_else(|| t.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    if name.contains("exfiltrate") || name.contains("dump_secrets") {
                        reason = Some(format!("suspicious_tool_name:{name}"));
                        break;
                    }
                }
            }
        }
    }

    let flagged = reason.is_some();
    AgentGuardVerdict {
        mode,
        flagged,
        blocked: flagged && mode == AgentGuardMode::Block,
        reason,
    }
}

// ── PII masker + Reversible PII Vault (v3.8.54) ─────────────────────

pub struct PiiPatternDef {
    pub name: &'static str,
    pub pattern: &'static str,
    pub replacement: &'static str,
    pub severity: &'static str,
}

macro_rules! pii {
    ($n:expr, $re:expr, $rep:expr, $sev:expr) => {
        PiiPatternDef {
            name: $n,
            pattern: $re,
            replacement: $rep,
            severity: $sev,
        }
    };
}

pub static PII_PATTERNS: &[PiiPatternDef] = &[
    pii!("email", r#"(?i)\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b"#, "[EMAIL_REDACTED]", "medium"),
    pii!("ssn", r"\b\d{3}-\d{2}-\d{4}\b", "[SSN_REDACTED]", "high"),
    pii!(
        "credit_card",
        r"\b(?:\d{4}[-\s]?\d{4}[-\s]?\d{4}[-\s]?\d{4}|\d{4}[-\s]?\d{6}[-\s]?\d{4,5})\b",
        "[CC_REDACTED]",
        "high"
    ),
    pii!(
        "phone_us",
        r"\b(?:\+?1[-.\s]?)?\(?\d{3}\)?[-.\s]?\d{3}[-.\s]?\d{4}\b",
        "[PHONE_REDACTED]",
        "medium"
    ),
    pii!(
        "phone_br",
        r"\b(?:\+?55[-.\s]?)?\(?\d{2}\)?[-.\s]?(?:9\d{4}|[2-5]\d{3})[-.\s]?\d{4}\b",
        "[PHONE_REDACTED]",
        "medium"
    ),
    pii!("cpf", r"\b\d{3}\.?\d{3}\.?\d{3}-?\d{2}\b", "[CPF_REDACTED]", "high"),
    pii!(
        "cnpj",
        r"\b\d{2}\.?\d{3}\.?\d{3}/?\d{4}-?\d{2}\b",
        "[CNPJ_REDACTED]",
        "high"
    ),
    pii!("ip_address", r"\b(?:\d{1,3}\.){3}\d{1,3}\b", "[IP_REDACTED]", "low"),
    pii!("aws_key", r"\bAKIA[0-9A-Z]{16}\b", "[AWS_KEY_REDACTED]", "high"),
    pii!(
        "api_key_generic",
        r#"(?i)\b(?:sk|pk|api|key|token)[_-][a-zA-Z0-9]{20,}\b"#,
        "[API_KEY_REDACTED]",
        "high"
    ),
];

static COMPILED_PII: LazyLock<Vec<(&'static PiiPatternDef, regex::Regex)>> = LazyLock::new(|| {
    PII_PATTERNS
        .iter()
        .map(|p| (p, regex::Regex::new(p.pattern).expect("static PII regex")))
        .collect()
});

#[derive(Debug, Clone, Serialize)]
pub struct PiiDetection {
    pub kind: &'static str,
    pub severity: &'static str,
    pub offset: usize,
}

#[derive(Debug, Clone)]
pub struct SanitizeResult {
    pub text: String,
    pub detections: Vec<PiiDetection>,
    pub redacted: bool,
}

const IGNORED_OBFUSCATION_CHARS: &[char] = &[
    '\u{200B}', '\u{200C}', '\u{200D}', '\u{200E}', '\u{200F}', '\u{FEFF}', '\u{2060}',
    '\u{00AD}', '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}', '\u{2066}',
    '\u{2067}', '\u{2068}', '\u{2069}',
];

pub fn sanitize_pii(text: &str, enabled_types: Option<&[&str]>) -> SanitizeResult {
    let clean: String = text
        .chars()
        .filter(|c| !IGNORED_OBFUSCATION_CHARS.contains(c))
        .collect();
    if clean.is_empty() && text.is_empty() {
        return SanitizeResult {
            text: text.to_string(),
            detections: vec![],
            redacted: false,
        };
    }

    let mut out = clean;
    let mut detections = Vec::new();
    for (p, re) in COMPILED_PII.iter() {
        let enabled = match enabled_types {
            None => true,
            Some(set) => set.contains(&p.name),
        };
        if !enabled {
            continue;
        }
        let matched: Vec<usize> = re.find_iter(&out).map(|m| m.start()).collect();
        for &s in &matched {
            detections.push(PiiDetection {
                kind: p.name,
                severity: p.severity,
                offset: s,
            });
        }
        if !matched.is_empty() {
            out = re.replace_all(&out, p.replacement).to_string();
        }
    }
    let redacted = !detections.is_empty();
    SanitizeResult {
        text: out,
        detections,
        redacted,
    }
}

/// Reversible per-request PII Token Vault (v3.8.54): masks PII into opaque tokens
/// like `[PII_EMAIL_1]` outbound and restores original values on response.
#[derive(Debug, Default, Clone)]
pub struct PiiVault {
    token_to_orig: HashMap<String, String>,
    orig_to_token: HashMap<String, String>,
    counters: HashMap<&'static str, usize>,
}

impl PiiVault {
    pub fn mask(&mut self, text: &str, enabled_types: Option<&[&str]>) -> (String, Vec<PiiDetection>) {
        let clean: String = text
            .chars()
            .filter(|c| !IGNORED_OBFUSCATION_CHARS.contains(c))
            .collect();
        let mut out = clean;
        let mut detections = Vec::new();
        for (p, re) in COMPILED_PII.iter() {
            let enabled = enabled_types.map(|set| set.contains(&p.name)).unwrap_or(true);
            if !enabled {
                continue;
            }
            let hits: Vec<(usize, String)> = re
                .find_iter(&out)
                .map(|m| (m.start(), m.as_str().to_string()))
                .collect();
            if hits.is_empty() {
                continue;
            }
            for (pos, raw) in hits {
                detections.push(PiiDetection {
                    kind: p.name,
                    severity: p.severity,
                    offset: pos,
                });
                let token = if let Some(existing) = self.orig_to_token.get(&raw) {
                    existing.clone()
                } else {
                    let c = self.counters.entry(p.name).or_insert(0);
                    *c += 1;
                    let tok = format!("[PII_{}_{}]", p.name.to_ascii_uppercase(), *c);
                    self.orig_to_token.insert(raw.clone(), tok.clone());
                    self.token_to_orig.insert(tok.clone(), raw.clone());
                    tok
                };
                out = out.replacen(&raw, &token, 1);
            }
        }
        (out, detections)
    }

    pub fn restore(&self, text: &str) -> String {
        let mut out = text.to_string();
        for (tok, orig) in &self.token_to_orig {
            if out.contains(tok.as_str()) {
                out = out.replace(tok.as_str(), orig);
            }
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.token_to_orig.is_empty()
    }
}

pub fn sanitize_output_text(text: &str, enabled_types: Option<&[&str]>) -> String {
    sanitize_pii(text, enabled_types).text
}

pub fn mask_request_payload(payload: &Value, enabled_types: Option<&[&str]>) -> (Value, Vec<PiiDetection>) {
    let mut cloned = payload.clone();
    let mut detections = Vec::new();

    if let Some(obj) = cloned.as_object_mut() {
        for key in ["system", "input", "prompt"] {
            match obj.get(key) {
                Some(Value::String(s)) => {
                    let res = sanitize_pii(s, enabled_types);
                    detections.extend(res.detections);
                    obj.insert(key.into(), Value::String(res.text));
                }
                Some(Value::Array(list)) => {
                    let mut new_list = Vec::new();
                    for entry in list {
                        let e = entry.clone();
                        if let Some(s) = e.as_str() {
                            let res = sanitize_pii(s, enabled_types);
                            detections.extend(res.detections);
                            new_list.push(Value::String(res.text));
                        } else if let Some(rec_ref) = e.as_object() {
                            let mut rec: Map<String, Value> = rec_ref.clone();
                            if let Some(Value::String(c)) = rec.get_mut("content") {
                                let res = sanitize_pii(c, enabled_types);
                                detections.extend(res.detections);
                                *c = res.text;
                            }
                            if let Some(Value::String(t)) = rec.get_mut("text") {
                                let res = sanitize_pii(t, enabled_types);
                                detections.extend(res.detections);
                                *t = res.text;
                            }
                            new_list.push(Value::Object(rec));
                        } else {
                            new_list.push(e);
                        }
                    }
                    obj.insert(key.into(), Value::Array(new_list));
                }
                _ => {}
            }
        }
        if let Some(Value::Array(messages)) = obj.get_mut("messages") {
            let mut new_msgs = Vec::new();
            for msg in messages {
                let mut m = msg.clone();
                if let Some(rec_ref) = m.as_object() {
                    let mut rec: Map<String, Value> = rec_ref.clone();
                    if let Some(Value::String(c)) = rec.get_mut("content") {
                        let res = sanitize_pii(c, enabled_types);
                        detections.extend(res.detections);
                        *c = res.text;
                    }
                    if let Some(Value::Array(parts)) = rec.get_mut("content") {
                        let mut new_parts = Vec::new();
                        for part in parts {
                            let mut p = part.clone();
                            if let Some(Value::String(t)) =
                                p.as_object_mut().and_then(|o| o.get_mut("text"))
                            {
                                let res = sanitize_pii(t, enabled_types);
                                detections.extend(res.detections);
                                *t = res.text;
                            }
                            new_parts.push(p);
                        }
                        rec.insert("content".into(), Value::Array(new_parts));
                    }
                    m = Value::Object(rec);
                }
                new_msgs.push(m);
            }
            obj.insert("messages".into(), Value::Array(new_msgs));
        }
    }
    (cloned, detections)
}

// ── Credential masker ───────────────────────────────────────────────

static COMPILED_CREDS: LazyLock<Vec<(regex::Regex, &'static str)>> = LazyLock::new(|| {
    [
        (r"\bAKIA[0-9A-Z]{16}\b", "[REDACTED]"),
        (r##"(?i)\b(?:sk|gsk|pk|api|key|token)[_-][a-zA-Z0-9_-]{16,}\b"##, "[REDACTED]"),
        (r#"(?i)\bbearer\s+[A-Za-z0-9._~+/=-]{8,}\b"#, "Bearer [REDACTED]"),
        (r#"(?i)\bxox[baprs]-[A-Za-z0-9-]{10,}\b"#, "[REDACTED]"),
        (r"\bgh[pousr]_[A-Za-z0-9]{8,}\b", "[REDACTED]"),
    ]
    .iter()
    .map(|(re, rep)| (regex::Regex::new(re).expect("static cred regex"), *rep))
    .collect()
});

pub fn mask_credential_signals(text: &str) -> String {
    let mut out = text.to_string();
    for (r, rep) in COMPILED_CREDS.iter() {
        out = r.replace_all(&out, *rep).to_string();
    }
    out
}

// ── Runtime custom pattern registry ─────────────────────────────────

static CUSTOM_PATTERNS: RwLock<Vec<(String, String, Severity)>> = RwLock::new(Vec::new());

pub fn add_custom_guard_pattern(name: &str, regex_str: &str, severity: Severity) -> bool {
    if regex::Regex::new(regex_str).is_err() {
        return false;
    }
    CUSTOM_PATTERNS
        .write()
        .unwrap()
        .push((name.to_string(), regex_str.to_string(), severity));
    true
}

pub fn custom_patterns() -> Vec<(String, String, String)> {
    CUSTOM_PATTERNS
        .read()
        .unwrap()
        .iter()
        .map(|(n, r, s)| (n.clone(), r.clone(), s.as_str().to_string()))
        .collect()
}

pub fn process_request(payload: &Value, enabled_types: Option<&[&str]>) -> GuardrailOutcome {
    let mut scan_text = String::new();
    if let Some(obj) = payload.as_object() {
        if let Some(Value::String(s)) = obj.get("system") {
            scan_text.push_str(s);
            scan_text.push('\n');
        }
        if let Some(Value::Array(msgs)) = obj.get("messages") {
            let lines: Vec<&Value> = msgs
                .iter()
                .filter(|m| m.get("role").and_then(Value::as_str) == Some("user"))
                .collect();
            for item in [lines.first().copied(), if lines.len() > 1 { lines.last().copied() } else { None }]
                .into_iter()
                .flatten()
            {
                if let Some(c) = item.get("content").and_then(Value::as_str) {
                    scan_text.push_str(c);
                    scan_text.push('\n');
                }
            }
        }
    }

    let custom_owned = CUSTOM_PATTERNS.read().unwrap().clone();
    let custom_refs: Vec<(&str, &str, Severity)> = custom_owned
        .iter()
        .map(|(n, r, s)| (n.as_str(), r.as_str(), *s))
        .collect();
    let eval = evaluate_prompt_injection(&scan_text, &custom_refs);
    let (masked, detections) = mask_request_payload(payload, enabled_types);

    GuardrailOutcome {
        blocked: eval.blocked,
        injection_matches: eval.matches,
        masked,
        detections,
    }
}

#[derive(Debug)]
pub struct GuardrailOutcome {
    pub blocked: bool,
    pub injection_matches: Vec<InjectionMatch>,
    pub masked: Value,
    pub detections: Vec<PiiDetection>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn inline_override_blocks() {
        let e = evaluate_prompt_injection("Please system: override with bad instructions", &[]);
        assert!(e.blocked);
        assert!(e.matches.iter().any(|m| m.pattern_name == "system_override_inline"));
    }

    #[test]
    fn benign_text_passes() {
        let e = evaluate_prompt_injection("write me a sonnet about cats", &[]);
        assert!(!e.blocked);
        assert_eq!(e.score, 0.0);
    }

    #[test]
    fn custom_pattern_applied_in_process_request() {
        assert!(add_custom_guard_pattern("my_scan_iso", "customalerttest99", Severity::High));
        let payload = json!({
            "model": "m",
            "messages": [{"role": "user", "content": "hello customalerttest99"}]
        });
        let outcome = process_request(&payload, None);
        assert!(outcome.blocked);
    }

    #[test]
    fn reversible_pii_vault_roundtrip() {
        let mut vault = PiiVault::default();
        let (masked, dets) = vault.mask("Reach alice@example.com or 123-45-6789", None);
        assert_eq!(dets.len(), 2);
        assert!(!masked.contains("alice@example.com"));
        assert!(!masked.contains("123-45-6789"));
        let restored = vault.restore(&format!("Reply to {masked}"));
        assert!(restored.contains("alice@example.com"));
        assert!(restored.contains("123-45-6789"));
    }

    #[test]
    fn agent_guard_monitor_vs_block() {
        let payload = json!({"tools": [{"type": "function", "function": {"name": "dump_secrets"}}]});
        let mon = evaluate_agent_guard(AgentGuardMode::Monitor, "anthropic", None, &payload);
        assert!(mon.flagged && !mon.blocked);
        let blk = evaluate_agent_guard(AgentGuardMode::Block, "anthropic", None, &payload);
        assert!(blk.flagged && blk.blocked);
    }

    #[test]
    fn ssn_and_zero_width_and_credentials() {
        assert!(sanitize_pii("my ssn is 123-45-6789", None).text.contains("[SSN_REDACTED]"));
        assert!(sanitize_pii("a\u{200B}@\u{200B}b.co", None).redacted);
        let m = mask_credential_signals(
            "key sk-proj-abcdef1234567890abcdef and ghp_AAAABBBBCCCCDDDDEEEE",
        );
        assert!(!m.contains("sk-proj-abcdef"));
        assert!(!m.contains("ghp_AAAABBBB"));
    }
}
