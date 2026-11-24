// omni-cache — Dual-layer exact + lightweight semantic response cache.
// Port of open-sse/config/semanticCacheConfig.ts + src/lib/semanticCache.ts
// designed for zero ML-runtime RAM bloat:
//   - Layer 1: SHA-256 exact normalized prompt hash (O(1))
//   - Layer 2: Sparse word + 3-gram TF cosine similarity search (isolated by api_key_hash + model)
//   - Bounded max_entries with LRU eviction + configurable TTL + atomic hit/miss/tokens-saved stats.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticCacheConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_threshold")]
    pub similarity_threshold: f64,
    #[serde(default = "default_ttl_ms")]
    pub ttl_ms: u64,
    #[serde(default = "default_max_entries")]
    pub max_entries: usize,
    #[serde(default = "default_true")]
    pub cache_by_model: bool,
    #[serde(default = "default_true")]
    pub require_zero_temperature: bool,
    #[serde(default = "default_history_depth")]
    pub conversation_history_depth: usize,
    #[serde(default = "default_history_threshold")]
    pub conversation_history_threshold: usize,
    #[serde(default)]
    pub exclude_system_prompt: bool,
}

fn default_threshold() -> f64 { 0.85 }
fn default_ttl_ms() -> u64 { 1_800_000 } // 30 minutes
fn default_max_entries() -> usize { 1_000 }
fn default_true() -> bool { true }
fn default_history_depth() -> usize { 3 }
fn default_history_threshold() -> usize { 50 }

impl Default for SemanticCacheConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            similarity_threshold: default_threshold(),
            ttl_ms: default_ttl_ms(),
            max_entries: default_max_entries(),
            cache_by_model: true,
            require_zero_temperature: true,
            conversation_history_depth: default_history_depth(),
            conversation_history_threshold: default_history_threshold(),
            exclude_system_prompt: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CacheHitKind {
    Direct,
    Semantic,
}

impl CacheHitKind {
    pub fn header_value(&self) -> &'static str {
        match self {
            Self::Direct => "HIT-DIRECT",
            Self::Semantic => "HIT-SEMANTIC",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CacheLookupHit {
    pub kind: CacheHitKind,
    pub similarity: f64,
    pub response: Value,
    pub provider: String,
    pub saved_tokens: i64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CacheStats {
    pub entries: usize,
    pub direct_hits: u64,
    pub semantic_hits: u64,
    pub misses: u64,
    pub stores: u64,
    pub evictions: u64,
    pub tokens_saved: i64,
    pub hit_rate: f64,
}

struct CacheEntry {
    scope_key: String,
    vector: HashMap<String, f64>,
    norm: f64,
    response: Value,
    provider: String,
    total_tokens: i64,
    expires_at: Instant,
    last_used: Instant,
}

struct CacheInner {
    by_exact: HashMap<String, CacheEntry>,
    direct_hits: u64,
    semantic_hits: u64,
    misses: u64,
    stores: u64,
    evictions: u64,
    tokens_saved: i64,
}

pub struct SemanticCache {
    inner: Mutex<CacheInner>,
}

impl Default for SemanticCache {
    fn default() -> Self {
        Self::new()
    }
}

impl SemanticCache {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(CacheInner {
                by_exact: HashMap::new(),
                direct_hits: 0,
                semantic_hits: 0,
                misses: 0,
                stores: 0,
                evictions: 0,
                tokens_saved: 0,
            }),
        }
    }

    pub fn lookup(
        &self,
        cfg: &SemanticCacheConfig,
        api_key_hash: &str,
        model: &str,
        normalized_prompt: &str,
    ) -> Option<CacheLookupHit> {
        if !cfg.enabled || normalized_prompt.is_empty() {
            return None;
        }
        let scope = make_scope(api_key_hash, model, cfg.cache_by_model);
        let exact_key = sha256_hex(&format!("{scope}::{normalized_prompt}"));
        let now = Instant::now();

        let mut guard = self.inner.lock().unwrap();
        // Lazy prune expired entries
        guard.by_exact.retain(|_, e| e.expires_at > now);

        // Layer 1: Direct SHA-256 hit
        if let Some(entry) = guard.by_exact.get_mut(&exact_key) {
            entry.last_used = now;
            let hit = CacheLookupHit {
                kind: CacheHitKind::Direct,
                similarity: 1.0,
                response: entry.response.clone(),
                provider: entry.provider.clone(),
                saved_tokens: entry.total_tokens,
            };
            guard.direct_hits += 1;
            guard.tokens_saved += hit.saved_tokens;
            return Some(hit);
        }

        // Layer 2: Sparse word + 3-gram cosine similarity within same isolation scope
        let (query_vec, query_norm) = build_sparse_vector(normalized_prompt);
        if query_norm > 0.0 && cfg.similarity_threshold < 1.0 {
            let mut best_key: Option<String> = None;
            let mut best_sim: f64 = 0.0;
            for (k, entry) in &guard.by_exact {
                if entry.scope_key != scope || entry.norm <= 0.0 {
                    continue;
                }
                let sim = cosine_similarity(&query_vec, query_norm, &entry.vector, entry.norm);
                if sim >= cfg.similarity_threshold && sim > best_sim {
                    best_sim = sim;
                    best_key = Some(k.clone());
                }
            }
            if let Some(k) = best_key {
                if let Some(entry) = guard.by_exact.get_mut(&k) {
                    entry.last_used = now;
                    let hit = CacheLookupHit {
                        kind: CacheHitKind::Semantic,
                        similarity: best_sim,
                        response: entry.response.clone(),
                        provider: entry.provider.clone(),
                        saved_tokens: entry.total_tokens,
                    };
                    guard.semantic_hits += 1;
                    guard.tokens_saved += hit.saved_tokens;
                    return Some(hit);
                }
            }
        }

        guard.misses += 1;
        None
    }

    pub fn store(
        &self,
        cfg: &SemanticCacheConfig,
        api_key_hash: &str,
        model: &str,
        normalized_prompt: &str,
        provider: &str,
        response: Value,
        total_tokens: i64,
    ) {
        if !cfg.enabled || normalized_prompt.is_empty() || cfg.max_entries == 0 {
            return;
        }
        let scope = make_scope(api_key_hash, model, cfg.cache_by_model);
        let exact_key = sha256_hex(&format!("{scope}::{normalized_prompt}"));
        let (vector, norm) = build_sparse_vector(normalized_prompt);
        let now = Instant::now();
        let expires_at = now + Duration::from_millis(cfg.ttl_ms.max(1_000));

        let mut guard = self.inner.lock().unwrap();
        guard.by_exact.retain(|_, e| e.expires_at > now);

        while guard.by_exact.len() >= cfg.max_entries && !guard.by_exact.contains_key(&exact_key) {
            if let Some(lru_key) = guard
                .by_exact
                .iter()
                .min_by_key(|(_, e)| e.last_used)
                .map(|(k, _)| k.clone())
            {
                guard.by_exact.remove(&lru_key);
                guard.evictions += 1;
            } else {
                break;
            }
        }

        guard.by_exact.insert(
            exact_key,
            CacheEntry {
                scope_key: scope,
                vector,
                norm,
                response,
                provider: provider.to_string(),
                total_tokens: total_tokens.max(0),
                expires_at,
                last_used: now,
            },
        );
        guard.stores += 1;
    }

    pub fn clear(&self) -> usize {
        let mut guard = self.inner.lock().unwrap();
        let n = guard.by_exact.len();
        guard.by_exact.clear();
        n
    }

    pub fn stats(&self) -> CacheStats {
        let now = Instant::now();
        let mut guard = self.inner.lock().unwrap();
        guard.by_exact.retain(|_, e| e.expires_at > now);
        let hits = guard.direct_hits + guard.semantic_hits;
        let total = hits + guard.misses;
        CacheStats {
            entries: guard.by_exact.len(),
            direct_hits: guard.direct_hits,
            semantic_hits: guard.semantic_hits,
            misses: guard.misses,
            stores: guard.stores,
            evictions: guard.evictions,
            tokens_saved: guard.tokens_saved,
            hit_rate: if total > 0 {
                (hits as f64 / total as f64) * 100.0
            } else {
                0.0
            },
        }
    }
}

/// Normalize chat messages into a canonical prompt string respecting depth & exclude_system_prompt.
pub fn normalize_messages_for_cache(
    messages: &[(String, String)],
    cfg: &SemanticCacheConfig,
) -> Option<String> {
    if messages.is_empty() || messages.len() > cfg.conversation_history_threshold {
        return None;
    }
    let filtered: Vec<&(String, String)> = messages
        .iter()
        .filter(|(role, _)| !(cfg.exclude_system_prompt && (role == "system" || role == "developer")))
        .collect();
    if filtered.is_empty() {
        return None;
    }
    let start = filtered.len().saturating_sub(cfg.conversation_history_depth.max(1) * 2);
    let mut out = String::new();
    for (role, text) in &filtered[start..] {
        let clean: String = text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase();
        out.push_str(role);
        out.push(':');
        out.push_str(&clean);
        out.push('\n');
    }
    let trimmed = out.trim().to_string();
    if trimmed.is_empty() { None } else { Some(trimmed) }
}

fn make_scope(api_key_hash: &str, model: &str, cache_by_model: bool) -> String {
    if cache_by_model {
        format!("{api_key_hash}::{model}")
    } else {
        api_key_hash.to_string()
    }
}

fn sha256_hex(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
}

fn build_sparse_vector(text: &str) -> (HashMap<String, f64>, f64) {
    let mut map: HashMap<String, f64> = HashMap::new();
    let tokens: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() >= 2)
        .map(|s| s.to_ascii_lowercase())
        .collect();
    for tok in &tokens {
        *map.entry(format!("w:{tok}")).or_insert(0.0) += 1.5;
    }
    for pair in tokens.windows(2) {
        *map.entry(format!("b:{}_{}", pair[0], pair[1])).or_insert(0.0) += 1.0;
    }
    let norm = map.values().map(|v| v * v).sum::<f64>().sqrt();
    (map, norm)
}

fn cosine_similarity(
    a: &HashMap<String, f64>,
    norm_a: f64,
    b: &HashMap<String, f64>,
    norm_b: f64,
) -> f64 {
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    let (small, big) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let dot: f64 = small
        .iter()
        .filter_map(|(k, v)| big.get(k).map(|bv| v * bv))
        .sum();
    (dot / (norm_a * norm_b)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn direct_and_semantic_hits_with_key_isolation() {
        let cache = SemanticCache::new();
        let cfg = SemanticCacheConfig {
            enabled: true,
            similarity_threshold: 0.72,
            ..Default::default()
        };
        let p1 = "user:explain rust ownership borrowing and lifetimes cleanly";
        cache.store(&cfg, "key-a", "gpt-6.1-sol", p1, "openai", json!({"ok": 1}), 120);

        // Exact hit for same key
        let h1 = cache.lookup(&cfg, "key-a", "gpt-6.1-sol", p1).unwrap();
        assert_eq!(h1.kind, CacheHitKind::Direct);
        assert_eq!(h1.saved_tokens, 120);

        // Key isolation: different key MUST NOT hit!
        assert!(cache.lookup(&cfg, "key-b", "gpt-6.1-sol", p1).is_none());

        // Semantic near-duplicate hit for same key
        let p2 = "user:explain rust ownership borrowing and lifetimes clearly";
        let h2 = cache.lookup(&cfg, "key-a", "gpt-6.1-sol", p2).unwrap();
        assert_eq!(h2.kind, CacheHitKind::Semantic);
        assert!(h2.similarity >= 0.72);

        let st = cache.stats();
        assert_eq!(st.direct_hits, 1);
        assert_eq!(st.semantic_hits, 1);
        assert_eq!(st.tokens_saved, 240);
    }

    #[test]
    fn lru_eviction_respects_max_entries() {
        let cache = SemanticCache::new();
        let cfg = SemanticCacheConfig {
            enabled: true,
            max_entries: 2,
            ..Default::default()
        };
        cache.store(&cfg, "k", "m", "alpha prompt one", "p", json!(1), 10);
        cache.store(&cfg, "k", "m", "beta prompt two", "p", json!(2), 10);
        cache.store(&cfg, "k", "m", "gamma prompt three", "p", json!(3), 10);
        let st = cache.stats();
        assert_eq!(st.entries, 2);
        assert_eq!(st.evictions, 1);
    }
}
