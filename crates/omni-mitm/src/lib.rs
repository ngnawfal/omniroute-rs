// MITM proxy — port of src/mitm core paths (targets registry + connect
// routing + hostname resolution), Rust-native TLS interception foundation.
//
// Architecture (mirrors manager.ts + server.cjs):
//   CONNECT capture → SNI hostname → resolveTarget:
//     bypass      — user patterns, straight TCP relay
//     target      — matched agent host: terminate TLS, inspect body, forward
//     passthrough — no match: TCP relay untouched

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MitmTarget {
    pub id: String,
    pub name: String,
    pub hosts: Vec<String>,
    pub port: u16,
    pub endpoint_patterns: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum ConnectionRoute {
    Bypass { reason: String },
    Target(MitmTarget),
    Passthrough,
}

/// ALL_TARGETS ports (10-agents registry).
pub fn all_targets() -> Vec<MitmTarget> {
    vec![
        MitmTarget { id: "antigravity".into(), name: "Antigravity".into(), hosts: vec!["antigravity.googleapis.com".into()], port: 443, endpoint_patterns: vec!["/v1/chat/completions".into()] },
        MitmTarget { id: "kiro".into(), name: "Kiro".into(), hosts: vec!["kiro.dev".into(), "codewhisperer.us-east-1.amazonaws.com".into()], port: 443, endpoint_patterns: vec!["/v1/chat/completions".into()] },
        MitmTarget { id: "copilot".into(), name: "GitHub Copilot".into(), hosts: vec!["api.business.githubcopilot.com".into(), "api.githubcopilot.com".into()], port: 443, endpoint_patterns: vec!["/chat/completions".into()] },
        MitmTarget { id: "ghe-copilot".into(), name: "GHE Copilot".into(), hosts: vec!["ghe-copilot.com".into()], port: 443, endpoint_patterns: vec![] },
        MitmTarget { id: "codex".into(), name: "Codex".into(), hosts: vec!["chatgpt.com".into()], port: 443, endpoint_patterns: vec!["/backend-api/code".into(), "/v1/chat/completions".into()] },
        MitmTarget { id: "cursor".into(), name: "Cursor".into(), hosts: vec!["api2.cursor.sh".into(), "api3.cursor.sh".into()], port: 443, endpoint_patterns: vec!["/v1/chat/completions".into()] },
        MitmTarget { id: "zed".into(), name: "Zed".into(), hosts: vec!["api.zed.dev".into()], port: 443, endpoint_patterns: vec![] },
        MitmTarget { id: "claude-code".into(), name: "Claude Code".into(), hosts: vec!["api.anthropic.com".into()], port: 443, endpoint_patterns: vec!["/v1/messages".into()] },
        MitmTarget { id: "open-code".into(), name: "OpenCode".into(), hosts: vec!["opencode.ai".into()], port: 443, endpoint_patterns: vec!["/v1/chat/completions".into()] },
        MitmTarget { id: "trae".into(), name: "Trae".into(), hosts: vec!["api.trae.ai".into()], port: 443, endpoint_patterns: vec![] },
    ]
}

/// resolveTarget hostname (case-insensitive exact).
pub fn resolve_target(hostname: &str) -> Option<MitmTarget> {
    let h = hostname.to_lowercase();
    all_targets()
        .into_iter()
        .find(|t| t.hosts.iter().any(|host| host.to_lowercase() == h))
}

/// routeConnection: bypass patterns > target match > passthrough.
/// case-insensitive, glob not used (exact per-target hosts).
pub fn route_connection(hostname: &str, user_bypass: &[String]) -> ConnectionRoute {
    if !hostname.is_empty() {
        let h = hostname.to_lowercase();
        if user_bypass
            .iter()
            .any(|p| p.to_lowercase() == h)
        {
            return ConnectionRoute::Bypass {
                reason: "bypass".into(),
            };
        }
    }
    match resolve_target(hostname) {
        Some(t) => ConnectionRoute::Target(t),
        None => ConnectionRoute::Passthrough,
    }
}

/// Detect whether an endpoint pattern hit (for handling introspection).
pub fn target_matches_path(target: &MitmTarget, path: &str) -> bool {
    target
        .endpoint_patterns
        .iter()
        .any(|p| path.starts_with(p))
}

/// Should-keepalive decision — from detection/*.
pub fn is_connect_target_host(hostline: &str) -> bool {
    let host = hostline.split(':').next().unwrap_or(hostline);
    let _host = host.to_lowercase();
    true // connect targets any host; routing decides next
}

/// Aggregated gate state (manager.ts orchestrator summary).
#[derive(Debug, Clone, Default)]
pub struct MitmGateState {
    pub dns_entries: HashMap<String, String>, // host → target_ip
    pub cert_dir: Option<String>,
    pub active_targets: Vec<String>,
    pub bypass_patterns: Vec<String>,
}

impl MitmGateState {
    /// puppet-target registration after cert provisioning succeeded.
    pub fn add_target(&mut self, id: &str) {
        if !self.active_targets.contains(&id.to_string()) {
            self.active_targets.push(id.to_string());
        }
    }
    /// Compute repair plan differences (repair.ts buildRepairPlan port).
    pub fn repair_needed(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.cert_dir.is_none() {
            out.push("generate_cert");
        }
        if self.dns_entries.is_empty() && !self.active_targets.is_empty() {
            out.push("provision_dns");
        }
        for t in &self.active_targets {
            match t.as_str() {
                "claude-code" | "codex" | "copilot" => {}
                _ => {}
            }
        }
        if self.bypass_patterns.is_empty() {
            out.push("reload_bypass");
        }
        out
    }
}

/// Upstream CA env/state wiring (upstreamTrust.ts — env var wins over stored).
pub fn resolve_upstream_ca_path(env_var: Option<String>, stored: Option<String>) -> Option<String> {
    env_var.or(stored)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_resolves_opencode() {
        let t = resolve_target("opencode.ai").unwrap();
        assert_eq!(t.id, "open-code");
        assert!(target_matches_path(&t, "/v1/chat/completions"));
        assert!(!target_matches_path(&t, "/v2/nope"));
    }

    #[test]
    fn target_resolves_case_insensitive() {
        assert!(resolve_target("OPENCODE.AI").is_some());
        assert!(resolve_target("Api.Anthropic.com").is_some());
    }

    #[test]
    fn no_match_is_passthrough() {
        assert!(resolve_target("example.com").is_none());
    }

    #[test]
    fn route_bypass_wins_over_target() {
        let bypass = vec!["opencode.ai".to_string()];
        match route_connection("opencode.ai", &bypass) {
            ConnectionRoute::Bypass { reason } => assert_eq!(reason, "bypass"),
            _ => panic!("bypass pattern should win"),
        }
        match route_connection("opencode.ai", &[]) {
            ConnectionRoute::Target(t) => assert_eq!(t.id, "open-code"),
            _ => panic!("no bypass → target"),
        }
        match route_connection("random.host", &[]) {
            ConnectionRoute::Passthrough => {}
            _ => panic!("unknown host → passthrough"),
        }
    }

    #[test]
    fn all_10_targets_have_hosts() {
        let targets = all_targets();
        assert_eq!(targets.len(), 10);
        for t in &targets {
            assert!(!t.hosts.is_empty(), "{} hosts empty", t.id);
            assert_eq!(t.port, 443);
        }
    }

    #[test]
    fn gate_repair_needed() {
        let mut g = MitmGateState::default();
        assert!(g.repair_needed().contains(&"generate_cert"));
        g.cert_dir = Some("C:/certs".into());
        g.active_targets = vec!["open-code".into()];
        assert!(g.repair_needed().contains(&"provision_dns"));
        g.dns_entries.insert("opencode.ai".into(), "127.0.0.1".into());
        g.repair_needed();
        assert!(g.add_target_check_passes());
    }

    impl MitmGateState {
        fn add_target_check_passes(&mut self) -> bool {
            self.add_target("open-code");
            self.active_targets.len() == 1
        }
    }

    #[test]
    fn upstream_ca_pref() {
        assert_eq!(
            resolve_upstream_ca_path(Some("env.pth".into()), Some("db.pth".into())),
            Some("env.pth".into())
        );
        assert_eq!(
            resolve_upstream_ca_path(None, Some("db.pth".into())),
            Some("db.pth".into())
        );
        assert_eq!(resolve_upstream_ca_path(None, None), None);
    }
}
