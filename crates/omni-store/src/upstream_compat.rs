// Upstream OmniRoute SQLite (`storage.sqlite`) & legacy `db.json` compatibility bridge,
// portable JSON snapshot export/restore, managed SQLite backup, and integrity diagnostics.

use crate::{ComboRow, ComboStep, CredentialRow, MemoryRow, SkillRow, Store, UsageEntry};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportReport {
    pub source_kind: String,
    pub provider_connections: usize,
    pub custom_providers: usize,
    pub combos: usize,
    pub api_keys: usize,
    pub settings: usize,
    pub aliases: usize,
    pub usage_rows: usize,
    pub memories: usize,
    pub skills: usize,
    pub upstream_migrations_detected: usize,
    pub warnings: Vec<String>,
}

fn parse_timestamp_ms(raw: Option<&str>) -> Option<i64> {
    let s = raw?.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(n) = s.parse::<i64>() {
        return Some(if n < 10_000_000_000 { n * 1000 } else { n });
    }
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_millis())
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|ndt| ndt.and_utc().timestamp_millis())
        })
}

fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
        rusqlite::params![table],
        |_| Ok(()),
    )
    .is_ok()
}

fn table_columns(conn: &Connection, table: &str) -> Vec<String> {
    let Ok(mut stmt) = conn.prepare(&format!("PRAGMA table_info({table})")) else {
        return vec![];
    };
    stmt.query_map([], |r| r.get::<_, String>(1))
        .ok()
        .map(|it| it.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

fn col_or_null(cols: &[String], name: &str) -> String {
    if cols.iter().any(|c| c == name) {
        name.to_string()
    } else {
        format!("NULL AS {name}")
    }
}

fn parse_combo_steps(val: &Value) -> Vec<ComboStep> {
    let Some(arr) = val.as_array() else {
        return vec![];
    };
    arr.iter()
        .filter_map(|item| {
            if let Some(s) = item.as_str() {
                let (prov, model) = s.split_once('/').unwrap_or(("openai", s));
                return Some(ComboStep {
                    provider: prov.to_string(),
                    model: model.to_string(),
                    weight: 1.0,
                    fallback_only: false,
                    connection_id: None,
                });
            }
            let obj = item.as_object()?;
            let raw_model = obj.get("model").and_then(Value::as_str).unwrap_or("");
            let (provider, model) = if let Some(p) = obj.get("provider").and_then(Value::as_str) {
                (p.to_string(), raw_model.to_string())
            } else if let Some((p, m)) = raw_model.split_once('/') {
                (p.to_string(), m.to_string())
            } else {
                return None;
            };
            Some(ComboStep {
                provider,
                model,
                weight: obj.get("weight").and_then(Value::as_f64).unwrap_or(1.0),
                fallback_only: obj
                    .get("fallbackOnly")
                    .or_else(|| obj.get("fallback_only"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                connection_id: obj
                    .get("connectionId")
                    .or_else(|| obj.get("connection_id"))
                    .and_then(Value::as_str)
                    .map(String::from),
            })
        })
        .collect()
}

impl Store {
    /// Inspect SQLite WAL health, integrity check, schema version, and row counts across all tables.
    pub fn db_diagnostics(&self) -> anyhow::Result<Value> {
        let conn = self.conn.lock().unwrap();
        let integrity: String = conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .unwrap_or_else(|e| format!("error: {e}"));
        let journal_mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap_or_default();
        let page_count: i64 = conn
            .query_row("PRAGMA page_count", [], |r| r.get(0))
            .unwrap_or(0);
        let page_size: i64 = conn
            .query_row("PRAGMA page_size", [], |r| r.get(0))
            .unwrap_or(4096);
        let applied_migrations: i64 = conn
            .query_row("SELECT COUNT(*) FROM _migration_meta", [], |r| r.get(0))
            .unwrap_or(0);

        let tables = [
            "api_keys",
            "provider_credentials",
            "usage",
            "custom_providers",
            "combos",
            "settings",
            "gamification_state",
            "guardrail_events",
            "model_aliases",
            "memories",
            "skills",
            "a2a_tasks",
            "call_logs",
            "domain_budgets",
            "key_templates",
        ];
        let mut counts = serde_json::Map::new();
        for t in tables {
            let cnt: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0))
                .unwrap_or(0);
            counts.insert(t.to_string(), json!(cnt));
        }

        Ok(json!({
            "integrity": integrity,
            "ok": integrity.eq_ignore_ascii_case("ok"),
            "journal_mode": journal_mode,
            "size_bytes": page_count * page_size,
            "applied_migrations": applied_migrations,
            "table_counts": counts,
        }))
    }

    /// Create an atomic hot backup of the SQLite database using `VACUUM INTO`.
    pub fn backup_to_file(&self, dest_path: &str) -> anyhow::Result<u64> {
        let p = Path::new(dest_path);
        if let Some(parent) = p.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        if p.exists() {
            let _ = std::fs::remove_file(p);
        }
        let escaped = dest_path.replace('\'', "''");
        {
            let conn = self.conn.lock().unwrap();
            conn.execute_batch(&format!("VACUUM INTO '{escaped}';"))?;
        }
        Ok(std::fs::metadata(p)?.len())
    }

    /// Export full portable JSON snapshot (compatible with `import_upstream_json` & restore).
    pub fn export_snapshot(&self, include_secrets: bool) -> anyhow::Result<Value> {
        let creds: Vec<Value> = self
            .list_credentials()?
            .into_iter()
            .map(|mut c| {
                if !include_secrets {
                    c.api_key = if c.api_key.is_empty() {
                        String::new()
                    } else {
                        "[REDACTED]".into()
                    };
                    if c.refresh_token.is_some() {
                        c.refresh_token = Some("[REDACTED]".into());
                    }
                }
                serde_json::to_value(c).unwrap_or(json!({}))
            })
            .collect();
        let keys: Vec<Value> = self
            .list_api_key_rows()?
            .into_iter()
            .map(|mut k| {
                if !include_secrets {
                    k.key = "[REDACTED]".into();
                }
                serde_json::to_value(k).unwrap_or(json!({}))
            })
            .collect();
        let combos = self.list_combos()?;
        let settings = self.list_settings()?;
        let aliases = self.list_model_aliases()?;
        let custom_providers = self.list_custom_providers()?;
        let (memories, _, _) =
            self.list_memories(None, None, None, None, None, chrono::Utc::now().timestamp_millis(), 10_000, 0)?;
        let skills = self.list_skills()?;
        let usage = self.usage_summary()?;

        Ok(json!({
            "schema": "omniroute-rs-snapshot-v1",
            "upstream_sync_version": omni_catalog::upstream_version(),
            "exported_at": chrono::Utc::now().to_rfc3339(),
            "include_secrets": include_secrets,
            "providerConnections": creds,
            "customProviders": custom_providers,
            "combos": combos,
            "apiKeys": keys,
            "settings": settings,
            "modelAliases": aliases,
            "memories": memories,
            "skills": skills,
            "usageSummary": usage,
        }))
    }

    /// Import directly from an upstream Node.js OmniRoute `storage.sqlite` database file.
    /// Reads `provider_connections`, `provider_nodes`, `combos`, `api_keys`, `key_value`,
    /// `usage_history`, `memories`, and `skills` non-destructively (read-only on source).
    pub fn import_upstream_sqlite(&self, sqlite_path: &str) -> anyhow::Result<ImportReport> {
        let src = Connection::open_with_flags(sqlite_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let mut report = ImportReport {
            source_kind: "upstream_storage_sqlite".into(),
            ..Default::default()
        };
        let now_ms = chrono::Utc::now().timestamp_millis();

        if table_exists(&src, "_omniroute_migrations") {
            report.upstream_migrations_detected = src
                .query_row("SELECT COUNT(*) FROM _omniroute_migrations", [], |r| r.get::<_, i64>(0))
                .unwrap_or(0) as usize;
        }

        // 1. provider_connections -> provider_credentials
        if table_exists(&src, "provider_connections") {
            let cols = table_columns(&src, "provider_connections");
            let sql = format!(
                "SELECT {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {} FROM provider_connections",
                col_or_null(&cols, "id"),
                col_or_null(&cols, "provider"),
                col_or_null(&cols, "auth_type"),
                col_or_null(&cols, "name"),
                col_or_null(&cols, "display_name"),
                col_or_null(&cols, "priority"),
                col_or_null(&cols, "is_active"),
                col_or_null(&cols, "api_key"),
                col_or_null(&cols, "access_token"),
                col_or_null(&cols, "refresh_token"),
                col_or_null(&cols, "expires_at"),
                col_or_null(&cols, "provider_specific_data"),
                col_or_null(&cols, "health_check_interval"),
            );
            if let Ok(mut stmt) = src.prepare(&sql) {
                let iter = stmt.query_map([], |r| {
                    let id: String = r.get::<_, Option<String>>(0)?.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                    let provider: String = r.get::<_, Option<String>>(1)?.unwrap_or_default();
                    let auth_type: String = r.get::<_, Option<String>>(2)?.unwrap_or_else(|| "apikey".into());
                    let name: String = r.get::<_, Option<String>>(4)?.or(r.get::<_, Option<String>>(3)?).unwrap_or_default();
                    let priority: i32 = r.get::<_, Option<i32>>(5)?.unwrap_or(0);
                    let is_active: i64 = r.get::<_, Option<i64>>(6)?.unwrap_or(1);
                    let api_key_col: Option<String> = r.get(7)?;
                    let access_tok_col: Option<String> = r.get(8)?;
                    let refresh_token: Option<String> = r.get(9)?;
                    let expires_raw: Option<String> = r.get(10)?;
                    let psj_raw: Option<String> = r.get(11)?;
                    let hc_interval: i64 = r.get::<_, Option<i64>>(12)?.unwrap_or(60);
                    Ok((id, provider, auth_type, name, priority, is_active, api_key_col, access_tok_col, refresh_token, expires_raw, psj_raw, hc_interval))
                })?;
                for item in iter.flatten() {
                    let (id, provider, auth_type, label, priority, is_active, ak, at, rt, exp_raw, psj_raw, hc_interval) = item;
                    if provider.is_empty() {
                        continue;
                    }
                    let secret = at
                        .filter(|s| !s.is_empty())
                        .or(ak)
                        .unwrap_or_default();
                    let psj: Value = psj_raw
                        .as_deref()
                        .and_then(|s| serde_json::from_str(s).ok())
                        .unwrap_or_else(|| json!({}));
                    let row = CredentialRow {
                        provider,
                        connection_id: id,
                        auth_type,
                        api_key: secret,
                        base_urls: vec![],
                        models: vec![],
                        enabled: is_active != 0,
                        priority,
                        weight: 1.0,
                        label,
                        refresh_token: rt.filter(|s| !s.is_empty()),
                        expires_at: parse_timestamp_ms(exp_raw.as_deref()),
                        provider_specific_json: psj,
                        health_status: "healthy".into(),
                        last_refreshed_at: Some(now_ms),
                        health_check_interval_min: hc_interval,
                    };
                    if self.upsert_credential_full(&row).is_ok() {
                        report.provider_connections += 1;
                    }
                }
            }
        }

        // 2. provider_nodes -> custom_providers
        if table_exists(&src, "provider_nodes") {
            let cols = table_columns(&src, "provider_nodes");
            let sql = format!(
                "SELECT {}, {}, {}, {}, {} FROM provider_nodes",
                col_or_null(&cols, "id"),
                col_or_null(&cols, "api_type"),
                col_or_null(&cols, "base_url"),
                col_or_null(&cols, "chat_path"),
                col_or_null(&cols, "name"),
            );
            if let Ok(mut stmt) = src.prepare(&sql) {
                if let Ok(rows) = stmt.query_map([], |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                        r.get::<_, Option<String>>(1)?.unwrap_or_else(|| "openai".into()),
                        r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                        r.get::<_, Option<String>>(3)?.unwrap_or_else(|| "/v1/chat/completions".into()),
                    ))
                }) {
                    for (id, fmt, base_url, chat_path) in rows.flatten() {
                        if id.is_empty() || base_url.is_empty() {
                            continue;
                        }
                        if self
                            .upsert_custom_provider(
                                &id,
                                &[base_url],
                                "bearer",
                                &chat_path,
                                &fmt,
                                &["*".into()],
                            )
                            .is_ok()
                        {
                            report.custom_providers += 1;
                        }
                    }
                }
            }
        }

        // 3. combos -> combos
        if table_exists(&src, "combos") {
            if let Ok(mut stmt) = src.prepare("SELECT id, name, data FROM combos") {
                if let Ok(rows) = stmt.query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                }) {
                    for (id, name, data_str) in rows.flatten() {
                        let parsed: Value = serde_json::from_str(&data_str).unwrap_or(json!({}));
                        let strategy = parsed
                            .get("strategy")
                            .and_then(Value::as_str)
                            .unwrap_or("priority")
                            .to_string();
                        let auto_strategy = parsed
                            .get("autoStrategy")
                            .or_else(|| parsed.get("auto_strategy"))
                            .and_then(Value::as_str)
                            .unwrap_or("rules")
                            .to_string();
                        let models = parsed
                            .get("models")
                            .map(parse_combo_steps)
                            .unwrap_or_default();
                        let enabled = parsed
                            .get("isActive")
                            .or_else(|| parsed.get("enabled"))
                            .and_then(Value::as_bool)
                            .unwrap_or(true);
                        let row = ComboRow {
                            id,
                            name,
                            strategy,
                            auto_strategy,
                            models,
                            config: parsed.get("config").cloned().unwrap_or(json!({})),
                            enabled,
                            created_at: now_ms,
                            updated_at: now_ms,
                        };
                        if self.upsert_combo(&row).is_ok() {
                            report.combos += 1;
                        }
                    }
                }
            }
        }

        // 4. api_keys -> api_keys
        if table_exists(&src, "api_keys") {
            let cols = table_columns(&src, "api_keys");
            let sql = format!(
                "SELECT {}, {}, {} FROM api_keys",
                col_or_null(&cols, "key"),
                col_or_null(&cols, "name"),
                col_or_null(&cols, "created_at"),
            );
            if let Ok(mut stmt) = src.prepare(&sql) {
                if let Ok(rows) = stmt.query_map([], |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                        r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                        r.get::<_, Option<String>>(2)?,
                    ))
                }) {
                    for (key, label, created_raw) in rows.flatten() {
                        if key.is_empty() {
                            continue;
                        }
                        let ts = parse_timestamp_ms(created_raw.as_deref()).unwrap_or(now_ms);
                        if self.add_api_key(&key, &label, ts).is_ok() {
                            report.api_keys += 1;
                        }
                    }
                }
            }
        }

        // 5. key_value -> settings & model_aliases
        if table_exists(&src, "key_value") {
            if let Ok(mut stmt) = src.prepare("SELECT namespace, key, value FROM key_value") {
                if let Ok(rows) = stmt.query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                }) {
                    for (ns, k, v) in rows.flatten() {
                        match ns.as_str() {
                            "settings" => {
                                let cleaned = serde_json::from_str::<Value>(&v)
                                    .ok()
                                    .and_then(|val| val.as_str().map(String::from))
                                    .unwrap_or(v);
                                if self.set_setting(&k, &cleaned, now_ms).is_ok() {
                                    report.settings += 1;
                                }
                            }
                            "modelAliases" | "aliases" => {
                                let target = serde_json::from_str::<Value>(&v)
                                    .ok()
                                    .and_then(|val| val.as_str().map(String::from))
                                    .unwrap_or(v);
                                if self.set_model_alias(&k, &target).is_ok() {
                                    report.aliases += 1;
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        // 6. usage_history -> usage (most recent 5,000 rows)
        if table_exists(&src, "usage_history") {
            let cols = table_columns(&src, "usage_history");
            let sql = format!(
                "SELECT {}, {}, {}, {}, {}, {}, {}, {}, {}, {} FROM usage_history ORDER BY id DESC LIMIT 5000",
                col_or_null(&cols, "provider"),
                col_or_null(&cols, "model"),
                col_or_null(&cols, "connection_id"),
                col_or_null(&cols, "tokens_input"),
                col_or_null(&cols, "tokens_output"),
                col_or_null(&cols, "tokens_cache_read"),
                col_or_null(&cols, "tokens_cache_creation"),
                col_or_null(&cols, "latency_ms"),
                col_or_null(&cols, "success"),
                col_or_null(&cols, "timestamp"),
            );
            if let Ok(mut stmt) = src.prepare(&sql) {
                if let Ok(rows) = stmt.query_map([], |r| {
                    let pt: i64 = r.get::<_, Option<i64>>(3)?.unwrap_or(0);
                    let ct: i64 = r.get::<_, Option<i64>>(4)?.unwrap_or(0);
                    let cr: i64 = r.get::<_, Option<i64>>(5)?.unwrap_or(0);
                    let cw: i64 = r.get::<_, Option<i64>>(6)?.unwrap_or(0);
                    let lat: i64 = r.get::<_, Option<i64>>(7)?.unwrap_or(0);
                    let ok: i64 = r.get::<_, Option<i64>>(8)?.unwrap_or(1);
                    let ts_raw: Option<String> = r.get(9)?;
                    Ok(UsageEntry {
                        ts_ms: parse_timestamp_ms(ts_raw.as_deref()).unwrap_or(now_ms),
                        provider: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                        connection_id: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                        model: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                        status: if ok != 0 { 200 } else { 500 },
                        prompt_tokens: pt,
                        completion_tokens: ct,
                        total_tokens: pt + ct,
                        cache_read_tokens: cr,
                        cache_write_tokens: cw,
                        latency_ms: lat,
                        cost_usd: 0.0,
                        api_key_hash: String::new(),
                        session_id: String::new(),
                    })
                }) {
                    for entry in rows.flatten() {
                        if self.record_usage_full(&entry).is_ok() {
                            report.usage_rows += 1;
                        }
                    }
                }
            }
        }

        Ok(report)
    }

    /// Import from either a legacy upstream `db.json` or an `omniroute-rs` exported JSON snapshot.
    pub fn import_upstream_json(&self, root: &Value) -> anyhow::Result<ImportReport> {
        let mut report = ImportReport {
            source_kind: "json_snapshot".into(),
            ..Default::default()
        };
        let now_ms = chrono::Utc::now().timestamp_millis();

        if let Some(arr) = root.get("providerConnections").and_then(Value::as_array) {
            for item in arr {
                if let Ok(row) = serde_json::from_value::<CredentialRow>(item.clone()) {
                    if row.api_key == "[REDACTED]" {
                        report.warnings.push(format!(
                            "skipped redacted credential {}/{}",
                            row.provider, row.connection_id
                        ));
                        continue;
                    }
                    if self.upsert_credential_full(&row).is_ok() {
                        report.provider_connections += 1;
                    }
                    continue;
                }
                // Upstream legacy db.json camelCase shape
                let Some(provider) = item.get("provider").and_then(Value::as_str) else {
                    continue;
                };
                let conn_id = item
                    .get("id")
                    .or_else(|| item.get("connection_id"))
                    .and_then(Value::as_str)
                    .map(String::from)
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                let secret = item
                    .get("accessToken")
                    .or_else(|| item.get("apiKey"))
                    .or_else(|| item.get("api_key"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let row = CredentialRow {
                    provider: provider.to_string(),
                    connection_id: conn_id,
                    auth_type: item
                        .get("authType")
                        .and_then(Value::as_str)
                        .unwrap_or("apikey")
                        .to_string(),
                    api_key: secret,
                    base_urls: vec![],
                    models: vec![],
                    enabled: item.get("isActive").and_then(Value::as_bool).unwrap_or(true),
                    priority: item.get("priority").and_then(Value::as_i64).unwrap_or(0) as i32,
                    weight: 1.0,
                    label: item
                        .get("name")
                        .or_else(|| item.get("displayName"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    refresh_token: item
                        .get("refreshToken")
                        .and_then(Value::as_str)
                        .map(String::from),
                    expires_at: parse_timestamp_ms(item.get("expiresAt").and_then(Value::as_str)),
                    provider_specific_json: item
                        .get("providerSpecificData")
                        .cloned()
                        .unwrap_or(json!({})),
                    health_status: "healthy".into(),
                    last_refreshed_at: Some(now_ms),
                    health_check_interval_min: 60,
                };
                if self.upsert_credential_full(&row).is_ok() {
                    report.provider_connections += 1;
                }
            }
        }

        if let Some(arr) = root.get("combos").and_then(Value::as_array) {
            for item in arr {
                if let Ok(row) = serde_json::from_value::<ComboRow>(item.clone()) {
                    if self.upsert_combo(&row).is_ok() {
                        report.combos += 1;
                    }
                    continue;
                }
                let Some(name) = item.get("name").and_then(Value::as_str) else {
                    continue;
                };
                let row = ComboRow {
                    id: item
                        .get("id")
                        .and_then(Value::as_str)
                        .map(String::from)
                        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                    name: name.to_string(),
                    strategy: item
                        .get("strategy")
                        .and_then(Value::as_str)
                        .unwrap_or("priority")
                        .to_string(),
                    auto_strategy: item
                        .get("autoStrategy")
                        .and_then(Value::as_str)
                        .unwrap_or("rules")
                        .to_string(),
                    models: item.get("models").map(parse_combo_steps).unwrap_or_default(),
                    config: item.get("config").cloned().unwrap_or(json!({})),
                    enabled: item.get("isActive").and_then(Value::as_bool).unwrap_or(true),
                    created_at: now_ms,
                    updated_at: now_ms,
                };
                if self.upsert_combo(&row).is_ok() {
                    report.combos += 1;
                }
            }
        }

        if let Some(arr) = root.get("apiKeys").and_then(Value::as_array) {
            for item in arr {
                let Some(key) = item.get("key").and_then(Value::as_str) else {
                    continue;
                };
                if key.is_empty() || key == "[REDACTED]" {
                    continue;
                }
                let label = item
                    .get("label")
                    .or_else(|| item.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let rpm = item.get("rpm_limit").and_then(Value::as_u64).map(|v| v as u32);
                let budget = item.get("budget_5h_usd").and_then(Value::as_f64);
                let exp = item.get("expires_at").and_then(Value::as_i64);
                if self
                    .add_api_key_with_limits(key, label, now_ms, exp, budget, rpm)
                    .is_ok()
                {
                    report.api_keys += 1;
                }
            }
        }

        if let Some(obj) = root.get("settings").and_then(Value::as_object) {
            for (k, v) in obj {
                let s = match v {
                    Value::String(st) => st.clone(),
                    other => other.to_string(),
                };
                if self.set_setting(k, &s, now_ms).is_ok() {
                    report.settings += 1;
                }
            }
        }

        if let Some(obj) = root.get("modelAliases").and_then(Value::as_object) {
            for (alias, target) in obj {
                if let Some(t) = target.as_str() {
                    if self.set_model_alias(alias, t).is_ok() {
                        report.aliases += 1;
                    }
                }
            }
        }

        if let Some(arr) = root.get("memories").and_then(Value::as_array) {
            for item in arr {
                if let Ok(m) = serde_json::from_value::<MemoryRow>(item.clone()) {
                    if self.upsert_memory(&m).is_ok() {
                        report.memories += 1;
                    }
                }
            }
        }

        if let Some(arr) = root.get("skills").and_then(Value::as_array) {
            for item in arr {
                if let Ok(sk) = serde_json::from_value::<SkillRow>(item.clone()) {
                    if self.upsert_skill(&sk).is_ok() {
                        report.skills += 1;
                    }
                }
            }
        }

        Ok(report)
    }

    pub fn list_model_aliases(&self) -> anyhow::Result<HashMap<String, String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT alias, target FROM model_aliases ORDER BY alias")?;
        let mut out = HashMap::new();
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for r in rows {
            let (a, t) = r?;
            out.insert(a, t);
        }
        Ok(out)
    }
}
