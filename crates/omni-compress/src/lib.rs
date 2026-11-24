// omni-compress — Rust port of OmniRoute's RTK + Caveman stacked compression,
// tool-result compressor, adaptive context budget, and context compaction.
//
// Invariants (matching upstream v3.8.55):
//   - Preserves fenced code blocks, inline code, URLs, and cache_control breakpoints (#3936).
//   - Preserves error tool results & stack trace signal (#21081).
//   - Supports English + Indonesian (id) Caveman filler/framing rules across lite/full/ultra.
//   - Adaptive context budget: reserve-output / percentage / absolute + progressive compaction.

use omni_types::sse::strip_ansi;
use omni_types::ChatRequest;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::LazyLock;

// ── Configuration types ─────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CavemanIntensity {
    Off,
    Lite,
    Full,
    Ultra,
}

impl Default for CavemanIntensity {
    fn default() -> Self {
        Self::Off
    }
}

impl CavemanIntensity {
    pub fn from_str_lossy(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "lite" | "low" => Self::Lite,
            "full" | "standard" | "medium" => Self::Full,
            "ultra" | "aggressive" | "max" => Self::Ultra,
            _ => Self::Off,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Lite => "lite",
            Self::Full => "full",
            Self::Ultra => "ultra",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RtkIntensity {
    Off,
    Minimal,
    Standard,
    Aggressive,
}

impl Default for RtkIntensity {
    fn default() -> Self {
        Self::Standard
    }
}

impl RtkIntensity {
    pub fn from_str_lossy(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" | "none" => Self::Off,
            "minimal" | "lite" => Self::Minimal,
            "aggressive" | "ultra" => Self::Aggressive,
            _ => Self::Standard,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContextBudgetMode {
    Off,
    Floor,
    ReplaceAutotrigger,
}

impl Default for ContextBudgetMode {
    fn default() -> Self {
        Self::Off
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContextBudgetPolicy {
    ReserveOutput,
    Percentage,
    Absolute,
}

impl Default for ContextBudgetPolicy {
    fn default() -> Self {
        Self::ReserveOutput
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextBudgetConfig {
    #[serde(default)]
    pub mode: ContextBudgetMode,
    #[serde(default)]
    pub policy: ContextBudgetPolicy,
    #[serde(default = "default_output_reserve")]
    pub output_reserve: u64,
    #[serde(default = "default_safety_margin")]
    pub safety_margin: u64,
    #[serde(default = "default_pct")]
    pub pct: f64,
    #[serde(default = "default_abs_budget")]
    pub absolute_budget: u64,
}

fn default_output_reserve() -> u64 { 8_192 }
fn default_safety_margin() -> u64 { 2_048 }
fn default_pct() -> f64 { 0.80 }
fn default_abs_budget() -> u64 { 64_000 }

impl Default for ContextBudgetConfig {
    fn default() -> Self {
        Self {
            mode: ContextBudgetMode::Off,
            policy: ContextBudgetPolicy::ReserveOutput,
            output_reserve: default_output_reserve(),
            safety_margin: default_safety_margin(),
            pct: default_pct(),
            absolute_budget: default_abs_budget(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub rtk_intensity: RtkIntensity,
    #[serde(default)]
    pub caveman_intensity: CavemanIntensity,
    #[serde(default = "default_true")]
    pub preserve_system_prompt: bool,
    #[serde(default = "default_true")]
    pub preserve_error_tool_results: bool,
    #[serde(default = "default_max_tool_lines")]
    pub max_lines_per_tool_result: usize,
    #[serde(default)]
    pub context_budget: ContextBudgetConfig,
}

fn default_true() -> bool { true }
fn default_max_tool_lines() -> usize { 60 }

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            rtk_intensity: RtkIntensity::Standard,
            caveman_intensity: CavemanIntensity::Lite,
            preserve_system_prompt: true,
            preserve_error_tool_results: true,
            max_lines_per_tool_result: default_max_tool_lines(),
            context_budget: ContextBudgetConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompressionReport {
    pub applied: bool,
    pub original_tokens: u64,
    pub compressed_tokens: u64,
    pub saved_tokens: u64,
    pub savings_pct: f64,
    pub techniques: Vec<String>,
    pub compacted_turns: usize,
}

pub fn estimate_tokens(text: &str) -> u64 {
    ((text.len() as f64) / 4.0).ceil() as u64
}

/// Pure target-token computation (port of adaptiveCompression/computeTarget.ts).
pub fn compute_context_target(
    config: &ContextBudgetConfig,
    model_context_limit: u64,
    request_max_tokens: Option<u64>,
) -> u64 {
    match config.policy {
        ContextBudgetPolicy::Absolute => config.absolute_budget,
        ContextBudgetPolicy::Percentage => {
            let pct = if config.pct > 0.0 && config.pct <= 1.0 { config.pct } else { 1.0 };
            ((model_context_limit as f64) * pct).floor() as u64
        }
        ContextBudgetPolicy::ReserveOutput => {
            let reserve = request_max_tokens.filter(|&v| v > 0).unwrap_or(config.output_reserve);
            model_context_limit.saturating_sub(reserve).saturating_sub(config.safety_margin)
        }
    }
}

// ── Preservation of code fences, inline code & URLs ─────────────────

static PRESERVE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)```.*?```|`[^`\n]+`|https?://[^\s)\]>]+").expect("preserve regex")
});

fn extract_preserved(text: &str) -> (String, Vec<String>) {
    let mut blocks = Vec::new();
    let mut out = String::with_capacity(text.len());
    let mut last = 0usize;
    for m in PRESERVE_RE.find_iter(text) {
        out.push_str(&text[last..m.start()]);
        let idx = blocks.len();
        blocks.push(m.as_str().to_string());
        out.push_str(&format!("__OMNI_KEEP_{idx}__"));
        last = m.end();
    }
    out.push_str(&text[last..]);
    (out, blocks)
}

fn restore_preserved(mut text: String, blocks: &[String]) -> String {
    for (idx, block) in blocks.iter().enumerate() {
        let token = format!("__OMNI_KEEP_{idx}__");
        text = text.replace(&token, block);
    }
    text
}

// ── Caveman Compressor (EN + ID rules across lite/full/ultra) ───────

struct CavemanRule {
    regex: Regex,
    replacement: &'static str,
    min_intensity: CavemanIntensity,
}

static CAVEMAN_RULES: LazyLock<Vec<CavemanRule>> = LazyLock::new(|| {
    let raw: &[(&str, &str, CavemanIntensity)] = &[
        // English Lite rules
        (r"(?i)\b(?:i'?d be happy to|i would be happy to|glad to help|thank you so much|thanks in advance|i really appreciate|thank you|thanks|no problem|you'?re welcome|certainly|of course)\b[,.!?]*\s*", "", CavemanIntensity::Lite),
        (r"(?i)\b(?:could you please|would you please|can you please|i would like you to|i want you to|i need you to|please|kindly)\b\s*", "", CavemanIntensity::Lite),
        (r"(?i)\b(?:it seems like|it appears that|i think that|i believe that|i guess|i suppose|more or less|in a way)\b\s*", "", CavemanIntensity::Lite),
        (r"(?i)\b(?:basically|essentially|actually|literally|simply|currently)\b\s*", "", CavemanIntensity::Lite),
        (r"(?i)^(?:hi there|hello|good morning|good afternoon|hey)\s*[,.!?]*\s*", "", CavemanIntensity::Lite),
        (r"(?i)\b(?:i was wondering if you could|would it be possible to|if possible|when you get a chance|at your convenience|just wondering)\b[,.!?]*\s*", "", CavemanIntensity::Lite),
        (r"(?i)\b(?:provide a detailed explanation of|give me a comprehensive explanation of|explain in detail)\b", "explain", CavemanIntensity::Lite),
        // Indonesian (id) Lite rules (from rules/id/filler.json)
        (r"(?i)\b(?:terima\s+kasih\s+banyak|terima\s+kasih|makasih\s+banyak|makasih|mohon\s+bantuannya|tolong\s+bantuannya|sama-sama|semoga\s+membantu|semoga\s+bermanfaat)\b[,.!?]*\s*", "", CavemanIntensity::Lite),
        (r"(?i)\b(?:mohon\s+info|mohon\s+bantuan|tolong\s+dong|minta\s+tolong|bisa\s+tolong|tolongin|tolong|mohon)\b\s*", "", CavemanIntensity::Lite),
        (r"(?i)\b(?:pada\s+dasarnya|sebenarnya|secara\s+harfiah|gampangnya|mudahnya|sederhananya|kurang\s+lebih|kira-kira)\b\s*", "", CavemanIntensity::Lite),
        (r"(?i)^(?:halo\s+gan|halo\s+min|halo|hai|permisi|selamat\s+(?:pagi|siang|sore|malam))\b\s*[,.!?]*\s*", "", CavemanIntensity::Lite),
        // Full rules (structural connectors, redundant quantifiers & phrases)
        (r"(?i)\b(?:in order to|so as to)\b", "to", CavemanIntensity::Full),
        (r"(?i)\b(?:due to the fact that|the reason is because)\b", "because", CavemanIntensity::Full),
        (r"(?i)\b(?:each and every|any and all)\b", "all", CavemanIntensity::Full),
        (r"(?i)\b(?:furthermore|additionally|moreover|in addition)\b[,\s]*", "", CavemanIntensity::Full),
        (r"(?i)\b(?:a bit|a little|somewhat|kind of|sort of|very|really|extremely|quite)\b\s*", "", CavemanIntensity::Full),
        (r"(?i)\b(?:note that|keep in mind that|remember that|as we discussed earlier|as mentioned before)\b[,\s]*", "", CavemanIntensity::Full),
        (r"(?i)\b(?:untuk\s+tujuan|dengan\s+tujuan\s+untuk)\b", "untuk", CavemanIntensity::Full),
        // Ultra rules (technical abbreviations)
        (r"(?i)\bconfiguration\b", "cfg", CavemanIntensity::Ultra),
        (r"(?i)\bimplementation\b", "impl", CavemanIntensity::Ultra),
        (r"(?i)\bauthentication\b", "auth", CavemanIntensity::Ultra),
        (r"(?i)\bauthorization\b", "authz", CavemanIntensity::Ultra),
        (r"(?i)\bapplication\b", "app", CavemanIntensity::Ultra),
        (r"(?i)\bdependencies\b", "deps", CavemanIntensity::Ultra),
        (r"(?i)\bdependency\b", "dep", CavemanIntensity::Ultra),
        (r"(?i)\bdatabase\b", "db", CavemanIntensity::Ultra),
        (r"(?i)\bfunction\b", "fn", CavemanIntensity::Ultra),
        (r"(?i)\brequest\b", "req", CavemanIntensity::Ultra),
        (r"(?i)\bresponse\b", "resp", CavemanIntensity::Ultra),
    ];
    raw.iter()
        .map(|(re, rep, lvl)| CavemanRule {
            regex: Regex::new(re).expect("static caveman regex"),
            replacement: rep,
            min_intensity: *lvl,
        })
        .collect()
});

static MULTI_SPACE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ \t]{2,}").unwrap());
static MULTI_NL_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{3,}").unwrap());

fn intensity_rank(i: CavemanIntensity) -> u8 {
    match i {
        CavemanIntensity::Off => 0,
        CavemanIntensity::Lite => 1,
        CavemanIntensity::Full => 2,
        CavemanIntensity::Ultra => 3,
    }
}

pub fn compress_caveman_text(text: &str, intensity: CavemanIntensity) -> String {
    if intensity == CavemanIntensity::Off || text.len() < 24 {
        return text.to_string();
    }
    let rank = intensity_rank(intensity);
    let (mut work, preserved) = extract_preserved(text);
    for rule in CAVEMAN_RULES.iter() {
        if intensity_rank(rule.min_intensity) <= rank {
            work = rule.regex.replace_all(&work, rule.replacement).to_string();
        }
    }
    work = MULTI_SPACE_RE.replace_all(&work, " ").to_string();
    work = MULTI_NL_RE.replace_all(&work, "\n\n").to_string();
    let restored = restore_preserved(work.trim().to_string(), &preserved);
    // Fidelity guard: never collapse a non-empty prompt into empty or grow it
    if restored.is_empty() || restored.len() >= text.len() {
        text.to_string()
    } else {
        restored
    }
}

// ── RTK + Tool Result Compressor ────────────────────────────────────

pub fn is_error_like(content: &str) -> bool {
    let lower = content.to_ascii_lowercase();
    lower.contains("error:")
        || lower.contains("[error]")
        || lower.contains("exception:")
        || lower.contains("traceback")
        || lower.contains("panicked at")
        || lower.contains("test result: failed")
        || lower.contains("exit code 1")
        || lower.contains("fatal:")
}

fn is_code_like_line(raw: &str) -> bool {
    let l = raw.trim_start();
    l.starts_with("import ")
        || l.starts_with("export ")
        || l.starts_with("function ")
        || l.starts_with("class ")
        || l.starts_with("const ")
        || l.starts_with("let ")
        || l.starts_with("pub fn ")
        || l.starts_with("fn ")
        || l.starts_with("struct ")
        || l.starts_with("def ")
        || l.starts_with("return ")
}

fn parse_grep_path(line: &str) -> Option<&str> {
    let c1 = line.find(':')?;
    if c1 == 0 { return None; }
    let rest = &line[c1 + 1..];
    let c2 = rest.find(':')?;
    let lnum = &rest[..c2];
    if lnum.is_empty() || !lnum.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let path = &line[..c1];
    if path.chars().any(char::is_whitespace) { None } else { Some(path) }
}

/// Detect RTK command class (`git-diff`, `git-status`, `test-output`, `grep`, `json`, `code`, `shell`).
pub fn detect_rtk_kind(content: &str) -> &'static str {
    if content.contains("diff --git ") || content.contains("\n@@ -") {
        return "git-diff";
    }
    if content.contains("On branch ") && (content.contains("Changes not staged") || content.contains("Untracked files")) {
        return "git-status";
    }
    if content.contains("test result:") || content.contains("PASS ") || content.contains("FAIL ") || content.contains("pytest") {
        return "test-output";
    }
    let trimmed = content.trim_start();
    if content.len() > 2_000 && (trimmed.starts_with('{') || trimmed.starts_with('[')) {
        return "json";
    }
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() >= 10 && lines.iter().filter(|l| parse_grep_path(l).is_some()).count() >= 5 {
        return "grep";
    }
    if lines.len() > 30 && lines.iter().any(|l| is_code_like_line(l)) {
        return "file-content";
    }
    "shell"
}

fn compress_json_output(content: &str) -> Option<String> {
    let parsed: Value = serde_json::from_str(content).ok()?;
    match parsed {
        Value::Array(arr) if arr.len() > 7 => {
            let first5 = arr[..5].to_vec();
            let last2 = arr[arr.len() - 2..].to_vec();
            Some(
                serde_json::to_string_pretty(&json!({
                    "type": "array",
                    "total": arr.len(),
                    "first5": first5,
                    "last2": last2
                }))
                .ok()?,
            )
        }
        Value::Object(map) if map.len() > 10 || content.len() > 2_000 => {
            let mut summary = serde_json::Map::new();
            let total = map.len();
            for (k, v) in map.into_iter().take(20) {
                match v {
                    Value::Object(inner) => {
                        summary.insert(k, json!(format!("{{...{} keys}}", inner.len())));
                    }
                    Value::Array(arr) if arr.len() > 5 => {
                        summary.insert(k, json!(format!("[...{} items]", arr.len())));
                    }
                    other => {
                        summary.insert(k, other);
                    }
                }
            }
            if total > 20 {
                summary.insert(format!("_remaining_{}_keys", total - 20), json!(true));
            }
            serde_json::to_string_pretty(&Value::Object(summary)).ok()
        }
        _ => None,
    }
}

/// RTK command-aware compression of a tool result string.
/// Honors v3.8.55 `#21081`: when `preserve_errors` is true and content looks like an error,
/// preserves the error header + head/tail stack frames instead of dropping error diagnostics.
pub fn compress_rtk_tool_output(
    content: &str,
    intensity: RtkIntensity,
    preserve_errors: bool,
    max_lines: usize,
) -> (String, &'static str) {
    if intensity == RtkIntensity::Off || content.len() < 400 || content.starts_with("[COMPRESSED:") {
        return (content.to_string(), "none");
    }

    let cleaned = strip_ansi(content);
    let lines: Vec<&str> = cleaned.lines().collect();

    // Error preservation (#21081): keep error message + top 12 + tail 6 lines.
    if preserve_errors && is_error_like(&cleaned) {
        if lines.len() <= 24 {
            return (cleaned, "error-preserved");
        }
        let head = lines[..12].join("\n");
        let tail = lines[lines.len() - 6..].join("\n");
        let elided = lines.len() - 18;
        return (
            format!("{head}\n... [{elided} frames/lines elided, error preserved] ...\n{tail}"),
            "error-preserved",
        );
    }

    let kind = detect_rtk_kind(&cleaned);
    let cap = match intensity {
        RtkIntensity::Minimal => max_lines.max(80),
        RtkIntensity::Standard => max_lines.max(40),
        RtkIntensity::Aggressive => max_lines.min(25).max(12),
        RtkIntensity::Off => return (content.to_string(), "none"),
    };

    match kind {
        "json" => {
            if let Some(comp) = compress_json_output(&cleaned) {
                if comp.len() < cleaned.len() {
                    return (comp, "rtk-json");
                }
            }
        }
        "grep" => {
            let grep_lines: Vec<&str> = lines.iter().copied().filter(|l| parse_grep_path(l).is_some()).collect();
            if grep_lines.len() > 30 {
                let mut files: Vec<&str> = Vec::new();
                let mut seen = HashSet::new();
                for l in &grep_lines {
                    if let Some(p) = parse_grep_path(l) {
                        if seen.insert(p) {
                            files.push(p);
                        }
                    }
                }
                let head = grep_lines[..30].join("\n");
                let rem = grep_lines.len() - 30;
                return (
                    format!("{head}\n... [{rem} more matches]\nFiles: {}", files.join(", ")),
                    "rtk-grep",
                );
            }
        }
        "test-output" => {
            // Keep failing/summary/important lines, collapse green passing boilerplate
            let mut kept: Vec<&str> = Vec::new();
            let mut skipped_pass = 0usize;
            for l in &lines {
                let low = l.to_ascii_lowercase();
                if low.contains("fail")
                    || low.contains("error")
                    || low.contains("warn")
                    || low.contains("test result:")
                    || low.contains("passed")
                    || low.contains("summary")
                {
                    kept.push(l);
                } else if low.contains("... ok") || low.starts_with("pass ") || low.starts_with("✓") {
                    skipped_pass += 1;
                } else if kept.len() < cap {
                    kept.push(l);
                }
            }
            if skipped_pass > 3 {
                let mut out = kept.join("\n");
                out.push_str(&format!("\n[rtk: {skipped_pass} passing test lines collapsed]"));
                if out.len() < cleaned.len() {
                    return (out, "rtk-test");
                }
            }
        }
        "git-diff" => {
            // Keep file headers, hunks, and +/- lines; elide large unchanged context runs
            let mut out: Vec<&str> = Vec::new();
            let mut ctx_run = 0usize;
            for l in &lines {
                if l.starts_with("diff --git") || l.starts_with("+++") || l.starts_with("---") || l.starts_with("@@") || l.starts_with('+') || l.starts_with('-') {
                    ctx_run = 0;
                    out.push(l);
                } else {
                    ctx_run += 1;
                    if ctx_run <= 2 {
                        out.push(l);
                    }
                }
                if out.len() >= cap * 2 {
                    break;
                }
            }
            if out.len() < lines.len() {
                let omitted = lines.len() - out.len();
                let mut res = out.join("\n");
                res.push_str(&format!("\n... [{omitted} diff lines elided]"));
                if res.len() < cleaned.len() {
                    return (res, "rtk-git-diff");
                }
            }
        }
        "file-content" => {
            let keep_head = (cap * 3) / 4;
            let keep_tail = (cap / 4).max(5);
            if lines.len() > keep_head + keep_tail {
                let head = lines[..keep_head].join("\n");
                let tail = lines[lines.len() - keep_tail..].join("\n");
                let elided = lines.len() - keep_head - keep_tail;
                return (
                    format!("{head}\n... [{elided} lines elided] ...\n{tail}"),
                    "rtk-file",
                );
            }
        }
        _ => {}
    }

    // Generic shell/log deduplication + bounded head/tail
    let mut deduped: Vec<String> = Vec::new();
    let mut prev: Option<&str> = None;
    let mut repeat = 0usize;
    for l in &lines {
        if prev == Some(*l) {
            repeat += 1;
        } else {
            if repeat > 1 {
                deduped.push(format!("  [... repeated {repeat} times]"));
            }
            repeat = 0;
            deduped.push((*l).to_string());
            prev = Some(*l);
        }
    }
    if repeat > 1 {
        deduped.push(format!("  [... repeated {repeat} times]"));
    }

    let final_str = if deduped.len() > cap {
        let head = deduped[..cap / 2].join("\n");
        let tail = deduped[deduped.len() - (cap / 2)..].join("\n");
        let elided = deduped.len() - cap;
        format!("{head}\n... [{elided} lines elided] ...\n{tail}")
    } else {
        deduped.join("\n")
    };

    if final_str.len() < content.len() {
        (final_str, "rtk-shell")
    } else {
        (content.to_string(), "none")
    }
}

// ── End-to-end ChatRequest Compression + Adaptive Compaction ────────

fn has_cache_control(v: &Value) -> bool {
    v.get("cache_control").map(|c| !c.is_null()).unwrap_or(false)
}

/// Apply RTK + Caveman + Adaptive Context Compaction to a ChatRequest in place.
pub fn compress_chat_request(
    req: &mut ChatRequest,
    config: &CompressionConfig,
    model_context_limit: u64,
) -> CompressionReport {
    if !config.enabled {
        return CompressionReport::default();
    }
    let before_json = serde_json::to_string(&req.messages).unwrap_or_default();
    let original_tokens = estimate_tokens(&before_json);
    let mut techniques: Vec<String> = Vec::new();
    let mut add_tech = |name: &str| {
        if !techniques.iter().any(|t| t == name) {
            techniques.push(name.to_string());
        }
    };

    for msg in req.messages.iter_mut() {
        if has_cache_control(&msg.extra) {
            continue;
        }
        let role = msg.role.as_str();
        if (role == "system" || role == "developer") && config.preserve_system_prompt {
            continue;
        }

        if role == "tool" {
            if let Some(Value::String(s)) = msg.content.as_mut() {
                let (comp, strat) = compress_rtk_tool_output(
                    s,
                    config.rtk_intensity,
                    config.preserve_error_tool_results,
                    config.max_lines_per_tool_result,
                );
                if strat != "none" && comp.len() < s.len() {
                    *s = comp;
                    add_tech(strat);
                }
            }
            continue;
        }

        if config.caveman_intensity != CavemanIntensity::Off && (role == "user" || role == "assistant") {
            match msg.content.as_mut() {
                Some(Value::String(s)) => {
                    let comp = compress_caveman_text(s, config.caveman_intensity);
                    if comp.len() < s.len() {
                        *s = comp;
                        add_tech(&format!("caveman-{}", config.caveman_intensity.as_str()));
                    }
                }
                Some(Value::Array(parts)) => {
                    for part in parts.iter_mut() {
                        if has_cache_control(part) {
                            continue;
                        }
                        if part.get("type").and_then(Value::as_str) == Some("text") {
                            if let Some(t) = part.get("text").and_then(Value::as_str).map(String::from) {
                                let comp = compress_caveman_text(&t, config.caveman_intensity);
                                if comp.len() < t.len() {
                                    part["text"] = Value::String(comp);
                                    add_tech(&format!("caveman-{}", config.caveman_intensity.as_str()));
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Adaptive context compaction when context_budget mode is enabled and prompt exceeds target.
    let mut compacted_turns = 0usize;
    if config.context_budget.mode != ContextBudgetMode::Off && model_context_limit > 0 {
        let target = compute_context_target(
            &config.context_budget,
            model_context_limit,
            req.max_tokens.or(req.max_completion_tokens),
        );
        let cur_tokens = estimate_tokens(&serde_json::to_string(&req.messages).unwrap_or_default());
        if target > 0 && cur_tokens > target && req.messages.len() > 6 {
            // Keep leading system messages + last 4 messages; drop oldest non-system middle turns
            let sys_end = req
                .messages
                .iter()
                .take_while(|m| m.role == "system" || m.role == "developer")
                .count();
            let keep_tail = 4usize.min(req.messages.len().saturating_sub(sys_end));
            let mut cur = cur_tokens;
            while cur > target && req.messages.len() > sys_end + keep_tail {
                let removed = req.messages.remove(sys_end);
                let removed_toks = estimate_tokens(&serde_json::to_string(&removed).unwrap_or_default());
                cur = cur.saturating_sub(removed_toks);
                compacted_turns += 1;
            }
            if compacted_turns > 0 {
                add_tech("context-compaction");
            }
        }
    }

    let after_json = serde_json::to_string(&req.messages).unwrap_or_default();
    let compressed_tokens = estimate_tokens(&after_json);
    let saved_tokens = original_tokens.saturating_sub(compressed_tokens);
    let savings_pct = if original_tokens > 0 {
        (saved_tokens as f64 / original_tokens as f64) * 100.0
    } else {
        0.0
    };

    CompressionReport {
        applied: saved_tokens > 0 || compacted_turns > 0,
        original_tokens,
        compressed_tokens,
        saved_tokens,
        savings_pct,
        techniques,
        compacted_turns,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omni_types::ChatMessage;

    #[test]
    fn caveman_compresses_en_and_id_while_preserving_code_blocks() {
        let input = "Hello! Could you please basically explain in detail how `let x = 1;` works?\n```rust\nfn main() { println!(\"please keep this\"); }\n```";
        let out = compress_caveman_text(input, CavemanIntensity::Full);
        assert!(out.len() < input.len(), "expected shorter output: {out}");
        assert!(out.contains("fn main() { println!(\"please keep this\"); }"));
        assert!(out.contains("`let x = 1;`"));

        let id_in = "Halo gan, tolong dong pada dasarnya jelaskan kenapa server error terima kasih banyak";
        let id_out = compress_caveman_text(id_in, CavemanIntensity::Lite);
        assert!(id_out.len() < id_in.len(), "expected Indonesian compression: {id_out}");
        assert!(id_out.contains("server error"));
    }

    #[test]
    fn rtk_preserves_errors_and_collapses_passing_tests() {
        let mut passing = String::new();
        for i in 0..80 {
            passing.push_str(&format!("test suite::case_{i} ... ok\n"));
        }
        passing.push_str("test result: ok. 80 passed; 0 failed\n");
        let (comp, strat) = compress_rtk_tool_output(&passing, RtkIntensity::Standard, true, 40);
        assert_eq!(strat, "rtk-test");
        assert!(comp.contains("80 passed"));
        assert!(comp.len() < passing.len());

        let mut err_out = String::from("error: compilation failed at main.rs:42\n");
        for i in 0..40 {
            err_out.push_str(&format!("  frame {i}: at module::fn_{i}\n"));
        }
        let (comp_err, strat_err) = compress_rtk_tool_output(&err_out, RtkIntensity::Aggressive, true, 20);
        assert_eq!(strat_err, "error-preserved");
        assert!(comp_err.contains("error: compilation failed at main.rs:42"));
    }

    #[test]
    fn adaptive_context_target_and_compaction_work() {
        let budget = ContextBudgetConfig {
            mode: ContextBudgetMode::Floor,
            policy: ContextBudgetPolicy::Absolute,
            absolute_budget: 60,
            ..Default::default()
        };
        assert_eq!(compute_context_target(&budget, 128_000, None), 60);

        let mut msgs = vec![ChatMessage {
            role: "system".into(),
            content: Some(json!("You are helpful.")),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            extra: json!({}),
        }];
        for i in 0..10 {
            msgs.push(ChatMessage {
                role: if i % 2 == 0 { "user".into() } else { "assistant".into() },
                content: Some(json!(format!("Turn {i}: {}", "word ".repeat(30)))),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                extra: json!({}),
            });
        }
        let mut req = ChatRequest {
            model: "gpt-6.1-sol".into(),
            messages: msgs,
            ..Default::default()
        };
        let cfg = CompressionConfig {
            enabled: true,
            rtk_intensity: RtkIntensity::Standard,
            caveman_intensity: CavemanIntensity::Lite,
            context_budget: budget,
            ..Default::default()
        };
        let rep = compress_chat_request(&mut req, &cfg, 128_000);
        assert!(rep.applied);
        assert!(rep.compacted_turns > 0);
        assert_eq!(req.messages[0].role, "system");
    }
}
