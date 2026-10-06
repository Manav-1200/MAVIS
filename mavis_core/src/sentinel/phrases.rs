// mavis_core/src/sentinel/phrases.rs
// Spoken clauses for step 4 and 5 changes: integrity, advisories, apps
// and scanner detections. `summary.rs` joins them into the sentence.

use super::change::{Change, ChangeKind, Severity};
use super::summary::{counted, file_name};

#[derive(Default)]
pub(super) struct Clauses {
    /// Worth saying unprompted.
    pub loud: Vec<String>,
    /// Routine; only said when the user asks what changed.
    pub quiet: Vec<String>,
}

#[derive(Default)]
struct Buckets {
    scanner: String,
    detections: Vec<String>,
    content: Vec<String>,
    content_package: String,
    missing: Vec<String>,
    permissions: Vec<String>,
    advisories: Vec<String>,
    advisory_severity: String,
    apps_new: Vec<String>,
    apps_back: Vec<String>,
    config: Vec<String>,
    touched: Vec<String>,
    restored: Vec<String>,
    lesser_advisories: Vec<String>,
    resolved: Vec<String>,
    apps_removed: Vec<String>,
    apps_updated: Vec<String>,
}

fn push_once(list: &mut Vec<String>, item: &str) {
    if !list.iter().any(|i| i == item) {
        list.push(item.to_string());
    }
}

pub(super) fn clauses(changes: &[&Change]) -> Clauses {
    let mut b = Buckets::default();
    for c in changes {
        match &c.kind {
            ChangeKind::ScannerDetection { scanner, threat, .. } => {
                b.scanner = scanner.clone();
                b.detections.push(threat.clone());
            }
            ChangeKind::FileAltered { path, config: true, .. } => b.config.push(file_name(path)),
            ChangeKind::FileAltered { path, package, what, .. } => match what.as_str() {
                "content" => {
                    b.content.push(file_name(path));
                    b.content_package = package.clone();
                }
                "missing" => b.missing.push(file_name(path)),
                "permissions" => b.permissions.push(file_name(path)),
                _ => b.touched.push(file_name(path)),
            },
            ChangeKind::FileRestored { path } => b.restored.push(file_name(path)),
            ChangeKind::AdvisoryAdded { scanner, package, severity, .. } => {
                b.scanner = scanner.clone();
                if c.severity > Severity::Routine {
                    push_once(&mut b.advisories, package);
                    b.advisory_severity = severity.clone();
                } else {
                    push_once(&mut b.lesser_advisories, package);
                }
            }
            ChangeKind::AdvisoryResolved { package, .. } => push_once(&mut b.resolved, package),
            ChangeKind::AppAdded { name, returned: true, .. } => b.apps_back.push(name.clone()),
            ChangeKind::AppAdded { name, .. } => b.apps_new.push(name.clone()),
            ChangeKind::AppRemoved { name } => b.apps_removed.push(name.clone()),
            ChangeKind::AppUpdated { name, .. } => b.apps_updated.push(name.clone()),
            _ => {}
        }
    }

    let (scanner, package, rating) = (&b.scanner, &b.content_package, &b.advisory_severity);
    let loud = [
        counted(&b.detections, &|t| format!("{} reported a detection: {}", scanner, t), &|n| {
            format!("{} reported {} detections", scanner, n)
        }),
        counted(
            &b.content,
            &|f| match package.is_empty() {
                true => format!("the packaged file {} no longer matches what was installed", f),
                false => format!("{}, from the {} package, no longer matches what was installed", f, package),
            },
            &|n| format!("{} packaged files no longer match what was installed", n),
        ),
        counted(&b.missing, &|f| format!("the packaged file {} is missing", f), &|n| {
            format!("{} packaged files are missing", n)
        }),
        counted(&b.permissions, &|f| format!("permissions changed on the packaged file {}", f), &|n| {
            format!("permissions changed on {} packaged files", n)
        }),
        counted(
            &b.advisories,
            &|p| format!("{} reported a new {}-risk advisory for {}", scanner, rating, p),
            &|n| format!("{} reported new high or critical advisories for {} packages", scanner, n),
        ),
        counted(&b.apps_new, &|a| format!("{} was installed", a), &|n| format!("{} new apps were installed", n)),
        counted(&b.apps_back, &|a| format!("{} is back after being removed", a), &|n| {
            format!("{} apps are back after being removed", n)
        }),
    ];
    let quiet = [
        counted(&b.config, &|f| format!("{} differs from its packaged version", f), &|n| {
            format!("{} configuration files differ from their packaged versions", n)
        }),
        counted(&b.touched, &|f| format!("the timestamp changed on {}", f), &|n| {
            format!("timestamps changed on {} packaged files", n)
        }),
        counted(&b.restored, &|f| format!("{} matches its package again", f), &|n| {
            format!("{} files match their packages again", n)
        }),
        counted(
            &b.lesser_advisories,
            &|p| format!("{} listed a lower-rated advisory for {}", scanner, p),
            &|n| format!("{} listed lower-rated advisories for {} packages", scanner, n),
        ),
        counted(&b.resolved, &|p| format!("the advisory for {} no longer applies", p), &|n| {
            format!("advisories for {} packages no longer apply", n)
        }),
        counted(&b.apps_removed, &|a| format!("{} was removed", a), &|n| format!("{} apps were removed", n)),
        counted(&b.apps_updated, &|a| format!("{} was updated", a), &|n| format!("{} apps were updated", n)),
    ];

    Clauses {
        loud: loud.into_iter().flatten().collect(),
        quiet: quiet.into_iter().flatten().collect(),
    }
}

#[cfg(test)]
mod tests {
    use crate::sentinel::change::{Change, ChangeKind};
    use crate::sentinel::summary::{describe_privilege, summarize};
    use chrono::Utc;

    fn change(kind: ChangeKind, source: &str) -> Change {
        Change::new(kind, source, Utc::now())
    }

    fn advisory(package: &str, severity: &str) -> Change {
        let kind = ChangeKind::AdvisoryAdded {
            scanner: "arch-audit".into(),
            package: package.into(),
            severity: severity.into(),
            ids: "CVE-1".into(),
            fix_available: false,
        };
        change(kind, "advisories")
    }

    fn file(path: &str, package: &str, what: &str, config: bool) -> Change {
        let kind = ChangeKind::FileAltered {
            path: path.into(),
            package: package.into(),
            what: what.into(),
            config,
        };
        change(kind, "integrity")
    }

    /// Reported as the scanner's finding, in the scanner's words.
    #[test]
    fn advisories_are_attributed_to_the_scanner() {
        let one = summarize(&[advisory("pkg-a", "high")]).unwrap();
        assert_eq!(one, "Today, arch-audit reported a new high-risk advisory for pkg-a.");

        let two = summarize(&[advisory("pkg-a", "high"), advisory("pkg-b", "critical"), advisory("pkg-c", "low")]).unwrap();
        assert_eq!(two, "Today, arch-audit reported new high or critical advisories for 2 packages: pkg-a and pkg-b.");
    }

    #[test]
    fn low_rated_advisories_wait_to_be_asked() {
        let group = [advisory("pkg-c", "low")];
        assert!(summarize(&group).is_none());
        assert_eq!(describe_privilege(&group).unwrap(), "Today, arch-audit listed a lower-rated advisory for pkg-c.");
    }

    #[test]
    fn changed_files_are_named_without_a_verdict() {
        let line = summarize(&[file("/usr/bin/example-tool", "example-pkg", "content", false)]).unwrap();
        assert_eq!(line, "Today, example-tool, from the example-pkg package, no longer matches what was installed.");
        for word in ["malware", "tamper", "infect", "compromis", "attack"] {
            assert!(!line.contains(word), "{}", line);
        }
    }

    /// An edited config file is never volunteered, only described when asked.
    #[test]
    fn edited_config_files_wait_to_be_asked() {
        let group = [file("/etc/example.conf", "example-pkg", "content", true)];
        assert!(summarize(&group).is_none());
        assert_eq!(describe_privilege(&group).unwrap(), "Today, example.conf differs from its packaged version.");
    }

    #[test]
    fn a_detection_is_reported_as_the_scanners_own() {
        let kind = ChangeKind::ScannerDetection {
            scanner: "Microsoft Defender".into(),
            threat: "Example.Threat".into(),
            resource: "file:_C:\\x.exe".into(),
        };
        let line = summarize(&[change(kind, "defender")]).unwrap();
        assert_eq!(line, "Today, Microsoft Defender reported a detection: Example.Threat.");
    }
}