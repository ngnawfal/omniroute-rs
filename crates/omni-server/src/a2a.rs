// A2A v0.3 (+ v1.0 compatibility layer) Protocol implementation:
//   - GET  /.well-known/agent.json     — Dynamic Agent Card (6 built-in skills + SQLite skills)
//   - POST /a2a                        — JSON-RPC 2.0 router:
//       v0.3: message/send, message/stream (SSE), tasks/get, tasks/cancel
//       v1.0: SendMessage, SendStreamingMessage (TASK_STATE_* + status.message.parts[].text)
//   - GET  /api/a2a/status             — A2A telemetry & counts
//   - GET  /api/a2a/tasks              — List owner-scoped tasks (GHSA-jcm5-6wpp-wjj8)
//   - GET  /api/a2a/tasks/{id}         — Get single owner-scoped task
//   - POST /api/a2a/tasks/{id}/cancel  — Cancel owner-scoped task

use crate::api::build_health_json;
use crate::chat::{authorize_request, expand_and_order_candidates, extract_api_key};
use crate::part_a::authorize_management;
use crate::state::{hash_api_key, now_ms, SharedState};
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::Response;
use omni_store::A2ATaskRow;
use serde::Deserialize;
use serde_json::{json, Value};

const TASK_TTL_MS: i64 = 5 * 60 * 1000;

fn json_status(status: StatusCode, body: Value) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn json_rpc_result(id: &Value, result: Value) -> Response {
    json_status(
        StatusCode::OK,
        json!({"jsonrpc": "2.0", "id": id, "result": result}),
    )
}

fn json_rpc_error(id: &Value, status: StatusCode, code: i64, message: &str) -> Response {
    json_status(
        status,
        json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
    )
}

fn builtin_a2a_skills() -> Vec<Value> {
    vec![
        json!({
            "id": "smart-routing",
            "name": "Smart Request Routing",
            "description": "Routes AI requests to the optimal provider based on quota, cost, latency, and reliability.",
            "tags": ["routing", "llm", "optimization", "fallback"],
        }),
        json!({
            "id": "quota-management",
            "name": "Quota & Cost Management",
            "description": "Tracks API quotas, active model lockouts, and free-tier availability across providers.",
            "tags": ["quota", "cost", "monitoring", "budget"],
        }),
        json!({
            "id": "provider-discovery",
            "name": "Provider Discovery",
            "description": "Discovers providers among the embedded 394-provider catalog matching a capability or query.",
            "tags": ["providers", "discovery", "capabilities", "health"],
        }),
        json!({
            "id": "cost-analysis",
            "name": "Cost Analysis",
            "description": "Analyzes usage, token volume, cache savings, and cost breakdowns from SQLite.",
            "tags": ["cost", "usage", "analytics", "optimization"],
        }),
        json!({
            "id": "health-report",
            "name": "Health Report",
            "description": "Summarizes provider health, circuit breakers, OAuth refresh status, and cache/compression telemetry.",
            "tags": ["health", "monitoring", "resilience", "telemetry"],
        }),
        json!({
            "id": "list-capabilities",
            "name": "List Capabilities",
            "description": "Returns the full catalog of OmniRoute A2A agent skills and MCP tools.",
            "tags": ["discovery", "capabilities"],
        }),
    ]
}

// ── GET /.well-known/agent.json ─────────────────────────────────────

pub async fn agent_card(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost:20128");
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("http");
    let base_url = format!("{scheme}://{host}");

    let mut skills = builtin_a2a_skills();
    if let Ok(installed) = state.store.list_skills() {
        for sk in installed.into_iter().filter(|s| s.enabled && s.mode != "off") {
            skills.push(json!({
                "id": sk.name,
                "name": sk.name,
                "description": sk.description,
                "tags": sk.tags,
            }));
        }
    }

    let (prov_cnt, model_cnt) = {
        let cat = state.catalog.read().unwrap();
        (cat.provider_count(), cat.model_count())
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "public, max-age=3600")
        .body(Body::from(
            json!({
                "name": "OmniRoute AI Gateway (Rust)",
                "description": format!(
                    "Intelligent native Rust AI routing gateway synced with upstream v{} ({} providers, {} models), smart fallback, OAuth refresh, RTK+Caveman compression, semantic cache, and live MITM proxy.",
                    omni_catalog::upstream_version(),
                    prov_cnt,
                    model_cnt
                ),
                "url": format!("{base_url}/a2a"),
                "version": omni_catalog::upstream_version(),
                "protocolVersion": "0.3",
                "capabilities": {
                    "streaming": true,
                    "pushNotifications": false,
                },
                "skills": skills,
                "authentication": {
                    "schemes": ["api-key"],
                    "apiKeyHeader": "Authorization",
                }
            })
            .to_string(),
        ))
        .unwrap()
}

// ── Message parsing & Skill execution ───────────────────────────────

fn extract_a2a_messages(params: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(arr) = params.get("messages").and_then(Value::as_array) {
        for item in arr {
            let role = item
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("user")
                .to_string();
            if let Some(c) = item.get("content").and_then(Value::as_str) {
                if !c.trim().is_empty() {
                    out.push((role, c.to_string()));
                }
            }
        }
    }
    if out.is_empty() {
        if let Some(msg) = params.get("message") {
            let role = msg
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("user")
                .to_string();
            if let Some(c) = msg.get("content").and_then(Value::as_str) {
                if !c.trim().is_empty() {
                    out.push((role, c.to_string()));
                }
            } else if let Some(parts) = msg.get("parts").and_then(Value::as_array) {
                let text = parts
                    .iter()
                    .filter_map(|p| {
                        p.as_str()
                            .or_else(|| p.get("text").and_then(Value::as_str))
                            .or_else(|| p.get("content").and_then(Value::as_str))
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if !text.trim().is_empty() {
                    out.push((role, text));
                }
            }
        }
    }
    out
}

fn execute_a2a_skill(state: &SharedState, skill: &str, messages: &[(String, String)], params: &Value) -> (String, Vec<Value>, Value) {
    let prompt_text = messages
        .last()
        .map(|(_, c)| c.as_str())
        .unwrap_or("");

    match skill {
        "health-report" => {
            let health = build_health_json(state);
            let text = format!(
                "OmniRoute Rust v{} healthy: {} providers, {} models, {} configured credentials, {} active lockouts.",
                omni_catalog::upstream_version(),
                health["provider_count"],
                health["model_count"],
                health["configured_credentials"],
                health["active_lockouts"].as_array().map(|a| a.len()).unwrap_or(0),
            );
            (
                "completed".into(),
                vec![
                    json!({"type": "text", "content": text}),
                    json!({"type": "json", "content": health.to_string()}),
                ],
                json!({"skill": skill}),
            )
        }
        "cost-analysis" => {
            let usage = state.store.usage_summary().unwrap_or_else(|_| json!({}));
            let comp = state.compression_summary();
            let cache = state.cache_summary();
            let summary = json!({"usage": usage, "compression": comp, "cache": cache});
            let text = format!(
                "Total requests: {}, total tokens: {}, estimated cost: ${:.4}, compression saved: {} tokens.",
                usage["requests"].as_i64().unwrap_or(0),
                usage["total_tokens"].as_i64().unwrap_or(0),
                usage["cost_usd"].as_f64().unwrap_or(0.0),
                comp["stats"]["total_tokens_saved"].as_u64().unwrap_or(0),
            );
            (
                "completed".into(),
                vec![
                    json!({"type": "text", "content": text}),
                    json!({"type": "json", "content": summary.to_string()}),
                ],
                json!({"skill": skill}),
            )
        }
        "quota-management" => {
            let creds = state.store.list_credentials().unwrap_or_default();
            let lockouts = omni_exec::resilience::all_lockouts();
            let data = json!({
                "configured_accounts": creds.len(),
                "active_lockouts": lockouts.len(),
                "oauth": state.oauth_summary(),
            });
            let text = format!(
                "Configured accounts: {}, active lockouts: {}, connected OAuth accounts: {}.",
                creds.len(),
                lockouts.len(),
                data["oauth"]["connected_oauth_accounts"].as_u64().unwrap_or(0),
            );
            (
                "completed".into(),
                vec![
                    json!({"type": "text", "content": text}),
                    json!({"type": "json", "content": data.to_string()}),
                ],
                json!({"skill": skill}),
            )
        }
        "provider-discovery" => {
            let q = params
                .get("query")
                .and_then(Value::as_str)
                .unwrap_or(prompt_text)
                .to_ascii_lowercase();
            let cat = state.catalog.read().unwrap();
            let matched: Vec<Value> = cat
                .providers
                .iter()
                .filter(|p| {
                    q.is_empty()
                        || p.id.to_ascii_lowercase().contains(&q)
                        || p.name.as_deref().unwrap_or("").to_ascii_lowercase().contains(&q)
                        || (q.contains("free") && p.is_free_tier)
                })
                .take(25)
                .map(|p| json!({"id": p.id, "format": p.format, "free_tier": p.is_free_tier, "models": p.model_specs.len()}))
                .collect();
            let text = format!("Found {} matching providers (out of {} total).", matched.len(), cat.provider_count());
            (
                "completed".into(),
                vec![
                    json!({"type": "text", "content": text}),
                    json!({"type": "json", "content": json!(matched).to_string()}),
                ],
                json!({"skill": skill, "matched": matched.len()}),
            )
        }
        "list-capabilities" => {
            let skills = builtin_a2a_skills();
            let mcp_tools = omni_mcp::TOOL_CATALOG.len();
            let text = format!(
                "OmniRoute A2A exposes {} built-in skills and {} MCP tools.",
                skills.len(),
                mcp_tools
            );
            (
                "completed".into(),
                vec![
                    json!({"type": "text", "content": text}),
                    json!({"type": "json", "content": json!({"skills": skills, "mcp_tools": mcp_tools}).to_string()}),
                ],
                json!({"skill": skill}),
            )
        }
        "smart-routing" => {
            let model = params
                .get("model")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("openai/gpt-6.1-sol");
            let strat = params
                .get("strategy")
                .and_then(Value::as_str)
                .map(omni_router::normalize_strategy);
            let (used, is_combo, ordered) = expand_and_order_candidates(state, model, strat, 1_000);
            let top = ordered
                .first()
                .map(|t| format!("{}/{}", t.provider, t.model))
                .unwrap_or_else(|| "none".into());
            let text = format!(
                "Selected route `{top}` via strategy `{}` ({} candidate(s), combo={is_combo}) for prompt ({} chars).",
                used.as_str(),
                ordered.len(),
                prompt_text.len()
            );
            (
                "completed".into(),
                vec![
                    json!({"type": "text", "content": text}),
                    json!({
                        "type": "json",
                        "content": json!({
                            "selected": top,
                            "strategy": used.as_str(),
                            "candidate_count": ordered.len(),
                        }).to_string()
                    }),
                ],
                json!({"skill": skill, "strategy": used.as_str(), "selected": top}),
            )
        }
        custom_name => {
            if let Ok(Some(sk)) = state.store.get_skill(custom_name) {
                if sk.enabled && sk.mode != "off" {
                    let _ = state.store.record_skill_execution(&sk.id, now_ms());
                    let text = format!(
                        "Executed installed skill `{}`: {}",
                        sk.name,
                        if sk.instructions.is_empty() {
                            sk.description.as_str()
                        } else {
                            sk.instructions.as_str()
                        }
                    );
                    return (
                        "completed".into(),
                        vec![json!({"type": "text", "content": text})],
                        json!({"skill": sk.name, "source": sk.source_provider}),
                    );
                }
            }
            (
                "failed".into(),
                vec![json!({"type": "error", "content": format!("unknown or disabled skill: {custom_name}")})],
                json!({"skill": custom_name}),
            )
        }
    }
}

fn to_v1_task_response(task: &A2ATaskRow, context_id: Option<&str>) -> Value {
    let text = task
        .artifacts
        .iter()
        .filter(|a| a.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|a| a.get("content").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    let v1_state = format!("TASK_STATE_{}", task.state.to_ascii_uppercase());
    let mut obj = json!({
        "id": task.id,
        "status": {
            "state": v1_state,
            "message": {
                "role": "ROLE_AGENT",
                "parts": [{"text": text, "mediaType": "text/plain"}],
            }
        },
        "artifacts": [{
            "role": "ROLE_AGENT",
            "parts": [{"text": text, "mediaType": "text/plain"}],
        }]
    });
    if let Some(cid) = context_id.filter(|s| !s.is_empty()) {
        obj["contextId"] = Value::String(cid.to_string());
    }
    obj
}

// ── POST /a2a (JSON-RPC 2.0) ────────────────────────────────────────

pub async fn a2a_rpc(
    State(state): State<SharedState>,
    headers: HeaderMap,
    body_str: String,
) -> Response {
    let api_key = match authorize_request(&state, &headers, false) {
        Ok(k) => k,
        Err(_) => {
            return json_rpc_error(
                &Value::Null,
                StatusCode::UNAUTHORIZED,
                -32600,
                "Unauthorized: missing or invalid API key",
            );
        }
    };
    let owner_hash = api_key.as_deref().map(hash_api_key).unwrap_or_default();

    let body: Value = match serde_json::from_str(&body_str) {
        Ok(v) => v,
        Err(_) => {
            return json_rpc_error(
                &Value::Null,
                StatusCode::BAD_REQUEST,
                -32700,
                "Parse error: invalid JSON",
            );
        }
    };
    let id = body.get("id").cloned().unwrap_or(Value::Null);
    if body.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return json_rpc_error(
            &id,
            StatusCode::BAD_REQUEST,
            -32600,
            "Invalid request: jsonrpc must be 2.0",
        );
    }
    let Some(raw_method) = body.get("method").and_then(Value::as_str) else {
        return json_rpc_error(
            &id,
            StatusCode::BAD_REQUEST,
            -32600,
            "Invalid request: missing method",
        );
    };
    if !state.a2a_enabled() {
        return json_rpc_error(
            &id,
            StatusCode::SERVICE_UNAVAILABLE,
            -32000,
            "A2A endpoint is disabled via settings.",
        );
    }

    let params = body.get("params").cloned().unwrap_or_else(|| json!({}));
    let is_v1 = matches!(raw_method, "SendMessage" | "SendStreamingMessage");
    let method = match raw_method {
        "SendMessage" => "message/send",
        "SendStreamingMessage" => "message/stream",
        other => other,
    };

    match method {
        "message/send" => {
            let skill = params
                .get("skill")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("smart-routing");
            let messages = extract_a2a_messages(&params);
            if messages.is_empty() {
                return json_rpc_error(
                    &id,
                    StatusCode::BAD_REQUEST,
                    -32602,
                    "Invalid params: missing non-empty messages/message",
                );
            }
            let now = now_ms();
            let now_iso = chrono::Utc::now().to_rfc3339();
            let (final_state, artifacts, meta) = execute_a2a_skill(&state, skill, &messages, &params);
            let task = A2ATaskRow {
                id: uuid::Uuid::new_v4().to_string(),
                owner_hash,
                skill: skill.to_string(),
                state: final_state.clone(),
                input: json!({
                    "skill": skill,
                    "messages": messages.iter().map(|(r, c)| json!({"role": r, "content": c})).collect::<Vec<_>>()
                }),
                artifacts: artifacts.clone(),
                events: vec![
                    json!({"timestamp": now_iso, "state": "submitted"}),
                    json!({"timestamp": now_iso, "state": "working"}),
                    json!({"timestamp": now_iso, "state": final_state}),
                ],
                metadata: meta.clone(),
                created_at: now,
                updated_at: now,
                expires_at: now + TASK_TTL_MS,
            };
            let _ = state.store.upsert_a2a_task(&task);
            if is_v1 {
                let ctx_id = params.get("contextId").and_then(Value::as_str);
                json_rpc_result(&id, json!({"task": to_v1_task_response(&task, ctx_id)}))
            } else {
                json_rpc_result(
                    &id,
                    json!({
                        "task": {
                            "id": task.id,
                            "skill": task.skill,
                            "state": task.state,
                            "createdAt": now_iso,
                        },
                        "artifacts": task.artifacts,
                        "metadata": task.metadata,
                    }),
                )
            }
        }
        "message/stream" => {
            let skill = params
                .get("skill")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("smart-routing");
            let messages = extract_a2a_messages(&params);
            if messages.is_empty() {
                return json_rpc_error(
                    &id,
                    StatusCode::BAD_REQUEST,
                    -32602,
                    "Invalid params: missing non-empty messages/message",
                );
            }
            let now = now_ms();
            let now_iso = chrono::Utc::now().to_rfc3339();
            let (final_state, artifacts, meta) = execute_a2a_skill(&state, skill, &messages, &params);
            let task = A2ATaskRow {
                id: uuid::Uuid::new_v4().to_string(),
                owner_hash,
                skill: skill.to_string(),
                state: final_state.clone(),
                input: json!({"skill": skill}),
                artifacts: artifacts.clone(),
                events: vec![
                    json!({"timestamp": now_iso, "state": "submitted"}),
                    json!({"timestamp": now_iso, "state": final_state}),
                ],
                metadata: meta,
                created_at: now,
                updated_at: now,
                expires_at: now + TASK_TTL_MS,
            };
            let _ = state.store.upsert_a2a_task(&task);

            let ev1 = format!(
                "event: task.status\ndata: {}\n\n",
                json!({"jsonrpc":"2.0","id":id,"result":{"taskId":task.id,"state":"working"}})
            );
            let ev2 = format!(
                "event: task.artifact\ndata: {}\n\n",
                json!({"jsonrpc":"2.0","id":id,"result":{"taskId":task.id,"artifacts":task.artifacts}})
            );
            let ev3 = format!(
                "event: task.completed\ndata: {}\n\n",
                json!({"jsonrpc":"2.0","id":id,"result":{"taskId":task.id,"state":task.state}})
            );
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/event-stream")
                .header(header::CACHE_CONTROL, "no-cache")
                .body(Body::from(format!("{ev1}{ev2}{ev3}")))
                .unwrap()
        }
        "tasks/get" => {
            let Some(task_id) = params.get("id").and_then(Value::as_str) else {
                return json_rpc_error(&id, StatusCode::BAD_REQUEST, -32602, "missing params.id");
            };
            match state.store.get_a2a_task(task_id, &owner_hash) {
                Ok(Some(task)) => json_rpc_result(&id, serde_json::to_value(task).unwrap()),
                _ => json_rpc_error(&id, StatusCode::NOT_FOUND, -32001, "task not found"),
            }
        }
        "tasks/cancel" => {
            let Some(task_id) = params.get("id").and_then(Value::as_str) else {
                return json_rpc_error(&id, StatusCode::BAD_REQUEST, -32602, "missing params.id");
            };
            match state.store.get_a2a_task(task_id, &owner_hash) {
                Ok(Some(mut task)) => {
                    if matches!(task.state.as_str(), "submitted" | "working") {
                        task.state = "cancelled".into();
                        task.updated_at = now_ms();
                        task.events.push(json!({
                            "timestamp": chrono::Utc::now().to_rfc3339(),
                            "state": "cancelled"
                        }));
                        let _ = state.store.upsert_a2a_task(&task);
                    }
                    json_rpc_result(&id, serde_json::to_value(task).unwrap())
                }
                _ => json_rpc_error(&id, StatusCode::NOT_FOUND, -32001, "task not found"),
            }
        }
        other => json_rpc_error(
            &id,
            StatusCode::NOT_FOUND,
            -32601,
            &format!("method not found: {other}"),
        ),
    }
}

// ── REST A2A Management Endpoints (/api/a2a/*) ──────────────────────

pub async fn a2a_status(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    let api_key = extract_api_key(&headers);
    let owner = api_key.as_deref().map(hash_api_key).unwrap_or_default();
    let tasks = state.store.list_a2a_tasks(&owner, None, None, 200, 0).unwrap_or_default();
    let completed = tasks.iter().filter(|t| t.state == "completed").count();
    let failed = tasks.iter().filter(|t| t.state == "failed").count();
    let cancelled = tasks.iter().filter(|t| t.state == "cancelled").count();
    json_status(
        StatusCode::OK,
        json!({
            "enabled": state.a2a_enabled(),
            "protocolVersion": "0.3",
            "v1Compat": true,
            "builtinSkills": builtin_a2a_skills().len(),
            "totalTasks": tasks.len(),
            "counts": {
                "completed": completed,
                "failed": failed,
                "cancelled": cancelled,
            }
        }),
    )
}

#[derive(Debug, Deserialize, Default)]
pub struct TaskListQuery {
    pub state: Option<String>,
    pub skill: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

pub async fn list_a2a_tasks(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(q): Query<TaskListQuery>,
) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    let api_key = extract_api_key(&headers);
    let owner = api_key.as_deref().map(hash_api_key).unwrap_or_default();
    let tasks = state
        .store
        .list_a2a_tasks(
            &owner,
            q.state.as_deref(),
            q.skill.as_deref(),
            q.limit.unwrap_or(50).min(200),
            q.offset.unwrap_or(0),
        )
        .unwrap_or_default();
    json_status(StatusCode::OK, json!({"tasks": tasks, "count": tasks.len()}))
}

pub async fn get_a2a_task(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    let api_key = extract_api_key(&headers);
    let owner = api_key.as_deref().map(hash_api_key).unwrap_or_default();
    match state.store.get_a2a_task(&id, &owner) {
        Ok(Some(task)) => json_status(StatusCode::OK, serde_json::to_value(task).unwrap()),
        _ => json_status(StatusCode::NOT_FOUND, json!({"error": "task not found"})),
    }
}

pub async fn cancel_a2a_task(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    let api_key = extract_api_key(&headers);
    let owner = api_key.as_deref().map(hash_api_key).unwrap_or_default();
    match state.store.get_a2a_task(&id, &owner) {
        Ok(Some(mut task)) => {
            task.state = "cancelled".into();
            task.updated_at = now_ms();
            let _ = state.store.upsert_a2a_task(&task);
            json_status(StatusCode::OK, serde_json::to_value(task).unwrap())
        }
        _ => json_status(StatusCode::NOT_FOUND, json!({"error": "task not found"})),
    }
}
