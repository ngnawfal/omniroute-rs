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
use omni_store::Store;
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

pub struct AppState {
    pub data_dir: String,
    pub store: Store,
    pub catalog: RwLock<Catalog>,
    pub dispatcher: Dispatcher,
    pub rr_counter: AtomicU64,
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

pub fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

pub type SharedState = Arc<AppState>;
