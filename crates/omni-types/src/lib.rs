pub mod errors;
pub mod sse;
pub use errors::*;

// Core wire types for the OmniRoute Rust port.
// Mirrors OpenAI Chat Completions (client surface) + Claude Messages + Gemini
// generateContent. Serde-driven; unknown upstream fields are preserved via `extra`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── OpenAI Chat Completions (client surface) ────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    /// OpenAI o-series / reasoning models field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Passthrough: OmniRoute internal flags (_disableToolPrefix etc).
    #[serde(flatten)]
    pub extra: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    /// String or array of parts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Assistant tool_call trail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// tool role: id of the answered call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(flatten)]
    pub extra: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: FunctionDef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDef {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ToolCallFn,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFn {
    pub name: String,
    /// JSON-encoded arguments string.
    pub arguments: String,
}

/// Unified content part across providers (OpenAI parts normalized here).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContentPart {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<ImageUrl>,
    #[serde(flatten)]
    pub extra: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUrl {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

// ── Chat response (non-stream) ──────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<Choice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Choice {
    pub index: u32,
    pub message: ChoiceMessage,
    #[serde(rename = "finish_reason", default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChoiceMessage {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

// ── Chat chunk (stream) ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChunk {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<ChunkChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkChoice {
    pub index: u32,
    pub delta: Delta,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "finish_reason")]
    pub finish_reason: Option<Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Delta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: i64,
    #[serde(default)]
    pub completion_tokens: i64,
    #[serde(default)]
    pub total_tokens: i64,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "reasoning_tokens")]
    pub reasoning_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens_details: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens_details: Option<Value>,
}

// ── Claude Messages API ─────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct ClaudeRequest {
    pub model: String,
    pub max_tokens: u64,
    pub messages: Vec<Value>,
    pub system: Option<Vec<Value>>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub stop_sequences: Option<Vec<String>>,
    pub tools: Option<Vec<Value>>,
    pub tool_choice: Option<Value>,
    pub thinking: Option<Value>,
    pub output_config: Option<Value>,
    pub stream: bool,
}

#[derive(Debug, Clone)]
pub struct ClaudeEvent {
    /// message_start | content_block_start | content_block_delta | ... | message_stop
    pub kind: String,
    pub payload: Value,
}

// ── Gemini generateContent ──────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct GeminiRequest {
    pub contents: Vec<Value>,
    pub system_instruction: Option<Value>,
    pub generation_config: Option<Value>,
    pub tools: Option<Vec<Value>>,
    pub tool_config: Option<Value>,
}

// ── Tool name map (OAuth prefix round-trip) ─────────────────────────

pub const CLAUDE_OAUTH_TOOL_PREFIX: &str = "proxy_";

// ── Errors ──────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum OmniError {
    #[error("upstream http status {status}")]
    Upstream {
        status: u16,
        body: Option<String>,
    },
    #[error("network error: {0}")]
    Network(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("translation error: {0}")]
    Translate(String),
}

impl OmniError {
    pub fn status(&self) -> u16 {
        match self {
            OmniError::Upstream { status, .. } => *status,
            OmniError::BadRequest(_) => 400,
            OmniError::Translate(_) | OmniError::Network(_) => 502,
        }
    }
}

// ── Canonical error body (OpenAI-shape) ─────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ErrorBody {
    pub error: ErrorInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream_details: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorInfo {
    pub message: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorReason {
    AuthError,
    QuotaExhausted,
    RateLimitExceeded,
    ModelCapacity,
    ServerError,
    Unknown,
}

impl ErrorReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AuthError => "auth_error",
            Self::QuotaExhausted => "quota_exhausted",
            Self::RateLimitExceeded => "rate_limit_exceeded",
            Self::ModelCapacity => "model_capacity",
            Self::ServerError => "server_error",
            Self::Unknown => "unknown",
        }
    }
}

// ── Generic SSE frame ───────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct SseFrame {
    pub event: Option<String>,
    pub data: String,
    pub done: bool,
}
