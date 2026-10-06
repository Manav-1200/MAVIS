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
    if group.iter().any(|c| is_privilege(&c.kind)) {
        return privilege_sentence(group, false);
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
                "pulled in {} packages you didn't ask for{}",
                n,
                listed(&pulled_in)
            ),
        });
    }
    if !removed.is_empty() {
        parts.push(match removed.len() {
            1 => format!("removed {}", removed[0]),
            n => format!("removed {} packages{}", n, listed(&removed)),
        });
    }
    if !downgraded.is_empty() {
        parts.push(match downgraded.len() {
            1 => format!("put {} back to an older version", downgraded[0]),
            n => format!(
                "put {} packages back to older versions{}",
                n,
                listed(&downgraded)
            ),
        });
    }

    Some(format!(
        "Your system update {} {}.",
        when,
        join_naturally_owned(&parts)
    ))
}

/// True for step 3's privilege-surface changes.
pub fn is_privilege(kind: &ChangeKind) -> bool {
    !matches!(
        kind,
        ChangeKind::PackageInstalled { .. }
            | ChangeKind::PackageRemoved { .. }
            | ChangeKind::PackageUpgraded { .. }
            | ChangeKind::PackageDowngraded { .. }
    )
}

/// Everything in a privilege group, routine changes included — for when
/// the user asks.
pub fn describe_privilege(group: &[Change]) -> Option<String> {
    privilege_sentence(group, true)
}

/// One sentence for a scan's privilege changes: "Today, eve was added to
/// the wheel group and a new SSH key, eve@box, was added to ~/.ssh/authorized_keys."
fn privilege_sentence(group: &[Change], include_routine: bool) -> Option<String> {
    use std::collections::BTreeMap;
    let first = group.iter().map(|c| c.occurred_at).min()?;
    let mut sorted: Vec<&Change> = group
        .iter()
        .filter(|c| include_routine || c.severity > Severity::Routine)
        .collect();
    sorted.sort_by_key(|c| c.occurred_at);

    let mut root_ids = Vec::new();
    let mut logins = Vec::new();
    let mut joined: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut keys_added: Vec<(String, String)> = Vec::new();
    let mut sudoers = false;
    let mut enabled = Vec::new();
    let mut setuid_root = Vec::new();
    let mut setuid_other = Vec::new();
    // Routine — only when asked.
    let mut system_accounts = Vec::new();
    let mut accounts_removed = Vec::new();
    let mut uid_changed = Vec::new();
    let mut groups_added = Vec::new();
    let mut groups_removed = Vec::new();
    let mut left: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut keys_removed = Vec::new();
    let mut disabled = Vec::new();
    let mut setuid_gone = Vec::new();

    for c in &sorted {
        match &c.kind {
            ChangeKind::UserAdded { name, uid: 0, .. } => root_ids.push(name.clone()),
            ChangeKind::UserUidChanged { name, to: 0, .. } => root_ids.push(name.clone()),
            ChangeKind::UserAdded { name, login: true, .. } => logins.push(name.clone()),
            ChangeKind::UserAdded { name, .. } => system_accounts.push(name.clone()),
            ChangeKind::UserRemoved { name } => accounts_removed.push(name.clone()),
            ChangeKind::UserUidChanged { name, .. } => uid_changed.push(name.clone()),
            ChangeKind::GroupAdded { name } => groups_added.push(name.clone()),
            ChangeKind::GroupRemoved { name } => groups_removed.push(name.clone()),
            ChangeKind::GroupMemberAdded { group, user } => {
                joined.entry(group).or_default().push(user.clone())
            }
            ChangeKind::GroupMemberRemoved { group, user } => {
                left.entry(group).or_default().push(user.clone())
            }
            ChangeKind::SshKeyAdded { file, label } => keys_added.push((file.clone(), label.clone())),
            ChangeKind::SshKeyRemoved { label, .. } => keys_removed.push(label.clone()),
            ChangeKind::SudoersChanged { .. } => sudoers = true,
            ChangeKind::UnitEnabled { unit, .. } => enabled.push(unit.clone()),
            ChangeKind::UnitDisabled { unit, .. } => disabled.push(unit.clone()),
            ChangeKind::SetuidAdded { path, setuid: true, root_owned: true } => setuid_root.push(file_name(path)),
            ChangeKind::SetuidAdded { path, .. } => setuid_other.push(file_name(path)),
            ChangeKind::SetuidRemoved { path } => setuid_gone.push(file_name(path)),
            _ => {}
        }
    }

    // Step 4 and 5 kinds are phrased in phrases.rs: loud ones first,
    // routine ones last.
    let extra = super::phrases::clauses(&sorted);
    let mut clauses: Vec<String> = extra.loud;

    clauses.extend(counted(&root_ids, &|n| format!("{} has user ID 0, the same as root", n), &|n| {
        format!("{} accounts have user ID 0, the same as root", n)
    }));
    clauses.extend(counted(&logins, &|n| format!("a new login account, {}, was created", n), &|n| {
        format!("{} new login accounts were created", n)
    }));
    for (group, users) in &joined {
        let names: Vec<&str> = users.iter().map(String::as_str).collect();
        let verb = if users.len() == 1 { "was" } else { "were" };
        clauses.push(format!("{} {} added to the {} group", join_naturally(&names), verb, group));
    }
    if let Some((file, _)) = keys_added.first() {
        let labels: Vec<String> = keys_added.iter().map(|(_, l)| l.clone()).collect();
        clauses.extend(counted(&labels, &|l| format!("a new SSH key, {}, was added to {}", l, file), &|n| {
            format!("{} new SSH keys were added to {}", n, file)
        }));
    }
    if sudoers {
        clauses.push("the sudo configuration changed".to_string());
    }
    clauses.extend(counted(&enabled, &|u| format!("{} was set to start automatically", u), &|n| {
        format!("{} units were set to start automatically", n)
    }));
    clauses.extend(counted(&setuid_root, &|p| format!("a new program that runs as root appeared: {}", p), &|n| {
        format!("{} new programs that run as root appeared", n)
    }));
    clauses.extend(counted(&setuid_other, &|p| format!("a new setuid or setgid program appeared: {}", p), &|n| {
        format!("{} new setuid or setgid programs appeared", n)
    }));
    clauses.extend(counted(&system_accounts, &|n| format!("system account {} was created", n), &|n| {
        format!("{} system accounts were created", n)
    }));
    clauses.extend(counted(&accounts_removed, &|n| format!("account {} was removed", n), &|n| {
        format!("{} accounts were removed", n)
    }));
    clauses.extend(counted(&uid_changed, &|n| format!("{}'s user ID changed", n), &|n| {
        format!("{} accounts changed user ID", n)
    }));
    clauses.extend(counted(&groups_added, &|n| format!("group {} was created", n), &|n| format!("{} groups were created", n)));
    clauses.extend(counted(&groups_removed, &|n| format!("group {} was removed", n), &|n| format!("{} groups were removed", n)));
    clauses.extend(counted(&keys_removed, &|l| format!("SSH key {} was removed", l), &|n| format!("{} SSH keys were removed", n)));
    clauses.extend(counted(&disabled, &|u| format!("{} was disabled", u), &|n| format!("{} units were disabled", n)));
    clauses.extend(counted(&setuid_gone, &|p| format!("{} is no longer setuid", p), &|n| {
        format!("{} programs are no longer setuid", n)
    }));
    for (group, users) in &left {
        let names: Vec<&str> = users.iter().map(String::as_str).collect();
        let verb = if users.len() == 1 { "was" } else { "were" };
        clauses.push(format!("{} {} removed from the {} group", join_naturally(&names), verb, group));
    }

    clauses.extend(extra.quiet);
    if clauses.is_empty() {
        return None;
    }
    Some(format!("{}, {}.", capitalize(&relative_day(first)), join_naturally_owned(&clauses)))
}

/// "x was …" for one item, "N things …: a, b and c" for several.
pub(super) fn counted(items: &[String], one: &dyn Fn(&str) -> String, many: &dyn Fn(usize) -> String) -> Option<String> {
    match items.len() {
        0 => None,
        1 => Some(one(&items[0])),
        n => {
            let names: Vec<&str> = items.iter().map(String::as_str).collect();
            Some(format!("{}{}", many(n), listed(&names)))
        }
    }
}

/// File name only: a spoken path is all slashes.
pub(super) fn file_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Sentences for a whole batch, one per transaction, newest last.
pub fn summarize_all(changes: &[Change]) -> Vec<String> {
    group_transactions(changes, TRANSACTION_GAP_SECS)
        .iter()
        .filter_map(|g| summarize(g))
        .collect()
}

/// Past this many names, say the count and a few examples. An install
/// can pull in a hundred dependencies; the full list stays in the store.
pub const MAX_SPOKEN_NAMES: usize = 5;

/// ": a, b and c", or past the cap ", including a, b, c, d and e".
pub(super) fn listed(items: &[&str]) -> String {
    if items.len() <= MAX_SPOKEN_NAMES {
        format!(": {}", join_naturally(items))
    } else {
        format!(", including {}", join_naturally(&items[..MAX_SPOKEN_NAMES]))
    }
}

/// How to refer to a day in speech. Anything older than a week gets a
/// date, because "on Tuesday" stops being useful past seven days.
pub(super) fn relative_day(when: DateTime<Utc>) -> String {
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

pub(super) fn join_naturally(items: &[&str]) -> String {
    let owned: Vec<String> = items.iter().map(|s| s.to_string()).collect();
    join_naturally_owned(&owned)
}

/// "a", "a and b", "a, b and c" — written out rather than comma-listed,
/// because this is read aloud.
pub(super) fn join_naturally_owned(items: &[String]) -> String {
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

    #[test]
    fn long_lists_are_cut_to_a_count_and_examples() {
        let changes: Vec<Change> = (0..40)
            .map(|i| installed(&format!("dep{:02}", i), false, 100 - i))
            .collect();
        let line = summarize(&changes).unwrap();
        assert!(line.contains("pulled in 40 packages you didn't ask for, including"), "{}", line);
        assert!(line.contains("dep00, dep01, dep02, dep03 and dep04"), "{}", line);
        assert!(!line.contains("dep05"), "{}", line);
    }

    #[test]
    fn a_list_at_the_cap_is_said_in_full() {
        let changes: Vec<Change> = (0..MAX_SPOKEN_NAMES as i64)
            .map(|i| installed(&format!("dep{}", i), false, 100 - i))
            .collect();
        let line = summarize(&changes).unwrap();
        assert!(!line.contains("including"), "{}", line);
        assert!(line.contains(&format!("dep{}", MAX_SPOKEN_NAMES - 1)), "{}", line);
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

    fn privilege(kind: ChangeKind, source: &str, secs: i64) -> Change {
        Change::new(kind, source, at(secs))
    }

    #[test]
    fn privilege_changes_get_their_own_sentence() {
        let group = vec![
            privilege(ChangeKind::GroupMemberAdded { group: "wheel".into(), user: "eve".into() }, "groups", 10),
            privilege(ChangeKind::GroupAdded { name: "eve".into() }, "groups", 10),
        ];
        let line = summarize(&group).unwrap();
        assert_eq!(line, "Today, eve was added to the wheel group.");
        assert!(!line.contains("system update"), "{}", line);
    }

    #[test]
    fn several_ssh_keys_are_counted() {
        let key = |l: &str| ChangeKind::SshKeyAdded { file: "~/.ssh/authorized_keys".into(), label: l.into() };
        let line = summarize(&[privilege(key("a@x"), "ssh_keys", 10), privilege(key("b@y"), "ssh_keys", 10)]).unwrap();
        assert_eq!(line, "Today, 2 new SSH keys were added to ~/.ssh/authorized_keys: a@x and b@y.");
    }

    #[test]
    fn setuid_programs_are_named_by_file_not_path() {
        let kind = ChangeKind::SetuidAdded { path: "/usr/local/bin/helper".into(), setuid: true, root_owned: true };
        let line = summarize(&[privilege(kind, "setuid", 10)]).unwrap();
        assert_eq!(line, "Today, a new program that runs as root appeared: helper.");
    }

    /// Routine privilege changes are never volunteered, but are described when asked.
    #[test]
    fn routine_privilege_changes_wait_to_be_asked() {
        let group = [privilege(ChangeKind::UnitDisabled { unit: "bluetooth.service".into(), scope: "system".into() }, "systemd", 10)];
        assert!(summarize(&group).is_none());
        assert_eq!(describe_privilege(&group).unwrap(), "Today, bluetooth.service was disabled.");
    }

    #[test]
    fn peak_severity_picks_the_worst() {
        let changes = vec![upgraded("a", 10), installed("b", false, 11)];
        assert_eq!(peak_severity(&changes), Some(Severity::Notable));
        assert_eq!(peak_severity(&[]), None);
    }
}