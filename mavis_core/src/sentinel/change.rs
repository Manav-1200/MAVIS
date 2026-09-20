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
}

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
        let (verb, name, detail) = match &self.kind {
            ChangeKind::PackageInstalled { name, version, .. } => {
                ("installed", name.as_str(), version.clone())
            }
            ChangeKind::PackageRemoved { name, version } => {
                ("removed", name.as_str(), version.clone())
            }
            ChangeKind::PackageUpgraded { name, from, to } => {
                ("upgraded", name.as_str(), format!("{}>{}", from, to))
            }
            ChangeKind::PackageDowngraded { name, from, to } => {
                ("downgraded", name.as_str(), format!("{}>{}", from, to))
            }
        };
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