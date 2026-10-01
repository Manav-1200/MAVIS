// mavis_core/src/sentinel/privilege.rs
// Phase 8.5 step 3: who can do what on this machine.
// Each surface is read into a snapshot (item -> detail) and diffed against
// the last one. Parsers and the diff are pure; only `collect_*` touch disk.

use super::change::{Change, ChangeKind};
use chrono::{DateTime, Utc};
use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub type Snapshot = BTreeMap<String, String>;

pub const USERS: &str = "users";
pub const GROUPS: &str = "groups";
pub const SSH_KEYS: &str = "ssh_keys";
pub const SUDOERS: &str = "sudoers";
pub const SYSTEMD: &str = "systemd";
pub const SETUID: &str = "setuid";

/// Where each surface lives. `system()` for real use; tests point it at a
/// temp directory.
#[derive(Clone, Debug)]
pub struct Surfaces {
    pub passwd: PathBuf,
    pub group: PathBuf,
    pub login_defs: PathBuf,
    pub authorized_keys: Vec<PathBuf>,
    pub sudoers: Vec<PathBuf>,
    /// (scope, directory holding `*.wants` / `*.requires`)
    pub systemd: Vec<(String, PathBuf)>,
    pub setuid_roots: Vec<PathBuf>,
}

impl Surfaces {
    pub fn system() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|h| h.join(".config")));

        let mut authorized_keys = Vec::new();
        if let Some(h) = &home {
            authorized_keys.push(h.join(".ssh/authorized_keys"));
            authorized_keys.push(h.join(".ssh/authorized_keys2"));
        }
        let mut systemd = vec![
            ("system".to_string(), PathBuf::from("/etc/systemd/system")),
            ("all users".to_string(), PathBuf::from("/etc/systemd/user")),
        ];
        if let Some(c) = config {
            systemd.push(("your user".to_string(), c.join("systemd/user")));
        }

        Self {
            passwd: "/etc/passwd".into(),
            group: "/etc/group".into(),
            login_defs: "/etc/login.defs".into(),
            authorized_keys,
            sudoers: vec!["/etc/sudoers".into(), "/etc/sudoers.d".into()],
            systemd,
            // /bin, /sbin and /usr/sbin are often symlinks into these; not followed.
            setuid_roots: ["/usr/bin", "/usr/sbin", "/usr/lib", "/usr/libexec", "/usr/local", "/opt"]
                .iter()
                .map(PathBuf::from)
                .collect(),
        }
    }
}

/// Read every surface. A surface that can't be read is left out, never
/// returned empty — an empty snapshot would read as "everything removed",
/// and the next good read as a flood of Critical additions.
pub fn collect_all(s: &Surfaces, include_setuid: bool) -> Vec<(&'static str, Snapshot)> {
    let uid_min = std::fs::read_to_string(&s.login_defs)
        .map(|t| uid_min(&t))
        .unwrap_or(1000);
    let mut out = Vec::new();
    if let Ok(t) = std::fs::read_to_string(&s.passwd) {
        out.push((USERS, parse_passwd(&t, uid_min)));
    }
    if let Ok(t) = std::fs::read_to_string(&s.group) {
        out.push((GROUPS, parse_group(&t)));
    }
    if let Some(snap) = collect_authorized_keys(&s.authorized_keys) {
        out.push((SSH_KEYS, snap));
    }
    out.push((SUDOERS, collect_sudoers(&s.sudoers)));
    out.push((SYSTEMD, collect_systemd(&s.systemd)));
    if include_setuid {
        out.push((SETUID, collect_setuid(&s.setuid_roots)));
    }
    out
}

// ---------------------------------------------------------------------
// Parsers
// ---------------------------------------------------------------------

/// UID_MIN from login.defs; 1000 if absent.
pub fn uid_min(login_defs: &str) -> u32 {
    login_defs
        .lines()
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            (parts.next()? == "UID_MIN").then(|| parts.next()?.parse().ok())?
        })
        .next()
        .unwrap_or(1000)
}

/// name -> "uid:login" where login is 1 for a human account.
pub fn parse_passwd(text: &str, uid_min: u32) -> Snapshot {
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            let uid: u32 = f.get(2)?.parse().ok()?;
            let shell = f.get(6).copied().unwrap_or("");
            let real_shell = !(shell.is_empty() || shell.ends_with("nologin") || shell.ends_with("false"));
            let login = uid >= uid_min && uid != 65534 && real_shell;
            Some((f[0].to_string(), format!("{}:{}", uid, login as u8)))
        })
        .collect()
}

/// group -> comma-separated, sorted members.
pub fn parse_group(text: &str) -> Snapshot {
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            let members: BTreeSet<&str> = f
                .get(3)?
                .split(',')
                .map(str::trim)
                .filter(|m| !m.is_empty())
                .collect();
            Some((f[0].to_string(), members.into_iter().collect::<Vec<_>>().join(",")))
        })
        .collect()
}

/// "file\tkey blob" -> label. Options before the key type are skipped.
pub fn parse_authorized_keys(file: &str, text: &str) -> Snapshot {
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .filter_map(|l| {
            let tokens: Vec<&str> = l.split_whitespace().collect();
            let at = tokens.iter().position(|t| is_key_type(t))?;
            let blob = tokens.get(at + 1)?;
            let comment = tokens[at + 2..].join(" ");
            let label = if comment.is_empty() { tokens[at].to_string() } else { comment };
            Some((format!("{}\t{}", file, blob), label))
        })
        .collect()
}

/// Key type prefixes from sshd(8)'s AUTHORIZED_KEYS FILE FORMAT.
fn is_key_type(token: &str) -> bool {
    token.starts_with("ssh-") || token.starts_with("ecdsa-sha2-") || token.starts_with("sk-")
}

// ---------------------------------------------------------------------
// Collectors
// ---------------------------------------------------------------------

/// None if a file exists but can't be read. A missing file is no keys.
fn collect_authorized_keys(files: &[PathBuf]) -> Option<Snapshot> {
    let mut snap = Snapshot::new();
    for path in files {
        match std::fs::read_to_string(path) {
            Ok(text) => snap.extend(parse_authorized_keys(&display_name(path), &text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return None,
        }
    }
    Some(snap)
}

/// Contents are root-only, so changes are seen through metadata: size,
/// mtime and inode of each file, or of the directory if it can't be listed.
fn collect_sudoers(paths: &[PathBuf]) -> Snapshot {
    let mut snap = Snapshot::new();
    for path in paths {
        let Ok(meta) = std::fs::metadata(path) else { continue };
        let mut entries = Vec::new();
        if meta.is_dir() {
            if let Ok(dir) = std::fs::read_dir(path) {
                entries.extend(dir.flatten().map(|e| e.path()));
            }
        }
        if entries.is_empty() {
            entries.push(path.clone());
        }
        for entry in entries {
            if let Ok(m) = std::fs::metadata(&entry) {
                let stamp = format!("{}:{}:{}", m.len(), m.mtime(), m.ino());
                snap.insert(entry.display().to_string(), stamp);
            }
        }
    }
    snap
}

/// "scope\tunit" -> the targets that pull it in.
fn collect_systemd(dirs: &[(String, PathBuf)]) -> Snapshot {
    let mut units: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (scope, dir) in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        for target in entries.flatten() {
            let target_name = target.file_name().to_string_lossy().to_string();
            if !(target_name.ends_with(".wants") || target_name.ends_with(".requires")) {
                continue;
            }
            let Ok(links) = std::fs::read_dir(target.path()) else { continue };
            for link in links.flatten() {
                let unit = link.file_name().to_string_lossy().to_string();
                units
                    .entry(format!("{}\t{}", scope, unit))
                    .or_default()
                    .insert(target_name.clone());
            }
        }
    }
    units
        .into_iter()
        .map(|(k, v)| (k, v.into_iter().collect::<Vec<_>>().join(",")))
        .collect()
}

/// path -> "u|g:owner uid" for every setuid/setgid regular file.
/// Symlinks are never followed, so nothing is visited twice.
pub fn collect_setuid(roots: &[PathBuf]) -> Snapshot {
    let mut snap = Snapshot::new();
    let mut stack: Vec<PathBuf> = roots.to_vec();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let Ok(meta) = entry.path().symlink_metadata() else { continue };
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                let mode = meta.permissions().mode();
                if mode & 0o6000 != 0 {
                    let kind = if mode & 0o4000 != 0 { "u" } else { "g" };
                    snap.insert(entry.path().display().to_string(), format!("{}:{}", kind, meta.uid()));
                }
            }
        }
    }
    snap
}

/// "~/.ssh/authorized_keys" reads better than a full home path.
fn display_name(path: &Path) -> String {
    let full = path.display().to_string();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => match full.strip_prefix(&home) {
            Some(rest) => format!("~{}", rest),
            None => full,
        },
        _ => full,
    }
}

// ---------------------------------------------------------------------
// Diff
// ---------------------------------------------------------------------

/// Changes between two snapshots of the same surface.
pub fn diff(source: &str, old: &Snapshot, new: &Snapshot, at: DateTime<Utc>) -> Vec<Change> {
    let added = new.iter().filter(|(k, _)| !old.contains_key(*k));
    let removed = old.iter().filter(|(k, _)| !new.contains_key(*k));
    let changed = new
        .iter()
        .filter_map(|(k, v)| old.get(k).filter(|o| *o != v).map(|o| (k, o, v)));

    let mut kinds: Vec<ChangeKind> = Vec::new();
    match source {
        USERS => {
            for (name, v) in added {
                let (uid, login) = user_fields(v);
                kinds.push(ChangeKind::UserAdded { name: name.clone(), uid, login });
            }
            for (name, _) in removed {
                kinds.push(ChangeKind::UserRemoved { name: name.clone() });
            }
            for (name, before, after) in changed {
                let (from, to) = (user_fields(before).0, user_fields(after).0);
                if from != to {
                    kinds.push(ChangeKind::UserUidChanged { name: name.clone(), from, to });
                }
            }
        }
        GROUPS => {
            for (name, members) in added {
                kinds.push(ChangeKind::GroupAdded { name: name.clone() });
                // Members of a brand-new group are as new as the group.
                for user in split_members(members) {
                    kinds.push(ChangeKind::GroupMemberAdded { group: name.clone(), user });
                }
            }
            for (name, _) in removed {
                kinds.push(ChangeKind::GroupRemoved { name: name.clone() });
            }
            for (group, before, after) in changed {
                let (b, a) = (split_members(before), split_members(after));
                for user in a.difference(&b) {
                    kinds.push(ChangeKind::GroupMemberAdded { group: group.clone(), user: user.clone() });
                }
                for user in b.difference(&a) {
                    kinds.push(ChangeKind::GroupMemberRemoved { group: group.clone(), user: user.clone() });
                }
            }
        }
        SSH_KEYS => {
            for (key, label) in added {
                kinds.push(ChangeKind::SshKeyAdded { file: key_file(key), label: label.clone() });
            }
            for (key, label) in removed {
                kinds.push(ChangeKind::SshKeyRemoved { file: key_file(key), label: label.clone() });
            }
        }
        SUDOERS => {
            // Any addition, removal or metadata change is a change to sudo's rules.
            let paths: BTreeSet<&String> = added
                .map(|(k, _)| k)
                .chain(removed.map(|(k, _)| k))
                .chain(changed.map(|(k, _, _)| k))
                .collect();
            for path in paths {
                kinds.push(ChangeKind::SudoersChanged { path: path.clone() });
            }
        }
        SYSTEMD => {
            for (key, _) in added {
                let (scope, unit) = split_unit(key);
                kinds.push(ChangeKind::UnitEnabled { unit, scope });
            }
            for (key, _) in removed {
                let (scope, unit) = split_unit(key);
                kinds.push(ChangeKind::UnitDisabled { unit, scope });
            }
        }
        SETUID => {
            // A file that only switches setuid <-> setgid counts as new.
            let fresh = added.chain(changed.map(|(k, _, v)| (k, v)));
            for (path, v) in fresh {
                let (kind, owner) = v.split_once(':').unwrap_or(("u", ""));
                kinds.push(ChangeKind::SetuidAdded {
                    path: path.clone(),
                    setuid: kind == "u",
                    root_owned: owner == "0",
                });
            }
            for (path, _) in removed {
                kinds.push(ChangeKind::SetuidRemoved { path: path.clone() });
            }
        }
        _ => {}
    }
    kinds.into_iter().map(|k| Change::new(k, source, at)).collect()
}

fn user_fields(v: &str) -> (u32, bool) {
    let (uid, login) = v.split_once(':').unwrap_or((v, "0"));
    (uid.parse().unwrap_or(u32::MAX), login == "1")
}

fn split_members(v: &str) -> BTreeSet<String> {
    v.split(',').filter(|m| !m.is_empty()).map(String::from).collect()
}

fn key_file(key: &str) -> String {
    key.split_once('\t').map(|(f, _)| f).unwrap_or(key).to_string()
}

fn split_unit(key: &str) -> (String, String) {
    let (scope, unit) = key.split_once('\t').unwrap_or(("system", key));
    (scope.to_string(), unit.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sentinel::change::Severity;

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).unwrap()
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mavis_priv_{}_{}_{}",
            tag,
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const PASSWD: &str = "\
root:x:0:0:root:/root:/bin/bash
polkitd:x:977:977:polkit:/:/usr/bin/nologin
manav:x:1000:1000::/home/manav:/usr/bin/zsh
nobody:x:65534:65534:Nobody:/:/usr/bin/nologin
";

    #[test]
    fn passwd_marks_only_human_accounts_as_login() {
        let s = parse_passwd(PASSWD, 1000);
        assert_eq!(s["root"], "0:0");
        assert_eq!(s["polkitd"], "977:0");
        assert_eq!(s["manav"], "1000:1");
        assert_eq!(s["nobody"], "65534:0");
    }

    #[test]
    fn uid_min_is_read_from_login_defs() {
        assert_eq!(uid_min("# comment\nUID_MIN\t\t\t 500\nUID_MAX 60000\n"), 500);
        assert_eq!(uid_min(""), 1000);
    }

    #[test]
    fn group_members_are_sorted_and_trimmed() {
        let s = parse_group("wheel:x:998:manav, alice\nvideo:x:985:\n");
        assert_eq!(s["wheel"], "alice,manav");
        assert_eq!(s["video"], "");
    }

    #[test]
    fn authorized_keys_skip_options_and_comments() {
        let text = "\
# a comment
ssh-ed25519 AAAAC3Nza1 manav@laptop
command=\"/bin/true\",no-pty ssh-rsa AAAAB3Nza2
garbage line
";
        let s = parse_authorized_keys("~/.ssh/authorized_keys", text);
        assert_eq!(s.len(), 2);
        assert_eq!(s["~/.ssh/authorized_keys\tAAAAC3Nza1"], "manav@laptop");
        assert_eq!(s["~/.ssh/authorized_keys\tAAAAB3Nza2"], "ssh-rsa", "no comment: type is the label");
    }

    #[test]
    fn a_new_root_account_is_critical() {
        let old = parse_passwd(PASSWD, 1000);
        let mut new = old.clone();
        new.insert("toor".into(), "0:0".into());
        let changes = diff(USERS, &old, &new, now());
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].severity, Severity::Critical);
    }

    #[test]
    fn a_uid_changed_to_zero_is_critical() {
        let old = parse_passwd(PASSWD, 1000);
        let mut new = old.clone();
        new.insert("manav".into(), "0:1".into());
        let changes = diff(USERS, &old, &new, now());
        assert!(matches!(changes[0].kind, ChangeKind::UserUidChanged { to: 0, .. }));
        assert_eq!(changes[0].severity, Severity::Critical);
    }

    #[test]
    fn joining_wheel_is_critical_and_new_groups_are_routine() {
        let old = parse_group("wheel:x:998:manav\n");
        let new = parse_group("wheel:x:998:manav,eve\nnewgroup:x:900:\n");
        let changes = diff(GROUPS, &old, &new, now());
        let worst = changes.iter().map(|c| c.severity).max();
        assert_eq!(worst, Some(Severity::Critical));
        assert!(changes
            .iter()
            .any(|c| matches!(&c.kind, ChangeKind::GroupAdded { name } if name == "newgroup")));
    }

    #[test]
    fn an_unchanged_surface_produces_nothing() {
        let s = parse_passwd(PASSWD, 1000);
        assert!(diff(USERS, &s, &s, now()).is_empty());
    }

    #[test]
    fn setuid_files_are_found_and_symlinks_are_not_followed() {
        let dir = temp_dir("setuid");
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let suid = bin.join("helper");
        let plain = bin.join("plain");
        std::fs::write(&suid, b"x").unwrap();
        std::fs::write(&plain, b"x").unwrap();
        std::fs::set_permissions(&suid, std::fs::Permissions::from_mode(0o4755)).unwrap();
        std::os::unix::fs::symlink(&bin, dir.join("loop")).unwrap();

        let snap = collect_setuid(std::slice::from_ref(&dir));
        assert_eq!(snap.len(), 1, "{:?}", snap);
        assert!(snap.keys().next().unwrap().ends_with("bin/helper"));
        assert!(snap.values().next().unwrap().starts_with("u:"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn enabled_units_are_read_from_wants_directories() {
        let dir = temp_dir("systemd");
        let wants = dir.join("multi-user.target.wants");
        std::fs::create_dir_all(&wants).unwrap();
        std::os::unix::fs::symlink("/usr/lib/systemd/system/sshd.service", wants.join("sshd.service")).unwrap();
        std::fs::write(dir.join("not-a-target.service"), b"").unwrap();

        let old = collect_systemd(&[("system".into(), dir.clone())]);
        assert_eq!(old.len(), 1);
        std::os::unix::fs::symlink("/x/evil.timer", wants.join("evil.timer")).unwrap();
        let new = collect_systemd(&[("system".into(), dir.clone())]);
        let changes = diff(SYSTEMD, &old, &new, now());
        assert_eq!(changes.len(), 1);
        assert!(matches!(&changes[0].kind, ChangeKind::UnitEnabled { unit, .. } if unit == "evil.timer"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_sudoers_drop_in_is_critical() {
        let dir = temp_dir("sudoers");
        let d = dir.join("sudoers.d");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("README"), b"x").unwrap();
        let old = collect_sudoers(std::slice::from_ref(&d));
        std::fs::write(d.join("90-me"), b"me ALL=(ALL) NOPASSWD: ALL").unwrap();
        let new = collect_sudoers(std::slice::from_ref(&d));
        let changes = diff(SUDOERS, &old, &new, now());
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].severity, Severity::Critical);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An unreadable surface is left out, not reported as empty.
    #[test]
    fn unreadable_surfaces_are_skipped_not_emptied() {
        let dir = temp_dir("skip");
        let s = Surfaces {
            passwd: dir.join("missing-passwd"),
            group: dir.join("missing-group"),
            login_defs: dir.join("missing-login.defs"),
            authorized_keys: vec![dir.join("no-keys")],
            sudoers: vec![],
            systemd: vec![],
            setuid_roots: vec![],
        };
        let sources: Vec<&str> = collect_all(&s, false).into_iter().map(|(n, _)| n).collect();
        assert!(!sources.contains(&USERS), "{:?}", sources);
        assert!(!sources.contains(&GROUPS));
        assert!(sources.contains(&SSH_KEYS), "a missing key file just means no keys");
        let _ = std::fs::remove_dir_all(&dir);
    }
}