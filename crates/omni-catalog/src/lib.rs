// Provider catalog — full 394-provider embedded registry synced from upstream
// cheaper-inference/OmniRoute (release/v3.8.55), plus runtime custom providers
// loaded from SQLite (omni-store).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;

const EMBEDDED_CATALOG_JSON: &str = include_str!("../data/providers.json");

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelSpec {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aliases: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_length: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_reasoning: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supported_thinking_efforts: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_vision: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsupported_params: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderDescriptor {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub base_urls: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub responses_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub messages_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url_suffix: Option<String>,
    /// apikey | oauth | optional | none (or legacy header name for custom rows)
    pub auth_type: String,
    /// bearer | x-api-key | x-goog-api-key | api-key | authorization-key | none
    #[serde(default = "default_auth_header")]
    pub auth_header: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_prefix: Option<String>,
    #[serde(default)]
    pub chat_path: String,
    /// openai | claude | gemini | openai-responses | ...
    pub format: String,
    #[serde(default = "default_executor")]
    pub executor: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub passthrough_models: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id_prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_context_length: Option<u64>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub model_specs: Vec<ModelSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anonymous_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub is_free_tier: bool,
}

fn default_auth_header() -> String {
    "bearer".to_string()
}

fn default_executor() -> String {
    "default".to_string()
}

#[derive(Debug, Clone, Deserialize)]
struct EmbeddedCatalogPayload {
    upstream_version: String,
    providers: Vec<ProviderDescriptor>,
}

static EMBEDDED_DATA: OnceLock<EmbeddedCatalogPayload> = OnceLock::new();

fn embedded_payload() -> &'static EmbeddedCatalogPayload {
    EMBEDDED_DATA.get_or_init(|| {
        serde_json::from_str(EMBEDDED_CATALOG_JSON)
            .expect("embedded providers.json must be valid JSON")
    })
}

pub fn upstream_version() -> &'static str {
    &embedded_payload().upstream_version
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireFormat {
    OpenAI,
    Claude,
    Gemini,
    OpenAIResponses,
}

impl ProviderDescriptor {
    pub fn wire_format(&self) -> WireFormat {
        match self.format.as_str() {
            "claude" | "anthropic" => WireFormat::Claude,
            "gemini" | "antigravity" => WireFormat::Gemini,
            "openai-responses" => WireFormat::OpenAIResponses,
            _ => WireFormat::OpenAI,
        }
    }

    pub fn effective_wire_format(&self, model: &str) -> WireFormat {
        if let Some(spec) = self.find_model_spec(model) {
            if let Some(tf) = &spec.target_format {
                return match tf.as_str() {
                    "claude" | "anthropic" => WireFormat::Claude,
                    "gemini" => WireFormat::Gemini,
                    "openai-responses" => WireFormat::OpenAIResponses,
                    _ => WireFormat::OpenAI,
                };
            }
        }
        self.wire_format()
    }

    pub fn effective_auth_header(&self) -> &str {
        // Support legacy custom provider rows where auth_type stored the header name.
        match self.auth_type.as_str() {
            "x-api-key" | "x-goog-api-key" | "api-key" | "authorization-key" | "none" => {
                self.auth_type.as_str()
            }
            _ => self.auth_header.as_str(),
        }
    }

    pub fn find_model_spec(&self, model: &str) -> Option<&ModelSpec> {
        self.model_specs.iter().find(|spec| {
            spec.id == model
                || spec
                    .aliases
                    .as_ref()
                    .map(|a| a.iter().any(|x| x == model))
                    .unwrap_or(false)
        })
    }

    pub fn supports_model(&self, model: &str) -> bool {
        if self.passthrough_models {
            return true;
        }
        if self.find_model_spec(model).is_some() {
            return true;
        }
        self.models
            .iter()
            .any(|m| m == model || wildcard_match(m, model))
    }

    pub fn matches_id_or_alias(&self, key: &str) -> bool {
        self.id.eq_ignore_ascii_case(key)
            || self
                .alias
                .as_deref()
                .map(|a| a.eq_ignore_ascii_case(key))
                .unwrap_or(false)
    }

    pub fn upstream_model_id(&self, model: &str) -> String {
        if let Some(prefix) = &self.model_id_prefix {
            if !model.starts_with(prefix) {
                return format!("{prefix}{model}");
            }
        }
        model.to_string()
    }

    pub fn context_length_for(&self, model: &str) -> u64 {
        self.find_model_spec(model)
            .and_then(|s| s.context_length)
            .or(self.default_context_length)
            .unwrap_or(128_000)
    }

    pub fn unsupported_params_for(&self, model: &str) -> Vec<String> {
        self.find_model_spec(model)
            .and_then(|s| s.unsupported_params.clone())
            .unwrap_or_default()
    }

    /// Build final upstream HTTP URL for a base URL, target model, and stream flag.
    /// Correctly handles:
    /// - Gemini `:generateContent` / `:streamGenerateContent?alt=sse` + `{model}` template
    /// - Anthropic `messages_url` + `url_suffix` (e.g. `?beta=true`)
    /// - OpenAI full-endpoint `baseUrl` vs bare `/v1` prefix (`opencode-zen`) vs custom `chat_path`
    pub fn resolve_chat_url(&self, base_url: &str, model: &str, stream: bool) -> String {
        let trimmed = base_url.trim().trim_end_matches('/');
        if trimmed.is_empty() {
            return String::new();
        }
        let wire = self.effective_wire_format(model);
        let upstream_model = self.upstream_model_id(model);

        if wire == WireFormat::Claude {
            let mut url = if let Some(murl) = &self.messages_url {
                murl.trim().to_string()
            } else if !self.chat_path.is_empty() && !trimmed.ends_with(&self.chat_path) {
                format!("{trimmed}{}", self.chat_path)
            } else if trimmed.ends_with("/v1") {
                format!("{trimmed}/messages")
            } else {
                trimmed.to_string()
            };
            if let Some(suffix) = &self.url_suffix {
                if !url.contains(suffix) {
                    url.push_str(suffix);
                }
            }
            return url;
        }

        if wire == WireFormat::Gemini {
            let action = if stream {
                ":streamGenerateContent?alt=sse"
            } else {
                ":generateContent"
            };
            let mut url = if !self.chat_path.is_empty() {
                format!("{trimmed}{}", self.chat_path)
            } else {
                trimmed.to_string()
            };
            if url.contains("{model}") {
                url = url.replace("{model}", &upstream_model);
                if stream && url.ends_with(":generateContent") {
                    url = url.replace(":generateContent", action);
                }
                return url;
            }
            if url.ends_with("/models") {
                return format!("{url}/{upstream_model}{action}");
            }
            if !url.contains(":generateContent") && !url.contains(":streamGenerateContent") {
                return format!("{url}/{upstream_model}{action}");
            }
            return url;
        }

        // OpenAI / OpenAI-Responses
        let mut url = if !self.chat_path.is_empty() && !trimmed.ends_with(&self.chat_path) {
            format!("{trimmed}{}", self.chat_path)
        } else if wire == WireFormat::OpenAIResponses {
            if let Some(rurl) = &self.responses_base_url {
                rurl.trim().to_string()
            } else if trimmed.ends_with("/v1") {
                format!("{trimmed}/responses")
            } else {
                trimmed.to_string()
            }
        } else if trimmed.ends_with("/v1") || trimmed.ends_with("/openai") {
            format!("{trimmed}/chat/completions")
        } else {
            trimmed.to_string()
        };

        if let Some(suffix) = &self.url_suffix {
            if !url.contains(suffix) {
                url.push_str(suffix);
            }
        }
        url
    }

    /// Derive a non-chat OpenAI-compatible endpoint URL (e.g. `/embeddings`,
    /// `/images/generations`, `/images/edits`, `/audio/speech`, `/audio/transcriptions`,
    /// `/rerank`, `/moderations`, `/decisions`) from a provider base URL.
    pub fn resolve_endpoint_url(&self, base_url: &str, endpoint_path: &str) -> String {
        let trimmed = base_url.trim().trim_end_matches('/');
        if trimmed.is_empty() {
            return String::new();
        }
        let ep = if endpoint_path.starts_with('/') {
            endpoint_path.to_string()
        } else {
            format!("/{endpoint_path}")
        };
        for suffix in [
            "/chat/completions",
            "/completions",
            "/messages",
            "/responses",
        ] {
            if let Some(root) = trimmed.strip_suffix(suffix) {
                return format!("{root}{ep}");
            }
        }
        if trimmed.ends_with("/v1") || trimmed.ends_with("/api/v1") || trimmed.ends_with("/openai/v1") {
            format!("{trimmed}{ep}")
        } else {
            format!("{trimmed}/v1{ep}")
        }
    }
}

/// `name-*`, `*-suffix`, or `*` wildcard matching on provider model entry.
fn wildcard_match(pattern: &str, model: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        model.starts_with(prefix)
    } else if let Some(suffix) = pattern.strip_prefix('*') {
        model.ends_with(suffix)
    } else {
        false
    }
}

/// Full upstream v3.8.55 provider catalog (394 providers).
pub fn seed_catalog() -> Vec<ProviderDescriptor> {
    embedded_payload().providers.clone()
}

/// Resolved target from a requested model string (either `provider/model` or bare `model`).
#[derive(Debug, Clone)]
pub struct ResolvedModel<'a> {
    pub provider: &'a ProviderDescriptor,
    pub model: String,
    pub explicit_provider: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Catalog {
    pub providers: Vec<ProviderDescriptor>,
}

impl Catalog {
    pub fn with_seed() -> Self {
        Self {
            providers: seed_catalog(),
        }
    }

    pub fn provider_count(&self) -> usize {
        self.providers.len()
    }

    pub fn model_count(&self) -> usize {
        self.providers.iter().map(|p| p.model_specs.len()).sum()
    }

    pub fn add(&mut self, p: ProviderDescriptor) {
        self.providers.retain(|x| x.id != p.id);
        self.providers.push(p);
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.providers.len();
        self.providers.retain(|x| x.id != id);
        self.providers.len() != before
    }

    /// Find provider by exact id OR alias (case-insensitive).
    pub fn find(&self, id_or_alias: &str) -> Option<&ProviderDescriptor> {
        self.providers
            .iter()
            .find(|p| p.id == id_or_alias)
            .or_else(|| self.providers.iter().find(|p| p.matches_id_or_alias(id_or_alias)))
    }

    /// Resolve a model specification:
    /// - If `spec` is `provider/model` and `provider` matches a known provider id/alias,
    ///   return that exact provider with stripped model id.
    /// - Otherwise match all providers supporting `spec` (explicit model matches first,
    ///   wildcard/passthrough providers after).
    pub fn resolve_model(&self, spec: &str) -> Vec<ResolvedModel<'_>> {
        if let Some((prefix, rest)) = spec.split_once('/') {
            if !rest.is_empty() {
                if let Some(p) = self.find(prefix) {
                    // Special case: models like `groq/compound` or `openai/gpt-oss-120b`
                    // might be exact model IDs owned by that provider or another provider.
                    if p.find_model_spec(spec).is_some() {
                        return vec![ResolvedModel {
                            provider: p,
                            model: spec.to_string(),
                            explicit_provider: true,
                        }];
                    }
                    return vec![ResolvedModel {
                        provider: p,
                        model: rest.to_string(),
                        explicit_provider: true,
                    }];
                }
            }
        }

        let mut exact = Vec::new();
        let mut wildcard = Vec::new();
        for p in &self.providers {
            if p.find_model_spec(spec).is_some() || p.models.iter().any(|m| m == spec) {
                exact.push(ResolvedModel {
                    provider: p,
                    model: spec.to_string(),
                    explicit_provider: false,
                });
            } else if p.supports_model(spec) {
                wildcard.push(ResolvedModel {
                    provider: p,
                    model: spec.to_string(),
                    explicit_provider: false,
                });
            }
        }
        exact.extend(wildcard);
        exact
    }

    pub fn find_for_model(&self, model: &str) -> Vec<&ProviderDescriptor> {
        self.resolve_model(model)
            .into_iter()
            .map(|r| r.provider)
            .collect()
    }

    pub fn list_model_ids(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for p in &self.providers {
            for spec in &p.model_specs {
                let qualified = format!("{}/{}", p.id, spec.id);
                if seen.insert(qualified.clone()) {
                    out.push(qualified);
                }
            }
            if p.model_specs.is_empty() {
                for m in &p.models {
                    if m == "*" {
                        continue;
                    }
                    let qualified = format!("{}/{}", p.id, m);
                    if seen.insert(qualified.clone()) {
                        out.push(qualified);
                    }
                }
            }
        }
        out
    }

    pub fn list_model_entries(&self) -> Vec<(&ProviderDescriptor, &ModelSpec)> {
        let mut out = Vec::new();
        for p in &self.providers {
            for spec in &p.model_specs {
                out.push((p, spec));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_loads_full_upstream_v3855_catalog() {
        let c = Catalog::with_seed();
        assert_eq!(upstream_version(), "3.8.55");
        assert!(c.provider_count() >= 390, "expected >=390 providers, got {}", c.provider_count());
        assert!(c.model_count() >= 3000, "expected >=3000 models, got {}", c.model_count());
        // Core + v3.8.54/v3.8.55 providers present
        for id in [
            "anthropic",
            "gemini",
            "openai",
            "groq",
            "pollinations",
            "openrouter",
            "chatgpt",
            "minimax-code",
            "opencode-zen",
            "cerebrium",
            "renderhour",
        ] {
            assert!(c.find(id).is_some(), "missing provider {id}");
        }
    }

    #[test]
    fn alias_lookup_and_prefix_resolution() {
        let c = Catalog::with_seed();
        assert_eq!(c.find("pol").unwrap().id, "pollinations");
        assert_eq!(c.find("ds").unwrap().id, "deepseek");
        let resolved = c.resolve_model("anthropic/claude-opus-4.7");
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].provider.id, "anthropic");
        assert_eq!(resolved[0].model, "claude-opus-4.7");
    }

    #[test]
    fn gemini_url_builder_handles_stream_and_non_stream() {
        let c = Catalog::with_seed();
        let g = c.find("gemini").unwrap();
        let non_stream = g.resolve_chat_url(&g.base_urls[0], "gemini-3.8-flash", false);
        assert_eq!(
            non_stream,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.8-flash:generateContent"
        );
        let stream = g.resolve_chat_url(&g.base_urls[0], "gemini-3.8-flash", true);
        assert_eq!(
            stream,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.8-flash:streamGenerateContent?alt=sse"
        );
    }

    #[test]
    fn anthropic_url_builder_appends_beta_suffix() {
        let c = Catalog::with_seed();
        let a = c.find("anthropic").unwrap();
        let url = a.resolve_chat_url(&a.base_urls[0], "claude-opus-4.7", false);
        assert_eq!(url, "https://api.anthropic.com/v1/messages?beta=true");
    }

    #[test]
    fn opencode_zen_v1_appends_chat_completions() {
        let c = Catalog::with_seed();
        let z = c.find("opencode-zen").unwrap();
        let url = z.resolve_chat_url(&z.base_urls[0], "big-pickle", false);
        assert_eq!(url, "https://opencode.ai/zen/v1/chat/completions");
    }

    #[test]
    fn add_replaces_by_id() {
        let mut c = Catalog::with_seed();
        let before = c.provider_count();
        c.add(ProviderDescriptor {
            id: "openai".into(),
            alias: None,
            name: Some("OpenAI Custom".into()),
            base_urls: vec!["http://localhost:8000".into()],
            responses_base_url: None,
            messages_url: None,
            url_suffix: None,
            auth_type: "apikey".into(),
            auth_header: "bearer".into(),
            auth_prefix: None,
            chat_path: "/v1/chat/completions".into(),
            format: "openai".into(),
            executor: "default".into(),
            headers: HashMap::new(),
            passthrough_models: false,
            model_id_prefix: None,
            default_context_length: None,
            models: vec!["gpt-z".into()],
            model_specs: vec![],
            anonymous_api_key: None,
            timeout_ms: None,
            is_free_tier: false,
        });
        assert_eq!(c.provider_count(), before);
        assert!(c.find("openai").unwrap().supports_model("gpt-z"));
    }

    #[test]
    fn wire_format_parsing() {
        let c = Catalog::with_seed();
        assert_eq!(c.find("anthropic").unwrap().wire_format(), WireFormat::Claude);
        assert_eq!(c.find("gemini").unwrap().wire_format(), WireFormat::Gemini);
        assert_eq!(c.find("openai").unwrap().wire_format(), WireFormat::OpenAI);
        assert_eq!(c.find("chatgpt").unwrap().wire_format(), WireFormat::OpenAIResponses);
    }
}
