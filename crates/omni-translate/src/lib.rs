// Bidirectional request & response/SSE translators:
//   - OpenAI Chat Completions <-> Claude Messages API (JSON + SSE)
//   - OpenAI Chat Completions -> Gemini generateContent (request + response/SSE normalization)
// Fixes: Claude tool_use.input serialized as JSON object (not raw string), OAuth-only
// proxy_ tool prefixing on both tools and tool_use blocks, no internal _toolNameMap leak,
// unsupported_params stripping (Opus 4.7/4.8/5, Fable 5, Sonnet 5, reasoning models).

use omni_types::{ChatMessage, ChatRequest, FunctionDef, Tool, ToolCall, ToolCallFn, CLAUDE_OAUTH_TOOL_PREFIX};
use serde_json::{json, Map, Value};

pub mod extended;
pub use extended::*;

fn content_to_string(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| {
                if p.get("type")?.as_str()? == "text" {
                    p.get("text")?.as_str().map(|s| s.to_string())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join(""),
        Some(other) => other.to_string(),
    }
}

fn as_arg_object(tool_call: &Value) -> Value {
    match tool_call.pointer("/function/arguments") {
        Some(Value::String(s)) => serde_json::from_str::<Value>(s)
            .ok()
            .filter(|v| v.is_object())
            .unwrap_or_else(|| json!({})),
        Some(obj @ Value::Object(_)) => obj.clone(),
        _ => json!({}),
    }
}

fn message_to_claude_blocks(
    msg: &Value,
    apply_oauth_prefix: bool,
    prefix_map: &mut Vec<(String, String)>,
) -> Vec<Value> {
    let role = msg.get("role").and_then(Value::as_str).unwrap_or("user");
    let mut blocks: Vec<Value> = Vec::new();

    if role == "tool" {
        let tool_use_id = msg
            .get("tool_call_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        let content_raw = msg.get("content");
        let (mut content_items, is_error) = match content_raw {
            Some(Value::String(s)) => (vec![json!({"type": "text", "text": s})], false),
            Some(Value::Array(a)) => {
                let has_err = a
                    .iter()
                    .any(|b| b.get("type").and_then(Value::as_str) == Some("tool_result_error"));
                (a.to_vec(), has_err)
            }
            Some(other) if !other.is_null() => {
                (vec![json!({"type": "text", "text": other.to_string()})], false)
            }
            _ => (vec![], false),
        };
        content_items.iter_mut().for_each(|b| {
            if let Some(o) = b.as_object_mut() {
                o.remove("cache_control");
            }
        });
        let mut block = json!({
            "type": "tool_result",
            "tool_use_id": tool_use_id,
            "content": content_items,
        });
        if is_error {
            block["is_error"] = json!(true);
        }
        blocks.push(block);
        return blocks;
    }

    match msg.get("content") {
        Some(Value::String(s)) if !s.is_empty() => {
            blocks.push(json!({"type": "text", "text": s}));
        }
        Some(Value::Array(parts)) => {
            for part in parts {
                let ptype = part.get("type").and_then(Value::as_str).unwrap_or("text");
                match ptype {
                    "text" => {
                        let t = part.get("text").and_then(Value::as_str).unwrap_or("");
                        if !t.is_empty() {
                            blocks.push(json!({"type": "text", "text": t}));
                        }
                    }
                    "image_url" | "input_image" => {
                        if let Some(url) = part.pointer("/image_url/url").and_then(Value::as_str) {
                            if let Some(rest) = url.strip_prefix("data:") {
                                let (meta, data) = rest.split_once(',').unwrap_or(("image/png", ""));
                                let media = meta.split(';').next().unwrap_or("image/png");
                                if !data.is_empty() {
                                    blocks.push(json!({
                                        "type": "image",
                                        "source": {"type": "base64", "media_type": media, "data": data}
                                    }));
                                }
                            }
                        }
                    }
                    _ => {
                        if let Some(t) = part.get("text").and_then(Value::as_str) {
                            if !t.is_empty() {
                                blocks.push(json!({"type": "text", "text": t}));
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }

    if role == "assistant" {
        if let Some(calls) = msg.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                let name = call
                    .pointer("/function/name")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let raw_id = call.get("id").and_then(Value::as_str).unwrap_or("");
                let out_name = if apply_oauth_prefix && !name.starts_with(CLAUDE_OAUTH_TOOL_PREFIX) {
                    let prefixed = format!("{CLAUDE_OAUTH_TOOL_PREFIX}{name}");
                    if !prefix_map.iter().any(|(p, _)| p == &prefixed) {
                        prefix_map.push((prefixed.clone(), name.to_string()));
                    }
                    prefixed
                } else {
                    name.to_string()
                };
                blocks.push(json!({
                    "type": "tool_use",
                    "id": raw_id,
                    "name": out_name,
                    "input": as_arg_object(call),
                }));
            }
        }
        if let Some(rc) = msg.get("reasoning_content").and_then(Value::as_str) {
            if !rc.is_empty() {
                let sig = msg
                    .get("thinking_signature")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                blocks.insert(
                    0,
                    json!({"type": "thinking", "thinking": rc, "signature": sig}),
                );
            }
        }
    }

    blocks
}

fn enforce_tool_result_adjacency(messages: &mut Vec<Value>) {
    let mut reordered: Vec<Value> = Vec::new();
    for msg in messages.iter() {
        let role = msg.get("role").and_then(Value::as_str).unwrap_or("user");
        if role != "user" {
            reordered.push(msg.clone());
            continue;
        }
        if let Some(content) = msg.get("content").and_then(Value::as_array).cloned() {
            let (results, rest): (Vec<Value>, Vec<Value>) = content
                .into_iter()
                .partition(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"));
            if !results.is_empty() && !rest.is_empty() {
                reordered.push(json!({"role": "user", "content": results}));
                reordered.push(json!({"role": "user", "content": rest}));
            } else {
                reordered.push(msg.clone());
            }
        } else {
            reordered.push(msg.clone());
        }
    }
    *messages = reordered;
}

/// Map reasoning_effort -> Claude thinking config (adaptive-only families use output_config.effort).
pub fn effort_to_thinking(model: &str, effort: &str) -> Option<(Value, Option<Value>)> {
    let lower_model = model.to_ascii_lowercase();
    let adaptive_only = lower_model.contains("opus-4-7")
        || lower_model.contains("opus-4.7")
        || lower_model.contains("opus-4-8")
        || lower_model.contains("opus-4.8")
        || lower_model.contains("opus-5")
        || lower_model.contains("fable")
        || lower_model.contains("sonnet-5");
    let effort = effort.to_ascii_lowercase();
    if adaptive_only {
        return Some((
            json!({"type": "adaptive"}),
            Some(json!({"effort": effort})),
        ));
    }
    match effort.as_str() {
        "low" => Some((json!({"type": "enabled", "budget_tokens": 1024}), None)),
        "medium" => Some((json!({"type": "enabled", "budget_tokens": 10240}), None)),
        "high" | "max" | "xhigh" => Some((
            json!({"type": "enabled", "budget_tokens": 131_072}),
            None,
        )),
        _ => None,
    }
}

const DEFAULT_MAX_TOKENS: u64 = 64_000;
const DEFAULT_MIN_TOKENS: u64 = 32_000;

fn resolve_max_tokens(body: &ChatRequest) -> u64 {
    let explicit = body
        .max_tokens
        .or(body.max_completion_tokens)
        .or_else(|| body.extra.get("max_output_tokens").and_then(Value::as_u64));
    explicit
        .map(|v| if v == 0 { DEFAULT_MAX_TOKENS } else { v })
        .unwrap_or(DEFAULT_MAX_TOKENS)
        .max(if body.tools.is_some() { DEFAULT_MIN_TOKENS } else { 0 })
}

/// Full OpenAI Chat -> Claude Messages translation returning clean upstream body + tool rename map.
pub fn openai_to_claude_with_map(
    body: &ChatRequest,
    stream: bool,
    has_oauth: bool,
    unsupported_params: &[String],
) -> (Value, Vec<(String, String)>) {
    let mut out = Map::new();
    out.insert("model".into(), json!(body.model));
    let mut max_tokens = resolve_max_tokens(body);
    out.insert("stream".into(), json!(stream));

    let model = &body.model;
    let mut thinking: Option<Value> = None;
    let mut output_config: Option<Value> = None;

    if let Some(r) = &body.reasoning_effort {
        if let Some((t, oc)) = effort_to_thinking(model, r) {
            thinking = Some(t);
            output_config = oc;
        }
    }

    if let Some(t) = &thinking {
        if let Some(budget) = t.get("budget_tokens").and_then(Value::as_u64) {
            max_tokens = max_tokens.max(budget + 1);
            out.insert(
                "thinking".into(),
                json!({"type": "enabled", "budget_tokens": budget}),
            );
        } else {
            out.insert("thinking".into(), t.clone());
        }
    }
    if let Some(oc) = output_config {
        out.insert("output_config".into(), oc);
    }
    out.insert("max_tokens".into(), json!(max_tokens));

    let thinking_on =
        out.contains_key("thinking") && out["thinking"].get("type") != Some(&json!("disabled"));
    let is_unsupported = |p: &str| unsupported_params.iter().any(|u| u == p);

    if let Some(t) = body.temperature {
        if !thinking_on
            && !is_unsupported("temperature")
            && !model.contains("opus-4.7")
            && !model.contains("opus-4-7")
            && !model.contains("opus-4.8")
            && !model.contains("opus-5")
            && !model.contains("fable")
            && !model.contains("sonnet-5")
        {
            out.insert("temperature".into(), json!(t));
        }
    } else if let Some(p) = body.top_p {
        if !thinking_on && !is_unsupported("top_p") {
            out.insert("top_p".into(), json!(p));
        }
    }

    let stops: Vec<String> = match &body.stop {
        None => vec![],
        Some(Value::String(s)) => {
            if s.is_empty() { vec![] } else { vec![s.clone()] }
        }
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect(),
        _ => vec![],
    };
    if !stops.is_empty() {
        out.insert("stop_sequences".into(), json!(stops));
    }

    let tool_prefix_disabled = body
        .extra
        .get("_disableToolPrefix")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let apply_oauth_prefix = has_oauth && !tool_prefix_disabled;

    let mut system_parts: Vec<String> = Vec::new();
    let mut prefix_map: Vec<(String, String)> = Vec::new();
    let mut messages: Vec<Value> = Vec::new();

    for msg in &body.messages {
        let role = msg.role.as_str();
        if role == "system" || role == "developer" {
            let s = content_to_string(msg.content.as_ref());
            if !s.is_empty() {
                system_parts.push(s);
            }
            continue;
        }
        let raw = serde_json::to_value(msg).unwrap_or(Value::Null);
        let blocks = message_to_claude_blocks(&raw, apply_oauth_prefix, &mut prefix_map);
        let has_result = blocks
            .iter()
            .any(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"));

        let target_role = if role == "user" || role == "tool" {
            "user"
        } else {
            "assistant"
        };

        if has_result {
            let (results, rest): (Vec<Value>, Vec<Value>) = blocks
                .into_iter()
                .partition(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"));
            if !results.is_empty() {
                // Merge consecutive tool_result blocks into the same user message if previous is user tool_results
                let merge_results = messages
                    .last()
                    .map(|m| {
                        m.get("role").and_then(Value::as_str) == Some("user")
                            && m.get("content")
                                .and_then(Value::as_array)
                                .map(|a| {
                                    a.iter().all(|b| {
                                        b.get("type").and_then(Value::as_str) == Some("tool_result")
                                    })
                                })
                                .unwrap_or(false)
                    })
                    .unwrap_or(false);
                if merge_results {
                    if let Some(arr) = messages
                        .last_mut()
                        .and_then(|m| m.get_mut("content"))
                        .and_then(Value::as_array_mut)
                    {
                        arr.extend(results);
                    }
                } else {
                    messages.push(json!({"role": "user", "content": results}));
                }
            }
            if !rest.is_empty() {
                messages.push(json!({"role": target_role, "content": rest}));
            }
            continue;
        }

        let merge = messages
            .last()
            .and_then(|m| m.get("role"))
            .and_then(Value::as_str)
            == Some(target_role);
        if merge {
            if let Some(arr) = messages
                .last_mut()
                .and_then(|m| m.get_mut("content"))
                .and_then(Value::as_array_mut)
            {
                arr.extend(blocks);
            }
        } else if !blocks.is_empty() {
            messages.push(json!({"role": target_role, "content": blocks}));
        }
    }

    messages.retain(|m| {
        !(m.get("role").and_then(Value::as_str) == Some("assistant")
            && m.get("content")
                .and_then(Value::as_array)
                .map(|a| a.is_empty())
                .unwrap_or(false))
    });
    enforce_tool_result_adjacency(&mut messages);

    for msg in messages.iter_mut().rev() {
        if msg.get("role").and_then(Value::as_str) == Some("assistant") {
            if let Some(arr) = msg.get_mut("content").and_then(Value::as_array_mut) {
                if let Some(last) = arr.last_mut() {
                    if let Some(obj) = last.as_object_mut() {
                        if obj.get("type").and_then(Value::as_str) != Some("thinking") {
                            obj.insert("cache_control".into(), json!({"type": "ephemeral"}));
                            break;
                        }
                    }
                }
            }
        }
    }
    out.insert("messages".into(), Value::Array(messages));

    if !system_parts.is_empty() {
        let joined = system_parts.join("\n");
        out.insert(
            "system".into(),
            json!([{"type": "text", "text": joined, "cache_control": {"type": "ephemeral"}}]),
        );
    }

    if let Some(tools) = &body.tools {
        let translated: Vec<Value> = tools
            .iter()
            .map(|tool| {
                let name = if apply_oauth_prefix
                    && !tool.function.name.starts_with(CLAUDE_OAUTH_TOOL_PREFIX)
                {
                    let prefixed = format!("{CLAUDE_OAUTH_TOOL_PREFIX}{}", tool.function.name);
                    if !prefix_map.iter().any(|(p, _)| p == &prefixed) {
                        prefix_map.push((prefixed.clone(), tool.function.name.clone()));
                    }
                    prefixed
                } else {
                    tool.function.name.clone()
                };
                let mut t = json!({
                    "name": name,
                    "input_schema": tool.function.parameters.clone().unwrap_or(json!({"type": "object"})),
                });
                if let Some(desc) = &tool.function.description {
                    t["description"] = json!(desc);
                }
                t
            })
            .collect();
        if !translated.is_empty() {
            out.insert("tools".into(), Value::Array(translated));
        }
    }

    if let Some(tc) = &body.tool_choice {
        if !tc.is_null() {
            let mapped = match tc {
                Value::String(s) => match s.as_str() {
                    "auto" => json!({"type": "auto"}),
                    "none" => json!({"type": "none"}),
                    "required" | "any" => json!({"type": "any"}),
                    _ => json!({"type": "auto"}),
                },
                Value::Object(_) => {
                    let raw_name = tc
                        .pointer("/function/name")
                        .or_else(|| tc.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let name = if apply_oauth_prefix
                        && !raw_name.starts_with(CLAUDE_OAUTH_TOOL_PREFIX)
                    {
                        format!("{CLAUDE_OAUTH_TOOL_PREFIX}{raw_name}")
                    } else {
                        raw_name.to_string()
                    };
                    json!({"type": "tool", "name": name})
                }
                _ => json!({"type": "auto"}),
            };
            if mapped.get("type").and_then(Value::as_str) != Some("none") {
                out.insert("tool_choice".into(), mapped);
            }
        }
    }

    (Value::Object(out), prefix_map)
}

pub fn openai_to_claude(body: &ChatRequest, stream: bool, has_oauth: bool) -> Value {
    openai_to_claude_with_map(body, stream, has_oauth, &[]).0
}

// ── Claude Messages -> OpenAI Chat request ──────────────────────────

pub fn claude_to_openai_request(body: &Value) -> ChatRequest {
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let stream = body.get("stream").and_then(Value::as_bool);
    let max_tokens = body.get("max_tokens").and_then(Value::as_u64);
    let temperature = body.get("temperature").and_then(Value::as_f64);
    let top_p = body.get("top_p").and_then(Value::as_f64);
    let stop = body.get("stop_sequences").cloned();

    let mut messages: Vec<ChatMessage> = Vec::new();
    if let Some(sys) = body.get("system") {
        let sys_text = match sys {
            Value::String(s) => s.clone(),
            Value::Array(arr) => arr
                .iter()
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        };
        if !sys_text.is_empty() {
            messages.push(ChatMessage {
                role: "system".into(),
                content: Some(Value::String(sys_text)),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                extra: json!({}),
            });
        }
    }

    if let Some(arr) = body.get("messages").and_then(Value::as_array) {
        for m in arr {
            let role = m.get("role").and_then(Value::as_str).unwrap_or("user");
            match m.get("content") {
                Some(Value::String(s)) => {
                    messages.push(ChatMessage {
                        role: role.to_string(),
                        content: Some(Value::String(s.clone())),
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
                        extra: json!({}),
                    });
                }
                Some(Value::Array(blocks)) => {
                    let mut text_parts: Vec<Value> = Vec::new();
                    let mut tool_calls: Vec<ToolCall> = Vec::new();
                    let mut tool_results: Vec<ChatMessage> = Vec::new();
                    let mut reasoning = String::new();

                    for b in blocks {
                        let btype = b.get("type").and_then(Value::as_str).unwrap_or("text");
                        match btype {
                            "text" => {
                                if let Some(t) = b.get("text").and_then(Value::as_str) {
                                    text_parts.push(json!({"type": "text", "text": t}));
                                }
                            }
                            "thinking" => {
                                if let Some(t) = b.get("thinking").and_then(Value::as_str) {
                                    reasoning.push_str(t);
                                }
                            }
                            "image" => {
                                if let (Some(media), Some(data)) = (
                                    b.pointer("/source/media_type").and_then(Value::as_str),
                                    b.pointer("/source/data").and_then(Value::as_str),
                                ) {
                                    text_parts.push(json!({
                                        "type": "image_url",
                                        "image_url": {"url": format!("data:{media};base64,{data}")}
                                    }));
                                }
                            }
                            "tool_use" => {
                                let id = b.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                                let name = b.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                                let args = b
                                    .get("input")
                                    .map(|v| v.to_string())
                                    .unwrap_or_else(|| "{}".into());
                                tool_calls.push(ToolCall {
                                    id,
                                    kind: "function".into(),
                                    function: ToolCallFn {
                                        name,
                                        arguments: args,
                                    },
                                });
                            }
                            "tool_result" => {
                                let call_id = b
                                    .get("tool_use_id")
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                                    .to_string();
                                let content_str = content_to_string(b.get("content"));
                                tool_results.push(ChatMessage {
                                    role: "tool".into(),
                                    content: Some(Value::String(content_str)),
                                    name: None,
                                    tool_calls: None,
                                    tool_call_id: Some(call_id),
                                    extra: json!({}),
                                });
                            }
                            _ => {}
                        }
                    }

                    for tr in tool_results {
                        messages.push(tr);
                    }

                    if !text_parts.is_empty() || !tool_calls.is_empty() || !reasoning.is_empty() {
                        let content_val = if text_parts.len() == 1
                            && text_parts[0].get("type").and_then(Value::as_str) == Some("text")
                        {
                            Some(text_parts[0]["text"].clone())
                        } else if text_parts.is_empty() {
                            None
                        } else {
                            Some(Value::Array(text_parts))
                        };
                        let mut extra = Map::new();
                        if !reasoning.is_empty() {
                            extra.insert("reasoning_content".into(), Value::String(reasoning));
                        }
                        messages.push(ChatMessage {
                            role: role.to_string(),
                            content: content_val,
                            name: None,
                            tool_calls: if tool_calls.is_empty() { None } else { Some(tool_calls) },
                            tool_call_id: None,
                            extra: Value::Object(extra),
                        });
                    }
                }
                _ => {}
            }
        }
    }

    let tools = body.get("tools").and_then(Value::as_array).map(|arr| {
        arr.iter()
            .filter_map(|t| {
                let name = t.get("name").and_then(Value::as_str)?.to_string();
                Some(Tool {
                    kind: "function".into(),
                    function: FunctionDef {
                        name,
                        description: t.get("description").and_then(Value::as_str).map(String::from),
                        parameters: t.get("input_schema").cloned(),
                    },
                })
            })
            .collect::<Vec<_>>()
    });

    let tool_choice = body.get("tool_choice").map(|tc| {
        match tc.get("type").and_then(Value::as_str) {
            Some("auto") => json!("auto"),
            Some("any") => json!("required"),
            Some("none") => json!("none"),
            Some("tool") => json!({
                "type": "function",
                "function": {"name": tc.get("name").and_then(Value::as_str).unwrap_or("")}
            }),
            _ => tc.clone(),
        }
    });

    let reasoning_effort = body
        .pointer("/output_config/effort")
        .and_then(Value::as_str)
        .map(String::from)
        .or_else(|| {
            let thinking = body.get("thinking")?;
            match thinking.get("type").and_then(Value::as_str) {
                Some("adaptive") => Some("high".into()),
                Some("enabled") => {
                    let b = thinking.get("budget_tokens").and_then(Value::as_u64).unwrap_or(8192);
                    if b <= 2048 { Some("low".into()) }
                    else if b <= 16_384 { Some("medium".into()) }
                    else { Some("high".into()) }
                }
                _ => None,
            }
        });

    ChatRequest {
        model,
        messages,
        stream,
        temperature,
        top_p,
        max_tokens,
        max_completion_tokens: None,
        stop,
        tools,
        tool_choice,
        reasoning_effort,
        response_format: None,
        user: None,
        extra: json!({}),
    }
}

// ── OpenAI -> Gemini request ────────────────────────────────────────

pub fn openai_to_gemini(body: &ChatRequest, stream: bool) -> Value {
    let mut contents: Vec<Value> = Vec::new();
    let mut system_text: Vec<String> = Vec::new();

    for msg in &body.messages {
        let role = msg.role.as_str();
        if role == "system" || role == "developer" {
            system_text.push(content_to_string(msg.content.as_ref()));
            continue;
        }
        let gem_role = match role {
            "assistant" => "model",
            _ => "user",
        };
        let mut parts: Vec<Value> = Vec::new();
        match &msg.content {
            Some(Value::String(s)) if !s.is_empty() => {
                parts.push(json!({"text": s}));
            }
            Some(Value::Array(items)) => {
                for part in items {
                    let pt = part.get("type").and_then(Value::as_str).unwrap_or("text");
                    if pt == "text" {
                        let t = part.get("text").and_then(Value::as_str).unwrap_or("");
                        if !t.is_empty() {
                            parts.push(json!({"text": t}));
                        }
                    } else if pt == "image_url" {
                        if let Some(url) = part.pointer("/image_url/url").and_then(Value::as_str) {
                            if let Some(rest) = url.strip_prefix("data:") {
                                let (meta, data) = rest.split_once(',').unwrap_or(("", ""));
                                let mime = meta.split(';').next().unwrap_or("image/png");
                                if !data.is_empty() {
                                    parts.push(json!({
                                        "inline_data": {"mime_type": mime, "data": data}
                                    }));
                                }
                            } else {
                                parts.push(json!({"file_data": {"file_uri": url}}));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        if let Some(calls) = &msg.tool_calls {
            parts.extend(calls.iter().map(|c| {
                let args: Value =
                    serde_json::from_str(&c.function.arguments).unwrap_or_else(|_| json!({}));
                json!({"function_call": {"name": c.function.name, "args": args}})
            }));
        }
        if role == "tool" || (role == "user" && msg.tool_call_id.is_some()) {
            let name = msg.name.clone().unwrap_or_else(|| "tool".into());
            let response: Value = match &msg.content {
                Some(Value::String(s)) => serde_json::from_str(s).unwrap_or_else(|_| json!({"result": s})),
                Some(v) if v.is_object() => v.clone(),
                Some(v) => json!({"result": v}),
                None => json!({}),
            };
            parts = vec![json!({
                "function_response": {"name": name, "response": response}
            })];
        }
        contents.push(json!({"role": gem_role, "parts": parts}));
    }

    let mut out = Map::new();
    out.insert("contents".into(), Value::Array(contents));
    if !system_text.is_empty() {
        out.insert(
            "system_instruction".into(),
            json!({"parts": [{"text": system_text.join("\n")}]}),
        );
    }

    let mut gen: Map<String, Value> = Map::new();
    if let Some(t) = body.temperature {
        gen.insert("temperature".into(), json!(t));
    }
    if let Some(p) = body.top_p {
        gen.insert("topP".into(), json!(p));
    }
    if let Some(e) = body.extra.get("top_k").and_then(Value::as_u64) {
        gen.insert("topK".into(), json!(e));
    }
    let max_out = body
        .max_tokens
        .or(body.max_completion_tokens)
        .unwrap_or(DEFAULT_MAX_TOKENS);
    gen.insert("maxOutputTokens".into(), json!(max_out));
    let stops: Vec<String> = match &body.stop {
        Some(Value::String(s)) if !s.is_empty() => vec![s.clone()],
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect(),
        _ => vec![],
    };
    if !stops.is_empty() {
        gen.insert("stopSequences".into(), json!(stops));
    }
    if let Some(effort) = &body.reasoning_effort {
        let t = match effort.as_str() {
            "low" => 1024,
            "medium" => 8192,
            "high" | "auto" | "max" | "xhigh" => 32_768,
            _ => 0,
        };
        gen.insert("thinkingConfig".into(), json!({"thinkingBudget": t}));
    }
    match &body.response_format {
        Some(rf) if rf.get("type").and_then(Value::as_str) == Some("json_object") => {
            gen.insert("responseMimeType".into(), json!("application/json"));
        }
        Some(rf) if rf.get("type").and_then(Value::as_str) == Some("json_schema") => {
            gen.insert("responseMimeType".into(), json!("application/json"));
            if let Some(schema) = rf.pointer("/json_schema/schema") {
                gen.insert("responseSchema".into(), schema.clone());
            }
        }
        _ => {}
    }
    out.insert("generationConfig".into(), Value::Object(gen));

    if let Some(tools) = &body.tools {
        let fns: Vec<Value> = tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.function.name,
                    "description": tool.function.description.clone().unwrap_or_default(),
                    "parameters": tool.function.parameters.clone().unwrap_or(json!({"type": "object"})),
                })
            })
            .collect();
        if !fns.is_empty() {
            out.insert("tools".into(), json!([{"functionDeclarations": fns}]));
        }
    }

    if let Some(tc) = &body.tool_choice {
        let mode = match tc {
            Value::String(s) => match s.as_str() {
                "none" => "NONE",
                "required" | "any" => "ANY",
                _ => "AUTO",
            },
            Value::Object(_) => "ANY",
            _ => "AUTO",
        };
        out.insert(
            "tool_config".into(),
            json!({"function_calling_config": {"mode": mode}}),
        );
    }

    let _ = stream;
    Value::Object(out)
}

// ── Response & Usage Translators ────────────────────────────────────

fn unprefix_tool_name(name: &str, tool_name_map: &[(String, String)]) -> String {
    if let Some((_, orig)) = tool_name_map.iter().find(|(p, _)| p == name) {
        return orig.clone();
    }
    name.to_string()
}

fn now_unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub(crate) fn short_id() -> String {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    format!("{:08x}{:04x}", n, std::process::id() & 0xffff)
}

/// Extract `(prompt_tokens, completion_tokens, total_tokens, cache_read_tokens, cache_write_tokens)`
/// from OpenAI, Anthropic, or Gemini response JSON.
pub fn extract_usage_tokens(v: &Value) -> (i64, i64, i64, i64, i64) {
    if let Some(u) = v.get("usage") {
        let pt = u
            .get("prompt_tokens")
            .or_else(|| u.get("input_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let ct = u
            .get("completion_tokens")
            .or_else(|| u.get("output_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let tt = u
            .get("total_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(pt + ct);
        let cr = u
            .get("cache_read_input_tokens")
            .or_else(|| u.pointer("/prompt_tokens_details/cached_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let cw = u
            .get("cache_creation_input_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        return (pt, ct, tt, cr, cw);
    }
    if let Some(u) = v.get("usageMetadata") {
        let pt = u.get("promptTokenCount").and_then(Value::as_i64).unwrap_or(0);
        let ct = u.get("candidatesTokenCount").and_then(Value::as_i64).unwrap_or(0);
        let tt = u.get("totalTokenCount").and_then(Value::as_i64).unwrap_or(pt + ct);
        let cr = u.get("cachedContentTokenCount").and_then(Value::as_i64).unwrap_or(0);
        return (pt, ct, tt, cr, 0);
    }
    (0, 0, 0, 0, 0)
}

/// Normalize non-stream OpenAI / Claude / Gemini JSON into canonical OpenAI ChatCompletion JSON.
pub fn normalize_non_stream_to_openai(
    model: &str,
    v: Value,
    tool_name_map: &[(String, String)],
) -> Value {
    if v.get("choices").is_some() {
        return v;
    }

    // Antigravity Cloud Code v1internal envelope (`{"response": {"candidates": [...]}}`)
    if let Some(inner) = v.get("response") {
        if inner.get("candidates").is_some() {
            return normalize_non_stream_to_openai(model, inner.clone(), tool_name_map);
        }
    }

    // OpenAI Responses API (`{"object": "response", "output_text": "...", "output": [...]}`)
    if v.get("object").and_then(Value::as_str) == Some("response")
        || v.get("output_text").is_some()
        || v.get("output").and_then(Value::as_array).is_some()
    {
        let mut text = v
            .get("output_text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let mut tool_calls: Vec<Value> = Vec::new();
        if let Some(items) = v.get("output").and_then(Value::as_array) {
            for item in items {
                match item.get("type").and_then(Value::as_str) {
                    Some("message") => {
                        if text.is_empty() {
                            if let Some(parts) = item.get("content").and_then(Value::as_array) {
                                for p in parts {
                                    if let Some(t) = p.get("text").and_then(Value::as_str) {
                                        text.push_str(t);
                                    }
                                }
                            }
                        }
                    }
                    Some("function_call") => {
                        let id = item
                            .get("call_id")
                            .or_else(|| item.get("id"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let name = item.get("name").and_then(Value::as_str).unwrap_or("");
                        let args = item
                            .get("arguments")
                            .and_then(Value::as_str)
                            .unwrap_or("{}");
                        tool_calls.push(json!({
                            "id": id,
                            "type": "function",
                            "function": {"name": name, "arguments": args}
                        }));
                    }
                    _ => {}
                }
            }
        }
        let finish = if !tool_calls.is_empty() {
            "tool_calls"
        } else if v.get("status").and_then(Value::as_str) == Some("incomplete") {
            "length"
        } else {
            "stop"
        };
        let (pt, ct, tt, cr, _) = extract_usage_tokens(&v);
        let mut msg = json!({"role": "assistant", "content": text});
        if !tool_calls.is_empty() {
            msg["tool_calls"] = Value::Array(tool_calls);
        }
        return json!({
            "id": v.get("id").cloned().unwrap_or(json!(format!("chatcmpl-{}", short_id()))),
            "object": "chat.completion",
            "created": now_unix_secs(),
            "model": model,
            "choices": [{"index": 0, "message": msg, "finish_reason": finish}],
            "usage": {
                "prompt_tokens": pt,
                "completion_tokens": ct,
                "total_tokens": tt,
                "prompt_tokens_details": {"cached_tokens": cr}
            }
        });
    }

    // Gemini generateContent
    if v.get("candidates").is_some() {
        let mut text = String::new();
        let mut tool_calls: Vec<Value> = Vec::new();
        if let Some(parts) = v.pointer("/candidates/0/content/parts").and_then(Value::as_array) {
            for (idx, p) in parts.iter().enumerate() {
                if let Some(t) = p.get("text").and_then(Value::as_str) {
                    text.push_str(t);
                }
                if let Some(fc) = p.get("functionCall").or_else(|| p.get("function_call")) {
                    let name = fc.get("name").and_then(Value::as_str).unwrap_or("");
                    let args = fc.get("args").cloned().unwrap_or(json!({}));
                    tool_calls.push(json!({
                        "id": format!("call_{}_{idx}", short_id()),
                        "type": "function",
                        "function": {"name": name, "arguments": args.to_string()}
                    }));
                }
            }
        }
        let finish = if !tool_calls.is_empty() {
            "tool_calls".to_string()
        } else {
            v.pointer("/candidates/0/finishReason")
                .and_then(Value::as_str)
                .map(|s| match s {
                    "STOP" => "stop".to_string(),
                    "MAX_TOKENS" => "length".to_string(),
                    other => other.to_ascii_lowercase(),
                })
                .unwrap_or_else(|| "stop".into())
        };
        let (pt, ct, tt, cr, _) = extract_usage_tokens(&v);
        let mut msg = json!({"role": "assistant", "content": text});
        if !tool_calls.is_empty() {
            msg["tool_calls"] = Value::Array(tool_calls);
        }
        return json!({
            "id": format!("chatcmpl-{}", short_id()),
            "object": "chat.completion",
            "created": now_unix_secs(),
            "model": model,
            "choices": [{"index": 0, "message": msg, "finish_reason": finish}],
            "usage": {
                "prompt_tokens": pt,
                "completion_tokens": ct,
                "total_tokens": tt,
                "prompt_tokens_details": {"cached_tokens": cr}
            }
        });
    }

    // Anthropic Messages
    if v.get("content").is_some() && v.get("role").and_then(Value::as_str) == Some("assistant") {
        let mut text = String::new();
        let mut reasoning = String::new();
        let mut tool_calls: Vec<Value> = Vec::new();
        if let Some(blocks) = v.get("content").and_then(Value::as_array) {
            for b in blocks {
                match b.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(t) = b.get("text").and_then(Value::as_str) {
                            text.push_str(t);
                        }
                    }
                    Some("thinking") => {
                        if let Some(t) = b.get("thinking").and_then(Value::as_str) {
                            reasoning.push_str(t);
                        }
                    }
                    Some("tool_use") => {
                        let id = b.get("id").and_then(Value::as_str).unwrap_or("");
                        let raw_name = b.get("name").and_then(Value::as_str).unwrap_or("");
                        let name = unprefix_tool_name(raw_name, tool_name_map);
                        let input = b.get("input").cloned().unwrap_or(json!({}));
                        tool_calls.push(json!({
                            "id": id,
                            "type": "function",
                            "function": {"name": name, "arguments": input.to_string()}
                        }));
                    }
                    _ => {}
                }
            }
        }
        let finish = match v.get("stop_reason").and_then(Value::as_str) {
            Some("tool_use") => "tool_calls",
            Some("max_tokens") => "length",
            _ => "stop",
        };
        let (pt, ct, tt, cr, _) = extract_usage_tokens(&v);
        let mut msg = json!({"role": "assistant", "content": text});
        if !reasoning.is_empty() {
            msg["reasoning_content"] = json!(reasoning);
        }
        if !tool_calls.is_empty() {
            msg["tool_calls"] = Value::Array(tool_calls);
        }
        return json!({
            "id": v.get("id").cloned().unwrap_or(json!(format!("chatcmpl-{}", short_id()))),
            "object": "chat.completion",
            "created": now_unix_secs(),
            "model": model,
            "choices": [{"index": 0, "message": msg, "finish_reason": finish}],
            "usage": {
                "prompt_tokens": pt,
                "completion_tokens": ct,
                "total_tokens": tt,
                "prompt_tokens_details": {"cached_tokens": cr}
            }
        });
    }

    v
}

/// Convert OpenAI ChatCompletion JSON (or passthrough Anthropic Message) -> Anthropic Messages JSON.
pub fn openai_to_claude_response(v: &Value, model: &str) -> Value {
    if v.get("type").and_then(Value::as_str) == Some("message") && v.get("content").is_some() {
        return v.clone();
    }
    let mut blocks: Vec<Value> = Vec::new();
    let choice = v.pointer("/choices/0");
    if let Some(msg) = choice.and_then(|c| c.get("message")) {
        if let Some(rc) = msg.get("reasoning_content").and_then(Value::as_str) {
            if !rc.is_empty() {
                blocks.push(json!({"type": "thinking", "thinking": rc}));
            }
        }
        if let Some(text) = msg.get("content").and_then(Value::as_str) {
            if !text.is_empty() {
                blocks.push(json!({"type": "text", "text": text}));
            }
        }
        if let Some(tcs) = msg.get("tool_calls").and_then(Value::as_array) {
            for tc in tcs {
                let id = tc.get("id").and_then(Value::as_str).unwrap_or("");
                let name = tc.pointer("/function/name").and_then(Value::as_str).unwrap_or("");
                let input = as_arg_object(tc);
                blocks.push(json!({
                    "type": "tool_use",
                    "id": id,
                    "name": name,
                    "input": input
                }));
            }
        }
    }
    let stop_reason = match choice.and_then(|c| c.get("finish_reason")).and_then(Value::as_str) {
        Some("tool_calls") => "tool_use",
        Some("length") => "max_tokens",
        _ => "end_turn",
    };
    let (pt, ct, _, cr, cw) = extract_usage_tokens(v);
    json!({
        "id": format!("msg_{}", short_id()),
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": blocks,
        "stop_reason": stop_reason,
        "stop_sequence": Value::Null,
        "usage": {
            "input_tokens": pt,
            "output_tokens": ct,
            "cache_read_input_tokens": cr,
            "cache_creation_input_tokens": cw
        }
    })
}

/// Stateful converter: OpenAI ChatCompletionChunk -> Anthropic Messages SSE event frames.
#[derive(Debug, Default)]
pub struct OpenAiToClaudeStreamState {
    pub model: String,
    pub msg_id: String,
    pub started: bool,
    pub active_block_index: Option<usize>,
    pub active_block_kind: Option<String>,
    pub next_index: usize,
    pub output_tokens: i64,
    pub stop_reason: String,
}

impl OpenAiToClaudeStreamState {
    pub fn new(model: &str) -> Self {
        Self {
            model: model.to_string(),
            msg_id: format!("msg_{}", short_id()),
            started: false,
            active_block_index: None,
            active_block_kind: None,
            next_index: 0,
            output_tokens: 0,
            stop_reason: "end_turn".into(),
        }
    }

    fn format_sse(event: &str, data: &Value) -> String {
        format!("event: {event}\ndata: {}\n\n", data)
    }

    fn ensure_started(&mut self, out: &mut Vec<String>) {
        if !self.started {
            self.started = true;
            out.push(Self::format_sse(
                "message_start",
                &json!({
                    "type": "message_start",
                    "message": {
                        "id": self.msg_id,
                        "type": "message",
                        "role": "assistant",
                        "model": self.model,
                        "content": [],
                        "stop_reason": Value::Null,
                        "stop_sequence": Value::Null,
                        "usage": {"input_tokens": 0, "output_tokens": 0}
                    }
                }),
            ));
        }
    }

    fn close_active_block(&mut self, out: &mut Vec<String>) {
        if let Some(idx) = self.active_block_index.take() {
            self.active_block_kind = None;
            out.push(Self::format_sse(
                "content_block_stop",
                &json!({"type": "content_block_stop", "index": idx}),
            ));
        }
    }

    fn ensure_block(&mut self, kind: &str, content_block: Value, out: &mut Vec<String>) -> usize {
        if self.active_block_kind.as_deref() == Some(kind) {
            if let Some(idx) = self.active_block_index {
                return idx;
            }
        }
        self.close_active_block(out);
        let idx = self.next_index;
        self.next_index += 1;
        self.active_block_index = Some(idx);
        self.active_block_kind = Some(kind.to_string());
        out.push(Self::format_sse(
            "content_block_start",
            &json!({"type": "content_block_start", "index": idx, "content_block": content_block}),
        ));
        idx
    }

    pub fn push_openai_chunk(&mut self, chunk: &Value) -> Vec<String> {
        let mut out = Vec::new();
        self.ensure_started(&mut out);

        if let Some(u) = chunk.get("usage") {
            if let Some(ct) = u.get("completion_tokens").and_then(Value::as_i64) {
                self.output_tokens = ct;
            }
        }

        if let Some(choice) = chunk.pointer("/choices/0") {
            if let Some(delta) = choice.get("delta") {
                if let Some(rc) = delta.get("reasoning_content").and_then(Value::as_str) {
                    if !rc.is_empty() {
                        let idx = self.ensure_block(
                            "thinking",
                            json!({"type": "thinking", "thinking": ""}),
                            &mut out,
                        );
                        out.push(Self::format_sse(
                            "content_block_delta",
                            &json!({"type": "content_block_delta", "index": idx, "delta": {"type": "thinking_delta", "thinking": rc}}),
                        ));
                    }
                }
                if let Some(text) = delta.get("content").and_then(Value::as_str) {
                    if !text.is_empty() {
                        let idx = self.ensure_block(
                            "text",
                            json!({"type": "text", "text": ""}),
                            &mut out,
                        );
                        out.push(Self::format_sse(
                            "content_block_delta",
                            &json!({"type": "content_block_delta", "index": idx, "delta": {"type": "text_delta", "text": text}}),
                        ));
                    }
                }
                if let Some(tcs) = delta.get("tool_calls").and_then(Value::as_array) {
                    for tc in tcs {
                        let id = tc.get("id").and_then(Value::as_str);
                        let name = tc.pointer("/function/name").and_then(Value::as_str);
                        if let (Some(id), Some(name)) = (id, name) {
                            self.close_active_block(&mut out);
                            let idx = self.next_index;
                            self.next_index += 1;
                            self.active_block_index = Some(idx);
                            self.active_block_kind = Some("tool_use".into());
                            out.push(Self::format_sse(
                                "content_block_start",
                                &json!({
                                    "type": "content_block_start",
                                    "index": idx,
                                    "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}}
                                }),
                            ));
                        }
                        if let Some(args) = tc.pointer("/function/arguments").and_then(Value::as_str) {
                            if !args.is_empty() {
                                if let Some(idx) = self.active_block_index {
                                    out.push(Self::format_sse(
                                        "content_block_delta",
                                        &json!({
                                            "type": "content_block_delta",
                                            "index": idx,
                                            "delta": {"type": "input_json_delta", "partial_json": args}
                                        }),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            if let Some(fr) = choice.get("finish_reason").and_then(Value::as_str) {
                self.stop_reason = match fr {
                    "tool_calls" => "tool_use".into(),
                    "length" => "max_tokens".into(),
                    _ => "end_turn".into(),
                };
            }
        }
        out
    }

    pub fn finish(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        self.ensure_started(&mut out);
        self.close_active_block(&mut out);
        out.push(Self::format_sse(
            "message_delta",
            &json!({
                "type": "message_delta",
                "delta": {"stop_reason": self.stop_reason, "stop_sequence": Value::Null},
                "usage": {"output_tokens": self.output_tokens}
            }),
        ));
        out.push(Self::format_sse(
            "message_stop",
            &json!({"type": "message_stop"}),
        ));
        out
    }
}

/// Convert one upstream SSE JSON event (Claude / Gemini / OpenAI) into an OpenAI chunk.
pub fn normalize_stream_chunk_to_openai(
    provider: &str,
    model: &str,
    event: &str,
    v: Option<Value>,
    tool_name_map: &[(String, String)],
) -> Option<Value> {
    let v = v?;
    // Claude Messages SSE
    if provider == "anthropic"
        || provider == "claude"
        || event.starts_with("message_")
        || event.starts_with("content_block_")
    {
        if event == "content_block_start" {
            let cb = v.get("content_block")?;
            if cb.get("type").and_then(Value::as_str) == Some("tool_use") {
                let idx = v.get("index").and_then(Value::as_u64).unwrap_or(0);
                let id = cb.get("id").and_then(Value::as_str).unwrap_or("");
                let raw_name = cb.get("name").and_then(Value::as_str).unwrap_or("");
                let name = unprefix_tool_name(raw_name, tool_name_map);
                return Some(json!({
                    "id": format!("chatcmpl-{}", short_id()),
                    "object": "chat.completion.chunk",
                    "created": now_unix_secs(),
                    "model": model,
                    "choices": [{
                        "index": 0,
                        "delta": {
                            "tool_calls": [{
                                "index": idx,
                                "id": id,
                                "type": "function",
                                "function": {"name": name, "arguments": ""}
                            }]
                        },
                        "finish_reason": Value::Null
                    }]
                }));
            }
            return None;
        }
        if event == "content_block_delta" {
            let delta = v.get("delta")?;
            let dt = delta.get("type").and_then(Value::as_str).unwrap_or("");
            let idx = v.get("index").and_then(Value::as_u64).unwrap_or(0);
            let delta_obj = match dt {
                "text_delta" => {
                    let t = delta.get("text").and_then(Value::as_str).unwrap_or("");
                    if t.is_empty() { return None; }
                    json!({"content": t})
                }
                "thinking_delta" => {
                    let t = delta.get("thinking").and_then(Value::as_str).unwrap_or("");
                    if t.is_empty() { return None; }
                    json!({"reasoning_content": t})
                }
                "input_json_delta" => {
                    let p = delta.get("partial_json").and_then(Value::as_str).unwrap_or("");
                    if p.is_empty() { return None; }
                    json!({"tool_calls": [{"index": idx, "function": {"arguments": p}}]})
                }
                _ => return None,
            };
            return Some(json!({
                "id": format!("chatcmpl-{}", short_id()),
                "object": "chat.completion.chunk",
                "created": now_unix_secs(),
                "model": model,
                "choices": [{"index": 0, "delta": delta_obj, "finish_reason": Value::Null}]
            }));
        }
        if event == "message_delta" {
            let finish = match v.pointer("/delta/stop_reason").and_then(Value::as_str) {
                Some("tool_use") => "tool_calls",
                Some("max_tokens") => "length",
                Some(_) => "stop",
                None => return None,
            };
            let out_tok = v.pointer("/usage/output_tokens").and_then(Value::as_i64).unwrap_or(0);
            return Some(json!({
                "id": format!("chatcmpl-{}", short_id()),
                "object": "chat.completion.chunk",
                "created": now_unix_secs(),
                "model": model,
                "choices": [{"index": 0, "delta": {}, "finish_reason": finish}],
                "usage": {"prompt_tokens": 0, "completion_tokens": out_tok, "total_tokens": out_tok}
            }));
        }
        if event == "error" {
            return Some(json!({"error": v.get("error").cloned().unwrap_or(v)}));
        }
        return None;
    }

    // Gemini SSE
    if v.get("candidates").is_some() || v.get("usageMetadata").is_some() {
        let c0 = v.pointer("/candidates/0")?;
        let mut delta = json!({});
        let mut tool_calls: Vec<Value> = Vec::new();
        if let Some(parts) = c0.pointer("/content/parts").and_then(Value::as_array) {
            let mut text = String::new();
            for (idx, part) in parts.iter().enumerate() {
                if let Some(t) = part.get("text").and_then(Value::as_str) {
                    text.push_str(t);
                }
                if let Some(fc) = part.get("functionCall").or_else(|| part.get("function_call")) {
                    let name = fc.get("name").and_then(Value::as_str).unwrap_or("");
                    let args = fc.get("args").cloned().unwrap_or(json!({}));
                    tool_calls.push(json!({
                        "index": idx,
                        "id": format!("call_{}_{idx}", short_id()),
                        "type": "function",
                        "function": {"name": name, "arguments": args.to_string()}
                    }));
                }
            }
            if !text.is_empty() {
                delta["content"] = json!(text);
            }
            if !tool_calls.is_empty() {
                delta["tool_calls"] = Value::Array(tool_calls);
            }
        }
        let finish_reason = c0.get("finishReason").and_then(Value::as_str).map(|fr| {
            if delta.get("tool_calls").is_some() {
                json!("tool_calls")
            } else if fr == "STOP" {
                json!("stop")
            } else if fr == "MAX_TOKENS" {
                json!("length")
            } else {
                json!(fr.to_ascii_lowercase())
            }
        });
        let (pt, ct, tt, _, _) = extract_usage_tokens(&v);
        let mut chunk = json!({
            "id": format!("chatcmpl-{}", short_id()),
            "object": "chat.completion.chunk",
            "created": now_unix_secs(),
            "model": model,
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish_reason.unwrap_or(Value::Null)}]
        });
        if tt > 0 {
            chunk["usage"] = json!({"prompt_tokens": pt, "completion_tokens": ct, "total_tokens": tt});
        }
        return Some(chunk);
    }

    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(model: &str, msgs: Value) -> ChatRequest {
        let mut r: ChatRequest = serde_json::from_value(json!({
            "model": model,
            "messages": msgs,
        }))
        .unwrap();
        r.messages = serde_json::from_value(msgs).unwrap();
        r
    }

    #[test]
    fn system_extracted_separate() {
        let body = req(
            "claude-3-5-sonnet-20241022",
            json!([
                {"role": "system", "content": "be terse"},
                {"role": "user", "content": "hi"}
            ]),
        );
        let out = openai_to_claude(&body, false, false);
        let sys = out.get("system").unwrap().as_array().unwrap();
        assert_eq!(sys[0]["text"], "be terse");
        let msgs = out["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[0]["content"][0]["text"], "hi");
    }

    #[test]
    fn tool_call_input_is_json_object_and_no_leak() {
        let mut body = req(
            "claude-3-5-sonnet-20241022",
            json!([
                {"role": "user", "content": "list files"},
                {"role": "assistant", "content": null, "tool_calls": [
                    {"id": "call_1", "type": "function", "function": {"name": "ls", "arguments": "{\"path\":\".\"}"}}
                ]},
                {"role": "tool", "tool_call_id": "call_1", "content": "a.txt"}
            ]),
        );
        body.tools = Some(vec![Tool {
            kind: "function".into(),
            function: FunctionDef {
                name: "ls".into(),
                description: Some("list".into()),
                parameters: Some(json!({"type": "object"})),
            },
        }]);
        // OAuth mode: prefixes both tools[] and tool_use, no _toolNameMap leak
        let (out, map) = openai_to_claude_with_map(&body, false, true, &[]);
        assert!(out.get("_toolNameMap").is_none());
        assert_eq!(map.len(), 1);
        assert_eq!(out["tools"][0]["name"], "proxy_ls");
        let msgs = out["messages"].as_array().unwrap();
        let tu = &msgs[1]["content"][0];
        assert_eq!(tu["type"], "tool_use");
        assert_eq!(tu["name"], "proxy_ls");
        assert!(tu["input"].is_object(), "input must be JSON object, got {}", tu["input"]);
        assert_eq!(tu["input"]["path"], ".");

        // API key mode (has_oauth=false): no proxy_ prefix
        let (out_key, map_key) = openai_to_claude_with_map(&body, false, false, &[]);
        assert!(map_key.is_empty());
        assert_eq!(out_key["tools"][0]["name"], "ls");
        assert_eq!(out_key["messages"][1]["content"][0]["name"], "ls");
    }

    #[test]
    fn claude_to_openai_and_back_roundtrip() {
        let claude_in = json!({
            "model": "claude-opus-4.7",
            "max_tokens": 1024,
            "system": [{"type": "text", "text": "You are helpful"}],
            "messages": [
                {"role": "user", "content": [{"type": "text", "text": "hello"}]}
            ],
            "tools": [{"name": "get_weather", "description": "weather", "input_schema": {"type": "object"}}],
            "tool_choice": {"type": "any"}
        });
        let oai_req = claude_to_openai_request(&claude_in);
        assert_eq!(oai_req.model, "claude-opus-4.7");
        assert_eq!(oai_req.messages.len(), 2);
        assert_eq!(oai_req.messages[0].role, "system");
        assert_eq!(oai_req.tool_choice, Some(json!("required")));
        assert_eq!(oai_req.tools.as_ref().unwrap()[0].function.name, "get_weather");

        let oai_resp = json!({
            "id": "chatcmpl-1",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "Hi!"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        });
        let claude_resp = openai_to_claude_response(&oai_resp, "claude-opus-4.7");
        assert_eq!(claude_resp["type"], "message");
        assert_eq!(claude_resp["content"][0]["text"], "Hi!");
        assert_eq!(claude_resp["stop_reason"], "end_turn");
        assert_eq!(claude_resp["usage"]["input_tokens"], 10);
    }

    #[test]
    fn openai_to_claude_sse_stream_frames() {
        let mut st = OpenAiToClaudeStreamState::new("claude-sonnet-4.6");
        let c1 = json!({"choices":[{"index":0,"delta":{"content":"Hel"},"finish_reason":null}]});
        let c2 = json!({"choices":[{"index":0,"delta":{"content":"lo"},"finish_reason":"stop"}]});
        let f1 = st.push_openai_chunk(&c1);
        let f2 = st.push_openai_chunk(&c2);
        let end = st.finish();
        let all = [f1, f2, end].concat().join("");
        assert!(all.contains("event: message_start"));
        assert!(all.contains("event: content_block_start"));
        assert!(all.contains("text_delta"));
        assert!(all.contains("event: message_stop"));
    }

    #[test]
    fn adaptive_model_gets_output_config() {
        let mut body = req("claude-opus-4.7", json!([{"role": "user", "content": "x"}]));
        body.reasoning_effort = Some("max".into());
        let out = openai_to_claude(&body, false, false);
        assert_eq!(out["thinking"]["type"], "adaptive");
        assert_eq!(out["output_config"]["effort"], "max");
    }

    #[test]
    fn gemini_roundtrip() {
        let body = req(
            "gemini-3.8-flash",
            json!([
                {"role": "system", "content": "s"},
                {"role": "user", "content": "hello"}
            ]),
        );
        let out = openai_to_gemini(&body, false);
        assert_eq!(out["contents"][0]["role"], "user");
        assert_eq!(out["contents"][0]["parts"][0]["text"], "hello");
        assert_eq!(out["system_instruction"]["parts"][0]["text"], "s");
    }
}
