// mavis_core/src/sentinel/speech.rs
// Phase 8.5 step 2b: what the Sentinel says, and when.
// Leads with pending changes, and answers "what changed?" and "what did that
// update do?" from the store. Deterministic: the model never sees this path.

use super::change::{Change, ChangeKind, Severity};
use super::store::SentinelStore;
use super::summary::{
    describe_privilege, group_transactions, is_privilege, join_naturally, join_naturally_owned, listed, relative_day, summarize,
    TRANSACTION_GAP_SECS,
};
use chrono::{DateTime, Duration, Utc};
use log::warn;

/// Most updates read out when leading; older ones are only counted.
pub const MAX_LEAD_SENTENCES: usize = 2;

/// Most updates described in answer to "what changed?".
pub const MAX_DESCRIBED_UPDATES: usize = 3;

/// What "recently" means when no time was given.
pub const DEFAULT_LOOKBACK_DAYS: i64 = 7;

/// Row cap for one answer.
const QUERY_LIMIT: usize = 20_000;

/// A time window the planner found in the utterance ("yesterday").
#[derive(Clone, Debug)]
pub struct Window {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub label: String,
}

/// Text to speak, plus which update it was about (for "that update").
#[derive(Clone, Debug, PartialEq)]
pub struct Spoken {
    pub text: String,
    pub about: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Turn {
    /// The whole reply; nothing else answers this turn.
    Answer(Spoken),
    /// Say this first, then handle the utterance as normal.
    Lead(Spoken),
}

/// The Sentinel's part in one user turn, if any.
/// If a lead can't be marked as told, nothing is said; otherwise it
/// would repeat on every utterance.
pub fn respond(
    store: &SentinelStore,
    text: &str,
    window: Option<Window>,
    last_told: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<Turn>> {
    // Checked first: "what did the update change" also matches below.
    if is_update_question(text) {
        return answer_update(store, last_told).map(|s| Some(Turn::Answer(s)));
    }
    if is_change_question(text) {
        let window = window.unwrap_or_else(|| Window {
            start: now - Duration::days(DEFAULT_LOOKBACK_DAYS),
            end: now,
            label: "in the last week".to_string(),
        });
        return answer_period(store, &window).map(|s| Some(Turn::Answer(s)));
    }

    let pending = store.unannounced(Severity::Notable)?;
    let Some(told) = announcement(&pending, MAX_LEAD_SENTENCES) else {
        return Ok(None);
    };
    store.mark_announced(&told.fingerprints)?;
    Ok(Some(Turn::Lead(Spoken {
        text: told.text,
        about: told.about,
    })))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Announcement {
    pub text: String,
    /// Every change covered, spoken or counted.
    pub fingerprints: Vec<String>,
    /// Start of the newest update spoken.
    pub about: Option<DateTime<Utc>>,
}

/// What to say about pending changes. The newest `max` updates are read,
/// older ones counted, and all are marked told so they don't trickle out
/// over the next few utterances.
pub fn announcement(pending: &[Change], max: usize) -> Option<Announcement> {
    let mut told: Vec<(String, Vec<Change>)> = group_transactions(pending, TRANSACTION_GAP_SECS)
        .into_iter()
        .filter_map(|g| summarize(&g).map(|s| (s, g)))
        .collect();
    if told.is_empty() || max == 0 {
        return None;
    }
    // Grouping orders by source first; speech wants time order.
    told.sort_by_key(|(_, g)| g[0].occurred_at);

    let fingerprints = told
        .iter()
        .flat_map(|(_, g)| g.iter().map(Change::fingerprint))
        .collect();

    let earlier = told.len().saturating_sub(max);
    let spoken = &told[earlier..];
    let mut sentences: Vec<String> = spoken.iter().map(|(s, _)| s.clone()).collect();
    match earlier {
        0 => {}
        1 => sentences.push(
            "There was also one earlier change. Ask me what changed to hear about it.".into(),
        ),
        n => sentences.push(format!(
            "There were also {} earlier changes. Ask me what changed to hear about them.",
            n
        )),
    }

    Some(Announcement {
        text: sentences.join(" "),
        fingerprints,
        about: spoken.last().map(|(_, g)| g[0].occurred_at),
    })
}

fn answer_period(store: &SentinelStore, window: &Window) -> anyhow::Result<Spoken> {
    let changes = store.between(window.start, window.end, QUERY_LIMIT)?;
    let mut groups = group_transactions(&changes, TRANSACTION_GAP_SECS);
    groups.sort_by_key(|g| g[0].occurred_at);

    if groups.is_empty() {
        return Ok(Spoken {
            text: format!("Nothing changed on the system {}.", window.label),
            about: None,
        });
    }

    let skipped = groups.len().saturating_sub(MAX_DESCRIBED_UPDATES);
    let described = &groups[skipped..];
    let mut sentences: Vec<String> = Vec::new();
    if skipped > 0 {
        sentences.push(format!(
            "There were {} changes {}. The latest {}:",
            groups.len(),
            window.label,
            number_word(described.len())
        ));
    }
    sentences.extend(described.iter().filter_map(|g| describe_transaction(g)));

    mark_told(store, described);
    Ok(Spoken {
        text: sentences.join(" "),
        about: described.last().map(|g| g[0].occurred_at),
    })
}

fn answer_update(
    store: &SentinelStore,
    last_told: Option<DateTime<Utc>>,
) -> anyhow::Result<Spoken> {
    let anchor = match last_told {
        Some(at) => Some(at),
        None => store.recent(1)?.first().map(|c| c.occurred_at),
    };
    let Some(anchor) = anchor else {
        return Ok(Spoken {
            text: "I haven't seen any system updates yet.".to_string(),
            about: None,
        });
    };

    // ±12 h comfortably covers any single transaction.
    let around = store.between(
        anchor - Duration::hours(12),
        anchor + Duration::hours(12),
        QUERY_LIMIT,
    )?;
    let Some(group) = group_containing(&around, anchor) else {
        return Ok(Spoken {
            text: "I can't find that update any more.".to_string(),
            about: None,
        });
    };

    let text = describe_transaction(&group)
        .unwrap_or_else(|| "I can't find that update any more.".to_string());
    mark_told(store, std::slice::from_ref(&group));
    Ok(Spoken {
        text,
        about: Some(group[0].occurred_at),
    })
}

/// The transaction holding `anchor`, or failing that the nearest one.
fn group_containing(changes: &[Change], anchor: DateTime<Utc>) -> Option<Vec<Change>> {
    let groups = group_transactions(changes, TRANSACTION_GAP_SECS);
    let exact = groups
        .iter()
        .position(|g| g.iter().any(|c| c.occurred_at == anchor));
    let index = exact.or_else(|| {
        groups
            .iter()
            .enumerate()
            .min_by_key(|(_, g)| (g[0].occurred_at - anchor).num_seconds().abs())
            .map(|(i, _)| i)
    })?;
    groups.into_iter().nth(index)
}

/// Changes covered by an answer count as told. A failure only means the
/// user may hear them once more, so the answer still goes out.
fn mark_told(store: &SentinelStore, groups: &[Vec<Change>]) {
    let prints: Vec<String> = groups
        .iter()
        .flatten()
        .filter(|c| c.severity >= Severity::Notable)
        .map(Change::fingerprint)
        .collect();
    if let Err(e) = store.mark_announced(&prints) {
        warn!(
            "Sentinel: answered, but could not mark changes as told: {}",
            e
        );
    }
}

/// Everything one update did: the Notable sentence, then what was asked
/// for and how much was upgraded (named up to two, counted beyond).
pub fn describe_transaction(group: &[Change]) -> Option<String> {
    if group.iter().any(|c| is_privilege(&c.kind)) {
        return describe_privilege(group);
    }
    let first = group.iter().map(|c| c.occurred_at).min()?;

    let mut sorted: Vec<&Change> = group.iter().collect();
    sorted.sort_by_key(|c| c.occurred_at);

    let mut asked_for: Vec<&str> = Vec::new();
    let mut upgraded: Vec<&str> = Vec::new();
    for change in &sorted {
        match &change.kind {
            ChangeKind::PackageInstalled {
                name,
                requested: true,
                ..
            } => asked_for.push(name),
            ChangeKind::PackageUpgraded { name, .. } => upgraded.push(name),
            _ => {}
        }
    }

    let mut routine: Vec<String> = Vec::new();
    match asked_for.len() {
        0 => {}
        1 => routine.push(format!("installed {}", asked_for[0])),
        n => routine.push(format!("installed {} packages{}", n, listed(&asked_for))),
    }
    match upgraded.len() {
        0 => {}
        1 | 2 => routine.push(format!("upgraded {}", join_naturally(&upgraded))),
        n => routine.push(format!("upgraded {} packages", n)),
    }

    match (summarize(group), routine.is_empty()) {
        (Some(notable), true) => Some(notable),
        (Some(notable), false) => Some(format!(
            "{} It also {}.",
            notable,
            join_naturally_owned(&routine)
        )),
        (None, false) => Some(format!(
            "Your system update {} {}.",
            relative_day(first),
            join_naturally_owned(&routine)
        )),
        (None, true) => None,
    }
}

fn number_word(n: usize) -> String {
    match n {
        1 => "one".into(),
        2 => "two".into(),
        3 => "three".into(),
        n => n.to_string(),
    }
}

/// "MAVIS, what's changed?" → ["whats", "changed"].
fn words(text: &str) -> Vec<String> {
    let flat: String = text
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect();
    let mut out: Vec<String> = flat.split_whitespace().map(String::from).collect();
    while out
        .first()
        .is_some_and(|w| matches!(w.as_str(), "hey" | "ok" | "okay" | "so" | "mavis"))
    {
        out.remove(0);
    }
    out
}

/// "What changed recently?" Every word must be in the grammar, so "what
/// changed in the Rust release?" still goes to the model. "installed" and
/// "removed" need an event word ("what got installed"), since "what's
/// installed?" is an inventory question.
#[rustfmt::skip]
pub fn is_change_question(text: &str) -> bool {
    const OPENERS: &[&str] = &["what", "whats", "anything", "has", "have", "did", "was", "were", "any"];
    const CHANGED: &[&str] = &["changed", "updated", "upgraded"];
    const STATE: &[&str] = &["installed", "removed", "uninstalled"];
    const EVENT: &[&str] = &[
        "was", "were", "got", "gotten", "been", "new", "recently", "lately", "today",
        "yesterday", "week", "morning", "afternoon", "evening", "since", "days",
    ];
    const GRAMMAR: &[&str] = &[
        "what", "whats", "has", "have", "had", "anything", "something", "been", "got",
        "gotten", "was", "were", "did", "any", "new", "change", "changed", "installed",
        "updated", "upgraded", "removed", "uninstalled", "recently", "lately", "today",
        "yesterday", "this", "last", "past", "week", "morning", "afternoon", "evening",
        "few", "days", "on", "in", "to", "my", "the", "system", "computer", "machine",
        "laptop", "pc", "since", "so", "far", "there", "packages", "package", "software",
    ];

    let w = words(text);
    let Some(first) = w.first() else { return false };
    if !OPENERS.contains(&first.as_str()) {
        return false;
    }
    if !w.iter().all(|x| GRAMMAR.contains(&x.as_str())) {
        return false;
    }
    let has = |set: &[&str]| w.iter().any(|x| set.contains(&x.as_str()));
    has(CHANGED) || (has(&["change"]) && has(&["did"])) || (has(STATE) && has(EVENT))
}

/// "What did that update do?" Every word must be in the grammar, so
/// "what is the latest update for Firefox" still goes to the model.
pub fn is_update_question(text: &str) -> bool {
    const UPDATE: &[&str] = &["update", "upgrade"];
    const POINTER: &[&str] = &["that", "the", "last", "latest", "recent", "this"];
    const GRAMMAR: &[&str] = &[
        "what", "did", "does", "was", "were", "in", "that", "the", "last", "latest", "recent",
        "this", "my", "system", "update", "upgrade", "do", "change", "install", "actually",
        "exactly", "just", "bring", "include",
    ];

    let w = words(text);
    if w.first().map(String::as_str) != Some("what") {
        return false;
    }
    if !w.iter().all(|x| GRAMMAR.contains(&x.as_str())) {
        return false;
    }
    let has = |set: &[&str]| w.iter().any(|x| set.contains(&x.as_str()));
    has(UPDATE) && has(POINTER)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(tag: &str) -> (SentinelStore, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "mavis_speech_{}_{}_{}.db",
            tag,
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        (SentinelStore::new(&path).expect("store"), path)
    }

    fn ago(secs: i64) -> DateTime<Utc> {
        // Whole seconds, matching what the store keeps.
        DateTime::from_timestamp(Utc::now().timestamp() - secs, 0).unwrap()
    }

    fn installed(name: &str, requested: bool, secs_ago: i64) -> Change {
        Change::new(
            ChangeKind::PackageInstalled {
                name: name.into(),
                version: "1.0".into(),
                requested,
            },
            "pacman",
            ago(secs_ago),
        )
    }

    fn upgraded(name: &str, secs_ago: i64) -> Change {
        Change::new(
            ChangeKind::PackageUpgraded {
                name: name.into(),
                from: "1.0".into(),
                to: "2.0".into(),
            },
            "pacman",
            ago(secs_ago),
        )
    }

    const HOUR: i64 = 3600;
    const DAY: i64 = 86_400;

    #[test]
    fn change_questions_are_recognised() {
        for q in [
            "What changed recently?",
            "what's changed",
            "MAVIS, what changed?",
            "Hey MAVIS, has anything changed on my system?",
            "What got installed yesterday?",
            "What was installed this week?",
            "anything new installed lately",
            "What's been updated since this morning?",
            "Did anything change on my computer?",
            "what packages were removed today",
        ] {
            assert!(is_change_question(q), "should match: {q}");
        }
    }

    #[test]
    fn other_questions_are_left_for_the_model() {
        for q in [
            "What changed in the Rust release?",
            "what's installed",
            "what have you changed",
            "How do I see what changed in git?",
            "is firefox installed",
            "changed my mind",
            "what change",
            "any change in the weather",
            "what time is it",
            "",
        ] {
            assert!(!is_change_question(q), "should not match: {q}");
        }
    }

    #[test]
    fn update_questions_are_recognised() {
        for q in [
            "What did that update do?",
            "what did the update do",
            "What was in the last update?",
            "MAVIS, what did the latest system update install?",
            "what exactly did that update change",
        ] {
            assert!(is_update_question(q), "should match: {q}");
        }
    }

    #[test]
    fn update_questions_need_a_particular_update() {
        for q in [
            "what is the latest update for firefox",
            "what update",
            "should I update",
            "what did you do",
            "what did that do",
        ] {
            assert!(!is_update_question(q), "should not match: {q}");
        }
    }

    #[test]
    fn matching_survives_non_ascii() {
        // Curly apostrophes and accents from Whisper must not panic.
        assert!(is_change_question("What’s changed?"));
        assert!(!is_change_question("qué cambió"));
        assert!(!is_update_question("what did thé update do"));
    }

    #[test]
    fn nothing_pending_means_nothing_said() {
        assert!(announcement(&[], MAX_LEAD_SENTENCES).is_none());
        // Routine only: must not produce an empty sentence.
        assert!(announcement(&[upgraded("firefox", 10)], 2).is_none());
    }

    #[test]
    fn one_update_is_one_sentence() {
        let pending = vec![
            installed("hyprland", false, 10),
            installed("foo", false, 11),
        ];
        let a = announcement(&pending, MAX_LEAD_SENTENCES).unwrap();
        assert!(
            a.text
                .starts_with("Your system update today pulled in 2 packages"),
            "{}",
            a.text
        );
        assert_eq!(a.fingerprints.len(), 2);
        assert!(!a.text.contains("earlier"), "{}", a.text);
    }

    /// Newest read, rest counted, all marked.
    #[test]
    fn a_backlog_is_counted_not_read() {
        let pending = vec![
            installed("oldest", false, 4 * DAY),
            installed("older", false, 3 * DAY),
            installed("newer", false, 2 * DAY),
            installed("newest", false, DAY),
        ];
        let a = announcement(&pending, 2).unwrap();
        assert!(a.text.contains("newer"), "{}", a.text);
        assert!(a.text.contains("newest"), "{}", a.text);
        assert!(!a.text.contains("oldest"), "{}", a.text);
        assert!(a.text.contains("also 2 earlier changes"), "{}", a.text);
        assert_eq!(a.fingerprints.len(), 4, "the counted ones are covered too");
        assert!(a.text.find("newer").unwrap() < a.text.find("newest").unwrap());
        assert_eq!(a.about, Some(ago(DAY)));
    }

    #[test]
    fn one_earlier_update_reads_in_the_singular() {
        let pending = vec![
            installed("a", false, 3 * DAY),
            installed("b", false, 2 * DAY),
            installed("c", false, DAY),
        ];
        let a = announcement(&pending, 2).unwrap();
        assert!(a.text.contains("also one earlier change"), "{}", a.text);
    }

    #[test]
    fn pending_changes_lead_once_and_only_once() {
        let (store, path) = temp_store("lead_once");
        store
            .record_all(&[
                installed("hyprland", false, HOUR),
                upgraded("firefox", HOUR),
            ])
            .unwrap();

        let first = respond(&store, "what's the weather", None, None, Utc::now()).unwrap();
        let Some(Turn::Lead(spoken)) = first else {
            panic!("expected a lead, got {:?}", first)
        };
        assert!(spoken.text.contains("hyprland"), "{}", spoken.text);
        assert!(
            !spoken.text.contains("firefox"),
            "routine is never volunteered"
        );
        assert_eq!(spoken.about, Some(ago(HOUR)));

        let second = respond(&store, "and tomorrow?", None, None, Utc::now()).unwrap();
        assert_eq!(second, None, "told once");

        assert_eq!(store.count().unwrap(), 2);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_question_about_changes_is_answered_not_led() {
        let (store, path) = temp_store("answer");
        store
            .record_all(&[
                installed("hyprland", false, DAY + 10),
                installed("neovim", true, DAY + 5),
                upgraded("firefox", DAY),
                upgraded("mesa", DAY),
                upgraded("glibc", DAY),
            ])
            .unwrap();

        let turn = respond(&store, "what changed recently?", None, None, Utc::now()).unwrap();
        let Some(Turn::Answer(spoken)) = turn else {
            panic!("expected an answer, got {:?}", turn)
        };
        assert!(
            spoken.text.contains("pulled in hyprland"),
            "{}",
            spoken.text
        );
        assert!(spoken.text.contains("installed neovim"), "{}", spoken.text);
        assert!(
            spoken.text.contains("upgraded 3 packages"),
            "{}",
            spoken.text
        );

        // Answered, so not led with next time.
        let next = respond(&store, "thanks", None, None, Utc::now()).unwrap();
        assert_eq!(next, None);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_quiet_week_says_so() {
        let (store, path) = temp_store("quiet");
        store
            .record_all_announced(&[installed("old", false, 30 * DAY)])
            .unwrap();
        let turn = respond(&store, "has anything changed?", None, None, Utc::now()).unwrap();
        assert_eq!(
            turn,
            Some(Turn::Answer(Spoken {
                text: "Nothing changed on the system in the last week.".into(),
                about: None
            }))
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_given_window_is_used_and_named() {
        let (store, path) = temp_store("window");
        store
            .record_all_announced(&[installed("recent", false, HOUR)])
            .unwrap();
        let window = Window {
            start: Utc::now() - Duration::days(2),
            end: Utc::now() - Duration::days(1),
            label: "yesterday".into(),
        };
        let turn = respond(
            &store,
            "what changed yesterday",
            Some(window),
            None,
            Utc::now(),
        )
        .unwrap();
        let Some(Turn::Answer(spoken)) = turn else {
            panic!()
        };
        assert_eq!(spoken.text, "Nothing changed on the system yesterday.");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn many_updates_are_trimmed_to_the_latest_three() {
        let (store, path) = temp_store("many");
        let changes: Vec<Change> = (1..=5)
            .map(|d| upgraded(&format!("pkg{d}"), d * DAY / 2))
            .collect();
        store.record_all_announced(&changes).unwrap();
        let turn = respond(&store, "what changed", None, None, Utc::now()).unwrap();
        let Some(Turn::Answer(spoken)) = turn else {
            panic!()
        };
        assert!(
            spoken
                .text
                .starts_with("There were 5 changes in the last week. The latest three:"),
            "{}",
            spoken.text
        );
        assert!(spoken.text.contains("pkg1"), "newest kept: {}", spoken.text);
        assert!(
            !spoken.text.contains("pkg5"),
            "oldest dropped: {}",
            spoken.text
        );
        let _ = std::fs::remove_file(&path);
    }

    /// A newer routine update must not be mistaken for "that update".
    #[test]
    fn that_update_means_the_one_last_mentioned() {
        let (store, path) = temp_store("that_update");
        store
            .record_all(&[
                installed("hyprland", false, 2 * DAY),
                upgraded("dms-shell", 2 * DAY),
            ])
            .unwrap();
        store.record_all(&[upgraded("firefox", DAY)]).unwrap(); // newer, routine

        let lead = respond(&store, "hello", None, None, Utc::now()).unwrap();
        let Some(Turn::Lead(told)) = lead else {
            panic!("{:?}", lead)
        };

        let turn = respond(
            &store,
            "what did that update do?",
            None,
            told.about,
            Utc::now(),
        )
        .unwrap();
        let Some(Turn::Answer(spoken)) = turn else {
            panic!()
        };
        assert!(spoken.text.contains("hyprland"), "{}", spoken.text);
        assert!(
            spoken.text.contains("upgraded dms-shell"),
            "{}",
            spoken.text
        );
        assert!(!spoken.text.contains("firefox"), "{}", spoken.text);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn that_update_without_context_is_the_most_recent() {
        let (store, path) = temp_store("latest");
        store
            .record_all_announced(&[upgraded("old", 2 * DAY)])
            .unwrap();
        store
            .record_all_announced(&[upgraded("firefox", DAY)])
            .unwrap();
        let turn = respond(
            &store,
            "what did the last update do",
            None,
            None,
            Utc::now(),
        )
        .unwrap();
        let Some(Turn::Answer(spoken)) = turn else {
            panic!()
        };
        assert!(spoken.text.contains("upgraded firefox"), "{}", spoken.text);
        assert!(!spoken.text.contains("old"), "{}", spoken.text);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn that_update_on_an_empty_store_says_so() {
        let (store, path) = temp_store("empty");
        let turn = respond(&store, "what did that update do", None, None, Utc::now()).unwrap();
        let Some(Turn::Answer(spoken)) = turn else {
            panic!()
        };
        assert_eq!(spoken.text, "I haven't seen any system updates yet.");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn nothing_pending_and_no_question_is_silence() {
        let (store, path) = temp_store("silence");
        store
            .record_all_announced(&[installed("hyprland", false, HOUR)])
            .unwrap();
        store.record_all(&[upgraded("firefox", 10)]).unwrap();
        assert_eq!(
            respond(&store, "open firefox", None, None, Utc::now()).unwrap(),
            None
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_routine_only_update_is_described_plainly() {
        let line =
            describe_transaction(&[upgraded("firefox", 10), installed("gimp", true, 10)]).unwrap();
        assert_eq!(
            line,
            "Your system update today installed gimp and upgraded firefox."
        );
    }

    #[test]
    fn descriptions_are_speakable() {
        let line = describe_transaction(&[
            installed("hyprland", false, 10),
            upgraded("a", 10),
            upgraded("b", 10),
            upgraded("c", 10),
        ])
        .unwrap();
        assert!(!line.contains('\n') && !line.contains('*') && !line.contains('#'));
        assert!(line.ends_with('.'), "{}", line);
        assert!(line.contains("It also upgraded 3 packages."), "{}", line);
    }
}