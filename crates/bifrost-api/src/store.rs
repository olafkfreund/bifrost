//! Proposal persistence.
//!
//! A [`ProposalStore`] trait with three implementations: an in-memory store
//! (default, zero-config — used for tests and ephemeral runs), a SQLite store
//! (air-gap / single-tenant, #46), and a Postgres store (server/multi-tenant,
//! #45) — chosen by the `BIFROST_DB` URL scheme.
//!
//! Storage shape: the proposal body is a JSON document in `proposals`, while the
//! audit trail is a separate **append-only** table (`audit_log`) — rows are only
//! ever inserted, never updated or deleted, which is what makes it the
//! attestation artifact (#48).

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use bifrost_core::{
    AuditEvent, AuditLog, ConfigEvent, Connection, Proposal, ProposalStatus, Runbook,
};
use bifrost_llm::RoutingPolicy;
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{PgPool, Row, SqlitePool};
use tokio::sync::RwLock;

/// A converted proposal plus its append-only audit trail.
#[derive(Debug, Clone)]
pub struct StoredProposal {
    pub proposal: Proposal,
    pub runbook: Runbook,
    pub audit: AuditLog,
    /// Owning tenant (multi-tenancy, #66). `default` for single-tenant/air-gap.
    pub tenant: String,
}

#[async_trait]
pub trait ProposalStore: Send + Sync {
    /// Load a proposal (with its full audit trail) by id.
    async fn get(&self, id: &str) -> anyhow::Result<Option<StoredProposal>>;
    /// Upsert the proposal body and append any audit events not yet persisted.
    async fn put(&self, rec: &StoredProposal) -> anyhow::Result<()>;
    /// Every stored proposal (for the portfolio review-state overlay).
    async fn list(&self) -> anyhow::Result<Vec<StoredProposal>>;

    /// Upsert a tenant's connection (#154). The connection carries only secret
    /// *references* / encrypted material — never plaintext.
    async fn put_connection(&self, conn: &Connection) -> anyhow::Result<()>;
    /// A tenant's connections.
    async fn list_connections(&self, tenant: &str) -> anyhow::Result<Vec<Connection>>;
    /// Delete a tenant's connection by id; `true` if one was removed.
    async fn delete_connection(&self, tenant: &str, id: &str) -> anyhow::Result<bool>;

    /// Append a config-change event (#159). Append-only — for the compliance pack.
    async fn append_config_event(&self, ev: &ConfigEvent) -> anyhow::Result<()>;
    /// A tenant's config-change history, oldest first.
    async fn list_config_events(&self, tenant: &str) -> anyhow::Result<Vec<ConfigEvent>>;

    /// Upsert a tenant's LLM routing policy (#158).
    async fn put_routing_policy(&self, tenant: &str, policy: &RoutingPolicy) -> anyhow::Result<()>;
    /// A tenant's routing policy, if one has been set.
    async fn get_routing_policy(&self, tenant: &str) -> anyhow::Result<Option<RoutingPolicy>>;
}

/// Resolve the store from the environment via `BIFROST_DB`:
/// - `postgres://…` / `postgresql://…` → Postgres (server/multi-tenant, #45)
/// - any other non-empty value (a `sqlite:` URL or bare path) → SQLite (#46)
/// - unset → in-memory.
///
/// A connect failure degrades to in-memory so the server always starts.
pub async fn from_env() -> Arc<dyn ProposalStore> {
    let url = match std::env::var("BIFROST_DB") {
        Ok(u) if !u.trim().is_empty() => u,
        _ => {
            tracing::info!("proposals held in memory (set BIFROST_DB to persist)");
            return Arc::new(InMemoryStore::default());
        }
    };

    let connected: anyhow::Result<Arc<dyn ProposalStore>> =
        if url.starts_with("postgres://") || url.starts_with("postgresql://") {
            PgStore::connect(&url)
                .await
                .map(|s| Arc::new(s) as Arc<dyn ProposalStore>)
        } else {
            SqliteStore::connect(&url)
                .await
                .map(|s| Arc::new(s) as Arc<dyn ProposalStore>)
        };

    match connected {
        Ok(store) => {
            tracing::info!("proposals persisted ({url})");
            store
        }
        Err(e) => {
            tracing::warn!("BIFROST_DB connect failed: {e}; using in-memory store");
            Arc::new(InMemoryStore::default())
        }
    }
}

// ── in-memory ───────────────────────────────────────────────────────────────

/// A non-durable store backed by a map. Default when `BIFROST_DB` is unset.
#[derive(Default)]
pub struct InMemoryStore {
    inner: RwLock<HashMap<String, StoredProposal>>,
    connections: RwLock<HashMap<String, Connection>>,
    config_events: RwLock<Vec<ConfigEvent>>,
    routing: RwLock<HashMap<String, RoutingPolicy>>,
}

#[async_trait]
impl ProposalStore for InMemoryStore {
    async fn get(&self, id: &str) -> anyhow::Result<Option<StoredProposal>> {
        Ok(self.inner.read().await.get(id).cloned())
    }
    async fn put(&self, rec: &StoredProposal) -> anyhow::Result<()> {
        self.inner
            .write()
            .await
            .insert(rec.proposal.id.clone(), rec.clone());
        Ok(())
    }
    async fn list(&self) -> anyhow::Result<Vec<StoredProposal>> {
        Ok(self.inner.read().await.values().cloned().collect())
    }
    async fn put_connection(&self, conn: &Connection) -> anyhow::Result<()> {
        self.connections
            .write()
            .await
            .insert(conn.id.clone(), conn.clone());
        Ok(())
    }
    async fn list_connections(&self, tenant: &str) -> anyhow::Result<Vec<Connection>> {
        Ok(self
            .connections
            .read()
            .await
            .values()
            .filter(|c| c.tenant == tenant)
            .cloned()
            .collect())
    }
    async fn delete_connection(&self, tenant: &str, id: &str) -> anyhow::Result<bool> {
        let mut g = self.connections.write().await;
        match g.get(id) {
            Some(c) if c.tenant == tenant => {
                g.remove(id);
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    async fn append_config_event(&self, ev: &ConfigEvent) -> anyhow::Result<()> {
        self.config_events.write().await.push(ev.clone());
        Ok(())
    }
    async fn list_config_events(&self, tenant: &str) -> anyhow::Result<Vec<ConfigEvent>> {
        Ok(self
            .config_events
            .read()
            .await
            .iter()
            .filter(|e| e.tenant == tenant)
            .cloned()
            .collect())
    }
    async fn put_routing_policy(&self, tenant: &str, policy: &RoutingPolicy) -> anyhow::Result<()> {
        self.routing
            .write()
            .await
            .insert(tenant.to_string(), policy.clone());
        Ok(())
    }
    async fn get_routing_policy(&self, tenant: &str) -> anyhow::Result<Option<RoutingPolicy>> {
        Ok(self.routing.read().await.get(tenant).cloned())
    }
}

// ── SQLite ────────────────────────────────────────────────────────────────────

/// A SQLite-backed store. The schema is created on connect (idempotent), so no
/// external migration step is needed for the single-tenant/air-gap deployment.
pub struct SqliteStore {
    pool: SqlitePool,
}

fn status_to_str(s: ProposalStatus) -> String {
    // ProposalStatus is a serde snake_case enum; serialize to its wire string.
    serde_json::to_value(s)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn status_from_str(s: &str) -> anyhow::Result<ProposalStatus> {
    let value = serde_json::Value::String(s.to_string());
    Ok(serde_json::from_value(value)?)
}

impl SqliteStore {
    /// Connect (creating the file if missing) and ensure the schema exists.
    /// Accepts a `sqlite:`/`sqlite://` URL or a bare file path.
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        let normalized = if url.contains(':') {
            url.to_string()
        } else {
            format!("sqlite:{url}")
        };
        let opts = SqliteConnectOptions::from_str(&normalized)?.create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(opts)
            .await?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    async fn migrate(&self) -> anyhow::Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS proposals (
                id          TEXT PRIMARY KEY,
                pipeline_id TEXT NOT NULL,
                status      TEXT NOT NULL,
                tenant      TEXT NOT NULL DEFAULT 'default',
                doc         TEXT NOT NULL,
                runbook     TEXT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        // Upgrade DBs created before the tenant column (#66); ignore if it exists.
        let _ =
            sqlx::query("ALTER TABLE proposals ADD COLUMN tenant TEXT NOT NULL DEFAULT 'default'")
                .execute(&self.pool)
                .await;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS audit_log (
                seq         INTEGER PRIMARY KEY AUTOINCREMENT,
                proposal_id TEXT NOT NULL,
                actor       TEXT NOT NULL,
                from_status TEXT NOT NULL,
                to_status   TEXT NOT NULL,
                at          TEXT NOT NULL,
                note        TEXT
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_audit_proposal ON audit_log(proposal_id, seq)")
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS connections (
                id     TEXT PRIMARY KEY,
                tenant TEXT NOT NULL,
                doc    TEXT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS config_audit (
                seq    INTEGER PRIMARY KEY AUTOINCREMENT,
                tenant TEXT NOT NULL,
                doc    TEXT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS routing_policy (
                tenant TEXT PRIMARY KEY,
                doc    TEXT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn load_audit(&self, id: &str) -> anyhow::Result<AuditLog> {
        let rows = sqlx::query(
            "SELECT actor, from_status, to_status, at, note FROM audit_log
             WHERE proposal_id = ? ORDER BY seq",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await?;
        let mut audit = AuditLog::new();
        for r in rows {
            let from: String = r.get("from_status");
            let to: String = r.get("to_status");
            audit.append(AuditEvent {
                proposal_id: id.to_string(),
                actor: r.get("actor"),
                from: status_from_str(&from)?,
                to: status_from_str(&to)?,
                at: r.get("at"),
                note: r.get::<Option<String>, _>("note"),
            });
        }
        Ok(audit)
    }
}

#[async_trait]
impl ProposalStore for SqliteStore {
    async fn get(&self, id: &str) -> anyhow::Result<Option<StoredProposal>> {
        let row = sqlx::query("SELECT doc, runbook, tenant FROM proposals WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        let Some(row) = row else { return Ok(None) };
        let doc: String = row.get("doc");
        let runbook_json: String = row.get("runbook");
        let tenant: String = row.get("tenant");
        let proposal: Proposal = serde_json::from_str(&doc)?;
        let runbook: Runbook = serde_json::from_str(&runbook_json)?;
        let audit = self.load_audit(id).await?;
        Ok(Some(StoredProposal {
            proposal,
            runbook,
            audit,
            tenant,
        }))
    }

    async fn put(&self, rec: &StoredProposal) -> anyhow::Result<()> {
        let doc = serde_json::to_string(&rec.proposal)?;
        let runbook = serde_json::to_string(&rec.runbook)?;
        sqlx::query(
            "INSERT INTO proposals (id, pipeline_id, status, tenant, doc, runbook)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                status = excluded.status, tenant = excluded.tenant,
                doc = excluded.doc, runbook = excluded.runbook",
        )
        .bind(&rec.proposal.id)
        .bind(&rec.proposal.pipeline_id)
        .bind(status_to_str(rec.proposal.status))
        .bind(&rec.tenant)
        .bind(&doc)
        .bind(&runbook)
        .execute(&self.pool)
        .await?;

        // The audit log is append-only: insert only events not yet persisted.
        let stored: i64 = sqlx::query("SELECT COUNT(*) AS n FROM audit_log WHERE proposal_id = ?")
            .bind(&rec.proposal.id)
            .fetch_one(&self.pool)
            .await?
            .get("n");
        for ev in rec.audit.events().iter().skip(stored as usize) {
            sqlx::query(
                "INSERT INTO audit_log (proposal_id, actor, from_status, to_status, at, note)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&ev.proposal_id)
            .bind(&ev.actor)
            .bind(status_to_str(ev.from))
            .bind(status_to_str(ev.to))
            .bind(&ev.at)
            .bind(ev.note.as_deref())
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    async fn list(&self) -> anyhow::Result<Vec<StoredProposal>> {
        let ids = sqlx::query("SELECT id FROM proposals")
            .fetch_all(&self.pool)
            .await?;
        let mut out = Vec::with_capacity(ids.len());
        for r in ids {
            let id: String = r.get("id");
            if let Some(sp) = self.get(&id).await? {
                out.push(sp);
            }
        }
        Ok(out)
    }

    async fn put_connection(&self, conn: &Connection) -> anyhow::Result<()> {
        let doc = serde_json::to_string(conn)?;
        sqlx::query(
            "INSERT INTO connections (id, tenant, doc) VALUES (?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET tenant = excluded.tenant, doc = excluded.doc",
        )
        .bind(&conn.id)
        .bind(&conn.tenant)
        .bind(&doc)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_connections(&self, tenant: &str) -> anyhow::Result<Vec<Connection>> {
        let rows = sqlx::query("SELECT doc FROM connections WHERE tenant = ?")
            .bind(tenant)
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|r| Ok(serde_json::from_str(&r.get::<String, _>("doc"))?))
            .collect()
    }

    async fn delete_connection(&self, tenant: &str, id: &str) -> anyhow::Result<bool> {
        let res = sqlx::query("DELETE FROM connections WHERE tenant = ? AND id = ?")
            .bind(tenant)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    async fn append_config_event(&self, ev: &ConfigEvent) -> anyhow::Result<()> {
        sqlx::query("INSERT INTO config_audit (tenant, doc) VALUES (?, ?)")
            .bind(&ev.tenant)
            .bind(serde_json::to_string(ev)?)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list_config_events(&self, tenant: &str) -> anyhow::Result<Vec<ConfigEvent>> {
        let rows = sqlx::query("SELECT doc FROM config_audit WHERE tenant = ? ORDER BY seq")
            .bind(tenant)
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|r| Ok(serde_json::from_str(&r.get::<String, _>("doc"))?))
            .collect()
    }

    async fn put_routing_policy(&self, tenant: &str, policy: &RoutingPolicy) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO routing_policy (tenant, doc) VALUES (?, ?)
             ON CONFLICT(tenant) DO UPDATE SET doc = excluded.doc",
        )
        .bind(tenant)
        .bind(serde_json::to_string(policy)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_routing_policy(&self, tenant: &str) -> anyhow::Result<Option<RoutingPolicy>> {
        let row = sqlx::query("SELECT doc FROM routing_policy WHERE tenant = ?")
            .bind(tenant)
            .fetch_optional(&self.pool)
            .await?;
        Ok(match row {
            Some(r) => Some(serde_json::from_str(&r.get::<String, _>("doc"))?),
            None => None,
        })
    }
}

// ── Postgres ──────────────────────────────────────────────────────────────────

/// A Postgres-backed store for the server / multi-tenant deployment (#45). Same
/// logical schema as SQLite — a `proposals` document table plus an append-only
/// `audit_log` — created idempotently on connect. SQL uses `$n` placeholders and
/// `BIGSERIAL`, the Postgres dialect.
pub struct PgStore {
    pool: PgPool,
}

impl PgStore {
    /// Connect to `postgres://…` and ensure the schema exists.
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        let pool = PgPoolOptions::new().max_connections(5).connect(url).await?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    async fn migrate(&self) -> anyhow::Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS proposals (
                id          TEXT PRIMARY KEY,
                pipeline_id TEXT NOT NULL,
                status      TEXT NOT NULL,
                tenant      TEXT NOT NULL DEFAULT 'default',
                doc         TEXT NOT NULL,
                runbook     TEXT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        // Upgrade DBs created before the tenant column (#66); ignore if it exists.
        let _ = sqlx::query(
            "ALTER TABLE proposals ADD COLUMN IF NOT EXISTS tenant TEXT NOT NULL DEFAULT 'default'",
        )
        .execute(&self.pool)
        .await;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS audit_log (
                seq         BIGSERIAL PRIMARY KEY,
                proposal_id TEXT NOT NULL,
                actor       TEXT NOT NULL,
                from_status TEXT NOT NULL,
                to_status   TEXT NOT NULL,
                at          TEXT NOT NULL,
                note        TEXT
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_audit_proposal ON audit_log(proposal_id, seq)")
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS connections (
                id     TEXT PRIMARY KEY,
                tenant TEXT NOT NULL,
                doc    TEXT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS config_audit (
                seq    BIGSERIAL PRIMARY KEY,
                tenant TEXT NOT NULL,
                doc    TEXT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS routing_policy (
                tenant TEXT PRIMARY KEY,
                doc    TEXT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn load_audit(&self, id: &str) -> anyhow::Result<AuditLog> {
        let rows = sqlx::query(
            "SELECT actor, from_status, to_status, at, note FROM audit_log
             WHERE proposal_id = $1 ORDER BY seq",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await?;
        let mut audit = AuditLog::new();
        for r in rows {
            let from: String = r.get("from_status");
            let to: String = r.get("to_status");
            audit.append(AuditEvent {
                proposal_id: id.to_string(),
                actor: r.get("actor"),
                from: status_from_str(&from)?,
                to: status_from_str(&to)?,
                at: r.get("at"),
                note: r.get::<Option<String>, _>("note"),
            });
        }
        Ok(audit)
    }
}

#[async_trait]
impl ProposalStore for PgStore {
    async fn get(&self, id: &str) -> anyhow::Result<Option<StoredProposal>> {
        let row = sqlx::query("SELECT doc, runbook, tenant FROM proposals WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        let Some(row) = row else { return Ok(None) };
        let doc: String = row.get("doc");
        let runbook_json: String = row.get("runbook");
        let tenant: String = row.get("tenant");
        let proposal: Proposal = serde_json::from_str(&doc)?;
        let runbook: Runbook = serde_json::from_str(&runbook_json)?;
        let audit = self.load_audit(id).await?;
        Ok(Some(StoredProposal {
            proposal,
            runbook,
            audit,
            tenant,
        }))
    }

    async fn put(&self, rec: &StoredProposal) -> anyhow::Result<()> {
        let doc = serde_json::to_string(&rec.proposal)?;
        let runbook = serde_json::to_string(&rec.runbook)?;
        sqlx::query(
            "INSERT INTO proposals (id, pipeline_id, status, tenant, doc, runbook)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (id) DO UPDATE SET
                status = EXCLUDED.status, tenant = EXCLUDED.tenant,
                doc = EXCLUDED.doc, runbook = EXCLUDED.runbook",
        )
        .bind(&rec.proposal.id)
        .bind(&rec.proposal.pipeline_id)
        .bind(status_to_str(rec.proposal.status))
        .bind(&rec.tenant)
        .bind(&doc)
        .bind(&runbook)
        .execute(&self.pool)
        .await?;

        // The audit log is append-only: insert only events not yet persisted.
        let stored: i64 = sqlx::query("SELECT COUNT(*) AS n FROM audit_log WHERE proposal_id = $1")
            .bind(&rec.proposal.id)
            .fetch_one(&self.pool)
            .await?
            .get("n");
        for ev in rec.audit.events().iter().skip(stored as usize) {
            sqlx::query(
                "INSERT INTO audit_log (proposal_id, actor, from_status, to_status, at, note)
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(&ev.proposal_id)
            .bind(&ev.actor)
            .bind(status_to_str(ev.from))
            .bind(status_to_str(ev.to))
            .bind(&ev.at)
            .bind(ev.note.as_deref())
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    async fn list(&self) -> anyhow::Result<Vec<StoredProposal>> {
        let ids = sqlx::query("SELECT id FROM proposals")
            .fetch_all(&self.pool)
            .await?;
        let mut out = Vec::with_capacity(ids.len());
        for r in ids {
            let id: String = r.get("id");
            if let Some(sp) = self.get(&id).await? {
                out.push(sp);
            }
        }
        Ok(out)
    }

    async fn put_connection(&self, conn: &Connection) -> anyhow::Result<()> {
        let doc = serde_json::to_string(conn)?;
        sqlx::query(
            "INSERT INTO connections (id, tenant, doc) VALUES ($1, $2, $3)
             ON CONFLICT (id) DO UPDATE SET tenant = EXCLUDED.tenant, doc = EXCLUDED.doc",
        )
        .bind(&conn.id)
        .bind(&conn.tenant)
        .bind(&doc)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_connections(&self, tenant: &str) -> anyhow::Result<Vec<Connection>> {
        let rows = sqlx::query("SELECT doc FROM connections WHERE tenant = $1")
            .bind(tenant)
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|r| Ok(serde_json::from_str(&r.get::<String, _>("doc"))?))
            .collect()
    }

    async fn delete_connection(&self, tenant: &str, id: &str) -> anyhow::Result<bool> {
        let res = sqlx::query("DELETE FROM connections WHERE tenant = $1 AND id = $2")
            .bind(tenant)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    async fn append_config_event(&self, ev: &ConfigEvent) -> anyhow::Result<()> {
        sqlx::query("INSERT INTO config_audit (tenant, doc) VALUES ($1, $2)")
            .bind(&ev.tenant)
            .bind(serde_json::to_string(ev)?)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list_config_events(&self, tenant: &str) -> anyhow::Result<Vec<ConfigEvent>> {
        let rows = sqlx::query("SELECT doc FROM config_audit WHERE tenant = $1 ORDER BY seq")
            .bind(tenant)
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|r| Ok(serde_json::from_str(&r.get::<String, _>("doc"))?))
            .collect()
    }

    async fn put_routing_policy(&self, tenant: &str, policy: &RoutingPolicy) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO routing_policy (tenant, doc) VALUES ($1, $2)
             ON CONFLICT (tenant) DO UPDATE SET doc = EXCLUDED.doc",
        )
        .bind(tenant)
        .bind(serde_json::to_string(policy)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_routing_policy(&self, tenant: &str) -> anyhow::Result<Option<RoutingPolicy>> {
        let row = sqlx::query("SELECT doc FROM routing_policy WHERE tenant = $1")
            .bind(tenant)
            .fetch_optional(&self.pool)
            .await?;
        Ok(match row {
            Some(r) => Some(serde_json::from_str(&r.get::<String, _>("doc"))?),
            None => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::{assess, RiskSignals};

    fn sample_proposal(id: &str) -> StoredProposal {
        let assessment = assess(&RiskSignals::default());
        let proposal = Proposal::new(
            id,
            "web-portal-ci",
            "trigger: { branches: [main] }",
            "on: push",
            "rationale",
            vec![],
            vec![],
            "gap-fill.v1",
            1.0,
            &assessment,
        );
        StoredProposal {
            proposal,
            runbook: Runbook::default(),
            audit: AuditLog::new(),
            tenant: "default".to_string(),
        }
    }

    async fn temp_store(name: &str) -> SqliteStore {
        let path = std::env::temp_dir().join(format!("bifrost-store-test-{name}.db"));
        let _ = std::fs::remove_file(&path);
        SqliteStore::connect(path.to_str().unwrap()).await.unwrap()
    }

    #[tokio::test]
    async fn sqlite_round_trips_a_proposal_and_appends_audit() {
        let store = temp_store("roundtrip").await;
        let mut rec = sample_proposal("prop-rt");

        store.put(&rec).await.unwrap();
        let got = store.get("prop-rt").await.unwrap().expect("present");
        assert_eq!(got.proposal.status, ProposalStatus::Draft);
        assert!(got.audit.is_empty());

        // Transition + persist: the audit event lands in the append-only table.
        rec.proposal
            .transition(ProposalStatus::InReview, "olaf", "t1", &mut rec.audit)
            .unwrap();
        store.put(&rec).await.unwrap();

        let got = store.get("prop-rt").await.unwrap().unwrap();
        assert_eq!(got.proposal.status, ProposalStatus::InReview);
        assert_eq!(got.audit.len(), 1);
        assert_eq!(got.audit.events()[0].actor, "olaf");

        // Idempotent re-put does not duplicate the (already-stored) event.
        store.put(&rec).await.unwrap();
        assert_eq!(store.get("prop-rt").await.unwrap().unwrap().audit.len(), 1);
    }

    #[tokio::test]
    async fn sqlite_round_trips_the_tenant() {
        let store = temp_store("tenant").await;
        let mut rec = sample_proposal("prop-t");
        rec.tenant = "acme".into();
        store.put(&rec).await.unwrap();
        assert_eq!(store.get("prop-t").await.unwrap().unwrap().tenant, "acme");
    }

    #[tokio::test]
    async fn sqlite_persists_across_reconnect() {
        let path = std::env::temp_dir().join("bifrost-store-test-reconnect.db");
        let _ = std::fs::remove_file(&path);
        let url = path.to_str().unwrap();

        {
            let store = SqliteStore::connect(url).await.unwrap();
            store.put(&sample_proposal("prop-persist")).await.unwrap();
        }
        // A fresh connection to the same file sees the durable proposal.
        let store = SqliteStore::connect(url).await.unwrap();
        assert!(store.get("prop-persist").await.unwrap().is_some());
        assert_eq!(store.list().await.unwrap().len(), 1);
    }

    /// Exercises the Postgres impl against a real server. Skipped by default; run
    /// with a throwaway DB, e.g.:
    ///   `BIFROST_PG_TEST_URL=postgres://postgres:postgres@localhost:5432/postgres \
    ///    cargo test -p bifrost-api postgres_ -- --ignored`
    #[tokio::test]
    #[ignore = "requires a Postgres server in BIFROST_PG_TEST_URL"]
    async fn postgres_round_trips_and_appends_audit() {
        let url = std::env::var("BIFROST_PG_TEST_URL").expect("BIFROST_PG_TEST_URL set");
        let store = PgStore::connect(&url).await.unwrap();
        // Clean slate (disposable test DB).
        sqlx::query("DELETE FROM audit_log")
            .execute(&store.pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM proposals")
            .execute(&store.pool)
            .await
            .unwrap();

        let mut rec = sample_proposal("prop-pg");
        store.put(&rec).await.unwrap();
        assert_eq!(
            store.get("prop-pg").await.unwrap().unwrap().proposal.status,
            ProposalStatus::Draft
        );

        rec.proposal
            .transition(ProposalStatus::InReview, "olaf", "t1", &mut rec.audit)
            .unwrap();
        store.put(&rec).await.unwrap();
        store.put(&rec).await.unwrap(); // idempotent append

        let got = store.get("prop-pg").await.unwrap().unwrap();
        assert_eq!(got.proposal.status, ProposalStatus::InReview);
        assert_eq!(got.audit.len(), 1);
        assert_eq!(got.audit.events()[0].actor, "olaf");
        assert_eq!(store.list().await.unwrap().len(), 1);
    }
}
