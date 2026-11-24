// Status page: plain HTML, self-contained, no external assets.
// Displays upstream sync version, 394-provider catalog stats, live SQLite usage,
// circuit breakers, active lockouts, guardrails & gamification summary, and endpoints.

use crate::state::SharedState;
use axum::extract::State;
use axum::http::header;
use axum::response::Response;
use omni_types::errors::BACKOFF_MAX_MS;

pub async fn status_page(State(state): State<SharedState>) -> Response {
    let (prov_count, model_count) = {
        let cat = state.catalog.read().unwrap();
        (cat.provider_count(), cat.model_count())
    };
    let usage = state.store.usage_summary().unwrap_or_else(|_| serde_json::json!({}));
    let reqs = usage.get("requests").and_then(|v| v.as_i64()).unwrap_or(0);
    let toks = usage.get("total_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
    let combos = state.store.list_combos().map(|c| c.len()).unwrap_or(0);
    let creds = state.store.list_credentials().map(|c| c.len()).unwrap_or(0);
    let gami = state.gamification_summary();
    let level = gami.get("level").and_then(|v| v.as_u64()).unwrap_or(1);
    let xp = gami.get("total_xp").and_then(|v| v.as_i64()).unwrap_or(0);
    let lockouts_count = omni_exec::resilience::all_lockouts().len();
    let cache_stats = state.semantic_cache.stats();
    let comp_saved = state.compression_totals.lock().unwrap().total_tokens_saved;

    let breaker_rows = omni_exec::resilience::all_provider_states()
        .into_iter()
        .map(|(name, state_str, streak)| {
            let color = match state_str {
                "CLOSED" => "#7ee787",
                "DEGRADED" => "#f2cc60",
                "OPEN" => "#f85149",
                _ => "#8b949e",
            };
            format!(
                "<tr><td>{name}</td><td style='color:{color}'>{state_str}</td><td>{streak}</td></tr>"
            )
        })
        .collect::<Vec<_>>()
        .join("");

    let breaker_table = if breaker_rows.is_empty() {
        "<tr><td colspan='3' class='dim'>All providers healthy (no tripped breakers)</td></tr>".to_string()
    } else {
        breaker_rows
    };

    let html = format!(
        r#"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>omniroute-rs</title>
<style>
body {{ font-family: ui-monospace, Menlo, Consolas, monospace; background: #0d1117;
       color: #c9d1d9; margin: 0; padding: 2rem; max-width: 860px; margin-inline: auto; }}
h1 {{ color: #58a6ff; font-size: 1.4rem; margin-bottom: .3rem; }}
h2 {{ color: #c9d1d9; font-size: 1.05rem; border-bottom: 1px solid #21262d; padding-bottom: .3rem; margin-top: 1.5rem; }}
.grid {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(170px, 1fr)); gap: .75rem; margin: 1rem 0; }}
.card {{ background: #161b22; border: 1px solid #30363d; border-radius: 6px; padding: .75rem; }}
.card .v {{ font-size: 1.25rem; color: #7ee787; font-weight: 700; }}
.card .k {{ font-size: .78rem; color: #8b949e; }}
table {{ border-collapse: collapse; width: 100%; font-size: .85rem; }}
td, th {{ text-align: left; padding: .35rem .6rem; border-bottom: 1px solid #21262d; }}
code {{ color: #79c0ff; }}
.dim {{ color: #8b949e; font-size: .8rem; }}
</style>
</head>
<body>
<h1>&#128640; omniroute-rs</h1>
<p class="dim">Rust AI Gateway &mdash; synced with upstream OmniRoute v{up_ver} &mdash; single native binary, zero Node/Electron.</p>

<div class="grid">
  <div class="card"><div class="v">{prov_count}</div><div class="k">Providers ({model_count} models)</div></div>
  <div class="card"><div class="v">{reqs}</div><div class="k">Total Requests ({toks} tokens)</div></div>
  <div class="card"><div class="v">{combos} / {creds}</div><div class="k">Combos / Credentials</div></div>
  <div class="card"><div class="v">Lv.{level} ({xp} XP)</div><div class="k">Lockouts: {lockouts_count} &middot; Guard: {guard}</div></div>
  <div class="card"><div class="v">{comp_saved} tok</div><div class="k">RTK+Caveman saved &middot; Cache hit: {cache_hit:.1}% ({cache_entries} entries)</div></div>
</div>

<h2>Gateway &amp; Management Endpoints</h2>
<p>
<code>POST /v1/chat/completions</code> &middot; <code>POST /v1/responses</code> &middot; <code>POST /v1/completions</code><br>
<code>POST /v1/messages</code> &middot; <code>POST /v1/messages/count_tokens</code> &middot; <code>POST /v1beta/models/{{model}}:generateContent</code><br>
<code>POST /v1/embeddings</code> &middot; <code>/v1/images/*</code> &middot; <code>/v1/audio/*</code> &middot; <code>/v1/rerank</code> &middot; <code>/v1/moderations</code> &middot; <code>/v1/decisions</code><br>
<code>GET/POST /v1/search</code> &middot; <code>POST /v1/web/fetch</code> (SSRF-guarded)<br>
<code>GET  /v1/models</code> &middot; <code>GET /v1/me/status</code> &middot; <code>GET /v1/me/usage</code> &middot; <code>GET /v1/me/quota</code><br>
<code>GET/POST /api/providers</code> &middot; <code>GET/POST /api/providers/credentials</code> &middot; <code>GET/POST /api/combos</code><br>
<code>GET/POST /api/keys</code> &middot; <code>GET/PUT /api/settings</code> &middot; <code>GET/PUT /api/settings/compression</code> &middot; <code>GET /api/cache/stats</code><br>
<code>GET /api/guardrails/status</code> &middot; <code>GET /api/gamification/status</code> &middot; <code>GET/POST /api/mitm/status|start|stop|ca.crt</code><br>
<code>GET /.well-known/agent.json</code> &middot; <code>POST /a2a</code> (A2A v0.3 + v1.0) &middot; <code>/api/memory/*</code> &middot; <code>/api/skills/*</code><br>
<code>GET /api/oauth/providers</code> &middot; <code>POST /api/oauth/{{provider}}/authorize|callback|poll|import</code> &middot; <code>POST /api/oauth/sweep</code><br>
<code>POST /api/mcp</code> &mdash; JSON-RPC 2.0 MCP Server (24 tools, also <code>omniroute-rs mcp</code> stdio)
</p>

<h2>Provider Breakers</h2>
<table>
<tr><th>Provider</th><th>State</th><th>Fail streak</th></tr>
{breaker_table}
</table>
<p class="dim">Default strategy: <code>{strat}</code> (22 v3.8.55 strategies) &middot; Backoff cap: {max}s &middot; Agent-guard: <code>{ag}</code>.</p>
</body>
</html>"#,
        up_ver = omni_catalog::upstream_version(),
        guard = if state.guardrails_enabled() { "on" } else { "off" },
        strat = state.default_strategy().as_str(),
        max = BACKOFF_MAX_MS / 1000,
        ag = state.agent_guard_mode().as_str(),
        cache_hit = cache_stats.hit_rate,
        cache_entries = cache_stats.entries,
    );

    Response::builder()
        .status(200)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(axum::body::Body::from(html))
        .unwrap()
}
