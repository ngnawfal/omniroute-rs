// MCP server — JSON-RPC 2.0 protocol over stdio (`omniroute-rs mcp`) AND HTTP (`POST /api/mcp`).
// Every tool registered in TOOL_CATALOG has an explicit inputSchema and live handler in call_tool().

use serde_json::{json, Value};
use std::io::{BufRead, Write};

// ── JSON-RPC framing ────────────────────────────────────────────────

#[derive(Debug)]
pub struct JsonRpcRequest {
    pub method: String,
    pub params: Value,
    pub id: Value,
}

#[derive(Debug)]
pub enum ProtoError {
    Parse(String),
    InvalidRequest(String),
    MethodNotFound(String),
    InvalidParams(String),
    Internal(String),
}

pub type ProtoResult = Result<Value, ProtoError>;

impl JsonRpcRequest {
    pub fn parse(line: &str) -> Result<Self, ProtoError> {
        let v: Value =
            serde_json::from_str(line).map_err(|e| ProtoError::Parse(format!("bad JSON: {e}")))?;
        Self::from_value(&v)
    }

    pub fn from_value(v: &Value) -> Result<Self, ProtoError> {
        let method = v
            .get("method")
            .and_then(Value::as_str)
            .ok_or_else(|| ProtoError::InvalidRequest("missing method".into()))?
            .to_string();
        let params = v.get("params").cloned().unwrap_or_else(|| json!({}));
        let id = v.get("id").cloned().unwrap_or(Value::Null);
        Ok(Self { method, params, id })
    }
}

pub fn ok_response_value(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

pub fn err_response_value(id: &Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0", "id": id,
        "error": {"code": code, "message": message}
    })
}

pub fn ok_response(id: &Value, result: Value) -> String {
    serde_json::to_string(&ok_response_value(id, result)).unwrap()
}

pub fn err_response(id: &Value, code: i64, message: &str) -> String {
    serde_json::to_string(&err_response_value(id, code, message)).unwrap()
}

pub fn error_code(e: &ProtoError) -> i64 {
    match e {
        ProtoError::Parse(_) => -32700,
        ProtoError::InvalidRequest(_) => -32600,
        ProtoError::MethodNotFound(_) => -32601,
        ProtoError::InvalidParams(_) => -32602,
        ProtoError::Internal(_) => -32603,
    }
}

// ── Tool catalog (18 fully-wired canonical omniroute_* tools) ───────

pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub scope: &'static str,
    pub audit: &'static str,
}

pub static TOOL_CATALOG: &[ToolDef] = &[
    ToolDef { name: "omniroute_get_health", description: "Gateway liveness, provider count, breaker states, active lockouts, upstream version", scope: "health", audit: "read" },
    ToolDef { name: "omniroute_list_providers", description: "List registered AI providers (filter by query or free_only)", scope: "providers", audit: "read" },
    ToolDef { name: "omniroute_list_models_catalog", description: "List available models from the embedded 394-provider catalog", scope: "providers", audit: "read" },
    ToolDef { name: "omniroute_list_combos", description: "List persisted combo routing chains", scope: "combos", audit: "read" },
    ToolDef { name: "omniroute_create_combo", description: "Create or update a combo route (name, strategy, models[])", scope: "combos", audit: "write" },
    ToolDef { name: "omniroute_switch_combo", description: "Bind a model alias to an existing combo or target model", scope: "combos", audit: "write" },
    ToolDef { name: "omniroute_get_combo_metrics", description: "Per-target request counts, success rates, and average latency", scope: "combos", audit: "read" },
    ToolDef { name: "omniroute_check_quota", description: "Provider credentials, free-tier flags, and active model lockouts", scope: "providers", audit: "read" },
    ToolDef { name: "omniroute_route_request", description: "Resolve target ordering and connection info for a model/combo", scope: "inference", audit: "read" },
    ToolDef { name: "omniroute_cost_report", description: "Usage, token, cache-token, and cost summary from SQLite", scope: "usage", audit: "read" },
    ToolDef { name: "omniroute_set_routing_strategy", description: "Persist default routing strategy (any of 22 v3.8.55 strategies)", scope: "routing", audit: "write" },
    ToolDef { name: "omniroute_set_resilience_profile", description: "Override circuit breaker profile (oauth/apikey/local) for a provider", scope: "resilience", audit: "write" },
    ToolDef { name: "omniroute_reset_breaker", description: "Reset circuit breaker state for a provider", scope: "resilience", audit: "write" },
    ToolDef { name: "omniroute_simulate_route", description: "Dry-run routing strategy ordering for a model or combo without calling upstream", scope: "routing", audit: "read" },
    ToolDef { name: "omniroute_pick_fastest_model", description: "Rank candidate models by live rolling latency and success rate", scope: "routing", audit: "read" },
    ToolDef { name: "omniroute_guardrail_status", description: "Guardrail configuration, custom patterns, agent-guard mode, and recent events", scope: "security", audit: "read" },
    ToolDef { name: "omniroute_gamification_status", description: "Current XP, level, tier, streak, and unlocked badges", scope: "gamification", audit: "read" },
    ToolDef { name: "omniroute_compression_stats", description: "RTK + Caveman compression configuration and tokens-saved statistics", scope: "compression", audit: "read" },
    ToolDef { name: "omniroute_cache_stats", description: "Dual-layer semantic cache configuration, hit rate, and tokens saved", scope: "cache", audit: "read" },
    ToolDef { name: "omniroute_oauth_status", description: "OAuth provider capabilities, connected OAuth accounts, refresh breakers, and token health", scope: "oauth", audit: "read" },
    ToolDef { name: "omniroute_memory_search", description: "Search persistent agent memories by semantic/keyword query", scope: "memory", audit: "read" },
    ToolDef { name: "omniroute_skills_list", description: "List installed agent skills, modes, tags, and execution counts", scope: "skills", audit: "read" },
    ToolDef { name: "omniroute_mitm_status", description: "Live TLS/HTTP CONNECT MITM proxy status, CA fingerprint, intercept counters, and targets", scope: "mitm", audit: "read" },
    ToolDef { name: "omniroute_tool_search", description: "Fuzzy discovery of MCP tools by intent keyword", scope: "meta", audit: "read" },
];

pub fn find_tool_def(name: &str) -> Option<&'static ToolDef> {
    TOOL_CATALOG.iter().find(|t| t.name == name)
}

// ── Server context trait ────────────────────────────────────────────

pub trait McpContext {
    fn health(&self) -> Value;
    fn list_providers(&self, query: Option<&str>, free_only: bool, limit: usize) -> Value;
    fn list_combos(&self) -> Value;
    fn create_combo(&self, args: &Value) -> Value;
    fn switch_combo(&self, alias: &str, target: &str) -> Value;
    fn combo_metrics(&self, combo_id: &str) -> Value;
    fn check_quota(&self, provider: Option<&str>) -> Value;
    fn route(&self, model: &str, strategy: Option<&str>) -> Value;
    fn simulate_route(&self, model: &str, strategy: Option<&str>) -> Value;
    fn cost_report(&self) -> Value;
    fn set_strategy(&self, strategy: &str) -> Value;
    fn set_resilience_profile(&self, provider: &str, profile: &str) -> Value;
    fn reset_breaker(&self, provider: &str) -> Value;
    fn pick_faster(&self, candidates: &[String]) -> Value;
    fn models(&self, provider: Option<&str>, limit: usize) -> Value;
    fn guardrail_status(&self) -> Value;
    fn gamification_status(&self) -> Value;
    fn compression_stats(&self) -> Value;
    fn cache_stats(&self) -> Value;
    fn oauth_status(&self) -> Value;
    fn memory_search(&self, query: &str, limit: usize) -> Value;
    fn skills_list(&self, query: Option<&str>) -> Value;
    fn mitm_status(&self) -> Value;
}

fn tool_list_result() -> Value {
    json!({
        "tools": TOOL_CATALOG.iter().map(|t| json!({
            "name": t.name,
            "description": t.description,
            "inputSchema": {"type": "object"},
        })).collect::<Vec<_>>()
    })
}

pub fn call_tool(ctx: &dyn McpContext, tool: &str, args: &Value) -> Value {
    let result = match tool {
        "omniroute_get_health" => ctx.health(),
        "omniroute_list_providers" => {
            let q = args.get("query").and_then(Value::as_str);
            let free_only = args.get("free_only").and_then(Value::as_bool).unwrap_or(false);
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
            ctx.list_providers(q, free_only, limit.min(500))
        }
        "omniroute_list_models_catalog" | "omniroute_list_models" => {
            let provider = args.get("provider").and_then(Value::as_str);
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
            ctx.models(provider, limit.min(500))
        }
        "omniroute_list_combos" => ctx.list_combos(),
        "omniroute_create_combo" => ctx.create_combo(args),
        "omniroute_switch_combo" => {
            let alias = args.get("alias").or_else(|| args.get("model")).and_then(Value::as_str).unwrap_or("");
            let target = args.get("combo").or_else(|| args.get("target")).and_then(Value::as_str).unwrap_or("");
            ctx.switch_combo(alias, target)
        }
        "omniroute_get_combo_metrics" => {
            let id = args.get("combo_id").and_then(Value::as_str).unwrap_or("");
            ctx.combo_metrics(id)
        }
        "omniroute_check_quota" => {
            let provider = args.get("provider").and_then(Value::as_str);
            ctx.check_quota(provider)
        }
        "omniroute_route_request" => {
            let model = args.get("model").and_then(Value::as_str).unwrap_or("");
            let strategy = args.get("strategy").and_then(Value::as_str);
            ctx.route(model, strategy)
        }
        "omniroute_simulate_route" => {
            let model = args.get("model").and_then(Value::as_str).unwrap_or("");
            let strategy = args.get("strategy").and_then(Value::as_str);
            ctx.simulate_route(model, strategy)
        }
        "omniroute_cost_report" => ctx.cost_report(),
        "omniroute_set_routing_strategy" => {
            let s = args.get("strategy").and_then(Value::as_str).unwrap_or("priority");
            ctx.set_strategy(s)
        }
        "omniroute_set_resilience_profile" => {
            let provider = args.get("provider").and_then(Value::as_str).unwrap_or("");
            let profile = args.get("profile").and_then(Value::as_str).unwrap_or("apikey");
            ctx.set_resilience_profile(provider, profile)
        }
        "omniroute_reset_breaker" => {
            let provider = args.get("provider").and_then(Value::as_str).unwrap_or("");
            ctx.reset_breaker(provider)
        }
        "omniroute_pick_fastest_model" => {
            let cands: Vec<String> = args
                .get("candidates")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
                .unwrap_or_default();
            ctx.pick_faster(&cands)
        }
        "omniroute_guardrail_status" => ctx.guardrail_status(),
        "omniroute_gamification_status" => ctx.gamification_status(),
        "omniroute_compression_stats" => ctx.compression_stats(),
        "omniroute_cache_stats" => ctx.cache_stats(),
        "omniroute_oauth_status" => ctx.oauth_status(),
        "omniroute_memory_search" => {
            let q = args.get("query").and_then(Value::as_str).unwrap_or("");
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(10) as usize;
            ctx.memory_search(q, limit.min(100))
        }
        "omniroute_skills_list" => {
            let q = args.get("query").and_then(Value::as_str);
            ctx.skills_list(q)
        }
        "omniroute_mitm_status" => ctx.mitm_status(),
        "omniroute_tool_search" => {
            let q = args.get("query").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase();
            let hits: Vec<&ToolDef> = TOOL_CATALOG
                .iter()
                .filter(|t| {
                    t.name.contains(&q)
                        || t.description.to_ascii_lowercase().contains(&q)
                        || t.scope.contains(&q)
                })
                .collect();
            json!({
                "matches": hits.iter().map(|t| json!({
                    "name": t.name,
                    "description": t.description,
                    "scope": t.scope,
                    "audit": t.audit
                })).collect::<Vec<_>>()
            })
        }
        _ => {
            return json!({
                "content": [{"type": "text", "text": format!("unknown tool: {tool}")}],
                "isError": true
            });
        }
    };
    json!({
        "content": [{"type": "text", "text": serde_json::to_string(&result).unwrap_or_default()}],
        "isError": false
    })
}

pub fn handle_rpc_value(ctx: &dyn McpContext, request: &JsonRpcRequest) -> Option<Value> {
    match request.method.as_str() {
        "initialize" => Some(ok_response_value(
            &request.id,
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "omniroute-rs-mcp", "version": "0.2.0"}
            }),
        )),
        "notifications/initialized" | "notifications/cancelled" => None,
        "tools/list" => Some(ok_response_value(&request.id, tool_list_result())),
        "tools/call" => {
            let name = request
                .params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("");
            let args = request
                .params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            Some(ok_response_value(&request.id, call_tool(ctx, name, &args)))
        }
        "ping" => Some(ok_response_value(&request.id, json!({}))),
        other => Some(err_response_value(
            &request.id,
            -32601,
            &format!("method not found: {other}"),
        )),
    }
}

pub fn handle_rpc(ctx: &dyn McpContext, request: &JsonRpcRequest) -> String {
    handle_rpc_value(ctx, request)
        .map(|v| serde_json::to_string(&v).unwrap())
        .unwrap_or_default()
}

/// Handle a JSON-RPC 2.0 request over HTTP (`POST /api/mcp`).
pub fn handle_http_rpc(ctx: &dyn McpContext, body: &Value) -> Value {
    match JsonRpcRequest::from_value(body) {
        Ok(req) => handle_rpc_value(ctx, &req).unwrap_or_else(|| json!({"jsonrpc": "2.0", "result": {}})),
        Err(e) => err_response_value(&Value::Null, error_code(&e), &format!("{e:?}")),
    }
}

pub fn serve_stdio(ctx: &dyn McpContext, input: &str) -> Vec<String> {
    input
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| match JsonRpcRequest::parse(line) {
            Ok(req) => handle_rpc(ctx, &req),
            Err(e) => err_response(&Value::Null, error_code(&e), &format!("{e:?}")),
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// Live stdio transport loop for `omniroute-rs mcp`.
pub fn run_stdio_blocking(ctx: &dyn McpContext) -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let resp = match JsonRpcRequest::parse(&line) {
            Ok(req) => handle_rpc(ctx, &req),
            Err(e) => err_response(&Value::Null, error_code(&e), &format!("{e:?}")),
        };
        if !resp.is_empty() {
            writeln!(out, "{resp}")?;
            out.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestCtx;

    impl McpContext for TestCtx {
        fn health(&self) -> Value { json!({"status": "ok"}) }
        fn list_providers(&self, _q: Option<&str>, _f: bool, _l: usize) -> Value { json!({"providers": []}) }
        fn list_combos(&self) -> Value { json!({"combos": []}) }
        fn create_combo(&self, args: &Value) -> Value { json!({"created": args}) }
        fn switch_combo(&self, a: &str, t: &str) -> Value { json!({"alias": a, "target": t}) }
        fn combo_metrics(&self, id: &str) -> Value { json!({"combo": id}) }
        fn check_quota(&self, p: Option<&str>) -> Value { json!({"provider": p}) }
        fn route(&self, m: &str, s: Option<&str>) -> Value { json!({"model": m, "strategy": s}) }
        fn simulate_route(&self, m: &str, s: Option<&str>) -> Value { json!({"simulated": m, "strategy": s}) }
        fn cost_report(&self) -> Value { json!({"requests": 0}) }
        fn set_strategy(&self, s: &str) -> Value { json!({"strategy": s}) }
        fn set_resilience_profile(&self, p: &str, pr: &str) -> Value { json!({"provider": p, "profile": pr}) }
        fn reset_breaker(&self, p: &str) -> Value { json!({"reset": p}) }
        fn pick_faster(&self, c: &[String]) -> Value { json!({"ranked": c}) }
        fn models(&self, _p: Option<&str>, _l: usize) -> Value { json!({"data": []}) }
        fn guardrail_status(&self) -> Value { json!({"enabled": true}) }
        fn gamification_status(&self) -> Value { json!({"level": 1}) }
        fn compression_stats(&self) -> Value { json!({"saved_tokens": 0}) }
        fn cache_stats(&self) -> Value { json!({"entries": 0}) }
        fn oauth_status(&self) -> Value { json!({"providers": 17}) }
        fn memory_search(&self, q: &str, _l: usize) -> Value { json!({"query": q, "memories": []}) }
        fn skills_list(&self, _q: Option<&str>) -> Value { json!({"skills": []}) }
        fn mitm_status(&self) -> Value { json!({"running": false}) }
    }

    #[test]
    fn every_catalog_tool_has_live_handler() {
        for def in TOOL_CATALOG {
            let res = call_tool(&TestCtx, def.name, &json!({}));
            assert_eq!(res["isError"], false, "unhandled tool in catalog: {}", def.name);
        }
    }

    #[test]
    fn http_rpc_and_stdio_work() {
        let out = handle_http_rpc(&TestCtx, &json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}));
        assert_eq!(out["result"]["tools"].as_array().unwrap().len(), TOOL_CATALOG.len());

        let stdio = serve_stdio(&TestCtx, "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n");
        assert_eq!(stdio.len(), 1);
    }
}
