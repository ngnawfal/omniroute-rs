// Combo routing strategies — port of upstream OmniRoute v3.8.55
// (shared/constants/routingStrategies.ts + open-sse/services/combo).
// Covers all 22 public strategies + 1 internal strategy (`quota-share`) +
// all 11 Auto-combo routing strategies (`decision`, `nadir`, `sla-aware`, etc.),
// plus task-aware routing model scoping (`targetModels` / `excludedModels`).

use rand::seq::SliceRandom;
use rand::Rng;
use serde::{Deserialize, Serialize};

/// All 22 public strategies + 1 internal (`quota-share`) from v3.8.55.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Strategy {
    Priority,
    Weighted,
    RoundRobin,
    ContextRelay,
    FillFirst,
    P2c,
    Random,
    LeastUsed,
    CostOptimized,
    ResetAware,
    ResetWindow,
    ExpiryFirst,
    Headroom,
    QuotaWeighted,
    StrictRandom,
    Auto,
    Lkgp,
    ContextOptimized,
    CacheOptimized,
    Fusion,
    Pipeline,
    Jev,
    QuotaShare,
}

/// All 11 Auto-combo sub-strategies from v3.8.55 (`AUTO_ROUTING_STRATEGY_VALUES`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AutoRoutingStrategy {
    Rules,
    Score,
    Cost,
    Eco,
    Latency,
    Fast,
    SlaAware,
    Sla,
    Lkgp,
    Nadir,
    Decision,
}

pub const PUBLIC_STRATEGIES: &[Strategy] = &[
    Strategy::Priority,
    Strategy::Weighted,
    Strategy::RoundRobin,
    Strategy::ContextRelay,
    Strategy::FillFirst,
    Strategy::P2c,
    Strategy::Random,
    Strategy::LeastUsed,
    Strategy::CostOptimized,
    Strategy::ResetAware,
    Strategy::ResetWindow,
    Strategy::ExpiryFirst,
    Strategy::Headroom,
    Strategy::QuotaWeighted,
    Strategy::StrictRandom,
    Strategy::Auto,
    Strategy::Lkgp,
    Strategy::ContextOptimized,
    Strategy::CacheOptimized,
    Strategy::Fusion,
    Strategy::Pipeline,
    Strategy::Jev,
];

/// normalizeRoutingStrategy aliases (shared/constants/routingStrategies.ts v3.8.55).
pub fn normalize_strategy(s: &str) -> Strategy {
    match s.trim().to_ascii_lowercase().as_str() {
        "priority" => Strategy::Priority,
        "weighted" => Strategy::Weighted,
        "round-robin" | "roundrobin" | "rr" => Strategy::RoundRobin,
        "context-relay" => Strategy::ContextRelay,
        "fill-first" | "fillfirst" => Strategy::FillFirst,
        "p2c" | "power-of-two-choices" => Strategy::P2c,
        "random" => Strategy::Random,
        "least-used" | "usage" => Strategy::LeastUsed,
        "cost-optimized" | "cost" | "cost-optimized-batch" => Strategy::CostOptimized,
        "reset-aware" => Strategy::ResetAware,
        "reset-window" | "weekly-reset" | "reset-window-order" => Strategy::ResetWindow,
        "expiry-first" | "expiry" => Strategy::ExpiryFirst,
        "headroom" => Strategy::Headroom,
        "quota-weighted" => Strategy::QuotaWeighted,
        "strict-random" => Strategy::StrictRandom,
        "auto" => Strategy::Auto,
        "lkgp" | "last-known-good-provider" => Strategy::Lkgp,
        "context-optimized" | "context" => Strategy::ContextOptimized,
        "cache-optimized" | "cache-aware" | "cache-optimized-batch" => Strategy::CacheOptimized,
        "fusion" | "ensemble" | "judge" => Strategy::Fusion,
        "pipeline" | "sequential" => Strategy::Pipeline,
        "jev" | "jevons" => Strategy::Jev,
        "quota-share" => Strategy::QuotaShare,
        _ => Strategy::Priority,
    }
}

pub fn normalize_auto_strategy(s: &str) -> AutoRoutingStrategy {
    match s.trim().to_ascii_lowercase().as_str() {
        "score" => AutoRoutingStrategy::Score,
        "cost" => AutoRoutingStrategy::Cost,
        "eco" => AutoRoutingStrategy::Eco,
        "latency" => AutoRoutingStrategy::Latency,
        "fast" => AutoRoutingStrategy::Fast,
        "sla-aware" => AutoRoutingStrategy::SlaAware,
        "sla" => AutoRoutingStrategy::Sla,
        "lkgp" => AutoRoutingStrategy::Lkgp,
        "nadir" => AutoRoutingStrategy::Nadir,
        "decision" => AutoRoutingStrategy::Decision,
        _ => AutoRoutingStrategy::Rules,
    }
}

impl Strategy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Priority => "priority",
            Self::Weighted => "weighted",
            Self::RoundRobin => "round-robin",
            Self::ContextRelay => "context-relay",
            Self::FillFirst => "fill-first",
            Self::P2c => "p2c",
            Self::Random => "random",
            Self::LeastUsed => "least-used",
            Self::CostOptimized => "cost-optimized",
            Self::ResetAware => "reset-aware",
            Self::ResetWindow => "reset-window",
            Self::ExpiryFirst => "expiry-first",
            Self::Headroom => "headroom",
            Self::QuotaWeighted => "quota-weighted",
            Self::StrictRandom => "strict-random",
            Self::Auto => "auto",
            Self::Lkgp => "lkgp",
            Self::ContextOptimized => "context-optimized",
            Self::CacheOptimized => "cache-optimized",
            Self::Fusion => "fusion",
            Self::Pipeline => "pipeline",
            Self::Jev => "jev",
            Self::QuotaShare => "quota-share",
        }
    }
}

// ── Target shape ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ComboTarget {
    pub provider: String,
    pub connection_id: String,
    pub model: String,
    pub weight: f64,
    pub fallback_only: bool,
    pub cost_per_m_out: f64,
    pub quota_remaining_hint: i64,
    /// ms until quota reset; <= 0 indicates lapsed/refilled reset (v3.8.55 #19583).
    pub reset_in_ms: i64,
    pub request_count: u64,
    pub success_rate: f64,
    pub avg_latency_ms: f64,
    pub headroom_bytes: i64,
    pub context_len: i64,
    pub prompt_tokens: i64,
    pub breaker_open: bool,
}

impl ComboTarget {
    pub fn execution_key(&self) -> String {
        format!("{}::{}::{}", self.provider, self.connection_id, self.model)
    }

    pub fn qualified_model(&self) -> String {
        format!("{}/{}", self.provider, self.model)
    }
}

/// Filter candidates by task-aware routing `target_models` / `excluded_models` (v3.8.55 #20294).
pub fn filter_scoped_targets(
    targets: &[ComboTarget],
    target_models: &[String],
    excluded_models: &[String],
) -> Vec<ComboTarget> {
    targets
        .iter()
        .filter(|t| {
            let q = t.qualified_model();
            let matches_pattern = |pat: &str| {
                pat == t.model
                    || pat == q
                    || pat == t.provider
                    || pat
                        .strip_suffix('*')
                        .map(|pre| t.model.starts_with(pre) || q.starts_with(pre))
                        .unwrap_or(false)
            };
            if !target_models.is_empty() && !target_models.iter().any(|p| matches_pattern(p)) {
                return false;
            }
            if excluded_models.iter().any(|p| matches_pattern(p)) {
                return false;
            }
            true
        })
        .cloned()
        .collect()
}

/// Partition primary healthy targets ahead of `fallback_only` and `breaker_open` targets.
fn partition_health(targets: &[ComboTarget]) -> (Vec<ComboTarget>, Vec<ComboTarget>) {
    let mut primary = Vec::new();
    let mut deferred = Vec::new();
    for t in targets {
        if t.breaker_open || t.fallback_only {
            deferred.push(t.clone());
        } else {
            primary.push(t.clone());
        }
    }
    // Keep fallback_only ahead of open-breaker inside deferred tail.
    deferred.sort_by_key(|t| t.breaker_open as u8);
    (primary, deferred)
}

// ── Ordering implementations ────────────────────────────────────────

pub fn fisher_yates_shuffle(targets: &mut Vec<ComboTarget>, rng: &mut impl Rng) {
    targets.shuffle(rng);
}

pub fn order_by_weight(targets: &[ComboTarget], rng: &mut impl Rng) -> Vec<ComboTarget> {
    if targets.is_empty() {
        return vec![];
    }
    let total: f64 = targets.iter().map(|t| t.weight.max(0.0)).sum();
    if total <= f64::EPSILON {
        let mut v = targets.to_vec();
        v.shuffle(rng);
        return v;
    }
    let mut pick = rng.gen::<f64>() * total;
    let mut chosen_idx = targets.len() - 1;
    for (i, t) in targets.iter().enumerate() {
        pick -= t.weight.max(0.0);
        if pick <= 0.0 {
            chosen_idx = i;
            break;
        }
    }
    let mut out = vec![targets[chosen_idx].clone()];
    let mut rest: Vec<ComboTarget> = targets
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != chosen_idx)
        .map(|(_, t)| t.clone())
        .collect();
    rest.sort_by(|a, b| {
        b.weight
            .partial_cmp(&a.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out.extend(rest);
    out
}

fn p2c_score(t: &ComboTarget) -> f64 {
    if t.breaker_open {
        return f64::NEG_INFINITY;
    }
    let stability = t.success_rate / 100.0;
    let speed = 1.0 / (t.avg_latency_ms + 10.0).max(1.0).log10();
    stability + speed
}

pub fn order_by_p2c(targets: &[ComboTarget], rng: &mut impl Rng) -> Vec<ComboTarget> {
    if targets.len() < 2 {
        return targets.to_vec();
    }
    let i1 = rng.gen_range(0..targets.len());
    let mut i2 = rng.gen_range(0..targets.len());
    while i2 == i1 {
        i2 = rng.gen_range(0..targets.len());
    }
    let (first, second) = if p2c_score(&targets[i1]) >= p2c_score(&targets[i2]) {
        (i1, i2)
    } else {
        (i2, i1)
    };
    let mut out = vec![targets[first].clone(), targets[second].clone()];
    out.extend(
        targets
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != first && *i != second)
            .map(|(_, t)| t.clone()),
    );
    out
}

pub fn order_by_least_used(targets: &[ComboTarget]) -> Vec<ComboTarget> {
    let mut v = targets.to_vec();
    v.sort_by_key(|t| t.request_count);
    v
}

pub fn order_by_cost(targets: &[ComboTarget]) -> Vec<ComboTarget> {
    let mut v = targets.to_vec();
    v.sort_by(|a, b| {
        a.cost_per_m_out
            .partial_cmp(&b.cost_per_m_out)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    v
}

pub fn order_by_reset_aware(targets: &[ComboTarget]) -> Vec<ComboTarget> {
    let mut v = targets.to_vec();
    v.sort_by_key(|t| t.reset_in_ms);
    v
}

pub fn order_by_reset_window(targets: &[ComboTarget]) -> Vec<ComboTarget> {
    order_by_reset_aware(targets)
}

/// expiry-first (v3.8.54 + v3.8.55 #19583):
/// Active quotas with soonest positive deadline go first so expiring quota is used
/// before renewal; lapsed reset (reset_in_ms <= 0) is treated as refilled (fresh)
/// rather than "expires now", placing it after expiring active deadlines.
pub fn order_by_expiry_first(targets: &[ComboTarget]) -> Vec<ComboTarget> {
    let mut v = targets.to_vec();
    v.sort_by_key(|t| {
        if t.reset_in_ms > 0 {
            (0u8, t.reset_in_ms)
        } else {
            (1u8, i64::MAX)
        }
    });
    v
}

pub fn order_by_headroom(targets: &[ComboTarget]) -> Vec<ComboTarget> {
    let mut v = targets.to_vec();
    v.sort_by(|a, b| b.quota_remaining_hint.cmp(&a.quota_remaining_hint));
    v
}

pub fn order_by_quota_weighted(targets: &[ComboTarget], rng: &mut impl Rng) -> Vec<ComboTarget> {
    let mut weighted: Vec<ComboTarget> = targets.to_vec();
    for t in weighted.iter_mut() {
        if t.weight <= 0.0 {
            t.weight = (t.quota_remaining_hint.max(0) as f64).max(1.0);
        }
    }
    order_by_weight(&weighted, rng)
}

pub fn order_by_context(targets: &[ComboTarget]) -> Vec<ComboTarget> {
    let mut v = targets.to_vec();
    v.sort_by_key(|t| {
        let slack = t.context_len - t.prompt_tokens;
        std::cmp::Reverse(if slack >= 0 { slack } else { i64::MIN / 2 })
    });
    v
}

/// fill-first: keep declared order for targets with remaining quota; move depleted
/// targets (`quota_remaining_hint == 0` when others have positive quota) to tail.
pub fn order_by_fill_first(targets: &[ComboTarget]) -> Vec<ComboTarget> {
    let any_quota_tracked = targets.iter().any(|t| t.quota_remaining_hint > 0);
    if !any_quota_tracked {
        return targets.to_vec();
    }
    let mut available = Vec::new();
    let mut depleted = Vec::new();
    for t in targets {
        if t.quota_remaining_hint > 0 {
            available.push(t.clone());
        } else {
            depleted.push(t.clone());
        }
    }
    available.extend(depleted);
    available
}

/// jev (v3.8.54): efficiency/value score combining reliability, speed, headroom, and low cost.
pub fn order_by_jev(targets: &[ComboTarget]) -> Vec<ComboTarget> {
    let mut v = targets.to_vec();
    v.sort_by(|a, b| {
        let score = |t: &ComboTarget| -> f64 {
            if t.breaker_open {
                return f64::NEG_INFINITY;
            }
            let reliability = (t.success_rate / 100.0).clamp(0.0, 1.0);
            let cost_factor = 1.0 / (1.0 + t.cost_per_m_out.max(0.0));
            let speed_factor = 1.0 / (t.avg_latency_ms.max(0.0) + 50.0).log10();
            let headroom_bonus = ((t.quota_remaining_hint.max(0) as f64) + 1.0).log10() * 0.1;
            reliability * 2.0 + cost_factor * 1.5 + speed_factor + headroom_bonus
        };
        score(b)
            .partial_cmp(&score(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    v
}

/// auto / score / decision composite ordering.
pub fn order_by_auto(
    auto_strategy: AutoRoutingStrategy,
    targets: &[ComboTarget],
    last_good: Option<&str>,
) -> Vec<ComboTarget> {
    match auto_strategy {
        AutoRoutingStrategy::Cost | AutoRoutingStrategy::Eco => order_by_cost(targets),
        AutoRoutingStrategy::Latency | AutoRoutingStrategy::Fast => {
            let mut v = targets.to_vec();
            v.sort_by(|a, b| {
                a.avg_latency_ms
                    .partial_cmp(&b.avg_latency_ms)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            v
        }
        AutoRoutingStrategy::Sla | AutoRoutingStrategy::SlaAware => {
            let mut v = targets.to_vec();
            v.sort_by(|a, b| {
                b.success_rate
                    .partial_cmp(&a.success_rate)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            v
        }
        AutoRoutingStrategy::Lkgp => order_by_lkgp(targets, last_good),
        AutoRoutingStrategy::Nadir => order_by_least_used(targets),
        AutoRoutingStrategy::Score | AutoRoutingStrategy::Decision | AutoRoutingStrategy::Rules => {
            order_by_jev(targets)
        }
    }
}

/// cache-optimized: sticky to `last_good` (prefix cache affinity) when available,
/// else declared order.
pub fn order_by_cache(targets: &[ComboTarget], last_good: Option<&str>) -> Vec<ComboTarget> {
    order_by_lkgp(targets, last_good)
}

/// context-relay: prioritize targets that fit prompt with headroom, preferring sticky relay target.
pub fn order_by_context_relay(targets: &[ComboTarget], last_good: Option<&str>) -> Vec<ComboTarget> {
    let mut fitting: Vec<ComboTarget> = targets
        .iter()
        .filter(|t| t.context_len >= t.prompt_tokens)
        .cloned()
        .collect();
    let overflow: Vec<ComboTarget> = targets
        .iter()
        .filter(|t| t.context_len < t.prompt_tokens)
        .cloned()
        .collect();
    if let Some(key) = last_good {
        fitting = order_by_lkgp(&fitting, Some(key));
    }
    fitting.extend(overflow);
    fitting
}

pub fn order_by_strict_random(targets: &[ComboTarget], rng: &mut impl Rng) -> Vec<ComboTarget> {
    let mut v = targets.to_vec();
    v.shuffle(rng);
    v
}

pub fn order_by_lkgp(targets: &[ComboTarget], last_good: Option<&str>) -> Vec<ComboTarget> {
    match last_good {
        None => targets.to_vec(),
        Some(key) => {
            let mut out: Vec<ComboTarget> = targets
                .iter()
                .filter(|t| t.execution_key() == key)
                .cloned()
                .collect();
            out.extend(
                targets
                    .iter()
                    .filter(|t| t.execution_key() != key)
                    .cloned(),
            );
            out
        }
    }
}

/// Top-level dispatcher: strategy → full ordered target list, keeping healthy primary
/// targets ahead of `fallback_only` / `breaker_open` targets (except `strict-random`).
pub fn order_targets(
    strategy: Strategy,
    targets: &[ComboTarget],
    rng: &mut impl Rng,
    rr_counter: Option<&mut u64>,
    last_good: Option<&str>,
) -> Vec<ComboTarget> {
    order_targets_with_auto(strategy, AutoRoutingStrategy::Rules, targets, rng, rr_counter, last_good)
}

pub fn order_targets_with_auto(
    strategy: Strategy,
    auto_strategy: AutoRoutingStrategy,
    targets: &[ComboTarget],
    rng: &mut impl Rng,
    rr_counter: Option<&mut u64>,
    last_good: Option<&str>,
) -> Vec<ComboTarget> {
    if targets.is_empty() {
        return vec![];
    }
    if strategy == Strategy::StrictRandom {
        return order_by_strict_random(targets, rng);
    }

    let (primary, deferred) = partition_health(targets);
    let pool = if primary.is_empty() { &deferred } else { &primary };

    let mut ordered = match strategy {
        Strategy::Priority | Strategy::Pipeline | Strategy::Fusion => pool.to_vec(),
        Strategy::FillFirst => order_by_fill_first(pool),
        Strategy::ContextRelay => order_by_context_relay(pool, last_good),
        Strategy::CacheOptimized => order_by_cache(pool, last_good),
        Strategy::Auto => order_by_auto(auto_strategy, pool, last_good),
        Strategy::RoundRobin => {
            let n = pool.len();
            let mut fallback_counter: u64 = 0;
            let counter_state = rr_counter.unwrap_or(&mut fallback_counter);
            let start = (*counter_state as usize) % n;
            let mut out: Vec<ComboTarget> = Vec::with_capacity(n);
            for i in 0..n {
                out.push(pool[(start + i) % n].clone());
            }
            *counter_state = ((*counter_state) + 1) % n.max(1) as u64;
            out
        }
        Strategy::Weighted => order_by_weight(pool, rng),
        Strategy::QuotaWeighted | Strategy::QuotaShare => order_by_quota_weighted(pool, rng),
        Strategy::P2c => order_by_p2c(pool, rng),
        Strategy::Random => {
            let mut v = pool.to_vec();
            v.shuffle(rng);
            v
        }
        Strategy::StrictRandom => unreachable!(),
        Strategy::LeastUsed => order_by_least_used(pool),
        Strategy::CostOptimized => order_by_cost(pool),
        Strategy::ResetAware => order_by_reset_aware(pool),
        Strategy::ResetWindow => order_by_reset_window(pool),
        Strategy::ExpiryFirst => order_by_expiry_first(pool),
        Strategy::Headroom => order_by_headroom(pool),
        Strategy::Lkgp => order_by_lkgp(pool, last_good),
        Strategy::ContextOptimized => order_by_context(pool),
        Strategy::Jev => order_by_jev(pool),
    };

    if !primary.is_empty() {
        ordered.extend(deferred);
    }
    ordered
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    fn t(id: &str, weight: f64) -> ComboTarget {
        ComboTarget {
            provider: id.to_string(),
            connection_id: "c".into(),
            model: "m".into(),
            weight,
            fallback_only: false,
            cost_per_m_out: 0.0,
            quota_remaining_hint: 0,
            reset_in_ms: 0,
            request_count: 0,
            success_rate: 100.0,
            avg_latency_ms: 100.0,
            headroom_bytes: 0,
            context_len: 100_000,
            prompt_tokens: 1_000,
            breaker_open: false,
        }
    }

    fn targets() -> Vec<ComboTarget> {
        vec![t("a", 1.0), t("b", 3.0), t("c", 6.0)]
    }

    #[test]
    fn normalize_v3855_aliases() {
        assert_eq!(PUBLIC_STRATEGIES.len(), 22);
        assert_eq!(normalize_strategy("usage"), Strategy::LeastUsed);
        assert_eq!(normalize_strategy("context"), Strategy::ContextOptimized);
        assert_eq!(normalize_strategy("weekly-reset"), Strategy::ResetWindow);
        assert_eq!(normalize_strategy("expiry-first"), Strategy::ExpiryFirst);
        assert_eq!(normalize_strategy("jev"), Strategy::Jev);
        assert_eq!(normalize_strategy("quota-share"), Strategy::QuotaShare);
        assert_eq!(normalize_strategy("nonsense"), Strategy::Priority);
        assert_eq!(normalize_auto_strategy("decision"), AutoRoutingStrategy::Decision);
        assert_eq!(normalize_auto_strategy("sla-aware"), AutoRoutingStrategy::SlaAware);
    }

    #[test]
    fn expiry_first_treats_lapsed_as_refilled() {
        let mut ts = targets();
        ts[0].reset_in_ms = -5000; // lapsed (refilled)
        ts[1].reset_in_ms = 60_000; // expires in 1m
        ts[2].reset_in_ms = 3_600_000; // expires in 1h
        let ordered = order_by_expiry_first(&ts);
        assert_eq!(ordered[0].provider, "b");
        assert_eq!(ordered[1].provider, "c");
        assert_eq!(ordered[2].provider, "a");
    }

    #[test]
    fn jev_prefers_cheap_fast_reliable() {
        let mut ts = targets();
        ts[0].cost_per_m_out = 15.0;
        ts[0].avg_latency_ms = 900.0;
        ts[1].cost_per_m_out = 0.2;
        ts[1].avg_latency_ms = 80.0;
        ts[2].cost_per_m_out = 5.0;
        ts[2].avg_latency_ms = 400.0;
        let ordered = order_by_jev(&ts);
        assert_eq!(ordered[0].provider, "b");
    }

    #[test]
    fn scoped_target_filtering_works() {
        let ts = targets();
        let filtered = filter_scoped_targets(&ts, &["a/m".into(), "b".into()], &["b".into()]);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].provider, "a");
    }

    #[test]
    fn weighted_draw_respects_weights() {
        let mut rng = StdRng::seed_from_u64(42);
        let mut wins = std::collections::HashMap::new();
        for _ in 0..300 {
            let ordered = order_by_weight(&targets(), &mut rng);
            *wins.entry(ordered[0].provider.clone()).or_insert(0u32) += 1;
        }
        let a = *wins.get("a").unwrap_or(&0);
        let c = *wins.get("c").unwrap_or(&0);
        assert!(c > a * 3, "c={} a={}", c, a);
    }

    #[test]
    fn round_robin_rotates() {
        let ts = targets();
        let mut counter: u64 = 0;
        let mut rng = StdRng::seed_from_u64(1);
        let first = order_targets(Strategy::RoundRobin, &ts, &mut rng, Some(&mut counter), None);
        assert_eq!(first[0].provider, "a");
        let second = order_targets(Strategy::RoundRobin, &ts, &mut rng, Some(&mut counter), None);
        assert_eq!(second[0].provider, "b");
        let third = order_targets(Strategy::RoundRobin, &ts, &mut rng, Some(&mut counter), None);
        assert_eq!(third[0].provider, "c");
        let fourth = order_targets(Strategy::RoundRobin, &ts, &mut rng, Some(&mut counter), None);
        assert_eq!(fourth[0].provider, "a");
    }

    #[test]
    fn p2c_rejects_open_breaker() {
        let mut ts = targets();
        ts[1].success_rate = 99.0;
        ts[1].breaker_open = true;
        ts[2].success_rate = 40.0;
        let ordered = order_targets(Strategy::P2c, &ts, &mut StdRng::seed_from_u64(3), None, None);
        assert_ne!(ordered[0].provider, "b");
        assert_eq!(ordered[2].provider, "b");
    }

    #[test]
    fn all_23_strategies_covered() {
        let mut rng = StdRng::seed_from_u64(5);
        for &s in PUBLIC_STRATEGIES.iter().chain(std::iter::once(&Strategy::QuotaShare)) {
            let out = order_targets(s, &targets(), &mut rng, None, None);
            assert_eq!(out.len(), 3, "{:?}", s);
        }
    }
}
