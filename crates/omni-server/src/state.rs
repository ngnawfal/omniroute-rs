// Shared app state: 394-provider catalog + SQLite store + dispatcher + routing/LKGP
// + rate limiter + guardrails + gamification + RTK/Caveman compression + semantic cache.

use chrono::{Duration, Utc};
use omni_cache::{SemanticCache, SemanticCacheConfig};
use omni_catalog::Catalog;
use omni_compress::{CavemanIntensity, CompressionConfig, RtkIntensity};
use omni_exec::dispatch::Dispatcher;
use omni_gami::{
    bump_action, calculate_level, check_new_badges, level_tier, merged_streak, record_unique,
    BadgeState, StreakData, XpAction,
};
use omni_guard::AgentGuardMode;
use omni_mitm::cert::CertAuthority;
use omni_mitm::proxy::MitmProxyHandle;
use omni_oauth::{OAuthSessionStore, RefreshCircuitBreaker, OAUTH_PROVIDERS};
use omni_router::{normalize_strategy, Strategy};
use omni_store::{CredentialRow, Store};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

#[derive(Debug, Clone, Default)]
pub struct CompressionTotals {
    pub requests_compressed: u64,
    pub total_tokens_saved: u64,
    pub total_compacted_turns: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpstreamProxyConfig {
    #[serde(default)]
    pub global: Option<String>,
    #[serde(default)]
    pub providers: HashMap<String, String>,
    #[serde(default)]
    pub pool: Vec<String>,
    #[serde(default)]
    pub apply_to_oauth: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertingConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub webhook_url: String,
    #[serde(default)]
    pub webhook_secret: String,
    #[serde(default)]
    pub telegram_bot_token: String,
    #[serde(default)]
    pub telegram_chat_id: String,
    #[serde(default = "default_true_bool")]
    pub on_needs_reauth: bool,
    #[serde(default = "default_true_bool")]
    pub on_breaker_open: bool,
    #[serde(default = "default_true_bool")]
    pub on_budget_alert: bool,
}

fn default_true_bool() -> bool {
    true
}

impl Default for AlertingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            webhook_url: String::new(),
            webhook_secret: String::new(),
            telegram_bot_token: String::new(),
            telegram_chat_id: String::new(),
            on_needs_reauth: true,
            on_breaker_open: true,
            on_budget_alert: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DashboardAuthConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub salt: String,
    #[serde(default)]
    pub password_hash: String,
    #[serde(default)]
    pub session_secret: String,
    #[serde(default)]
    pub updated_at: i64,
}

pub struct AppState {
    pub data_dir: String,
    pub store: Store,
    pub catalog: RwLock<Catalog>,
    pub dispatcher: Dispatcher,
    pub rr_counter: AtomicU64,
    pub proxy_rr_counter: AtomicU64,
    pub target_metrics: RwLock<HashMap<String, TargetMetric>>,
    pub last_good: RwLock<HashMap<String, String>>,
    pub started_at: Instant,
    rate_windows: Mutex<HashMap<String, (Instant, u32)>>,
    pub semantic_cache: SemanticCache,
    pub compression_totals: Mutex<CompressionTotals>,
    pub oauth_sessions: OAuthSessionStore,
    pub oauth_breaker: RefreshCircuitBreaker,
    pub mitm_ca: RwLock<Arc<CertAuthority>>,
    pub mitm_handle: Mutex<Option<MitmProxyHandle>>,
    pub mitm_bypass: RwLock<Vec<String>>,
    pub alert_dedup: Mutex<HashMap<String, i64>>,
}

#[derive(Debug, Clone, Default)]
pub struct TargetMetric {
    pub requests: u64,
    pub success_rate: f64,
    pub avg_latency_ms: f64,
}

impl AppState {
    pub fn open(data_dir: &str) -> anyhow::Result<Self> {
        let store = Store::open_default(data_dir)?;
        let mut catalog = Catalog::with_seed();
        for p in store.list_custom_providers()? {
            catalog.add(p);
        }
        let mitm_dir = std::path::Path::new(data_dir).join("mitm");
        let mitm_ca = CertAuthority::load_or_create(&mitm_dir)
            .or_else(|_| CertAuthority::generate())
            .map_err(|e| anyhow::anyhow!("MITM CA init: {e}"))?;
        Ok(Self {
            data_dir: data_dir.to_string(),
            store,
            catalog: RwLock::new(catalog),
            dispatcher: Dispatcher::default(),
            rr_counter: AtomicU64::new(0),
            proxy_rr_counter: AtomicU64::new(0),
            target_metrics: RwLock::new(HashMap::new()),
            last_good: RwLock::new(HashMap::new()),
            started_at: Instant::now(),
            rate_windows: Mutex::new(HashMap::new()),
            semantic_cache: SemanticCache::new(),
            compression_totals: Mutex::new(CompressionTotals::default()),
            oauth_sessions: OAuthSessionStore::new(),
            oauth_breaker: RefreshCircuitBreaker::new(),
            mitm_ca: RwLock::new(Arc::new(mitm_ca)),
            mitm_handle: Mutex::new(None),
            mitm_bypass: RwLock::new(vec![]),
            alert_dedup: Mutex::new(HashMap::new()),
        })
    }

    pub fn reload_catalog(&self) -> anyhow::Result<()> {
        let mut catalog = Catalog::with_seed();
        for p in self.store.list_custom_providers()? {
            catalog.add(p);
        }
        *self.catalog.write().unwrap() = catalog;
        Ok(())
    }

    pub fn next_rr(&self) -> u64 {
        self.rr_counter.fetch_add(1, Ordering::Relaxed)
    }

    pub fn bump_metric(&self, exec_key: &str, ok: bool, latency_ms: f64) {
        let mut map = self.target_metrics.write().unwrap();
        let m = map.entry(exec_key.to_string()).or_default();
        m.requests += 1;
        m.success_rate = if m.requests == 1 {
            if ok { 100.0 } else { 0.0 }
        } else if ok {
            m.success_rate * 0.9 + 100.0 * 0.1
        } else {
            m.success_rate * 0.9
        };
        m.avg_latency_ms = if m.avg_latency_ms == 0.0 {
            latency_ms
        } else {
            m.avg_latency_ms * 0.9 + latency_ms * 0.1
        };
    }

    pub fn set_last_good(&self, scope: &str, exec_key: &str) {
        self.last_good
            .write()
            .unwrap()
            .insert(scope.to_string(), exec_key.to_string());
    }

    pub fn get_last_good(&self, scope: &str) -> Option<String> {
        self.last_good.read().unwrap().get(scope).cloned()
    }

    pub fn default_strategy(&self) -> Strategy {
        if let Ok(Some(s)) = self.store.get_setting("default_strategy") {
            return normalize_strategy(&s);
        }
        if let Ok(s) = std::env::var("OMNIROUTE_DEFAULT_STRATEGY") {
            return normalize_strategy(&s);
        }
        Strategy::Priority
    }

    pub fn guardrails_enabled(&self) -> bool {
        if let Ok(Some(v)) = self.store.get_setting("guardrails_enabled") {
            return matches!(v.as_str(), "true" | "1" | "on");
        }
        std::env::var("GUARDRAILS_ENABLED")
            .map(|v| matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "on"))
            .unwrap_or(true)
    }

    pub fn pii_redaction_enabled(&self) -> bool {
        if let Ok(Some(v)) = self.store.get_setting("pii_redaction_enabled") {
            return matches!(v.as_str(), "true" | "1" | "on");
        }
        std::env::var("PII_REDACTION_ENABLED")
            .map(|v| matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "on"))
            .unwrap_or(false)
    }

    pub fn pii_vault_enabled(&self) -> bool {
        if let Ok(Some(v)) = self.store.get_setting("pii_vault_enabled") {
            return matches!(v.as_str(), "true" | "1" | "on");
        }
        std::env::var("PII_VAULT_ENABLED")
            .map(|v| matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "on"))
            .unwrap_or(false)
    }

    pub fn agent_guard_mode(&self) -> AgentGuardMode {
        if let Ok(Some(v)) = self.store.get_setting("agent_guard_mode") {
            return AgentGuardMode::from_str_lossy(&v);
        }
        std::env::var("AGENT_GUARD_MODE")
            .map(|v| AgentGuardMode::from_str_lossy(&v))
            .unwrap_or(AgentGuardMode::Off)
    }

    pub fn compression_config(&self) -> CompressionConfig {
        if let Ok(Some(raw)) = self.store.get_setting("compression_config") {
            if let Ok(cfg) = serde_json::from_str::<CompressionConfig>(&raw) {
                return cfg;
            }
        }
        let mut cfg = CompressionConfig::default();
        if let Ok(v) = std::env::var("OMNIROUTE_COMPRESSION_ENABLED") {
            cfg.enabled = matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "on");
        }
        if let Ok(v) = std::env::var("OMNIROUTE_CAVEMAN_INTENSITY") {
            cfg.caveman_intensity = CavemanIntensity::from_str_lossy(&v);
        }
        if let Ok(v) = std::env::var("OMNIROUTE_RTK_INTENSITY") {
            cfg.rtk_intensity = RtkIntensity::from_str_lossy(&v);
        }
        cfg
    }

    pub fn semantic_cache_config(&self) -> SemanticCacheConfig {
        if let Ok(Some(raw)) = self.store.get_setting("semantic_cache_config") {
            if let Ok(cfg) = serde_json::from_str::<SemanticCacheConfig>(&raw) {
                return cfg;
            }
        }
        let mut cfg = SemanticCacheConfig::default();
        if let Ok(v) = std::env::var("OMNIROUTE_SEMANTIC_CACHE_ENABLED") {
            cfg.enabled = matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "on");
        }
        if let Ok(v) = std::env::var("OMNIROUTE_SEMANTIC_CACHE_THRESHOLD") {
            if let Ok(t) = v.parse::<f64>() {
                cfg.similarity_threshold = t.clamp(0.1, 1.0);
            }
        }
        if let Ok(v) = std::env::var("OMNIROUTE_SEMANTIC_CACHE_TTL_MS") {
            if let Ok(t) = v.parse::<u64>() {
                cfg.ttl_ms = t;
            }
        }
        cfg
    }

    pub fn record_compression_savings(&self, saved_tokens: u64, compacted_turns: usize) {
        let mut guard = self.compression_totals.lock().unwrap();
        guard.requests_compressed += 1;
        guard.total_tokens_saved += saved_tokens;
        guard.total_compacted_turns += compacted_turns as u64;
    }

    pub fn compression_summary(&self) -> serde_json::Value {
        let cfg = self.compression_config();
        let totals = self.compression_totals.lock().unwrap().clone();
        serde_json::json!({
            "config": cfg,
            "stats": {
                "requests_compressed": totals.requests_compressed,
                "total_tokens_saved": totals.total_tokens_saved,
                "total_compacted_turns": totals.total_compacted_turns,
            }
        })
    }

    pub fn cache_summary(&self) -> serde_json::Value {
        serde_json::json!({
            "config": self.semantic_cache_config(),
            "stats": self.semantic_cache.stats(),
        })
    }

    pub fn oauth_summary(&self) -> serde_json::Value {
        let now = now_ms();
        let creds = self.store.list_credentials().unwrap_or_default();
        let oauth_creds: Vec<serde_json::Value> = creds
            .into_iter()
            .filter(|c| c.auth_type == "oauth" || c.refresh_token.is_some())
            .map(|c| {
                serde_json::json!({
                    "provider": c.provider,
                    "connection_id": c.connection_id,
                    "label": c.label,
                    "health_status": c.health_status,
                    "has_refresh_token": c.refresh_token.as_deref().map(|s| !s.is_empty()).unwrap_or(false),
                    "expires_at": c.expires_at,
                    "last_refreshed_at": c.last_refreshed_at,
                    "health_check_interval_min": c.health_check_interval_min,
                })
            })
            .collect();
        serde_json::json!({
            "supported_providers": OAUTH_PROVIDERS.len(),
            "active_pending_sessions": self.oauth_sessions.active_count(now),
            "connected_oauth_accounts": oauth_creds.len(),
            "accounts": oauth_creds,
            "refresh_breakers": self.oauth_breaker.snapshot(now),
            "healthcheck_disabled": omni_oauth::is_healthcheck_disabled(),
            "per_provider_concurrency": omni_oauth::healthcheck_per_provider_concurrency(),
            "google_refresh_delay_ms": omni_oauth::bg_refresh_google_delay_ms(),
        })
    }

    pub fn a2a_enabled(&self) -> bool {
        if let Ok(Some(v)) = self.store.get_setting("a2a_enabled") {
            return !matches!(v.to_ascii_lowercase().as_str(), "false" | "0" | "off" | "no");
        }
        true
    }

    // ── Upstream Proxy Resolution (Per-Account > Per-Provider > Global/Pool > Direct) ─

    pub fn proxy_config(&self) -> UpstreamProxyConfig {
        let mut cfg = self
            .store
            .get_setting("upstream_proxy_config")
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_str::<UpstreamProxyConfig>(&raw).ok())
            .unwrap_or_default();
        if cfg.global.as_deref().map(str::trim).unwrap_or("").is_empty() {
            if let Ok(env_p) = std::env::var("OMNIROUTE_UPSTREAM_PROXY") {
                let trimmed = env_p.trim().to_string();
                if !trimmed.is_empty() {
                    cfg.global = Some(trimmed);
                }
            }
        }
        cfg
    }

    pub fn resolve_proxy_for(&self, provider: &str, cred: Option<&CredentialRow>) -> Option<String> {
        // 1. Per-account override in provider_specific_json["proxy_url" | "proxyUrl"]
        if let Some(c) = cred {
            if let Some(raw) = c
                .provider_specific_json
                .get("proxy_url")
                .or_else(|| c.provider_specific_json.get("proxyUrl"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                if raw.eq_ignore_ascii_case("direct") || raw.eq_ignore_ascii_case("none") {
                    return None;
                }
                return Some(raw.to_string());
            }
        }
        let cfg = self.proxy_config();
        // 2. Per-provider override
        if let Some(raw) = cfg
            .providers
            .get(provider)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            if raw.eq_ignore_ascii_case("direct") || raw.eq_ignore_ascii_case("none") {
                return None;
            }
            return Some(raw.to_string());
        }
        // 3. Global proxy or round-robin pool
        if let Some(raw) = cfg.global.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            if !raw.eq_ignore_ascii_case("direct") && !raw.eq_ignore_ascii_case("none") {
                return Some(raw.to_string());
            }
        }
        let active_pool: Vec<&str> = cfg
            .pool
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if !active_pool.is_empty() {
            let idx = (self.proxy_rr_counter.fetch_add(1, Ordering::Relaxed) as usize)
                % active_pool.len();
            return Some(active_pool[idx].to_string());
        }
        None
    }

    pub fn http_client_for(&self, provider: &str, cred: Option<&CredentialRow>) -> reqwest::Client {
        let proxy = self.resolve_proxy_for(provider, cred);
        self.dispatcher
            .client_for_proxy(proxy.as_deref())
            .unwrap_or_else(|_| self.dispatcher.client.clone())
    }

    pub fn oauth_http_client_for(&self, provider: &str, cred: Option<&CredentialRow>) -> reqwest::Client {
        // Always respect explicit per-account proxy_url; otherwise follow apply_to_oauth
        let acct_proxy = cred.and_then(|c| {
            c.provider_specific_json
                .get("proxy_url")
                .or_else(|| c.provider_specific_json.get("proxyUrl"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
        });
        if let Some(p) = acct_proxy {
            return self
                .dispatcher
                .client_for_proxy(Some(&p))
                .unwrap_or_else(|_| self.dispatcher.client.clone());
        }
        if self.proxy_config().apply_to_oauth {
            return self.http_client_for(provider, cred);
        }
        self.dispatcher.client.clone()
    }

    // ── Alerting & Dashboard Password Auth Config ───────────────────

    pub fn alerting_config(&self) -> AlertingConfig {
        self.store
            .get_setting("alerting_config")
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_str::<AlertingConfig>(&raw).ok())
            .unwrap_or_default()
    }

    pub fn dashboard_auth_config(&self) -> DashboardAuthConfig {
        self.store
            .get_setting("dashboard_auth_config")
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_str::<DashboardAuthConfig>(&raw).ok())
            .unwrap_or_default()
    }

    pub fn is_dashboard_locked(&self) -> bool {
        let cfg = self.dashboard_auth_config();
        cfg.enabled && !cfg.password_hash.is_empty()
    }

    pub fn hash_dashboard_password(salt: &str, password: &str) -> String {
        let mut h = Sha256::new();
        h.update(b"omniroute-dash-v1:");
        h.update(salt.as_bytes());
        h.update(b":");
        h.update(password.as_bytes());
        hex::encode(h.finalize())
    }

    pub fn verify_dashboard_password(&self, password: &str) -> bool {
        let cfg = self.dashboard_auth_config();
        if !cfg.enabled || cfg.password_hash.is_empty() {
            return true;
        }
        let candidate = Self::hash_dashboard_password(&cfg.salt, password);
        constant_time_eq(candidate.as_bytes(), cfg.password_hash.as_bytes())
    }

    pub fn mint_dashboard_session_token(&self) -> Option<String> {
        let cfg = self.dashboard_auth_config();
        if !cfg.enabled || cfg.session_secret.is_empty() {
            return None;
        }
        let exp = now_ms() + 7 * 86_400_000; // 7 days
        let payload = format!("{exp}");
        let sig = sign_session_payload(&cfg.session_secret, &payload);
        Some(format!("{payload}.{sig}"))
    }

    pub fn verify_dashboard_session_token(&self, token: &str) -> bool {
        let cfg = self.dashboard_auth_config();
        if !cfg.enabled || cfg.session_secret.is_empty() {
            return true;
        }
        let Some((payload, sig)) = token.split_once('.') else {
            return false;
        };
        let Ok(exp) = payload.parse::<i64>() else {
            return false;
        };
        if now_ms() > exp {
            return false;
        }
        let expected = sign_session_payload(&cfg.session_secret, payload);
        constant_time_eq(sig.as_bytes(), expected.as_bytes())
    }

    pub fn mitm_status_json(&self) -> serde_json::Value {
        let ca = self.mitm_ca.read().unwrap().clone();
        let bypass = self.mitm_bypass.read().unwrap().clone();
        let targets = omni_mitm::all_targets();
        let runtime = self
            .mitm_handle
            .lock()
            .unwrap()
            .as_ref()
            .map(|h| h.stats());
        serde_json::json!({
            "mode": "live_tls_connect_proxy",
            "running": runtime.as_ref().map(|r| r.running).unwrap_or(false),
            "bind_addr": runtime.as_ref().and_then(|r| r.bind_addr.clone()),
            "ca_fingerprint_sha256": ca.ca_fingerprint_sha256(),
            "ca_download_path": "/api/mitm/ca.crt",
            "cached_leaf_certs": ca.cached_leaf_count(),
            "bypass_patterns": bypass,
            "stats": runtime,
            "target_count": targets.len(),
            "targets": targets,
        })
    }

    pub fn check_rpm_limit(&self, key: &str, rpm_limit: u32) -> bool {
        if rpm_limit == 0 {
            return true;
        }
        let mut map = self.rate_windows.lock().unwrap();
        let now = Instant::now();
        let entry = map.entry(key.to_string()).or_insert((now, 0));
        if now.duration_since(entry.0).as_secs() >= 60 {
            *entry = (now, 1);
            true
        } else if entry.1 < rpm_limit {
            entry.1 += 1;
            true
        } else {
            false
        }
    }

    pub fn record_gamification_action(
        &self,
        action: XpAction,
        provider: Option<&str>,
        model: Option<&str>,
    ) {
        let Ok(mut row) = self.store.load_gamification("default") else {
            return;
        };
        let now = Utc::now();
        let today = now.format("%Y-%m-%d").to_string();
        let yesterday = (now - Duration::days(1)).format("%Y-%m-%d").to_string();

        let prev_streak = StreakData {
            current_streak: row.current_streak,
            longest_streak: row.longest_streak,
            last_active_date: row.last_active_date.clone(),
            streak_start_date: row.streak_start_date.clone(),
        };
        let next_streak = merged_streak(&prev_streak, &today, &yesterday);
        let streak_bonus = if next_streak.current_streak > prev_streak.current_streak {
            XpAction::StreakBonus.base_xp()
        } else {
            0
        };

        let mut badge_state: BadgeState =
            serde_json::from_str(&row.badge_state_json).unwrap_or_default();
        bump_action(&mut badge_state, action.as_str(), 1);
        record_unique(&mut badge_state, provider, model);

        let newly_unlocked =
            check_new_badges(&badge_state, next_streak.current_streak, &row.unlocked_badges);
        let badge_xp = (newly_unlocked.len() as i64) * XpAction::BadgeUnlock.base_xp();
        row.unlocked_badges.extend(newly_unlocked);

        row.total_xp += action.base_xp() + streak_bonus + badge_xp;
        row.current_streak = next_streak.current_streak;
        row.longest_streak = next_streak.longest_streak;
        row.last_active_date = next_streak.last_active_date;
        row.streak_start_date = next_streak.streak_start_date;
        row.badge_state_json = serde_json::to_string(&badge_state).unwrap_or_else(|_| "{}".into());
        row.updated_at = now.timestamp_millis();

        let _ = self.store.save_gamification(&row);
    }

    pub fn gamification_summary(&self) -> serde_json::Value {
        let row = self.store.load_gamification("default").unwrap_or_else(|_| {
            omni_store::GamificationRow {
                user_id: "default".into(),
                total_xp: 0,
                current_streak: 0,
                longest_streak: 0,
                last_active_date: String::new(),
                streak_start_date: String::new(),
                badge_state_json: "{}".into(),
                unlocked_badges: vec![],
                updated_at: 0,
            }
        });
        let level = calculate_level(row.total_xp);
        serde_json::json!({
            "user_id": row.user_id,
            "total_xp": row.total_xp,
            "level": level,
            "tier": level_tier(level),
            "xp_to_next_level": omni_gami::xp_to_next_level(row.total_xp, level + 1),
            "current_streak": row.current_streak,
            "longest_streak": row.longest_streak,
            "last_active_date": row.last_active_date,
            "unlocked_badges": row.unlocked_badges,
            "total_builtin_badges": omni_gami::BUILTIN_BADGES.len(),
        })
    }
}

pub fn hash_api_key(key: &str) -> String {
    if key.is_empty() {
        return String::new();
    }
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    let digest = hasher.finalize();
    hex::encode(&digest[..12])
}

fn sign_session_payload(secret: &str, payload: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"omni-sess:");
    h.update(secret.as_bytes());
    h.update(b":");
    h.update(payload.as_bytes());
    hex::encode(h.finalize())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

pub fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

pub type SharedState = Arc<AppState>;
