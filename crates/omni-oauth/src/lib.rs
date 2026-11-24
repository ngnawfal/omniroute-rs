// omni-oauth — Native Rust OAuth 2.0 PKCE / Device-Code flows, Copilot token exchange,
// and v3.8.55 background token refresh policies:
//   - PKCE (RFC 7636 S256) + in-memory pending session store (5-minute TTL)
//   - RFC 8628 Device Authorization Grant (GitHub Copilot, GHE Copilot, Kiro, Kimi, MiniMax, Grok)
//   - GitHub Copilot short-lived token exchange (copilot_internal/v2/token)
//   - TOKEN_REFRESH_RETRY_JITTER=1 (deterministic SHA-256 75–125% retry scaling)
//   - BG_REFRESH_GOOGLE_DELAY_MS (serialized Google OAuth refresh slot clock)
//   - HEALTHCHECK_PER_PROVIDER_CONCURRENCY + OMNIROUTE_HEALTHCHECK_SKIP_PROVIDERS
//   - Per-provider / per-connection refresh circuit breaker + unrecoverable error detection
//   - AWS region allowlist SSRF guard (GHSA-6mwv-4mrm-5p3m)

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

pub const OAUTH_SESSION_TTL_MS: i64 = 5 * 60 * 1000;
pub const DEFAULT_REFRESH_TIMEOUT_MS: u64 = 30_000;
pub const REFRESH_BREAKER_THRESHOLD: u32 = 5;
pub const REFRESH_BREAKER_COOLDOWN_MS: i64 = 30 * 60 * 1000;
pub const REFRESH_BREAKER_REGISTRY_CAP: usize = 500;

pub const GITHUB_COPILOT_USER_AGENT: &str = "GitHubCopilotChat/0.26.7";
pub const GITHUB_COPILOT_EDITOR_VERSION: &str = "vscode/1.99.3";
pub const GITHUB_COPILOT_PLUGIN_VERSION: &str = "copilot-chat/0.26.7";
pub const GITHUB_COPILOT_API_VERSION: &str = "2025-04-01";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OAuthFlowKind {
    Pkce,
    DeviceCode,
    ImportOnly,
}

#[derive(Debug, Clone, Serialize)]
pub struct OAuthProviderConfig {
    pub id: &'static str,
    pub display_name: &'static str,
    pub flow: OAuthFlowKind,
    pub authorize_url: &'static str,
    pub token_url: &'static str,
    pub device_code_url: &'static str,
    pub userinfo_url: &'static str,
    pub copilot_token_url: &'static str,
    pub scopes: &'static [&'static str],
    pub client_id_env: &'static str,
    pub client_secret_env: &'static str,
    pub default_client_id: &'static str,
    pub uses_pkce: bool,
    pub loopback_port: u16,
    pub callback_path: &'static str,
    pub extra_authorize_params: &'static [(&'static str, &'static str)],
}

impl OAuthProviderConfig {
    pub fn resolve_client_id(&self) -> String {
        if !self.client_id_env.is_empty() {
            if let Ok(v) = std::env::var(self.client_id_env) {
                let trimmed = v.trim();
                if !trimmed.is_empty() {
                    return trimmed.to_string();
                }
            }
        }
        self.default_client_id.to_string()
    }

    pub fn resolve_client_secret(&self) -> Option<String> {
        if !self.client_secret_env.is_empty() {
            if let Ok(v) = std::env::var(self.client_secret_env) {
                let trimmed = v.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
        None
    }

    pub fn default_redirect_uri(&self) -> String {
        if self.loopback_port > 0 {
            format!("http://127.0.0.1:{}{}", self.loopback_port, self.callback_path)
        } else {
            format!("http://localhost:20128/api/oauth/{}/callback", self.id)
        }
    }
}

/// Priority OAuth provider registry aligned with OmniRoute v3.8.55 `src/lib/oauth/constants/oauth.ts`.
pub static OAUTH_PROVIDERS: &[OAuthProviderConfig] = &[
    OAuthProviderConfig {
        id: "claude",
        display_name: "Claude Code (Anthropic OAuth)",
        flow: OAuthFlowKind::Pkce,
        authorize_url: "https://claude.ai/oauth/authorize",
        token_url: "https://console.anthropic.com/v1/oauth/token",
        device_code_url: "",
        userinfo_url: "",
        copilot_token_url: "",
        scopes: &["org:create_api_key", "user:profile", "user:inference"],
        client_id_env: "CLAUDE_OAUTH_CLIENT_ID",
        client_secret_env: "",
        default_client_id: "9d1c250a-e61b-44d9-88ed-5944d1962f5e",
        uses_pkce: true,
        loopback_port: 54545,
        callback_path: "/callback",
        extra_authorize_params: &[("code", "true")],
    },
    OAuthProviderConfig {
        id: "codex",
        display_name: "OpenAI Codex CLI",
        flow: OAuthFlowKind::Pkce,
        authorize_url: "https://auth.openai.com/oauth/authorize",
        token_url: "https://auth.openai.com/oauth/token",
        device_code_url: "",
        userinfo_url: "",
        copilot_token_url: "",
        scopes: &["openid", "profile", "email", "offline_access"],
        client_id_env: "CODEX_OAUTH_CLIENT_ID",
        client_secret_env: "",
        default_client_id: "app_EMoamEEZ73f0CkXaXp7hrann",
        uses_pkce: true,
        loopback_port: 1455,
        callback_path: "/auth/callback",
        extra_authorize_params: &[
            ("id_token_add_organizations", "true"),
            ("codex_cli_simplified_flow", "true"),
            ("originator", "codex_cli_rs"),
            ("prompt", "login"),
        ],
    },
    OAuthProviderConfig {
        id: "chatgpt",
        display_name: "ChatGPT / OpenAI Native OAuth (PKCE)",
        flow: OAuthFlowKind::Pkce,
        authorize_url: "https://auth.openai.com/oauth/authorize",
        token_url: "https://auth.openai.com/oauth/token",
        device_code_url: "",
        userinfo_url: "",
        copilot_token_url: "",
        scopes: &["openid", "profile", "email", "offline_access"],
        client_id_env: "CODEX_OAUTH_CLIENT_ID",
        client_secret_env: "",
        default_client_id: "app_EMoamEEZ73f0CkXaXp7hrann",
        uses_pkce: true,
        loopback_port: 1455,
        callback_path: "/auth/callback",
        extra_authorize_params: &[
            ("id_token_add_organizations", "true"),
            ("originator", "openai_native"),
            ("prompt", "login"),
        ],
    },
    OAuthProviderConfig {
        id: "github",
        display_name: "GitHub Copilot",
        flow: OAuthFlowKind::DeviceCode,
        authorize_url: "",
        token_url: "https://github.com/login/oauth/access_token",
        device_code_url: "https://github.com/login/device/code",
        userinfo_url: "https://api.github.com/user",
        copilot_token_url: "https://api.github.com/copilot_internal/v2/token",
        scopes: &["read:user"],
        client_id_env: "GITHUB_OAUTH_CLIENT_ID",
        client_secret_env: "",
        default_client_id: "Iv1.b507a08c87ecfe98",
        uses_pkce: false,
        loopback_port: 0,
        callback_path: "",
        extra_authorize_params: &[],
    },
    OAuthProviderConfig {
        id: "ghe-copilot",
        display_name: "GitHub Enterprise Copilot",
        flow: OAuthFlowKind::DeviceCode,
        authorize_url: "",
        token_url: "https://github.com/login/oauth/access_token",
        device_code_url: "https://github.com/login/device/code",
        userinfo_url: "https://api.github.com/user",
        copilot_token_url: "https://api.github.com/copilot_internal/v2/token",
        scopes: &["read:user"],
        client_id_env: "GHE_COPILOT_OAUTH_CLIENT_ID",
        client_secret_env: "",
        default_client_id: "Iv1.b507a08c87ecfe98",
        uses_pkce: false,
        loopback_port: 0,
        callback_path: "",
        extra_authorize_params: &[],
    },
    OAuthProviderConfig {
        id: "antigravity",
        display_name: "Antigravity (Google Cloud Code OAuth)",
        flow: OAuthFlowKind::Pkce,
        authorize_url: "https://accounts.google.com/o/oauth2/v2/auth",
        token_url: "https://oauth2.googleapis.com/token",
        device_code_url: "",
        userinfo_url: "https://www.googleapis.com/oauth2/v1/userinfo",
        copilot_token_url: "",
        // Matches upstream 9router/v3.8.55 fix: intentionally omits `openid` scope
        // so Google does not route into the hanging `firstparty/nativeapp` consent.
        scopes: &[
            "https://www.googleapis.com/auth/cloud-platform",
            "https://www.googleapis.com/auth/userinfo.email",
            "https://www.googleapis.com/auth/userinfo.profile",
            "https://www.googleapis.com/auth/cclog",
            "https://www.googleapis.com/auth/experimentsandconfigs",
        ],
        client_id_env: "ANTIGRAVITY_OAUTH_CLIENT_ID",
        client_secret_env: "ANTIGRAVITY_OAUTH_CLIENT_SECRET",
        default_client_id: "",
        uses_pkce: false,
        loopback_port: 51121,
        callback_path: "/oauth-callback",
        extra_authorize_params: &[("access_type", "offline"), ("prompt", "consent")],
    },
    OAuthProviderConfig {
        id: "agy",
        display_name: "Antigravity CLI (agy)",
        flow: OAuthFlowKind::Pkce,
        authorize_url: "https://accounts.google.com/o/oauth2/v2/auth",
        token_url: "https://oauth2.googleapis.com/token",
        device_code_url: "",
        userinfo_url: "https://www.googleapis.com/oauth2/v1/userinfo",
        copilot_token_url: "",
        scopes: &[
            "https://www.googleapis.com/auth/cloud-platform",
            "https://www.googleapis.com/auth/userinfo.email",
            "https://www.googleapis.com/auth/userinfo.profile",
            "https://www.googleapis.com/auth/cclog",
            "https://www.googleapis.com/auth/experimentsandconfigs",
        ],
        client_id_env: "ANTIGRAVITY_OAUTH_CLIENT_ID",
        client_secret_env: "ANTIGRAVITY_OAUTH_CLIENT_SECRET",
        default_client_id: "",
        uses_pkce: false,
        loopback_port: 51121,
        callback_path: "/oauth-callback",
        extra_authorize_params: &[("access_type", "offline"), ("prompt", "consent")],
    },
    OAuthProviderConfig {
        id: "gemini-cli",
        display_name: "Google Gemini CLI",
        flow: OAuthFlowKind::Pkce,
        authorize_url: "https://accounts.google.com/o/oauth2/v2/auth",
        token_url: "https://oauth2.googleapis.com/token",
        device_code_url: "",
        userinfo_url: "https://www.googleapis.com/oauth2/v1/userinfo",
        copilot_token_url: "",
        scopes: &[
            "https://www.googleapis.com/auth/cloud-platform",
            "https://www.googleapis.com/auth/userinfo.email",
            "https://www.googleapis.com/auth/userinfo.profile",
        ],
        client_id_env: "GEMINI_CLI_OAUTH_CLIENT_ID",
        client_secret_env: "GEMINI_CLI_OAUTH_CLIENT_SECRET",
        default_client_id: "",
        uses_pkce: true,
        loopback_port: 8085,
        callback_path: "/oauth2callback",
        extra_authorize_params: &[("access_type", "offline"), ("prompt", "consent")],
    },
    OAuthProviderConfig {
        id: "grok-cli",
        display_name: "Grok Build / CLI (xAI)",
        flow: OAuthFlowKind::Pkce,
        authorize_url: "https://auth.x.ai/oauth2/authorize",
        token_url: "https://auth.x.ai/oauth2/token",
        device_code_url: "https://auth.x.ai/oauth2/device/code",
        userinfo_url: "",
        copilot_token_url: "",
        scopes: &["openid", "profile", "email", "offline_access", "grok-cli:access"],
        client_id_env: "GROK_OAUTH_CLIENT_ID",
        client_secret_env: "",
        default_client_id: "",
        uses_pkce: true,
        loopback_port: 56122,
        callback_path: "/callback",
        extra_authorize_params: &[],
    },
    OAuthProviderConfig {
        id: "xai-oauth",
        display_name: "xAI API OAuth",
        flow: OAuthFlowKind::Pkce,
        authorize_url: "https://auth.x.ai/oauth2/authorize",
        token_url: "https://auth.x.ai/oauth2/token",
        device_code_url: "",
        userinfo_url: "",
        copilot_token_url: "",
        scopes: &["openid", "profile", "email", "offline_access", "grok-cli:access", "api:access"],
        client_id_env: "GROK_OAUTH_CLIENT_ID",
        client_secret_env: "",
        default_client_id: "",
        uses_pkce: true,
        loopback_port: 56121,
        callback_path: "/callback",
        extra_authorize_params: &[],
    },
    OAuthProviderConfig {
        id: "minimax-code",
        display_name: "MiniMax Code (Device Code)",
        flow: OAuthFlowKind::DeviceCode,
        authorize_url: "",
        token_url: "https://api.minimax.io/oauth2/token",
        device_code_url: "https://api.minimax.io/oauth2/device/code",
        userinfo_url: "",
        copilot_token_url: "",
        scopes: &["group_id", "profile", "model.completion"],
        client_id_env: "MINIMAX_OAUTH_CLIENT_ID",
        client_secret_env: "",
        default_client_id: "",
        uses_pkce: true,
        loopback_port: 0,
        callback_path: "",
        extra_authorize_params: &[],
    },
    OAuthProviderConfig {
        id: "kimi-coding",
        display_name: "Kimi Coding (Device Code)",
        flow: OAuthFlowKind::DeviceCode,
        authorize_url: "",
        token_url: "https://auth.kimi.com/api/oauth/token",
        device_code_url: "https://auth.kimi.com/api/oauth/device_authorization",
        userinfo_url: "",
        copilot_token_url: "",
        scopes: &[],
        client_id_env: "KIMI_CODING_OAUTH_CLIENT_ID",
        client_secret_env: "",
        default_client_id: "",
        uses_pkce: false,
        loopback_port: 0,
        callback_path: "",
        extra_authorize_params: &[],
    },
    OAuthProviderConfig {
        id: "kiro",
        display_name: "Kiro (AWS Builder ID / Social)",
        flow: OAuthFlowKind::DeviceCode,
        authorize_url: "https://prod.us-east-1.auth.desktop.kiro.dev/login",
        token_url: "https://oidc.us-east-1.amazonaws.com/token",
        device_code_url: "https://oidc.us-east-1.amazonaws.com/device_authorization",
        userinfo_url: "",
        copilot_token_url: "",
        scopes: &[
            "codewhisperer:completions",
            "codewhisperer:analysis",
            "codewhisperer:conversations",
        ],
        client_id_env: "KIRO_OAUTH_CLIENT_ID",
        client_secret_env: "",
        default_client_id: "kiro-cli",
        uses_pkce: false,
        loopback_port: 0,
        callback_path: "",
        extra_authorize_params: &[],
    },
    OAuthProviderConfig {
        id: "gitlab-duo",
        display_name: "GitLab Duo",
        flow: OAuthFlowKind::Pkce,
        authorize_url: "https://gitlab.com/oauth/authorize",
        token_url: "https://gitlab.com/oauth/token",
        device_code_url: "",
        userinfo_url: "https://gitlab.com/api/v4/user",
        copilot_token_url: "",
        scopes: &["ai_features", "read_user"],
        client_id_env: "GITLAB_DUO_OAUTH_CLIENT_ID",
        client_secret_env: "GITLAB_DUO_OAUTH_CLIENT_SECRET",
        default_client_id: "",
        uses_pkce: true,
        loopback_port: 0,
        callback_path: "",
        extra_authorize_params: &[],
    },
    OAuthProviderConfig {
        id: "openference",
        display_name: "Openference",
        flow: OAuthFlowKind::Pkce,
        authorize_url: "https://openference.com/app/oauth/authorize",
        token_url: "https://openference.com/oauth/token",
        device_code_url: "",
        userinfo_url: "https://openference.com/oauth/userinfo",
        copilot_token_url: "",
        scopes: &["openid", "profile", "email", "model:invoke", "offline_access"],
        client_id_env: "OPENFERENCE_OAUTH_CLIENT_ID",
        client_secret_env: "",
        default_client_id: "",
        uses_pkce: true,
        loopback_port: 56123,
        callback_path: "/callback",
        extra_authorize_params: &[],
    },
    OAuthProviderConfig {
        id: "cursor",
        display_name: "Cursor IDE (Import / Deep Control)",
        flow: OAuthFlowKind::ImportOnly,
        authorize_url: "https://cursor.com/loginDeepControl",
        token_url: "https://api2.cursor.sh/auth/exchange_user_api_key",
        device_code_url: "",
        userinfo_url: "",
        copilot_token_url: "",
        scopes: &[],
        client_id_env: "",
        client_secret_env: "",
        default_client_id: "",
        uses_pkce: true,
        loopback_port: 0,
        callback_path: "",
        extra_authorize_params: &[],
    },
    OAuthProviderConfig {
        id: "trae",
        display_name: "Trae IDE (Import Token)",
        flow: OAuthFlowKind::ImportOnly,
        authorize_url: "https://trae.ai",
        token_url: "",
        device_code_url: "",
        userinfo_url: "",
        copilot_token_url: "",
        scopes: &[],
        client_id_env: "",
        client_secret_env: "",
        default_client_id: "",
        uses_pkce: false,
        loopback_port: 0,
        callback_path: "",
        extra_authorize_params: &[],
    },
];

pub fn find_oauth_provider(id: &str) -> Option<&'static OAuthProviderConfig> {
    let norm = id.trim().to_ascii_lowercase();
    let canonical = match norm.as_str() {
        "anthropic" | "claude-code" => "claude",
        "openai" | "openai-native" => "chatgpt",
        "copilot" | "github-copilot" => "github",
        "gemini" => "gemini-cli",
        other => other,
    };
    OAUTH_PROVIDERS.iter().find(|p| p.id == canonical)
}

// ── AWS Region SSRF Guard (GHSA-6mwv-4mrm-5p3m) ─────────────────────

/// Strictly validates canonical AWS region format (`^[a-z]{2}-[a-z]+-\d{1,2}$`)
/// so user-supplied region strings can never inject hostnames into upstream URLs.
pub fn is_valid_aws_region(region: &str) -> bool {
    let mut parts = region.split('-');
    let (Some(p1), Some(p2), Some(p3), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    p1.len() == 2
        && p1.bytes().all(|b| b.is_ascii_lowercase())
        && !p2.is_empty()
        && p2.bytes().all(|b| b.is_ascii_lowercase())
        && (1..=2).contains(&p3.len())
        && p3.bytes().all(|b| b.is_ascii_digit())
}

pub fn assert_valid_aws_region(region: &str) -> Result<&str, String> {
    if is_valid_aws_region(region) {
        Ok(region)
    } else {
        Err(format!("Invalid AWS region: {region}"))
    }
}

// ── PKCE (RFC 7636 S256) ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PkcePair {
    pub code_verifier: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub state: String,
}

pub fn compute_s256_challenge(code_verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(code_verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

pub fn generate_pkce() -> PkcePair {
    let mut verifier_bytes = [0u8; 32];
    let mut state_bytes = [0u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut verifier_bytes);
    rand::rngs::OsRng.fill_bytes(&mut state_bytes);
    let code_verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);
    let code_challenge = compute_s256_challenge(&code_verifier);
    let state = URL_SAFE_NO_PAD.encode(state_bytes);
    PkcePair {
        code_verifier,
        code_challenge,
        code_challenge_method: "S256".into(),
        state,
    }
}

/// RFC 3986 percent-encoding for OAuth query/form parameters.
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push_str(&format!("{b:02X}"));
            }
        }
    }
    out
}

pub fn build_authorize_url(
    cfg: &OAuthProviderConfig,
    redirect_uri: &str,
    pkce: &PkcePair,
    client_id_override: Option<&str>,
    extra_params: &[(&str, &str)],
) -> String {
    let client_id = client_id_override
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| cfg.resolve_client_id());

    let mut params: Vec<(String, String)> = vec![
        ("response_type".into(), "code".into()),
        ("client_id".into(), client_id),
        ("redirect_uri".into(), redirect_uri.to_string()),
        ("state".into(), pkce.state.clone()),
    ];
    if !cfg.scopes.is_empty() {
        params.push(("scope".into(), cfg.scopes.join(" ")));
    }
    if cfg.uses_pkce {
        params.push(("code_challenge".into(), pkce.code_challenge.clone()));
        params.push((
            "code_challenge_method".into(),
            pkce.code_challenge_method.clone(),
        ));
    }
    for (k, v) in cfg.extra_authorize_params {
        params.push(((*k).to_string(), (*v).to_string()));
    }
    for (k, v) in extra_params {
        params.push(((*k).to_string(), (*v).to_string()));
    }

    let query = params
        .iter()
        .map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    let sep = if cfg.authorize_url.contains('?') { "&" } else { "?" };
    format!("{}{sep}{query}", cfg.authorize_url)
}

// ── Pending OAuth Session Store (5-minute TTL) ──────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingOAuthSession {
    pub state: String,
    pub provider: String,
    pub connection_id: String,
    pub redirect_uri: String,
    pub code_verifier: String,
    pub client_id: String,
    pub token_url_override: Option<String>,
    pub device_code: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Default)]
pub struct OAuthSessionStore {
    sessions: Mutex<HashMap<String, PendingOAuthSession>>,
}

impl OAuthSessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, session: PendingOAuthSession, now_ms: i64) {
        let mut map = self.sessions.lock().unwrap();
        map.retain(|_, s| now_ms - s.created_at_ms <= OAUTH_SESSION_TTL_MS);
        map.insert(session.state.clone(), session);
    }

    pub fn take(&self, state: &str, now_ms: i64) -> Option<PendingOAuthSession> {
        let mut map = self.sessions.lock().unwrap();
        map.retain(|_, s| now_ms - s.created_at_ms <= OAUTH_SESSION_TTL_MS);
        map.remove(state)
    }

    pub fn get(&self, state: &str, now_ms: i64) -> Option<PendingOAuthSession> {
        let mut map = self.sessions.lock().unwrap();
        map.retain(|_, s| now_ms - s.created_at_ms <= OAUTH_SESSION_TTL_MS);
        map.get(state).cloned()
    }

    pub fn active_count(&self, now_ms: i64) -> usize {
        let mut map = self.sessions.lock().unwrap();
        map.retain(|_, s| now_ms - s.created_at_ms <= OAUTH_SESSION_TTL_MS);
        map.len()
    }
}

// ── OAuth Token Exchange, Device Code, Refresh, Copilot Mint ────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_type: Option<String>,
    pub expires_in: Option<i64>,
    pub scope: Option<String>,
    pub id_token: Option<String>,
    pub raw: Value,
}

impl OAuthTokenResponse {
    pub fn expires_at_ms(&self, now_ms: i64) -> Option<i64> {
        self.expires_in
            .filter(|&s| s > 0)
            .map(|s| now_ms.saturating_add(s.saturating_mul(1000)))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: i64,
    pub interval: u64,
}

#[derive(Debug, Clone)]
pub enum DevicePollOutcome {
    Authorized(OAuthTokenResponse),
    Pending,
    SlowDown { next_interval_secs: u64 },
    Expired,
    Denied(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopilotToken {
    pub token: String,
    pub expires_at_ms: i64,
    pub api_endpoint: Option<String>,
}

#[derive(Debug, Clone)]
pub enum RefreshOutcome {
    Refreshed(OAuthTokenResponse),
    Unrecoverable(String),
    Transient(String),
}

fn parse_token_json(v: Value) -> Result<OAuthTokenResponse, String> {
    let access_token = v
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            let err = v
                .get("error_description")
                .or_else(|| v.get("error"))
                .and_then(Value::as_str)
                .unwrap_or("missing access_token in OAuth token response");
            err.to_string()
        })?
        .to_string();
    Ok(OAuthTokenResponse {
        access_token,
        refresh_token: v
            .get("refresh_token")
            .and_then(Value::as_str)
            .map(String::from),
        token_type: v
            .get("token_type")
            .and_then(Value::as_str)
            .map(String::from),
        expires_in: v.get("expires_in").and_then(Value::as_i64),
        scope: v.get("scope").and_then(Value::as_str).map(String::from),
        id_token: v.get("id_token").and_then(Value::as_str).map(String::from),
        raw: v,
    })
}

pub async fn exchange_authorization_code(
    client: &reqwest::Client,
    cfg: &OAuthProviderConfig,
    code: &str,
    redirect_uri: &str,
    code_verifier: Option<&str>,
    client_id_override: Option<&str>,
    token_url_override: Option<&str>,
) -> Result<OAuthTokenResponse, String> {
    let token_url = token_url_override
        .filter(|s| !s.is_empty())
        .unwrap_or(cfg.token_url);
    if token_url.is_empty() {
        return Err(format!("provider {} does not support token exchange", cfg.id));
    }
    let client_id = client_id_override
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| cfg.resolve_client_id());

    // Claude OAuth expects JSON body; standard OAuth2 endpoints accept form-urlencoded.
    let resp = if cfg.id == "claude" {
        let mut body = json!({
            "grant_type": "authorization_code",
            "code": code,
            "redirect_uri": redirect_uri,
            "client_id": client_id,
        });
        if let Some(cv) = code_verifier.filter(|s| !s.is_empty()) {
            body["code_verifier"] = Value::String(cv.to_string());
        }
        client
            .post(token_url)
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("token exchange network error: {e}"))?
    } else {
        let mut form: Vec<(&str, String)> = vec![
            ("grant_type", "authorization_code".into()),
            ("code", code.to_string()),
            ("redirect_uri", redirect_uri.to_string()),
            ("client_id", client_id),
        ];
        if let Some(secret) = cfg.resolve_client_secret() {
            form.push(("client_secret", secret));
        }
        if let Some(cv) = code_verifier.filter(|s| !s.is_empty()) {
            form.push(("code_verifier", cv.to_string()));
        }
        client
            .post(token_url)
            .header("Accept", "application/json")
            .form(&form)
            .send()
            .await
            .map_err(|e| format!("token exchange network error: {e}"))?
    };

    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("OAuth token exchange HTTP {}: {text}", status.as_u16()));
    }
    let v: Value = serde_json::from_str(&text)
        .map_err(|e| format!("invalid OAuth token JSON: {e}: {text}"))?;
    parse_token_json(v)
}

pub async fn start_device_code(
    client: &reqwest::Client,
    cfg: &OAuthProviderConfig,
    client_id_override: Option<&str>,
    device_url_override: Option<&str>,
) -> Result<DeviceCodeResponse, String> {
    let url = device_url_override
        .filter(|s| !s.is_empty())
        .unwrap_or(cfg.device_code_url);
    if url.is_empty() {
        return Err(format!("provider {} does not support device code flow", cfg.id));
    }
    let client_id = client_id_override
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| cfg.resolve_client_id());

    let mut form: Vec<(&str, String)> = vec![("client_id", client_id)];
    if !cfg.scopes.is_empty() {
        form.push(("scope", cfg.scopes.join(" ")));
    }
    let resp = client
        .post(url)
        .header("Accept", "application/json")
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("device code network error: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("device code HTTP {}: {text}", status.as_u16()));
    }
    let v: Value =
        serde_json::from_str(&text).map_err(|e| format!("invalid device code JSON: {e}"))?;
    let device_code = v
        .get("device_code")
        .and_then(Value::as_str)
        .ok_or("missing device_code")?
        .to_string();
    let user_code = v
        .get("user_code")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let verification_uri = v
        .get("verification_uri")
        .or_else(|| v.get("verification_url"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let verification_uri_complete = v
        .get("verification_uri_complete")
        .and_then(Value::as_str)
        .map(String::from);
    let expires_in = v.get("expires_in").and_then(Value::as_i64).unwrap_or(900);
    let interval = v.get("interval").and_then(Value::as_u64).unwrap_or(5).max(1);
    Ok(DeviceCodeResponse {
        device_code,
        user_code,
        verification_uri,
        verification_uri_complete,
        expires_in,
        interval,
    })
}

pub async fn poll_device_code(
    client: &reqwest::Client,
    cfg: &OAuthProviderConfig,
    device_code: &str,
    client_id_override: Option<&str>,
    token_url_override: Option<&str>,
) -> Result<DevicePollOutcome, String> {
    let token_url = token_url_override
        .filter(|s| !s.is_empty())
        .unwrap_or(cfg.token_url);
    let client_id = client_id_override
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| cfg.resolve_client_id());

    let form = [
        ("client_id", client_id.as_str()),
        ("device_code", device_code),
        ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
    ];
    let resp = client
        .post(token_url)
        .header("Accept", "application/json")
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("device poll network error: {e}"))?;
    let text = resp.text().await.unwrap_or_default();
    let v: Value =
        serde_json::from_str(&text).map_err(|e| format!("invalid device poll JSON: {e}"))?;
    if let Some(err) = v.get("error").and_then(Value::as_str) {
        return Ok(match err {
            "authorization_pending" => DevicePollOutcome::Pending,
            "slow_down" => DevicePollOutcome::SlowDown {
                next_interval_secs: v.get("interval").and_then(Value::as_u64).unwrap_or(10),
            },
            "expired_token" => DevicePollOutcome::Expired,
            other => DevicePollOutcome::Denied(
                v.get("error_description")
                    .and_then(Value::as_str)
                    .unwrap_or(other)
                    .to_string(),
            ),
        });
    }
    Ok(DevicePollOutcome::Authorized(parse_token_json(v)?))
}

/// Exchange a GitHub OAuth token (`gho_*` / `ghu_*`) for a short-lived Copilot API token.
pub async fn exchange_copilot_token(
    client: &reqwest::Client,
    github_access_token: &str,
    copilot_token_url_override: Option<&str>,
) -> Result<CopilotToken, String> {
    let url = copilot_token_url_override
        .filter(|s| !s.is_empty())
        .unwrap_or("https://api.github.com/copilot_internal/v2/token");
    let resp = client
        .get(url)
        .header("Authorization", format!("token {github_access_token}"))
        .header("Accept", "application/json")
        .header("User-Agent", GITHUB_COPILOT_USER_AGENT)
        .header("Editor-Version", GITHUB_COPILOT_EDITOR_VERSION)
        .header("Editor-Plugin-Version", GITHUB_COPILOT_PLUGIN_VERSION)
        .header("X-GitHub-Api-Version", GITHUB_COPILOT_API_VERSION)
        .send()
        .await
        .map_err(|e| format!("copilot token exchange network error: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("Copilot token exchange HTTP {}: {text}", status.as_u16()));
    }
    let v: Value =
        serde_json::from_str(&text).map_err(|e| format!("invalid copilot token JSON: {e}"))?;
    let token = v
        .get("token")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or("missing token in copilot response")?
        .to_string();
    let expires_at_sec = v.get("expires_at").and_then(Value::as_i64).unwrap_or(0);
    let expires_at_ms = if expires_at_sec > 1_000_000_000_000 {
        expires_at_sec
    } else if expires_at_sec > 0 {
        expires_at_sec * 1000
    } else {
        chrono::Utc::now().timestamp_millis() + 25 * 60 * 1000
    };
    let api_endpoint = v
        .pointer("/endpoints/api")
        .and_then(Value::as_str)
        .map(String::from);
    Ok(CopilotToken {
        token,
        expires_at_ms,
        api_endpoint,
    })
}

// ── Token Refresh + v3.8.55 Resilience Policies ─────────────────────

const UNRECOVERABLE_MARKERS: &[&str] = &[
    "invalid_grant",
    "refresh_token_reused",
    "refresh_token_expired",
    "token_revoked",
    "revoked",
    "invalid_client",
    "unauthorized_client",
    "account_suspended",
    "access_denied",
];

pub fn is_unrecoverable_refresh_error(status: u16, body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    if UNRECOVERABLE_MARKERS.iter().any(|m| lower.contains(m)) {
        return true;
    }
    status == 400 && (lower.contains("invalid") && lower.contains("token"))
}

/// Deterministic SHA-256 retry jitter (75%–125% of `base_ms`), opt-in via
/// `TOKEN_REFRESH_RETRY_JITTER=1` matching upstream `open-sse/services/tokenRefresh/circuitBreaker.ts`.
pub fn jittered_retry_delay_ms(
    base_ms: u64,
    account_id: &str,
    attempt: u32,
    force_enabled: Option<bool>,
) -> u64 {
    let enabled = force_enabled.unwrap_or_else(|| {
        std::env::var("TOKEN_REFRESH_RETRY_JITTER")
            .map(|v| v.trim() == "1")
            .unwrap_or(false)
    });
    if !enabled {
        return base_ms;
    }
    let mut hasher = Sha256::new();
    hasher.update(format!("{account_id}:{attempt}").as_bytes());
    let digest = hasher.finalize();
    let h = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) as f64;
    let factor = 0.75 + 0.5 * (h / (u32::MAX as f64 + 1.0));
    ((base_ms as f64) * factor).round().max(1.0) as u64
}

pub fn healthcheck_per_provider_concurrency() -> usize {
    std::env::var("HEALTHCHECK_PER_PROVIDER_CONCURRENCY")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0)
}

pub fn healthcheck_skip_providers() -> HashSet<String> {
    std::env::var("OMNIROUTE_HEALTHCHECK_SKIP_PROVIDERS")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn is_healthcheck_disabled() -> bool {
    std::env::var("OMNIROUTE_DISABLE_TOKEN_HEALTHCHECK")
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

pub fn bg_refresh_google_delay_ms() -> u64 {
    std::env::var("BG_REFRESH_GOOGLE_DELAY_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(0)
}

pub fn is_google_bg_refresh_provider(provider: &str) -> bool {
    matches!(
        provider.trim().to_ascii_lowercase().as_str(),
        "antigravity" | "agy" | "gemini" | "gemini-cli"
    )
}

/// Serialized slot waiter for Google OAuth background refreshes (`BG_REFRESH_GOOGLE_DELAY_MS`).
static GOOGLE_SLOT_LOCK: LazyLock<tokio::sync::Mutex<Option<Instant>>> =
    LazyLock::new(|| tokio::sync::Mutex::new(None));

pub async fn wait_for_google_refresh_slot(provider: &str, base_ms_override: Option<u64>) {
    if !is_google_bg_refresh_provider(provider) {
        return;
    }
    let base_ms = base_ms_override.unwrap_or_else(bg_refresh_google_delay_ms);
    if base_ms == 0 {
        return;
    }
    let mut guard = GOOGLE_SLOT_LOCK.lock().await;
    if let Some(last) = *guard {
        let jitter_span = (base_ms / 3).max(1);
        let jitter = (rand::rngs::OsRng.next_u64()) % jitter_span;
        let gap = Duration::from_millis(base_ms + jitter);
        let elapsed = last.elapsed();
        if elapsed < gap {
            tokio::time::sleep(gap - elapsed).await;
        }
    }
    *guard = Some(Instant::now());
}

// ── Per-provider / per-connection Refresh Circuit Breaker ───────────

#[derive(Debug, Clone, Serialize)]
pub struct RefreshBreakerEntry {
    pub failures: u32,
    pub blocked_until_ms: i64,
}

#[derive(Default)]
pub struct RefreshCircuitBreaker {
    entries: Mutex<HashMap<String, RefreshBreakerEntry>>,
}

impl RefreshCircuitBreaker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn key(provider: &str, connection_id: Option<&str>) -> String {
        match connection_id.map(str::trim).filter(|s| !s.is_empty()) {
            Some(cid) => format!("{provider}::conn::{cid}"),
            None => provider.to_string(),
        }
    }

    pub fn is_blocked(&self, key: &str, now_ms: i64) -> bool {
        let mut map = self.entries.lock().unwrap();
        if let Some(entry) = map.get(key) {
            if entry.blocked_until_ms > now_ms {
                return true;
            }
            if entry.blocked_until_ms > 0 {
                map.remove(key);
            }
        }
        false
    }

    pub fn record_success(&self, key: &str) {
        self.entries.lock().unwrap().remove(key);
    }

    pub fn record_failure(&self, key: &str, now_ms: i64, threshold: u32, cooldown_ms: i64) -> bool {
        let mut map = self.entries.lock().unwrap();
        if map.len() >= REFRESH_BREAKER_REGISTRY_CAP && !map.contains_key(key) {
            // Evict expired or non-blocked entries; never evict active blocks.
            let evict_key = map
                .iter()
                .find(|(_, v)| v.blocked_until_ms <= now_ms)
                .map(|(k, _)| k.clone());
            if let Some(ek) = evict_key {
                map.remove(&ek);
            } else {
                return false;
            }
        }
        let entry = map.entry(key.to_string()).or_insert(RefreshBreakerEntry {
            failures: 0,
            blocked_until_ms: 0,
        });
        entry.failures += 1;
        if entry.failures >= threshold {
            entry.blocked_until_ms = now_ms + cooldown_ms;
            return true;
        }
        false
    }

    pub fn snapshot(&self, now_ms: i64) -> HashMap<String, Value> {
        let map = self.entries.lock().unwrap();
        map.iter()
            .map(|(k, v)| {
                let blocked = v.blocked_until_ms > now_ms;
                (
                    k.clone(),
                    json!({
                        "failures": v.failures,
                        "blocked": blocked,
                        "remaining_ms": if blocked { v.blocked_until_ms - now_ms } else { 0 },
                    }),
                )
            })
            .collect()
    }
}

pub async fn refresh_access_token_once(
    client: &reqwest::Client,
    cfg: &OAuthProviderConfig,
    refresh_token: &str,
    client_id_override: Option<&str>,
    token_url_override: Option<&str>,
) -> RefreshOutcome {
    let token_url = token_url_override
        .filter(|s| !s.is_empty())
        .unwrap_or(cfg.token_url);
    if token_url.is_empty() {
        return RefreshOutcome::Unrecoverable(format!(
            "provider {} does not support token refresh",
            cfg.id
        ));
    }
    let client_id = client_id_override
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| cfg.resolve_client_id());

    let req = if cfg.id == "claude" {
        client
            .post(token_url)
            .timeout(Duration::from_millis(DEFAULT_REFRESH_TIMEOUT_MS))
            .header("Accept", "application/json")
            .json(&json!({
                "grant_type": "refresh_token",
                "refresh_token": refresh_token,
                "client_id": client_id,
            }))
    } else {
        let mut form: Vec<(&str, String)> = vec![
            ("grant_type", "refresh_token".into()),
            ("refresh_token", refresh_token.to_string()),
            ("client_id", client_id),
        ];
        if let Some(secret) = cfg.resolve_client_secret() {
            form.push(("client_secret", secret));
        }
        client
            .post(token_url)
            .timeout(Duration::from_millis(DEFAULT_REFRESH_TIMEOUT_MS))
            .header("Accept", "application/json")
            .form(&form)
    };

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => return RefreshOutcome::Transient(format!("network error: {e}")),
    };
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    if (200..300).contains(&status) {
        match serde_json::from_str::<Value>(&text).and_then(|v| {
            parse_token_json(v).map_err(serde::de::Error::custom)
        }) {
            Ok(tok) => RefreshOutcome::Refreshed(tok),
            Err(e) => RefreshOutcome::Transient(format!("invalid refresh JSON: {e}")),
        }
    } else if is_unrecoverable_refresh_error(status, &text) {
        RefreshOutcome::Unrecoverable(format!("HTTP {status}: {text}"))
    } else {
        RefreshOutcome::Transient(format!("HTTP {status}: {text}"))
    }
}

/// Refresh an OAuth token with retries, deterministic SHA-256 jitter, Google slot spacing,
/// and per-connection circuit breaker protection.
#[allow(clippy::too_many_arguments)]
pub async fn refresh_with_retry(
    client: &reqwest::Client,
    breaker: &RefreshCircuitBreaker,
    cfg: &OAuthProviderConfig,
    connection_id: &str,
    refresh_token: &str,
    client_id_override: Option<&str>,
    token_url_override: Option<&str>,
    max_retries: u32,
) -> RefreshOutcome {
    let key = RefreshCircuitBreaker::key(cfg.id, Some(connection_id));
    let now_ms = chrono::Utc::now().timestamp_millis();
    if breaker.is_blocked(&key, now_ms) {
        return RefreshOutcome::Transient(format!("refresh circuit breaker active for {key}"));
    }

    wait_for_google_refresh_slot(cfg.id, None).await;

    let mut last_err = String::from("refresh failed");
    let attempts = max_retries.max(1);
    for attempt in 0..attempts {
        if attempt > 0 {
            let delay = jittered_retry_delay_ms((attempt as u64) * 1000, connection_id, attempt, None);
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
        match refresh_access_token_once(
            client,
            cfg,
            refresh_token,
            client_id_override,
            token_url_override,
        )
        .await
        {
            RefreshOutcome::Refreshed(tok) => {
                breaker.record_success(&key);
                return RefreshOutcome::Refreshed(tok);
            }
            RefreshOutcome::Unrecoverable(msg) => {
                // Short-circuit retries immediately so caller marks credential needs_reauth.
                return RefreshOutcome::Unrecoverable(msg);
            }
            RefreshOutcome::Transient(msg) => {
                last_err = msg;
            }
        }
    }

    let now_end = chrono::Utc::now().timestamp_millis();
    breaker.record_failure(
        &key,
        now_end,
        REFRESH_BREAKER_THRESHOLD,
        REFRESH_BREAKER_COOLDOWN_MS,
    );
    RefreshOutcome::Transient(last_err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::{get, post};
    use axum::Router;

    #[test]
    fn pkce_rfc7636_s256_vector() {
        // RFC 7636 Appendix B test vector
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let expected = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert_eq!(compute_s256_challenge(verifier), expected);

        let pair = generate_pkce();
        assert_eq!(pair.code_challenge_method, "S256");
        assert_eq!(compute_s256_challenge(&pair.code_verifier), pair.code_challenge);
    }

    #[test]
    fn authorize_url_includes_codex_prompt_login_and_pkce() {
        let cfg = find_oauth_provider("codex").unwrap();
        let pkce = generate_pkce();
        let url = build_authorize_url(cfg, &cfg.default_redirect_uri(), &pkce, None, &[]);
        assert!(url.starts_with("https://auth.openai.com/oauth/authorize?"));
        assert!(url.contains("prompt=login"), "must prevent multi-account session takeover: {url}");
        assert!(url.contains("originator=codex_cli_rs"));
        assert!(url.contains("code_challenge_method=S256"));
    }

    #[test]
    fn antigravity_omits_openid_scope_per_v3855() {
        let cfg = find_oauth_provider("antigravity").unwrap();
        assert!(!cfg.scopes.contains(&"openid"));
        assert!(!cfg.uses_pkce);
    }

    #[test]
    fn deterministic_retry_jitter_stays_within_75_to_125_percent() {
        assert_eq!(jittered_retry_delay_ms(2000, "acct-1", 1, Some(false)), 2000);
        for i in 0..50 {
            let id = format!("account-{i}");
            let d1 = jittered_retry_delay_ms(2000, &id, 1, Some(true));
            let d2 = jittered_retry_delay_ms(2000, &id, 1, Some(true));
            assert_eq!(d1, d2, "must be deterministic across replicas");
            assert!((1500..=2500).contains(&d1), "out of 75-125% range: {d1}");
        }
    }

    #[test]
    fn aws_region_ssrf_guard() {
        assert!(is_valid_aws_region("us-east-1"));
        assert!(is_valid_aws_region("ap-southeast-2"));
        assert!(is_valid_aws_region("eu-central-1"));
        assert!(!is_valid_aws_region("127.0.0.1"));
        assert!(!is_valid_aws_region("us-east-1.evil.com"));
        assert!(!is_valid_aws_region("US-EAST-1"));
        assert!(!is_valid_aws_region("us-east-1/foo"));
    }

    #[test]
    fn refresh_breaker_trips_and_recovers() {
        let cb = RefreshCircuitBreaker::new();
        let key = RefreshCircuitBreaker::key("claude", Some("acct-1"));
        assert!(!cb.is_blocked(&key, 1_000));
        for _ in 0..4 {
            assert!(!cb.record_failure(&key, 1_000, 5, 60_000));
        }
        assert!(cb.record_failure(&key, 1_000, 5, 60_000));
        assert!(cb.is_blocked(&key, 30_000));
        assert!(!cb.is_blocked(&key, 65_000));
    }

    #[tokio::test]
    async fn mock_pkce_exchange_device_poll_refresh_and_copilot_mint() {
        let mock = Router::new()
            .route(
                "/token",
                post(|body: String| async move {
                    if body.contains("refresh_token=bad-revoked") {
                        return (
                            axum::http::StatusCode::BAD_REQUEST,
                            json!({"error": "invalid_grant", "error_description": "refresh token revoked"}).to_string(),
                        );
                    }
                    if body.contains("device_code=pending-code") {
                        return (
                            axum::http::StatusCode::OK,
                            json!({"error": "authorization_pending"}).to_string(),
                        );
                    }
                    (
                        axum::http::StatusCode::OK,
                        json!({
                            "access_token": "at_mock_123",
                            "refresh_token": "rt_mock_456",
                            "expires_in": 3600,
                            "token_type": "Bearer"
                        })
                        .to_string(),
                    )
                }),
            )
            .route(
                "/device/code",
                post(|| async {
                    json!({
                        "device_code": "dc_abc",
                        "user_code": "ABCD-1234",
                        "verification_uri": "https://github.com/login/device",
                        "expires_in": 900,
                        "interval": 5
                    })
                    .to_string()
                }),
            )
            .route(
                "/copilot_internal/v2/token",
                get(|| async {
                    json!({
                        "token": "tid=copilot_short_lived",
                        "expires_at": 1_900_000_000i64,
                        "endpoints": {"api": "https://api.individual.githubcopilot.com"}
                    })
                    .to_string()
                }),
            );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let _ = axum::serve(listener, mock).await;
        });

        let client = reqwest::Client::new();
        let base = format!("http://127.0.0.1:{port}");

        // 1. Authorization code exchange
        let codex = find_oauth_provider("codex").unwrap();
        let tok = exchange_authorization_code(
            &client,
            codex,
            "auth_code_1",
            "http://127.0.0.1:1455/auth/callback",
            Some("verifier_1"),
            None,
            Some(&format!("{base}/token")),
        )
        .await
        .unwrap();
        assert_eq!(tok.access_token, "at_mock_123");
        assert_eq!(tok.refresh_token.as_deref(), Some("rt_mock_456"));

        // 2. Device code start + pending poll + authorized poll
        let gh = find_oauth_provider("github").unwrap();
        let dc = start_device_code(&client, gh, None, Some(&format!("{base}/device/code")))
            .await
            .unwrap();
        assert_eq!(dc.user_code, "ABCD-1234");

        let p1 = poll_device_code(&client, gh, "pending-code", None, Some(&format!("{base}/token")))
            .await
            .unwrap();
        assert!(matches!(p1, DevicePollOutcome::Pending));

        let p2 = poll_device_code(&client, gh, "dc_abc", None, Some(&format!("{base}/token")))
            .await
            .unwrap();
        assert!(matches!(p2, DevicePollOutcome::Authorized(_)));

        // 3. Refresh success + unrecoverable invalid_grant
        let breaker = RefreshCircuitBreaker::new();
        let r_ok = refresh_with_retry(
            &client,
            &breaker,
            codex,
            "conn-1",
            "rt_valid",
            None,
            Some(&format!("{base}/token")),
            2,
        )
        .await;
        assert!(matches!(r_ok, RefreshOutcome::Refreshed(_)));

        let r_bad = refresh_with_retry(
            &client,
            &breaker,
            codex,
            "conn-2",
            "bad-revoked",
            None,
            Some(&format!("{base}/token")),
            3,
        )
        .await;
        assert!(matches!(r_bad, RefreshOutcome::Unrecoverable(_)));

        // 4. GitHub Copilot token exchange
        let copilot = exchange_copilot_token(
            &client,
            "gho_test",
            Some(&format!("{base}/copilot_internal/v2/token")),
        )
        .await
        .unwrap();
        assert_eq!(copilot.token, "tid=copilot_short_lived");
        assert_eq!(
            copilot.api_endpoint.as_deref(),
            Some("https://api.individual.githubcopilot.com")
        );
    }
}
