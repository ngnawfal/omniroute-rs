// Resilience registry — port of accountFallback.ts lockout + provider breaker
// semantics.
//
// Scopes (kept distinct per upstream design):
//   provider breaker   — whole provider (name `provider`)
//   connection breaker — one account/key (`provider::conn::id`)
//   model lockout      — provider+connection+model tuple, expiry-based

use crate::breaker::{get_breaker, BreakerConfig, BreakerState};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct ProviderProfile {
    pub transient_cooldown: u64,
    pub rate_limit_cooldown: u64,
    pub max_backoff_level: u32,
    pub breaker: BreakerConfig,
}

pub static OAUTH_PROFILE: ProviderProfile = ProviderProfile {
    transient_cooldown: 5_000,
    rate_limit_cooldown: 60_000,
    max_backoff_level: 8,
    breaker: BreakerConfig {
        failure_threshold: 8,
        reset_timeout: Duration::from_millis(60_000),
        half_open_requests: 1,
        max_backoff_multiplier: 8,
        backoff_escalation_count: 2,
    },
};

pub static APIKEY_PROFILE: ProviderProfile = ProviderProfile {
    transient_cooldown: 3_000,
    rate_limit_cooldown: 0,
    max_backoff_level: 5,
    breaker: BreakerConfig {
        failure_threshold: 12,
        reset_timeout: Duration::from_millis(30_000),
        half_open_requests: 1,
        max_backoff_multiplier: 4,
        backoff_escalation_count: 3,
    },
};

pub static LOCAL_PROFILE: ProviderProfile = ProviderProfile {
    transient_cooldown: 2_000,
    rate_limit_cooldown: 5_000,
    max_backoff_level: 3,
    breaker: BreakerConfig {
        failure_threshold: 2,
        reset_timeout: Duration::from_millis(15_000),
        half_open_requests: 1,
        max_backoff_multiplier: 4,
        backoff_escalation_count: 3,
    },
};

static PROFILE_OVERRIDES: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn set_provider_profile(provider: &str, profile: &str) -> bool {
    let norm = profile.trim().to_ascii_lowercase();
    if !matches!(norm.as_str(), "oauth" | "apikey" | "local") {
        return false;
    }
    PROFILE_OVERRIDES
        .lock()
        .unwrap()
        .insert(provider.to_string(), norm);
    true
}

pub fn profile_for(provider: &str) -> &'static ProviderProfile {
    if let Some(ov) = PROFILE_OVERRIDES.lock().unwrap().get(provider).cloned() {
        return match ov.as_str() {
            "oauth" => &OAUTH_PROFILE,
            "local" => &LOCAL_PROFILE,
            _ => &APIKEY_PROFILE,
        };
    }
    if provider.starts_with("local-")
        || provider.ends_with("-local")
        || matches!(provider, "ollama" | "lmstudio" | "vllm" | "mlx")
    {
        return &LOCAL_PROFILE;
    }
    match provider {
        "claude" | "codex" | "chatgpt" | "antigravity" | "agy" | "kimi-coding" | "copilot"
        | "cline" | "cursor" | "gemini-cli" | "colab" | "qwen-code" | "kiro" => &OAUTH_PROFILE,
        _ => &APIKEY_PROFILE,
    }
}

// ── Model lockout ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ModelLockout {
    pub provider: String,
    pub connection_id: String,
    pub model: String,
    pub reason: String,
    pub until: Instant,
    pub locked_at: Instant,
    pub failure_count: u32,
    pub reset_after_ms: u64,
}

static LOCKOUTS: LazyLock<Mutex<HashMap<String, ModelLockout>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static PROVIDER_FAILURE_STREAK: LazyLock<Mutex<HashMap<String, u32>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn lockout_key(provider: &str, conn: &str, model: &str) -> String {
    let scoped_model = crate::antigravity::quota_scoped_model(provider, model);
    format!("{provider}::{conn}::{scoped_model}")
}

fn lock_single(
    map: &mut HashMap<String, ModelLockout>,
    provider: &str,
    connection_id: &str,
    scoped_model: &str,
    reason: &str,
    cooldown_ms: u64,
    failure_count: u32,
) {
    let key = format!("{provider}::{connection_id}::{scoped_model}");
    let now = Instant::now();
    let new_until = now + Duration::from_millis(cooldown_ms);
    let entry = map.entry(key).or_insert_with(|| ModelLockout {
        provider: provider.to_string(),
        connection_id: connection_id.to_string(),
        model: scoped_model.to_string(),
        reason: reason.to_string(),
        until: new_until,
        locked_at: now,
        failure_count,
        reset_after_ms: cooldown_ms,
    });
    if entry.until < new_until {
        entry.until = new_until;
        entry.reason = reason.to_string();
        entry.reset_after_ms = cooldown_ms;
    }
    entry.failure_count = entry.failure_count.max(failure_count);
}

/// Lock provider+connection+model for cooldownMs. Longest-of wins.
/// For Antigravity/`agy`, locks are scoped by quota family (`family:gemini`, `family:claude`,
/// `family:argon`) and mirrored across both `antigravity` and `agy` for the same connection.
pub fn lock_model(
    provider: &str,
    connection_id: &str,
    model: &str,
    reason: &str,
    cooldown_ms: u64,
    failure_count: u32,
) {
    if model.is_empty() || cooldown_ms == 0 {
        return;
    }
    let scoped = crate::antigravity::quota_scoped_model(provider, model);
    let mut map = LOCKOUTS.lock().unwrap();
    if crate::antigravity::is_antigravity_provider(provider) {
        lock_single(&mut map, "antigravity", connection_id, &scoped, reason, cooldown_ms, failure_count);
        lock_single(&mut map, "agy", connection_id, &scoped, reason, cooldown_ms, failure_count);
    } else {
        lock_single(&mut map, provider, connection_id, &scoped, reason, cooldown_ms, failure_count);
    }
}

pub fn is_model_locked(provider: &str, connection_id: &str, model: &str) -> bool {
    let key = lockout_key(provider, connection_id, model);
    let mut map = LOCKOUTS.lock().unwrap();
    match map.get(&key) {
        Some(l) if l.until > Instant::now() => true,
        Some(_) => {
            map.remove(&key);
            false
        }
        None => false,
    }
}

pub fn model_lockout_remaining_ms(provider: &str, connection_id: &str, model: &str) -> Option<u64> {
    let key = lockout_key(provider, connection_id, model);
    let mut map = LOCKOUTS.lock().unwrap();
    match map.get(&key) {
        Some(l) => {
            let now = Instant::now();
            if l.until > now {
                Some((l.until - now).as_millis() as u64)
            } else {
                map.remove(&key);
                None
            }
        }
        None => None,
    }
}

pub fn get_lockout(provider: &str, connection_id: &str, model: &str) -> Option<ModelLockout> {
    let key = lockout_key(provider, connection_id, model);
    let map = LOCKOUTS.lock().unwrap();
    map.get(&key).cloned()
}

pub fn clear_model_lock(provider: &str, connection_id: &str, model: &str) {
    let mut map = LOCKOUTS.lock().unwrap();
    if crate::antigravity::is_antigravity_provider(provider) {
        let scoped = crate::antigravity::quota_scoped_model(provider, model);
        map.remove(&format!("antigravity::{connection_id}::{scoped}"));
        map.remove(&format!("agy::{connection_id}::{scoped}"));
    } else {
        map.remove(&lockout_key(provider, connection_id, model));
    }
}

pub fn clear_all_lockouts() {
    LOCKOUTS.lock().unwrap().clear();
}

pub fn all_lockouts() -> Vec<ModelLockout> {
    let mut map = LOCKOUTS.lock().unwrap();
    map.retain(|_, l| l.until > Instant::now());
    map.values().cloned().collect()
}

pub fn classify_lockout_reason(status: u16) -> &'static str {
    match status {
        401 | 403 => "auth",
        402 => "credits",
        429 => "rate_limit",
        404 | 410 => "model_unavailable",
        s if s >= 500 => "server",
        _ => "unknown",
    }
}

pub fn is_terminal_status(status: u16) -> bool {
    status == 401 || status == 402 || status == 403
}

// ── Provider & connection breaker wrappers ──────────────────────────

pub fn record_provider_failure(provider: &str) {
    let profile = profile_for(provider);
    let breaker = get_breaker(provider, profile.breaker.clone());
    let mut b = breaker.lock().unwrap();
    if b.status().state == BreakerState::HalfOpen {
        return;
    }
    if !b.can_execute() {
        return;
    }
    b.record_failure(None);
    let mut streak = PROVIDER_FAILURE_STREAK.lock().unwrap();
    *streak.entry(provider.to_string()).or_insert(0) += 1;
}

pub fn record_connection_failure(provider: &str, connection_id: &str) {
    let name = format!("{}::conn::{}", provider, connection_id);
    let profile = profile_for(provider);
    let breaker = get_breaker(&name, profile.breaker.clone());
    breaker.lock().unwrap().record_failure(None);
}

pub fn record_provider_success(provider: &str) {
    let breaker = get_breaker(provider, profile_for(provider).breaker.clone());
    breaker.lock().unwrap().record_success();
    PROVIDER_FAILURE_STREAK.lock().unwrap().remove(provider);
}

pub fn record_connection_success(provider: &str, connection_id: &str) {
    let name = format!("{}::conn::{}", provider, connection_id);
    get_breaker(&name, profile_for(provider).breaker.clone())
        .lock()
        .unwrap()
        .record_success();
}

pub fn reset_provider_breaker(provider: &str) {
    let breaker = get_breaker(provider, profile_for(provider).breaker.clone());
    breaker.lock().unwrap().reset();
    PROVIDER_FAILURE_STREAK.lock().unwrap().remove(provider);
}

pub fn is_provider_in_cooldown(provider: &str) -> bool {
    let b = get_breaker(provider, profile_for(provider).breaker.clone());
    let mut guard = b.lock().unwrap();
    !guard.can_execute()
}

pub fn is_connection_in_cooldown(provider: &str, connection_id: &str) -> bool {
    let name = format!("{}::conn::{}", provider, connection_id);
    let b = get_breaker(&name, profile_for(provider).breaker.clone());
    let mut guard = b.lock().unwrap();
    !guard.can_execute()
}

pub fn provider_cooldown_remaining_ms(provider: &str) -> Option<u64> {
    let b = get_breaker(provider, profile_for(provider).breaker.clone());
    let mut b = b.lock().unwrap();
    if b.can_execute() {
        return None;
    }
    let rem = b.retry_after_ms();
    if rem > 0 { Some(rem) } else { None }
}

pub fn provider_breaker_state(provider: &str) -> BreakerState {
    get_breaker(provider, profile_for(provider).breaker.clone())
        .lock()
        .unwrap()
        .status()
        .state
}

pub fn all_provider_states() -> Vec<(String, &'static str, u32)> {
    let streak = PROVIDER_FAILURE_STREAK.lock().unwrap().clone();
    let reg = crate::breaker::registry_snapshot();
    reg.into_iter()
        .filter(|(name, _)| !name.contains("::conn::"))
        .map(|(name, _)| {
            let state = provider_breaker_state(&name);
            let cnt = streak.get(&name).copied().unwrap_or(0);
            (name, state.as_str(), cnt)
        })
        .collect()
}

pub fn evict_lockout_overflow() -> usize {
    let mut map = LOCKOUTS.lock().unwrap();
    let before = map.len();
    map.retain(|_, l| l.until > Instant::now());
    before - map.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_lockout_scopes_never_collide() {
        // Use isolated provider key so parallel tests never interfere.
        let p = "prov-scope-iso";
        lock_model(p, "conn1", "m1", "429", 50_000, 1);
        assert!(is_model_locked(p, "conn1", "m1"));
        assert!(!is_model_locked(p, "conn2", "m1"));
        assert!(!is_model_locked(p, "conn1", "m2"));
        clear_model_lock(p, "conn1", "m1");
        assert!(!is_model_locked(p, "conn1", "m1"));
    }

    #[test]
    fn longest_lock_wins() {
        let p = "prov-longest-iso";
        lock_model(p, "c", "m", "short", 10_000, 1);
        lock_model(p, "c", "m", "long", 60_000, 2);
        let l = get_lockout(p, "c", "m").unwrap();
        assert_eq!(l.reason, "long");
        assert_eq!(l.failure_count, 2);
        clear_model_lock(p, "c", "m");
    }

    #[test]
    fn classify_reasons() {
        assert_eq!(classify_lockout_reason(429), "rate_limit");
        assert_eq!(classify_lockout_reason(401), "auth");
        assert_eq!(classify_lockout_reason(503), "server");
    }

    #[test]
    fn terminal_status_ban() {
        assert!(is_terminal_status(402));
        assert!(!is_terminal_status(429));
    }

    #[test]
    fn provider_breaker_states_flow() {
        let name = format!("test-provider-{}", rand_id());
        assert!(!is_provider_in_cooldown(&name));
        for _ in 0..APIKEY_PROFILE.breaker.failure_threshold {
            record_provider_failure(&name);
        }
        assert!(is_provider_in_cooldown(&name));
        record_provider_success(&name);
        assert!(!is_provider_in_cooldown(&name));
    }

    fn rand_id() -> u16 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .subsec_nanos() as u16
    }
}
