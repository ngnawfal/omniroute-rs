// Circuit breaker — port of repo/src/shared/utils/circuitBreaker.ts (v2.0).
// States CLOSED → DEGRADED → OPEN → HALF_OPEN → CLOSED, adaptive backoff via
// open-cycle escalation, lazy OPEN→HALF_OPEN refresh (no background timers).
// DB persistence omitted; in-memory registry keyed by name.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerState {
    Closed,
    Degraded,
    Open,
    HalfOpen,
}

impl BreakerState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Closed => "CLOSED",
            Self::Degraded => "DEGRADED",
            Self::Open => "OPEN",
            Self::HalfOpen => "HALF_OPEN",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BreakerConfig {
    pub failure_threshold: u32,
    pub reset_timeout: Duration,
    pub half_open_requests: u32,
    pub max_backoff_multiplier: u32,
    pub backoff_escalation_count: u32,
}

impl Default for BreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            reset_timeout: Duration::from_secs(30),
            half_open_requests: 1,
            max_backoff_multiplier: 16,
            backoff_escalation_count: 3,
        }
    }
}

impl BreakerConfig {
    /// Degradation threshold = ceil(failureThreshold * 60 / 100) unless overridden.
    pub fn degradation_threshold(&self) -> u32 {
        self.failure_threshold.saturating_mul(6).div_ceil(10)
    }
}

const MAX_TRANSITION_HISTORY: usize = 20;

#[derive(Debug, Clone)]
pub struct TransitionRecord {
    pub from: BreakerState,
    pub to: BreakerState,
    pub at: SystemTime,
    pub failure_count: u32,
    pub reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BreakerStatus {
    pub name: String,
    pub state: BreakerState,
    pub failure_count: u32,
    pub retry_after_ms: u64,
    pub open_cycle_count: u32,
    pub effective_reset_timeout: Duration,
    pub transition_history: Vec<TransitionRecord>,
}

pub struct CircuitBreaker {
    pub name: String,
    pub config: BreakerConfig,
    state: BreakerState,
    failure_count: u32,
    success_count: u32,
    last_failure_time: Option<Instant>,
    half_open_allowed: u32,
    half_open_probe_started_at: Option<Instant>,
    open_cycle_count: u32,
    transition_history: Vec<TransitionRecord>,
}

impl CircuitBreaker {
    pub fn new(name: &str, config: BreakerConfig) -> Self {
        Self {
            name: name.to_string(),
            config,
            state: BreakerState::Closed,
            failure_count: 0,
            success_count: 0,
            last_failure_time: None,
            half_open_allowed: 0,
            half_open_probe_started_at: None,
            open_cycle_count: 0,
            transition_history: Vec::new(),
        }
    }

    /// Steady-state recovery: one success heals one failure (gradual), like TS.
    pub fn record_success(&mut self) {
        match self.state {
            BreakerState::Open => {
                self.transition(BreakerState::Closed, Some("success-recovery".into()));
                self.failure_count = 0;
                self.success_count = 0;
                self.last_failure_time = None;
                self.open_cycle_count = 0;
            }
            BreakerState::HalfOpen => {
                self.success_count += 1;
                self.transition(BreakerState::Closed, Some("probe-success".into()));
                self.failure_count = 0;
                self.open_cycle_count = 0;
            }
            _ => {
                self.failure_count = self.failure_count.saturating_sub(1);
                if self.state == BreakerState::Degraded
                    && self.failure_count <= self.config.degradation_threshold()
                {
                    self.transition(BreakerState::Closed, Some("recovery".into()));
                }
            }
        }
    }

    pub fn record_failure(&mut self, reason: Option<String>) {
        self.decay_stale_degradation();
        self.failure_count += 1;
        self.last_failure_time = Some(Instant::now());

        match self.state {
            BreakerState::Open => {} // already open; persistence only in TS
            BreakerState::HalfOpen => {
                self.open_cycle_count += 1;
                self.transition(
                    BreakerState::Open,
                    Some(format!("probe-failed (cycle {})", self.open_cycle_count)),
                );
            }
            BreakerState::Degraded => {
                if self.failure_count >= self.config.failure_threshold {
                    self.transition(BreakerState::Open, reason);
                }
            }
            BreakerState::Closed => {
                if self.failure_count >= self.config.failure_threshold {
                    self.transition(BreakerState::Open, reason);
                } else if self.failure_count >= self.config.degradation_threshold() {
                    self.transition(
                        BreakerState::Degraded,
                        Some(format!(
                            "elevated-failures ({}/{}): {}",
                            self.failure_count,
                            self.config.failure_threshold,
                            reason.unwrap_or_default()
                        )),
                    );
                }
            }
        }
    }

    pub fn can_execute(&mut self) -> bool {
        self.refresh_open_state();
        match self.state {
            BreakerState::Closed | BreakerState::Degraded => true,
            BreakerState::Open => false,
            BreakerState::HalfOpen => self.half_open_allowed > 0,
        }
    }

    /// Acquire one probe slot when HALF_OPEN. Returns false if none left.
    pub fn acquire_probe(&mut self) -> bool {
        self.refresh_open_state();
        if self.state == BreakerState::HalfOpen && self.half_open_allowed > 0 {
            self.half_open_allowed -= 1;
            if self.half_open_probe_started_at.is_none() {
                self.half_open_probe_started_at = Some(Instant::now());
            }
            true
        } else {
            false
        }
    }

    pub fn status(&mut self) -> BreakerStatus {
        self.refresh_open_state();
        BreakerStatus {
            name: self.name.clone(),
            state: self.state,
            failure_count: self.failure_count,
            retry_after_ms: self.time_until_reset(),
            open_cycle_count: self.open_cycle_count,
            effective_reset_timeout: self.effective_reset_timeout(),
            transition_history: self.transition_history.clone(),
        }
    }

    pub fn retry_after_ms(&mut self) -> u64 {
        self.refresh_open_state();
        match self.state {
            BreakerState::Closed | BreakerState::Degraded => 0,
            _ => self.time_until_reset(),
        }
    }

    pub fn reset(&mut self) {
        self.transition(BreakerState::Closed, Some("manual-reset".into()));
        self.failure_count = 0;
        self.success_count = 0;
        self.last_failure_time = None;
        self.open_cycle_count = 0;
    }

    pub fn close_degraded(&mut self, reason: &str) -> bool {
        if self.state != BreakerState::Degraded {
            return false;
        }
        self.transition(BreakerState::Closed, Some(reason.to_string()));
        self.failure_count = 0;
        true
    }

    // ── internals ──

    /// Effective reset timeout, escalated by open cycle count (2^ per cycle
    /// beyond backoffEscalationCount, capped at maxBackoffMultiplier× base).
    fn effective_reset_timeout(&self) -> Duration {
        if self.open_cycle_count <= self.config.backoff_escalation_count {
            return self.config.reset_timeout;
        }
        let extra = self.open_cycle_count - self.config.backoff_escalation_count;
        let factor = 2u32.saturating_pow(extra.min(16));
        let capped = factor.min(self.config.max_backoff_multiplier);
        self.config.reset_timeout.saturating_mul(capped)
    }

    fn effective_cooldown(&self) -> Duration {
        self.effective_reset_timeout()
    }

    fn time_until_reset(&self) -> u64 {
        if self.state == BreakerState::HalfOpen {
            if let Some(start) = self.half_open_probe_started_at {
                let elapsed = start.elapsed();
                return self.config.reset_timeout.saturating_sub(elapsed).as_millis() as u64;
            }
            return 0;
        }
        match self.last_failure_time {
            None => 0,
            Some(t) => {
                let elapsed = t.elapsed();
                self.effective_cooldown().saturating_sub(elapsed).as_millis() as u64
            }
        }
    }

    fn should_attempt_reset(&self) -> bool {
        match self.last_failure_time {
            None => true,
            Some(t) => t.elapsed() >= self.effective_cooldown(),
        }
    }

    /// DEGRADED decays after the longest OPEN hold for the same config:
    /// cooldown × maxBackoffMultiplier (never less than one cooldown).
    fn decay_stale_degradation(&mut self) {
        if self.state != BreakerState::Degraded {
            return;
        }
        let quiet = self
            .effective_reset_timeout()
            .saturating_mul(self.config.max_backoff_multiplier.max(1));
        let stale = match self.last_failure_time {
            None => true,
            Some(t) => t.elapsed() >= quiet,
        };
        if stale {
            self.close_degraded("failure-decay");
        }
    }

    fn refresh_open_state(&mut self) {
        self.decay_stale_degradation();
        if self.state == BreakerState::Open && self.should_attempt_reset() {
            self.transition(BreakerState::HalfOpen, Some("timeout-elapsed".into()));
        } else if self.state == BreakerState::HalfOpen
            && self.half_open_allowed == 0
            && self
                .half_open_probe_started_at
                .map(|t| t.elapsed() >= self.config.reset_timeout)
                .unwrap_or(false)
        {
            self.half_open_allowed = self.config.half_open_requests;
            self.half_open_probe_started_at = None;
        }
    }

    fn transition(&mut self, to: BreakerState, reason: Option<String>) {
        let from = self.state;
        self.state = to;
        if to == BreakerState::HalfOpen {
            self.half_open_allowed = self.config.half_open_requests;
        }
        self.half_open_probe_started_at = None;
        self.transition_history.push(TransitionRecord {
            from,
            to,
            at: SystemTime::now(),
            failure_count: self.failure_count,
            reason,
        });
        if self.transition_history.len() > MAX_TRANSITION_HISTORY {
            self.transition_history.remove(0);
        }
    }
}

// ── Process-wide registry ───────────────────────────────────────────

const MAX_REGISTRY_SIZE: usize = 500;

static REGISTRY: LazyLock<Mutex<HashMap<String, Arc<Mutex<CircuitBreaker>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn registry() -> &'static Mutex<HashMap<String, Arc<Mutex<CircuitBreaker>>>> {
    &REGISTRY
}

/// Get-or-create a shared breaker by name (provider id or provider::conn::id).
pub fn get_breaker(name: &str, config: BreakerConfig) -> Arc<Mutex<CircuitBreaker>> {
    let mut map = registry().lock().unwrap();
    if map.len() >= MAX_REGISTRY_SIZE && !map.contains_key(name) {
        // Sweep closed, idle breakers before refusing.
        map.retain(|_, arc| {
            let inner = arc.lock().unwrap();
            !(inner.state == BreakerState::Closed && inner.failure_count == 0)
        });
    }
    map.entry(name.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(CircuitBreaker::new(name, config))))
        .clone()
}

/// Snapshot of all registered breaker names (for status/inspection).
pub fn registry_snapshot() -> Vec<(String, BreakerState)> {
    let map = registry().lock().unwrap();
    map.iter()
        .map(|(k, v)| (k.clone(), v.lock().unwrap().state))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(threshold: u32, reset_ms: u64) -> BreakerConfig {
        BreakerConfig {
            failure_threshold: threshold,
            reset_timeout: Duration::from_millis(reset_ms),
            half_open_requests: 1,
            max_backoff_multiplier: 16,
            backoff_escalation_count: 3,
        }
    }

    #[test]
    fn closed_to_degraded_to_open() {
        let mut b = CircuitBreaker::new("p", cfg(5, 30_000));
        assert_eq!(b.config.degradation_threshold(), 3);
        for _ in 0..3 {
            b.record_failure(Some("e".into()));
        }
        assert_eq!(b.status().state, BreakerState::Degraded);
        b.record_failure(Some("e".into()));
        b.record_failure(Some("e".into()));
        assert_eq!(b.status().state, BreakerState::Open);
        assert!(!b.can_execute());
        assert!(b.retry_after_ms() > 0);
    }

    #[test]
    fn open_to_halfopen_to_closed() {
        let mut b = CircuitBreaker::new("p2", cfg(1, 10));
        b.record_failure(Some("e".into()));
        assert_eq!(b.status().state, BreakerState::Open);
        std::thread::sleep(Duration::from_millis(15));
        // Lazy refresh flips OPEN → HALF_OPEN inside status(); probe allowed.
        assert!(b.acquire_probe());
        assert_eq!(b.state, BreakerState::HalfOpen);
        b.record_success();
        assert_eq!(b.status().state, BreakerState::Closed);
    }

    #[test]
    fn probe_fail_escalates() {
        let mut b = CircuitBreaker::new("p3", cfg(1, 5));
        b.record_failure(Some("e".into()));
        std::thread::sleep(Duration::from_millis(10));
        b.refresh_open_state();
        assert_eq!(b.state, BreakerState::HalfOpen);
        b.acquire_probe();
        b.record_failure(Some("e".into()));
        assert_eq!(b.status().state, BreakerState::Open);
        assert_eq!(b.open_cycle_count, 1);
        // Escalation kicks in past backoff_escalation_count
        let esc = b.effective_reset_timeout();
        assert_eq!(esc, b.config.reset_timeout); // cycle 1 ≤ escalation count 3
    }

    #[test]
    fn degraded_decay_after_quiet_window() {
        let mut b = CircuitBreaker::new(
            "p4",
            BreakerConfig {
                failure_threshold: 5,
                reset_timeout: Duration::from_millis(20),
                half_open_requests: 1,
                max_backoff_multiplier: 2,
                backoff_escalation_count: 3,
            },
        );
        for _ in 0..3 {
            b.record_failure(None);
        }
        assert_eq!(b.state, BreakerState::Degraded);
        std::thread::sleep(Duration::from_millis(50)); // quiet = 20ms × 2 = 40ms
        b.record_failure(None); // decay check fires first: closes, count restarts
        assert_eq!(b.state, BreakerState::Closed);
        assert_eq!(b.failure_count, 1);
    }

    #[test]
    fn registry_shared_instance() {
        let a = get_breaker("shared-test", cfg(5, 1000));
        let b2 = get_breaker("shared-test", cfg(5, 1000));
        assert!(std::sync::Arc::ptr_eq(&a, &b2));
    }
}
