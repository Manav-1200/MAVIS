// mavis_core/src/sentinel/summary.rs
// Turning a pile of changes into something worth saying out loud.
//
// This exists because of a number from a real machine: an Arch install
// with 2079 logged transactions and only 110 explicitly-installed
// packages. On Arch roughly nine in ten packages are dependencies, so
// "installed as a dependency" is common, and reporting each one as its
// own sentence would make MAVIS unbearable within a week — at which
// point the user stops listening and the subsystem has failed at its
// only job.
//
// So changes are grouped back into the transaction that produced them
// (a single `pacman -Syu` writes dozens of log lines within a few
// seconds) and each transaction becomes one sentence.
//
// Routine changes are never spoken. They are recorded and can be asked
// about, which is a different thing.

use super::change::{Change, ChangeKind, Severity};
use chrono::{DateTime, Datelike, Local, Utc};

/// Log entries this far apart belong to different transactions. A single
/// package operation writes its lines within milliseconds of each other,
/// but a large update runs for minutes, so the gap has to tolerate slow
/// install scripts without merging two separate updates.
pub const TRANSACTION_GAP_SECS: i64 = 300;

/// Group changes into the transactions that produced them.
///
/// Input need not be sorted. Grouping is by source as well as time, so a
/// machine with two package managers (a Nix or Homebrew install
/// alongside the system one) doesn't merge their transactions.
pub fn group_transactions(changes: &[Change], gap_secs: i64) -> Vec<Vec<Change>> {
    let mut sorted: Vec<Change> = changes.to_vec();
    sorted.sort_by(|a, b| {
        a.source
            .cmp(&b.source)
            .then(a.occurred_at.cmp(&b.occurred_at))
    });

    let mut groups: Vec<Vec<Change>> = Vec::new();
    for change in sorted {
        let fits = groups.last().is_some_and(|g| {
            let last = g.last().expect("groups are never empty");
            last.source == change.source
                && (change.occurred_at - last.occurred_at).num_seconds().abs() <= gap_secs
        });
        if fits {
            groups.last_mut().expect("just checked").push(change);
        } else {
            groups.push(vec![change]);
        }
    }
    groups
}

/// One spoken sentence for a transaction, or None when nothing in it is
/// worth interrupting for.
///
/// Deliberately reports and stops. No verdict, no advice — the user knows
/// whether they wanted Hyprland on their machine and MAVIS does not.
pub fn summarize(group: &[Change]) -> Option<String> {
    if group.is_empty() {
        return None;
    }

    // Sort here rather than trusting the caller. `summarize_all` arrives
    // pre-sorted via grouping, but `summarize` is also called directly,
    // and a sentence whose order depends on how the slice was assembled
    // is a bug waiting to happen. Oldest first, so it reads in the order
    // things actually happened.
    let mut group: Vec<&Change> = group.iter().collect();
    group.sort_by_key(|c| c.occurred_at);

    let mut pulled_in: Vec<&str> = Vec::new();
    let mut removed: Vec<&str> = Vec::new();
    let mut downgraded: Vec<&str> = Vec::new();

    for change in &group {
        match &change.kind {
            ChangeKind::PackageInstalled {
                name,
                requested: false,
                ..
            } => pulled_in.push(name),
            ChangeKind::PackageRemoved { name, .. } => removed.push(name),
            ChangeKind::PackageDowngraded { name, .. } => downgraded.push(name),
            _ => {}
        }
    }

    if pulled_in.is_empty() && removed.is_empty() && downgraded.is_empty() {
        return None;
    }

    let when = relative_day(group[0].occurred_at);
    let mut parts: Vec<String> = Vec::new();

    if !pulled_in.is_empty() {
        parts.push(match pulled_in.len() {
            1 => format!("pulled in {}, which you didn't ask for", pulled_in[0]),
            n => format!(
                "pulled in {} packages you didn't ask for: {}",
                n,
                join_naturally(&pulled_in)
            ),
        });
    }
    if !removed.is_empty() {
        parts.push(match removed.len() {
            1 => format!("removed {}", removed[0]),
            n => format!("removed {} packages: {}", n, join_naturally(&removed)),
        });
    }
    if !downgraded.is_empty() {
        parts.push(match downgraded.len() {
            1 => format!("put {} back to an older version", downgraded[0]),
            n => format!(
                "put {} packages back to older versions: {}",
                n,
                join_naturally(&downgraded)
            ),
        });
    }

    Some(format!(
        "Your system update {} {}.",
        when,
        join_naturally_owned(&parts)
    ))
}

/// Sentences for a whole batch, one per transaction, newest last.
pub fn summarize_all(changes: &[Change]) -> Vec<String> {
    group_transactions(changes, TRANSACTION_GAP_SECS)
        .iter()
        .filter_map(|g| summarize(g))
        .collect()
}

/// How to refer to a day in speech. Anything older than a week gets a
/// date, because "on Tuesday" stops being useful past seven days.
fn relative_day(when: DateTime<Utc>) -> String {
    let local = when.with_timezone(&Local);
    let today = Local::now().date_naive();
    let that_day = local.date_naive();

    match (today - that_day).num_days() {
        0 => "today".to_string(),
        1 => "yesterday".to_string(),
        2..=6 => format!("on {}", local.format("%A")),
        _ => format!("on {} {}", that_day.day(), local.format("%B")),
    }
}

fn join_naturally(items: &[&str]) -> String {
    let owned: Vec<String> = items.iter().map(|s| s.to_string()).collect();
    join_naturally_owned(&owned)
}

/// "a", "a and b", "a, b and c" — written out rather than comma-listed,
/// because this is read aloud.
fn join_naturally_owned(items: &[String]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        2 => format!("{} and {}", items[0], items[1]),
        _ => {
            let (last, rest) = items.split_last().expect("len >= 3");
            format!("{} and {}", rest.join(", "), last)
        }
    }
}

/// The highest severity present, used to pick the alert channel.
pub fn peak_severity(changes: &[Change]) -> Option<Severity> {
    changes.iter().map(|c| c.severity).max()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> DateTime<Utc> {
        Utc::now() - chrono::Duration::seconds(secs)
    }

    fn installed(name: &str, requested: bool, secs: i64) -> Change {
        Change::new(
            ChangeKind::PackageInstalled {
                name: name.into(),
                version: "1.0".into(),
                requested,
            },
            "pacman",
            at(secs),
        )
    }

    fn upgraded(name: &str, secs: i64) -> Change {
        Change::new(
            ChangeKind::PackageUpgraded {
                name: name.into(),
                from: "1.0".into(),
                to: "2.0".into(),
            },
            "pacman",
            at(secs),
        )
    }

    fn removed(name: &str, secs: i64) -> Change {
        Change::new(
            ChangeKind::PackageRemoved {
                name: name.into(),
                version: "1.0".into(),
            },
            "pacman",
            at(secs),
        )
    }

    // -----------------------------------------------------------------
    // Grouping
    // -----------------------------------------------------------------

    #[test]
    fn one_update_is_one_group() {
        let changes = vec![
            installed("a", false, 10),
            installed("b", false, 11),
            upgraded("c", 12),
        ];
        assert_eq!(group_transactions(&changes, TRANSACTION_GAP_SECS).len(), 1);
    }

    #[test]
    fn updates_far_apart_are_separate_groups() {
        let changes = vec![installed("a", false, 10), installed("b", false, 100_000)];
        assert_eq!(group_transactions(&changes, TRANSACTION_GAP_SECS).len(), 2);
    }

    #[test]
    fn different_package_managers_never_merge() {
        let mut nix = installed("b", false, 10);
        nix.source = "nix".to_string();
        let changes = vec![installed("a", false, 10), nix];
        assert_eq!(group_transactions(&changes, TRANSACTION_GAP_SECS).len(), 2);
    }

    #[test]
    fn grouping_does_not_require_sorted_input() {
        let changes = vec![
            installed("c", false, 10),
            installed("a", false, 30),
            installed("b", false, 20),
        ];
        let groups = group_transactions(&changes, TRANSACTION_GAP_SECS);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 3);
    }

    #[test]
    fn grouping_handles_the_empty_case() {
        assert!(group_transactions(&[], TRANSACTION_GAP_SECS).is_empty());
    }

    // -----------------------------------------------------------------
    // Summarizing
    // -----------------------------------------------------------------

    /// The whole point: one sentence per update, not one per package.
    ///
    /// Note the helper takes *seconds ago*, so the largest number is the
    /// oldest change and appears first in the sentence.
    #[test]
    fn a_big_update_becomes_a_single_sentence() {
        let changes = vec![
            installed("hyprland", false, 14),
            installed("linux-firmware-amd", false, 13),
            installed("linux-firmware-ti", false, 12),
            upgraded("firefox", 11),
            upgraded("mesa", 10),
        ];
        let lines = summarize_all(&changes);
        assert_eq!(lines.len(), 1, "got: {:#?}", lines);
        let line = &lines[0];

        assert!(line.contains("3 packages you didn't ask for"), "{}", line);
        // Oldest first, joined for speech.
        assert!(
            line.contains("hyprland, linux-firmware-amd and linux-firmware-ti"),
            "{}",
            line
        );
        // Routine upgrades are not mentioned at all.
        assert!(!line.contains("firefox"), "{}", line);
        assert!(!line.contains("mesa"), "{}", line);
    }

    /// Order follows the log, oldest first, so the sentence reads in the
    /// order things actually happened.
    #[test]
    fn packages_are_listed_oldest_first() {
        let changes = vec![
            installed("last", false, 10),
            installed("first", false, 30),
            installed("middle", false, 20),
        ];
        let line = summarize(&changes).unwrap();
        assert!(line.contains("first, middle and last"), "{}", line);
    }

    #[test]
    fn a_single_pulled_in_package_reads_naturally() {
        let line = summarize(&[installed("hyprland", false, 10)]).unwrap();
        assert!(line.contains("pulled in hyprland, which you didn't ask for"), "{}", line);
    }

    /// An update that only bumped versions is not worth interrupting for.
    #[test]
    fn a_routine_update_produces_nothing() {
        let changes = vec![upgraded("firefox", 10), upgraded("mesa", 11)];
        assert!(summarize_all(&changes).is_empty());
        assert!(summarize(&changes).is_none());
    }

    #[test]
    fn packages_the_user_asked_for_are_not_mentioned() {
        let changes = vec![installed("neovim", true, 10), installed("ripgrep", true, 11)];
        assert!(summarize_all(&changes).is_empty());
    }

    #[test]
    fn removals_are_reported_alongside_pull_ins() {
        let changes = vec![installed("hyprland", false, 10), removed("oldthing", 11)];
        let line = summarize(&changes).unwrap();
        assert!(line.contains("hyprland"), "{}", line);
        assert!(line.contains("removed oldthing"), "{}", line);
        assert!(line.contains(" and "), "{}", line);
    }

    #[test]
    fn empty_input_summarizes_to_nothing() {
        assert!(summarize(&[]).is_none());
        assert!(summarize_all(&[]).is_empty());
    }

    /// Spoken aloud, so no markdown, no newlines, and it ends properly.
    #[test]
    fn summaries_are_speakable() {
        let changes = vec![installed("hyprland", false, 10), installed("foo", false, 11)];
        let line = summarize(&changes).unwrap();
        assert!(!line.contains('\n'));
        assert!(!line.contains('*'));
        assert!(!line.contains('#'));
        assert!(line.ends_with('.'), "{}", line);
    }

    // -----------------------------------------------------------------
    // Phrasing helpers
    // -----------------------------------------------------------------

    #[test]
    fn lists_are_joined_for_speech() {
        assert_eq!(join_naturally(&["a"]), "a");
        assert_eq!(join_naturally(&["a", "b"]), "a and b");
        assert_eq!(join_naturally(&["a", "b", "c"]), "a, b and c");
        assert_eq!(join_naturally(&[]), "");
    }

    #[test]
    fn recent_days_are_named_not_dated() {
        assert_eq!(relative_day(Utc::now()), "today");
        let yesterday = relative_day(Utc::now() - chrono::Duration::days(1));
        assert_eq!(yesterday, "yesterday");
        // Within the week: a weekday name.
        let midweek = relative_day(Utc::now() - chrono::Duration::days(3));
        assert!(midweek.starts_with("on "), "{}", midweek);
        assert!(!midweek.chars().any(|c| c.is_ascii_digit()), "{}", midweek);
        // Older than a week: an actual date.
        let old = relative_day(Utc::now() - chrono::Duration::days(30));
        assert!(old.chars().any(|c| c.is_ascii_digit()), "{}", old);
    }

    #[test]
    fn peak_severity_picks_the_worst() {
        let changes = vec![upgraded("a", 10), installed("b", false, 11)];
        assert_eq!(peak_severity(&changes), Some(Severity::Notable));
        assert_eq!(peak_severity(&[]), None);
    }
}
