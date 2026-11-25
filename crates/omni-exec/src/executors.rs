// Priority Provider-Specific Executors — port of OmniRoute v3.8.55 open-sse/executors/*:
//   1. Claude / Claude Code (`claude`, `anthropic`) — OAuth vs API-key beta headers,
//      conditional `skills-2025-10-02` only when `code_execution` tool is present (#14200),
//      Claude Code identity headers, and `api.anthropic.com` gzip >= 4096B.
//   2. Codex / ChatGPT (`codex`, `chatgpt`) — Responses API URL, `originator: codex_cli_rs`,
//      opt-in `X-OmniRoute-Selected-Connection-Plan` header (`OMNIROUTE_CODEX_PLAN_RESPONSE_HEADER=1`).
//   3. GitHub Copilot (`github`, `ghe-copilot`) — short-lived Copilot token, Claude -> `/v1/messages`,
//      `*-codex` -> `/responses` (with guard keeping Claude/Gemini off `/responses` per #1536).
//   4. Antigravity & Gemini CLI (`antigravity`, `agy`, `gemini-cli`) — Cloud Code `v1internal`
//      envelope + multi-endpoint fallback (`cloudcode-pa` -> `daily-cloudcode-pa.sandbox`).
//   5. AWS Bedrock (`bedrock`) — region SSRF guard, Converse API URL + OpenAI <-> Converse
//      translation, pure-Rust AWS SigV4 HMAC-SHA256 signer AND Bedrock Native Bearer key mode.
//   6. GCP Vertex AI (`vertex`, `vertex-partner`) — Express API key vs project-scoped
//      Gemini / Claude `rawPredict` (with legacy 1h TTL downgrade) / Mistral / OpenAI MaaS partner.
//   7. Azure OpenAI & Azure AI Foundry (`azure`, `azure-ai`, `azure-openai`) — Foundry `/openai/v1/*`
//      vs Classic `/openai/deployments/{model}/chat/completions?api-version=...` detection.

use crate::dispatch::{AuthHeader, AuthScheme, ConnectionEndpoint};
use chrono::{DateTime, Utc};
use omni_catalog::{ProviderDescriptor, WireFormat};
use omni_oauth::{
    assert_valid_aws_region, url_encode, GITHUB_COPILOT_API_VERSION, GITHUB_COPILOT_EDITOR_VERSION,
    GITHUB_COPILOT_PLUGIN_VERSION, GITHUB_COPILOT_USER_AGENT,
};
use omni_translate::{
    chat_to_responses_request, openai_to_claude_with_map, openai_to_gemini,
};
use omni_types::ChatRequest;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const ANTHROPIC_VERSION: &str = "2023-06-01";
pub const ANTHROPIC_BETA_API_KEY: &str =
    "interleaved-thinking-2025-05-14,fine-grained-tool-streaming-2025-05-14,prompt-caching-2024-07-31";
pub const ANTHROPIC_BETA_CLAUDE_OAUTH: &str =
    "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14,fine-grained-tool-streaming-2025-05-14";
pub const ANTHROPIC_SKILLS_BETA: &str = "skills-2025-10-02";

pub const ANTIGRAVITY_DEFAULT_ENDPOINTS: &[&str] = &[
    "https://cloudcode-pa.googleapis.com",
    "https://daily-cloudcode-pa.sandbox.googleapis.com",
];

const CODEX_PLAN_ALLOWLIST: &[&str] = &[
    "free",
    "go",
    "plus",
    "pro",
    "prolite",
    "promax",
    "team",
    "chatgptteam",
    "business",
    "enterprise",
    "edu",
    "education",
];

const VERTEX_LEGACY_1H_TTL_MODELS: &[&str] = &[
    "claude-3-7-sonnet",
    "claude-3-5-sonnet-v2",
    "claude-3-5-sonnet",
    "claude-3-opus",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpstreamResponseFormat {
    OpenAI,
    Claude,
    Gemini,
    Responses,
    BedrockConverse,
}

#[derive(Debug, Clone)]
pub struct PreparedExecution {
    pub endpoint: ConnectionEndpoint,
    pub body: Value,
    pub tool_name_map: Vec<(String, String)>,
    pub response_format: UpstreamResponseFormat,
    pub extra_response_headers: Vec<(String, String)>,
}

fn upsert_header(headers: &mut Vec<(String, String)>, key: &str, value: impl Into<String>) {
    let val = value.into();
    if let Some(slot) = headers.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
        slot.1 = val;
    } else {
        headers.push((key.to_string(), val));
    }
}

fn has_code_execution_tool(req: &ChatRequest) -> bool {
    let Some(tools) = &req.tools else {
        return false;
    };
    tools.iter().any(|t| {
        t.kind.starts_with("code_execution") || t.function.name == "code_execution"
    })
}

/// Normalize & validate Codex/ChatGPT subscription tier for opt-in response header
/// (`OMNIROUTE_CODEX_PLAN_RESPONSE_HEADER=1`).
pub fn resolve_codex_plan_header(
    provider_specific: &Value,
    force_enabled: Option<bool>,
) -> Option<String> {
    let enabled = force_enabled.unwrap_or_else(|| {
        std::env::var("OMNIROUTE_CODEX_PLAN_RESPONSE_HEADER")
            .map(|v| v.trim() == "1")
            .unwrap_or(false)
    });
    if !enabled {
        return None;
    }
    let raw = provider_specific
        .get("plan")
        .or_else(|| provider_specific.get("subscription_plan"))
        .or_else(|| provider_specific.get("planType"))
        .and_then(Value::as_str)?;
    let norm: String = raw
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|c| !matches!(c, ' ' | '_' | '-'))
        .collect();
    if CODEX_PLAN_ALLOWLIST.contains(&norm.as_str()) {
        Some(norm)
    } else {
        None
    }
}

// ── GitHub Copilot Model Routing Guard (v3.8.55 #1536) ──────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopilotRouteMode {
    ChatCompletions,
    AnthropicMessages,
    OpenAiResponses,
}

pub fn select_copilot_route(model: &str) -> CopilotRouteMode {
    let m = model.trim().to_ascii_lowercase();
    if m.starts_with("claude") || m.contains("/claude") {
        return CopilotRouteMode::AnthropicMessages;
    }
    // Guard #1536: never send Gemini or Claude models to `/responses`
    if m.starts_with("gemini") || m.contains("/gemini") {
        return CopilotRouteMode::ChatCompletions;
    }
    if m.contains("codex") {
        return CopilotRouteMode::OpenAiResponses;
    }
    CopilotRouteMode::ChatCompletions
}

// ── Azure OpenAI / Azure AI Foundry URL Detection ───────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AzureUrlFormat {
    Foundry,
    Classic,
}

pub fn detect_azure_url_format(base_url: &str) -> AzureUrlFormat {
    let lower = base_url.to_ascii_lowercase();
    if lower.contains("services.ai.azure.com")
        || lower.contains("/openai/v1")
        || lower.contains("/models")
    {
        AzureUrlFormat::Foundry
    } else {
        AzureUrlFormat::Classic
    }
}

pub fn build_azure_url(base_url: &str, model: &str, api_version: &str, use_responses: bool) -> String {
    let trimmed = base_url.trim_end_matches('/');
    match detect_azure_url_format(trimmed) {
        AzureUrlFormat::Foundry => {
            if trimmed.ends_with("/chat/completions") || trimmed.ends_with("/responses") {
                return trimmed.to_string();
            }
            let suffix = if use_responses { "responses" } else { "chat/completions" };
            if trimmed.ends_with("/openai/v1") {
                format!("{trimmed}/{suffix}")
            } else {
                format!("{trimmed}/openai/v1/{suffix}")
            }
        }
        AzureUrlFormat::Classic => {
            if trimmed.contains("/deployments/") {
                if trimmed.contains("api-version=") {
                    trimmed.to_string()
                } else {
                    format!("{trimmed}?api-version={api_version}")
                }
            } else {
                format!(
                    "{trimmed}/openai/deployments/{}/chat/completions?api-version={api_version}",
                    url_encode(model)
                )
            }
        }
    }
}

// ── GCP Vertex AI URL Builder & Legacy TTL Downgrade ────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VertexTransport {
    Gemini,
    Anthropic,
    Mistral,
    OpenAiPartner,
}

pub fn vertex_model_transport(model: &str) -> VertexTransport {
    let m = model.trim().to_ascii_lowercase();
    if m.starts_with("claude") || m.contains("anthropic") {
        VertexTransport::Anthropic
    } else if m.starts_with("mistral") || m.starts_with("codestral") || m.starts_with("mixtral") {
        VertexTransport::Mistral
    } else if m.starts_with("gemini") {
        VertexTransport::Gemini
    } else {
        VertexTransport::OpenAiPartner
    }
}

pub fn vertex_host_for_region(region: &str) -> String {
    match region.trim().to_ascii_lowercase().as_str() {
        "" | "global" => "aiplatform.googleapis.com".into(),
        "us" => "aiplatform.us.rep.googleapis.com".into(),
        "eu" => "aiplatform.eu.rep.googleapis.com".into(),
        r => format!("{r}-aiplatform.googleapis.com"),
    }
}

pub fn build_vertex_url(
    model: &str,
    stream: bool,
    project: Option<&str>,
    region: &str,
    express_api_key: Option<&str>,
) -> Result<(String, VertexTransport), String> {
    let transport = vertex_model_transport(model);
    if let Some(proj) = project.filter(|s| !s.is_empty()) {
        let host = vertex_host_for_region(region);
        let key_q = express_api_key
            .filter(|k| !k.is_empty())
            .map(url_encode);
        let url = match transport {
            VertexTransport::Anthropic => {
                // Upstream v3.8.55: always use `:rawPredict` (not `streamRawPredict?alt=sse`)
                let suffix = key_q.map(|k| format!("?key={k}")).unwrap_or_default();
                format!("https://{host}/v1/projects/{proj}/locations/{region}/publishers/anthropic/models/{model}:rawPredict{suffix}")
            }
            VertexTransport::Mistral => {
                let op = if stream { "streamRawPredict" } else { "rawPredict" };
                let suffix = key_q.map(|k| format!("?key={k}")).unwrap_or_default();
                format!("https://{host}/v1/projects/{proj}/locations/{region}/publishers/mistralai/models/{model}:{op}{suffix}")
            }
            VertexTransport::OpenAiPartner => {
                let suffix = key_q.map(|k| format!("?key={k}")).unwrap_or_default();
                format!("https://aiplatform.googleapis.com/v1/projects/{proj}/locations/global/endpoints/openapi/chat/completions{suffix}")
            }
            VertexTransport::Gemini => {
                let op = if stream {
                    "streamGenerateContent?alt=sse"
                } else {
                    "generateContent"
                };
                let key_part = match key_q {
                    Some(k) if stream => format!("&key={k}"),
                    Some(k) => format!("?key={k}"),
                    None => String::new(),
                };
                format!("https://{host}/v1/projects/{proj}/locations/{region}/publishers/google/models/{model}:{op}{key_part}")
            }
        };
        Ok((url, transport))
    } else if let Some(key) = express_api_key.filter(|k| !k.is_empty()) {
        if transport != VertexTransport::Gemini {
            return Err(
                "Vertex Express API keys support Gemini models only; configure project_id for partner models"
                    .into(),
            );
        }
        let op = if stream {
            "streamGenerateContent?alt=sse&"
        } else {
            "generateContent?"
        };
        Ok((
            format!(
                "https://aiplatform.googleapis.com/v1/publishers/google/models/{model}:{op}key={}",
                url_encode(key)
            ),
            VertexTransport::Gemini,
        ))
    } else {
        Err("Vertex AI requires either a project_id or an Express API key".into())
    }
}

pub fn downgrade_legacy_vertex_claude_ttl(body: &mut Value, model: &str) {
    let base = model.to_ascii_lowercase();
    let prefix = base.split('@').next().unwrap_or(&base);
    if !VERTEX_LEGACY_1H_TTL_MODELS.iter().any(|m| prefix.starts_with(m)) {
        return;
    }
    fn strip_ttl(val: &mut Value) {
        match val {
            Value::Object(map) => {
                if let Some(Value::Object(cc)) = map.get_mut("cache_control") {
                    if cc.get("ttl").and_then(Value::as_str) == Some("1h") {
                        cc.remove("ttl");
                    }
                }
                for v in map.values_mut() {
                    strip_ttl(v);
                }
            }
            Value::Array(arr) => {
                for item in arr {
                    strip_ttl(item);
                }
            }
            _ => {}
        }
    }
    strip_ttl(body);
}

// ── AWS Bedrock Converse + Pure-Rust AWS SigV4 Signer ───────────────

pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut k_pad = [0u8; BLOCK];
    if key.len() > BLOCK {
        let mut h = Sha256::new();
        h.update(key);
        k_pad[..32].copy_from_slice(&h.finalize());
    } else {
        k_pad[..key.len()].copy_from_slice(key);
    }
    let mut i_key_pad = [0x36u8; BLOCK];
    let mut o_key_pad = [0x5cu8; BLOCK];
    for i in 0..BLOCK {
        i_key_pad[i] ^= k_pad[i];
        o_key_pad[i] ^= k_pad[i];
    }
    let mut inner = Sha256::new();
    inner.update(i_key_pad);
    inner.update(data);
    let inner_hash = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(o_key_pad);
    outer.update(inner_hash);
    let mut out = [0u8; 32];
    out.copy_from_slice(&outer.finalize());
    out
}

pub fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex::encode(h.finalize())
}

#[derive(Debug, Clone)]
pub struct AwsSigV4Headers {
    pub authorization: String,
    pub amz_date: String,
    pub content_sha256: String,
    pub security_token: Option<String>,
}

/// Pure-Rust AWS Signature Version 4 (AWS4-HMAC-SHA256) request signer for Bedrock Runtime.
#[allow(clippy::too_many_arguments)]
pub fn sign_aws_sigv4(
    method: &str,
    url: &str,
    region: &str,
    service: &str,
    access_key: &str,
    secret_key: &str,
    session_token: Option<&str>,
    body: &[u8],
    now: DateTime<Utc>,
) -> Result<AwsSigV4Headers, String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid AWS URL: {e}"))?;
    let host = match parsed.port() {
        Some(p) => format!("{}:{p}", parsed.host_str().unwrap_or_default()),
        None => parsed.host_str().unwrap_or_default().to_string(),
    };
    let canonical_uri = if parsed.path().is_empty() {
        "/"
    } else {
        parsed.path()
    };
    let mut query_pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .map(|(k, v)| (url_encode(&k), url_encode(&v)))
        .collect();
    query_pairs.sort();
    let canonical_query = query_pairs
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");

    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date_stamp = now.format("%Y%m%d").to_string();
    let payload_hash = sha256_hex(body);

    let mut hdrs: Vec<(&str, String)> = vec![
        ("host", host),
        ("x-amz-content-sha256", payload_hash.clone()),
        ("x-amz-date", amz_date.clone()),
    ];
    if let Some(tok) = session_token.filter(|s| !s.is_empty()) {
        hdrs.push(("x-amz-security-token", tok.to_string()));
    }
    hdrs.sort_by(|a, b| a.0.cmp(b.0));

    let canonical_headers: String = hdrs
        .iter()
        .map(|(k, v)| format!("{k}:{}\n", v.trim()))
        .collect();
    let signed_headers: String = hdrs
        .iter()
        .map(|(k, _)| *k)
        .collect::<Vec<_>>()
        .join(";");

    let canonical_request = format!(
        "{method}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
    );
    let credential_scope = format!("{date_stamp}/{region}/{service}/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );

    let k_date = hmac_sha256(format!("AWS4{secret_key}").as_bytes(), date_stamp.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    let k_signing = hmac_sha256(&k_service, b"aws4_request");
    let signature = hex::encode(hmac_sha256(&k_signing, string_to_sign.as_bytes()));

    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={access_key}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}"
    );

    Ok(AwsSigV4Headers {
        authorization,
        amz_date,
        content_sha256: payload_hash,
        security_token: session_token.filter(|s| !s.is_empty()).map(String::from),
    })
}

pub fn build_bedrock_converse_url(
    base_url_override: Option<&str>,
    region: &str,
    model: &str,
    stream: bool,
) -> Result<String, String> {
    let op = if stream { "converse-stream" } else { "converse" };
    let encoded_model = url_encode(model);
    if let Some(custom) = base_url_override.filter(|s| !s.is_empty()) {
        let trimmed = custom.trim_end_matches('/');
        if trimmed.ends_with("/converse") || trimmed.ends_with("/converse-stream") {
            return Ok(trimmed.to_string());
        }
        return Ok(format!("{trimmed}/model/{encoded_model}/{op}"));
    }
    let valid_region = assert_valid_aws_region(region)?;
    Ok(format!(
        "https://bedrock-runtime.{valid_region}.amazonaws.com/model/{encoded_model}/{op}"
    ))
}

/// Translate OpenAI ChatRequest to AWS Bedrock Converse API payload.
pub fn openai_to_bedrock_converse(req: &ChatRequest) -> Value {
    let mut system: Vec<Value> = Vec::new();
    let mut messages: Vec<Value> = Vec::new();
    for m in &req.messages {
        let text = match &m.content {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Array(arr)) => arr
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
            Some(other) => other.to_string(),
            None => String::new(),
        };
        if m.role == "system" || m.role == "developer" {
            if !text.is_empty() {
                system.push(json!({"text": text}));
            }
        } else {
            let role = if m.role == "assistant" { "assistant" } else { "user" };
            messages.push(json!({
                "role": role,
                "content": [{"text": text}]
            }));
        }
    }

    let mut inference_config = serde_json::Map::new();
    if let Some(max_toks) = req.max_tokens.or(req.max_completion_tokens) {
        inference_config.insert("maxTokens".into(), json!(max_toks));
    }
    if let Some(temp) = req.temperature {
        inference_config.insert("temperature".into(), json!(temp));
    }
    if let Some(tp) = req.top_p {
        inference_config.insert("topP".into(), json!(tp));
    }

    let mut out = json!({ "messages": messages });
    if !system.is_empty() {
        out["system"] = Value::Array(system);
    }
    if !inference_config.is_empty() {
        out["inferenceConfig"] = Value::Object(inference_config);
    }
    if let Some(tools) = &req.tools {
        let specs: Vec<Value> = tools
            .iter()
            .filter(|t| !t.function.name.is_empty())
            .map(|t| {
                let desc = t.function.description.as_deref().unwrap_or("");
                let params = t
                    .function
                    .parameters
                    .clone()
                    .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
                json!({
                    "toolSpec": {
                        "name": t.function.name,
                        "description": desc,
                        "inputSchema": {"json": params}
                    }
                })
            })
            .collect();
        if !specs.is_empty() {
            out["toolConfig"] = json!({"tools": specs});
        }
    }
    out
}

/// Normalize AWS Bedrock Converse JSON response to standard OpenAI Chat Completion JSON.
pub fn bedrock_converse_to_openai(model: &str, v: &Value) -> Value {
    let content_blocks = v
        .pointer("/output/message/content")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut text_parts = Vec::new();
    let mut tool_calls = Vec::new();
    for (idx, block) in content_blocks.iter().enumerate() {
        if let Some(t) = block.get("text").and_then(Value::as_str) {
            text_parts.push(t.to_string());
        } else if let Some(tu) = block.get("toolUse") {
            let id = tu
                .get("toolUseId")
                .and_then(Value::as_str)
                .map(String::from)
                .unwrap_or_else(|| format!("call_bedrock_{idx}"));
            let name = tu.get("name").and_then(Value::as_str).unwrap_or("");
            let input = tu.get("input").cloned().unwrap_or_else(|| json!({}));
            tool_calls.push(json!({
                "id": id,
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": input.to_string()
                }
            }));
        }
    }

    let stop_reason = match v.get("stopReason").and_then(Value::as_str) {
        Some("tool_use") => "tool_calls",
        Some("max_tokens") => "length",
        _ => "stop",
    };
    let pt = v.pointer("/usage/inputTokens").and_then(Value::as_i64).unwrap_or(0);
    let ct = v.pointer("/usage/outputTokens").and_then(Value::as_i64).unwrap_or(0);
    let tt = v
        .pointer("/usage/totalTokens")
        .and_then(Value::as_i64)
        .unwrap_or(pt + ct);

    let mut msg = json!({
        "role": "assistant",
        "content": text_parts.join("")
    });
    if !tool_calls.is_empty() {
        msg["tool_calls"] = Value::Array(tool_calls);
    }

    json!({
        "id": format!("chatcmpl-bedrock-{}", Utc::now().timestamp_millis()),
        "object": "chat.completion",
        "model": model,
        "choices": [{
            "index": 0,
            "message": msg,
            "finish_reason": stop_reason
        }],
        "usage": {
            "prompt_tokens": pt,
            "completion_tokens": ct,
            "total_tokens": tt
        }
    })
}

// ── Unified Priority Executor Dispatcher ────────────────────────────

pub fn prepare_provider_request(
    provider: &ProviderDescriptor,
    mut endpoint: ConnectionEndpoint,
    req: &ChatRequest,
    upstream_model: &str,
    stream: bool,
    provider_specific: &Value,
    unsupported_params: &[String],
) -> PreparedExecution {
    let mut target_req = req.clone();
    target_req.model = upstream_model.to_string();

    let mut extra_response_headers = vec![(
        "X-OmniRoute-Selected-Connection-Id".to_string(),
        endpoint.connection_id.clone(),
    )];

    match provider.id.as_str() {
        // 1. Claude / Anthropic (OAuth Claude Code vs API key + conditional skills beta)
        "claude" | "anthropic" => {
            let is_oauth = endpoint.auth_type == AuthScheme::OAuth
                || endpoint.api_key.starts_with("sk-ant-oat");
            let (body, tool_map) =
                openai_to_claude_with_map(&target_req, stream, is_oauth, unsupported_params);
            upsert_header(&mut endpoint.extra_headers, "anthropic-version", ANTHROPIC_VERSION);

            let mut betas = if is_oauth {
                endpoint.auth_header = AuthHeader::Bearer;
                upsert_header(
                    &mut endpoint.extra_headers,
                    "User-Agent",
                    "claude-cli/2.1.77 (external, cli)",
                );
                upsert_header(&mut endpoint.extra_headers, "x-app", "cli");
                upsert_header(
                    &mut endpoint.extra_headers,
                    "anthropic-dangerous-direct-browser-access",
                    "true",
                );
                ANTHROPIC_BETA_CLAUDE_OAUTH.to_string()
            } else {
                endpoint.auth_header = AuthHeader::XApiKey;
                ANTHROPIC_BETA_API_KEY.to_string()
            };
            if has_code_execution_tool(&target_req) {
                betas.push(',');
                betas.push_str(ANTHROPIC_SKILLS_BETA);
            }
            upsert_header(&mut endpoint.extra_headers, "anthropic-beta", betas);

            if provider_specific
                .get("force_gzip")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                endpoint.force_gzip = true;
            }

            PreparedExecution {
                endpoint,
                body,
                tool_name_map: tool_map,
                response_format: UpstreamResponseFormat::Claude,
                extra_response_headers,
            }
        }

        // 2. Codex & ChatGPT OAuth (Responses API + identity headers + opt-in plan header)
        "codex" | "chatgpt" => {
            upsert_header(&mut endpoint.extra_headers, "originator", "codex_cli_rs");
            upsert_header(
                &mut endpoint.extra_headers,
                "OpenAI-Beta",
                "responses=experimental",
            );
            if let Some(plan) = resolve_codex_plan_header(provider_specific, None) {
                extra_response_headers.push(("X-OmniRoute-Selected-Connection-Plan".into(), plan));
            }
            if let Some(resp_url) = &provider.responses_base_url {
                if !resp_url.is_empty() && endpoint.base_urls.iter().all(|u| !u.contains("127.0.0.1")) {
                    endpoint.base_urls = vec![resp_url.clone()];
                }
            }
            let body = chat_to_responses_request(&target_req, stream);
            PreparedExecution {
                endpoint,
                body,
                tool_name_map: vec![],
                response_format: UpstreamResponseFormat::Responses,
                extra_response_headers,
            }
        }

        // 3. GitHub Copilot / GHE Copilot (token exchange + Claude /messages + Codex /responses guard)
        "github" | "ghe-copilot" => {
            if let Some(ctok) = provider_specific
                .get("copilot_token")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                endpoint.api_key = ctok.to_string();
            }
            endpoint.auth_header = AuthHeader::Bearer;
            upsert_header(&mut endpoint.extra_headers, "Copilot-Integration-Id", "vscode-chat");
            upsert_header(
                &mut endpoint.extra_headers,
                "Editor-Version",
                GITHUB_COPILOT_EDITOR_VERSION,
            );
            upsert_header(
                &mut endpoint.extra_headers,
                "Editor-Plugin-Version",
                GITHUB_COPILOT_PLUGIN_VERSION,
            );
            upsert_header(&mut endpoint.extra_headers, "User-Agent", GITHUB_COPILOT_USER_AGENT);
            upsert_header(&mut endpoint.extra_headers, "Openai-Intent", "conversation-panel");
            upsert_header(
                &mut endpoint.extra_headers,
                "X-GitHub-Api-Version",
                GITHUB_COPILOT_API_VERSION,
            );

            let api_base = provider_specific
                .get("copilot_api_endpoint")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(|s| s.trim_end_matches('/').to_string())
                .unwrap_or_else(|| {
                    endpoint
                        .base_urls
                        .first()
                        .map(|u| {
                            u.trim_end_matches("/chat/completions")
                                .trim_end_matches("/v1/messages")
                                .trim_end_matches("/responses")
                                .trim_end_matches('/')
                                .to_string()
                        })
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "https://api.githubcopilot.com".into())
                });

            match select_copilot_route(upstream_model) {
                CopilotRouteMode::AnthropicMessages => {
                    endpoint.base_urls = vec![format!("{api_base}/v1/messages")];
                    upsert_header(&mut endpoint.extra_headers, "anthropic-version", ANTHROPIC_VERSION);
                    let (body, tool_map) =
                        openai_to_claude_with_map(&target_req, stream, false, unsupported_params);
                    PreparedExecution {
                        endpoint,
                        body,
                        tool_name_map: tool_map,
                        response_format: UpstreamResponseFormat::Claude,
                        extra_response_headers,
                    }
                }
                CopilotRouteMode::OpenAiResponses => {
                    endpoint.base_urls = vec![format!("{api_base}/responses")];
                    let body = chat_to_responses_request(&target_req, stream);
                    PreparedExecution {
                        endpoint,
                        body,
                        tool_name_map: vec![],
                        response_format: UpstreamResponseFormat::Responses,
                        extra_response_headers,
                    }
                }
                CopilotRouteMode::ChatCompletions => {
                    endpoint.base_urls = vec![format!("{api_base}/chat/completions")];
                    let mut v = serde_json::to_value(&target_req).unwrap_or_else(|_| json!({}));
                    if let Some(obj) = v.as_object_mut() {
                        obj.remove("_strategy");
                        obj.remove("_disableToolPrefix");
                        for u in unsupported_params {
                            obj.remove(u);
                        }
                    }
                    PreparedExecution {
                        endpoint,
                        body: v,
                        tool_name_map: vec![],
                        response_format: UpstreamResponseFormat::OpenAI,
                        extra_response_headers,
                    }
                }
            }
        }

        // 4. Antigravity & Gemini CLI (OAuth Bearer + multi-endpoint fallback + v1internal envelope
        //    + v3.8.55 anti-ban safeguards: native CLI/IDE UA, header scrub, prompt sanitizer, FNV-1a sessionId)
        "antigravity" | "agy" | "gemini-cli" => {
            endpoint.auth_header = AuthHeader::Bearer;
            let project_id = provider_specific
                .get("project_id")
                .or_else(|| provider_specific.get("projectId"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if !project_id.is_empty() {
                upsert_header(&mut endpoint.extra_headers, "x-goog-user-project", project_id);
            }
            let mut gemini_body = openai_to_gemini(&target_req, stream);
            let is_antigravity = crate::antigravity::is_antigravity_provider(&provider.id);
            let op = if stream {
                "v1internal:streamGenerateContent?alt=sse"
            } else {
                "v1internal:generateContent"
            };
            if is_antigravity {
                // Protection 1: Native CLI/IDE User-Agent + scrub proxy/SDK/Chromium fingerprint headers
                let native_ua = crate::antigravity::antigravity_content_user_agent(
                    &provider.id,
                    provider_specific,
                );
                upsert_header(&mut endpoint.extra_headers, "User-Agent", &native_ua);
                crate::antigravity::scrub_proxy_and_fingerprint_headers(
                    &mut endpoint.extra_headers,
                );

                // Protection 2 & 3: Competing-agent prompt sanitizer + UUIDv4 requestId + FNV-1a sessionId + generation defaults
                crate::antigravity::normalize_antigravity_system_instruction(&mut gemini_body);
                let identity = crate::antigravity::build_envelope_identity(
                    provider_specific,
                    &endpoint.connection_id,
                );
                crate::antigravity::apply_antigravity_request_defaults(
                    &mut gemini_body,
                    &identity.session_id,
                );

                let uses_default = endpoint.base_urls.is_empty()
                    || endpoint
                        .base_urls
                        .iter()
                        .all(|u| u.contains("cloudcode-pa") || u.contains("googleapis.com"));
                if uses_default {
                    endpoint.base_urls = ANTIGRAVITY_DEFAULT_ENDPOINTS
                        .iter()
                        .map(|b| format!("{b}/{op}"))
                        .collect();
                }
                let envelope = json!({
                    "project": project_id,
                    "model": upstream_model,
                    "userAgent": "antigravity",
                    "requestType": "agent",
                    "requestId": identity.request_id,
                    "request": gemini_body
                });
                PreparedExecution {
                    endpoint,
                    body: envelope,
                    tool_name_map: vec![],
                    response_format: UpstreamResponseFormat::Gemini,
                    extra_response_headers,
                }
            } else {
                PreparedExecution {
                    endpoint,
                    body: gemini_body,
                    tool_name_map: vec![],
                    response_format: UpstreamResponseFormat::Gemini,
                    extra_response_headers,
                }
            }
        }

        // 5. AWS Bedrock (Converse API + SigV4 signing OR Bedrock Native Bearer key)
        "bedrock" => {
            let region = provider_specific
                .get("region")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("us-east-1");
            let custom_base = endpoint
                .base_urls
                .first()
                .filter(|u| u.contains("127.0.0.1") || u.contains("localhost") || !u.contains("amazonaws.com"))
                .map(|u| {
                    u.trim_end_matches("/v1/chat/completions")
                        .trim_end_matches("/chat/completions")
                        .to_string()
                });
            if let Ok(url) = build_bedrock_converse_url(custom_base.as_deref(), region, upstream_model, stream) {
                endpoint.base_urls = vec![url];
            }
            let body = openai_to_bedrock_converse(&target_req);

            // Check for IAM SigV4 credentials in provider_specific_json or `AKIA...:SECRET`
            let access_key = provider_specific
                .get("aws_access_key_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty());
            let secret_key = provider_specific
                .get("aws_secret_access_key")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty());
            let session_token = provider_specific
                .get("aws_session_token")
                .and_then(Value::as_str);

            if let (Some(ak), Some(sk)) = (access_key, secret_key) {
                let raw_json = serde_json::to_string(&body).unwrap_or_else(|_| "{}".into());
                if let Some(target_url) = endpoint.base_urls.first() {
                    if let Ok(sig) = sign_aws_sigv4(
                        "POST",
                        target_url,
                        region,
                        "bedrock",
                        ak,
                        sk,
                        session_token,
                        raw_json.as_bytes(),
                        Utc::now(),
                    ) {
                        endpoint.auth_header = AuthHeader::None;
                        upsert_header(&mut endpoint.extra_headers, "Authorization", sig.authorization);
                        upsert_header(&mut endpoint.extra_headers, "x-amz-date", sig.amz_date);
                        upsert_header(
                            &mut endpoint.extra_headers,
                            "x-amz-content-sha256",
                            sig.content_sha256,
                        );
                        if let Some(st) = sig.security_token {
                            upsert_header(&mut endpoint.extra_headers, "x-amz-security-token", st);
                        }
                        endpoint.raw_body_override = Some(raw_json);
                    }
                }
            } else {
                // Bedrock Native API key Bearer mode
                endpoint.auth_header = AuthHeader::Bearer;
            }

            PreparedExecution {
                endpoint,
                body,
                tool_name_map: vec![],
                response_format: UpstreamResponseFormat::BedrockConverse,
                extra_response_headers,
            }
        }

        // 6. GCP Vertex AI (Express key vs Project-scoped Gemini/Claude/Mistral/OpenAI Partner)
        "vertex" | "vertex-partner" => {
            let region = provider_specific
                .get("region")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("us-central1");
            let project = provider_specific
                .get("project_id")
                .or_else(|| provider_specific.get("project"))
                .and_then(Value::as_str);
            let is_express = project.is_none() && endpoint.auth_type != AuthScheme::OAuth;
            let express_key = if is_express && !endpoint.api_key.is_empty() {
                Some(endpoint.api_key.as_str())
            } else {
                None
            };
            if let Ok((url, transport)) =
                build_vertex_url(upstream_model, stream, project, region, express_key)
            {
                if endpoint.base_urls.iter().all(|u| !u.contains("127.0.0.1")) {
                    endpoint.base_urls = vec![url];
                }
                if express_key.is_some() {
                    endpoint.auth_header = AuthHeader::None;
                } else {
                    endpoint.auth_header = AuthHeader::Bearer;
                }
                return match transport {
                    VertexTransport::Anthropic => {
                        let (mut body, tool_map) =
                            openai_to_claude_with_map(&target_req, stream, false, unsupported_params);
                        if let Some(obj) = body.as_object_mut() {
                            obj.insert("anthropic_version".into(), json!("vertex-2023-10-16"));
                            obj.remove("model");
                        }
                        downgrade_legacy_vertex_claude_ttl(&mut body, upstream_model);
                        PreparedExecution {
                            endpoint,
                            body,
                            tool_name_map: tool_map,
                            response_format: UpstreamResponseFormat::Claude,
                            extra_response_headers,
                        }
                    }
                    VertexTransport::Gemini => PreparedExecution {
                        endpoint,
                        body: openai_to_gemini(&target_req, stream),
                        tool_name_map: vec![],
                        response_format: UpstreamResponseFormat::Gemini,
                        extra_response_headers,
                    },
                    VertexTransport::Mistral | VertexTransport::OpenAiPartner => {
                        let v = serde_json::to_value(&target_req).unwrap_or_else(|_| json!({}));
                        PreparedExecution {
                            endpoint,
                            body: v,
                            tool_name_map: vec![],
                            response_format: UpstreamResponseFormat::OpenAI,
                            extra_response_headers,
                        }
                    }
                };
            }
            PreparedExecution {
                endpoint,
                body: openai_to_gemini(&target_req, stream),
                tool_name_map: vec![],
                response_format: UpstreamResponseFormat::Gemini,
                extra_response_headers,
            }
        }

        // 7. Azure OpenAI / Azure AI Foundry (Foundry vs Classic URL detection)
        "azure" | "azure-ai" | "azure-openai" => {
            endpoint.auth_header = AuthHeader::AzureApiKey;
            let api_version = provider_specific
                .get("api_version")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("2024-10-21");
            let use_responses = provider_specific
                .get("use_responses")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            endpoint.base_urls = endpoint
                .base_urls
                .iter()
                .map(|u| build_azure_url(u, upstream_model, api_version, use_responses))
                .collect();
            let (body, resp_fmt) = if use_responses {
                (
                    chat_to_responses_request(&target_req, stream),
                    UpstreamResponseFormat::Responses,
                )
            } else {
                let mut v = serde_json::to_value(&target_req).unwrap_or_else(|_| json!({}));
                if let Some(obj) = v.as_object_mut() {
                    obj.remove("_strategy");
                    obj.remove("_disableToolPrefix");
                    for u in unsupported_params {
                        obj.remove(u);
                    }
                }
                (v, UpstreamResponseFormat::OpenAI)
            };
            PreparedExecution {
                endpoint,
                body,
                tool_name_map: vec![],
                response_format: resp_fmt,
                extra_response_headers,
            }
        }

        // Default generic catalog executor
        _ => match provider.effective_wire_format(&endpoint.model) {
            WireFormat::OpenAI => {
                let mut v = serde_json::to_value(&target_req).unwrap_or_else(|_| json!({}));
                if let Some(obj) = v.as_object_mut() {
                    obj.remove("_strategy");
                    obj.remove("_disableToolPrefix");
                    for u in unsupported_params {
                        obj.remove(u);
                    }
                }
                PreparedExecution {
                    endpoint,
                    body: v,
                    tool_name_map: vec![],
                    response_format: UpstreamResponseFormat::OpenAI,
                    extra_response_headers,
                }
            }
            WireFormat::OpenAIResponses => PreparedExecution {
                endpoint,
                body: chat_to_responses_request(&target_req, stream),
                tool_name_map: vec![],
                response_format: UpstreamResponseFormat::Responses,
                extra_response_headers,
            },
            WireFormat::Claude => {
                let has_oauth = endpoint.auth_type == AuthScheme::OAuth;
                let (body, tool_map) =
                    openai_to_claude_with_map(&target_req, stream, has_oauth, unsupported_params);
                PreparedExecution {
                    endpoint,
                    body,
                    tool_name_map: tool_map,
                    response_format: UpstreamResponseFormat::Claude,
                    extra_response_headers,
                }
            }
            WireFormat::Gemini => PreparedExecution {
                endpoint,
                body: openai_to_gemini(&target_req, stream),
                tool_name_map: vec![],
                response_format: UpstreamResponseFormat::Gemini,
                extra_response_headers,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use omni_catalog::Catalog;
    use omni_types::{ChatMessage, FunctionDef, Tool};

    fn sample_req(model: &str, with_code_exec: bool) -> ChatRequest {
        ChatRequest {
            model: model.into(),
            messages: vec![ChatMessage {
                role: "user".into(),
                content: Some(Value::String("hello".into())),
                name: None,
                tool_call_id: None,
                tool_calls: None,
                extra: Value::Null,
            }],
            stream: Some(false),
            temperature: Some(0.2),
            top_p: None,
            max_tokens: Some(128),
            max_completion_tokens: None,
            tools: if with_code_exec {
                Some(vec![Tool {
                    kind: "code_execution_20250522".into(),
                    function: FunctionDef {
                        name: "code_execution".into(),
                        description: None,
                        parameters: None,
                    },
                }])
            } else {
                None
            },
            tool_choice: None,
            stop: None,
            reasoning_effort: None,
            response_format: None,
            user: None,
            extra: json!({}),
        }
    }

    fn base_endpoint(provider: &str, model: &str, auth: AuthScheme) -> ConnectionEndpoint {
        ConnectionEndpoint {
            provider: provider.into(),
            connection_id: "acct-1".into(),
            model: model.into(),
            auth_type: auth,
            base_urls: vec!["https://example.invalid".into()],
            api_key: "secret-key".into(),
            auth_header: AuthHeader::Bearer,
            auth_prefix: None,
            extra_headers: vec![],
            timeout_ms: None,
            raw_body_override: None,
            force_gzip: false,
        }
    }

    #[test]
    fn claude_oauth_vs_apikey_and_conditional_skills_beta() {
        let cat = Catalog::with_seed();
        let claude = cat.find("claude").unwrap();

        let p_no_ce = prepare_provider_request(
            claude,
            base_endpoint("claude", "claude-opus-4-7", AuthScheme::OAuth),
            &sample_req("claude-opus-4-7", false),
            "claude-opus-4-7",
            false,
            &json!({}),
            &[],
        );
        let beta_no = p_no_ce
            .endpoint
            .extra_headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("anthropic-beta"))
            .unwrap()
            .1
            .clone();
        assert!(beta_no.contains("oauth-2025-04-20"));
        assert!(!beta_no.contains(ANTHROPIC_SKILLS_BETA), "skills beta must be gated on code_execution");

        let p_ce = prepare_provider_request(
            claude,
            base_endpoint("claude", "claude-opus-4-7", AuthScheme::ApiKey),
            &sample_req("claude-opus-4-7", true),
            "claude-opus-4-7",
            false,
            &json!({}),
            &[],
        );
        let beta_yes = p_ce
            .endpoint
            .extra_headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("anthropic-beta"))
            .unwrap()
            .1
            .clone();
        assert!(beta_yes.contains(ANTHROPIC_SKILLS_BETA));
        assert_eq!(p_ce.endpoint.auth_header, AuthHeader::XApiKey);
    }

    #[test]
    fn copilot_routes_claude_to_messages_codex_to_responses_and_guards_gemini() {
        assert_eq!(
            select_copilot_route("claude-sonnet-4-5"),
            CopilotRouteMode::AnthropicMessages
        );
        assert_eq!(
            select_copilot_route("gpt-5.3-codex"),
            CopilotRouteMode::OpenAiResponses
        );
        assert_eq!(
            select_copilot_route("gemini-2.5-pro"),
            CopilotRouteMode::ChatCompletions
        );
        assert_eq!(
            select_copilot_route("gpt-4o"),
            CopilotRouteMode::ChatCompletions
        );
    }

    #[test]
    fn azure_foundry_vs_classic_url_builder() {
        assert_eq!(
            build_azure_url(
                "https://my-foundry.services.ai.azure.com",
                "gpt-4o",
                "2024-10-21",
                false
            ),
            "https://my-foundry.services.ai.azure.com/openai/v1/chat/completions"
        );
        assert_eq!(
            build_azure_url(
                "https://my-foundry.services.ai.azure.com/openai/v1",
                "gpt-5",
                "2024-10-21",
                true
            ),
            "https://my-foundry.services.ai.azure.com/openai/v1/responses"
        );
        assert_eq!(
            build_azure_url(
                "https://my-res.openai.azure.com",
                "gpt-4o-deploy",
                "2024-10-21",
                false
            ),
            "https://my-res.openai.azure.com/openai/deployments/gpt-4o-deploy/chat/completions?api-version=2024-10-21"
        );
    }

    #[test]
    fn vertex_express_vs_project_scoped_and_legacy_ttl_downgrade() {
        let (u_exp, t_exp) = build_vertex_url("gemini-2.5-pro", true, None, "us-central1", Some("AIzaTest")).unwrap();
        assert_eq!(t_exp, VertexTransport::Gemini);
        assert!(u_exp.contains("streamGenerateContent?alt=sse&key=AIzaTest"));

        assert!(build_vertex_url("claude-sonnet-4", false, None, "us-central1", Some("AIzaTest")).is_err());

        let (u_claude, t_claude) =
            build_vertex_url("claude-3-7-sonnet@20250219", true, Some("my-proj"), "us-east5", None).unwrap();
        assert_eq!(t_claude, VertexTransport::Anthropic);
        assert!(u_claude.ends_with("/publishers/anthropic/models/claude-3-7-sonnet@20250219:rawPredict"));

        let mut body = json!({
            "system": [{"type": "text", "text": "sys", "cache_control": {"type": "ephemeral", "ttl": "1h"}}]
        });
        downgrade_legacy_vertex_claude_ttl(&mut body, "claude-3-7-sonnet@20250219");
        assert!(body["system"][0]["cache_control"].get("ttl").is_none());
    }

    #[test]
    fn bedrock_sigv4_signature_and_converse_roundtrip() {
        let url = build_bedrock_converse_url(
            None,
            "us-east-1",
            "anthropic.claude-3-5-sonnet-20241022-v2:0",
            false,
        )
        .unwrap();
        assert_eq!(
            url,
            "https://bedrock-runtime.us-east-1.amazonaws.com/model/anthropic.claude-3-5-sonnet-20241022-v2%3A0/converse"
        );
        assert!(build_bedrock_converse_url(None, "127.0.0.1", "m", false).is_err());

        let ts = Utc.with_ymd_and_hms(2026, 11, 25, 12, 0, 0).unwrap();
        let sig = sign_aws_sigv4(
            "POST",
            &url,
            "us-east-1",
            "bedrock",
            "AKIAEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            Some("sess-tok"),
            br#"{"messages":[]}"#,
            ts,
        )
        .unwrap();
        assert!(sig.authorization.starts_with(
            "AWS4-HMAC-SHA256 Credential=AKIAEXAMPLE/20261125/us-east-1/bedrock/aws4_request, SignedHeaders=host;x-amz-content-sha256;x-amz-date;x-amz-security-token, Signature="
        ));
        assert_eq!(sig.amz_date, "20261125T120000Z");

        // Converse request & response translation
        let conv = openai_to_bedrock_converse(&sample_req("anthropic.claude-3-5-sonnet", false));
        assert_eq!(conv["messages"][0]["content"][0]["text"], "hello");
        assert_eq!(conv["inferenceConfig"]["maxTokens"], 128);

        let oai = bedrock_converse_to_openai(
            "anthropic.claude-3-5-sonnet",
            &json!({
                "output": {"message": {"role": "assistant", "content": [{"text": "Hi from Bedrock"}]}},
                "stopReason": "end_turn",
                "usage": {"inputTokens": 7, "outputTokens": 4, "totalTokens": 11}
            }),
        );
        assert_eq!(oai["choices"][0]["message"]["content"], "Hi from Bedrock");
        assert_eq!(oai["usage"]["total_tokens"], 11);
    }

    #[test]
    fn codex_plan_header_allowlist_only() {
        assert_eq!(
            resolve_codex_plan_header(&json!({"plan": "Pro Max"}), Some(true)).as_deref(),
            Some("promax")
        );
        assert_eq!(
            resolve_codex_plan_header(&json!({"plan": "Pro Max"}), Some(false)),
            None
        );
        assert_eq!(
            resolve_codex_plan_header(&json!({"plan": "evil\r\ninjection"}), Some(true)),
            None
        );
    }
}
