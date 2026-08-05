use rusqlite::{Connection, params};
use anyhow::Result;
use std::path::Path;
use crate::models::event::Event;

pub struct EpisodicStore {
    conn: Connection,
}

#[derive(Debug)]
pub struct Episode {
    pub id: i64,
    pub event_id: String,
    pub event_type: String,
    pub source: String,
    pub payload: String,
    pub timestamp: String,
    pub summary: String,
}

impl EpisodicStore {
    pub fn new(db_path: &Path) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS episodes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                event_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                source TEXT NOT NULL,
                payload TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                summary TEXT
            )",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_episodes_timestamp ON episodes(timestamp)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_episodes_type ON episodes(event_type)",
            [],
        )?;
        Ok(Self { conn })
    }

    pub fn record(&self, event: &Event) -> Result<()> {
        let summary = event.payload.get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        self.conn.execute(
            "INSERT INTO episodes (event_id, event_type, source, payload, timestamp, summary)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                event.id.to_string(),
                format!("{:?}", event.event_type),
                event.source,
                event.payload.to_string(),
                event.timestamp.to_rfc3339(),
                summary
            ],
        )?;
        Ok(())
    }

    pub fn recent(&self, n: usize) -> Result<Vec<Episode>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, event_id, event_type, source, payload, timestamp, summary
             FROM episodes ORDER BY timestamp DESC LIMIT ?1"
        )?;
        let rows = stmt.query_map(params![n as i64], |row| {
            Ok(Episode {
                id: row.get(0)?,
                event_id: row.get(1)?,
                event_type: row.get(2)?,
                source: row.get(3)?,
                payload: row.get(4)?,
                timestamp: row.get(5)?,
                summary: row.get(6)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.into())
    }

    pub fn search(&self, query: &str, n: usize) -> Result<Vec<Episode>> {
        let pattern = format!("%{}%", query);
        let mut stmt = self.conn.prepare(
            "SELECT id, event_id, event_type, source, payload, timestamp, summary
             FROM episodes
             WHERE summary LIKE ?1 OR payload LIKE ?1 OR event_type LIKE ?1
             ORDER BY timestamp DESC LIMIT ?2"
        )?;
        let rows = stmt.query_map(params![pattern, n as i64], |row| {
            Ok(Episode {
                id: row.get(0)?,
                event_id: row.get(1)?,
                event_type: row.get(2)?,
                source: row.get(3)?,
                payload: row.get(4)?,
                timestamp: row.get(5)?,
                summary: row.get(6)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.into())
    }
}
