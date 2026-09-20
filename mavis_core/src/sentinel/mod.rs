// mavis_core/src/sentinel/mod.rs
// Phase 8.5 — System Sentinel.
//
// Phase 8's permission gate audits what MAVIS does. This audits what
// happened to the machine: what the package manager installed, removed
// or replaced, and (in later steps) what changed about who can do what.
//
// The problem it solves is one of awareness, not of detection. A system
// update pulls in a dependency nobody asked for, and it sits there
// unnoticed for days because nothing ever mentions it. MAVIS reads the
// package manager's own transaction log and says so.
//
// Two rules this module holds to:
//
//   1. Severity is static (see change.rs). The LLM may phrase a change
//      more naturally, or raise a severity as a second opinion. It may
//      never lower one, and it never decides severity in the first place.
//
//   2. MAVIS reports facts and leaves the verdict to the user. It does
//      not classify anything as malware. Where a real scanner exists
//      (Defender, ClamAV, XProtect), its findings are reported as that
//      tool's findings. An assistant that implies an all-clear it cannot
//      back up is worse than one that stays quiet.
//
// Off by default, like every other context source, via MAVIS_SENTINEL=1.

pub mod change;
pub mod packages;
pub mod store;
pub mod summary;

use crate::event_bus::EventBus;
use crate::models::event::{Event, EventType};
use change::{Change, Severity};
use log::{info, warn};
use packages::PackageManager;
use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;
use store::SentinelStore;
use tokio::time::{interval, Duration};

/// How often to look. Package changes are rare and never urgent to the
/// second, and the check is only a stat() unless the log actually moved.
const SCAN_INTERVAL: Duration = Duration::from_secs(60);

/// Wait before the first scan so startup isn't competing with model
/// loading and the first transcription.
const STARTUP_DELAY: Duration = Duration::from_secs(20);

/// Whether the sentinel is switched on. Same opt-in shape as the
/// MAVIS_CONTEXT_* sources: reading the machine's package history is
/// still reading something about the user, so it is their call.
pub fn enabled() -> bool {
    matches!(
        std::env::var("MAVIS_SENTINEL").as_deref(),
        Ok("1") | Ok("true")
    )
}

/// Which log entries a scan should consider, given the last watermark.
///
/// Extracted so the boundary rule is testable at the point `scan` uses
/// it, rather than only where it happens to be implemented.
///
/// At or after the watermark — NOT strictly after. A large update writes
/// its log lines over several minutes, and the mtime check makes it easy
/// for a scan to land in the middle of one. If that scan sets the
/// watermark to 12:00:05 and the package manager then writes more lines
/// stamped 12:00:05, a strictly-after filter drops them permanently:
/// they are discarded before they are ever fingerprinted, so neither the
/// store nor the announcement path ever sees them.
///
/// Re-examining the boundary second costs nothing, because the
/// fingerprint does the deduplication — `SentinelStore::record` returns
/// false for anything already stored, so nothing is announced twice.
fn fresh_since(
    entries: &[packages::LogEntry],
    watermark: Option<chrono::DateTime<chrono::Utc>>,
) -> Vec<packages::LogEntry> {
    match watermark {
        Some(mark) => packages::entries_since(entries, mark),
        None => entries.to_vec(),
    }
}

pub struct Sentinel {
    bus: Arc<EventBus>,
    store: SentinelStore,
    manager: PackageManager,
    /// Modification time of the log at the last scan. The log is re-read
    /// only when this changes, so the steady-state cost of running the
    /// sentinel is one stat() per minute.
    last_mtime: Option<SystemTime>,
}

impl Sentinel {
    pub fn new(bus: Arc<EventBus>, data_dir: &Path) -> anyhow::Result<Self> {
        let store = SentinelStore::new(&data_dir.join("sentinel.db"))?;
        let manager = PackageManager::detect();
        Ok(Self {
            bus,
            store,
            manager,
            last_mtime: None,
        })
    }

    pub async fn run(&mut self) {
        if !enabled() {
            info!("Sentinel: disabled (set MAVIS_SENTINEL=1 to switch it on)");
            return;
        }
        if self.manager == PackageManager::Unknown {
            warn!(
                "Sentinel: no supported package manager found — \
                 package monitoring is off for this machine"
            );
            return;
        }
        info!(
            "Sentinel: watching {} ({})",
            self.manager.log_path().unwrap_or("?"),
            self.manager.source_name()
        );

        tokio::time::sleep(STARTUP_DELAY).await;
        let mut ticker = interval(SCAN_INTERVAL);

        loop {
            ticker.tick().await;
            if !self.bus.is_open() {
                info!("Sentinel: bus closed, shutting down");
                return;
            }
            match self.scan() {
                Ok(changes) if !changes.is_empty() => self.publish(changes),
                Ok(_) => {}
                Err(e) => warn!("Sentinel: scan failed: {}", e),
            }
        }
    }

    /// One pass over the package log. Returns only changes not seen before.
    ///
    /// The first run is deliberately silent. A real Arch log carries over
    /// two thousand transactions, and announcing a machine's entire
    /// history the first time MAVIS starts would be worse than saying
    /// nothing at all. Everything is still *recorded* — so "what changed
    /// last month?" works immediately — but it is marked as already
    /// announced, and only changes from then on are spoken.
    fn scan(&mut self) -> anyhow::Result<Vec<Change>> {
        let Some(path) = self.manager.log_path() else {
            return Ok(Vec::new());
        };
        let source = self.manager.source_name();

        // Cheap guard: if the log hasn't been touched, there is nothing
        // to do. Skipped on the first pass so a restart still catches up.
        let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        if self.last_mtime.is_some() && mtime == self.last_mtime {
            return Ok(Vec::new());
        }
        let first_run = self.store.watermark(source)?.is_none();

        let text = std::fs::read_to_string(path)?;
        let entries = self.manager.parse_log(&text);
        self.last_mtime = mtime;

        if entries.is_empty() {
            warn!(
                "Sentinel: {} had no parseable transactions — the log format \
                 may differ on this system",
                path
            );
            return Ok(Vec::new());
        }

        let newest = entries
            .iter()
            .map(|e| e.occurred_at)
            .max()
            .expect("entries is non-empty");

        let fresh_entries = fresh_since(&entries, self.store.watermark(source)?);

        let explicit = packages::explicitly_installed(self.manager);
        let changes = packages::to_changes(&fresh_entries, &explicit, source);
        let recorded = self.store.record_all(&changes)?;
        self.store.set_watermark(source, newest)?;

        if first_run {
            // Record, but treat as already told — this is history, not news.
            let prints: Vec<String> = recorded.iter().map(|c| c.fingerprint()).collect();
            self.store.mark_announced(&prints)?;
            info!(
                "Sentinel: first run — imported {} past transactions from {} \
                 without announcing them. Changes from now on will be reported.",
                recorded.len(),
                source
            );
            return Ok(Vec::new());
        }

        if !recorded.is_empty() {
            info!("Sentinel: {} new change(s) since last scan", recorded.len());
        }
        Ok(recorded)
    }

    /// Announce a batch of changes.
    ///
    /// Critical changes get a desktop notification immediately, because
    /// "someone can now do something new on this machine" should not wait
    /// for the user to happen to start a conversation. Notable changes
    /// are left in the store for the planner to mention next time the
    /// user speaks — MAVIS is never intrusive for something that is
    /// merely worth knowing.
    fn publish(&self, changes: Vec<Change>) {
        let peak = summary::peak_severity(&changes).unwrap_or(Severity::Routine);
        let lines = summary::summarize_all(&changes);

        self.bus.publish(Event {
            id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            source: "sentinel".to_string(),
            event_type: EventType::SystemChange,
            payload: serde_json::json!({
                "severity": peak.as_str(),
                "count": changes.len(),
                "summaries": lines,
                "changes": changes,
            }),
        });

        if peak >= Severity::Critical {
            for line in &lines {
                // Goes through the permission gate like anything else.
                // A notify action scores 0, so it is approved without
                // asking — but it is still recorded in the audit log.
                self.bus.publish(Event {
                    id: uuid::Uuid::new_v4(),
                    timestamp: chrono::Utc::now(),
                    source: "sentinel".to_string(),
                    event_type: EventType::PlanReady,
                    payload: serde_json::json!({
                        "plan": {
                            "type": "notify",
                            "title": "MAVIS — system change",
                            "message": line,
                        }
                    }),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mavis_sentinel_{}_{}_{}",
            tag,
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn enabled_follows_the_env_var() {
        // Not asserting on the live value — just that it does not panic
        // and returns a bool for whatever is set.
        let _ = enabled();
    }

    #[test]
    fn a_sentinel_can_be_built_against_a_fresh_directory() {
        let dir = temp_dir("build");
        let bus = Arc::new(EventBus::new());
        let s = Sentinel::new(bus, &dir).expect("sentinel");
        // Detection depends on the host; both outcomes are valid.
        assert!(s.store.count().unwrap() == 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The boundary case the `>=` filter exists for.
    ///
    /// A long `-Syu` writes its log lines over minutes. A scan that lands
    /// mid-transaction sets the watermark to that second; pacman then
    /// writes more lines stamped with the SAME second. Under a
    /// strictly-after filter those lines were dropped before they were
    /// ever fingerprinted, so they were lost for good.
    ///
    /// This walks the scan pipeline directly (no file I/O) to prove that
    /// the boundary second is re-examined, the already-recorded entry is
    /// not announced twice, and the late arrival IS caught.
    #[test]
    fn entries_written_in_the_watermark_second_are_not_lost() {
        use packages::{Action, LogEntry};
        use std::collections::HashSet;

        let dir = temp_dir("boundary");
        let store = SentinelStore::new(&dir.join("sentinel.db")).unwrap();
        let source = "pacman";
        let explicit: HashSet<String> = HashSet::from(["firefox".to_string()]);

        let boundary = chrono::DateTime::from_timestamp(1_700_000_100, 0).unwrap();
        let earlier = chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap();

        let entry = |name: &str, at: chrono::DateTime<chrono::Utc>| LogEntry {
            action: Action::Installed,
            name: name.to_string(),
            old_version: String::new(),
            new_version: "1.0".to_string(),
            occurred_at: at,
        };

        // --- Scan 1: lands mid-transaction, sees only part of it. ---
        let seen = vec![entry("firefox", earlier), entry("hyprland", boundary)];
        let newest = seen.iter().map(|e| e.occurred_at).max().unwrap();
        let changes = packages::to_changes(&seen, &explicit, source);
        let recorded = store.record_all(&changes).unwrap();
        store.set_watermark(source, newest).unwrap();
        assert_eq!(recorded.len(), 2, "first scan records what it saw");

        // --- pacman writes one more line, stamped the SAME second. ---
        let all = vec![
            entry("firefox", earlier),
            entry("hyprland", boundary),
            entry("xdg-desktop-portal-hyprland", boundary),
        ];

        // --- Scan 2 ---
        let mark = store.watermark(source).unwrap().unwrap();
        assert_eq!(mark, boundary);
        // Drives the same helper `scan` uses, so reverting it to a
        // strictly-after filter fails this test.
        let fresh = fresh_since(&all, Some(mark));
        assert_eq!(
            fresh.len(),
            2,
            "the boundary second must be re-examined, not skipped"
        );

        let changes = packages::to_changes(&fresh, &explicit, source);
        let recorded = store.record_all(&changes).unwrap();

        assert_eq!(
            recorded.len(),
            1,
            "only the late arrival is new; the fingerprint suppresses the replay"
        );
        assert!(
            recorded[0].detail.contains("xdg-desktop-portal-hyprland"),
            "got: {}",
            recorded[0].detail
        );
        assert_eq!(store.count().unwrap(), 3, "nothing duplicated in the store");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An unknown package manager must be inert rather than noisy.
    #[test]
    fn an_unknown_package_manager_scans_to_nothing() {
        let dir = temp_dir("unknown");
        let bus = Arc::new(EventBus::new());
        let mut s = Sentinel::new(bus, &dir).expect("sentinel");
        s.manager = PackageManager::Unknown;
        assert!(s.scan().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}