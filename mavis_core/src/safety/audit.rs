// mavis_core/src/safety/audit.rs
// Append-only record of every action MAVIS was asked to take.
//
// Append-only on purpose: an audit log that can be edited by the thing it
// audits is decoration. Nothing here updates or deletes, and the store
// exposes no method to do so.

#![allow(dead_code)]

use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct AuditEntry {
    pub timestamp: String,
    pub action_type: String,
    pub detail: String,
    pub risk_score: u8,
    pub outcome: String,
    pub reason: String,
}

pub struct AuditLog {
    conn: Connection,
}

impl AuditLog {
    pub fn new(db_path: &Path) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS audit (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                action_type TEXT NOT NULL,
                detail TEXT NOT NULL,
                risk_score INTEGER NOT NULL,
                outcome TEXT NOT NULL,
                reason TEXT NOT NULL
            )",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_audit_time ON audit(timestamp)",
            [],
        )?;
        Ok(Self { conn })
    }

    pub fn record(
        &self,
        action_type: &str,
        detail: &str,
        risk_score: u8,
        outcome: &str,
        reason: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO audit (timestamp, action_type, detail, risk_score, outcome, reason)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                chrono::Utc::now().to_rfc3339(),
                action_type,
                detail,
                risk_score as i64,
                outcome,
                reason
            ],
        )?;
        Ok(())
    }

    /// Most recent entries, newest first. For "what have you done today".
    pub fn recent(&self, limit: usize) -> Result<Vec<AuditEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT timestamp, action_type, detail, risk_score, outcome, reason
             FROM audit ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(AuditEntry {
                timestamp: row.get(0)?,
                action_type: row.get(1)?,
                detail: row.get(2)?,
                risk_score: row.get::<_, i64>(3)? as u8,
                outcome: row.get(4)?,
                reason: row.get(5)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.into())
    }

    pub fn count(&self) -> Result<i64> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM audit", [], |r| r.get(0))?;
        Ok(n)
    }
}