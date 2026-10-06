// mavis_core/src/sentinel/advisories.rs
// Phase 8.5 step 4: what the distribution's own security scanner says.
// Parses `arch-audit` and `debsecan`. Their findings are reported as
// theirs; MAVIS adds no judgement of its own.

use super::change::{Change, ChangeKind};
use super::checks::Snapshot;
use chrono::{DateTime, Utc};

pub const SOURCE: &str = "advisories";

/// Passed to `arch-audit --format`: name, severity, fixed version, CVEs.
pub const ARCH_AUDIT_FORMAT: &str = "%n\t%s\t%v\t%c";

/// package -> "severity\tfix\tids", from `arch-audit --format`.
pub fn parse_arch_audit(output: &str) -> Snapshot {
    output
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            let package = f.first().filter(|p| !p.is_empty())?;
            // "High risk" -> "high"
            let severity = f.get(1)?.trim_end_matches(" risk").to_lowercase();
            let fix = !f.get(2)?.is_empty();
            let ids = f.get(3)?.trim();
            Some((package.to_string(), format!("{}\t{}\t{}", severity, fix as u8, ids)))
        })
        .collect()
}

/// "package\tCVE" -> "severity\tfix\tCVE", from debsecan's default output:
/// `CVE-2024-0001 bash (fixed, remotely exploitable, high urgency)`.
pub fn parse_debsecan(output: &str) -> Snapshot {
    output
        .lines()
        .filter_map(|line| {
            let (id, rest) = line.split_once(' ')?;
            let (package, notes) = match rest.split_once(" (") {
                Some((p, n)) => (p, n.trim_end_matches(')')),
                None => (rest, ""),
            };
            if package.is_empty() || package.contains(' ') {
                return None;
            }
            let mut severity = "";
            let mut fix = false;
            for item in notes.split(", ") {
                fix |= item == "fixed";
                if let Some(urgency) = item.strip_suffix(" urgency") {
                    severity = urgency;
                }
            }
            Some((format!("{}\t{}", package, id), format!("{}\t{}\t{}", severity, fix as u8, id)))
        })
        .collect()
}

/// New advisories, and ones that no longer apply. An entry counts as new
/// when its IDs change; a fix becoming available is not re-announced.
pub fn diff(scanner: &str, old: &Snapshot, new: &Snapshot, at: DateTime<Utc>) -> Vec<Change> {
    let package = |key: &str| key.split('\t').next().unwrap_or(key).to_string();
    let ids = |detail: &str| detail.split('\t').nth(2).unwrap_or("").to_string();

    let mut kinds = Vec::new();
    for (key, detail) in new {
        if old.get(key).is_some_and(|o| ids(o) == ids(detail)) {
            continue;
        }
        let mut f = detail.split('\t');
        kinds.push(ChangeKind::AdvisoryAdded {
            scanner: scanner.to_string(),
            package: package(key),
            severity: f.next().unwrap_or("").to_string(),
            fix_available: f.next() == Some("1"),
            ids: f.next().unwrap_or("").to_string(),
        });
    }
    for key in old.keys().filter(|k| !new.contains_key(*k)) {
        kinds.push(ChangeKind::AdvisoryResolved {
            scanner: scanner.to_string(),
            package: package(key),
        });
    }
    kinds.into_iter().map(|k| Change::new(k, SOURCE, at)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sentinel::change::Severity;

    // Real output of arch-audit 0.1.20 with ARCH_AUDIT_FORMAT (2026-10-06).
    const ARCH_AUDIT: &str = "\
testpkg\tHigh risk\t\tCVE-2024-0001, CVE-2024-0002
otherpkg\tLow risk\t2.1-1\tCVE-2024-0003
";

    // Real output of debsecan 0.4.20.1 (2026-10-06).
    const DEBSECAN: &str = "\
CVE-2024-0001 bash (fixed, remotely exploitable, high urgency)
CVE-2024-0002 bash (low urgency)
CVE-2024-0003 coreutils (medium urgency)
CVE-2024-0004 zlib1g
";

    fn at() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).unwrap()
    }

    #[test]
    fn arch_audit_lines_are_read() {
        let s = parse_arch_audit(ARCH_AUDIT);
        assert_eq!(s["testpkg"], "high\t0\tCVE-2024-0001, CVE-2024-0002");
        assert_eq!(s["otherpkg"], "low\t1\tCVE-2024-0003");
        assert!(parse_arch_audit("").is_empty(), "a clean system prints nothing");
    }

    #[test]
    fn debsecan_lines_are_read_per_cve() {
        let s = parse_debsecan(DEBSECAN);
        assert_eq!(s["bash\tCVE-2024-0001"], "high\t1\tCVE-2024-0001");
        assert_eq!(s["bash\tCVE-2024-0002"], "low\t0\tCVE-2024-0002");
        assert_eq!(s["zlib1g\tCVE-2024-0004"], "\t0\tCVE-2024-0004", "no notes at all");
        assert_eq!(s.len(), 4);
    }

    /// The scanner's rating decides how loud it is, by a fixed rule.
    #[test]
    fn only_high_and_critical_advisories_are_spoken() {
        let changes = diff("arch-audit", &Snapshot::new(), &parse_arch_audit(ARCH_AUDIT), at());
        assert_eq!(changes.len(), 2);
        let by_package = |name: &str| {
            changes
                .iter()
                .find(|c| matches!(&c.kind, ChangeKind::AdvisoryAdded { package, .. } if package == name))
                .unwrap()
        };
        assert_eq!(by_package("testpkg").severity, Severity::Notable);
        assert_eq!(by_package("otherpkg").severity, Severity::Routine);
        assert!(by_package("testpkg").detail.starts_with("arch-audit reports"), "reported as the scanner's");
    }

    #[test]
    fn a_fix_becoming_available_is_not_a_new_advisory() {
        let before = parse_arch_audit("pkg\tHigh risk\t\tCVE-1\n");
        let after = parse_arch_audit("pkg\tHigh risk\t2.0-1\tCVE-1\n");
        assert!(diff("arch-audit", &before, &after, at()).is_empty());

        let more = parse_arch_audit("pkg\tHigh risk\t\tCVE-1, CVE-2\n");
        assert_eq!(diff("arch-audit", &before, &more, at()).len(), 1, "a new CVE is");
    }

    #[test]
    fn resolved_advisories_are_routine() {
        let before = parse_arch_audit(ARCH_AUDIT);
        let changes = diff("arch-audit", &before, &Snapshot::new(), at());
        assert_eq!(changes.len(), 2);
        assert!(changes.iter().all(|c| c.severity == Severity::Routine));
    }
}