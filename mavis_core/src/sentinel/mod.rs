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

pub mod advisories;
pub mod change;
pub mod checks;
pub mod integrity;
pub mod inventory;
pub mod packages;
mod phrases;
#[cfg(unix)]
pub mod privilege;
pub mod speech;
pub mod store;
pub mod summary;

use crate::event_bus::EventBus;
use crate::models::event::{Event, EventType};
use change::{Change, ChangeKind, Severity};
use checks::{Check, Snapshot};
use chrono::{DateTime, Utc};
use log::{info, warn};
use packages::PackageManager;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime};
use store::SentinelStore;
use tokio::time::{interval, Duration};

/// How often to look. Package changes are rare and never urgent to the
/// second, and the check is only a stat() unless the log actually moved.
const SCAN_INTERVAL: Duration = Duration::from_secs(60);

/// Wait before the first scan so startup isn't competing with model
/// loading and the first transcription.
const STARTUP_DELAY: Duration = Duration::from_secs(20);

/// The setuid walk reads ~120k files (0.6–2.6 s), so it runs hourly and
/// whenever the package log moves, not every minute.
#[cfg(unix)]
const SETUID_INTERVAL: Duration = Duration::from_secs(3600);

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

/// Store a scan's changes, returning the new ones. The first run goes in
/// already marked announced, in one step, so a concurrent reader never
/// sees history as news.
fn import(store: &SentinelStore, changes: &[Change], first_run: bool) -> anyhow::Result<Vec<Change>> {
    if first_run {
        store.record_all_announced(changes)
    } else {
        store.record_all(changes)
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
    #[cfg(unix)]
    surfaces: privilege::Surfaces,
    #[cfg(unix)]
    last_setuid_scan: Option<std::time::Instant>,
    /// The slow, external-tool checks for this machine (steps 4 and 5).
    checks: Vec<Check>,
    /// When each check was last tried in this process, and whether it worked.
    attempts: HashMap<&'static str, (Instant, bool)>,
    /// Present while the package manager is mid-transaction.
    package_lock: Option<PathBuf>,
    package_log: Option<PathBuf>,
    /// So a held lock is logged once, not every minute.
    lock_noted: bool,
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
            #[cfg(unix)]
            surfaces: privilege::Surfaces::system(),
            #[cfg(unix)]
            last_setuid_scan: None,
            checks: Vec::new(),
            attempts: HashMap::new(),
            package_lock: (manager == PackageManager::Pacman).then(|| "/var/lib/pacman/db.lck".into()),
            package_log: manager.log_path().map(PathBuf::from),
            lock_noted: false,
        })
    }

    pub async fn run(&mut self) {
        if !enabled() {
            info!("Sentinel: disabled (set MAVIS_SENTINEL=1 to switch it on)");
            return;
        }
        let watch_packages = self.manager != PackageManager::Unknown;
        if watch_packages {
            info!(
                "Sentinel: watching {} ({})",
                self.manager.log_path().unwrap_or("?"),
                self.manager.source_name()
            );
        } else {
            warn!("Sentinel: no supported package manager found — package history is not watched");
        }

        self.checks = checks::for_this_machine(self.manager);
        if !checks::advisories_enabled() {
            info!("Sentinel: security advisories are off (set MAVIS_SENTINEL_ADVISORIES=1 — it downloads the advisory list)");
        }

        tokio::time::sleep(STARTUP_DELAY).await;
        let mut ticker = interval(SCAN_INTERVAL);

        loop {
            ticker.tick().await;
            if !self.bus.is_open() {
                info!("Sentinel: bus closed, shutting down");
                return;
            }
            let mut changes = Vec::new();
            let mut log_moved = false;
            if watch_packages {
                let before = self.last_mtime;
                match self.scan() {
                    Ok(found) => changes.extend(found),
                    Err(e) => warn!("Sentinel: package scan failed: {}", e),
                }
                log_moved = self.last_mtime != before;
            }
            #[cfg(unix)]
            match self.scan_privileges(log_moved).await {
                Ok(found) => changes.extend(found),
                Err(e) => warn!("Sentinel: privilege scan failed: {}", e),
            }
            #[cfg(not(unix))]
            let _ = log_moved;
            if !changes.is_empty() {
                self.publish(changes);
            }

            // Slow checks last, so a long verify never delays the news above.
            let found = self.run_checks().await;
            if !found.is_empty() {
                self.publish(found);
            }
        }
    }

    /// Run whichever slow checks are due, one after another.
    async fn run_checks(&mut self) -> Vec<Change> {
        let mut found = Vec::new();
        for i in 0..self.checks.len() {
            let source = self.checks[i].source;
            let needs_idle = self.checks[i].needs_idle_packages;
            match self.is_due(&self.checks[i]) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(e) => {
                    warn!("Sentinel: {} check skipped: {}", source, e);
                    continue;
                }
            }
            // A verify that overlaps a package transaction sees half-installed
            // files as mismatches. Skip while one runs; discard if one started.
            if needs_idle && self.packages_busy() {
                if !self.lock_noted {
                    info!("Sentinel: the package manager is busy — the {} check will wait for it", source);
                    self.lock_noted = true;
                }
                continue;
            }
            self.lock_noted = false;
            let log_before = self.package_log_mtime();

            info!("Sentinel: running the {} check", source);
            let snapshot = self.checks[i].snapshot().await;

            if needs_idle && (self.packages_busy() || self.package_log_mtime() != log_before) {
                info!("Sentinel: packages changed during the {} check — result discarded, will retry", source);
                continue;
            }
            self.attempts.insert(source, (Instant::now(), snapshot.is_some()));
            let Some(snapshot) = snapshot else { continue };

            let check = &self.checks[i];
            let applied = self
                .apply(source, &snapshot, |old, new, at| self.mark_returning(check.diff(old, new, at)))
                .and_then(|changes| {
                    self.store.set_watermark(source, Utc::now())?;
                    Ok(changes)
                });
            match applied {
                Ok(changes) => found.extend(changes),
                Err(e) => warn!("Sentinel: could not record the {} check: {}", source, e),
            }
        }
        found
    }

    /// Due if it has never run, or last ran longer ago than its interval.
    /// The store remembers the last run, so a restart doesn't repeat a
    /// full disk verify.
    fn is_due(&self, check: &Check) -> anyhow::Result<bool> {
        if let Some((at, ok)) = self.attempts.get(check.source) {
            let wait = if *ok { check.every } else { checks::RETRY };
            return Ok(at.elapsed() >= wait);
        }
        if checks::check_now() {
            return Ok(true);
        }
        Ok(match self.store.watermark(check.source)? {
            None => true,
            Some(last) => (Utc::now() - last).to_std().map_or(true, |age| age >= check.every),
        })
    }

    fn packages_busy(&self) -> bool {
        self.package_lock.as_ref().is_some_and(|lock| lock.exists())
    }

    fn package_log_mtime(&self) -> Option<SystemTime> {
        let log = self.package_log.as_ref()?;
        std::fs::metadata(log).and_then(|m| m.modified()).ok()
    }

    /// An app that was removed earlier and has appeared again is marked
    /// as returning — the case of an update quietly restoring it.
    fn mark_returning(&self, changes: Vec<Change>) -> Vec<Change> {
        changes
            .into_iter()
            .map(|c| match &c.kind {
                ChangeKind::AppAdded { name, version, .. }
                    if self.store.has_recorded(&c.source, change::APP_REMOVED, name).unwrap_or(false) =>
                {
                    let kind = ChangeKind::AppAdded {
                        name: name.clone(),
                        version: version.clone(),
                        returned: true,
                    };
                    Change::new(kind, &c.source, c.occurred_at)
                }
                _ => c,
            })
            .collect()
    }

    /// Store a source's new snapshot and return what changed. The first
    /// snapshot of a source is a silent baseline, like the package history.
    fn apply(
        &self,
        source: &str,
        snapshot: &Snapshot,
        diff: impl Fn(&Snapshot, &Snapshot, DateTime<Utc>) -> Vec<Change>,
    ) -> anyhow::Result<Vec<Change>> {
        let now = Utc::now();
        let first_run = self.store.watermark(source)?.is_none();
        let previous = self.store.snapshot(source)?;
        if !first_run && previous == *snapshot {
            return Ok(Vec::new());
        }
        let changes = if first_run {
            info!("Sentinel: baseline for {} ({} items)", source, snapshot.len());
            Vec::new()
        } else {
            diff(&previous, snapshot, now)
        };
        self.store.apply_snapshot(source, snapshot, &changes, now)
    }

    /// Diff each privilege surface against its stored snapshot. The first
    /// read of a surface is a silent baseline, like the package history.
    #[cfg(unix)]
    async fn scan_privileges(&mut self, package_log_moved: bool) -> anyhow::Result<Vec<Change>> {
        let setuid_due = package_log_moved
            || self
                .last_setuid_scan
                .is_none_or(|t| t.elapsed() >= SETUID_INTERVAL);
        let surfaces = self.surfaces.clone();
        let snapshots =
            tokio::task::spawn_blocking(move || privilege::collect_all(&surfaces, setuid_due)).await?;
        if setuid_due {
            self.last_setuid_scan = Some(std::time::Instant::now());
        }

        let mut found = Vec::new();
        for (source, snapshot) in snapshots {
            found.extend(self.apply(source, &snapshot, |old, new, at| privilege::diff(source, old, new, at))?);
        }
        if !found.is_empty() {
            info!("Sentinel: {} privilege change(s)", found.len());
        }
        Ok(found)
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
        let recorded = import(&self.store, &changes, first_run)?;
        self.store.set_watermark(source, newest)?;

        if first_run {
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

    /// A second connection (the planner) polls while the first run
    /// imports; it must never see a pending change.
    #[test]
    fn a_first_run_import_is_never_visible_as_pending() {
        use change::ChangeKind;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

        let dir = temp_dir("import_race");
        let db = dir.join("sentinel.db");
        drop(SentinelStore::new(&db).unwrap());

        let history: Vec<Change> = (0..3000)
            .map(|i| {
                Change::new(
                    ChangeKind::PackageInstalled {
                        name: format!("dep-{}", i),
                        version: "1.0".into(),
                        requested: false, // Notable — the kind that gets announced
                    },
                    "pacman",
                    chrono::DateTime::from_timestamp(1_700_000_000 + i, 0).unwrap(),
                )
            })
            .collect();

        let done = Arc::new(AtomicBool::new(false));
        let worst_seen = Arc::new(AtomicUsize::new(0));

        let reader = {
            let (db, done, worst_seen) = (db.clone(), done.clone(), worst_seen.clone());
            std::thread::spawn(move || {
                let planner_side = SentinelStore::new(&db).unwrap();
                while !done.load(Ordering::SeqCst) {
                    // Busy means mid-commit; just retry.
                    if let Ok(pending) = planner_side.unannounced(Severity::Notable) {
                        worst_seen.fetch_max(pending.len(), Ordering::SeqCst);
                    }
                }
            })
        };

        let sentinel_side = SentinelStore::new(&db).unwrap();
        let imported = import(&sentinel_side, &history, true).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        done.store(true, Ordering::SeqCst);
        reader.join().unwrap();

        assert_eq!(imported.len(), 3000);
        assert_eq!(
            worst_seen.load(Ordering::SeqCst),
            0,
            "a concurrent reader saw first-run history as pending news"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn later_scans_leave_their_changes_pending() {
        use change::ChangeKind;
        let dir = temp_dir("later_scan");
        let store = SentinelStore::new(&dir.join("sentinel.db")).unwrap();
        let change = Change::new(
            ChangeKind::PackageInstalled {
                name: "hyprland".into(),
                version: "1.0".into(),
                requested: false,
            },
            "pacman",
            chrono::Utc::now(),
        );
        let recorded = import(&store, &[change], false).unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(store.unannounced(Severity::Notable).unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Surfaces in a temp dir: one account with wheel membership.
    #[cfg(unix)]
    fn temp_surfaces(dir: &std::path::Path) -> privilege::Surfaces {
        std::fs::write(dir.join("passwd"), "manav:x:1000:1000::/home/manav:/bin/zsh\n").unwrap();
        std::fs::write(dir.join("group"), "wheel:x:998:manav\n").unwrap();
        privilege::Surfaces {
            passwd: dir.join("passwd"),
            group: dir.join("group"),
            login_defs: dir.join("login.defs"),
            authorized_keys: vec![dir.join("authorized_keys")],
            sudoers: vec![],
            systemd: vec![],
            setuid_roots: vec![],
        }
    }

    /// Baseline silently, then a new wheel member is Critical and raises
    /// a desktop notification through the permission gate.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_new_wheel_member_is_noticed_and_notified() {
        let dir = temp_dir("privilege");
        let bus = Arc::new(EventBus::new());
        let mut rx = bus.subscribe();
        let mut s = Sentinel::new(Arc::clone(&bus), &dir).expect("sentinel");
        s.surfaces = temp_surfaces(&dir);

        assert!(s.scan_privileges(false).await.unwrap().is_empty(), "first read is a baseline");
        assert!(s.scan_privileges(false).await.unwrap().is_empty(), "nothing changed");

        std::fs::write(dir.join("group"), "wheel:x:998:manav,eve\n").unwrap();
        let found = s.scan_privileges(false).await.unwrap();
        assert_eq!(found.len(), 1, "{:?}", found);
        assert_eq!(found[0].severity, Severity::Critical);

        s.publish(found);
        let mut notified = false;
        while let Ok(e) = rx.try_recv() {
            if e.event_type == EventType::PlanReady && e.payload["plan"]["type"] == "notify" {
                let msg = e.payload["plan"]["message"].as_str().unwrap_or("");
                assert!(msg.contains("eve was added to the wheel group"), "{}", msg);
                notified = true;
            }
        }
        assert!(notified, "a Critical change must notify immediately");
        assert_eq!(s.store.unannounced(Severity::Notable).unwrap().len(), 1, "and is still spoken later");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A surface that vanishes for one scan must not come back as a flood.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_unreadable_surface_does_not_cause_a_flood() {
        let dir = temp_dir("flood");
        let bus = Arc::new(EventBus::new());
        let mut s = Sentinel::new(bus, &dir).expect("sentinel");
        s.surfaces = temp_surfaces(&dir);
        s.scan_privileges(false).await.unwrap();

        std::fs::rename(dir.join("group"), dir.join("group.away")).unwrap();
        assert!(s.scan_privileges(false).await.unwrap().is_empty());
        std::fs::rename(dir.join("group.away"), dir.join("group")).unwrap();
        assert!(s.scan_privileges(false).await.unwrap().is_empty(), "same members as before: no change");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---------------------------------------------------------------
    // Steps 4 and 5 — slow checks
    // ---------------------------------------------------------------

    use std::sync::Mutex;

    fn snap(items: &[(&str, &str)]) -> Snapshot {
        items.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    /// A Sentinel whose only check returns whatever `next` currently holds.
    fn sentinel_with_check(tag: &str, needs_idle: bool) -> (Sentinel, Arc<Mutex<Option<Snapshot>>>, std::path::PathBuf) {
        let dir = temp_dir(tag);
        let mut s = Sentinel::new(Arc::new(EventBus::new()), &dir).expect("sentinel");
        let next = Arc::new(Mutex::new(Some(Snapshot::new())));
        let source = Arc::clone(&next);
        s.checks = vec![checks::fake_integrity(needs_idle, move || source.lock().unwrap().clone())];
        s.package_lock = Some(dir.join("db.lck"));
        s.package_log = None;
        (s, next, dir)
    }

    const CHANGED_BINARY: (&str, &str) = ("/usr/bin/sudo", "content\t0\tsudo");

    /// Baseline silently; a changed binary is then Critical and notified;
    /// the same finding next time is not news.
    #[tokio::test]
    async fn a_changed_packaged_binary_is_critical_once() {
        let (mut s, next, dir) = sentinel_with_check("integrity", true);
        let edited_config = ("/etc/pacman.conf", "content\t1\tpacman");
        *next.lock().unwrap() = Some(snap(&[edited_config]));
        assert!(s.run_checks().await.is_empty(), "what was already there is the baseline");

        *next.lock().unwrap() = Some(snap(&[edited_config, CHANGED_BINARY]));
        let found = s.run_checks().await;
        assert_eq!(found.len(), 1, "{:?}", found);
        assert_eq!(found[0].severity, Severity::Critical);

        let mut rx = s.bus.subscribe();
        s.publish(found);
        let notified = std::iter::from_fn(|| rx.try_recv().ok()).any(|e| {
            e.event_type == EventType::PlanReady
                && e.payload["plan"]["message"]
                    .as_str()
                    .is_some_and(|m| m.contains("sudo, from the sudo package, no longer matches what was installed"))
        });
        assert!(notified);

        assert!(s.run_checks().await.is_empty(), "told once");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A check that fails must leave the last snapshot alone. Read as
    /// empty, the next good run would report every old finding as new.
    #[tokio::test]
    async fn a_failed_check_does_not_cause_a_flood() {
        let (mut s, next, dir) = sentinel_with_check("check_flood", false);
        *next.lock().unwrap() = Some(snap(&[CHANGED_BINARY]));
        s.run_checks().await;

        *next.lock().unwrap() = None;
        assert!(s.run_checks().await.is_empty());
        assert!(!s.is_due(&s.checks[0]).unwrap(), "a failure waits for the retry interval");

        s.attempts.clear();
        *next.lock().unwrap() = Some(snap(&[CHANGED_BINARY]));
        assert!(s.run_checks().await.is_empty(), "same finding as before the failure");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A verify that overlaps a package transaction sees half-installed
    /// files. Its result is thrown away and the check stays due.
    #[tokio::test]
    async fn a_check_that_overlaps_a_package_transaction_is_discarded() {
        let (mut s, _next, dir) = sentinel_with_check("overlap", true);
        s.run_checks().await; // clean baseline

        // The transaction starts while the check is running.
        let lock = dir.join("db.lck");
        let (during, result) = (lock.clone(), snap(&[CHANGED_BINARY]));
        s.checks = vec![checks::fake_integrity(true, move || {
            std::fs::write(&during, b"").unwrap();
            Some(result.clone())
        })];
        s.attempts.clear();
        assert!(s.run_checks().await.is_empty(), "mid-upgrade mismatches are not findings");
        assert!(s.attempts.is_empty(), "and the check is still due");

        // Still locked: the check doesn't even start.
        assert!(s.run_checks().await.is_empty());

        // Transaction over, and the file still differs: now it is a finding.
        std::fs::remove_file(&lock).unwrap();
        s.checks = vec![checks::fake_integrity(true, || Some(snap(&[CHANGED_BINARY])))];
        assert_eq!(s.run_checks().await.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The store remembers when a check last ran, so a restart doesn't
    /// repeat a daily check.
    #[tokio::test]
    async fn a_restart_does_not_repeat_a_recent_check() {
        let (mut s, _next, dir) = sentinel_with_check("cadence", false);
        s.run_checks().await;

        let mut restarted = Sentinel::new(Arc::new(EventBus::new()), &dir).expect("sentinel");
        let mut daily = checks::fake_integrity(false, || Some(Snapshot::new()));
        daily.every = std::time::Duration::from_secs(24 * 3600);
        assert!(!restarted.is_due(&daily).unwrap());
        restarted.checks = vec![daily];
        assert!(restarted.attempts.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Windows restoring an app the user removed: the second appearance
    /// is reported as a return, not as a new install.
    #[tokio::test]
    async fn an_app_that_comes_back_is_reported_as_returning() {
        let dir = temp_dir("returning");
        let mut s = Sentinel::new(Arc::new(EventBus::new()), &dir).expect("sentinel");
        let next = Arc::new(Mutex::new(snap(&[("Candy Crush", "1.0"), ("Notepad", "11")])));
        let source = Arc::clone(&next);
        s.checks = vec![checks::fake_apps(move || Some(source.lock().unwrap().clone()))];

        assert!(s.run_checks().await.is_empty(), "baseline");
        *next.lock().unwrap() = snap(&[("Notepad", "11")]);
        assert_eq!(s.run_checks().await.len(), 1, "removed");

        *next.lock().unwrap() = snap(&[("Candy Crush", "1.1"), ("Notepad", "11")]);
        let found = s.run_checks().await;
        assert!(matches!(&found[0].kind, ChangeKind::AppAdded { returned: true, .. }), "{:?}", found);
        let line = summary::summarize(&found).unwrap();
        assert_eq!(line, "Today, Candy Crush is back after being removed.");
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