// mavis_core/src/sentinel/change.rs
// What changed on the machine, and how much it matters.
//
// Severity is decided here by static rules, never by the LLM — the same
// rule the permission gate follows in safety/risk.rs, for the same reason.
// A model that misjudges a risk score runs something destructive; a model
// that misjudges a severity either cries wolf until the user stops
// listening, or stays quiet about the one change that mattered. Both
// failures are silent. Static rules are auditable and can't be talked
// around.
//
// An LLM may later *describe* a change in nicer words, or *raise* a
// severity as a second opinion. It must never lower one.
//
// Note what this module deliberately does NOT do: decide whether
// something is malware. Any heuristic for that would flag ordinary
// packages and miss anything built to evade, and — worse — it would imply
// an all-clear that MAVIS is in no position to give. Real scanner
// verdicts (Defender, ClamAV, XProtect) are reported as facts from those
// tools; MAVIS forms no opinion of its own.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// How loudly a change should be surfaced.
///
/// Ordering matters: `Routine < Notable < Critical`, so a batch of
/// changes can be reduced to its worst.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Severity {
    /// Expected consequence of an update. Recorded, never announced.
    Routine,
    /// Worth a glance. Spoken the next time the user talks to MAVIS.
    Notable,
    /// Affects who can do what on this machine. Notified immediately.
    Critical,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Routine => "routine",
            Severity::Notable => "notable",
            Severity::Critical => "critical",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "routine" => Some(Severity::Routine),
            "notable" => Some(Severity::Notable),
            "critical" => Some(Severity::Critical),
            _ => None,
        }
    }
}

/// A single observed change. Kinds are added as sources are built; the
/// severity rules for each live in `classify` below so they stay in one
/// readable place rather than scattered across the collectors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ChangeKind {
    /// A package appeared that was not installed before.
    ///
    /// `requested` distinguishes "I asked for this" from "this came along
    /// for the ride". The second case is the one that matters: it is how
    /// a Hyprland ends up on a machine that runs niri, because something
    /// else listed it as a dependency.
    PackageInstalled {
        name: String,
        version: String,
        requested: bool,
    },
    /// A package that was installed is now gone.
    PackageRemoved { name: String, version: String },
    /// An already-installed package changed version.
    PackageUpgraded {
        name: String,
        from: String,
        to: String,
    },
    /// An already-installed package went backwards in version. Unusual
    /// outside a deliberate downgrade, and worth mentioning because it is
    /// also what a supply-chain rollback attack looks like.
    PackageDowngraded {
        name: String,
        from: String,
        to: String,
    },

    // Step 3 — privilege surfaces. `login` marks a human account
    // (UID at or above UID_MIN, with a real shell).
    UserAdded { name: String, uid: u32, login: bool },
    UserRemoved { name: String },
    UserUidChanged { name: String, from: u32, to: u32 },
    GroupAdded { name: String },
    GroupRemoved { name: String },
    GroupMemberAdded { group: String, user: String },
    GroupMemberRemoved { group: String, user: String },
    /// `label` is the key's comment, or its type when it has none.
    SshKeyAdded { file: String, label: String },
    SshKeyRemoved { file: String, label: String },
    /// Content is root-only, so only the fact of a change is known.
    SudoersChanged { path: String },
    UnitEnabled { unit: String, scope: String },
    UnitDisabled { unit: String, scope: String },
    SetuidAdded { path: String, setuid: bool, root_owned: bool },
    SetuidRemoved { path: String },

    // Step 4 — integrity and advisories.
    /// A packaged file differs from what its package installed.
    /// `what` is "content", "missing", "permissions" or "timestamp".
    FileAltered { path: String, package: String, what: String, config: bool },
    FileRestored { path: String },
    /// A scanner's own finding. `severity` is the scanner's word, lowercased.
    AdvisoryAdded { scanner: String, package: String, severity: String, ids: String, fix_available: bool },
    AdvisoryResolved { scanner: String, package: String },

    // Step 5 — Windows and macOS. Autostart entries reuse UnitEnabled/Disabled.
    /// `returned` marks an app that was removed earlier and is back.
    AppAdded { name: String, version: String, returned: bool },
    AppRemoved { name: String },
    AppUpdated { name: String, from: String, to: String },
    ScannerDetection { scanner: String, threat: String, resource: String },
}

/// Groups whose members conventionally get root, or its equivalent.
pub const PRIVILEGED_GROUPS: &[&str] = &["root", "wheel", "sudo", "admin", "docker"];

/// One change, with everything needed to report it later.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub kind: ChangeKind,
    pub severity: Severity,
    /// Which collector saw it — "pacman", "dpkg", "rpm", …
    pub source: String,
    /// When the change happened, not when MAVIS noticed.
    pub occurred_at: DateTime<Utc>,
    /// One plain sentence, ready to be spoken or shown.
    pub detail: String,
}

impl Change {
    pub fn new(kind: ChangeKind, source: &str, occurred_at: DateTime<Utc>) -> Self {
        let severity = classify(&kind);
        let detail = describe(&kind);
        Self {
            kind,
            severity,
            source: source.to_string(),
            occurred_at,
            detail,
        }
    }

    /// Stable identity for a change, so re-reading a log that still
    /// contains entries MAVIS has already reported does not announce them
    /// twice. Deliberately includes the timestamp: the same package
    /// installed again later is a new event worth knowing about.
    pub fn fingerprint(&self) -> String {
        let (verb, name, detail) = identity(&self.kind);
        format!(
            "{}:{}:{}:{}:{}",
            self.source,
            self.occurred_at.timestamp(),
            verb,
            name,
            detail
        )
    }
}

/// Fingerprint verb for a removed app; the store looks for it to tell a
/// returning app from a new one.
pub const APP_REMOVED: &str = "app_removed";

/// (verb, subject, detail) — the parts of a change that make it unique.
fn identity(kind: &ChangeKind) -> (&'static str, String, String) {
    use ChangeKind::*;
    match kind {
        PackageInstalled { name, version, .. } => ("installed", name.clone(), version.clone()),
        PackageRemoved { name, version } => ("removed", name.clone(), version.clone()),
        PackageUpgraded { name, from, to } => ("upgraded", name.clone(), format!("{}>{}", from, to)),
        PackageDowngraded { name, from, to } => ("downgraded", name.clone(), format!("{}>{}", from, to)),
        UserAdded { name, uid, .. } => ("user_added", name.clone(), uid.to_string()),
        UserRemoved { name } => ("user_removed", name.clone(), String::new()),
        UserUidChanged { name, from, to } => ("uid_changed", name.clone(), format!("{}>{}", from, to)),
        GroupAdded { name } => ("group_added", name.clone(), String::new()),
        GroupRemoved { name } => ("group_removed", name.clone(), String::new()),
        GroupMemberAdded { group, user } => ("member_added", group.clone(), user.clone()),
        GroupMemberRemoved { group, user } => ("member_removed", group.clone(), user.clone()),
        SshKeyAdded { file, label } => ("key_added", file.clone(), label.clone()),
        SshKeyRemoved { file, label } => ("key_removed", file.clone(), label.clone()),
        SudoersChanged { path } => ("sudoers", path.clone(), String::new()),
        UnitEnabled { unit, scope } => ("enabled", unit.clone(), scope.clone()),
        UnitDisabled { unit, scope } => ("disabled", unit.clone(), scope.clone()),
        SetuidAdded { path, .. } => ("setuid_added", path.clone(), String::new()),
        SetuidRemoved { path } => ("setuid_removed", path.clone(), String::new()),
        FileAltered { path, what, .. } => ("file_altered", path.clone(), what.clone()),
        FileRestored { path } => ("file_restored", path.clone(), String::new()),
        AdvisoryAdded { package, ids, .. } => ("advisory", package.clone(), ids.clone()),
        AdvisoryResolved { package, .. } => ("advisory_resolved", package.clone(), String::new()),
        AppAdded { name, version, .. } => ("app_added", name.clone(), version.clone()),
        AppRemoved { name } => (APP_REMOVED, name.clone(), String::new()),
        AppUpdated { name, from, to } => ("app_updated", name.clone(), format!("{}>{}", from, to)),
        ScannerDetection { scanner, threat, resource } => ("detection", scanner.clone(), format!("{} {}", threat, resource)),
    }
}

/// The severity rules, in one place.
pub fn classify(kind: &ChangeKind) -> Severity {
    match kind {
        // The case this whole subsystem exists for. A package the user
        // never asked for is now on their machine, pulled in by something
        // else. Not inherently bad — but it is exactly the thing that
        // goes unnoticed for days.
        ChangeKind::PackageInstalled {
            requested: false, ..
        } => Severity::Notable,

        // The user asked for it, so telling them about it is noise.
        ChangeKind::PackageInstalled {
            requested: true, ..
        } => Severity::Routine,

        // Something the user had is gone. Usually an intentional removal
        // or an obsoleted dependency, but a package disappearing without
        // the user removing it is worth a glance.
        ChangeKind::PackageRemoved { .. } => Severity::Notable,

        // The ordinary result of an update.
        ChangeKind::PackageUpgraded { .. } => Severity::Routine,

        // Going backwards is unusual enough to mention.
        ChangeKind::PackageDowngraded { .. } => Severity::Notable,

        // Privilege surfaces: anything that lets someone do more is Critical
        // or Notable; anything that takes access away is Routine.
        ChangeKind::UserAdded { uid: 0, .. } => Severity::Critical,
        ChangeKind::UserAdded { login: true, .. } => Severity::Notable,
        ChangeKind::UserUidChanged { to: 0, .. } => Severity::Critical,
        ChangeKind::GroupMemberAdded { group, .. } if PRIVILEGED_GROUPS.contains(&group.as_str()) => {
            Severity::Critical
        }
        ChangeKind::GroupMemberAdded { .. } => Severity::Notable,
        ChangeKind::SshKeyAdded { .. } => Severity::Critical,
        ChangeKind::SudoersChanged { .. } => Severity::Critical,
        ChangeKind::SetuidAdded { .. } => Severity::Critical,
        ChangeKind::UnitEnabled { .. } => Severity::Notable,

        // Integrity: an edited config file is ordinary. Changed content
        // anywhere else is the mismatch the phase table calls Critical.
        ChangeKind::FileAltered { config: true, .. } => Severity::Routine,
        ChangeKind::FileAltered { what, .. } => match what.as_str() {
            "content" => Severity::Critical,
            "missing" | "permissions" => Severity::Notable,
            _ => Severity::Routine,
        },
        // The scanner's own rating, mapped by a fixed rule — never judged here.
        ChangeKind::AdvisoryAdded { severity, .. } => match severity.as_str() {
            "high" | "critical" => Severity::Notable,
            _ => Severity::Routine,
        },
        ChangeKind::AppAdded { .. } => Severity::Notable,
        ChangeKind::ScannerDetection { .. } => Severity::Critical,
        ChangeKind::FileRestored { .. }
        | ChangeKind::AdvisoryResolved { .. }
        | ChangeKind::AppRemoved { .. }
        | ChangeKind::AppUpdated { .. } => Severity::Routine,
        ChangeKind::UserAdded { .. }
        | ChangeKind::UserRemoved { .. }
        | ChangeKind::UserUidChanged { .. }
        | ChangeKind::GroupAdded { .. }
        | ChangeKind::GroupRemoved { .. }
        | ChangeKind::GroupMemberRemoved { .. }
        | ChangeKind::SshKeyRemoved { .. }
        | ChangeKind::UnitDisabled { .. }
        | ChangeKind::SetuidRemoved { .. } => Severity::Routine,
    }
}

/// One plain sentence per change. Written to be spoken aloud, so no
/// bullet points, no jargon, and no implied verdict — MAVIS reports what
/// happened and leaves the judgement to the user.
pub fn describe(kind: &ChangeKind) -> String {
    match kind {
        ChangeKind::PackageInstalled {
            name,
            version,
            requested: false,
        } => format!(
            "{} ({}) was installed as a dependency — you didn't ask for it directly.",
            name, version
        ),
        ChangeKind::PackageInstalled {
            name,
            version,
            requested: true,
        } => format!("{} ({}) was installed.", name, version),
        ChangeKind::PackageRemoved { name, version } => {
            format!("{} ({}) was removed.", name, version)
        }
        ChangeKind::PackageUpgraded { name, from, to } => {
            format!("{} was updated from {} to {}.", name, from, to)
        }
        ChangeKind::PackageDowngraded { name, from, to } => format!(
            "{} went backwards from {} to {} — that's unusual.",
            name, from, to
        ),
        ChangeKind::UserAdded { name, uid: 0, .. } => {
            format!("New account {} has user ID 0, the same as root.", name)
        }
        ChangeKind::UserAdded { name, uid, login: true } => {
            format!("New login account {} (user ID {}) was created.", name, uid)
        }
        ChangeKind::UserAdded { name, uid, .. } => {
            format!("System account {} (user ID {}) was created.", name, uid)
        }
        ChangeKind::UserRemoved { name } => format!("Account {} was removed.", name),
        ChangeKind::UserUidChanged { name, from, to } => {
            format!("{}'s user ID changed from {} to {}.", name, from, to)
        }
        ChangeKind::GroupAdded { name } => format!("Group {} was created.", name),
        ChangeKind::GroupRemoved { name } => format!("Group {} was removed.", name),
        ChangeKind::GroupMemberAdded { group, user } => {
            format!("{} was added to the {} group.", user, group)
        }
        ChangeKind::GroupMemberRemoved { group, user } => {
            format!("{} was removed from the {} group.", user, group)
        }
        ChangeKind::SshKeyAdded { file, label } => {
            format!("SSH key {} was added to {}.", label, file)
        }
        ChangeKind::SshKeyRemoved { file, label } => {
            format!("SSH key {} was removed from {}.", label, file)
        }
        ChangeKind::SudoersChanged { path } => format!("{} changed.", path),
        ChangeKind::UnitEnabled { unit, scope } => format!("{} was enabled ({}).", unit, scope),
        ChangeKind::UnitDisabled { unit, scope } => format!("{} was disabled ({}).", unit, scope),
        ChangeKind::SetuidAdded { path, setuid, root_owned } => {
            let mode = if *setuid { "setuid" } else { "setgid" };
            let owner = if *root_owned { " and owned by root" } else { "" };
            format!("{} is new, {}{}.", path, mode, owner)
        }
        ChangeKind::SetuidRemoved { path } => format!("{} is no longer setuid or setgid.", path),
        ChangeKind::FileAltered { path, package, what, .. } => {
            let from = if package.is_empty() { String::new() } else { format!(" (from {})", package) };
            match what.as_str() {
                "content" => format!("{}{} no longer matches what its package installed.", path, from),
                "missing" => format!("{}{} is missing.", path, from),
                "permissions" => format!("{}{} has different permissions from its package.", path, from),
                _ => format!("{}{} has a different timestamp from its package.", path, from),
            }
        }
        ChangeKind::FileRestored { path } => format!("{} matches its package again.", path),
        ChangeKind::AdvisoryAdded { scanner, package, severity, ids, fix_available } => {
            let rating = if severity.is_empty() { String::new() } else { format!(", rated {}", severity) };
            let fix = if *fix_available { " A fix is available." } else { "" };
            format!("{} reports {} is affected by {}{}.{}", scanner, package, ids, rating, fix)
        }
        ChangeKind::AdvisoryResolved { scanner, package } => {
            format!("{} no longer lists an advisory for {}.", scanner, package)
        }
        ChangeKind::AppAdded { name, version, returned: true } => {
            format!("{} ({}) is back after being removed.", name, version)
        }
        ChangeKind::AppAdded { name, version, .. } => format!("{} ({}) appeared.", name, version),
        ChangeKind::AppRemoved { name } => format!("{} was removed.", name),
        ChangeKind::AppUpdated { name, from, to } => format!("{} went from {} to {}.", name, from, to),
        ChangeKind::ScannerDetection { scanner, threat, resource } => {
            format!("{} reported {} in {}.", scanner, threat, resource)
        }
    }
}

/// The worst severity in a batch, or None if the batch is empty.
pub fn worst(changes: &[Change]) -> Option<Severity> {
    changes.iter().map(|c| c.severity).max()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ts: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(ts, 0).expect("valid timestamp")
    }

    #[test]
    fn severity_orders_correctly() {
        assert!(Severity::Critical > Severity::Notable);
        assert!(Severity::Notable > Severity::Routine);
    }

    #[test]
    fn severity_round_trips_through_strings() {
        for s in [Severity::Routine, Severity::Notable, Severity::Critical] {
            assert_eq!(Severity::from_str(s.as_str()), Some(s));
        }
        assert_eq!(Severity::from_str("nonsense"), None);
    }

    /// The hyprland case: a package the user never asked for is Notable,
    /// while the same package installed deliberately is Routine.
    #[test]
    fn unrequested_installs_are_notable_and_requested_ones_are_not() {
        let pulled_in = ChangeKind::PackageInstalled {
            name: "hyprland".into(),
            version: "0.41.2-1".into(),
            requested: false,
        };
        let asked_for = ChangeKind::PackageInstalled {
            name: "hyprland".into(),
            version: "0.41.2-1".into(),
            requested: true,
        };
        assert_eq!(classify(&pulled_in), Severity::Notable);
        assert_eq!(classify(&asked_for), Severity::Routine);
    }

    #[test]
    fn routine_upgrades_stay_quiet() {
        let kind = ChangeKind::PackageUpgraded {
            name: "firefox".into(),
            from: "140.0-1".into(),
            to: "141.0-1".into(),
        };
        assert_eq!(classify(&kind), Severity::Routine);
    }

    #[test]
    fn removals_and_downgrades_are_notable() {
        assert_eq!(
            classify(&ChangeKind::PackageRemoved {
                name: "foo".into(),
                version: "1.0".into()
            }),
            Severity::Notable
        );
        assert_eq!(
            classify(&ChangeKind::PackageDowngraded {
                name: "foo".into(),
                from: "2.0".into(),
                to: "1.0".into()
            }),
            Severity::Notable
        );
    }

    #[test]
    fn descriptions_name_the_package_and_stay_speakable() {
        let c = Change::new(
            ChangeKind::PackageInstalled {
                name: "hyprland".into(),
                version: "0.41.2-1".into(),
                requested: false,
            },
            "pacman",
            at(1_700_000_000),
        );
        assert!(c.detail.contains("hyprland"));
        assert!(c.detail.contains("didn't ask for it"));
        // Speakable: no markdown, no newlines.
        assert!(!c.detail.contains('\n'));
        assert!(!c.detail.contains('*'));
    }

    #[test]
    fn fingerprints_distinguish_different_changes() {
        let a = Change::new(
            ChangeKind::PackageInstalled {
                name: "hyprland".into(),
                version: "0.41.2-1".into(),
                requested: false,
            },
            "pacman",
            at(1_700_000_000),
        );
        let b = Change::new(
            ChangeKind::PackageInstalled {
                name: "hyprland".into(),
                version: "0.41.3-1".into(),
                requested: false,
            },
            "pacman",
            at(1_700_000_000),
        );
        // Same package installed again later is a genuinely new event.
        let c = Change::new(
            ChangeKind::PackageInstalled {
                name: "hyprland".into(),
                version: "0.41.2-1".into(),
                requested: false,
            },
            "pacman",
            at(1_700_009_999),
        );
        assert_ne!(a.fingerprint(), b.fingerprint());
        assert_ne!(a.fingerprint(), c.fingerprint());
    }

    #[test]
    fn identical_changes_share_a_fingerprint() {
        let mk = || {
            Change::new(
                ChangeKind::PackageRemoved {
                    name: "foo".into(),
                    version: "1.0".into(),
                },
                "dpkg",
                at(1_700_000_000),
            )
        };
        assert_eq!(mk().fingerprint(), mk().fingerprint());
    }

    #[test]
    fn privilege_gains_are_loud_and_losses_are_quiet() {
        use ChangeKind::*;
        let user = |uid, login| UserAdded { name: "x".into(), uid, login };
        assert_eq!(classify(&user(0, false)), Severity::Critical);
        assert_eq!(classify(&user(1001, true)), Severity::Notable);
        assert_eq!(classify(&user(977, false)), Severity::Routine);
        assert_eq!(classify(&UserUidChanged { name: "x".into(), from: 1000, to: 0 }), Severity::Critical);

        let member = |g: &str| GroupMemberAdded { group: g.into(), user: "bob".into() };
        assert_eq!(classify(&member("wheel")), Severity::Critical);
        assert_eq!(classify(&member("video")), Severity::Notable);

        let key = SshKeyAdded { file: "authorized_keys".into(), label: "me@laptop".into() };
        assert_eq!(classify(&key), Severity::Critical);
        assert_eq!(classify(&SudoersChanged { path: "/etc/sudoers".into() }), Severity::Critical);
        let suid = SetuidAdded { path: "/usr/local/bin/x".into(), setuid: true, root_owned: true };
        assert_eq!(classify(&suid), Severity::Critical);
        assert_eq!(classify(&UnitEnabled { unit: "a.service".into(), scope: "system".into() }), Severity::Notable);

        for loss in [
            UserRemoved { name: "x".into() },
            GroupMemberRemoved { group: "wheel".into(), user: "bob".into() },
            SshKeyRemoved { file: "authorized_keys".into(), label: "k".into() },
            UnitDisabled { unit: "a.service".into(), scope: "system".into() },
            SetuidRemoved { path: "/x".into() },
            GroupAdded { name: "g".into() },
        ] {
            assert_eq!(classify(&loss), Severity::Routine, "{:?}", loss);
        }
    }

    #[test]
    fn integrity_and_advisory_severities_are_fixed_rules() {
        use ChangeKind::*;
        let file = |what: &str, config| FileAltered {
            path: "/usr/bin/x".into(),
            package: "x".into(),
            what: what.into(),
            config,
        };
        assert_eq!(classify(&file("content", false)), Severity::Critical);
        assert_eq!(classify(&file("content", true)), Severity::Routine, "an edited config file is ordinary");
        assert_eq!(classify(&file("missing", false)), Severity::Notable);
        assert_eq!(classify(&file("permissions", false)), Severity::Notable);
        assert_eq!(classify(&file("timestamp", false)), Severity::Routine);

        let advisory = |sev: &str| AdvisoryAdded {
            scanner: "arch-audit".into(),
            package: "example-pkg".into(),
            severity: sev.into(),
            ids: "CVE-1".into(),
            fix_available: false,
        };
        assert_eq!(classify(&advisory("critical")), Severity::Notable);
        assert_eq!(classify(&advisory("high")), Severity::Notable);
        assert_eq!(classify(&advisory("medium")), Severity::Routine);
        assert_eq!(classify(&advisory("")), Severity::Routine);

        let detection = ScannerDetection { scanner: "Microsoft Defender".into(), threat: "T".into(), resource: "f".into() };
        assert_eq!(classify(&detection), Severity::Critical);
        assert_eq!(classify(&AppAdded { name: "a".into(), version: "1".into(), returned: false }), Severity::Notable);
        assert_eq!(classify(&AppRemoved { name: "a".into() }), Severity::Routine);
    }

    #[test]
    fn privilege_fingerprints_tell_changes_apart() {
        let a = Change::new(
            ChangeKind::GroupMemberAdded { group: "wheel".into(), user: "bob".into() },
            "groups",
            at(0),
        );
        let b = Change::new(
            ChangeKind::GroupMemberAdded { group: "wheel".into(), user: "eve".into() },
            "groups",
            at(0),
        );
        assert_ne!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn worst_picks_the_highest_severity() {
        let changes = vec![
            Change::new(
                ChangeKind::PackageUpgraded {
                    name: "a".into(),
                    from: "1".into(),
                    to: "2".into(),
                },
                "dpkg",
                at(0),
            ),
            Change::new(
                ChangeKind::PackageInstalled {
                    name: "b".into(),
                    version: "1".into(),
                    requested: false,
                },
                "dpkg",
                at(0),
            ),
        ];
        assert_eq!(worst(&changes), Some(Severity::Notable));
        assert_eq!(worst(&[]), None);
    }
}