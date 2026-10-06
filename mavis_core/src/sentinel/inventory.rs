// mavis_core/src/sentinel/inventory.rs
// Phase 8.5 step 5: what is installed and what starts by itself, on
// Windows and macOS. Parsers and diffs only; `checks.rs` runs the commands.
//
// UNVERIFIED ON REAL HARDWARE. Written from the documented cmdlets and
// tools and tested against hand-written samples, never a real machine.

use super::change::{Change, ChangeKind};
use super::checks::Snapshot;
use chrono::{DateTime, Utc};
use std::path::PathBuf;

/// Last line of every PowerShell script here. Without it the output was
/// cut short and is discarded, so a failed run can't look like "all removed".
pub const END: &str = "MAVIS-END";

const UTF8: &str = "[Console]::OutputEncoding=[Text.Encoding]::UTF8;";

/// Store apps and classic installers, as "name<TAB>version". These two
/// lists are also everything `winget list` shows, so winget isn't run.
pub fn windows_apps_script() -> String {
    format!(
        r#"{UTF8} Get-AppxPackage | ForEach-Object {{ $_.Name + "`t" + $_.Version }};
Get-ItemProperty 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*','HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*','HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*' -ErrorAction SilentlyContinue |
  Where-Object {{ $_.DisplayName }} | ForEach-Object {{ $_.DisplayName + "`t" + $_.DisplayVersion }};
'{END}'"#
    )
}

/// Enabled scheduled tasks and startup items, as "scope<TAB>name".
pub fn windows_autostart_script() -> String {
    format!(
        r#"{UTF8} Get-ScheduledTask | Where-Object {{ $_.State -ne 'Disabled' }} | ForEach-Object {{ 'scheduled task' + "`t" + $_.TaskPath + $_.TaskName }};
Get-CimInstance Win32_StartupCommand | ForEach-Object {{ 'startup' + "`t" + $_.Name }};
'{END}'"#
    )
}

/// Defender's detection history, as "id<TAB>threat<TAB>resources".
pub fn windows_defender_script() -> String {
    format!(
        r#"{UTF8} Get-MpThreatDetection | ForEach-Object {{ $t = Get-MpThreat -ThreatID $_.ThreatID -ErrorAction SilentlyContinue;
  [string]$_.DetectionID + "`t" + $t.ThreatName + "`t" + ($_.Resources -join '; ') }};
'{END}'"#
    )
}

/// First tab-separated field -> the rest. None without the end marker.
pub fn parse_marked(output: &str) -> Option<Snapshot> {
    let mut lines: Vec<&str> = output.lines().map(str::trim_end).collect();
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    if lines.pop() != Some(END) {
        return None;
    }
    Some(
        lines
            .into_iter()
            .filter_map(|l| {
                let (key, rest) = l.split_once('\t').unwrap_or((l, ""));
                (!key.trim().is_empty()).then(|| (key.trim().to_string(), rest.trim().to_string()))
            })
            .collect(),
    )
}

/// Like `parse_marked`, but the whole "scope<TAB>name" line is the key.
pub fn parse_marked_pairs(output: &str) -> Option<Snapshot> {
    let snap = parse_marked(output)?;
    Some(
        snap.into_iter()
            .filter(|(_, name)| !name.is_empty())
            .map(|(scope, name)| (format!("{}\t{}", scope, name), String::new()))
            .collect(),
    )
}

/// `brew list --versions`: "name version [older versions…]".
pub fn parse_brew(output: &str) -> Snapshot {
    output
        .lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let name = f.next()?;
            Some((name.to_string(), f.next_back().unwrap_or("").to_string()))
        })
        .collect()
}

/// `pkgutil --pkgs`: one package ID per line. Apple's own IDs are left
/// out — a macOS update replaces hundreds of them at once.
pub fn parse_pkgutil(output: &str) -> Snapshot {
    output
        .lines()
        .map(str::trim)
        .filter(|id| !id.is_empty() && !id.starts_with("com.apple."))
        .map(|id| (id.to_string(), String::new()))
        .collect()
}

/// launchd jobs outside /System, as "scope<TAB>file". None if no
/// directory could be listed at all.
pub fn read_launchd() -> Option<Snapshot> {
    let mut dirs = vec![
        ("system", PathBuf::from("/Library/LaunchDaemons")),
        ("all users", PathBuf::from("/Library/LaunchAgents")),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(("your user", PathBuf::from(home).join("Library/LaunchAgents")));
    }
    let mut snap = Snapshot::new();
    let mut listed = false;
    for (scope, dir) in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        listed = true;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".plist") {
                snap.insert(format!("{}\t{}", scope, name), String::new());
            }
        }
    }
    listed.then_some(snap)
}

/// Apps that appeared, disappeared or changed version.
pub fn diff_apps(source: &str, old: &Snapshot, new: &Snapshot, at: DateTime<Utc>) -> Vec<Change> {
    let mut kinds = Vec::new();
    for (name, version) in new {
        match old.get(name) {
            None => kinds.push(ChangeKind::AppAdded {
                name: name.clone(),
                version: version.clone(),
                returned: false,
            }),
            Some(before) if before != version => kinds.push(ChangeKind::AppUpdated {
                name: name.clone(),
                from: before.clone(),
                to: version.clone(),
            }),
            Some(_) => {}
        }
    }
    for name in old.keys().filter(|n| !new.contains_key(*n)) {
        kinds.push(ChangeKind::AppRemoved { name: name.clone() });
    }
    kinds.into_iter().map(|k| Change::new(k, source, at)).collect()
}

/// Things newly set to start by themselves, or no longer set to.
pub fn diff_autostart(source: &str, old: &Snapshot, new: &Snapshot, at: DateTime<Utc>) -> Vec<Change> {
    let split = |key: &str| {
        let (scope, unit) = key.split_once('\t').unwrap_or(("", key));
        (unit.to_string(), scope.to_string())
    };
    let added = new.keys().filter(|k| !old.contains_key(*k)).map(|k| {
        let (unit, scope) = split(k);
        ChangeKind::UnitEnabled { unit, scope }
    });
    let removed = old.keys().filter(|k| !new.contains_key(*k)).map(|k| {
        let (unit, scope) = split(k);
        ChangeKind::UnitDisabled { unit, scope }
    });
    added.chain(removed).map(|k| Change::new(k, source, at)).collect()
}

/// New entries in a scanner's detection history. Entries that age out of
/// the history are not changes.
pub fn diff_detections(
    source: &str,
    scanner: &str,
    old: &Snapshot,
    new: &Snapshot,
    at: DateTime<Utc>,
) -> Vec<Change> {
    new.iter()
        .filter(|(id, _)| !old.contains_key(*id))
        .map(|(_, detail)| {
            let (threat, resource) = detail.split_once('\t').unwrap_or((detail, ""));
            let threat = if threat.is_empty() { "an unnamed threat" } else { threat };
            ChangeKind::ScannerDetection {
                scanner: scanner.to_string(),
                threat: threat.to_string(),
                resource: resource.chars().take(200).collect(),
            }
        })
        .map(|k| Change::new(k, source, at))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sentinel::change::Severity;

    fn at() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).unwrap()
    }

    // Hand-written in the shape the scripts print. Not real output.
    const APPS: &str = "Example.StoreApp\t11.2405.2.0\r\nExample Program (x64)\t131.0\r\nNo Version App\t\r\nMAVIS-END\r\n";

    #[test]
    fn marked_output_is_read_and_unmarked_output_is_refused() {
        let s = parse_marked(APPS).unwrap();
        assert_eq!(s["Example.StoreApp"], "11.2405.2.0");
        assert_eq!(s["Example Program (x64)"], "131.0");
        assert_eq!(s["No Version App"], "");

        assert!(parse_marked("Example.StoreApp\t11.0\r\n").is_none(), "cut short");
        assert!(parse_marked("").is_none());
        assert_eq!(parse_marked("MAVIS-END\n"), Some(Snapshot::new()), "finished, nothing found");
    }

    #[test]
    fn a_new_app_is_notable_and_an_update_is_routine() {
        let old = parse_marked(APPS).unwrap();
        let mut new = old.clone();
        new.insert("example-app".into(), "1.0".into());
        new.insert("Example Program (x64)".into(), "132.0".into());
        new.remove("No Version App");

        let changes = diff_apps("apps", &old, &new, at());
        assert_eq!(changes.len(), 3);
        let severity = |want: fn(&ChangeKind) -> bool| changes.iter().find(|c| want(&c.kind)).map(|c| c.severity);
        assert_eq!(severity(|k| matches!(k, ChangeKind::AppAdded { .. })), Some(Severity::Notable));
        assert_eq!(severity(|k| matches!(k, ChangeKind::AppUpdated { .. })), Some(Severity::Routine));
        assert_eq!(severity(|k| matches!(k, ChangeKind::AppRemoved { .. })), Some(Severity::Routine));
    }

    #[test]
    fn a_new_scheduled_task_is_noticed() {
        let old = parse_marked_pairs("startup\texample-startup\nMAVIS-END\n").unwrap();
        let new = parse_marked_pairs("startup\texample-startup\nscheduled task\t\\Updater\\Nightly\nMAVIS-END\n").unwrap();
        let changes = diff_autostart("autostart", &old, &new, at());
        assert_eq!(changes.len(), 1);
        assert!(matches!(
            &changes[0].kind,
            ChangeKind::UnitEnabled { unit, scope } if unit == "\\Updater\\Nightly" && scope == "scheduled task"
        ));
        assert_eq!(changes[0].severity, Severity::Notable);
    }

    #[test]
    fn a_new_detection_is_critical_and_old_history_is_not_news() {
        let old = parse_marked("{A1}\tExample.OldThreat\tfile:_C:\\old.exe\nMAVIS-END\n").unwrap();
        let new = parse_marked("{B2}\tExample.NewThreat\tfile:_C:\\new.exe\nMAVIS-END\n").unwrap();
        let changes = diff_detections("defender", "Microsoft Defender", &old, &new, at());
        assert_eq!(changes.len(), 1, "the entry that aged out is not a change");
        assert_eq!(changes[0].severity, Severity::Critical);
        assert!(changes[0].detail.starts_with("Microsoft Defender reported Example.NewThreat"));
    }

    #[test]
    fn brew_and_pkgutil_lists_are_read() {
        let brew = parse_brew("tool-a 1.24.5\ntool-b@3 3.12.3 3.12.4\n\n");
        assert_eq!(brew["tool-a"], "1.24.5");
        assert_eq!(brew["tool-b@3"], "3.12.4", "the newest installed version");

        let pkgs = parse_pkgutil("com.apple.pkg.Core\norg.example.tool\n\n");
        assert_eq!(pkgs.len(), 1);
        assert!(pkgs.contains_key("org.example.tool"));
    }
}