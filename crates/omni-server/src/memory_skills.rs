// Persistent Agent Memory (/api/memory/*) & Skills Registry (/api/skills/*) REST APIs.

use crate::chat::extract_api_key;
use crate::part_a::authorize_management;
use crate::state::{hash_api_key, now_ms, SharedState};
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::Response;
use omni_store::{MemoryRow, SkillRow};
use serde::Deserialize;
use serde_json::{json, Value};

fn json_status(status: StatusCode, body: Value) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

// ── Memory Endpoints (/api/memory/*) ────────────────────────────────

#[derive(Debug, Deserialize, Default)]
pub struct ListMemoryQuery {
    pub q: Option<String>,
    #[serde(rename = "type")]
    pub mem_type: Option<String>,
    #[serde(rename = "sessionId")]
    pub session_id: Option<String>,
    pub category: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

pub async fn list_memories(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(q): Query<ListMemoryQuery>,
) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    let key = extract_api_key(&headers);
    let kh = key.as_deref().map(hash_api_key);
    let limit = q.limit.unwrap_or(50).min(500);
    let offset = q.offset.unwrap_or(0);
    match state.store.list_memories(
        kh.as_deref(),
        q.session_id.as_deref(),
        q.mem_type.as_deref(),
        q.category.as_deref(),
        q.q.as_deref(),
        now_ms(),
        limit,
        offset,
    ) {
        Ok((items, total, tokens_used)) => json_status(
            StatusCode::OK,
            json!({
                "data": items,
                "total": total,
                "limit": limit,
                "offset": offset,
                "stats": {
                    "total": total,
                    "tokensUsed": tokens_used,
                }
            }),
        ),
        Err(e) => json_status(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error": e.to_string()}),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateMemoryBody {
    pub key: String,
    pub content: String,
    #[serde(rename = "type", default = "default_factual")]
    pub mem_type: String,
    #[serde(rename = "sessionId", default)]
    pub session_id: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub metadata: Value,
    #[serde(rename = "expiresAt")]
    pub expires_at: Option<i64>,
}

fn default_factual() -> String {
    "factual".into()
}

pub async fn create_memory(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<CreateMemoryBody>,
) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    let key = extract_api_key(&headers);
    if body.key.trim().is_empty() || body.content.trim().is_empty() {
        return json_status(
            StatusCode::BAD_REQUEST,
            json!({"error": "key and content are required"}),
        );
    }
    let now = now_ms();
    let id = uuid::Uuid::new_v4().to_string();
    let row = MemoryRow {
        id: id.clone(),
        api_key_hash: key.as_deref().map(hash_api_key).unwrap_or_default(),
        session_id: body.session_id,
        mem_type: body.mem_type,
        mem_key: body.key,
        content: body.content,
        category: body.category,
        metadata: if body.metadata.is_null() { json!({}) } else { body.metadata },
        tokens_est: 0,
        expires_at: body.expires_at,
        created_at: now,
        updated_at: now,
    };
    match state.store.upsert_memory(&row) {
        Ok(()) => json_status(StatusCode::CREATED, json!({"success": true, "id": id, "memory": row})),
        Err(e) => json_status(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error": e.to_string()}),
        ),
    }
}

pub async fn get_memory(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    match state.store.get_memory(&id) {
        Ok(Some(m)) => json_status(StatusCode::OK, serde_json::to_value(m).unwrap()),
        _ => json_status(StatusCode::NOT_FOUND, json!({"error": "memory not found"})),
    }
}

pub async fn update_memory(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    let Ok(Some(mut existing)) = state.store.get_memory(&id) else {
        return json_status(StatusCode::NOT_FOUND, json!({"error": "memory not found"}));
    };
    if let Some(c) = body.get("content").and_then(Value::as_str) {
        existing.content = c.to_string();
        existing.tokens_est = 0;
    }
    if let Some(k) = body.get("key").and_then(Value::as_str) {
        existing.mem_key = k.to_string();
    }
    if let Some(cat) = body.get("category").and_then(Value::as_str) {
        existing.category = cat.to_string();
    }
    if let Some(mt) = body.get("type").and_then(Value::as_str) {
        existing.mem_type = mt.to_string();
    }
    existing.updated_at = now_ms();
    match state.store.upsert_memory(&existing) {
        Ok(()) => json_status(StatusCode::OK, serde_json::to_value(existing).unwrap()),
        Err(e) => json_status(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error": e.to_string()}),
        ),
    }
}

pub async fn delete_memory(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    let deleted = state.store.delete_memory(&id).unwrap_or(false);
    json_status(StatusCode::OK, json!({"deleted": deleted, "id": id}))
}

#[derive(Debug, Deserialize)]
pub struct RetrieveMemoryBody {
    pub query: String,
    #[serde(rename = "sessionId")]
    pub session_id: Option<String>,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
}

fn default_top_k() -> usize {
    5
}

pub async fn retrieve_memories(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<RetrieveMemoryBody>,
) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    let key = extract_api_key(&headers);
    let kh = key.as_deref().map(hash_api_key);
    let hits = state
        .store
        .retrieve_memories(
            &body.query,
            kh.as_deref(),
            body.session_id.as_deref(),
            now_ms(),
            body.top_k.clamp(1, 50),
        )
        .unwrap_or_default();
    json_status(
        StatusCode::OK,
        json!({
            "query": body.query,
            "matches": hits.into_iter().map(|(m, score)| json!({
                "score": score,
                "memory": m,
            })).collect::<Vec<_>>()
        }),
    )
}

pub async fn memory_health(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    let (_, total, tokens_used) = state
        .store
        .list_memories(None, None, None, None, None, now_ms(), 1, 0)
        .unwrap_or((vec![], 0, 0));
    json_status(
        StatusCode::OK,
        json!({
            "status": "ok",
            "engine": "sqlite-wal-tf-cosine",
            "totalMemories": total,
            "tokensUsed": tokens_used,
        }),
    )
}

// ── Skills Endpoints (/api/skills/*) ────────────────────────────────

#[derive(Debug, Deserialize, Default)]
pub struct ListSkillsQuery {
    pub q: Option<String>,
    pub mode: Option<String>,
    pub source: Option<String>,
}

pub async fn list_skills(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(q): Query<ListSkillsQuery>,
) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    let ql = q.q.as_deref().map(|s| s.to_ascii_lowercase());
    let skills: Vec<SkillRow> = state
        .store
        .list_skills()
        .unwrap_or_default()
        .into_iter()
        .filter(|s| {
            if let Some(ref m) = q.mode {
                if !s.mode.eq_ignore_ascii_case(m) {
                    return false;
                }
            }
            if let Some(ref src) = q.source {
                if !s.source_provider.eq_ignore_ascii_case(src) {
                    return false;
                }
            }
            if let Some(ref text) = ql {
                let hit = s.name.to_ascii_lowercase().contains(text)
                    || s.description.to_ascii_lowercase().contains(text)
                    || s.tags.iter().any(|t| t.to_ascii_lowercase().contains(text));
                if !hit {
                    return false;
                }
            }
            true
        })
        .collect();
    json_status(
        StatusCode::OK,
        json!({
            "total": skills.len(),
            "skills": skills,
            "popularDefaults": ["web-search", "file-reader", "sql-assistant", "devops-helper", "docs-assistant"],
        }),
    )
}

#[derive(Debug, Deserialize)]
pub struct UpsertSkillBody {
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_local")]
    pub source_provider: String,
    #[serde(default = "default_on")]
    pub mode: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub instructions: String,
    #[serde(default)]
    pub schema: Value,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_local() -> String {
    "local".into()
}
fn default_on() -> String {
    "on".into()
}
fn default_true() -> bool {
    true
}

pub async fn upsert_skill(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<UpsertSkillBody>,
) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    if body.name.trim().is_empty() {
        return json_status(StatusCode::BAD_REQUEST, json!({"error": "name is required"}));
    }
    let now = now_ms();
    let row = SkillRow {
        id: body
            .id
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        name: body.name,
        description: body.description,
        source_provider: body.source_provider,
        mode: body.mode,
        tags: body.tags,
        instructions: body.instructions,
        schema: if body.schema.is_null() { json!({}) } else { body.schema },
        enabled: body.enabled,
        executions_count: 0,
        created_at: now,
        updated_at: now,
    };
    match state.store.upsert_skill(&row) {
        Ok(()) => json_status(StatusCode::CREATED, serde_json::to_value(row).unwrap()),
        Err(e) => json_status(StatusCode::BAD_REQUEST, json!({"error": e.to_string()})),
    }
}

pub async fn get_skill(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    match state.store.get_skill(&id) {
        Ok(Some(sk)) => json_status(StatusCode::OK, serde_json::to_value(sk).unwrap()),
        _ => json_status(StatusCode::NOT_FOUND, json!({"error": "skill not found"})),
    }
}

pub async fn delete_skill(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    let deleted = state.store.delete_skill(&id).unwrap_or(false);
    json_status(StatusCode::OK, json!({"deleted": deleted, "id": id}))
}

pub async fn execute_skill(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Option<axum::Json<Value>>,
) -> Response {
    if let Err(resp) = authorize_management(&state, &headers) {
        return resp;
    }
    let Ok(Some(sk)) = state.store.get_skill(&id) else {
        return json_status(StatusCode::NOT_FOUND, json!({"error": "skill not found"}));
    };
    if !sk.enabled || sk.mode == "off" {
        return json_status(StatusCode::FORBIDDEN, json!({"error": "skill is disabled"}));
    }
    let _ = state.store.record_skill_execution(&sk.id, now_ms());
    let input = body.map(|b| b.0).unwrap_or_else(|| json!({}));
    json_status(
        StatusCode::OK,
        json!({
            "ok": true,
            "skill": sk.name,
            "mode": sk.mode,
            "executions_count": sk.executions_count + 1,
            "rendered_prompt": sk.instructions,
            "input": input,
        }),
    )
}
