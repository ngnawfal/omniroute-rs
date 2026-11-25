// SQLite store: api keys (auth + limits + expiry), provider credentials (multi-account + OAuth),
// combos, settings, usage analytics, custom providers, gamification state, guardrail audit events,
// model aliases, memories, skills, A2A tasks, call_logs, domain_budgets, key_templates,
// plus direct upstream Node.js `storage.sqlite` & `db.json` import/export bridge.
// Single-file DB at data/omniroute-rs.db, WAL journaling.

pub mod upstream_compat;
pub use upstream_compat::ImportReport;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

const BASE_MIGRATIONS: &[&str] = &[
    // 1 — api keys
    "CREATE TABLE IF NOT EXISTS api_keys (
        key TEXT PRIMARY KEY,
        label TEXT NOT NULL DEFAULT '',
        created_at INTEGER NOT NULL,
        disabled INTEGER NOT NULL DEFAULT 0
    );",
    // 2 — provider credentials
    "CREATE TABLE IF NOT EXISTS provider_credentials (
        provider TEXT NOT NULL,
        connection_id TEXT NOT NULL,
        auth_type TEXT NOT NULL,
        api_key TEXT NOT NULL,
        base_urls TEXT NOT NULL,
        models TEXT NOT NULL DEFAULT '[]',
        PRIMARY KEY (provider, connection_id)
    );",
    // 3 — usage log
    "CREATE TABLE IF NOT EXISTS usage (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        ts INTEGER NOT NULL,
        provider TEXT NOT NULL,
        model TEXT NOT NULL,
        status INTEGER NOT NULL,
        prompt_tokens INTEGER NOT NULL DEFAULT 0,
        completion_tokens INTEGER NOT NULL DEFAULT 0,
        total_tokens INTEGER NOT NULL DEFAULT 0,
        latency_ms INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX IF NOT EXISTS usage_ts_idx ON usage(ts);
    CREATE INDEX IF NOT EXISTS usage_provider_idx ON usage(provider, ts);",
    // 4 — custom providers (extends embedded catalog)
    "CREATE TABLE IF NOT EXISTS custom_providers (
        id TEXT PRIMARY KEY,
        base_urls TEXT NOT NULL,
        auth_type TEXT NOT NULL,
        chat_path TEXT NOT NULL,
        format TEXT NOT NULL,
        models TEXT NOT NULL
    );",
    // 5 — combos (multi-model fallback/routing chains)
    "CREATE TABLE IF NOT EXISTS combos (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL UNIQUE,
        strategy TEXT NOT NULL DEFAULT 'priority',
        auto_strategy TEXT NOT NULL DEFAULT 'rules',
        models_json TEXT NOT NULL DEFAULT '[]',
        config_json TEXT NOT NULL DEFAULT '{}',
        enabled INTEGER NOT NULL DEFAULT 1,
        created_at INTEGER NOT NULL DEFAULT 0,
        updated_at INTEGER NOT NULL DEFAULT 0
    );",
    // 6 — gateway settings (key/value JSON)
    "CREATE TABLE IF NOT EXISTS settings (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL,
        updated_at INTEGER NOT NULL DEFAULT 0
    );",
    // 7 — gamification state (XP, streak, badges)
    "CREATE TABLE IF NOT EXISTS gamification_state (
        user_id TEXT PRIMARY KEY,
        total_xp INTEGER NOT NULL DEFAULT 0,
        current_streak INTEGER NOT NULL DEFAULT 0,
        longest_streak INTEGER NOT NULL DEFAULT 0,
        last_active_date TEXT NOT NULL DEFAULT '',
        streak_start_date TEXT NOT NULL DEFAULT '',
        badge_state_json TEXT NOT NULL DEFAULT '{}',
        unlocked_badges_json TEXT NOT NULL DEFAULT '[]',
        updated_at INTEGER NOT NULL DEFAULT 0
    );",
    // 8 — guardrail audit events
    "CREATE TABLE IF NOT EXISTS guardrail_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        ts INTEGER NOT NULL,
        kind TEXT NOT NULL,
        action TEXT NOT NULL,
        provider TEXT NOT NULL DEFAULT '',
        model TEXT NOT NULL DEFAULT '',
        details_json TEXT NOT NULL DEFAULT '{}'
    );
    CREATE INDEX IF NOT EXISTS guardrail_events_ts_idx ON guardrail_events(ts);",
    // 9 — model aliases
    "CREATE TABLE IF NOT EXISTS model_aliases (
        alias TEXT PRIMARY KEY,
        target TEXT NOT NULL
    );",
    // 10 — persistent agent memories
    "CREATE TABLE IF NOT EXISTS memories (
        id TEXT PRIMARY KEY,
        api_key_hash TEXT NOT NULL DEFAULT '',
        session_id TEXT NOT NULL DEFAULT '',
        mem_type TEXT NOT NULL DEFAULT 'factual',
        mem_key TEXT NOT NULL,
        content TEXT NOT NULL,
        category TEXT NOT NULL DEFAULT '',
        metadata_json TEXT NOT NULL DEFAULT '{}',
        tokens_est INTEGER NOT NULL DEFAULT 0,
        expires_at INTEGER,
        created_at INTEGER NOT NULL DEFAULT 0,
        updated_at INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX IF NOT EXISTS memories_key_idx ON memories(api_key_hash, session_id);",
    // 11 — installed agent skills
    "CREATE TABLE IF NOT EXISTS skills (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL UNIQUE,
        description TEXT NOT NULL DEFAULT '',
        source_provider TEXT NOT NULL DEFAULT 'local',
        mode TEXT NOT NULL DEFAULT 'on',
        tags_json TEXT NOT NULL DEFAULT '[]',
        instructions TEXT NOT NULL DEFAULT '',
        schema_json TEXT NOT NULL DEFAULT '{}',
        enabled INTEGER NOT NULL DEFAULT 1,
        executions_count INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL DEFAULT 0,
        updated_at INTEGER NOT NULL DEFAULT 0
    );",
    // 12 — A2A v0.3/v1.0 tasks & event history
    "CREATE TABLE IF NOT EXISTS a2a_tasks (
        id TEXT PRIMARY KEY,
        owner_hash TEXT NOT NULL DEFAULT '',
        skill TEXT NOT NULL,
        state TEXT NOT NULL,
        input_json TEXT NOT NULL DEFAULT '{}',
        artifacts_json TEXT NOT NULL DEFAULT '[]',
        events_json TEXT NOT NULL DEFAULT '[]',
        metadata_json TEXT NOT NULL DEFAULT '{}',
        created_at INTEGER NOT NULL DEFAULT 0,
        updated_at INTEGER NOT NULL DEFAULT 0,
        expires_at INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX IF NOT EXISTS a2a_tasks_owner_idx ON a2a_tasks(owner_hash, created_at);",
    // 13 — call_logs (upstream parity table)
    "CREATE TABLE IF NOT EXISTS call_logs (
        id TEXT PRIMARY KEY,
        timestamp TEXT NOT NULL,
        method TEXT,
        path TEXT,
        status INTEGER,
        model TEXT,
        provider TEXT,
        connection_id TEXT,
        duration INTEGER DEFAULT 0,
        tokens_in INTEGER DEFAULT 0,
        tokens_out INTEGER DEFAULT 0,
        error_summary TEXT
    );
    CREATE INDEX IF NOT EXISTS call_logs_ts_idx ON call_logs(timestamp);",
    // 14 — domain_budgets (upstream parity table)
    "CREATE TABLE IF NOT EXISTS domain_budgets (
        api_key_id TEXT PRIMARY KEY,
        daily_limit_usd REAL NOT NULL DEFAULT 0,
        weekly_limit_usd REAL DEFAULT 0,
        monthly_limit_usd REAL DEFAULT 0,
        warning_threshold REAL DEFAULT 0.8
    );",
    // 15 — key_templates (upstream parity table)
    "CREATE TABLE IF NOT EXISTS key_templates (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL UNIQUE,
        description TEXT NOT NULL DEFAULT '',
        policy TEXT NOT NULL DEFAULT '{}',
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyRow {
    pub key: String,
    pub label: String,
    pub created_at: i64,
    pub disabled: bool,
    pub expires_at: Option<i64>,
    pub budget_5h_usd: Option<f64>,
    pub rpm_limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialRow {
    pub provider: String,
    pub connection_id: String,
    pub auth_type: String,
    pub api_key: String,
    pub base_urls: Vec<String>,
    pub models: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub priority: i32,
    #[serde(default = "default_weight")]
    pub weight: f64,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_at: Option<i64>,
    #[serde(default = "default_empty_obj")]
    pub provider_specific_json: serde_json::Value,
    #[serde(default = "default_healthy")]
    pub health_status: String,
    #[serde(default)]
    pub last_refreshed_at: Option<i64>,
    #[serde(default = "default_hc_interval")]
    pub health_check_interval_min: i64,
}

fn default_true() -> bool {
    true
}
fn default_weight() -> f64 {
    1.0
}
fn default_empty_obj() -> serde_json::Value {
    serde_json::json!({})
}
fn default_healthy() -> String {
    "healthy".into()
}
fn default_hc_interval() -> i64 {
    60
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComboStep {
    pub provider: String,
    pub model: String,
    #[serde(default = "default_weight")]
    pub weight: f64,
    #[serde(default)]
    pub fallback_only: bool,
    #[serde(default)]
    pub connection_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComboRow {
    pub id: String,
    pub name: String,
    pub strategy: String,
    pub auto_strategy: String,
    pub models: Vec<ComboStep>,
    pub config: serde_json::Value,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageEntry {
    pub ts_ms: i64,
    pub provider: String,
    pub connection_id: String,
    pub model: String,
    pub status: u16,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub latency_ms: i64,
    pub cost_usd: f64,
    pub api_key_hash: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GamificationRow {
    pub user_id: String,
    pub total_xp: i64,
    pub current_streak: u32,
    pub longest_streak: u32,
    pub last_active_date: String,
    pub streak_start_date: String,
    pub badge_state_json: String,
    pub unlocked_badges: Vec<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardrailEventRow {
    pub id: i64,
    pub ts: i64,
    pub kind: String,
    pub action: String,
    pub provider: String,
    pub model: String,
    pub details: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRow {
    pub id: String,
    #[serde(default)]
    pub api_key_hash: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default = "default_factual")]
    pub mem_type: String,
    pub mem_key: String,
    pub content: String,
    #[serde(default)]
    pub category: String,
    #[serde(default = "default_empty_obj")]
    pub metadata: serde_json::Value,
    #[serde(default)]
    pub tokens_est: i64,
    #[serde(default)]
    pub expires_at: Option<i64>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

fn default_factual() -> String {
    "factual".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRow {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_local_source")]
    pub source_provider: String,
    #[serde(default = "default_on_mode")]
    pub mode: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub instructions: String,
    #[serde(default = "default_empty_obj")]
    pub schema: serde_json::Value,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub executions_count: i64,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

fn default_local_source() -> String {
    "local".into()
}
fn default_on_mode() -> String {
    "on".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2ATaskRow {
    pub id: String,
    #[serde(default)]
    pub owner_hash: String,
    pub skill: String,
    pub state: String,
    #[serde(default = "default_empty_obj")]
    pub input: serde_json::Value,
    #[serde(default)]
    pub artifacts: Vec<serde_json::Value>,
    #[serde(default)]
    pub events: Vec<serde_json::Value>,
    #[serde(default = "default_empty_obj")]
    pub metadata: serde_json::Value,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub expires_at: i64,
}

fn ensure_column(conn: &Connection, table: &str, col: &str, def: &str) -> anyhow::Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let cols: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if !cols.iter().any(|c| c == col) {
        conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {col} {def};"))?;
    }
    Ok(())
}

impl Store {
    pub fn open(path: &str) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS _migration_meta(version INTEGER PRIMARY KEY);",
        )?;
        for (i, m) in BASE_MIGRATIONS.iter().enumerate() {
            conn.execute_batch(&format!(
                "INSERT INTO _migration_meta(version) SELECT {} WHERE NOT EXISTS(SELECT 1 FROM _migration_meta WHERE version = {});",
                i + 1,
                i + 1
            ))?;
            conn.execute_batch(m)?;
        }
        // Incremental column upgrades for backward compatibility with older DB files.
        ensure_column(&conn, "api_keys", "expires_at", "INTEGER")?;
        ensure_column(&conn, "api_keys", "budget_5h_usd", "REAL")?;
        ensure_column(&conn, "api_keys", "rpm_limit", "INTEGER")?;

        ensure_column(&conn, "provider_credentials", "enabled", "INTEGER NOT NULL DEFAULT 1")?;
        ensure_column(&conn, "provider_credentials", "priority", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(&conn, "provider_credentials", "weight", "REAL NOT NULL DEFAULT 1.0")?;
        ensure_column(&conn, "provider_credentials", "label", "TEXT NOT NULL DEFAULT ''")?;
        ensure_column(&conn, "provider_credentials", "refresh_token", "TEXT")?;
        ensure_column(&conn, "provider_credentials", "expires_at", "INTEGER")?;
        ensure_column(&conn, "provider_credentials", "provider_specific_json", "TEXT NOT NULL DEFAULT '{}'")?;
        ensure_column(&conn, "provider_credentials", "health_status", "TEXT NOT NULL DEFAULT 'healthy'")?;
        ensure_column(&conn, "provider_credentials", "last_refreshed_at", "INTEGER")?;
        ensure_column(&conn, "provider_credentials", "health_check_interval_min", "INTEGER NOT NULL DEFAULT 60")?;

        ensure_column(&conn, "usage", "api_key_hash", "TEXT NOT NULL DEFAULT ''")?;
        ensure_column(&conn, "usage", "connection_id", "TEXT NOT NULL DEFAULT ''")?;
        ensure_column(&conn, "usage", "cost_usd", "REAL NOT NULL DEFAULT 0.0")?;
        ensure_column(&conn, "usage", "cache_read_tokens", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(&conn, "usage", "cache_write_tokens", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(&conn, "usage", "session_id", "TEXT NOT NULL DEFAULT ''")?;

        ensure_column(&conn, "custom_providers", "headers", "TEXT NOT NULL DEFAULT '{}'")?;
        ensure_column(&conn, "custom_providers", "anonymous_api_key", "TEXT")?;
        ensure_column(&conn, "custom_providers", "timeout_ms", "INTEGER")?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn open_default(data_dir: &str) -> anyhow::Result<Self> {
        let dir = std::path::Path::new(data_dir);
        std::fs::create_dir_all(dir).ok();
        Self::open(&format!("{}/omniroute-rs.db", data_dir))
    }

    // ── API keys ────────────────────────────────────────────────────

    pub fn add_api_key(&self, key: &str, label: &str, now_ms: i64) -> anyhow::Result<()> {
        self.add_api_key_with_limits(key, label, now_ms, None, None, None)
    }

    pub fn add_api_key_with_limits(
        &self,
        key: &str,
        label: &str,
        now_ms: i64,
        expires_at: Option<i64>,
        budget_5h_usd: Option<f64>,
        rpm_limit: Option<u32>,
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO api_keys(key, label, created_at, disabled, expires_at, budget_5h_usd, rpm_limit)
             VALUES (?1, ?2, ?3, 0, ?4, ?5, ?6)",
            rusqlite::params![key, label, now_ms, expires_at, budget_5h_usd, rpm_limit.map(|v| v as i64)],
        )?;
        Ok(())
    }

    pub fn delete_api_key(&self, key: &str) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute("DELETE FROM api_keys WHERE key = ?1", rusqlite::params![key])?;
        Ok(n > 0)
    }

    pub fn has_api_keys(&self) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM api_keys WHERE disabled = 0", [], |r| r.get(0))?;
        Ok(count > 0)
    }

    /// Validate an API key.
    /// - If 0 active keys are configured -> open-door mode (returns true even for empty/None).
    /// - If >=1 keys are configured -> key MUST be Some, non-empty, present, not disabled, and not expired.
    pub fn validate_api_key(&self, key: Option<&str>, now_ms: i64) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let total: i64 = conn.query_row("SELECT COUNT(*) FROM api_keys", [], |r| r.get(0))?;
        if total == 0 {
            return Ok(true);
        }
        let Some(k) = key.filter(|s| !s.is_empty()) else {
            return Ok(false);
        };
        let row: Option<(i64, Option<i64>)> = conn
            .query_row(
                "SELECT disabled, expires_at FROM api_keys WHERE key = ?1",
                rusqlite::params![k],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        match row {
            None => Ok(false),
            Some((disabled, expires_at)) => {
                if disabled != 0 {
                    return Ok(false);
                }
                if let Some(exp) = expires_at {
                    if exp > 0 && now_ms > exp {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
        }
    }

    pub fn is_api_key_valid(&self, key: &str) -> anyhow::Result<bool> {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        self.validate_api_key(Some(key), now_ms)
    }

    pub fn get_api_key_row(&self, key: &str) -> anyhow::Result<Option<ApiKeyRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT key, label, created_at, disabled, expires_at, budget_5h_usd, rpm_limit FROM api_keys WHERE key = ?1",
        )?;
        let mut rows = stmt.query_map(rusqlite::params![key], |r| {
            Ok(ApiKeyRow {
                key: r.get(0)?,
                label: r.get(1)?,
                created_at: r.get(2)?,
                disabled: r.get::<_, i64>(3)? != 0,
                expires_at: r.get(4)?,
                budget_5h_usd: r.get(5)?,
                rpm_limit: r.get::<_, Option<i64>>(6)?.map(|v| v as u32),
            })
        })?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    pub fn list_api_keys(&self) -> anyhow::Result<Vec<(String, String, i64)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT key, label, created_at FROM api_keys ORDER BY created_at")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn list_api_key_rows(&self) -> anyhow::Result<Vec<ApiKeyRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT key, label, created_at, disabled, expires_at, budget_5h_usd, rpm_limit FROM api_keys ORDER BY created_at",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(ApiKeyRow {
                    key: r.get(0)?,
                    label: r.get(1)?,
                    created_at: r.get(2)?,
                    disabled: r.get::<_, i64>(3)? != 0,
                    expires_at: r.get(4)?,
                    budget_5h_usd: r.get(5)?,
                    rpm_limit: r.get::<_, Option<i64>>(6)?.map(|v| v as u32),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // ── Provider credentials ────────────────────────────────────────

    pub fn upsert_credential(
        &self,
        provider: &str,
        connection_id: &str,
        auth_type: &str,
        api_key: &str,
        base_urls: &[String],
        models: &[String],
    ) -> anyhow::Result<()> {
        self.upsert_credential_full(&CredentialRow {
            provider: provider.to_string(),
            connection_id: connection_id.to_string(),
            auth_type: auth_type.to_string(),
            api_key: api_key.to_string(),
            base_urls: base_urls.to_vec(),
            models: models.to_vec(),
            enabled: true,
            priority: 0,
            weight: 1.0,
            label: String::new(),
            refresh_token: None,
            expires_at: None,
            provider_specific_json: serde_json::json!({}),
            health_status: "healthy".into(),
            last_refreshed_at: None,
            health_check_interval_min: 60,
        })
    }

    pub fn upsert_credential_full(&self, row: &CredentialRow) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        let health = if row.health_status.is_empty() {
            "healthy"
        } else {
            row.health_status.as_str()
        };
        conn.execute(
            "INSERT OR REPLACE INTO provider_credentials(
                provider, connection_id, auth_type, api_key, base_urls, models,
                enabled, priority, weight, label,
                refresh_token, expires_at, provider_specific_json, health_status, last_refreshed_at, health_check_interval_min
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            rusqlite::params![
                row.provider,
                row.connection_id,
                row.auth_type,
                row.api_key,
                serde_json::to_string(&row.base_urls)?,
                serde_json::to_string(&row.models)?,
                if row.enabled { 1i64 } else { 0i64 },
                row.priority,
                row.weight,
                row.label,
                row.refresh_token,
                row.expires_at,
                serde_json::to_string(&row.provider_specific_json)?,
                health,
                row.last_refreshed_at,
                row.health_check_interval_min,
            ],
        )?;
        Ok(())
    }

    pub fn delete_credential(&self, provider: &str, connection_id: &str) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(
            "DELETE FROM provider_credentials WHERE provider = ?1 AND connection_id = ?2",
            rusqlite::params![provider, connection_id],
        )?;
        Ok(n > 0)
    }

    pub fn list_credentials(&self) -> anyhow::Result<Vec<CredentialRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT provider, connection_id, auth_type, api_key, base_urls, models,
                    enabled, priority, weight, label,
                    refresh_token, expires_at, provider_specific_json, health_status, last_refreshed_at, health_check_interval_min
             FROM provider_credentials ORDER BY provider, priority DESC, connection_id",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(CredentialRow {
                    provider: r.get(0)?,
                    connection_id: r.get(1)?,
                    auth_type: r.get(2)?,
                    api_key: r.get(3)?,
                    base_urls: serde_json::from_str::<Vec<String>>(&r.get::<_, String>(4)?).unwrap_or_default(),
                    models: serde_json::from_str::<Vec<String>>(&r.get::<_, String>(5)?).unwrap_or_default(),
                    enabled: r.get::<_, i64>(6)? != 0,
                    priority: r.get(7)?,
                    weight: r.get(8)?,
                    label: r.get(9)?,
                    refresh_token: r.get(10)?,
                    expires_at: r.get(11)?,
                    provider_specific_json: serde_json::from_str(&r.get::<_, String>(12)?).unwrap_or(serde_json::json!({})),
                    health_status: r.get(13)?,
                    last_refreshed_at: r.get(14)?,
                    health_check_interval_min: r.get(15)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_credential(
        &self,
        provider: &str,
        connection_id: &str,
    ) -> anyhow::Result<Option<CredentialRow>> {
        Ok(self
            .list_credentials()?
            .into_iter()
            .find(|c| c.provider == provider && c.connection_id == connection_id))
    }

    pub fn list_credentials_for_provider(&self, provider: &str) -> anyhow::Result<Vec<CredentialRow>> {
        Ok(self
            .list_credentials()?
            .into_iter()
            .filter(|c| c.enabled && c.provider == provider)
            .collect())
    }

    /// List enabled OAuth credentials that have a refresh token, are not flagged `needs_reauth`,
    /// have `health_check_interval_min > 0` (v3.8.55 `#8443`), and expire within `within_ms`.
    pub fn list_expiring_oauth_credentials(
        &self,
        now_ms: i64,
        within_ms: i64,
    ) -> anyhow::Result<Vec<CredentialRow>> {
        let horizon = now_ms.saturating_add(within_ms);
        Ok(self
            .list_credentials()?
            .into_iter()
            .filter(|c| {
                c.enabled
                    && c.health_check_interval_min > 0
                    && c.health_status != "needs_reauth"
                    && c.refresh_token.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
                    && c.expires_at.map(|exp| exp <= horizon).unwrap_or(true)
            })
            .collect())
    }

    pub fn update_oauth_tokens(
        &self,
        provider: &str,
        connection_id: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        expires_at: Option<i64>,
        provider_specific_json: Option<&serde_json::Value>,
        now_ms: i64,
    ) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n = if let Some(psj) = provider_specific_json {
            conn.execute(
                "UPDATE provider_credentials
                 SET api_key = ?1,
                     refresh_token = COALESCE(?2, refresh_token),
                     expires_at = ?3,
                     provider_specific_json = ?4,
                     health_status = 'healthy',
                     last_refreshed_at = ?5
                 WHERE provider = ?6 AND connection_id = ?7",
                rusqlite::params![
                    access_token,
                    refresh_token,
                    expires_at,
                    serde_json::to_string(psj)?,
                    now_ms,
                    provider,
                    connection_id
                ],
            )?
        } else {
            conn.execute(
                "UPDATE provider_credentials
                 SET api_key = ?1,
                     refresh_token = COALESCE(?2, refresh_token),
                     expires_at = ?3,
                     health_status = 'healthy',
                     last_refreshed_at = ?4
                 WHERE provider = ?5 AND connection_id = ?6",
                rusqlite::params![
                    access_token,
                    refresh_token,
                    expires_at,
                    now_ms,
                    provider,
                    connection_id
                ],
            )?
        };
        Ok(n > 0)
    }

    pub fn mark_credential_health(
        &self,
        provider: &str,
        connection_id: &str,
        health_status: &str,
        now_ms: i64,
    ) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(
            "UPDATE provider_credentials SET health_status = ?1, last_refreshed_at = ?2 WHERE provider = ?3 AND connection_id = ?4",
            rusqlite::params![health_status, now_ms, provider, connection_id],
        )?;
        Ok(n > 0)
    }

    // ── Custom providers ────────────────────────────────────────────

    pub fn upsert_custom_provider(
        &self,
        id: &str,
        base_urls: &[String],
        auth_type: &str,
        chat_path: &str,
        format: &str,
        models: &[String],
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO custom_providers(id, base_urls, auth_type, chat_path, format, models)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                id,
                serde_json::to_string(base_urls)?,
                auth_type,
                chat_path,
                format,
                serde_json::to_string(models)?
            ],
        )?;
        Ok(())
    }

    pub fn delete_custom_provider(&self, id: &str) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute("DELETE FROM custom_providers WHERE id = ?1", rusqlite::params![id])?;
        Ok(n > 0)
    }

    pub fn list_custom_providers(&self) -> anyhow::Result<Vec<omni_catalog::ProviderDescriptor>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, base_urls, auth_type, chat_path, format, models, headers, anonymous_api_key, timeout_ms FROM custom_providers",
        )?;
        let rows = stmt
            .query_map([], |r| {
                let auth_type: String = r.get(2)?;
                let headers_str: String = r.get(6)?;
                let models: Vec<String> =
                    serde_json::from_str::<Vec<String>>(&r.get::<_, String>(5)?).unwrap_or_default();
                let passthrough = models.iter().any(|m| m == "*");
                Ok(omni_catalog::ProviderDescriptor {
                    id: r.get(0)?,
                    alias: None,
                    name: None,
                    base_urls: serde_json::from_str::<Vec<String>>(&r.get::<_, String>(1)?).unwrap_or_default(),
                    responses_base_url: None,
                    messages_url: None,
                    url_suffix: None,
                    auth_header: auth_type.clone(),
                    auth_type,
                    auth_prefix: None,
                    chat_path: r.get(3)?,
                    format: r.get(4)?,
                    executor: "default".into(),
                    headers: serde_json::from_str::<HashMap<String, String>>(&headers_str).unwrap_or_default(),
                    passthrough_models: passthrough,
                    model_id_prefix: None,
                    default_context_length: None,
                    models,
                    model_specs: vec![],
                    anonymous_api_key: r.get(7)?,
                    timeout_ms: r.get::<_, Option<i64>>(8)?.map(|v| v as u64),
                    is_free_tier: false,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // ── Combos ──────────────────────────────────────────────────────

    pub fn upsert_combo(&self, combo: &ComboRow) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO combos(id, name, strategy, auto_strategy, models_json, config_json, enabled, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                combo.id,
                combo.name,
                combo.strategy,
                combo.auto_strategy,
                serde_json::to_string(&combo.models)?,
                serde_json::to_string(&combo.config)?,
                if combo.enabled { 1i64 } else { 0i64 },
                combo.created_at,
                combo.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn get_combo(&self, id_or_name: &str) -> anyhow::Result<Option<ComboRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, strategy, auto_strategy, models_json, config_json, enabled, created_at, updated_at
             FROM combos WHERE id = ?1 OR name = ?1 LIMIT 1",
        )?;
        let mut rows = stmt.query_map(rusqlite::params![id_or_name], |r| {
            Ok(ComboRow {
                id: r.get(0)?,
                name: r.get(1)?,
                strategy: r.get(2)?,
                auto_strategy: r.get(3)?,
                models: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or_default(),
                config: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or(serde_json::json!({})),
                enabled: r.get::<_, i64>(6)? != 0,
                created_at: r.get(7)?,
                updated_at: r.get(8)?,
            })
        })?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    pub fn list_combos(&self) -> anyhow::Result<Vec<ComboRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, strategy, auto_strategy, models_json, config_json, enabled, created_at, updated_at
             FROM combos ORDER BY name",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(ComboRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    strategy: r.get(2)?,
                    auto_strategy: r.get(3)?,
                    models: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or_default(),
                    config: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or(serde_json::json!({})),
                    enabled: r.get::<_, i64>(6)? != 0,
                    created_at: r.get(7)?,
                    updated_at: r.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn delete_combo(&self, id_or_name: &str) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(
            "DELETE FROM combos WHERE id = ?1 OR name = ?1",
            rusqlite::params![id_or_name],
        )?;
        Ok(n > 0)
    }

    // ── Model aliases ───────────────────────────────────────────────

    pub fn set_model_alias(&self, alias: &str, target: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO model_aliases(alias, target) VALUES (?1, ?2)",
            rusqlite::params![alias, target],
        )?;
        Ok(())
    }

    pub fn resolve_model_alias(&self, alias: &str) -> anyhow::Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let res = conn
            .query_row(
                "SELECT target FROM model_aliases WHERE alias = ?1",
                rusqlite::params![alias],
                |r| r.get(0),
            )
            .ok();
        Ok(res)
    }

    // ── Settings ────────────────────────────────────────────────────

    pub fn set_setting(&self, key: &str, value: &str, now_ms: i64) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO settings(key, value, updated_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![key, value, now_ms],
        )?;
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> anyhow::Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        Ok(conn
            .query_row("SELECT value FROM settings WHERE key = ?1", rusqlite::params![key], |r| r.get(0))
            .ok())
    }

    pub fn list_settings(&self) -> anyhow::Result<HashMap<String, String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT key, value FROM settings")?;
        let mut map = HashMap::new();
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for r in rows {
            let (k, v) = r?;
            map.insert(k, v);
        }
        Ok(map)
    }

    // ── Usage ───────────────────────────────────────────────────────

    pub fn record_usage(
        &self,
        ts_ms: i64,
        provider: &str,
        model: &str,
        status: u16,
        prompt_tokens: i64,
        completion_tokens: i64,
        total_tokens: i64,
        latency_ms: i64,
    ) -> anyhow::Result<()> {
        self.record_usage_full(&UsageEntry {
            ts_ms,
            provider: provider.to_string(),
            connection_id: String::new(),
            model: model.to_string(),
            status,
            prompt_tokens,
            completion_tokens,
            total_tokens,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            latency_ms,
            cost_usd: 0.0,
            api_key_hash: String::new(),
            session_id: String::new(),
        })
    }

    pub fn record_usage_full(&self, e: &UsageEntry) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO usage(ts, provider, model, status, prompt_tokens, completion_tokens, total_tokens, latency_ms, api_key_hash, connection_id, cost_usd, cache_read_tokens, cache_write_tokens, session_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            rusqlite::params![
                e.ts_ms,
                e.provider,
                e.model,
                e.status,
                e.prompt_tokens,
                e.completion_tokens,
                e.total_tokens,
                e.latency_ms,
                e.api_key_hash,
                e.connection_id,
                e.cost_usd,
                e.cache_read_tokens,
                e.cache_write_tokens,
                e.session_id,
            ],
        )?;
        Ok(())
    }

    pub fn usage_summary(&self) -> anyhow::Result<serde_json::Value> {
        let conn = self.conn.lock().unwrap();
        let (reqs, pt, ct, tt, cr, cw, cost): (i64, i64, i64, i64, i64, i64, f64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(prompt_tokens),0), COALESCE(SUM(completion_tokens),0),
                    COALESCE(SUM(total_tokens),0), COALESCE(SUM(cache_read_tokens),0),
                    COALESCE(SUM(cache_write_tokens),0), COALESCE(SUM(cost_usd),0.0)
             FROM usage",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
        )?;
        let by_provider: Vec<(String, i64, i64, f64)> = {
            let mut stmt = conn.prepare(
                "SELECT provider, COUNT(*), COALESCE(SUM(total_tokens),0), COALESCE(SUM(cost_usd),0.0)
                 FROM usage GROUP BY provider ORDER BY COUNT(*) DESC LIMIT 25",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        Ok(serde_json::json!({
            "requests": reqs,
            "prompt_tokens": pt,
            "completion_tokens": ct,
            "total_tokens": tt,
            "cache_read_tokens": cr,
            "cache_write_tokens": cw,
            "cost_usd": cost,
            "by_provider": by_provider.into_iter().map(|(p, r, t, c)| serde_json::json!({
                "provider": p, "requests": r, "total_tokens": t, "cost_usd": c
            })).collect::<Vec<_>>()
        }))
    }

    /// Usage stats for a specific key hash since `since_ms` (for `/v1/me/status` & `/v1/me/usage`).
    pub fn usage_for_key(&self, api_key_hash: &str, since_ms: i64) -> anyhow::Result<serde_json::Value> {
        let conn = self.conn.lock().unwrap();
        let (reqs, pt, ct, tt, cost): (i64, i64, i64, i64, f64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(prompt_tokens),0), COALESCE(SUM(completion_tokens),0),
                    COALESCE(SUM(total_tokens),0), COALESCE(SUM(cost_usd),0.0)
             FROM usage WHERE (?1 = '' OR api_key_hash = ?1) AND ts >= ?2",
            rusqlite::params![api_key_hash, since_ms],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )?;
        let by_model: Vec<(String, String, i64, i64, f64)> = {
            let mut stmt = conn.prepare(
                "SELECT provider, model, COUNT(*), COALESCE(SUM(total_tokens),0), COALESCE(SUM(cost_usd),0.0)
                 FROM usage WHERE (?1 = '' OR api_key_hash = ?1) AND ts >= ?2
                 GROUP BY provider, model ORDER BY COUNT(*) DESC LIMIT 50",
            )?;
            let rows = stmt.query_map(rusqlite::params![api_key_hash, since_ms], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        Ok(serde_json::json!({
            "requests": reqs,
            "prompt_tokens": pt,
            "completion_tokens": ct,
            "total_tokens": tt,
            "cost_usd": cost,
            "by_model": by_model.into_iter().map(|(p, m, r, t, c)| serde_json::json!({
                "provider": p, "model": m, "requests": r, "total_tokens": t, "cost_usd": c
            })).collect::<Vec<_>>()
        }))
    }

    // ── Gamification persistence ────────────────────────────────────

    pub fn load_gamification(&self, user_id: &str) -> anyhow::Result<GamificationRow> {
        let conn = self.conn.lock().unwrap();
        let res = conn.query_row(
            "SELECT user_id, total_xp, current_streak, longest_streak, last_active_date, streak_start_date, badge_state_json, unlocked_badges_json, updated_at
             FROM gamification_state WHERE user_id = ?1",
            rusqlite::params![user_id],
            |r| {
                Ok(GamificationRow {
                    user_id: r.get(0)?,
                    total_xp: r.get(1)?,
                    current_streak: r.get::<_, i64>(2)? as u32,
                    longest_streak: r.get::<_, i64>(3)? as u32,
                    last_active_date: r.get(4)?,
                    streak_start_date: r.get(5)?,
                    badge_state_json: r.get(6)?,
                    unlocked_badges: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or_default(),
                    updated_at: r.get(8)?,
                })
            },
        );
        match res {
            Ok(row) => Ok(row),
            Err(_) => Ok(GamificationRow {
                user_id: user_id.to_string(),
                total_xp: 0,
                current_streak: 0,
                longest_streak: 0,
                last_active_date: String::new(),
                streak_start_date: String::new(),
                badge_state_json: "{}".into(),
                unlocked_badges: vec![],
                updated_at: 0,
            }),
        }
    }

    pub fn save_gamification(&self, row: &GamificationRow) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO gamification_state(user_id, total_xp, current_streak, longest_streak, last_active_date, streak_start_date, badge_state_json, unlocked_badges_json, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                row.user_id,
                row.total_xp,
                row.current_streak as i64,
                row.longest_streak as i64,
                row.last_active_date,
                row.streak_start_date,
                row.badge_state_json,
                serde_json::to_string(&row.unlocked_badges)?,
                row.updated_at,
            ],
        )?;
        Ok(())
    }

    // ── Guardrail audit events ──────────────────────────────────────

    pub fn record_guardrail_event(
        &self,
        ts_ms: i64,
        kind: &str,
        action: &str,
        provider: &str,
        model: &str,
        details: &serde_json::Value,
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO guardrail_events(ts, kind, action, provider, model, details_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![ts_ms, kind, action, provider, model, details.to_string()],
        )?;
        Ok(())
    }

    pub fn list_guardrail_events(&self, limit: usize) -> anyhow::Result<Vec<GuardrailEventRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, ts, kind, action, provider, model, details_json FROM guardrail_events ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![limit as i64], |r| {
                Ok(GuardrailEventRow {
                    id: r.get(0)?,
                    ts: r.get(1)?,
                    kind: r.get(2)?,
                    action: r.get(3)?,
                    provider: r.get(4)?,
                    model: r.get(5)?,
                    details: serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or(serde_json::json!({})),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // ── Memories CRUD + Semantic TF-Cosine Retrieval ────────────────

    pub fn upsert_memory(&self, row: &MemoryRow) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        let tokens = if row.tokens_est > 0 {
            row.tokens_est
        } else {
            ((row.content.len() as f64) / 4.0).ceil().max(1.0) as i64
        };
        conn.execute(
            "INSERT OR REPLACE INTO memories(
                id, api_key_hash, session_id, mem_type, mem_key, content, category,
                metadata_json, tokens_est, expires_at, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                row.id,
                row.api_key_hash,
                row.session_id,
                row.mem_type,
                row.mem_key,
                row.content,
                row.category,
                serde_json::to_string(&row.metadata)?,
                tokens,
                row.expires_at,
                row.created_at,
                row.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn get_memory(&self, id: &str) -> anyhow::Result<Option<MemoryRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, api_key_hash, session_id, mem_type, mem_key, content, category,
                    metadata_json, tokens_est, expires_at, created_at, updated_at
             FROM memories WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(rusqlite::params![id], |r| {
            Ok(MemoryRow {
                id: r.get(0)?,
                api_key_hash: r.get(1)?,
                session_id: r.get(2)?,
                mem_type: r.get(3)?,
                mem_key: r.get(4)?,
                content: r.get(5)?,
                category: r.get(6)?,
                metadata: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or(serde_json::json!({})),
                tokens_est: r.get(8)?,
                expires_at: r.get(9)?,
                created_at: r.get(10)?,
                updated_at: r.get(11)?,
            })
        })?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    pub fn delete_memory(&self, id: &str) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute("DELETE FROM memories WHERE id = ?1", rusqlite::params![id])?;
        Ok(n > 0)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn list_memories(
        &self,
        api_key_hash: Option<&str>,
        session_id: Option<&str>,
        mem_type: Option<&str>,
        category: Option<&str>,
        query: Option<&str>,
        now_ms: i64,
        limit: usize,
        offset: usize,
    ) -> anyhow::Result<(Vec<MemoryRow>, usize, i64)> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, api_key_hash, session_id, mem_type, mem_key, content, category,
                    metadata_json, tokens_est, expires_at, created_at, updated_at
             FROM memories ORDER BY updated_at DESC",
        )?;
        let all: Vec<MemoryRow> = stmt
            .query_map([], |r| {
                Ok(MemoryRow {
                    id: r.get(0)?,
                    api_key_hash: r.get(1)?,
                    session_id: r.get(2)?,
                    mem_type: r.get(3)?,
                    mem_key: r.get(4)?,
                    content: r.get(5)?,
                    category: r.get(6)?,
                    metadata: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or(serde_json::json!({})),
                    tokens_est: r.get(8)?,
                    expires_at: r.get(9)?,
                    created_at: r.get(10)?,
                    updated_at: r.get(11)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let q_lower = query.map(|s| s.to_ascii_lowercase());
        let filtered: Vec<MemoryRow> = all
            .into_iter()
            .filter(|m| {
                if let Some(exp) = m.expires_at {
                    if exp > 0 && now_ms > exp {
                        return false;
                    }
                }
                if let Some(kh) = api_key_hash.filter(|s| !s.is_empty()) {
                    if m.api_key_hash != kh {
                        return false;
                    }
                }
                if let Some(sid) = session_id.filter(|s| !s.is_empty()) {
                    if m.session_id != sid {
                        return false;
                    }
                }
                if let Some(mt) = mem_type.filter(|s| !s.is_empty()) {
                    if !m.mem_type.eq_ignore_ascii_case(mt) {
                        return false;
                    }
                }
                if let Some(cat) = category.filter(|s| !s.is_empty()) {
                    if !m.category.eq_ignore_ascii_case(cat) {
                        return false;
                    }
                }
                if let Some(ref ql) = q_lower {
                    if !m.mem_key.to_ascii_lowercase().contains(ql)
                        && !m.content.to_ascii_lowercase().contains(ql)
                        && !m.category.to_ascii_lowercase().contains(ql)
                    {
                        return false;
                    }
                }
                true
            })
            .collect();

        let total = filtered.len();
        let tokens_used: i64 = filtered.iter().map(|m| m.tokens_est).sum();
        let page = filtered.into_iter().skip(offset).take(limit).collect();
        Ok((page, total, tokens_used))
    }

    /// Lightweight sparse word-TF cosine + substring retrieval for context injection.
    pub fn retrieve_memories(
        &self,
        query: &str,
        api_key_hash: Option<&str>,
        session_id: Option<&str>,
        now_ms: i64,
        top_k: usize,
    ) -> anyhow::Result<Vec<(MemoryRow, f64)>> {
        let (items, _, _) = self.list_memories(api_key_hash, session_id, None, None, None, now_ms, 500, 0)?;
        let q_vec = tf_vector(query);
        let q_lower = query.to_ascii_lowercase();
        let mut scored: Vec<(MemoryRow, f64)> = items
            .into_iter()
            .filter_map(|m| {
                let doc = format!("{} {} {}", m.mem_key, m.category, m.content);
                let mut score = cosine_sim(&q_vec, &tf_vector(&doc));
                if !q_lower.is_empty() && doc.to_ascii_lowercase().contains(&q_lower) {
                    score = (score + 0.35).min(1.0);
                }
                if score > 0.05 {
                    Some((m, score))
                } else {
                    None
                }
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);
        Ok(scored)
    }

    // ── Skills CRUD & Executions ────────────────────────────────────

    pub fn upsert_skill(&self, row: &SkillRow) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO skills(
                id, name, description, source_provider, mode, tags_json,
                instructions, schema_json, enabled, executions_count, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                row.id,
                row.name,
                row.description,
                row.source_provider,
                row.mode,
                serde_json::to_string(&row.tags)?,
                row.instructions,
                serde_json::to_string(&row.schema)?,
                if row.enabled { 1i64 } else { 0i64 },
                row.executions_count,
                row.created_at,
                row.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn get_skill(&self, id_or_name: &str) -> anyhow::Result<Option<SkillRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, description, source_provider, mode, tags_json,
                    instructions, schema_json, enabled, executions_count, created_at, updated_at
             FROM skills WHERE id = ?1 OR name = ?1 LIMIT 1",
        )?;
        let mut rows = stmt.query_map(rusqlite::params![id_or_name], |r| {
            Ok(SkillRow {
                id: r.get(0)?,
                name: r.get(1)?,
                description: r.get(2)?,
                source_provider: r.get(3)?,
                mode: r.get(4)?,
                tags: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
                instructions: r.get(6)?,
                schema: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or(serde_json::json!({})),
                enabled: r.get::<_, i64>(8)? != 0,
                executions_count: r.get(9)?,
                created_at: r.get(10)?,
                updated_at: r.get(11)?,
            })
        })?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    pub fn list_skills(&self) -> anyhow::Result<Vec<SkillRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, description, source_provider, mode, tags_json,
                    instructions, schema_json, enabled, executions_count, created_at, updated_at
             FROM skills ORDER BY name",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(SkillRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    description: r.get(2)?,
                    source_provider: r.get(3)?,
                    mode: r.get(4)?,
                    tags: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
                    instructions: r.get(6)?,
                    schema: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or(serde_json::json!({})),
                    enabled: r.get::<_, i64>(8)? != 0,
                    executions_count: r.get(9)?,
                    created_at: r.get(10)?,
                    updated_at: r.get(11)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn delete_skill(&self, id_or_name: &str) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(
            "DELETE FROM skills WHERE id = ?1 OR name = ?1",
            rusqlite::params![id_or_name],
        )?;
        Ok(n > 0)
    }

    pub fn record_skill_execution(&self, id_or_name: &str, now_ms: i64) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(
            "UPDATE skills SET executions_count = executions_count + 1, updated_at = ?1 WHERE id = ?2 OR name = ?2",
            rusqlite::params![now_ms, id_or_name],
        )?;
        Ok(n > 0)
    }

    // ── A2A Tasks Persistence (with owner scoping GHSA-jcm5-6wpp-wjj8) ─

    pub fn upsert_a2a_task(&self, row: &A2ATaskRow) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO a2a_tasks(
                id, owner_hash, skill, state, input_json, artifacts_json,
                events_json, metadata_json, created_at, updated_at, expires_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                row.id,
                row.owner_hash,
                row.skill,
                row.state,
                serde_json::to_string(&row.input)?,
                serde_json::to_string(&row.artifacts)?,
                serde_json::to_string(&row.events)?,
                serde_json::to_string(&row.metadata)?,
                row.created_at,
                row.updated_at,
                row.expires_at,
            ],
        )?;
        Ok(())
    }

    pub fn get_a2a_task(&self, id: &str, caller_owner: &str) -> anyhow::Result<Option<A2ATaskRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, owner_hash, skill, state, input_json, artifacts_json,
                    events_json, metadata_json, created_at, updated_at, expires_at
             FROM a2a_tasks WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(rusqlite::params![id], |r| {
            Ok(A2ATaskRow {
                id: r.get(0)?,
                owner_hash: r.get(1)?,
                skill: r.get(2)?,
                state: r.get(3)?,
                input: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or(serde_json::json!({})),
                artifacts: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
                events: serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default(),
                metadata: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or(serde_json::json!({})),
                created_at: r.get(8)?,
                updated_at: r.get(9)?,
                expires_at: r.get(10)?,
            })
        })?;
        match rows.next() {
            Some(Ok(task)) => {
                // GHSA-jcm5-6wpp-wjj8: owner-scoped tasks are invisible to other API key owners
                if !task.owner_hash.is_empty() && task.owner_hash != caller_owner {
                    Ok(None)
                } else {
                    Ok(Some(task))
                }
            }
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }

    pub fn list_a2a_tasks(
        &self,
        caller_owner: &str,
        state: Option<&str>,
        skill: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> anyhow::Result<Vec<A2ATaskRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, owner_hash, skill, state, input_json, artifacts_json,
                    events_json, metadata_json, created_at, updated_at, expires_at
             FROM a2a_tasks ORDER BY created_at DESC",
        )?;
        let all: Vec<A2ATaskRow> = stmt
            .query_map([], |r| {
                Ok(A2ATaskRow {
                    id: r.get(0)?,
                    owner_hash: r.get(1)?,
                    skill: r.get(2)?,
                    state: r.get(3)?,
                    input: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or(serde_json::json!({})),
                    artifacts: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
                    events: serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default(),
                    metadata: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or(serde_json::json!({})),
                    created_at: r.get(8)?,
                    updated_at: r.get(9)?,
                    expires_at: r.get(10)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(all
            .into_iter()
            .filter(|t| {
                (t.owner_hash.is_empty() || t.owner_hash == caller_owner)
                    && state.map(|s| t.state.eq_ignore_ascii_case(s)).unwrap_or(true)
                    && skill.map(|s| t.skill.eq_ignore_ascii_case(s)).unwrap_or(true)
            })
            .skip(offset)
            .take(limit)
            .collect())
    }
}

fn tf_vector(text: &str) -> HashMap<String, f64> {
    let mut map = HashMap::new();
    for tok in text
        .to_ascii_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() >= 2)
    {
        *map.entry(tok.to_string()).or_insert(0.0) += 1.0;
    }
    map
}

fn cosine_sim(a: &HashMap<String, f64>, b: &HashMap<String, f64>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let dot: f64 = a.iter().map(|(k, v)| v * b.get(k).copied().unwrap_or(0.0)).sum();
    let na = a.values().map(|v| v * v).sum::<f64>().sqrt();
    let nb = b.values().map(|v| v * v).sum::<f64>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn temp_store() -> Store {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = format!(
            "{}/omni-test-{}-{}",
            std::env::temp_dir().display(),
            std::process::id(),
            n
        );
        Store::open_default(&dir).unwrap()
    }

    #[test]
    fn open_door_when_no_keys_and_strict_once_seeded() {
        let s = temp_store();
        assert!(s.validate_api_key(None, 1_000).unwrap());
        s.add_api_key("sk-real", "test", 1_000).unwrap();
        assert!(!s.validate_api_key(None, 2_000).unwrap(), "missing key must be rejected once keys exist");
        assert!(!s.validate_api_key(Some(""), 2_000).unwrap());
        assert!(!s.validate_api_key(Some("sk-wrong"), 2_000).unwrap());
        assert!(s.validate_api_key(Some("sk-real"), 2_000).unwrap());
    }

    #[test]
    fn expired_key_rejected() {
        let s = temp_store();
        s.add_api_key_with_limits("sk-exp", "temp", 1_000, Some(5_000), Some(10.0), Some(60)).unwrap();
        assert!(s.validate_api_key(Some("sk-exp"), 4_000).unwrap());
        assert!(!s.validate_api_key(Some("sk-exp"), 6_000).unwrap());
    }

    #[test]
    fn combo_and_settings_and_gamification_roundtrip() {
        let s = temp_store();
        s.upsert_combo(&ComboRow {
            id: "c1".into(),
            name: "fast-free".into(),
            strategy: "jev".into(),
            auto_strategy: "decision".into(),
            models: vec![ComboStep {
                provider: "groq".into(),
                model: "openai/gpt-oss-120b".into(),
                weight: 2.0,
                fallback_only: false,
                connection_id: None,
            }],
            config: serde_json::json!({}),
            enabled: true,
            created_at: 100,
            updated_at: 100,
        })
        .unwrap();
        let fetched = s.get_combo("fast-free").unwrap().unwrap();
        assert_eq!(fetched.strategy, "jev");
        assert_eq!(fetched.models.len(), 1);

        s.set_setting("default_strategy", "expiry-first", 100).unwrap();
        assert_eq!(s.get_setting("default_strategy").unwrap().as_deref(), Some("expiry-first"));

        let mut g = s.load_gamification("default").unwrap();
        g.total_xp = 250;
        g.unlocked_badges.push("first-token".into());
        s.save_gamification(&g).unwrap();
        let g2 = s.load_gamification("default").unwrap();
        assert_eq!(g2.total_xp, 250);
        assert_eq!(g2.unlocked_badges, vec!["first-token"]);
    }

    #[test]
    fn oauth_credential_lifecycle_and_interval_zero_skip() {
        let s = temp_store();
        // Account expiring in 30s -> should be returned by list_expiring_oauth_credentials
        s.upsert_credential_full(&CredentialRow {
            provider: "claude".into(),
            connection_id: "acct-expiring".into(),
            auth_type: "oauth".into(),
            api_key: "at_old".into(),
            base_urls: vec![],
            models: vec![],
            enabled: true,
            priority: 1,
            weight: 1.0,
            label: "Primary".into(),
            refresh_token: Some("rt_1".into()),
            expires_at: Some(1_030_000),
            provider_specific_json: serde_json::json!({"plan": "pro"}),
            health_status: "healthy".into(),
            last_refreshed_at: None,
            health_check_interval_min: 15,
        })
        .unwrap();
        // Account with health_check_interval_min = 0 -> disabled per v3.8.55 #8443
        s.upsert_credential_full(&CredentialRow {
            provider: "codex".into(),
            connection_id: "acct-disabled-hc".into(),
            auth_type: "oauth".into(),
            api_key: "at_codex".into(),
            base_urls: vec![],
            models: vec![],
            enabled: true,
            priority: 0,
            weight: 1.0,
            label: "Manual".into(),
            refresh_token: Some("rt_2".into()),
            expires_at: Some(1_010_000),
            provider_specific_json: serde_json::json!({}),
            health_status: "healthy".into(),
            last_refreshed_at: None,
            health_check_interval_min: 0,
        })
        .unwrap();

        let expiring = s.list_expiring_oauth_credentials(1_000_000, 60_000).unwrap();
        assert_eq!(expiring.len(), 1);
        assert_eq!(expiring[0].connection_id, "acct-expiring");

        // Update tokens
        assert!(s
            .update_oauth_tokens(
                "claude",
                "acct-expiring",
                "at_new",
                Some("rt_new"),
                Some(5_000_000),
                None,
                1_005_000
            )
            .unwrap());
        let updated = s.get_credential("claude", "acct-expiring").unwrap().unwrap();
        assert_eq!(updated.api_key, "at_new");
        assert_eq!(updated.refresh_token.as_deref(), Some("rt_new"));
        assert_eq!(updated.expires_at, Some(5_000_000));
        assert_eq!(updated.last_refreshed_at, Some(1_005_000));

        // Mark needs_reauth excludes from expiring list
        s.mark_credential_health("claude", "acct-expiring", "needs_reauth", 1_010_000)
            .unwrap();
        let after = s.list_expiring_oauth_credentials(5_000_000, 60_000).unwrap();
        assert!(after.is_empty());
    }

    #[test]
    fn memory_retrieval_skills_and_a2a_owner_isolation() {
        let s = temp_store();
        s.upsert_memory(&MemoryRow {
            id: "m1".into(),
            api_key_hash: "key-a".into(),
            session_id: "s1".into(),
            mem_type: "factual".into(),
            mem_key: "rust-gateway".into(),
            content: "OmniRoute Rust gateway uses Axum and SQLite WAL for low memory usage".into(),
            category: "architecture".into(),
            metadata: serde_json::json!({}),
            tokens_est: 0,
            expires_at: None,
            created_at: 100,
            updated_at: 100,
        })
        .unwrap();
        let hits = s.retrieve_memories("Axum SQLite low memory", Some("key-a"), None, 200, 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0.id, "m1");
        assert!(hits[0].1 > 0.3);

        s.upsert_skill(&SkillRow {
            id: "sk1".into(),
            name: "sql-assistant".into(),
            description: "Helps optimize SQLite queries".into(),
            source_provider: "local".into(),
            mode: "on".into(),
            tags: vec!["sqlite".into(), "sql".into()],
            instructions: "Analyze query plans and recommend indexes".into(),
            schema: serde_json::json!({}),
            enabled: true,
            executions_count: 0,
            created_at: 100,
            updated_at: 100,
        })
        .unwrap();
        assert!(s.record_skill_execution("sql-assistant", 200).unwrap());
        assert_eq!(s.get_skill("sql-assistant").unwrap().unwrap().executions_count, 1);

        s.upsert_a2a_task(&A2ATaskRow {
            id: "task-1".into(),
            owner_hash: "owner-alpha".into(),
            skill: "health-report".into(),
            state: "completed".into(),
            input: serde_json::json!({}),
            artifacts: vec![serde_json::json!({"type": "text", "content": "ok"})],
            events: vec![],
            metadata: serde_json::json!({}),
            created_at: 100,
            updated_at: 100,
            expires_at: 400_000,
        })
        .unwrap();
        assert!(s.get_a2a_task("task-1", "owner-alpha").unwrap().is_some());
        assert!(
            s.get_a2a_task("task-1", "owner-beta").unwrap().is_none(),
            "cross-owner A2A task read must be isolated"
        );
    }

    #[test]
    fn upstream_storage_sqlite_import_json_export_and_hot_backup() {
        // 1. Build a genuine upstream Node.js OmniRoute `storage.sqlite` fixture
        let tmp = format!(
            "{}/omni-upstream-src-{}-{}.sqlite",
            std::env::temp_dir().display(),
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        );
        {
            let up = Connection::open(&tmp).unwrap();
            up.execute_batch(
                "CREATE TABLE _omniroute_migrations (id TEXT PRIMARY KEY);
                 INSERT INTO _omniroute_migrations VALUES ('001_initial'), ('253_v3855');
                 CREATE TABLE provider_connections (
                     id TEXT PRIMARY KEY, provider TEXT NOT NULL, auth_type TEXT, name TEXT,
                     priority INTEGER DEFAULT 0, is_active INTEGER DEFAULT 1,
                     api_key TEXT, access_token TEXT, refresh_token TEXT,
                     expires_at TEXT, provider_specific_data TEXT, health_check_interval INTEGER
                 );
                 INSERT INTO provider_connections VALUES (
                     'conn-up-1', 'claude', 'oauth', 'Claude Pro', 5, 1,
                     NULL, 'sk-ant-oat-up', 'rt-up-1', '2027-01-01T00:00:00Z', '{\"plan\":\"pro\"}', 30
                 );
                 CREATE TABLE combos (id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, data TEXT NOT NULL);
                 INSERT INTO combos VALUES (
                     'cmb-1', 'smart-fast',
                     '{\"strategy\":\"jev\",\"models\":[\"groq/openai/gpt-oss-120b\",{\"provider\":\"pollinations\",\"model\":\"openai-fast\",\"weight\":2}]}'
                 );
                 CREATE TABLE api_keys (id TEXT PRIMARY KEY, name TEXT NOT NULL, key TEXT NOT NULL UNIQUE, created_at TEXT NOT NULL);
                 INSERT INTO api_keys VALUES ('k1', 'prod', 'sk-from-upstream', '2026-11-01T00:00:00Z');
                 CREATE TABLE key_value (namespace TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY (namespace, key));
                 INSERT INTO key_value VALUES ('settings', 'default_strategy', '\"expiry-first\"');
                 INSERT INTO key_value VALUES ('modelAliases', 'fast', '\"smart-fast\"');
                 CREATE TABLE usage_history (
                     id INTEGER PRIMARY KEY AUTOINCREMENT, provider TEXT, model TEXT, connection_id TEXT,
                     tokens_input INTEGER, tokens_output INTEGER, tokens_cache_read INTEGER,
                     tokens_cache_creation INTEGER, latency_ms INTEGER, success INTEGER, timestamp TEXT NOT NULL
                 );
                 INSERT INTO usage_history(provider, model, connection_id, tokens_input, tokens_output, tokens_cache_read, tokens_cache_creation, latency_ms, success, timestamp)
                 VALUES ('claude', 'claude-opus-4-7', 'conn-up-1', 120, 40, 20, 0, 450, 1, '2026-11-24T10:00:00Z');",
            )
            .unwrap();
        }

        // 2. Import into omniroute-rs Store
        let s = temp_store();
        let rep = s.import_upstream_sqlite(&tmp).unwrap();
        assert_eq!(rep.upstream_migrations_detected, 2);
        assert_eq!(rep.provider_connections, 1);
        assert_eq!(rep.combos, 1);
        assert_eq!(rep.api_keys, 1);
        assert_eq!(rep.settings, 1);
        assert_eq!(rep.aliases, 1);
        assert_eq!(rep.usage_rows, 1);

        let cred = s.get_credential("claude", "conn-up-1").unwrap().unwrap();
        assert_eq!(cred.api_key, "sk-ant-oat-up");
        assert_eq!(cred.refresh_token.as_deref(), Some("rt-up-1"));
        assert_eq!(cred.health_check_interval_min, 30);
        assert_eq!(s.resolve_model_alias("fast").unwrap().as_deref(), Some("smart-fast"));
        assert_eq!(s.get_setting("default_strategy").unwrap().as_deref(), Some("expiry-first"));

        // 3. Export snapshot -> restore into a brand new Store -> verify roundtrip
        let snap = s.export_snapshot(true).unwrap();
        let s2 = temp_store();
        let rep2 = s2.import_upstream_json(&snap).unwrap();
        assert_eq!(rep2.provider_connections, 1);
        assert_eq!(rep2.combos, 1);
        assert_eq!(rep2.api_keys, 1);

        // 4. Diagnostics + hot VACUUM INTO backup
        let diag = s2.db_diagnostics().unwrap();
        assert_eq!(diag["ok"], true);
        let bak_path = format!("{tmp}.bak");
        let bak_size = s2.backup_to_file(&bak_path).unwrap();
        assert!(bak_size > 4096);
        let _ = std::fs::remove_file(&tmp);
        let _ = std::fs::remove_file(&bak_path);
    }
}
