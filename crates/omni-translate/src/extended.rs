// Extended protocol translators for Phase 3:
//   - OpenAI Responses API (/v1/responses): request + response + SSE
//   - Native Gemini API (/v1beta/models/{model}:generateContent): request + response + SSE
//   - Legacy OpenAI Completions (/v1/completions)

use crate::{extract_usage_tokens, short_id};
use omni_types::{ChatMessage, ChatRequest, FunctionDef, Tool, ToolCall, ToolCallFn};
use serde_json::{json, Value};

// ── OpenAI Responses API <-> Chat Completions ───────────────────────

pub fn responses_to_chat_request(body: &Value) -> ChatRequest {
    let model = body.get("model").and_then(Value::as_str).unwrap_or("").to_string();
    let stream = body.get("stream").and_then(Value::as_bool);
    let max_tokens = body
        .get("max_output_tokens")
        .or_else(|| body.get("max_tokens"))
        .and_then(Value::as_u64);
    let temperature = body.get("temperature").and_then(Value::as_f64);
    let top_p = body.get("top_p").and_then(Value::as_f64);
    let reasoning_effort = body
        .pointer("/reasoning/effort")
        .and_then(Value::as_str)
        .map(String::from);

    let mut messages: Vec<ChatMessage> = Vec::new();
    if let Some(instr) = body.get("instructions").and_then(Value::as_str) {
        if !instr.is_empty() {
            messages.push(ChatMessage {
                role: "system".into(),
                content: Some(Value::String(instr.to_string())),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                extra: json!({}),
            });
        }
    }

    match body.get("input") {
        Some(Value::String(s)) => {
            messages.push(ChatMessage {
                role: "user".into(),
                content: Some(Value::String(s.clone())),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                extra: json!({}),
            });
        }
        Some(Value::Array(items)) => {
            for item in items {
                let itype = item.get("type").and_then(Value::as_str).unwrap_or("message");
                match itype {
                    "function_call" => {
                        let id = item
                            .get("call_id")
                            .or_else(|| item.get("id"))
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let name = item.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                        let args = match item.get("arguments") {
                            Some(Value::String(s)) => s.clone(),
                            Some(other) => other.to_string(),
                            None => "{}".into(),
                        };
                        messages.push(ChatMessage {
                            role: "assistant".into(),
                            content: None,
                            name: None,
                            tool_calls: Some(vec![ToolCall {
                                id,
                                kind: "function".into(),
                                function: ToolCallFn { name, arguments: args },
                            }]),
                            tool_call_id: None,
                            extra: json!({}),
                        });
                    }
                    "function_call_output" | "custom_tool_call_output" => {
                        let call_id = item
                            .get("call_id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let out = match item.get("output") {
                            Some(Value::String(s)) => s.clone(),
                            Some(Value::Array(parts)) => parts
                                .iter()
                                .filter_map(|p| p.get("text").and_then(Value::as_str))
                                .collect::<Vec<_>>()
                                .join("\n"),
                            Some(other) => other.to_string(),
                            None => String::new(),
                        };
                        messages.push(ChatMessage {
                            role: "tool".into(),
                            content: Some(Value::String(out)),
                            name: None,
                            tool_calls: None,
                            tool_call_id: Some(call_id),
                            extra: json!({}),
                        });
                    }
                    _ => {
                        let role = item.get("role").and_then(Value::as_str).unwrap_or("user");
                        let content = match item.get("content") {
                            Some(Value::String(s)) => Some(Value::String(s.clone())),
                            Some(Value::Array(parts)) => {
                                let converted: Vec<Value> = parts
                                    .iter()
                                    .filter_map(|p| {
                                        let pt = p.get("type").and_then(Value::as_str).unwrap_or("input_text");
                                        match pt {
                                            "input_text" | "output_text" | "text" => {
                                                let t = p.get("text").and_then(Value::as_str)?;
                                                Some(json!({"type": "text", "text": t}))
                                            }
                                            "input_image" | "image_url" => {
                                                let url = p
                                                    .get("image_url")
                                                    .and_then(|v| v.as_str().or_else(|| v.get("url").and_then(Value::as_str)))?;
                                                Some(json!({"type": "image_url", "image_url": {"url": url}}))
                                            }
                                            _ => None,
                                        }
                                    })
                                    .collect();
                                if converted.len() == 1
                                    && converted[0].get("type").and_then(Value::as_str) == Some("text")
                                {
                                    Some(converted[0]["text"].clone())
                                } else if converted.is_empty() {
                                    None
                                } else {
                                    Some(Value::Array(converted))
                                }
                            }
                            _ => None,
                        };
                        messages.push(ChatMessage {
                            role: role.to_string(),
                            content,
                            name: None,
                            tool_calls: None,
                            tool_call_id: None,
                            extra: json!({}),
                        });
                    }
                }
            }
        }
        _ => {}
    }

    let tools = body.get("tools").and_then(Value::as_array).map(|arr| {
        arr.iter()
            .filter_map(|t| {
                if let Some(f) = t.get("function") {
                    let name = f.get("name").and_then(Value::as_str)?.to_string();
                    return Some(Tool {
                        kind: "function".into(),
                        function: FunctionDef {
                            name,
                            description: f.get("description").and_then(Value::as_str).map(String::from),
                            parameters: f.get("parameters").cloned(),
                        },
                    });
                }
                if t.get("type").and_then(Value::as_str) == Some("function") {
                    let name = t.get("name").and_then(Value::as_str)?.to_string();
                    return Some(Tool {
                        kind: "function".into(),
                        function: FunctionDef {
                            name,
                            description: t.get("description").and_then(Value::as_str).map(String::from),
                            parameters: t.get("parameters").cloned(),
                        },
                    });
                }
                None
            })
            .collect::<Vec<_>>()
    });

    ChatRequest {
        model,
        messages,
        stream,
        temperature,
        top_p,
        max_tokens,
        max_completion_tokens: None,
        stop: None,
        tools: tools.filter(|v| !v.is_empty()),
        tool_choice: body.get("tool_choice").cloned(),
        reasoning_effort,
        response_format: None,
        user: body.get("user").and_then(Value::as_str).map(String::from),
        extra: json!({}),
    }
}

/// Convert ChatRequest -> native OpenAI Responses request payload (for `chatgpt`/`codex` upstreams).
pub fn chat_to_responses_request(req: &ChatRequest, stream: bool) -> Value {
    let mut instructions: Vec<String> = Vec::new();
    let mut input: Vec<Value> = Vec::new();

    for m in &req.messages {
        if m.role == "system" || m.role == "developer" {
            if let Some(Value::String(s)) = &m.content {
                instructions.push(s.clone());
            }
            continue;
        }
        if m.role == "tool" {
            input.push(json!({
                "type": "function_call_output",
                "call_id": m.tool_call_id.clone().unwrap_or_default(),
                "output": match &m.content {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => other.to_string(),
                    None => String::new(),
                }
            }));
            continue;
        }
        if let Some(tcs) = &m.tool_calls {
            for tc in tcs {
                input.push(json!({
                    "type": "function_call",
                    "call_id": tc.id,
                    "name": tc.function.name,
                    "arguments": tc.function.arguments
                }));
            }
        }
        if let Some(c) = &m.content {
            let text_type = if m.role == "assistant" { "output_text" } else { "input_text" };
            let content = match c {
                Value::String(s) => json!([{"type": text_type, "text": s}]),
                other => other.clone(),
            };
            input.push(json!({
                "type": "message",
                "role": m.role,
                "content": content
            }));
        }
    }

    let mut out = json!({
        "model": req.model,
        "input": input,
        "stream": stream,
    });
    if !instructions.is_empty() {
        out["instructions"] = json!(instructions.join("\n"));
    }
    if let Some(mt) = req.max_tokens.or(req.max_completion_tokens) {
        out["max_output_tokens"] = json!(mt);
    }
    if let Some(eff) = &req.reasoning_effort {
        out["reasoning"] = json!({"effort": eff});
    }
    if let Some(tools) = &req.tools {
        out["tools"] = Value::Array(
            tools
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "name": t.function.name,
                        "description": t.function.description,
                        "parameters": t.function.parameters.clone().unwrap_or(json!({"type": "object"}))
                    })
                })
                .collect(),
        );
    }
    out
}

/// Convert OpenAI ChatCompletion JSON -> OpenAI Responses JSON.
pub fn openai_to_responses_response(v: &Value, model: &str) -> Value {
    if v.get("object").and_then(Value::as_str) == Some("response") {
        return v.clone();
    }
    let resp_id = format!("resp_{}", short_id());
    let mut output: Vec<Value> = Vec::new();
    let mut output_text = String::new();

    if let Some(msg) = v.pointer("/choices/0/message") {
        if let Some(rc) = msg.get("reasoning_content").and_then(Value::as_str) {
            if !rc.is_empty() {
                output.push(json!({
                    "id": format!("rs_{}", short_id()),
                    "type": "reasoning",
                    "summary": [{"type": "summary_text", "text": rc}]
                }));
            }
        }
        if let Some(text) = msg.get("content").and_then(Value::as_str) {
            if !text.is_empty() {
                output_text = text.to_string();
                output.push(json!({
                    "id": format!("msg_{}", short_id()),
                    "type": "message",
                    "status": "completed",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": text, "annotations": []}]
                }));
            }
        }
        if let Some(tcs) = msg.get("tool_calls").and_then(Value::as_array) {
            for tc in tcs {
                output.push(json!({
                    "id": format!("fc_{}", short_id()),
                    "type": "function_call",
                    "status": "completed",
                    "call_id": tc.get("id").and_then(Value::as_str).unwrap_or(""),
                    "name": tc.pointer("/function/name").and_then(Value::as_str).unwrap_or(""),
                    "arguments": tc.pointer("/function/arguments").and_then(Value::as_str).unwrap_or("{}")
                }));
            }
        }
    }

    let (pt, ct, tt, cr, _) = extract_usage_tokens(v);
    json!({
        "id": resp_id,
        "object": "response",
        "status": "completed",
        "model": model,
        "output": output,
        "output_text": output_text,
        "usage": {
            "input_tokens": pt,
            "output_tokens": ct,
            "total_tokens": tt,
            "input_tokens_details": {"cached_tokens": cr}
        }
    })
}

/// Stateful converter: OpenAI Chat SSE chunks -> OpenAI Responses SSE event frames.
pub struct OpenAiToResponsesStreamState {
    pub resp_id: String,
    pub msg_id: String,
    pub model: String,
    pub started: bool,
    pub text_acc: String,
    pub output_tokens: i64,
}

impl OpenAiToResponsesStreamState {
    pub fn new(model: &str) -> Self {
        Self {
            resp_id: format!("resp_{}", short_id()),
            msg_id: format!("msg_{}", short_id()),
            model: model.to_string(),
            started: false,
            text_acc: String::new(),
            output_tokens: 0,
        }
    }

    fn frame(event: &str, data: &Value) -> String {
        format!("event: {event}\ndata: {}\n\n", data)
    }

    fn ensure_started(&mut self, out: &mut Vec<String>) {
        if !self.started {
            self.started = true;
            out.push(Self::frame(
                "response.created",
                &json!({
                    "type": "response.created",
                    "response": {
                        "id": self.resp_id,
                        "object": "response",
                        "status": "in_progress",
                        "model": self.model,
                        "output": []
                    }
                }),
            ));
            out.push(Self::frame(
                "response.output_item.added",
                &json!({
                    "type": "response.output_item.added",
                    "output_index": 0,
                    "item": {
                        "id": self.msg_id,
                        "type": "message",
                        "status": "in_progress",
                        "role": "assistant",
                        "content": []
                    }
                }),
            ));
        }
    }

    pub fn push_openai_chunk(&mut self, chunk: &Value) -> Vec<String> {
        let mut out = Vec::new();
        self.ensure_started(&mut out);

        if let Some(u) = chunk.get("usage") {
            if let Some(ct) = u.get("completion_tokens").and_then(Value::as_i64) {
                self.output_tokens = ct;
            }
        }
        if let Some(delta) = chunk.pointer("/choices/0/delta") {
            if let Some(text) = delta.get("content").and_then(Value::as_str) {
                if !text.is_empty() {
                    self.text_acc.push_str(text);
                    out.push(Self::frame(
                        "response.output_text.delta",
                        &json!({
                            "type": "response.output_text.delta",
                            "item_id": self.msg_id,
                            "output_index": 0,
                            "content_index": 0,
                            "delta": text
                        }),
                    ));
                }
            }
        }
        out
    }

    pub fn finish(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        self.ensure_started(&mut out);
        out.push(Self::frame(
            "response.output_text.done",
            &json!({
                "type": "response.output_text.done",
                "item_id": self.msg_id,
                "output_index": 0,
                "content_index": 0,
                "text": self.text_acc
            }),
        ));
        out.push(Self::frame(
            "response.completed",
            &json!({
                "type": "response.completed",
                "response": {
                    "id": self.resp_id,
                    "object": "response",
                    "status": "completed",
                    "model": self.model,
                    "output_text": self.text_acc,
                    "output": [{
                        "id": self.msg_id,
                        "type": "message",
                        "status": "completed",
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": self.text_acc, "annotations": []}]
                    }],
                    "usage": {"input_tokens": 0, "output_tokens": self.output_tokens, "total_tokens": self.output_tokens}
                }
            }),
        ));
        out
    }
}

// ── Native Gemini API (/v1beta/models/{model}:generateContent) ──────

pub fn gemini_to_openai_request(model: &str, body: &Value, stream: bool) -> ChatRequest {
    let mut messages: Vec<ChatMessage> = Vec::new();

    if let Some(sys_parts) = body
        .pointer("/systemInstruction/parts")
        .or_else(|| body.pointer("/system_instruction/parts"))
        .and_then(Value::as_array)
    {
        let sys_text: String = sys_parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
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

    let mut call_seq: u64 = 0;
    if let Some(contents) = body.get("contents").and_then(Value::as_array) {
        for c in contents {
            let role = match c.get("role").and_then(Value::as_str).unwrap_or("user") {
                "model" => "assistant",
                other => other,
            };
            let Some(parts) = c.get("parts").and_then(Value::as_array) else {
                continue;
            };
            let mut text_parts: Vec<Value> = Vec::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();
            let mut tool_responses: Vec<ChatMessage> = Vec::new();

            for p in parts {
                if let Some(t) = p.get("text").and_then(Value::as_str) {
                    text_parts.push(json!({"type": "text", "text": t}));
                }
                if let Some(inline) = p.get("inlineData").or_else(|| p.get("inline_data")) {
                    if let Some(data) = inline.get("data").and_then(Value::as_str) {
                        let mime = inline
                            .get("mimeType")
                            .or_else(|| inline.get("mime_type"))
                            .and_then(Value::as_str)
                            .unwrap_or("image/png");
                        text_parts.push(json!({
                            "type": "image_url",
                            "image_url": {"url": format!("data:{mime};base64,{data}")}
                        }));
                    }
                }
                if let Some(fc) = p.get("functionCall").or_else(|| p.get("function_call")) {
                    call_seq += 1;
                    let id = fc
                        .get("id")
                        .and_then(Value::as_str)
                        .map(String::from)
                        .unwrap_or_else(|| format!("call_gem_{call_seq}"));
                    let name = fc.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                    let args = fc.get("args").cloned().unwrap_or(json!({})).to_string();
                    tool_calls.push(ToolCall {
                        id,
                        kind: "function".into(),
                        function: ToolCallFn { name, arguments: args },
                    });
                }
                if let Some(fr) = p.get("functionResponse").or_else(|| p.get("function_response")) {
                    let id = fr
                        .get("id")
                        .and_then(Value::as_str)
                        .map(String::from)
                        .unwrap_or_else(|| format!("call_gem_{call_seq}"));
                    let name = fr.get("name").and_then(Value::as_str).map(String::from);
                    let resp = fr.get("response").cloned().unwrap_or(json!({}));
                    tool_responses.push(ChatMessage {
                        role: "tool".into(),
                        content: Some(Value::String(resp.to_string())),
                        name,
                        tool_calls: None,
                        tool_call_id: Some(id),
                        extra: json!({}),
                    });
                }
            }

            for tr in tool_responses {
                messages.push(tr);
            }
            if !text_parts.is_empty() || !tool_calls.is_empty() {
                let content = if text_parts.len() == 1
                    && text_parts[0].get("type").and_then(Value::as_str) == Some("text")
                {
                    Some(text_parts[0]["text"].clone())
                } else if text_parts.is_empty() {
                    None
                } else {
                    Some(Value::Array(text_parts))
                };
                messages.push(ChatMessage {
                    role: role.to_string(),
                    content,
                    name: None,
                    tool_calls: if tool_calls.is_empty() { None } else { Some(tool_calls) },
                    tool_call_id: None,
                    extra: json!({}),
                });
            }
        }
    }

    let mut tools_out: Vec<Tool> = Vec::new();
    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        for t in tools {
            if let Some(decls) = t
                .get("functionDeclarations")
                .or_else(|| t.get("function_declarations"))
                .and_then(Value::as_array)
            {
                for d in decls {
                    if let Some(name) = d.get("name").and_then(Value::as_str) {
                        tools_out.push(Tool {
                            kind: "function".into(),
                            function: FunctionDef {
                                name: name.to_string(),
                                description: d.get("description").and_then(Value::as_str).map(String::from),
                                parameters: d
                                    .get("parameters")
                                    .or_else(|| d.get("parametersJsonSchema"))
                                    .cloned(),
                            },
                        });
                    }
                }
            }
        }
    }

    let gen = body.get("generationConfig").or_else(|| body.get("generation_config"));
    ChatRequest {
        model: model.to_string(),
        messages,
        stream: Some(stream),
        temperature: gen.and_then(|g| g.get("temperature")).and_then(Value::as_f64),
        top_p: gen.and_then(|g| g.get("topP")).and_then(Value::as_f64),
        max_tokens: gen.and_then(|g| g.get("maxOutputTokens")).and_then(Value::as_u64),
        max_completion_tokens: None,
        stop: gen.and_then(|g| g.get("stopSequences")).cloned(),
        tools: if tools_out.is_empty() { None } else { Some(tools_out) },
        tool_choice: None,
        reasoning_effort: None,
        response_format: None,
        user: None,
        extra: json!({}),
    }
}

pub fn openai_to_gemini_response(v: &Value, model: &str) -> Value {
    if v.get("candidates").is_some() {
        return v.clone();
    }
    let mut parts: Vec<Value> = Vec::new();
    let choice = v.pointer("/choices/0");
    if let Some(msg) = choice.and_then(|c| c.get("message")) {
        if let Some(text) = msg.get("content").and_then(Value::as_str) {
            if !text.is_empty() {
                parts.push(json!({"text": text}));
            }
        }
        if let Some(tcs) = msg.get("tool_calls").and_then(Value::as_array) {
            for tc in tcs {
                let name = tc.pointer("/function/name").and_then(Value::as_str).unwrap_or("");
                let args: Value = tc
                    .pointer("/function/arguments")
                    .and_then(Value::as_str)
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(json!({}));
                parts.push(json!({"functionCall": {"name": name, "args": args}}));
            }
        }
    }
    let finish = match choice.and_then(|c| c.get("finish_reason")).and_then(Value::as_str) {
        Some("length") => "MAX_TOKENS",
        _ => "STOP",
    };
    let (pt, ct, tt, cr, _) = extract_usage_tokens(v);
    json!({
        "candidates": [{
            "index": 0,
            "content": {"role": "model", "parts": parts},
            "finishReason": finish
        }],
        "usageMetadata": {
            "promptTokenCount": pt,
            "candidatesTokenCount": ct,
            "totalTokenCount": tt,
            "cachedContentTokenCount": cr
        },
        "modelVersion": model
    })
}

pub fn openai_chunk_to_gemini_sse(chunk: &Value, model: &str) -> Option<String> {
    let choice = chunk.pointer("/choices/0")?;
    let mut parts: Vec<Value> = Vec::new();
    if let Some(text) = choice.pointer("/delta/content").and_then(Value::as_str) {
        if !text.is_empty() {
            parts.push(json!({"text": text}));
        }
    }
    let finish = choice.get("finish_reason").and_then(Value::as_str).map(|fr| {
        if fr == "length" { "MAX_TOKENS" } else { "STOP" }
    });
    if parts.is_empty() && finish.is_none() {
        return None;
    }
    let mut cand = json!({"index": 0, "content": {"role": "model", "parts": parts}});
    if let Some(fr) = finish {
        cand["finishReason"] = json!(fr);
    }
    let (pt, ct, tt, _, _) = extract_usage_tokens(chunk);
    let mut payload = json!({"candidates": [cand], "modelVersion": model});
    if tt > 0 {
        payload["usageMetadata"] = json!({
            "promptTokenCount": pt,
            "candidatesTokenCount": ct,
            "totalTokenCount": tt
        });
    }
    Some(format!("data: {payload}\n\n"))
}

// ── Legacy /v1/completions ──────────────────────────────────────────

pub fn legacy_completion_to_chat(body: &Value) -> ChatRequest {
    let model = body.get("model").and_then(Value::as_str).unwrap_or("").to_string();
    let prompt = match body.get("prompt") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    };
    ChatRequest {
        model,
        messages: vec![ChatMessage {
            role: "user".into(),
            content: Some(Value::String(prompt)),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            extra: json!({}),
        }],
        stream: body.get("stream").and_then(Value::as_bool),
        temperature: body.get("temperature").and_then(Value::as_f64),
        top_p: body.get("top_p").and_then(Value::as_f64),
        max_tokens: body.get("max_tokens").and_then(Value::as_u64),
        stop: body.get("stop").cloned(),
        ..Default::default()
    }
}

pub fn openai_chat_to_legacy_completion(v: &Value, model: &str) -> Value {
    let text = v
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .unwrap_or("");
    let finish = v
        .pointer("/choices/0/finish_reason")
        .and_then(Value::as_str)
        .unwrap_or("stop");
    let (pt, ct, tt, _, _) = extract_usage_tokens(v);
    json!({
        "id": format!("cmpl-{}", short_id()),
        "object": "text_completion",
        "model": model,
        "choices": [{"text": text, "index": 0, "logprobs": Value::Null, "finish_reason": finish}],
        "usage": {"prompt_tokens": pt, "completion_tokens": ct, "total_tokens": tt}
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responses_roundtrip_and_sse() {
        let req_body = json!({
            "model": "openai/gpt-6.1-sol",
            "instructions": "be concise",
            "input": "hello world",
            "max_output_tokens": 256
        });
        let chat = responses_to_chat_request(&req_body);
        assert_eq!(chat.messages.len(), 2);
        assert_eq!(chat.messages[0].role, "system");
        assert_eq!(chat.max_tokens, Some(256));

        let oai = json!({
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi!"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7}
        });
        let resp = openai_to_responses_response(&oai, "gpt-6.1-sol");
        assert_eq!(resp["object"], "response");
        assert_eq!(resp["output_text"], "hi!");
        assert_eq!(resp["usage"]["input_tokens"], 5);

        let mut st = OpenAiToResponsesStreamState::new("gpt-6.1-sol");
        let a = st.push_openai_chunk(&json!({"choices":[{"delta":{"content":"hi"}}]}));
        let b = st.finish();
        let all = [a, b].concat().join("");
        assert!(all.contains("event: response.created"));
        assert!(all.contains("event: response.output_text.delta"));
        assert!(all.contains("event: response.completed"));
    }

    #[test]
    fn gemini_native_roundtrip() {
        let gem_in = json!({
            "systemInstruction": {"parts": [{"text": "sys"}]},
            "contents": [{"role": "user", "parts": [{"text": "ping"}]}],
            "generationConfig": {"maxOutputTokens": 128}
        });
        let chat = gemini_to_openai_request("gemini-3.8-flash", &gem_in, false);
        assert_eq!(chat.messages.len(), 2);
        assert_eq!(chat.max_tokens, Some(128));

        let oai = json!({
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "pong"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 3, "completion_tokens": 1, "total_tokens": 4}
        });
        let gem_out = openai_to_gemini_response(&oai, "gemini-3.8-flash");
        assert_eq!(gem_out["candidates"][0]["content"]["parts"][0]["text"], "pong");
        assert_eq!(gem_out["usageMetadata"]["totalTokenCount"], 4);
    }
}
