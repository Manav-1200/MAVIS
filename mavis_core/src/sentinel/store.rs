// mavis_core/src/sentinel/store.rs
// Where observed changes are kept.
//
// Two jobs, and they are deliberately separate:
//
//   1. A watermark per source — the timestamp of the newest entry MAVIS
//      has already processed. Package logs are append-only and long (a
//      real Arch log carries 2079 transactions), so without a watermark
//      the first scan would announce the machine's entire history.
//
//   2. The changes themselves, keyed by fingerprint so re-reading a log
//      that still contains entries MAVIS already reported cannot produce
//      a duplicate.
//
// The change history is append-only, like safety/audit.db: there is no
// method here that deletes or rewrites a recorded change. The one column
// that does get updated is `announced`, which is bookkeeping about
// whether MAVIS has told the user yet — not a claim about what happened.

use super::change::{Change, ChangeKind, Severity};
use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use std::path::Path;

pub struct SentinelStore {
    conn: Connection,
}

impl SentinelStore {
    pub fn new(db_path: &Path) -> Result<Self> {
        let conn = Connection::open(db_path)?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS changes (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                fingerprint TEXT NOT NULL UNIQUE,
                kind        TEXT NOT NULL,
                severity    TEXT NOT NULL,
                source      TEXT NOT NULL,
                occurred_at TEXT NOT NULL,
                detail      TEXT NOT NULL,
                announced   INTEGER NOT NULL DEFAULT 0,
                recorded_at TEXT NOT NULL
            )",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_changes_occurred ON changes(occurred_at)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_changes_announced ON changes(announced)",
            [],
        )?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS watermarks (
                source    TEXT PRIMARY KEY,
                last_seen TEXT NOT NULL
            )",
            [],
        )?;

        Ok(Self { conn })
    }

    /// The newest entry already processed for a source, if any.
    /// `None` means this source has never been scanned — the caller
    /// should set the watermark and stay quiet rather than announce
    /// everything that ever happened.
    pub fn watermark(&self, source: &str) -> Result<Option<DateTime<Utc>>> {
        let mut stmt = self
            .conn
            .prepare("SELECT last_seen FROM watermarks WHERE source = ?1")?;
        let mut rows = stmt.query(params![source])?;
        match rows.next()? {
            Some(row) => {
                let raw: String = row.get(0)?;
                Ok(DateTime::parse_from_rfc3339(&raw)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc)))
            }
            None => Ok(None),
        }
    }

    pub fn set_watermark(&self, source: &str, at: DateTime<Utc>) -> Result<()> {
        self.conn.execute(
            "INSERT INTO watermarks (source, last_seen) VALUES (?1, ?2)
             ON CONFLICT(source) DO UPDATE SET last_seen = excluded.last_seen",
            params![source, at.to_rfc3339()],
        )?;
        Ok(())
    }

    /// Record a change. Returns false if this exact change was already
    /// recorded, which is the normal case when a log is re-read.
    pub fn record(&self, change: &Change) -> Result<bool> {
        let kind = serde_json::to_string(&change.kind)?;
        let affected = self.conn.execute(
            "INSERT OR IGNORE INTO changes
                (fingerprint, kind, severity, source, occurred_at, detail, announced, recorded_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)",
            params![
                change.fingerprint(),
                kind,
                change.severity.as_str(),
                change.source,
                change.occurred_at.to_rfc3339(),
                change.detail,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(affected > 0)
    }

    /// Record several changes, returning only the ones that were new.
    pub fn record_all(&self, changes: &[Change]) -> Result<Vec<Change>> {
        let mut fresh = Vec::new();
        for change in changes {
            if self.record(change)? {
                fresh.push(change.clone());
            }
        }
        Ok(fresh)
    }

    /// Changes at or above `min_severity` that the user has not been told
    /// about yet, oldest first.
    pub fn unannounced(&self, min_severity: Severity) -> Result<Vec<Change>> {
        let mut stmt = self.conn.prepare(
            "SELECT kind, severity, source, occurred_at, detail
             FROM changes WHERE announced = 0
             ORDER BY occurred_at ASC",
        )?;
        let rows = stmt.query_map([], row_to_change)?;
        Ok(rows
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .filter(|c| c.severity >= min_severity)
            .collect())
    }

    /// Mark changes as told-to-the-user. Takes fingerprints so the caller
    /// can announce a subset without racing a concurrent scan.
    pub fn mark_announced(&self, fingerprints: &[String]) -> Result<()> {
        for fp in fingerprints {
            self.conn.execute(
                "UPDATE changes SET announced = 1 WHERE fingerprint = ?1",
                params![fp],
            )?;
        }
        Ok(())
    }

    /// Everything observed in a time range, oldest first. Answers
    /// "what changed yesterday?" and "what did that update do?".
    pub fn between(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<Change>> {
        let mut stmt = self.conn.prepare(
            "SELECT kind, severity, source, occurred_at, detail
             FROM changes
             WHERE occurred_at >= ?1 AND occurred_at <= ?2
             ORDER BY occurred_at ASC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            params![start.to_rfc3339(), end.to_rfc3339(), limit as i64],
            row_to_change,
        )?;
        Ok(rows
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect())
    }

    /// The most recent changes regardless of time, newest first.
    pub fn recent(&self, limit: usize) -> Result<Vec<Change>> {
        let mut stmt = self.conn.prepare(
            "SELECT kind, severity, source, occurred_at, detail
             FROM changes ORDER BY occurred_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], row_to_change)?;
        Ok(rows
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect())
    }

    pub fn count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM changes", [], |r| r.get(0))?)
    }
}

/// Rebuild a Change from a row. Returns Ok(None) for a row that cannot be
/// decoded — a schema written by a newer MAVIS, say — so one unreadable
/// row doesn't fail the whole query.
fn row_to_change(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<Change>> {
    let kind_json: String = row.get(0)?;
    let severity: String = row.get(1)?;
    let source: String = row.get(2)?;
    let occurred_at: String = row.get(3)?;
    let detail: String = row.get(4)?;

    let Ok(kind) = serde_json::from_str::<ChangeKind>(&kind_json) else {
        return Ok(None);
    };
    let Some(severity) = Severity::from_str(&severity) else {
        return Ok(None);
    };
    let Ok(occurred_at) = DateTime::parse_from_rfc3339(&occurred_at) else {
        return Ok(None);
    };

    Ok(Some(Change {
        kind,
        severity,
        source,
        occurred_at: occurred_at.with_timezone(&Utc),
        detail,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> SentinelStore {
        // A file-backed store in a unique temp path: ":memory:" would not
        // exercise the same path the real one takes.
        let path = std::env::temp_dir().join(format!(
            "mavis_sentinel_test_{}_{}.db",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        SentinelStore::new(&path).expect("store")
    }

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    fn pulled_in(name: &str, secs: i64) -> Change {
        Change::new(
            ChangeKind::PackageInstalled {
                name: name.into(),
                version: "1.0".into(),
                requested: false,
            },
            "pacman",
            at(secs),
        )
    }

    fn upgraded(name: &str, secs: i64) -> Change {
        Change::new(
            ChangeKind::PackageUpgraded {
                name: name.into(),
                from: "1.0".into(),
                to: "2.0".into(),
            },
            "pacman",
            at(secs),
        )
    }

    #[test]
    fn a_fresh_store_has_no_watermark() {
        let s = store();
        assert_eq!(s.watermark("pacman").unwrap(), None);
    }

    #[test]
    fn watermarks_round_trip_and_overwrite() {
        let s = store();
        s.set_watermark("pacman", at(0)).unwrap();
        assert_eq!(s.watermark("pacman").unwrap(), Some(at(0)));
        s.set_watermark("pacman", at(100)).unwrap();
        assert_eq!(s.watermark("pacman").unwrap(), Some(at(100)));
        // Sources are independent.
        assert_eq!(s.watermark("dpkg").unwrap(), None);
    }

    #[test]
    fn a_change_round_trips_intact() {
        let s = store();
        let c = pulled_in("hyprland", 0);
        assert!(s.record(&c).unwrap());

        let back = s.recent(10).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].kind, c.kind);
        assert_eq!(back[0].severity, c.severity);
        assert_eq!(back[0].detail, c.detail);
        assert_eq!(back[0].occurred_at, c.occurred_at);
    }

    /// Re-reading a log that still holds already-reported entries must not
    /// announce them twice.
    #[test]
    fn recording_the_same_change_twice_is_ignored() {
        let s = store();
        let c = pulled_in("hyprland", 0);
        assert!(s.record(&c).unwrap(), "first insert is new");
        assert!(!s.record(&c).unwrap(), "second insert is a duplicate");
        assert_eq!(s.count().unwrap(), 1);
    }

    #[test]
    fn record_all_returns_only_the_new_ones() {
        let s = store();
        let a = pulled_in("hyprland", 0);
        let b = pulled_in("numactl", 1);

        let fresh = s.record_all(&[a.clone()]).unwrap();
        assert_eq!(fresh.len(), 1);

        let fresh = s.record_all(&[a, b]).unwrap();
        assert_eq!(fresh.len(), 1, "only the unseen one comes back");
        assert_eq!(s.count().unwrap(), 2);
    }

    #[test]
    fn unannounced_filters_by_severity_and_orders_oldest_first() {
        let s = store();
        s.record(&upgraded("firefox", 10)).unwrap(); // Routine
        s.record(&pulled_in("hyprland", 0)).unwrap(); // Notable
        s.record(&pulled_in("numactl", 20)).unwrap(); // Notable

        let notable = s.unannounced(Severity::Notable).unwrap();
        assert_eq!(notable.len(), 2);
        assert!(notable[0].detail.contains("hyprland"), "oldest first");
        assert!(notable[1].detail.contains("numactl"));

        let everything = s.unannounced(Severity::Routine).unwrap();
        assert_eq!(everything.len(), 3);
    }

    #[test]
    fn announcing_removes_from_the_pending_list() {
        let s = store();
        let c = pulled_in("hyprland", 0);
        s.record(&c).unwrap();
        assert_eq!(s.unannounced(Severity::Notable).unwrap().len(), 1);

        s.mark_announced(&[c.fingerprint()]).unwrap();
        assert!(s.unannounced(Severity::Notable).unwrap().is_empty());

        // ...but the change itself is still on record.
        assert_eq!(s.count().unwrap(), 1);
        assert_eq!(s.recent(10).unwrap().len(), 1);
    }

    #[test]
    fn marking_an_unknown_fingerprint_is_harmless() {
        let s = store();
        s.mark_announced(&["nothing-like-this".to_string()]).unwrap();
    }

    #[test]
    fn between_filters_by_time() {
        let s = store();
        s.record(&pulled_in("a", 0)).unwrap();
        s.record(&pulled_in("b", 100)).unwrap();
        s.record(&pulled_in("c", 200)).unwrap();

        let mid = s.between(at(50), at(150), 100).unwrap();
        assert_eq!(mid.len(), 1);
        assert!(mid[0].detail.contains("b"));

        let all = s.between(at(-1), at(1000), 100).unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn recent_is_newest_first_and_respects_the_limit() {
        let s = store();
        s.record(&pulled_in("a", 0)).unwrap();
        s.record(&pulled_in("b", 100)).unwrap();
        s.record(&pulled_in("c", 200)).unwrap();

        let recent = s.recent(2).unwrap();
        assert_eq!(recent.len(), 2);
        assert!(recent[0].detail.contains("c"), "newest first");
    }

    #[test]
    fn a_reopened_store_keeps_everything() {
        let path = std::env::temp_dir().join(format!(
            "mavis_sentinel_reopen_{}_{}.db",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        {
            let s = SentinelStore::new(&path).unwrap();
            s.record(&pulled_in("hyprland", 0)).unwrap();
            s.set_watermark("pacman", at(500)).unwrap();
        }
        let s = SentinelStore::new(&path).unwrap();
        assert_eq!(s.count().unwrap(), 1);
        assert_eq!(s.watermark("pacman").unwrap(), Some(at(500)));
        let _ = std::fs::remove_file(&path);
    }
}
