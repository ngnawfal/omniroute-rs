// Full CLI subcommand runner for `omniroute-rs`:
//   serve [--port N] [--mitm-port N]
//   mcp
//   mitm [--port N] [--gateway URL]
//   status
//   doctor
//   providers [--free] [--q QUERY] [--limit N]
//   models [--provider ID] [--q QUERY] [--limit N]
//   keys <list|create|delete>
//   combos <list|simulate>
//   oauth <list|authorize>
//   memory <list|add|search>
//   skills <list>
//   version | --version | -V
//   help | --help | -h

use crate::api::ServerMcpContext;
use crate::chat::expand_and_order_candidates;
use crate::state::{now_ms, AppState};
use omni_mitm::cert::CertAuthority;
use omni_mitm::proxy::{MitmProxyConfig, MitmProxyHandle};
use omni_oauth::{build_authorize_url, find_oauth_provider, generate_pkce, OAUTH_PROVIDERS};
use omni_router::normalize_strategy;
use omni_store::MemoryRow;
use std::sync::Arc;

pub fn print_help() {
    println!(
        r#"omniroute-rs {} (upstream OmniRoute v{})

USAGE:
  omniroute-rs [COMMAND] [OPTIONS]

COMMANDS:
  serve [--port N] [--mitm-port N]         Start HTTP AI Gateway (default port 20128)
  mcp                                      Run JSON-RPC 2.0 MCP server over stdio (24 tools)
  mitm [--port N] [--gateway URL]          Run standalone TCP/TLS HTTP CONNECT MITM proxy
  status                                   Show local catalog, SQLite, OAuth & MITM summary
  doctor                                   Run environment, SQLite, CA cert & catalog diagnostics
  providers [--free] [--q TEXT] [--limit N] List embedded 394 providers
  models [--provider ID] [--q TEXT] [--limit N] Search embedded 3,239 models
  keys list                                List configured API keys
  keys create [--label TEXT] [--rpm N]     Generate & persist a new API key
  keys delete <KEY>                        Delete an API key
  combos list                              List configured routing combos
  combos simulate <MODEL> [--strategy S]   Dry-run 22-strategy routing for a model/combo
  oauth list                               List supported OAuth 2.0 providers
  oauth authorize <PROVIDER>               Generate PKCE authorization URL
  memory list [--q TEXT]                   List stored persistent agent memories
  memory add <KEY> <CONTENT>               Store a new factual memory
  memory search <QUERY>                    Semantic TF-cosine memory retrieval
  skills list                              List installed agent skills
  version                                  Print version & upstream catalog sync metadata
  help                                     Print this help message"#,
        env!("CARGO_PKG_VERSION"),
        omni_catalog::upstream_version()
    );
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|w| w[0] == flag)
        .map(|w| w[1].clone())
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

/// Returns `Ok(Some((port, mitm_port)))` when the user invoked `serve` (or no subcommand),
/// or `Ok(None)` when a CLI subcommand executed to completion.
pub async fn run_cli(args: &[String], data_dir: &str) -> anyhow::Result<Option<(u16, Option<u16>)>> {
    let subcmd = args.get(1).map(|s| s.as_str()).unwrap_or("serve");
    let rest = if args.len() > 2 { &args[2..] } else { &[] };

    match subcmd {
        "serve" => {
            let port: u16 = flag_value(rest, "--port")
                .and_then(|v| v.parse().ok())
                .or_else(|| std::env::var("OMNIROUTE_PORT").ok().and_then(|v| v.parse().ok()))
                .unwrap_or(20128);
            let mitm_port: Option<u16> = flag_value(rest, "--mitm-port")
                .and_then(|v| v.parse().ok())
                .or_else(|| {
                    std::env::var("OMNIROUTE_MITM_PORT")
                        .ok()
                        .and_then(|v| v.parse().ok())
                });
            Ok(Some((port, mitm_port)))
        }
        "version" | "--version" | "-V" => {
            let cat = omni_catalog::Catalog::with_seed();
            println!(
                "omniroute-rs {} (upstream OmniRoute v{}, {} providers, {} models)",
                env!("CARGO_PKG_VERSION"),
                omni_catalog::upstream_version(),
                cat.provider_count(),
                cat.model_count()
            );
            Ok(None)
        }
        "help" | "--help" | "-h" => {
            print_help();
            Ok(None)
        }
        "mcp" => {
            let app_state = Arc::new(AppState::open(data_dir)?);
            let ctx = ServerMcpContext { state: app_state };
            omni_mcp::run_stdio_blocking(&ctx)?;
            Ok(None)
        }
        "mitm" => {
            let port: u16 = flag_value(rest, "--port")
                .and_then(|v| v.parse().ok())
                .unwrap_or(20443);
            let gateway = flag_value(rest, "--gateway")
                .unwrap_or_else(|| "http://127.0.0.1:20128".into());
            let mitm_dir = std::path::Path::new(data_dir).join("mitm");
            let ca = Arc::new(
                CertAuthority::load_or_create(&mitm_dir)
                    .map_err(|e| anyhow::anyhow!("CA init failed: {e}"))?,
            );
            let handle = MitmProxyHandle::start(
                ca.clone(),
                MitmProxyConfig {
                    bind_addr: format!("127.0.0.1:{port}"),
                    gateway_base_url: gateway.clone(),
                    bypass_patterns: vec![],
                    extra_targets: vec![],
                },
            )
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
            println!(
                "omniroute-rs MITM proxy listening on http://{} -> gateway {gateway}",
                handle.bind_addr()
            );
            println!("Root CA cert: {}/ca.crt", mitm_dir.display());
            println!("SHA-256:      {}", ca.ca_fingerprint_sha256());
            println!("Press Ctrl+C to stop.");
            let _ = tokio::signal::ctrl_c().await;
            handle.stop();
            Ok(None)
        }
        "status" => {
            let st = Arc::new(AppState::open(data_dir)?);
            let cat = st.catalog.read().unwrap();
            let usage = st.store.usage_summary().unwrap_or_else(|_| serde_json::json!({}));
            let keys = st.store.list_api_key_rows().unwrap_or_default().len();
            let creds = st.store.list_credentials().unwrap_or_default().len();
            let combos = st.store.list_combos().unwrap_or_default().len();
            let (_, mems, _) = st
                .store
                .list_memories(None, None, None, None, None, now_ms(), 1, 0)
                .unwrap_or((vec![], 0, 0));
            let skills = st.store.list_skills().unwrap_or_default().len();
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "version": env!("CARGO_PKG_VERSION"),
                    "upstream_sync_version": omni_catalog::upstream_version(),
                    "data_dir": data_dir,
                    "providers": cat.provider_count(),
                    "models": cat.model_count(),
                    "api_keys": keys,
                    "credentials": creds,
                    "combos": combos,
                    "memories": mems,
                    "skills": skills,
                    "mcp_tools": omni_mcp::TOOL_CATALOG.len(),
                    "oauth_providers": OAUTH_PROVIDERS.len(),
                    "mitm_ca_sha256": st.mitm_ca.read().unwrap().ca_fingerprint_sha256(),
                    "usage": usage,
                }))?
            );
            Ok(None)
        }
        "doctor" => {
            let st = Arc::new(AppState::open(data_dir)?);
            let (p_cnt, m_cnt) = {
                let c = st.catalog.read().unwrap();
                (c.provider_count(), c.model_count())
            };
            let ca_fp = st.mitm_ca.read().unwrap().ca_fingerprint_sha256();
            let db_path = format!("{data_dir}/omniroute-rs.db");
            let ca_path = format!("{data_dir}/mitm/ca.crt");
            let port: u16 = std::env::var("OMNIROUTE_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(20128);
            let port_free = std::net::TcpListener::bind(("127.0.0.1", port)).is_ok();
            println!("omniroute-rs doctor (upstream v{})", omni_catalog::upstream_version());
            println!("  [OK] Catalog embedded:   {p_cnt} providers, {m_cnt} models");
            println!("  [OK] SQLite WAL store:   {db_path}");
            println!("  [OK] MITM Root CA:       {ca_path} (sha256: {}...)", &ca_fp[..16]);
            println!("  [OK] MCP tools:          {} registered", omni_mcp::TOOL_CATALOG.len());
            println!("  [OK] OAuth providers:    {} configured", OAUTH_PROVIDERS.len());
            println!(
                "  [{}] Gateway port {port}: {}",
                if port_free { "OK" } else { "IN_USE" },
                if port_free { "available" } else { "already bound" }
            );
            Ok(None)
        }
        "providers" => {
            let free_only = has_flag(rest, "--free");
            let q = flag_value(rest, "--q").map(|s| s.to_ascii_lowercase());
            let limit: usize = flag_value(rest, "--limit")
                .and_then(|v| v.parse().ok())
                .unwrap_or(50);
            let cat = omni_catalog::Catalog::with_seed();
            let mut shown = 0usize;
            for p in &cat.providers {
                if free_only && !p.is_free_tier {
                    continue;
                }
                if let Some(ref ql) = q {
                    if !p.id.to_ascii_lowercase().contains(ql)
                        && !p.name.as_deref().unwrap_or("").to_ascii_lowercase().contains(ql)
                    {
                        continue;
                    }
                }
                println!(
                    "{:<24} {:<10} {:<6} models={:<4} free={}",
                    p.id,
                    p.format,
                    p.auth_type,
                    p.model_specs.len(),
                    p.is_free_tier
                );
                shown += 1;
                if shown >= limit {
                    break;
                }
            }
            println!("--- shown {shown} / {} providers ---", cat.provider_count());
            Ok(None)
        }
        "models" => {
            let prov = flag_value(rest, "--provider");
            let q = flag_value(rest, "--q").map(|s| s.to_ascii_lowercase());
            let limit: usize = flag_value(rest, "--limit")
                .and_then(|v| v.parse().ok())
                .unwrap_or(50);
            let cat = omni_catalog::Catalog::with_seed();
            let mut shown = 0usize;
            for p in &cat.providers {
                if let Some(ref pf) = prov {
                    if !p.matches_id_or_alias(pf) {
                        continue;
                    }
                }
                for m in &p.model_specs {
                    let full = format!("{}/{}", p.id, m.id);
                    if let Some(ref ql) = q {
                        if !full.to_ascii_lowercase().contains(ql) {
                            continue;
                        }
                    }
                    println!("{full}");
                    shown += 1;
                    if shown >= limit {
                        break;
                    }
                }
                if shown >= limit {
                    break;
                }
            }
            println!("--- shown {shown} / {} models ---", cat.model_count());
            Ok(None)
        }
        "keys" => {
            let st = AppState::open(data_dir)?;
            let action = rest.first().map(|s| s.as_str()).unwrap_or("list");
            match action {
                "create" => {
                    let label = flag_value(rest, "--label").unwrap_or_else(|| "cli".into());
                    let rpm = flag_value(rest, "--rpm").and_then(|v| v.parse::<u32>().ok());
                    let key = format!("sk-omni-{}", uuid::Uuid::new_v4().simple());
                    st.store
                        .add_api_key_with_limits(&key, &label, now_ms(), None, None, rpm)?;
                    println!("{key}");
                }
                "delete" => {
                    let key = rest.get(1).map(|s| s.as_str()).unwrap_or("");
                    let deleted = st.store.delete_api_key(key)?;
                    println!("deleted={deleted}");
                }
                _ => {
                    for row in st.store.list_api_key_rows()? {
                        let preview = if row.key.len() > 10 {
                            format!("{}...{}", &row.key[..6], &row.key[row.key.len() - 4..])
                        } else {
                            "***".into()
                        };
                        println!("{preview:<20} label={:<16} rpm={:?}", row.label, row.rpm_limit);
                    }
                }
            }
            Ok(None)
        }
        "combos" => {
            let st = Arc::new(AppState::open(data_dir)?);
            let action = rest.first().map(|s| s.as_str()).unwrap_or("list");
            match action {
                "simulate" => {
                    let model = rest.get(1).map(|s| s.as_str()).unwrap_or("openai/gpt-6.1-sol");
                    let strat = flag_value(rest, "--strategy").map(|s| normalize_strategy(&s));
                    let (used, is_combo, ordered) =
                        expand_and_order_candidates(&st, model, strat, 1_000);
                    println!("model={model} is_combo={is_combo} strategy={}", used.as_str());
                    for (i, t) in ordered.iter().enumerate() {
                        println!(
                            "  {}. {}/{} (conn={}, weight={:.2}, breaker_open={})",
                            i + 1,
                            t.provider,
                            t.model,
                            t.connection_id,
                            t.weight,
                            t.breaker_open
                        );
                    }
                }
                _ => {
                    for c in st.store.list_combos()? {
                        println!(
                            "{:<24} strategy={:<14} steps={} enabled={}",
                            c.name,
                            c.strategy,
                            c.models.len(),
                            c.enabled
                        );
                    }
                }
            }
            Ok(None)
        }
        "oauth" => {
            let action = rest.first().map(|s| s.as_str()).unwrap_or("list");
            match action {
                "authorize" => {
                    let prov = rest.get(1).map(|s| s.as_str()).unwrap_or("codex");
                    if let Some(cfg) = find_oauth_provider(prov) {
                        let pkce = generate_pkce();
                        let url = build_authorize_url(
                            cfg,
                            &cfg.default_redirect_uri(),
                            &pkce,
                            None,
                            &[],
                        );
                        println!("provider:      {}", cfg.id);
                        println!("authorize_url: {url}");
                        println!("state:         {}", pkce.state);
                    } else {
                        anyhow::bail!("unsupported OAuth provider: {prov}");
                    }
                }
                _ => {
                    for p in OAUTH_PROVIDERS {
                        println!("{:<16} flow={:?} pkce={}", p.id, p.flow, p.uses_pkce);
                    }
                }
            }
            Ok(None)
        }
        "memory" => {
            let st = AppState::open(data_dir)?;
            let action = rest.first().map(|s| s.as_str()).unwrap_or("list");
            match action {
                "add" => {
                    let key = rest.get(1).map(|s| s.as_str()).unwrap_or("note");
                    let content = rest.get(2..).map(|s| s.join(" ")).unwrap_or_default();
                    let now = now_ms();
                    let id = uuid::Uuid::new_v4().to_string();
                    st.store.upsert_memory(&MemoryRow {
                        id: id.clone(),
                        api_key_hash: String::new(),
                        session_id: String::new(),
                        mem_type: "factual".into(),
                        mem_key: key.into(),
                        content,
                        category: "cli".into(),
                        metadata: serde_json::json!({}),
                        tokens_est: 0,
                        expires_at: None,
                        created_at: now,
                        updated_at: now,
                    })?;
                    println!("stored memory id={id}");
                }
                "search" => {
                    let q = rest.get(1..).map(|s| s.join(" ")).unwrap_or_default();
                    for (m, score) in st.store.retrieve_memories(&q, None, None, now_ms(), 10)? {
                        println!("[{score:.2}] {} ({}): {}", m.mem_key, m.mem_type, m.content);
                    }
                }
                _ => {
                    let q = flag_value(rest, "--q");
                    let (rows, total, tokens) = st.store.list_memories(
                        None,
                        None,
                        None,
                        None,
                        q.as_deref(),
                        now_ms(),
                        50,
                        0,
                    )?;
                    for m in rows {
                        println!("{} [{}] {}: {}", m.id, m.mem_type, m.mem_key, m.content);
                    }
                    println!("--- total={total} tokens_est={tokens} ---");
                }
            }
            Ok(None)
        }
        "skills" => {
            let st = AppState::open(data_dir)?;
            for sk in st.store.list_skills()? {
                println!(
                    "{:<24} mode={:<5} source={:<10} runs={} {}",
                    sk.name, sk.mode, sk.source_provider, sk.executions_count, sk.description
                );
            }
            Ok(None)
        }
        other => {
            eprintln!("Unknown subcommand: {other}\n");
            print_help();
            anyhow::bail!("unknown subcommand: {other}");
        }
    }
}
