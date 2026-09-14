// mavis_core/src/memory/long_term.rs
// Long-term memory: one compressed summary per day, distilled from the
// high-importance entries in the recall store.
//
// The point is survival past decay. Recall purges ordinary statements after
// 90 days and questions after 30; without consolidation, everything except
// stated preferences eventually disappears. A day's summary is small enough
// to keep indefinitely.
//
// Summaries are written by the worker LLM (see the consolidation task in
// main.rs) — this module only stores and retrieves them.

#![allow(dead_code)]

use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct DailySummary {
    pub date: String,
    pub summary: String,
    pub source_count: i64,
}

pub struct LongTermMemory {
    conn: Connection,
}

impl LongTermMemory {
    pub fn new(db_path: &Path) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS daily_summaries (
                date TEXT PRIMARY KEY,
                summary TEXT NOT NULL,
                source_count INTEGER NOT NULL,
                created_at TEXT NOT NULL
            )",
            [],
        )?;
        // Same FTS5 approach as the recall store — consistent, already
        // proven here, and no new dependency.
        conn.execute(
            "CREATE VIRTUAL TABLE IF NOT EXISTS summaries_fts USING fts5(
                summary,
                date UNINDEXED
            )",
            [],
        )?;
        Ok(Self { conn })
    }

    /// True if this date has already been consolidated. Checked before
    /// spending an LLM call on it.
    pub fn has_summary(&self, date: &str) -> Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM daily_summaries WHERE date = ?1",
            params![date],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    pub fn store_summary(&self, date: &str, summary: &str, source_count: i64) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO daily_summaries (date, summary, source_count, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(date) DO UPDATE SET
                summary = excluded.summary,
                source_count = excluded.source_count,
                created_at = excluded.created_at",
            params![date, summary, source_count, now],
        )?;
        self.conn
            .execute("DELETE FROM summaries_fts WHERE date = ?1", params![date])?;
        self.conn.execute(
            "INSERT INTO summaries_fts (summary, date) VALUES (?1, ?2)",
            params![summary, date],
        )?;
        log::info!(
            "LongTermMemory: stored summary for {} ({} source memories)",
            date,
            source_count
        );
        Ok(())
    }

    /// Summaries for a date range, oldest first. Dates are YYYY-MM-DD, so
    /// string comparison orders correctly.
    pub fn summaries_between(&self, start: &str, end: &str) -> Result<Vec<DailySummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT date, summary, source_count FROM daily_summaries
             WHERE date >= ?1 AND date <= ?2
             ORDER BY date ASC",
        )?;
        let rows = stmt.query_map(params![start, end], |row| {
            Ok(DailySummary {
                date: row.get(0)?,
                summary: row.get(1)?,
                source_count: row.get(2)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.into())
    }

    /// Keyword search across summaries. `fts_query` must already be a valid
    /// FTS5 expression — build it with the recall store's query builder so
    /// arbitrary speech can't break MATCH syntax.
    pub fn search(&self, fts_query: &str, limit: usize) -> Result<Vec<DailySummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT f.date, f.summary, COALESCE(d.source_count, 0)
             FROM summaries_fts f
             LEFT JOIN daily_summaries d ON d.date = f.date
             WHERE summaries_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![fts_query, limit as i64], |row| {
            Ok(DailySummary {
                date: row.get(0)?,
                summary: row.get(1)?,
                source_count: row.get(2)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.into())
    }

    pub fn count(&self) -> Result<i64> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM daily_summaries", [], |r| r.get(0))?;
        Ok(n)
    }
}