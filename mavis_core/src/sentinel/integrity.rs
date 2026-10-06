// mavis_core/src/sentinel/integrity.rs
// Phase 8.5 step 4: do packaged files still match their packages?
// Parses `pacman -Qkk`, `rpm -Va` and `dpkg --verify`. Text in, findings out.

use super::change::{Change, ChangeKind};
use super::checks::Snapshot;
use chrono::{DateTime, Utc};

pub const SOURCE: &str = "integrity";

// What differs, strongest first. A file reported several ways keeps the strongest.
const CONTENT: &str = "content";
const MISSING: &str = "missing";
const PERMISSIONS: &str = "permissions";
const TIMESTAMP: &str = "timestamp";

fn rank(what: &str) -> u8 {
    match what {
        CONTENT => 3,
        MISSING => 2,
        PERMISSIONS => 1,
        _ => 0,
    }
}

/// path -> "what\tconfig\tpackage", keeping the strongest finding per path.
fn note(snap: &mut Snapshot, path: &str, what: &str, config: bool, package: &str) {
    let stronger = snap
        .get(path)
        .and_then(|d| d.split('\t').next())
        .is_none_or(|old| rank(what) > rank(old));
    if stronger {
        snap.insert(path.to_string(), format!("{}\t{}\t{}", what, config as u8, package));
    }
}

/// `pacman -Qkk`, stdout and stderr together, run with LC_ALL=C.
/// None unless a per-package summary line is present: without one the
/// run didn't finish, and an empty result would read as "all fixed".
pub fn parse_pacman_qkk(output: &str) -> Option<Snapshot> {
    let mut snap = Snapshot::new();
    let mut finished = false;
    for line in output.lines() {
        let (config, rest) = if let Some(r) = line.strip_prefix("backup file: ") {
            (true, r)
        } else if let Some(r) = line.strip_prefix("warning: ") {
            (false, r)
        } else {
            finished |= line.contains(" total files, ");
            continue;
        };
        let Some((package, rest)) = rest.split_once(": ") else { continue };
        let Some((path, reason)) = rest.rsplit_once(" (") else { continue };
        let Some(what) = pacman_reason(reason.trim_end_matches(')')) else { continue };
        note(&mut snap, path, what, config, package);
    }
    finished.then_some(snap)
}

/// None for anything that isn't a finding — notably "Permission denied",
/// which only means MAVIS isn't root.
fn pacman_reason(reason: &str) -> Option<&'static str> {
    match reason {
        "No such file or directory" => Some(MISSING),
        "Size mismatch" | "File type mismatch" | "Symlink path mismatch" => Some(CONTENT),
        "UID mismatch" | "GID mismatch" | "Permissions mismatch" => Some(PERMISSIONS),
        "Modification time mismatch" => Some(TIMESTAMP),
        r if r.ends_with("checksum mismatch") => Some(CONTENT),
        _ => None,
    }
}

/// `rpm -Va` and `dpkg --verify` share a format: nine flag characters (or
/// "missing"), an optional attribute such as `c` for config, then the path.
pub fn parse_verify(output: &str) -> Snapshot {
    let mut snap = Snapshot::new();
    for line in output.lines() {
        let Some(at) = line.find(" /") else { continue };
        let (head, path) = (&line[..at], &line[at + 1..]);
        if path.ends_with("(Permission denied)") {
            continue;
        }
        let mut fields = head.split_whitespace();
        let Some(flags) = fields.next() else { continue };
        let config = fields.next() == Some("c");
        let Some(what) = verify_flags(flags) else { continue };
        note(&mut snap, path, what, config, "");
    }
    snap
}

/// S size, 5 digest, L link, D device; M mode, U user, G group, P caps;
/// T mtime. '.' passed and '?' couldn't be checked.
fn verify_flags(flags: &str) -> Option<&'static str> {
    if flags == "missing" {
        return Some(MISSING);
    }
    if flags.len() != 9 || !flags.chars().all(|c| "SM5DLUGTP.?".contains(c)) {
        return None;
    }
    let has = |set: &str| flags.chars().any(|c| set.contains(c));
    if has("S5LD") {
        Some(CONTENT)
    } else if has("MUGP") {
        Some(PERMISSIONS)
    } else if has("T") {
        Some(TIMESTAMP)
    } else {
        None
    }
}

/// New or worsened findings, and files that match again.
pub fn diff(old: &Snapshot, new: &Snapshot, at: DateTime<Utc>) -> Vec<Change> {
    let mut kinds = Vec::new();
    for (path, detail) in new {
        if old.get(path) == Some(detail) {
            continue;
        }
        let mut f = detail.split('\t');
        kinds.push(ChangeKind::FileAltered {
            path: path.clone(),
            what: f.next().unwrap_or(TIMESTAMP).to_string(),
            config: f.next() == Some("1"),
            package: f.next().unwrap_or("").to_string(),
        });
    }
    for path in old.keys().filter(|p| !new.contains_key(*p)) {
        kinds.push(ChangeKind::FileRestored { path: path.clone() });
    }
    kinds.into_iter().map(|k| Change::new(k, SOURCE, at)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sentinel::change::Severity;

    // Real lines from pacman 6.0.2 against a crafted package, run without
    // root (2026-10-06). Only the test root's prefix is cut from the paths.
    const PACMAN: &str = "\
backup file: testpkg: /etc/hello.conf (Size mismatch)
backup file: testpkg: /etc/hello.conf (MD5 checksum mismatch)
backup file: testpkg: /etc/hello.conf (SHA256 checksum mismatch)
testpkg: 13 total files, 7 altered files
otherpkg: no mtree file
warning: testpkg: /usr/bin/hello (Size mismatch)
warning: testpkg: /usr/bin/hello (MD5 checksum mismatch)
warning: testpkg: /usr/bin/hello (SHA256 checksum mismatch)
warning: testpkg: /usr/lib/gone (No such file or directory)
warning: testpkg: /usr/lib/perm (Permissions mismatch)
warning: testpkg: /usr/lib/samesize (MD5 checksum mismatch)
warning: testpkg: /usr/lib/samesize (SHA256 checksum mismatch)
warning: testpkg: /usr/lib/secret (Permissions mismatch)
warning: testpkg: /usr/lib/secret/key (Permission denied)
warning: testpkg: /usr/lib/touched (Modification time mismatch)
";

    // Real output of rpm 4.18.2 against a crafted package (2026-10-06).
    const RPM: &str = "\
S.5....T.  c /etc/hello.conf
S.5....T.    /usr/bin/hello
missing     /usr/lib/t/gone
.....U...    /usr/lib/t/lib
.M.......    /usr/lib/t/perm
.......T.    /usr/lib/t/touched
";

    // Real lines from dpkg 1.22.6 --verify (2026-10-06).
    const DPKG: &str = "\
??5??????   /usr/bin/debsums
??5?????? c /etc/default/debsums
missing     /usr/share/doc/debsums/copyright
";

    fn at() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).unwrap()
    }

    #[test]
    fn pacman_findings_are_classified() {
        let s = parse_pacman_qkk(PACMAN).unwrap();
        assert_eq!(s["/usr/bin/hello"], "content\t0\ttestpkg");
        assert_eq!(s["/usr/lib/samesize"], "content\t0\ttestpkg", "same size, different checksum");
        assert_eq!(s["/etc/hello.conf"], "content\t1\ttestpkg", "backup files are config");
        assert_eq!(s["/usr/lib/gone"], "missing\t0\ttestpkg");
        assert_eq!(s["/usr/lib/perm"], "permissions\t0\ttestpkg");
        assert_eq!(s["/usr/lib/touched"], "timestamp\t0\ttestpkg");
        assert!(!s.contains_key("/usr/lib/secret/key"), "permission denied is not a finding");
        assert_eq!(s.len(), 7);
    }

    /// A run that was cut short must not look like a clean machine.
    #[test]
    fn an_unfinished_pacman_run_is_not_a_result() {
        assert!(parse_pacman_qkk("").is_none());
        assert!(parse_pacman_qkk("warning: a: /usr/bin/a (Size mismatch)\n").is_none());
        assert_eq!(parse_pacman_qkk("a: 3 total files, 0 altered files\n"), Some(Snapshot::new()));
    }

    #[test]
    fn rpm_and_dpkg_share_a_parser() {
        let rpm = parse_verify(RPM);
        assert_eq!(rpm["/usr/bin/hello"], "content\t0\t");
        assert_eq!(rpm["/etc/hello.conf"], "content\t1\t");
        assert_eq!(rpm["/usr/lib/t/gone"], "missing\t0\t");
        assert_eq!(rpm["/usr/lib/t/lib"], "permissions\t0\t");
        assert_eq!(rpm["/usr/lib/t/perm"], "permissions\t0\t");
        assert_eq!(rpm["/usr/lib/t/touched"], "timestamp\t0\t");

        let dpkg = parse_verify(DPKG);
        assert_eq!(dpkg["/usr/bin/debsums"], "content\t0\t");
        assert_eq!(dpkg["/etc/default/debsums"], "content\t1\t");
        assert_eq!(dpkg["/usr/share/doc/debsums/copyright"], "missing\t0\t");
    }

    #[test]
    fn verify_ignores_what_it_cannot_read() {
        let text = "\
missing     /root/x (Permission denied)
..?......    /etc/shadow
error: cannot open Packages database in /var/lib/rpm
";
        assert!(parse_verify(text).is_empty());
    }

    #[test]
    fn a_changed_binary_is_critical_and_an_edited_config_is_not() {
        let changes = diff(&Snapshot::new(), &parse_pacman_qkk(PACMAN).unwrap(), at());
        let severity = |path: &str| {
            changes
                .iter()
                .find(|c| matches!(&c.kind, ChangeKind::FileAltered { path: p, .. } if p == path))
                .map(|c| c.severity)
        };
        assert_eq!(severity("/usr/bin/hello"), Some(Severity::Critical));
        assert_eq!(severity("/etc/hello.conf"), Some(Severity::Routine));
        assert_eq!(severity("/usr/lib/gone"), Some(Severity::Notable));
        assert_eq!(severity("/usr/lib/touched"), Some(Severity::Routine));
    }

    #[test]
    fn unchanged_findings_are_not_repeated_and_fixes_are_noted() {
        let before = parse_pacman_qkk(PACMAN).unwrap();
        assert!(diff(&before, &before, at()).is_empty());

        let mut after = before.clone();
        after.remove("/usr/bin/hello");
        let changes = diff(&before, &after, at());
        assert_eq!(changes.len(), 1);
        assert!(matches!(&changes[0].kind, ChangeKind::FileRestored { path } if path == "/usr/bin/hello"));
    }
}