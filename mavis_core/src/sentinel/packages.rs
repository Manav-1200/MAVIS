// mavis_core/src/sentinel/packages.rs
// Reading what the package manager actually did.
//
// Every mainstream package manager keeps a timestamped transaction log,
// world-readable, no root needed:
//
//   Arch            /var/log/pacman.log
//   Debian/Ubuntu   /var/log/dpkg.log
//   Fedora/RHEL     /var/log/dnf.rpm.log
//
// Reading the log beats diffing "installed packages now" against "installed
// packages last time" in two ways that matter. It carries the real
// timestamp, so MAVIS can say "your update on Thursday" rather than "at
// some point since I last looked". And it distinguishes a brand-new
// package from a version bump, which snapshot diffing of names alone
// cannot do at all.
//
// Parsing is kept pure — text in, entries out — so every distro's format
// is testable on any machine. Only `installed_packages` and the
// explicit/manual set need to actually shell out.

use super::change::{Change, ChangeKind};
use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Utc};
use std::collections::HashSet;

/// What a log line says happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Installed,
    Removed,
    Upgraded,
    Downgraded,
}

/// One parsed transaction-log line, before MAVIS knows whether the user
/// asked for the package.
#[derive(Clone, Debug, PartialEq)]
pub struct LogEntry {
    pub action: Action,
    pub name: String,
    /// Empty for installs.
    pub old_version: String,
    /// Empty for removals.
    pub new_version: String,
    pub occurred_at: DateTime<Utc>,
}

/// Which package manager this machine uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageManager {
    Pacman,
    Dpkg,
    Rpm,
    Unknown,
}

impl PackageManager {
    /// Detect by looking for the log file, then the binary. The log is
    /// checked first because a machine can have several package managers
    /// installed while only one is actually managing the system.
    pub fn detect() -> Self {
        use std::path::Path;
        if Path::new("/var/log/pacman.log").exists() {
            return PackageManager::Pacman;
        }
        if Path::new("/var/log/dpkg.log").exists() {
            return PackageManager::Dpkg;
        }
        if Path::new("/var/log/dnf.rpm.log").exists() || Path::new("/var/log/dnf5.log").exists() {
            return PackageManager::Rpm;
        }
        PackageManager::Unknown
    }

    pub fn log_path(self) -> Option<&'static str> {
        match self {
            PackageManager::Pacman => Some("/var/log/pacman.log"),
            PackageManager::Dpkg => Some("/var/log/dpkg.log"),
            PackageManager::Rpm => Some("/var/log/dnf.rpm.log"),
            PackageManager::Unknown => None,
        }
    }

    pub fn source_name(self) -> &'static str {
        match self {
            PackageManager::Pacman => "pacman",
            PackageManager::Dpkg => "dpkg",
            PackageManager::Rpm => "rpm",
            PackageManager::Unknown => "unknown",
        }
    }

    pub fn parse_log(self, text: &str) -> Vec<LogEntry> {
        match self {
            PackageManager::Pacman => parse_pacman_log(text),
            PackageManager::Dpkg => parse_dpkg_log(text),
            PackageManager::Rpm => parse_rpm_log(text),
            PackageManager::Unknown => Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------
// Arch — /var/log/pacman.log
// ---------------------------------------------------------------------
//
//   [2026-09-18T14:32:01+0545] [ALPM] installed hyprland (0.41.2-1)
//   [2026-09-18T14:32:01+0545] [ALPM] upgraded firefox (140.0-1 -> 141.0-1)
//   [2026-09-18T14:32:01+0545] [ALPM] removed foo (1.0-1)
//   [2026-09-18T14:32:01+0545] [ALPM] downgraded bar (2.0-1 -> 1.0-1)
//
// pacman changed its timestamp format in 2019 (from `[2019-01-01 12:00]`
// to ISO-8601 with an offset). Both are still found in logs that have
// never been rotated, so both are accepted.

pub fn parse_pacman_log(text: &str) -> Vec<LogEntry> {
    let mut out = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix('[') else {
            continue;
        };
        let Some((timestamp, rest)) = rest.split_once(']') else {
            continue;
        };
        let Some(occurred_at) = parse_pacman_timestamp(timestamp) else {
            continue;
        };

        // Only [ALPM] lines describe package transactions; [PACMAN] lines
        // record the command that was run, and [ALPM-SCRIPTLET] records
        // post-install script output.
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix("[ALPM]") else {
            continue;
        };
        let rest = rest.trim();

        let (action, rest) = if let Some(r) = rest.strip_prefix("installed ") {
            (Action::Installed, r)
        } else if let Some(r) = rest.strip_prefix("upgraded ") {
            (Action::Upgraded, r)
        } else if let Some(r) = rest.strip_prefix("downgraded ") {
            (Action::Downgraded, r)
        } else if let Some(r) = rest.strip_prefix("removed ") {
            (Action::Removed, r)
        } else {
            continue;
        };

        // "name (version)" or "name (old -> new)"
        let Some((name, versions)) = rest.split_once(" (") else {
            continue;
        };
        let versions = versions.trim_end_matches(')');

        let (old_version, new_version) = match versions.split_once(" -> ") {
            Some((old, new)) => (old.trim().to_string(), new.trim().to_string()),
            None => match action {
                Action::Removed => (versions.trim().to_string(), String::new()),
                _ => (String::new(), versions.trim().to_string()),
            },
        };

        out.push(LogEntry {
            action,
            name: name.trim().to_string(),
            old_version,
            new_version,
            occurred_at,
        });
    }

    out
}

fn parse_pacman_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    let raw = raw.trim();
    // Modern: 2026-09-18T14:32:01+0545
    if let Ok(dt) = DateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S%z") {
        return Some(dt.with_timezone(&Utc));
    }
    // Modern with a colon in the offset, which some tools rewrite it to.
    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        return Some(dt.with_timezone(&Utc));
    }
    // Pre-2019: 2019-01-01 12:00 (local time, no offset recorded)
    if let Ok(naive) = NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M") {
        return local_to_utc(naive);
    }
    None
}

// ---------------------------------------------------------------------
// Debian / Ubuntu — /var/log/dpkg.log
// ---------------------------------------------------------------------
//
//   2026-09-20 09:50:40 install libasound2-dev:amd64 <none> 1.2.11-1ubuntu0.3
//   2026-09-20 09:50:40 upgrade libasound2t64:amd64 1.2.11-1ubuntu0.2 1.2.11-1ubuntu0.3
//   2026-09-20 09:50:40 remove foo:amd64 1.0-1 <none>
//
// `<none>` as the old version is dpkg saying the package was not present
// before — a genuinely new package rather than an update. That single
// token is what makes the "you didn't ask for this" case detectable.
//
// `status`, `configure` and `trigproc` lines are progress noise and are
// skipped. `purge` is skipped because apt logs `remove` first for any
// package that was actually installed; counting both would report one
// removal twice.

pub fn parse_dpkg_log(text: &str) -> Vec<LogEntry> {
    let mut out = Vec::new();

    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // date time action package old new
        if fields.len() < 6 {
            continue;
        }

        let action = match fields[2] {
            "install" => Action::Installed,
            "upgrade" => Action::Upgraded,
            "remove" => Action::Removed,
            _ => continue,
        };

        let Some(occurred_at) = parse_dpkg_timestamp(fields[0], fields[1]) else {
            continue;
        };

        // Package names carry an architecture suffix: "libfoo:amd64".
        let name = fields[3].split(':').next().unwrap_or(fields[3]).to_string();
        if name.is_empty() {
            continue;
        }

        let old_raw = fields[4];
        let new_raw = fields[5];
        let old_version = none_to_empty(old_raw);
        let new_version = none_to_empty(new_raw);

        // dpkg logs downgrades as `upgrade` with a lower target version.
        // Comparing Debian versions properly needs dpkg's own algorithm,
        // so this only claims a downgrade when a plain comparison is
        // confident; anything ambiguous stays an upgrade, which is the
        // quieter of the two and therefore the safer default.
        let action = if action == Action::Upgraded && is_probable_downgrade(&old_version, &new_version)
        {
            Action::Downgraded
        } else {
            action
        };

        out.push(LogEntry {
            action,
            name,
            old_version,
            new_version,
            occurred_at,
        });
    }

    out
}

fn parse_dpkg_timestamp(date: &str, time: &str) -> Option<DateTime<Utc>> {
    let combined = format!("{} {}", date, time);
    let naive = NaiveDateTime::parse_from_str(&combined, "%Y-%m-%d %H:%M:%S").ok()?;
    local_to_utc(naive)
}

// ---------------------------------------------------------------------
// Fedora / RHEL — /var/log/dnf.rpm.log
// ---------------------------------------------------------------------
//
//   2026-09-20T10:00:00+0000 SUBDEBUG Installed: foo-1.0-1.fc40.x86_64
//   2026-09-20T10:00:00+0000 SUBDEBUG Upgraded: bar-1:2.0-1.fc40.x86_64
//   2026-09-20T10:00:00+0000 SUBDEBUG Erased: baz-1.0-1.fc40.x86_64
//
// UNVERIFIED ON REAL HARDWARE. The dpkg and pacman formats above are
// confirmed against real logs; this one is written from the documented
// format and covered by unit tests, but has not been run against a live
// Fedora system. Treat a surprise here as a parser bug before assuming
// the machine changed.

pub fn parse_rpm_log(text: &str) -> Vec<LogEntry> {
    let mut out = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        let Some((timestamp, rest)) = line.split_once(' ') else {
            continue;
        };
        let Some(occurred_at) = parse_rpm_timestamp(timestamp) else {
            continue;
        };

        let rest = rest.trim_start_matches("SUBDEBUG").trim();
        let (action, nevra) = if let Some(r) = rest.strip_prefix("Installed:") {
            (Action::Installed, r)
        } else if let Some(r) = rest.strip_prefix("Upgraded:") {
            (Action::Upgraded, r)
        } else if let Some(r) = rest.strip_prefix("Downgraded:") {
            (Action::Downgraded, r)
        } else if let Some(r) = rest.strip_prefix("Erased:") {
            (Action::Removed, r)
        } else if let Some(r) = rest.strip_prefix("Obsoleted:") {
            (Action::Removed, r)
        } else {
            continue;
        };

        let Some((name, version)) = split_nevra(nevra.trim()) else {
            continue;
        };

        let (old_version, new_version) = match action {
            Action::Removed => (version, String::new()),
            _ => (String::new(), version),
        };

        out.push(LogEntry {
            action,
            name,
            old_version,
            new_version,
            occurred_at,
        });
    }

    out
}

fn parse_rpm_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        return Some(dt.with_timezone(&Utc));
    }
    if let Ok(dt) = DateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S%z") {
        return Some(dt.with_timezone(&Utc));
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S") {
        return local_to_utc(naive);
    }
    None
}

/// Split an RPM NEVRA ("foo-1.0-1.fc40.x86_64") into name and version.
/// The name is everything before the last two hyphen-separated fields,
/// since a package name may itself contain hyphens.
fn split_nevra(nevra: &str) -> Option<(String, String)> {
    let release_split = nevra.rfind('-')?;
    let (head, release_and_arch) = nevra.split_at(release_split);
    let version_split = head.rfind('-')?;
    let (name, version) = head.split_at(version_split);
    if name.is_empty() {
        return None;
    }
    let version = version.trim_start_matches('-');
    let release_and_arch = release_and_arch.trim_start_matches('-');
    Some((name.to_string(), format!("{}-{}", version, release_and_arch)))
}

// ---------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------

fn none_to_empty(v: &str) -> String {
    if v == "<none>" {
        String::new()
    } else {
        v.to_string()
    }
}

fn local_to_utc(naive: NaiveDateTime) -> Option<DateTime<Utc>> {
    // A local time can be ambiguous (the hour that repeats when clocks go
    // back) or nonexistent (the hour that is skipped when they go
    // forward). Taking the earliest valid interpretation keeps this
    // total — a package log entry is not worth dropping over an hour of
    // DST ambiguity.
    Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|dt| dt.with_timezone(&Utc))
}

/// One component of a version string. Versions tokenize into strictly
/// alternating non-digit and digit runs, starting with a (possibly empty)
/// non-digit run — the same decomposition dpkg uses.
#[derive(Clone, Debug, PartialEq, Eq)]
enum VersionToken {
    /// A run of non-digits, separators and tildes included. Keeping
    /// separators is essential: `14-20240412` vs `14.2.0` is decided
    /// entirely by the `-` against the `.`, and dropping them made
    /// gcc-14-base's ordinary upgrade look like a downgrade.
    Alpha(String),
    Num(u64),
}

/// Character ordering inside a non-digit run, following dpkg's rule:
/// `~` sorts before everything (including the end of the string), then
/// letters, then every other character in ASCII order.
///
/// The end of a string is 0, which falls between `~` (negative) and the
/// letters (65+) — that is what makes `1.0~rc1 < 1.0 < 1.0.1` come out
/// right without any special-casing.
fn debian_char_order(c: char) -> i32 {
    if c == '~' {
        -1
    } else if c.is_ascii_alphabetic() {
        c as i32
    } else {
        c as i32 + 256
    }
}

fn compare_alpha(a: &str, b: &str) -> std::cmp::Ordering {
    let ac: Vec<char> = a.chars().collect();
    let bc: Vec<char> = b.chars().collect();
    for i in 0..ac.len().max(bc.len()) {
        let x = ac.get(i).copied().map(debian_char_order).unwrap_or(0);
        let y = bc.get(i).copied().map(debian_char_order).unwrap_or(0);
        if x != y {
            return x.cmp(&y);
        }
    }
    std::cmp::Ordering::Equal
}

/// Split a version into comparable components.
///
/// Not a complete implementation of any distro's algorithm — those have
/// corner cases nobody should reimplement from memory — but it handles
/// the shapes that actually occur in the wild:
///
///   0.41.2-1            Arch
///   1.2.11-1ubuntu0.3   Debian/Ubuntu
///   1.0-1.fc40          Fedora
///   1.0~rc1             pre-release
///
/// Digit runs become numbers so that 10 > 9, which is the whole reason
/// naive string comparison is not good enough here.
fn tokenize_version(v: &str) -> Vec<VersionToken> {
    // Drop any epoch prefix ("1:2.0-1" -> "2.0-1").
    let v = v.split_once(':').map(|(_, rest)| rest).unwrap_or(v);

    let mut tokens = Vec::new();
    let chars: Vec<char> = v.chars().collect();
    let mut i = 0;

    // Strictly alternating, always starting with a non-digit run so that
    // the two token streams line up position for position.
    loop {
        let start = i;
        while i < chars.len() && !chars[i].is_ascii_digit() {
            i += 1;
        }
        tokens.push(VersionToken::Alpha(chars[start..i].iter().collect()));

        if i >= chars.len() {
            break;
        }

        let start = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        let run: String = chars[start..i].iter().collect();
        // Saturate rather than drop: a number that large is nonsense, but
        // losing the component entirely would be worse.
        tokens.push(VersionToken::Num(run.parse().unwrap_or(u64::MAX)));

        if i >= chars.len() {
            break;
        }
    }

    tokens
}

/// Compare two version strings, following dpkg's algorithm closely
/// enough for the one job this has: deciding whether a version went
/// backwards. `None` when neither side has any content.
///
/// Missing components are compared as an empty non-digit run or a zero,
/// which is what makes `1.0~rc1 < 1.0 < 1.0.1` fall out naturally.
fn compare_versions(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering;

    if a.is_empty() && b.is_empty() {
        return None;
    }

    let ta = tokenize_version(a);
    let tb = tokenize_version(b);

    let empty_alpha = VersionToken::Alpha(String::new());
    let zero = VersionToken::Num(0);

    for i in 0..ta.len().max(tb.len()) {
        // Even positions are non-digit runs, odd positions are numbers.
        let default = if i % 2 == 0 { &empty_alpha } else { &zero };
        let x = ta.get(i).unwrap_or(default);
        let y = tb.get(i).unwrap_or(default);

        let ord = match (x, y) {
            (VersionToken::Alpha(p), VersionToken::Alpha(q)) => compare_alpha(p, q),
            (VersionToken::Num(p), VersionToken::Num(q)) => p.cmp(q),
            // Cannot happen while both streams alternate, but stay total
            // rather than panicking on a version shape nobody anticipated.
            _ => Ordering::Equal,
        };

        if ord != Ordering::Equal {
            return Some(ord);
        }
    }

    Some(Ordering::Equal)
}

/// Whether `new` is older than `old`. Used only to spot downgrades in
/// formats that don't label them.
///
/// Returns false whenever it isn't sure, so an unrecognised scheme is
/// reported as the quieter "upgrade" rather than raising a false alarm.
fn is_probable_downgrade(old: &str, new: &str) -> bool {
    if old.is_empty() || new.is_empty() {
        return false;
    }
    compare_versions(new, old) == Some(std::cmp::Ordering::Less)
}

// ---------------------------------------------------------------------
// Turning log entries into changes
// ---------------------------------------------------------------------

/// Convert parsed log entries into changes, deciding for each install
/// whether the user actually asked for that package.
///
/// `explicitly_installed` is the set the package manager considers
/// user-requested: `pacman -Qeq`, `apt-mark showmanual`, or
/// `dnf repoquery --userinstalled`. A newly installed package that is
/// *not* in that set arrived as somebody else's dependency — which is
/// the whole point of this subsystem.
///
/// An empty set means MAVIS could not determine intent (the query failed,
/// or the manager is unknown). Everything is then reported as requested,
/// so a broken query produces silence rather than a flood of false
/// "you didn't ask for this" alerts.
pub fn to_changes(
    entries: &[LogEntry],
    explicitly_installed: &HashSet<String>,
    source: &str,
) -> Vec<Change> {
    let intent_known = !explicitly_installed.is_empty();

    entries
        .iter()
        .map(|entry| {
            let kind = match entry.action {
                Action::Installed => ChangeKind::PackageInstalled {
                    name: entry.name.clone(),
                    version: entry.new_version.clone(),
                    requested: !intent_known || explicitly_installed.contains(&entry.name),
                },
                Action::Removed => ChangeKind::PackageRemoved {
                    name: entry.name.clone(),
                    version: entry.old_version.clone(),
                },
                Action::Upgraded => ChangeKind::PackageUpgraded {
                    name: entry.name.clone(),
                    from: entry.old_version.clone(),
                    to: entry.new_version.clone(),
                },
                Action::Downgraded => ChangeKind::PackageDowngraded {
                    name: entry.name.clone(),
                    from: entry.old_version.clone(),
                    to: entry.new_version.clone(),
                },
            };
            Change::new(kind, source, entry.occurred_at)
        })
        .collect()
}

/// Entries at or after `since`. Transaction logs are append-only and can
/// be large (this container's dpkg.log is 675 KB), so MAVIS reads the
/// whole file but only reports what it hasn't seen.
pub fn entries_since(entries: &[LogEntry], since: DateTime<Utc>) -> Vec<LogEntry> {
    entries
        .iter()
        .filter(|e| e.occurred_at >= since)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sentinel::change::Severity;

    // -----------------------------------------------------------------
    // pacman
    // -----------------------------------------------------------------

    const PACMAN_SAMPLE: &str = "\
[2026-09-18T14:32:00+0545] [PACMAN] Running 'pacman -Syu'
[2026-09-18T14:32:01+0545] [ALPM] transaction started
[2026-09-18T14:32:01+0545] [ALPM] installed hyprland (0.41.2-1)
[2026-09-18T14:32:02+0545] [ALPM] upgraded firefox (140.0-1 -> 141.0-1)
[2026-09-18T14:32:03+0545] [ALPM] removed obsolete-thing (1.0-1)
[2026-09-18T14:32:04+0545] [ALPM] downgraded mesa (24.2-1 -> 24.1-1)
[2026-09-18T14:32:05+0545] [ALPM-SCRIPTLET] Updating icon cache...
[2026-09-18T14:32:06+0545] [ALPM] transaction completed
";

    #[test]
    fn pacman_parses_all_four_actions() {
        let entries = parse_pacman_log(PACMAN_SAMPLE);
        assert_eq!(entries.len(), 4, "got: {:#?}", entries);

        assert_eq!(entries[0].action, Action::Installed);
        assert_eq!(entries[0].name, "hyprland");
        assert_eq!(entries[0].new_version, "0.41.2-1");

        assert_eq!(entries[1].action, Action::Upgraded);
        assert_eq!(entries[1].name, "firefox");
        assert_eq!(entries[1].old_version, "140.0-1");
        assert_eq!(entries[1].new_version, "141.0-1");

        assert_eq!(entries[2].action, Action::Removed);
        assert_eq!(entries[2].name, "obsolete-thing");
        assert_eq!(entries[2].old_version, "1.0-1");

        assert_eq!(entries[3].action, Action::Downgraded);
        assert_eq!(entries[3].name, "mesa");
    }

    #[test]
    fn pacman_ignores_non_transaction_lines() {
        let entries = parse_pacman_log(
            "[2026-09-18T14:32:00+0545] [PACMAN] Running 'pacman -Syu'\n\
             [2026-09-18T14:32:05+0545] [ALPM-SCRIPTLET] installed something odd\n\
             garbage line with no brackets\n\
             \n",
        );
        assert!(entries.is_empty(), "got: {:#?}", entries);
    }

    #[test]
    fn pacman_accepts_the_pre_2019_timestamp_format() {
        let entries = parse_pacman_log("[2018-05-01 09:15] [ALPM] installed oldpkg (1.0-1)\n");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "oldpkg");
    }

    #[test]
    fn pacman_handles_hyphenated_package_names() {
        let entries =
            parse_pacman_log("[2026-09-18T14:32:01+0545] [ALPM] installed xdg-desktop-portal-hyprland (1.3.1-2)\n");
        assert_eq!(entries[0].name, "xdg-desktop-portal-hyprland");
        assert_eq!(entries[0].new_version, "1.3.1-2");
    }

    #[test]
    fn pacman_survives_malformed_lines() {
        for line in [
            "[not a timestamp] [ALPM] installed foo (1.0)",
            "[2026-09-18T14:32:01+0545] [ALPM] installed",
            "[2026-09-18T14:32:01+0545] [ALPM] installed foo-no-parens",
            "[",
            "[]",
            "",
        ] {
            let _ = parse_pacman_log(line);
        }
    }

    // -----------------------------------------------------------------
    // dpkg
    // -----------------------------------------------------------------

    const DPKG_SAMPLE: &str = "\
2026-09-20 09:50:40 status half-configured libasound2t64:amd64 1.2.11-1ubuntu0.2
2026-09-20 09:50:40 upgrade libasound2t64:amd64 1.2.11-1ubuntu0.2 1.2.11-1ubuntu0.3
2026-09-20 09:50:40 install libasound2-dev:amd64 <none> 1.2.11-1ubuntu0.3
2026-09-20 09:50:41 install libwayland-dev:amd64 <none> 1.22.0-2.1build1
2026-09-20 09:50:42 remove oldpkg:amd64 3.2.1-1 <none>
2026-09-20 09:50:42 purge oldpkg:amd64 3.2.1-1 <none>
2026-09-20 09:50:43 configure libwayland-dev:amd64 1.22.0-2.1build1 <none>
";

    #[test]
    fn dpkg_parses_installs_upgrades_and_removals() {
        let entries = parse_dpkg_log(DPKG_SAMPLE);
        assert_eq!(entries.len(), 4, "got: {:#?}", entries);

        assert_eq!(entries[0].action, Action::Upgraded);
        assert_eq!(entries[0].name, "libasound2t64");
        assert_eq!(entries[0].old_version, "1.2.11-1ubuntu0.2");

        assert_eq!(entries[1].action, Action::Installed);
        assert_eq!(entries[1].name, "libasound2-dev");
        // `<none>` must become empty, not the literal token.
        assert_eq!(entries[1].old_version, "");

        assert_eq!(entries[3].action, Action::Removed);
        assert_eq!(entries[3].name, "oldpkg");
    }

    /// apt logs `remove` then `purge` for the same package. Counting both
    /// would report one removal twice.
    #[test]
    fn dpkg_does_not_double_count_purge_after_remove() {
        let removals = parse_dpkg_log(DPKG_SAMPLE)
            .into_iter()
            .filter(|e| e.action == Action::Removed)
            .count();
        assert_eq!(removals, 1);
    }

    #[test]
    fn dpkg_strips_the_architecture_suffix() {
        let entries = parse_dpkg_log("2026-09-20 09:50:40 install foo:arm64 <none> 1.0\n");
        assert_eq!(entries[0].name, "foo");
    }

    #[test]
    fn dpkg_detects_a_downgrade_logged_as_an_upgrade() {
        let entries = parse_dpkg_log("2026-09-20 09:50:40 upgrade foo:amd64 2.0-1 1.9-1\n");
        assert_eq!(entries[0].action, Action::Downgraded);
    }

    /// An unparseable version scheme must stay the quieter "upgrade"
    /// rather than raising a false downgrade alarm.
    #[test]
    fn dpkg_does_not_guess_at_unfamiliar_version_schemes() {
        let entries = parse_dpkg_log("2026-09-20 09:50:40 upgrade foo:amd64 abc-xyz def-uvw\n");
        assert_eq!(entries[0].action, Action::Upgraded);
    }

    #[test]
    fn dpkg_survives_malformed_lines() {
        for line in [
            "2026-09-20 09:50:40 install",
            "not a log line at all",
            "2026-13-45 99:99:99 install foo:amd64 <none> 1.0",
            "",
        ] {
            let _ = parse_dpkg_log(line);
        }
    }

    // -----------------------------------------------------------------
    // rpm
    // -----------------------------------------------------------------

    #[test]
    fn rpm_parses_the_documented_format() {
        let entries = parse_rpm_log(
            "2026-09-20T10:00:00+0000 SUBDEBUG Installed: foo-1.0-1.fc40.x86_64\n\
             2026-09-20T10:00:01+0000 SUBDEBUG Upgraded: bar-2.0-1.fc40.x86_64\n\
             2026-09-20T10:00:02+0000 SUBDEBUG Erased: baz-1.0-1.fc40.x86_64\n\
             2026-09-20T10:00:03+0000 INFO --- logging initialized ---\n",
        );
        assert_eq!(entries.len(), 3, "got: {:#?}", entries);
        assert_eq!(entries[0].name, "foo");
        assert_eq!(entries[0].action, Action::Installed);
        assert_eq!(entries[2].action, Action::Removed);
    }

    #[test]
    fn rpm_splits_hyphenated_names_correctly() {
        assert_eq!(
            split_nevra("xdg-desktop-portal-1.18.4-1.fc40.x86_64"),
            Some((
                "xdg-desktop-portal".to_string(),
                "1.18.4-1.fc40.x86_64".to_string()
            ))
        );
    }

    #[test]
    fn rpm_survives_malformed_lines() {
        for line in ["2026-09-20T10:00:00+0000 SUBDEBUG Installed:", "nonsense", ""] {
            let _ = parse_rpm_log(line);
        }
    }

    // -----------------------------------------------------------------
    // Version comparison
    // -----------------------------------------------------------------

    #[test]
    fn downgrade_detection_handles_real_distro_versions() {
        // Arch
        assert!(is_probable_downgrade("0.41.2-1", "0.41.1-1"));
        assert!(!is_probable_downgrade("0.41.1-1", "0.41.2-1"));
        // Debian/Ubuntu — the revision suffix is where the change lives
        assert!(is_probable_downgrade("1.2.11-1ubuntu0.3", "1.2.11-1ubuntu0.2"));
        assert!(!is_probable_downgrade("1.2.11-1ubuntu0.2", "1.2.11-1ubuntu0.3"));
        // Fedora
        assert!(is_probable_downgrade("1.0-2.fc40", "1.0-1.fc40"));
        // Plain
        assert!(is_probable_downgrade("2.0", "1.9"));
        assert!(!is_probable_downgrade("1.9", "2.0"));
        assert!(!is_probable_downgrade("1.0", "1.0"));
        // Epoch prefixes are stripped before comparing.
        assert!(is_probable_downgrade("1:2.0", "1:1.0"));
    }

    /// Numeric runs must compare as numbers, not as text, or 10 sorts
    /// below 9 and every double-digit release looks like a downgrade.
    #[test]
    fn version_components_compare_numerically() {
        assert!(!is_probable_downgrade("9.0", "10.0"));
        assert!(is_probable_downgrade("10.0", "9.0"));
        assert!(!is_probable_downgrade("1.9.0", "1.10.0"));
    }

    /// Debian's tilde marks a pre-release and sorts BELOW the plain
    /// version, so 1.0 -> 1.0~rc1 really is going backwards.
    #[test]
    fn tilde_sorts_below_the_release_it_precedes() {
        assert!(is_probable_downgrade("1.0", "1.0~rc1"));
        assert!(!is_probable_downgrade("1.0~rc1", "1.0"));
    }

    /// Real versions taken from this project's own CI container. All
    /// three are ORDINARY UPGRADES that an earlier, separator-dropping
    /// comparison reported as downgrades — it compared 20240412 against
    /// 2 instead of noticing that the '-' and the '.' decide it first.
    /// These packages are on every Ubuntu machine, so the false alarm
    /// would have fired for everyone.
    #[test]
    fn real_ubuntu_gcc_versions_are_not_downgrades() {
        for (old, new) in [
            ("14-20240412-0ubuntu1", "14.2.0-4ubuntu2~24.04.1"), // gcc-14-base
            ("14-20240412-0ubuntu1", "14.2.0-4ubuntu2~24.04.1"), // libgcc-s1
            ("14-20240412-0ubuntu1", "14.2.0-4ubuntu2~24.04.1"), // libstdc++6
        ] {
            assert!(
                !is_probable_downgrade(old, new),
                "{} -> {} is an upgrade, not a downgrade",
                old,
                new
            );
            // ...and the reverse direction really is a downgrade.
            assert!(is_probable_downgrade(new, old));
        }
    }

    /// Separators are significant, which is the whole reason the case
    /// above works.
    #[test]
    fn separators_are_compared_not_discarded() {
        use std::cmp::Ordering;
        // '-' (0x2D) sorts before '.' (0x2E)
        assert_eq!(compare_versions("14-1", "14.1"), Some(Ordering::Less));
        assert_eq!(compare_versions("14.1", "14-1"), Some(Ordering::Greater));
    }

    #[test]
    fn downgrade_detection_stays_quiet_when_unsure() {
        assert!(!is_probable_downgrade("", "1.0"));
        assert!(!is_probable_downgrade("1.0", ""));
        assert_eq!(compare_versions("", ""), None);
    }

    #[test]
    fn tokenizer_survives_odd_input() {
        for v in ["", "~", "...", "---", "1:", ":::", "\u{2019}", "99999999999999999999999"] {
            let _ = tokenize_version(v);
            let _ = compare_versions(v, "1.0");
        }
    }

    // -----------------------------------------------------------------
    // Log entries -> changes
    // -----------------------------------------------------------------

    fn explicit(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    /// The scenario this subsystem exists for, end to end: an update
    /// pulls in a package the user never asked for, and it comes out as
    /// Notable while the rest of the update stays quiet.
    #[test]
    fn a_dependency_pulled_in_by_an_update_is_notable() {
        let entries = parse_pacman_log(PACMAN_SAMPLE);
        // The user explicitly installed firefox, never hyprland.
        let changes = to_changes(&entries, &explicit(&["firefox", "niri"]), "pacman");

        let hyprland = changes
            .iter()
            .find(|c| c.detail.contains("hyprland"))
            .expect("hyprland change present");
        assert_eq!(hyprland.severity, Severity::Notable);
        assert!(hyprland.detail.contains("didn't ask for it"));

        let firefox = changes
            .iter()
            .find(|c| c.detail.contains("firefox"))
            .expect("firefox change present");
        assert_eq!(firefox.severity, Severity::Routine);
    }

    #[test]
    fn an_explicitly_installed_package_is_routine() {
        let entries = parse_pacman_log("[2026-09-18T14:32:01+0545] [ALPM] installed neovim (0.10-1)\n");
        let changes = to_changes(&entries, &explicit(&["neovim"]), "pacman");
        assert_eq!(changes[0].severity, Severity::Routine);
    }

    /// If MAVIS cannot determine intent, it must not flood the user with
    /// false "you didn't ask for this" alerts.
    #[test]
    fn an_unknown_explicit_set_reports_everything_as_requested() {
        let entries = parse_pacman_log(PACMAN_SAMPLE);
        let changes = to_changes(&entries, &HashSet::new(), "pacman");
        let installs: Vec<_> = changes
            .iter()
            .filter(|c| matches!(c.kind, ChangeKind::PackageInstalled { .. }))
            .collect();
        assert!(!installs.is_empty());
        assert!(installs.iter().all(|c| c.severity == Severity::Routine));
    }

    #[test]
    fn entries_since_filters_by_time() {
        let entries = parse_pacman_log(PACMAN_SAMPLE);
        let cutoff = entries[2].occurred_at;
        let recent = entries_since(&entries, cutoff);
        assert_eq!(recent.len(), 2, "got: {:#?}", recent);
        assert_eq!(recent[0].name, "obsolete-thing");
    }

    #[test]
    fn fingerprints_are_unique_across_a_real_transaction() {
        let entries = parse_pacman_log(PACMAN_SAMPLE);
        let changes = to_changes(&entries, &explicit(&["firefox"]), "pacman");
        let prints: HashSet<String> = changes.iter().map(|c| c.fingerprint()).collect();
        assert_eq!(prints.len(), changes.len(), "fingerprints must be unique");
    }
}