use std::path::Path;
use std::str::FromStr;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::error::LoongMemoryError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuditEventKind {
    Allowed,
    Denied,
    Read,
    Write,
    Recall,
    Delete,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub event_id: String,
    pub timestamp: DateTime<Utc>,
    pub principal: String,
    pub namespace: String,
    pub action: String,
    pub kind: AuditEventKind,
    pub detail: serde_json::Value,
}

impl AuditEventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuditEventKind::Allowed => "allowed",
            AuditEventKind::Denied => "denied",
            AuditEventKind::Read => "read",
            AuditEventKind::Write => "write",
            AuditEventKind::Recall => "recall",
            AuditEventKind::Delete => "delete",
            AuditEventKind::Unknown => "unknown",
        }
    }
}

impl FromStr for AuditEventKind {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "allowed" => AuditEventKind::Allowed,
            "denied" => AuditEventKind::Denied,
            "read" => AuditEventKind::Read,
            "write" => AuditEventKind::Write,
            "recall" => AuditEventKind::Recall,
            "delete" => AuditEventKind::Delete,
            _ => AuditEventKind::Unknown,
        })
    }
}

pub trait AuditSink: Send + Sync {
    fn record(&self, event: AuditEvent);
}

#[derive(Debug, Default)]
pub struct InMemoryAuditSink {
    events: std::sync::Mutex<Vec<AuditEvent>>,
}

impl InMemoryAuditSink {
    pub fn snapshot(&self) -> Vec<AuditEvent> {
        self.events.lock().map(|v| v.clone()).unwrap_or_default()
    }
}

impl AuditSink for InMemoryAuditSink {
    fn record(&self, event: AuditEvent) {
        if let Ok(mut guard) = self.events.lock() {
            guard.push(event);
        }
    }
}

fn ensure_audit_schema(conn: &Connection) -> Result<(), LoongMemoryError> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS memory_audit (
            event_id TEXT PRIMARY KEY,
            timestamp TEXT NOT NULL,
            principal TEXT NOT NULL,
            namespace TEXT NOT NULL,
            action TEXT NOT NULL,
            kind TEXT NOT NULL,
            detail TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_memory_audit_namespace_time
            ON memory_audit(namespace, timestamp DESC);
        "#,
    )
    .map_err(|e| LoongMemoryError::Storage(format!("init memory_audit schema: {e}")))?;
    Ok(())
}

fn open_sqlite_conn(path: impl AsRef<Path>) -> Result<Connection, LoongMemoryError> {
    let conn = Connection::open(path)
        .map_err(|e| LoongMemoryError::Storage(format!("open audit sqlite connection: {e}")))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| LoongMemoryError::Storage(format!("set journal_mode=WAL: {e}")))?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(|e| LoongMemoryError::Storage(format!("set synchronous=NORMAL: {e}")))?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|e| LoongMemoryError::Storage(format!("set foreign_keys=ON: {e}")))?;
    conn.busy_timeout(std::time::Duration::from_millis(5_000))
        .map_err(|e| LoongMemoryError::Storage(format!("set busy_timeout: {e}")))?;
    ensure_audit_schema(&conn)?;
    Ok(conn)
}

#[derive(Debug)]
pub struct SqliteAuditSink {
    conn: Mutex<Connection>,
}

impl SqliteAuditSink {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, LoongMemoryError> {
        let conn = open_sqlite_conn(path)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
}

impl AuditSink for SqliteAuditSink {
    fn record(&self, event: AuditEvent) {
        let Ok(detail) = serde_json::to_string(&event.detail) else {
            return;
        };
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                r#"
                INSERT OR REPLACE INTO memory_audit (
                    event_id, timestamp, principal, namespace, action, kind, detail
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                "#,
                params![
                    event.event_id,
                    event.timestamp.to_rfc3339(),
                    event.principal,
                    event.namespace,
                    event.action,
                    event.kind.as_str(),
                    detail
                ],
            );
        }
    }
}

#[derive(Debug)]
pub struct SqliteAuditLog {
    conn: Connection,
}

impl SqliteAuditLog {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, LoongMemoryError> {
        let conn = open_sqlite_conn(path)?;
        Ok(Self { conn })
    }

    pub fn list(
        &self,
        namespace: Option<&str>,
        limit: usize,
    ) -> Result<Vec<AuditEvent>, LoongMemoryError> {
        let limit = limit.clamp(1, 2_000) as i64;
        let mut out = Vec::new();

        if let Some(ns) = namespace {
            let mut stmt = self
                .conn
                .prepare(
                    r#"
                    SELECT event_id, timestamp, principal, namespace, action, kind, detail
                    FROM memory_audit
                    WHERE namespace = ?1
                    ORDER BY timestamp DESC
                    LIMIT ?2
                    "#,
                )
                .map_err(|e| LoongMemoryError::Storage(format!("prepare audit query: {e}")))?;
            let mut rows = stmt
                .query(params![ns, limit])
                .map_err(|e| LoongMemoryError::Storage(format!("query audit rows: {e}")))?;
            while let Some(row) = rows
                .next()
                .map_err(|e| LoongMemoryError::Storage(format!("scan audit rows: {e}")))?
            {
                out.push(parse_audit_row(row)?);
            }
            return Ok(out);
        }

        let mut stmt = self
            .conn
            .prepare(
                r#"
                SELECT event_id, timestamp, principal, namespace, action, kind, detail
                FROM memory_audit
                ORDER BY timestamp DESC
                LIMIT ?1
                "#,
            )
            .map_err(|e| LoongMemoryError::Storage(format!("prepare audit query: {e}")))?;
        let mut rows = stmt
            .query(params![limit])
            .map_err(|e| LoongMemoryError::Storage(format!("query audit rows: {e}")))?;
        while let Some(row) = rows
            .next()
            .map_err(|e| LoongMemoryError::Storage(format!("scan audit rows: {e}")))?
        {
            out.push(parse_audit_row(row)?);
        }

        Ok(out)
    }

    pub fn get_by_id(&self, event_id: &str) -> Result<Option<AuditEvent>, LoongMemoryError> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
                SELECT event_id, timestamp, principal, namespace, action, kind, detail
                FROM memory_audit
                WHERE event_id = ?1
                "#,
            )
            .map_err(|e| LoongMemoryError::Storage(format!("prepare audit query by id: {e}")))?;
        let mut rows = stmt
            .query(params![event_id])
            .map_err(|e| LoongMemoryError::Storage(format!("query audit by id: {e}")))?;
        let Some(row) = rows
            .next()
            .map_err(|e| LoongMemoryError::Storage(format!("scan audit by id: {e}")))?
        else {
            return Ok(None);
        };
        Ok(Some(parse_audit_row(row)?))
    }
}

fn parse_audit_row(row: &rusqlite::Row<'_>) -> Result<AuditEvent, LoongMemoryError> {
    let timestamp: String = row
        .get("timestamp")
        .map_err(|e| LoongMemoryError::Storage(format!("read audit timestamp: {e}")))?;
    let kind: String = row
        .get("kind")
        .map_err(|e| LoongMemoryError::Storage(format!("read audit kind: {e}")))?;
    let detail: String = row
        .get("detail")
        .map_err(|e| LoongMemoryError::Storage(format!("read audit detail: {e}")))?;

    let parsed_time = DateTime::parse_from_rfc3339(&timestamp)
        .map_err(|e| LoongMemoryError::Storage(format!("parse audit timestamp: {e}")))?
        .with_timezone(&Utc);

    let parsed_detail = serde_json::from_str(&detail)
        .map_err(|e| LoongMemoryError::Storage(format!("parse audit detail json: {e}")))?;

    Ok(AuditEvent {
        event_id: row
            .get("event_id")
            .map_err(|e| LoongMemoryError::Storage(format!("read audit event_id: {e}")))?,
        timestamp: parsed_time,
        principal: row
            .get("principal")
            .map_err(|e| LoongMemoryError::Storage(format!("read audit principal: {e}")))?,
        namespace: row
            .get("namespace")
            .map_err(|e| LoongMemoryError::Storage(format!("read audit namespace: {e}")))?,
        action: row
            .get("action")
            .map_err(|e| LoongMemoryError::Storage(format!("read audit action: {e}")))?,
        kind: kind.parse().unwrap_or(AuditEventKind::Unknown),
        detail: parsed_detail,
    })
}
