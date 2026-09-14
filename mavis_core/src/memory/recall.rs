// mavis_core/src/memory/recall.rs
// Searchable long-term memory.
//
// Uses SQLite's built-in FTS5 rather than vector embeddings. The phase plan
// called for sentence-transformers + FAISS, which is three new dependencies
// and an ~80 MB model download; full-text search covers a large share of the
// same ground for none of that cost. If recall proves too literal in
// practice — missing "audio problem" when the memory says "microphone bug" —
// embeddings become a decision backed by evidence rather than an assumption.

#![allow(dead_code)]

use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;

/// Words too common to narrow a search, dropped before querying.
const STOPWORDS: &[&str] = &[
    "the", "a", "an", "is", "are", "was", "were", "do", "did", "does", "what", "when",
    "where", "who", "how", "why", "i", "you", "me", "my", "your", "we", "it", "that",
    "this", "of", "to", "in", "on", "for", "and", "or", "about", "tell", "can", "with",
];

/// Phrases where the user is stating something durable about themselves —
/// worth remembering long after the conversation ends.
const SELF_FACT_MARKERS: &[&str] = &[
    "my name is", "call me", "i work", "i use", "i prefer", "i like", "i hate",
    "i'm working on", "i am working on", "remember that", "don't forget",
    "i always", "i usually", "my project", "my setup", "i live",
];

#[derive(Debug, Clone)]
pub struct Memory {
    pub text: String,
    pub role: String,
    pub timestamp: String,
    pub importance: i64,
}

pub struct RecallStore {
    conn: Connection,
}

impl RecallStore {
    pub fn new(db_path: &Path) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        // Standalone FTS5 table rather than an external-content one: the
        // latter needs triggers to stay in sync with its base table, and
        // there's nothing here that a plain table gives us.
        conn.execute(
            "CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
                text,
                role UNINDEXED,
                timestamp UNINDEXED,
                importance UNINDEXED
            )",
            [],
        )?;
        Ok(Self { conn })
    }

    pub fn record(&self, role: &str, text: &str, timestamp: &str) -> Result<()> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(());
        }
        let importance = score_importance(role, text);
        // Nothing below 2 is worth storing — "yes", "open firefox", and
        // other one-shot commands would just dilute later searches.
        if importance < 2 {
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO memory_fts (text, role, timestamp, importance)
             VALUES (?1, ?2, ?3, ?4)",
            params![text, role, timestamp, importance],
        )?;
        Ok(())
    }

    /// Find memories relevant to what the user just said. Returns empty when
    /// nothing genuinely matches — recall should stay quiet rather than pad
    /// the prompt with loosely-related noise.
    pub fn recall(&self, query: &str, limit: usize) -> Result<Vec<Memory>> {
        let fts = match build_fts_query(query) {
            Some(q) => q,
            None => return Ok(Vec::new()),
        };

        let mut stmt = self.conn.prepare(
            "SELECT text, role, timestamp, importance
             FROM memory_fts
             WHERE memory_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![fts, limit as i64], |row| {
            Ok(Memory {
                text: row.get(0)?,
                role: row.get(1)?,
                timestamp: row.get(2)?,
                importance: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.into())
    }

    pub fn count(&self) -> Result<i64> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM memory_fts", [], |r| r.get(0))?;
        Ok(n)
    }

    /// Everything recorded between two RFC3339 timestamps, oldest first.
    /// Answers "what was I doing yesterday afternoon" — the data was always
    /// there, nothing ever read it back by time.
    pub fn recall_between(&self, start: &str, end: &str, limit: usize) -> Result<Vec<Memory>> {
        let mut stmt = self.conn.prepare(
            "SELECT text, role, timestamp, importance
             FROM memory_fts
             WHERE timestamp >= ?1 AND timestamp <= ?2
             ORDER BY timestamp ASC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![start, end, limit as i64], |row| {
            Ok(Memory {
                text: row.get(0)?,
                role: row.get(1)?,
                timestamp: row.get(2)?,
                importance: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.into())
    }

    /// Drop memories that have outlived their usefulness.
    ///
    /// Retention scales with importance rather than a flat 30 days: a stated
    /// preference is worth keeping indefinitely, while "what time is it"
    /// stops being useful almost immediately. Without this the store grows
    /// without bound and old chatter starts crowding real matches out of
    /// search results.
    pub fn purge_expired(&self) -> Result<usize> {
        let now = chrono::Utc::now();
        let cutoff = |days: i64| (now - chrono::Duration::days(days)).to_rfc3339();

        // (max importance for this bucket, retention in days)
        let policy = [
            (2, 7),   // MAVIS's own replies — context, not knowledge
            (4, 30),  // questions and passing remarks
            (7, 90),  // ordinary statements
                      // importance >= 8 (stated facts, preferences) never expires
        ];

        let mut removed = 0usize;
        for (max_importance, days) in policy {
            removed += self.conn.execute(
                "DELETE FROM memory_fts WHERE importance <= ?1 AND timestamp < ?2",
                params![max_importance, cutoff(days)],
            )?;
        }

        if removed > 0 {
            log::info!("RecallStore: purged {} expired memories", removed);
        }
        Ok(removed)
    }
}

/// Turn arbitrary speech into a valid FTS5 MATCH expression.
///
/// This has to be defensive: FTS5 has its own query syntax, and characters
/// that routinely appear in transcribed speech (quotes, hyphens, colons,
/// asterisks) are operators there. Rather than escape them, extract plain
/// alphanumeric terms and OR them together.
fn build_fts_query(raw: &str) -> Option<String> {
    let terms: Vec<String> = raw
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| w.len() > 2 && !STOPWORDS.contains(w))
        .take(8)
        .map(|w| format!("\"{}\"", w))
        .collect();

    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" OR "))
    }
}

/// Rate how worth remembering an utterance is, 1-10.
///
/// Deliberately a heuristic and not an LLM call. The phase plan specified
/// "lightweight LLM call rates memory importance", but the model is already
/// the latency bottleneck, and adding a round trip after every interaction
/// to score it would be felt on every single exchange.
fn score_importance(role: &str, text: &str) -> i64 {
    let t = text.to_lowercase();

    if role != "user" {
        return 2;
    }

    if SELF_FACT_MARKERS.iter().any(|m| t.contains(m)) {
        return 9;
    }

    let trimmed = t.trim_end_matches(|c: char| c.is_ascii_punctuation());
    let is_question = text.trim_end().ends_with('?')
        || [
            "what", "when", "where", "who", "how", "why", "is ", "are ", "do ", "did ",
            "can ",
        ]
        .iter()
        .any(|p| trimmed.starts_with(p));
    if is_question {
        return 3;
    }

    if t.split_whitespace().count() <= 3 {
        return 1;
    }

    5
}