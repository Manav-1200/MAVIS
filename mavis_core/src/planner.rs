// mavis_core/src/planner.rs
use crate::context_snapshot::{AppEntry, WindowInfo};
use crate::event_bus::EventBus;
use crate::memory::entities::{EntityKind, EntityStore};
use crate::memory::long_term::LongTermMemory;
use crate::memory::recall::{build_fts_query, RecallStore};
use crate::memory::working::WorkingMemory;
use crate::models::event::{Event, EventType};
use crate::safety::SharedAwaiting;
use crate::sentinel::speech;
use crate::sentinel::store::SentinelStore;
use log::{info, warn};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

// Phase 6 privacy gate: each context source is off by default and must be
// explicitly opted into. This is the specific gate Phase 6 itself asks for —
// not the fuller 5-tier system Phase 8 builds later.
/// Is the user asking about what they copied?
fn mentions_clipboard(text: &str) -> bool {
    let lower = text.to_lowercase();
    ["clipboard", "copied", "i copy", "just copy", "paste", "pasted"]
        .iter()
        .any(|w| lower.contains(w))
}

fn context_source_enabled(env_var: &str) -> bool {
    matches!(std::env::var(env_var).as_deref(), Ok("1") | Ok("true"))
}

// Common code file extensions, used only by the generic fallback below —
// not needed for exact-matched editors.
const CODE_EXTENSIONS: &[&str] = &[
    ".rs", ".py", ".js", ".ts", ".jsx", ".tsx", ".go", ".java", ".c", ".cpp", ".h",
    ".rb", ".php", ".html", ".css", ".json", ".toml", ".yaml", ".yml", ".md",
    ".sh", ".lua", ".swift", ".kt", ".cs", ".sql",
];

fn find_filename_in_title(title: &str) -> Option<&str> {
    title.split_whitespace().find(|word| CODE_EXTENSIONS.iter().any(|ext| word.ends_with(ext)))
}

// Terminals set their title to the running command, which on this setup
// meant MAVIS reported its own launch line (a wall of MAVIS_CONTEXT_*=1
// assignments) as "what the user is doing". Strip leading VAR=value tokens
// so the actual command survives — "cargo run" instead of the env prefix.
fn strip_env_prefix(title: &str) -> &str {
    let mut rest = title.trim_start();
    loop {
        let token = match rest.split_whitespace().next() {
            Some(t) => t,
            None => return rest,
        };
        let (key, has_eq) = match token.split_once('=') {
            Some((k, _)) => (k, true),
            None => (token, false),
        };
        let is_env_assignment = has_eq
            && !key.is_empty()
            && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && key.chars().all(|c| !c.is_ascii_lowercase());
        if !is_env_assignment {
            return rest;
        }
        match rest[token.len()..].trim_start() {
            "" => return rest, // nothing after the assignment; keep as-is
            r => rest = r,
        }
    }
}

/// The focused window as plain facts. Nothing here knows an app by name:
/// a terminal's title is its command and an editor's title names its file,
/// so the model reads them the way a person glancing at the screen would.
fn describe_active_window(window: &WindowInfo) -> String {
    let title = without_redirection(strip_env_prefix(&window.window_title)).trim();
    let mut line = if title.is_empty() {
        format!("The focused window is {}.", window.app_name)
    } else {
        format!("The focused window is {} — \"{}\".", window.app_name, title)
    };
    if let Some(file) = find_filename_in_title(title) {
        line.push_str(&format!(" Its title names the file {}.", file));
    }
    line
}

/// A command title up to its first redirection: "cargo run 2>&1 | tee
/// log" is "cargo run". Read aloud, the rest was symbols.
fn without_redirection(title: &str) -> &str {
    let is_redirect = |t: &str| {
        matches!(t, "&&" | "||" | ";" | "&>" | ">" | ">>")
            || t.trim_start_matches(|c: char| c.is_ascii_digit()).starts_with('>')
    };
    let mut end = title.len();
    let mut at = 0;
    for token in title.split_whitespace() {
        let start = at + title[at..].find(token).unwrap_or(0);
        if is_redirect(token) {
            end = start;
            break;
        }
        at = start + token.len();
    }
    &title[..end]
}

/// Every open window, counted here rather than by the model, which miscounts
/// lists. Workspaces are the numbers the user sees.
fn describe_open_windows(windows: &[WindowInfo], current: Option<u64>) -> Option<String> {
    let mut apps: Vec<(String, usize, Vec<u64>)> = Vec::new();
    for w in windows {
        if w.app_name.is_empty() || w.app_name == "unknown" {
            continue;
        }
        let at = match apps.iter().position(|(name, _, _)| *name == w.app_name) {
            Some(i) => i,
            None => {
                apps.push((w.app_name.clone(), 0, Vec::new()));
                apps.len() - 1
            }
        };
        apps[at].1 += 1;
        if let Some(ws) = w.workspace_id {
            if !apps[at].2.contains(&ws) {
                apps[at].2.push(ws);
            }
        }
    }
    if apps.is_empty() {
        return None;
    }
    apps.sort_by(|a, b| a.0.cmp(&b.0));

    let listed: Vec<String> = apps
        .iter()
        .map(|(name, count, spaces)| {
            let mut notes = Vec::new();
            if *count > 1 {
                notes.push(format!("{} windows", count));
            }
            match spaces.len() {
                0 => {}
                1 => notes.push(format!("workspace {}", spaces[0])),
                _ => notes.push(format!("workspaces {}", join_numbers(spaces))),
            }
            if notes.is_empty() {
                name.clone()
            } else {
                format!("{} ({})", name, notes.join(", "))
            }
        })
        .collect();
    // Windows and apps are counted separately; the model confuses the two.
    let total: usize = apps.iter().map(|(_, count, _)| count).sum();
    let mut parts = vec![format!(
        "{} {} open, from {} {}: {}.",
        total,
        if total == 1 { "window is" } else { "windows are" },
        apps.len(),
        if apps.len() == 1 { "application" } else { "applications" },
        listed.join("; ")
    )];

    let mut used: Vec<u64> = apps.iter().flat_map(|(_, _, s)| s.iter().copied()).collect();
    used.sort_unstable();
    used.dedup();
    if !used.is_empty() {
        parts.push(format!(
            "Windows are on {} {}: {}.",
            used.len(),
            if used.len() == 1 { "workspace" } else { "workspaces" },
            join_numbers(&used)
        ));
    }
    if let Some(ws) = current {
        parts.push(format!("The user is on workspace {}.", ws));
    }
    Some(parts.join(" "))
}

fn join_numbers(numbers: &[u64]) -> String {
    let mut sorted = numbers.to_vec();
    sorted.sort_unstable();
    let words: Vec<String> = sorted.iter().map(|n| n.to_string()).collect();
    match words.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {}", rest.join(", "), last),
        _ => words.join(""),
    }
}

// ---------------------------------------------------------------------
// Action intents
// ---------------------------------------------------------------------
// Deterministic phrase matching, deliberately not LLM tool-calling: a 3.8B
// model has been unreliable at following even simple format rules here, and
// putting it in charge of emitting actions would make its mistakes
// consequential rather than just wrong.
//
// Note what's absent: there is no path from speech to the executor's
// `shell` action. That stays unreachable until Phase 8 builds real
// permissions — an unrestricted `sh -c` driven by voice transcription is
// exactly the hole the initial audit flagged.

/// Normalise for matching: lowercase, keep only alphanumerics and spaces.
/// "Code - OSS" and "code oss" should compare equal.
fn normalize(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ')
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Find the app the user asked for, tolerant of how people actually speak.
/// Tries progressively looser matches; returns None rather than guessing,
/// so "open my heart" falls through to the LLM instead of failing to spawn.
fn find_app<'a>(spoken: &str, apps: &'a [AppEntry]) -> Option<&'a AppEntry> {
    let s = normalize(spoken);
    if s.is_empty() {
        return None;
    }

    // Exact application name.
    if let Some(a) = apps.iter().find(|a| normalize(&a.name) == s) {
        return Some(a);
    }
    // Exact binary name — "code-oss" as spoken. Several entries can share
    // one binary ("filey" is also Bulk Rename, `filey --bulk-rename`):
    // the one named after it wins, then the one with the fewest arguments.
    if let Some(a) = apps
        .iter()
        .filter(|a| {
            a.exec
                .split_whitespace()
                .next()
                .and_then(|b| b.rsplit('/').next())
                .is_some_and(|b| normalize(b) == s)
        })
        .min_by_key(|a| {
            let named = normalize(&a.name).split_whitespace().any(|w| w == s);
            let args = a.exec.split_whitespace().skip(1).filter(|x| !x.starts_with('%')).count();
            (!named, args)
        })
    {
        return Some(a);
    }
    // Name begins with what was said — "code" finds "Code - OSS".
    if let Some(a) = apps.iter().find(|a| normalize(&a.name).starts_with(&s)) {
        return Some(a);
    }
    // Every spoken word appears in the name — "notes" finds "Quick Notes".
    let words: Vec<&str> = s.split_whitespace().collect();
    if let Some(a) = apps.iter().find(|a| {
        let n = normalize(&a.name);
        words.iter().all(|w| n.contains(w))
    }) {
        return Some(a);
    }

    // The app's name appears in what was said — "open webby browser"
    // finds Webby, "open the code editor" doesn't find anything unless
    // an app is actually called that. People add words the launcher entry
    // doesn't have ("browser", "please", "for me"), and the previous rules
    // all required the opposite: that every spoken word be in the name.
    // The longest match wins, so "visual studio code" beats "code".
    let padded = format!(" {} ", s);
    if let Some(a) = apps
        .iter()
        .filter(|a| {
            let n = normalize(&a.name);
            !n.is_empty() && n.len() >= 3 && padded.contains(&format!(" {} ", n))
        })
        .max_by_key(|a| normalize(&a.name).len())
    {
        return Some(a);
    }
    None
}

/// `find_app`, then a near miss — for "open" and "close" only, where the
/// whole target is meant to be an app name.
fn find_named_app<'a>(spoken: &str, apps: &'a [AppEntry]) -> Option<&'a AppEntry> {
    find_app(spoken, apps).or_else(|| misheard_app(&normalize(spoken), apps))
}

/// Apps whose kind the words name: "terminal", "the browser", "video
/// player". From GenericName and Categories, so no app names are needed.
fn apps_of_kind<'a>(spoken: &str, apps: &'a [AppEntry]) -> Vec<&'a AppEntry> {
    const FILLER: &[&str] = &["the", "a", "my", "app", "application", "program", "window"];
    let words: Vec<String> = normalize(spoken)
        .split_whitespace()
        .filter(|w| !FILLER.contains(w))
        .map(String::from)
        .collect();
    if words.is_empty() {
        return Vec::new();
    }
    apps.iter()
        .filter(|a| words.iter().all(|w| a.kinds.iter().any(|k| k == w)))
        .collect()
}

/// The fact that several apps fit, for MAVIS to ask which.
fn which_one(names: &[String]) -> String {
    let names: Vec<&str> = names.iter().take(3).map(String::as_str).collect();
    format!("Nothing was done: more than one app fits ({}). Ask which one they mean.", names.join(", "))
}

/// A one-word name Whisper got slightly wrong: "tuner" for Filey. Matched
/// against each word of the app's name and its binary; only a single
/// closest app counts, so two near names give nothing rather than a guess.
fn misheard_app<'a>(spoken: &str, apps: &'a [AppEntry]) -> Option<&'a AppEntry> {
    if spoken.contains(' ') || spoken.len() < 4 {
        return None;
    }
    let near = |w: &str| {
        // One letter off for short names, two for longer ones.
        let allowed = if w.len().max(spoken.len()) >= 6 { 2 } else { 1 };
        let d = edit_distance(spoken, w);
        (w.len() >= 4 && d <= allowed).then_some(d)
    };
    // Ranked by distance, then a match on the name over one on the binary
    // only: "Bulk Rename" shares Filey's binary and would otherwise tie.
    let mut best: Option<((usize, bool), &AppEntry)> = None;
    let mut tied = false;
    for app in apps {
        let binary = crate::util::split_command_line(&app.exec)
            .into_iter()
            .next()
            .map(|b| normalize(b.rsplit('/').next().unwrap_or(&b)))
            .unwrap_or_default();
        let by_name = normalize(&app.name).split_whitespace().filter_map(near).min();
        let distance = match (by_name, near(&binary)) {
            (Some(n), Some(b)) if b < n => (b, true),
            (Some(n), _) => (n, false),
            (None, Some(b)) => (b, true),
            (None, None) => continue,
        };
        match best {
            Some((d, a)) if d == distance && a.name != app.name => tied = true,
            Some((d, _)) if d <= distance => {}
            _ => {
                best = Some((distance, app));
                tied = false;
            }
        }
    }
    if tied {
        return None;
    }
    best.map(|(_, a)| a)
}

/// Strip a leading address to MAVIS so "MAVIS, open webby" matches the
/// same as "open webby".
///
/// Uses `get(..n)` rather than `[..n]`. The old form indexed by raw byte
/// offset, which panics when a multi-byte character straddles the prefix
/// length — `strip_address("ééé mavis ...")` panicked on byte 5, inside
/// the second 'é'. Whisper produces non-ASCII regularly (curly
/// apostrophes, em-dashes, accented loanwords), and because a panic in a
/// spawned task ends that task silently, the effect was not a crash but a
/// planner that stopped answering for the rest of the session while
/// everything else kept running.
///
/// `get` returns None on a non-boundary instead of panicking, which
/// correctly means "this prefix doesn't match here".
fn strip_address(text: &str) -> &str {
    let t = text.trim_start();
    for prefix in ["hey mavis", "ok mavis", "okay mavis", "mavis"] {
        let matches_prefix = t
            .get(..prefix.len())
            .map(|head| head.eq_ignore_ascii_case(prefix))
            .unwrap_or(false);
        if matches_prefix {
            // Safe: the `get` above proved prefix.len() is a boundary.
            return t[prefix.len()..].trim_start_matches([',', ' ', '.', '?', '!']);
        }
    }
    // Whisper often mishears the name by a letter. A first
    // word one letter away from it counts, when punctuation sets it off the
    // way Whisper writes someone being addressed.
    if let Some(end) = t.find([',', '.', '!', '?']) {
        if is_misheard_name(&t[..end]) {
            return t[end..].trim_start_matches([',', ' ', '.', '!', '?']);
        }
    }
    t
}

fn is_misheard_name(word: &str) -> bool {
    let w = word.trim().to_lowercase();
    (4..=6).contains(&w.len()) && w.chars().all(|c| c.is_ascii_alphabetic()) && edit_distance(&w, "mavis") <= 1
}

/// Levenshtein distance; inputs here are a single short word.
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for j in 0..b.len() {
            let cur = row[j + 1];
            row[j + 1] = if ca == b[j] { prev } else { 1 + prev.min(cur).min(row[j]) };
            prev = cur;
        }
    }
    row[b.len()]
}

/// Lines waiting before they are folded into the summary.
const SUMMARISE_AT: usize = 8;

/// What came before the recent turns: the model's summary, then the lines
/// not yet folded into it.
fn earlier_in_conversation(summary: &str, waiting: &[String]) -> Option<String> {
    let mut parts = Vec::new();
    if !summary.trim().is_empty() {
        parts.push(summary.trim().to_string());
    }
    if !waiting.is_empty() {
        parts.push(format!("Then: {}", waiting.join(" / ")));
    }
    (!parts.is_empty()).then(|| format!("Earlier in this conversation: {}", parts.join(" ")))
}

/// "tube.test", "docs.rs/serde": one word, a dot, letters after it.
fn looks_like_domain(target: &str) -> bool {
    let host = target.split('/').next().unwrap_or("");
    !target.contains(' ')
        && host.rsplit_once('.').is_some_and(|(name, tld)| {
            !name.is_empty() && tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic())
        })
}

/// The window, named after its installed app where one owns it.
fn with_app_name(w: &WindowInfo, apps: &[AppEntry]) -> WindowInfo {
    let mut w = w.clone();
    let words = window_app_words(&w.app_name);
    // Filey's binary is also Bulk Rename's: the one named like the window wins.
    let owner = apps.iter().filter(|a| app_owns(Some(a), "", &w)).min_by_key(|a| {
        let named = normalize(&a.name).split_whitespace().any(|n| words.split(' ').any(|x| x == n));
        (!named, a.exec.split_whitespace().count())
    });
    if let Some(app) = owner {
        w.app_name = app.name.clone();
    }
    w
}

/// Whether window `w` belongs to `app` (or, with no app, to the words).
fn app_owns(app: Option<&AppEntry>, words: &str, w: &WindowInfo) -> bool {
    let window_words = window_app_words(&w.app_name);
    match app {
        Some(a) => {
            let binary = crate::util::split_command_line(&a.exec)
                .into_iter()
                .next()
                .map(|b| normalize(b.rsplit('/').next().unwrap_or(&b)))
                .unwrap_or_default();
            let compact_name: String = normalize(&a.name).split_whitespace().collect();
            let first_word = window_words.split(' ').next().unwrap_or_default();
            (!binary.is_empty() && (window_words == binary || window_words.split(' ').any(|w| w == binary)))
                || window_words == normalize(&a.name)
                || (!compact_name.is_empty() && first_word == compact_name)
        }
        None => window_words == normalize(words),
    }
}

/// Everything a plan says aloud, whether it's one action or a list.
pub(crate) fn plan_says(plan: &serde_json::Value) -> String {
    let actions = plan.as_array().cloned().unwrap_or_else(|| vec![plan.clone()]);
    actions
        .iter()
        .filter(|a| a.get("type").and_then(|t| t.as_str()) == Some("say"))
        .filter_map(|a| a.get("text").and_then(|t| t.as_str()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A turn of MAVIS's as the model wrote it: the tool call it made, or
/// what it said.
fn mavis_turn(payload: &serde_json::Value) -> String {
    match payload["did"].as_str() {
        Some(did) => did.to_string(),
        None => payload.get("plan").map(plan_says).unwrap_or_default(),
    }
}

/// The plan with what happened told at its end, once everything before it
/// has worked: a note when something didn't go as asked, or that it was
/// done. The model says it in its own words.
fn with_tell(plan: serde_json::Value, line: &str) -> serde_json::Value {
    let mut actions = plan.as_array().cloned().unwrap_or_else(|| vec![plan]);
    let notes: Vec<String> = actions
        .iter()
        .filter(|a| a["type"] == "note")
        .filter_map(|a| a["text"].as_str().map(String::from))
        .collect();
    actions.retain(|a| a["type"] != "note");
    // Undo's result is told once it's known.
    if actions.iter().any(|a| a["type"] == "undo") {
        return serde_json::Value::Array(actions);
    }
    let fact = if notes.is_empty() { format!("Done: {}", line) } else { notes.join(" ") };
    let tell = serde_json::json!({"type": "tell", "fact": fact});
    // Ending the session: told first, since nothing is heard after.
    let ends_session =
        actions.iter().any(|a| matches!(a["op"].as_str(), Some("lock" | "logout" | "suspend" | "poweroff" | "reboot")));
    if ends_session {
        actions.insert(0, tell);
    } else {
        actions.push(tell);
    }
    serde_json::Value::Array(actions)
}

/// The last `days` days, from local midnight, and how to name them.
fn days_window(days: u64) -> (chrono::DateTime<chrono::Local>, chrono::DateTime<chrono::Local>, String) {
    let now = chrono::Local::now();
    let midnight = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|t| t.and_local_timezone(chrono::Local).single())
        .unwrap_or(now);
    let start = midnight - chrono::Duration::days(days.saturating_sub(1) as i64);
    let label = match days {
        1 => "today".to_string(),
        2 => "since yesterday".to_string(),
        n => format!("over the last {} days", n),
    };
    (start, now, label)
}

/// A fact to remember must restate what the user said: most of its words
/// were theirs.
fn restates(fact: &str, said: &str) -> bool {
    let words = |t: &str| -> Vec<String> {
        normalize(&t.to_lowercase()).split_whitespace().filter(|w| w.len() > 2).map(String::from).collect()
    };
    let (fact, said) = (words(fact), words(said));
    !fact.is_empty() && fact.iter().filter(|w| said.contains(w)).count() * 2 >= fact.len()
}

/// What a request comes to: actions to run, or a fact for MAVIS to tell in
/// its own words (what couldn't be done, or what was looked up).
type Step = Result<serde_json::Value, String>;

/// The plan for a close request.
fn close_plan(close: Close) -> Step {
    match close {
        Close::Windows { app, ids } => {
            info!("Planner: close request — {} ({} window(s))", app, ids.len());
            Ok(serde_json::json!([{"type": "close", "app": app, "windows": ids}]))
        }
        Close::NotOpen(app) => Err(format!("Nothing was closed: {} isn't open.", app)),
        Close::Unsupported => Err("Nothing was closed: you can't close windows on this desktop yet.".into()),
        Close::NoFocus => Err("Nothing was closed: you can't tell which window is focused. Ask which app to close.".into()),
        Close::Mine(app) => Err(format!("Nothing was closed: that {} window is the one you run in.", app)),
        Close::Which(names) => Err(which_one(&names)),
    }
}

/// One thing MAVIS can do. This says what it does, not how anyone asks for
/// it: understanding the request is the model's job.
struct Action {
    name: &'static str,
    args: &'static str,
    what: &'static str,
    /// A system op runs as is; None means a planner builds it.
    op: Option<&'static str>,
    /// The argument names an app or site, so it must come from the user.
    names_target: bool,
}

const fn act(name: &'static str, args: &'static str, what: &'static str, op: Option<&'static str>, names_target: bool) -> Action {
    Action { name, args, what, op, names_target }
}

const ACTIONS: &[Action] = &[
    act("open", "<app>", "open an installed app", None, true),
    act("open_website", "<website>", "open a website in the browser", None, true),
    act("close", "<app>", "close an app's windows", None, true),
    act("close_focused", "", "close the window in front", None, false),
    act("close_last_opened", "", "close the app you opened last", None, false),
    act("workspace", "<number>", "switch to a workspace", None, false),
    act("fullscreen", "<app>", "make an app's window full screen", None, true),
    act("fullscreen_focused", "", "make the window in front full screen", None, false),
    act("volume_set", "<0-100>", "set the speaker volume in percent", Some("volume_set"), false),
    act("volume_up", "", "turn the volume up a step", Some("volume_up"), false),
    act("volume_down", "", "turn the volume down a step", Some("volume_down"), false),
    act("mute", "", "mute or unmute the speakers", Some("volume_mute"), false),
    act("brightness_set", "<0-100>", "set the screen brightness in percent", Some("brightness_set"), false),
    act("brightness_up", "", "make the screen brighter a step", Some("brightness_up"), false),
    act("brightness_down", "", "make the screen dimmer a step", Some("brightness_down"), false),
    act("bluetooth_on", "", "turn Bluetooth on", Some("bluetooth_on"), false),
    act("bluetooth_off", "", "turn Bluetooth off", Some("bluetooth_off"), false),
    act("wifi_on", "", "turn Wi-Fi on", Some("wifi_on"), false),
    act("wifi_off", "", "turn Wi-Fi off", Some("wifi_off"), false),
    act("lock", "", "lock the screen", Some("lock"), false),
    act("logout", "", "log the user out", Some("logout"), false),
    act("suspend", "", "put the computer to sleep", Some("suspend"), false),
    act("poweroff", "", "shut the computer down", Some("poweroff"), false),
    act("reboot", "", "restart the computer", Some("reboot"), false),
    act("play", "<what> [on <site or app>]", "play music or a video", None, false),
    act("search", "<what> [on <site or app>]", "search a site or app without playing anything", None, false),
    act("web_search", "<what>", "search the web", None, false),
    act("pause", "", "pause what's playing", Some("media_pause"), false),
    act("resume", "", "resume what was playing", Some("media_play"), false),
    act("next", "", "skip to the next track", Some("media_next"), false),
    act("previous", "", "go back a track", Some("media_previous"), false),
    act("undo", "", "undo your last action", None, false),
    act("stop", "", "stop talking, or pause what's playing if you're quiet", None, false),
    act("confirm", "", "yes to the question you just asked", None, false),
    act("cancel", "", "no to the question you just asked", None, false),
    act("system_changes", "<number>", "tell what changed on the system in the last number of days", None, false),
    act("update_details", "", "describe the system update you last mentioned, or the latest one", None, false),
    act("user_history", "<number>", "for a question about what the user did in the past: look back over that many days, then answer", None, false),
    act("remember", "<fact>", "keep a lasting fact the user told you about themselves", None, false),
    act("user_name", "<name>", "the user tells you their name or what to call them", None, false),
];

/// The actions as the model is shown them. Confirm and cancel only while
/// a question is open.
fn action_menu(question_open: bool) -> Vec<String> {
    ACTIONS
        .iter()
        .filter(|a| question_open || !matches!(a.name, "confirm" | "cancel"))
        .map(|a| match a.args {
            "" => format!("{}: {}", a.name, a.what),
            args => format!("{} {}: {}", a.name, args, a.what),
        })
        .collect()
}

/// Whether an app or site the model named was mentioned by the user, by
/// name or by kind ("the browser"). "Open About Desk" for "what is open on
/// workspace one" is rejected: the model made it up.
fn grounded(target: &str, said: &str, apps: &[AppEntry]) -> bool {
    let clean = |s: &str| normalize(&s.to_lowercase());
    let said = clean(said);
    let said_words: std::collections::HashSet<&str> = said.split_whitespace().collect();
    let compact: String = said.split_whitespace().collect();
    let mut allowed = std::collections::HashSet::new();
    for app in apps {
        let name = clean(&app.name);
        let named = !name.is_empty() && compact.contains(&name.split_whitespace().collect::<String>());
        if named || app.kinds.iter().any(|k| said_words.contains(k.as_str())) {
            allowed.extend(name.split_whitespace().map(str::to_string));
        }
    }
    clean(target)
        .split_whitespace()
        .all(|w| said_words.contains(w) || allowed.contains(w) || (w.len() >= 3 && compact.contains(w)))
}

/// Open an app (by name, or by kind when only one fits) or a website.
fn open_plan(target: &str, apps: &[AppEntry]) -> Step {
    let open = |target: String, args: Vec<String>| Ok(serde_json::json!([{"type": "app", "target": target, "args": args}]));
    if target.contains("://") {
        return open(target.to_string(), vec![]);
    }
    if looks_like_domain(target) {
        return open(format!("https://{}", target), vec![]);
    }
    let of_kind = apps_of_kind(target, apps);
    let app = find_named_app(target, apps).or(match of_kind.as_slice() {
        [one] => Some(*one),
        _ => None,
    });
    if let Some(app) = app {
        let mut parts = crate::util::split_command_line(&app.exec).into_iter();
        let bin = parts.next().unwrap_or_default();
        return open(bin, parts.collect());
    }
    if of_kind.len() > 1 {
        let names: Vec<String> = of_kind.iter().map(|a| a.name.clone()).collect();
        return Err(which_one(&names));
    }
    Err(format!("Nothing was opened: no installed app is called {}.", target))
}

/// Close an app's windows, never MAVIS's own; "this window" is the focused one.
fn close_target(
    target: &str,
    windows: &[WindowInfo],
    apps: &[AppEntry],
    spared: &std::collections::HashSet<u32>,
) -> Step {
    close_plan(close_app(Some(target), windows, apps, spared))
}

/// What closing an app (or, with None, the focused window) means right now.
/// MAVIS's own windows are left out.
fn close_app(
    target: Option<&str>,
    windows: &[WindowInfo],
    apps: &[AppEntry],
    spared: &std::collections::HashSet<u32>,
) -> Close {
    let found = match target {
        None => match windows.iter().find(|w| w.is_focused) {
            None => return Close::NoFocus,
            Some(w) => match w.id {
                None => return Close::Unsupported,
                Some(id) => {
                    let words = window_app_words(&w.app_name);
                    let name = find_app(&words, apps).map(|a| a.name.clone()).unwrap_or(words);
                    Close::Windows { app: name, ids: vec![id] }
                }
            },
        },
        Some(target) => {
            let target = normalize(&target.to_lowercase());
            let app = find_named_app(&target, apps);
            let matching: Vec<&WindowInfo> = windows
                .iter()
                .filter(|w| {
                    let words = window_app_words(&w.app_name);
                    words == target || app_owns(app, &target, w)
                })
                .collect();
            if matching.is_empty() {
                match app {
                    Some(a) => Close::NotOpen(a.name.clone()),
                    None => close_by_kind(&target, windows, apps).unwrap_or(Close::NotOpen(target.clone())),
                }
            } else if matching.iter().any(|w| w.id.is_none()) {
                Close::Unsupported
            } else {
                let name = app.map(|a| a.name.clone()).unwrap_or(target.clone());
                Close::Windows { app: name, ids: matching.iter().filter_map(|w| w.id).collect() }
            }
        }
    };
    let Close::Windows { app, ids } = found else { return found };
    let mine = |id: &u64| windows.iter().any(|w| w.id == Some(*id) && w.pid.is_some_and(|p| spared.contains(&p)));
    let kept: Vec<u64> = ids.iter().copied().filter(|id| !mine(id)).collect();
    if kept.is_empty() {
        Close::Mine(app)
    } else {
        Close::Windows { app, ids: kept }
    }
}

/// Full screen for an app's window, or with None the focused one.
fn fullscreen_plan(target: Option<&str>, windows: &[WindowInfo], apps: &[AppEntry]) -> Step {
    let window = match target {
        None => windows.iter().find(|w| w.is_focused),
        Some(t) => {
            let app = find_named_app(t, apps).or_else(|| apps_of_kind(t, apps).into_iter().next());
            windows.iter().filter(|w| app_owns(app, t, w)).max_by_key(|w| w.is_focused)
        }
    };
    match window.and_then(|w| w.id) {
        Some(id) => Ok(serde_json::json!([{"type": "fullscreen", "window": id}])),
        None => Err(match target {
            None => "Nothing changed: you can't tell which window they mean.".to_string(),
            Some(t) => format!("Nothing changed: {} isn't open.", t),
        }),
    }
}

/// The installed app a plan launches, by its program.
fn launched_app(plan: &serde_json::Value, apps: &[AppEntry]) -> Option<String> {
    let actions = plan.as_array().cloned().unwrap_or_else(|| vec![plan.clone()]);
    let target = actions
        .iter()
        .find(|a| a.get("type").and_then(|t| t.as_str()) == Some("app"))?
        .get("target")?
        .as_str()?
        .to_string();
    let program = |exec: &str| crate::util::split_command_line(exec).into_iter().next().unwrap_or_default();
    apps.iter().find(|a| program(&a.exec) == target || program(&a.exec).rsplit('/').next() == target.rsplit('/').next()).map(|a| a.name.clone())
}

/// What "close <app>" turned out to mean.
#[derive(Debug, PartialEq)]
enum Close {
    /// Open windows of the app, by the compositor's ids.
    Windows { app: String, ids: Vec<u64> },
    /// Installed, but nothing of it is open.
    NotOpen(String),
    /// It's open, but this desktop gives no way to close it.
    Unsupported,
    /// "Close it", with no focused window known.
    NoFocus,
    /// Every window it named is MAVIS's own terminal.
    Mine(String),
    /// Several apps of the kind named are open.
    Which(Vec<String>),
}

/// "Close the terminal": the open windows of apps of that kind. One app
/// is closed (after asking); several are asked about by name.
fn close_by_kind(target: &str, windows: &[WindowInfo], apps: &[AppEntry]) -> Option<Close> {
    let of_kind = apps_of_kind(target, apps);
    if of_kind.is_empty() {
        return None;
    }
    let windows_of = |a: &AppEntry| -> Vec<u64> {
        let binary = crate::util::split_command_line(&a.exec)
            .into_iter()
            .next()
            .map(|b| normalize(b.rsplit('/').next().unwrap_or(&b)))
            .unwrap_or_default();
        windows
            .iter()
            .filter(|w| {
                let words = window_app_words(&w.app_name);
                words == binary || words == normalize(&a.name)
            })
            .filter_map(|w| w.id)
            .collect()
    };
    let open: Vec<(&AppEntry, Vec<u64>)> =
        of_kind.into_iter().map(|a| (a, windows_of(a))).filter(|(_, ids)| !ids.is_empty()).collect();
    match open.as_slice() {
        [] => Some(Close::NotOpen(format!("The {}", target))),
        [(app, ids)] => Some(Close::Windows { app: app.name.clone(), ids: ids.clone() }),
        many => Some(Close::Which(many.iter().map(|(a, _)| a.name.clone()).collect())),
    }
}

/// The processes MAVIS runs inside: itself, its parent, theirs, up to
/// init. A window belonging to one of these is the terminal it was
/// started from.
pub fn own_process_tree() -> std::collections::HashSet<u32> {
    let mut pids = std::collections::HashSet::new();
    let mut pid = std::process::id();
    while pid > 1 && pids.insert(pid) {
        let stat = std::fs::read_to_string(format!("/proc/{}/stat", pid)).unwrap_or_default();
        // "pid (comm) state ppid …"; comm may hold spaces, so read after ')'.
        pid = stat
            .rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().nth(1))
            .and_then(|p| p.parse().ok())
            .unwrap_or(0);
    }
    pids
}

/// The open windows with the user's window marked focused. While the orb
/// has focus none is, and "close it" found nothing to close.
fn with_focus(windows: &[WindowInfo], active: Option<&WindowInfo>) -> Vec<WindowInfo> {
    let mut windows = windows.to_vec();
    if !windows.iter().any(|w| w.is_focused) {
        if let Some(id) = active.and_then(|a| a.id) {
            for w in windows.iter_mut().filter(|w| w.id == Some(id)) {
                w.is_focused = true;
            }
        }
    }
    windows
}

/// "org.example.Filey" → "filey", "surfy-browser" → "surfy browser".
fn window_app_words(app_id: &str) -> String {
    let last = app_id.rsplit('.').next().unwrap_or(app_id);
    let spaced: String = last
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    spaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub struct Planner {
    bus: Arc<EventBus>,
    working: Arc<RwLock<WorkingMemory>>,
    /// Scanned once at startup rather than per utterance — 100+ file reads
    /// on every spoken word would be wasteful. New installs are picked up
    /// on restart.
    apps: Vec<AppEntry>,
    recall: Arc<Mutex<RecallStore>>,
    long_term: Arc<Mutex<LongTermMemory>>,
    entities: Arc<Mutex<EntityStore>>,
    /// Only set when MAVIS_SENTINEL=1. Locked inside spawn_blocking only.
    sentinel: Option<Arc<std::sync::Mutex<SentinelStore>>>,
    /// Whether the permission gate has a question open.
    awaiting: SharedAwaiting,
    /// The update MAVIS last spoke about, for "what did that update do?".
    last_told: std::sync::Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    /// True while MAVIS is speaking, so "stop" knows what it means.
    speaking: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// Sites for "play" and "search", and the app last played in.
    media: Media,
    /// MAVIS's own process tree, whose windows are never closed.
    spared: std::collections::HashSet<u32>,
    /// The app MAVIS last opened, for "close what you just opened".
    last_opened: std::sync::Mutex<Option<String>>,
    /// What the user said last, for telling how its actions went.
    last_said: std::sync::Mutex<String>,
    /// Lines of conversation sent to be summarised, while that's under way.
    summarising: std::sync::Mutex<Option<usize>>,
}

impl Planner {
    /// `apps` comes from the platform layer — scanning is OS-specific, so
    /// it lives behind PlatformProvider rather than here.
    pub fn new(
        bus: Arc<EventBus>,
        working: Arc<RwLock<WorkingMemory>>,
        apps: Vec<AppEntry>,
        recall: Arc<Mutex<RecallStore>>,
        long_term: Arc<Mutex<LongTermMemory>>,
        entities: Arc<Mutex<EntityStore>>,
        sentinel: Option<Arc<std::sync::Mutex<SentinelStore>>>,
    ) -> Self {
        info!("Planner: {} installed applications available", apps.len());
        Self {
            bus,
            working,
            apps,
            recall,
            long_term,
            entities,
            sentinel,
            awaiting: SharedAwaiting::default(),
            last_told: std::sync::Mutex::new(None),
            speaking: None,
            media: Media::default(),
            spared: own_process_tree(),
            last_opened: std::sync::Mutex::new(None),
            last_said: std::sync::Mutex::new(String::new()),
            summarising: std::sync::Mutex::new(None),
        }
    }

    pub fn with_media(mut self, media: Media) -> Self {
        self.media = media;
        self
    }

    /// Share the permission gate's "question open" state, so the answer
    /// to a confirmation isn't planned as well.
    /// Share whether MAVIS is speaking.
    pub fn with_speaking(mut self, speaking: Arc<std::sync::atomic::AtomicBool>) -> Self {
        self.speaking = Some(speaking);
        self
    }

    pub fn with_confirmations(mut self, awaiting: SharedAwaiting) -> Self {
        self.awaiting = awaiting;
        self
    }

    pub async fn run(&mut self) {
        let mut rx = self.bus.subscribe();
        info!("Planner: listening for events");
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Err(e) = self.handle_event(event).await {
                        warn!("Planner error: {}", e);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    warn!("Planner lagged by {} events", n);
                }
            }
        }
        info!("Planner: shutting down");
    }

    async fn handle_event(&self, event: Event) -> anyhow::Result<()> {
        match event.event_type {
            EventType::UserIntent => {
                info!("Planner: received UserIntent — generating plan");
                self.plan_intent(event).await?;
            }
            EventType::WorkerResponse => {
                info!("Planner: received WorkerResponse");
                self.handle_worker_response(event).await?;
            }
            EventType::ActionComplete => self.action_done(&event).await,
            EventType::Tell => {
                let fact = event.payload["fact"].as_str().unwrap_or_default().to_string();
                let said = self.last_said.lock().unwrap_or_else(|p| p.into_inner()).clone();
                self.report(&said, &fact).await;
            }
            _ => {}
        }
        Ok(())
    }

    async fn plan_intent(&self, event: Event) -> anyhow::Result<()> {
        let intent = event
            .payload
            .get("text")
            .and_then(|v| v.as_str())
            .or_else(|| event.payload.get("intent").and_then(|v| v.as_str()))
            .unwrap_or("unknown");
        // "MAVIS, …" (or a mishearing of it) is who it's said to, not what.
        let intent = match strip_address(intent) {
            "" => intent,
            rest => rest,
        };
        *self.last_said.lock().unwrap_or_else(|p| p.into_inner()) = intent.to_string();

        let source = event
            .payload
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let user_message = if source == "voice" {
            format!("[Voice] {}", intent)
        } else {
            intent.to_string()
        };

        let mut working_memory = self.build_working_memory(intent, event.id, None).await;
        // Pending system changes reach the model as news, told before the reply.
        if let Some(spoken) = self.sentinel_lead().await {
            info!("Planner: leading with pending system changes");
            working_memory.push(serde_json::json!({ "source": "news", "content": spoken.text }));
        }
        let targets: Vec<String> = {
            let wm = self.working.read().await;
            let mut names: Vec<String> = self.apps.iter().map(|a| a.name.clone()).collect();
            names.extend(wm.open_windows.iter().map(|w| with_app_name(w, &self.apps).app_name));
            names.sort();
            names.dedup();
            names
        };
        // One call: the model answers "DO <action>" or "SAY <reply>". The
        // system prompt lives in Python; the actions are listed from code.
        self.bus.publish(Event {
            id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            source: "planner".to_string(),
            event_type: EventType::WorkerRequest,
            payload: serde_json::json!({
                "request_type": "chat",
                "text": intent,
                "messages": [{"role": "user", "content": user_message}],
                // Replies are cut to three sentences after generation; this
                // is the backstop.
                "max_tokens": 128,
                // Lower than plain chat: choosing an action should be steady.
                "temperature": 0.3,
                "working_memory": working_memory,
                "actions": action_menu(self.question_open()),
                "apps": self.apps.iter().map(|a| a.name.clone()).collect::<Vec<_>>(),
                // What an app argument may be: installed apps and open windows.
                "targets": targets,
            }),
        });
        Ok(())
    }

    /// Carry out the action the model called (`line`; `call` is the call
    /// as it wrote it). What happened goes back to the model to tell.
    async fn carry_out(&self, line: &str, said: &str, call: &str) {
        match self.action_plan(line, said).await {
            Ok(plan) if plan.is_null() => {}
            Ok(plan) => {
                self.bus.publish(Event {
                    id: uuid::Uuid::new_v4(),
                    timestamp: chrono::Utc::now(),
                    source: "planner".to_string(),
                    event_type: EventType::PlanReady,
                    payload: serde_json::json!({ "plan": with_tell(plan, line.trim()), "did": call.trim() }),
                });
            }
            Err(fact) => self.report(said, &fact).await,
        }
    }

    /// The plan for the model's "DO <action>": actions, Null when it was
    /// handled here, or the fact of why it can't be done, never a guess.
    async fn action_plan(&self, line: &str, said: &str) -> Step {
        let record = |text: String| Err(format!("From the system's record: {}", text));
        // "volume_set <80>." still means 80: the model copies the menu's form.
        let line = line.trim().trim_end_matches(['.', '!']);
        let (name, arg) = match line.split_once(' ') {
            Some((n, a)) => (n, a.trim().trim_matches(['"', '<', '>'])),
            None => (line, ""),
        };
        let Some(action) = ACTIONS.iter().find(|a| a.name == name) else {
            return Err(format!("Nothing was done: \"{}\" isn't one of your actions. Ask what they want.", line));
        };
        if action.args.is_empty() != arg.is_empty() {
            return Err(format!("Nothing was done: \"{}\" isn't a form your actions take. Ask what they want.", line));
        }
        // An app or site the user never mentioned is the model's invention.
        if action.names_target && !grounded(arg, said, &self.apps) {
            info!("Planner: model chose \"{}\", naming something never said — ignored", line);
            return Err(format!("Nothing was done: they never named {}. Ask which one they mean.", arg));
        }
        if let Some(op) = action.op {
            return match (arg.parse::<u64>(), action.args) {
                (Ok(n), "<0-100>") if n <= 100 => Ok(serde_json::json!([{"type": "system", "op": op, "value": n}])),
                (_, "<0-100>") => Err("Nothing changed: a level has to be between 0 and 100.".into()),
                _ => Ok(serde_json::json!([{"type": "system", "op": op}])),
            };
        }
        let windows = {
            let wm = self.working.read().await;
            with_focus(&wm.open_windows, wm.active_window.as_ref())
        };
        match name {
            "open" | "open_website" => {
                let plan = open_plan(arg, &self.apps)?;
                if let Some(app) = launched_app(&plan, &self.apps) {
                    if let Ok(mut last) = self.last_opened.lock() {
                        *last = Some(app);
                    }
                }
                Ok(plan)
            }
            "close" => close_target(arg, &windows, &self.apps, &self.spared),
            "close_focused" => close_plan(close_app(None, &windows, &self.apps, &self.spared)),
            "close_last_opened" => match self.last_opened.lock().ok().and_then(|n| n.clone()) {
                Some(app) => close_target(&app, &windows, &self.apps, &self.spared),
                None => Err("Nothing was closed: you haven't opened anything yet.".into()),
            },
            "workspace" => match arg.parse::<u64>() {
                Ok(n) if n >= 1 => Ok(serde_json::json!([{"type": "workspace", "index": n}])),
                _ => Err("Nothing changed: no workspace number was given. Ask which one.".into()),
            },
            "fullscreen" => fullscreen_plan(Some(arg), &windows, &self.apps),
            "fullscreen_focused" => fullscreen_plan(None, &windows, &self.apps),
            "play" | "search" => {
                let (query, on) = match arg.rsplit_once(" on ") {
                    Some((q, o)) => (q.trim(), Some(o.trim())),
                    None => (arg, None),
                };
                // A site or app the user never named falls back to the usual one.
                let on = on.filter(|o| grounded(o, said, &self.apps));
                let app = on.and_then(|o| find_named_app(o, &self.apps));
                let mut plan = serde_json::json!([{
                    "type": "play",
                    "query": query,
                    "app": app.map(|a| a.name.clone()),
                    "on": if app.is_none() { on } else { None },
                    "search_only": name == "search",
                }]);
                resolve_media(&mut plan, &self.apps, &self.media).await;
                Ok(plan)
            }
            "web_search" => Ok(serde_json::json!([{"type": "search", "query": arg, "app": null}])),
            // Its result is told when it's known.
            "undo" => Ok(serde_json::json!([{"type": "undo"}])),
            "stop" => {
                self.stop();
                Ok(serde_json::Value::Null)
            }
            "confirm" | "cancel" => {
                self.bus.publish(Event {
                    id: uuid::Uuid::new_v4(),
                    timestamp: chrono::Utc::now(),
                    source: "planner".to_string(),
                    event_type: EventType::Confirmation,
                    payload: serde_json::json!({ "approved": name == "confirm" }),
                });
                Ok(serde_json::Value::Null)
            }
            "system_changes" => match arg.parse::<u64>() {
                Ok(days) => match self.sentinel_answer(Some(days)).await {
                    Some(text) => record(text),
                    None => Err("Nothing to tell: you aren't watching the system right now.".into()),
                },
                Err(_) => Err("Nothing looked up: no number of days was given. Ask over how many days.".into()),
            },
            "update_details" => match self.sentinel_answer(None).await {
                Some(text) => record(text),
                None => Err("Nothing to tell: you aren't watching the system right now.".into()),
            },
            "user_history" => {
                let days = arg.parse::<u64>().unwrap_or(1).clamp(1, 365);
                self.answer_from_memory(said, days).await;
                Ok(serde_json::Value::Null)
            }
            "remember" => {
                if !restates(arg, said) {
                    info!("Planner: model wanted to remember \"{}\", which wasn't said", arg);
                    return Err("Nothing was stored: the fact you chose wasn't what they said. Ask what to remember.".into());
                }
                let stored = self.recall.lock().await.remember(arg, &chrono::Utc::now().to_rfc3339());
                match stored {
                    Ok(()) => Ok(serde_json::json!([])),
                    Err(e) => Err(format!("Nothing was stored: {}", e)),
                }
            }
            "user_name" => {
                // Only a name they actually said, as they said it.
                let name = said
                    .split(|c: char| !c.is_alphabetic() && c != '\'' && c != '-')
                    .find(|w| w.eq_ignore_ascii_case(arg));
                let stored = match name {
                    Some(name) => {
                        let mut name = name.to_string();
                        if let Some(first) = name.get_mut(0..1) {
                            first.make_ascii_uppercase();
                        }
                        self.working.write().await.set_user_name(name)
                    }
                    None => false,
                };
                if stored {
                    Ok(serde_json::json!([]))
                } else {
                    Err("Nothing was stored: that isn't a name they said. Ask what to call them.".into())
                }
            }
            _ => Err(format!("Nothing was done: you can't {} yet.", action.what)),
        }
    }

    /// An action has run. Its result is told when it failed, or when it is
    /// undo's, whose result isn't known until it runs.
    async fn action_done(&self, event: &Event) {
        let p = &event.payload;
        let kind = p["action_type"].as_str().unwrap_or_default();
        let ok = p["success"].as_bool().unwrap_or(true);
        let fact = match (kind, ok) {
            ("say" | "notify" | "tell", _) => return,
            ("undo", true) => format!("Undo ran: {}", p["output"].as_str().unwrap_or_default()),
            (_, true) => return,
            (_, false) => {
                let reason = p["error"].as_str().unwrap_or_default().lines().next().unwrap_or_default().to_string();
                format!("That didn't work, so it didn't happen: {}", crate::util::truncate_bytes(&reason, 160))
            }
        };
        let said = self.last_said.lock().unwrap_or_else(|p| p.into_inner()).clone();
        self.report(&said, &fact).await;
    }

    /// Once enough of the conversation has left the recent turns, have the
    /// model fold it into the running summary. One request at a time.
    async fn maybe_summarise(&self) {
        let (summary, lines) = {
            let wm = self.working.read().await;
            if wm.earlier.len() < SUMMARISE_AT {
                return;
            }
            (wm.summary.clone(), wm.earlier.iter().cloned().collect::<Vec<_>>())
        };
        {
            let mut busy = self.summarising.lock().unwrap_or_else(|p| p.into_inner());
            if busy.is_some() {
                return;
            }
            *busy = Some(lines.len());
        }
        info!("Planner: summarising {} earlier lines", lines.len());
        self.bus.publish(Event {
            id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            source: "planner".to_string(),
            event_type: EventType::WorkerRequest,
            payload: serde_json::json!({
                "request_type": "summarise",
                "summary": summary,
                "lines": lines,
            }),
        });
    }

    /// Have the model tell what happened, in its own words.
    async fn report(&self, said: &str, fact: &str) {
        info!("Planner: telling the outcome — {}", fact);
        let mut working_memory = self.build_working_memory(said, uuid::Uuid::new_v4(), None).await;
        working_memory.push(serde_json::json!({ "source": "result", "content": fact }));
        self.bus.publish(Event {
            id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            source: "planner".to_string(),
            event_type: EventType::WorkerRequest,
            payload: serde_json::json!({
                "request_type": "chat",
                "text": said,
                "messages": [{"role": "user", "content": said}],
                "max_tokens": 96,
                "temperature": 0.6,
                "working_memory": working_memory,
            }),
        });
    }

    /// Whether the permission gate is waiting for an answer.
    fn question_open(&self) -> bool {
        self.awaiting.lock().unwrap_or_else(|p| p.into_inner()).is_open()
    }

    /// Go quiet: drop MAVIS's speech, or with MAVIS silent, pause media.
    fn stop(&self) {
        let speaking = self.speaking.as_ref().is_some_and(|s| s.load(std::sync::atomic::Ordering::SeqCst));
        if !speaking {
            info!("Planner: nothing of MAVIS's to stop — pausing media");
            self.bus.publish(Event {
                id: uuid::Uuid::new_v4(),
                timestamp: chrono::Utc::now(),
                source: "planner".to_string(),
                event_type: EventType::PlanReady,
                payload: serde_json::json!({ "plan": [{"type": "system", "op": "media_pause"}] }),
            });
        }
        for (event_type, payload) in [
            (EventType::TtsInterrupt, serde_json::json!({ "reason": "user_asked" })),
            (EventType::UiStateChange, serde_json::json!({ "state": "idle" })),
        ] {
            self.bus.publish(Event {
                id: uuid::Uuid::new_v4(),
                timestamp: chrono::Utc::now(),
                source: "planner".to_string(),
                event_type,
                payload,
            });
        }
    }

    /// Answer again with memory of the last `days` days in front of the
    /// model, as a plain reply.
    async fn answer_from_memory(&self, said: &str, days: u64) {
        let working_memory = self.build_working_memory(said, uuid::Uuid::new_v4(), Some(days)).await;
        self.bus.publish(Event {
            id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            source: "planner".to_string(),
            event_type: EventType::WorkerRequest,
            payload: serde_json::json!({
                "request_type": "chat",
                "text": said,
                "messages": [{"role": "user", "content": said}],
                "max_tokens": 128,
                "temperature": 0.7,
                "working_memory": working_memory,
            }),
        });
    }

    /// The Sentinel's part in this turn. Failures return None, so a broken
    /// store never blocks the actual reply.
    /// Pending system changes to say before the reply, if any.
    async fn sentinel_lead(&self) -> Option<speech::Spoken> {
        let store = self.sentinel.clone()?;
        let told = tokio::task::spawn_blocking(move || {
            let store = store.lock().unwrap_or_else(|p| p.into_inner());
            speech::lead(&store)
        })
        .await;
        match told {
            Ok(Ok(Some(spoken))) => {
                if spoken.about.is_some() {
                    *self.last_told.lock().unwrap_or_else(|p| p.into_inner()) = spoken.about;
                }
                Some(spoken)
            }
            Ok(Ok(None)) => None,
            Ok(Err(e)) => {
                warn!("Planner: Sentinel store unavailable this turn: {}", e);
                None
            }
            Err(e) => {
                warn!("Planner: Sentinel lookup did not finish: {}", e);
                None
            }
        }
    }

    /// The Sentinel's answer: changes over `days`, or with None the update
    /// last mentioned. None when the Sentinel is off.
    async fn sentinel_answer(&self, days: Option<u64>) -> Option<String> {
        let store = self.sentinel.clone()?;
        let last_told = *self.last_told.lock().unwrap_or_else(|p| p.into_inner());
        let answer = tokio::task::spawn_blocking(move || {
            let store = store.lock().unwrap_or_else(|p| p.into_inner());
            match days {
                Some(d) => speech::answer_changes(&store, d, chrono::Utc::now()),
                None => speech::answer_update(&store, last_told),
            }
        })
        .await;
        match answer {
            Ok(Ok(spoken)) => {
                if spoken.about.is_some() {
                    *self.last_told.lock().unwrap_or_else(|p| p.into_inner()) = spoken.about;
                }
                Some(spoken.text)
            }
            Ok(Err(e)) => Some(format!("You couldn't read the system's history: {}", e)),
            Err(e) => Some(format!("You couldn't read the system's history: {}", e)),
        }
    }

    async fn build_working_memory(
        &self,
        current_intent: &str,
        current_id: uuid::Uuid,
        look_back: Option<u64>,
    ) -> Vec<serde_json::Value> {
        let snapshot = self.working.read().await;
        let mut items = Vec::new();

        // A question MAVIS asked is still open: what's said next may answer it.
        let question = self.awaiting.lock().unwrap_or_else(|p| p.into_inner()).question();
        if let Some(question) = question {
            items.push(serde_json::json!({
                "source": "waiting",
                "content": format!("You just asked the user {}. Their answer decides it.", question),
            }));
        }

        // NEW — inject user profile first so it appears at the top of context
        if let Some(name) = &snapshot.user_name {
            items.push(serde_json::json!({
                "source": "user_profile",
                "content": format!("The user's name is {}.", name),
            }));
        }

        // Semantic-ish recall: pull past exchanges relevant to what was just
        // said. Returns nothing when there's no genuine match, so ordinary
        // chatter doesn't drag irrelevant history into the prompt.
        {
            let store = self.recall.lock().await;

            // A question about a time period ("what was I doing yesterday")
            // wants that whole window, not keyword matches scattered across
            // months — so replay takes precedence when a time reference is
            // present.
            if let Some((start, end, label)) = look_back.map(days_window) {
                // Daily summaries first — they survive decay, so for older
                // periods they may be all that's left.
                {
                    let lt = self.long_term.lock().await;
                    let from = start.format("%Y-%m-%d").to_string();
                    let to = end.format("%Y-%m-%d").to_string();
                    if let Ok(summaries) = lt.summaries_between(&from, &to) {
                        for s in summaries {
                            items.push(serde_json::json!({
                                "source": "daily_summary",
                                "content": format!("On {}: {}", s.date, s.summary),
                            }));
                        }
                    }
                }

                match store.recall_between(&start.to_rfc3339(), &end.to_rfc3339(), 12) {
                    Ok(memories) if !memories.is_empty() => {
                        let lines: Vec<String> = memories
                            .iter()
                            .map(|m| format!("{}: {}", m.role, m.text))
                            .collect();
                        items.push(serde_json::json!({
                            "source": "replay",
                            "content": format!("What happened {}: {}", label, lines.join(" | ")),
                        }));
                    }
                    Ok(_) => {
                        items.push(serde_json::json!({
                            "source": "replay",
                            "content": format!("Nothing was recorded {}.", label),
                        }));
                    }
                    Err(e) => warn!("Planner: replay failed: {}", e),
                }
            } else {
                match store.recall(current_intent, 3) {
                    Ok(memories) if !memories.is_empty() => {
                        // The context engine records each utterance as it
                        // arrives, so recall can return the very sentence
                        // being answered. That's not a memory.
                        for m in memories.into_iter().filter(|m| m.text != current_intent) {
                            items.push(serde_json::json!({
                                "source": "recalled",
                                "content": format!("Earlier, {} said: {}", m.role, m.text),
                            }));
                        }
                    }
                    Ok(_) => {
                        // Nothing in raw memory — the originals may have
                        // decayed, so try the daily summaries.
                        if let Some(fts) = build_fts_query(current_intent) {
                            let lt = self.long_term.lock().await;
                            if let Ok(summaries) = lt.search(&fts, 2) {
                                for s in summaries {
                                    items.push(serde_json::json!({
                                        "source": "daily_summary",
                                        "content": format!("On {}: {}", s.date, s.summary),
                                    }));
                                }
                            }
                        }
                    }
                    Err(e) => warn!("Planner: recall failed: {}", e),
                }
            }
        }

        // Date/time is always injected — it's not private, and without it
        // the model guesses at anything time-related.
        items.push(serde_json::json!({
            "source": "datetime",
            "content": format!(
                "The current date and time is {}.",
                chrono::Local::now().format("%A, %-d %B %Y at %-I:%M %p")
            ),
        }));

        // Phase 6 — active window and clipboard, both off by default.
        if context_source_enabled("MAVIS_CONTEXT_ACTIVE_WINDOW") {
            // App names, not window ids: "Moovie", not "org.kde.moovie".
            let active = snapshot.active_window.as_ref().map(|w| with_app_name(w, &self.apps));
            let open_windows: Vec<WindowInfo> =
                snapshot.open_windows.iter().map(|w| with_app_name(w, &self.apps)).collect();
            match &active {
                Some(window) => items.push(serde_json::json!({
                    "source": "active_window",
                    "content": describe_active_window(window),
                })),
                // Better to say so than let the model pick one from the list.
                None if !snapshot.open_windows.is_empty() => items.push(serde_json::json!({
                    "source": "active_window",
                    "content": "Which window is focused isn't known right now.",
                })),
                None => {}
            }

            if let Some(content) = describe_open_windows(&open_windows, snapshot.active_workspace) {
                items.push(serde_json::json!({ "source": "open_windows", "content": content }));
            }
            if let Some(levels) = crate::system::dbus::current_levels().await {
                items.push(serde_json::json!({ "source": "levels", "content": levels }));
            }
            // The desktop names itself; nothing here assumes which one.
            if let Ok(desktop) = std::env::var("XDG_CURRENT_DESKTOP") {
                if !desktop.trim().is_empty() {
                    items.push(serde_json::json!({
                        "source": "desktop",
                        "content": format!("The user's desktop (window manager) is {}.", desktop.trim()),
                    }));
                }
            }

            if let Some(project) = &snapshot.project {
                let content = match &project.git_branch {
                    Some(branch) => format!(
                        "The user is working in the {} project (git branch {}) at {}.",
                        project.name, branch, project.path
                    ),
                    None => format!(
                        "The user is working in the {} project at {}.",
                        project.name, project.path
                    ),
                };
                items.push(serde_json::json!({
                    "source": "project",
                    "content": content,
                }));
            }
        }

        // Entity graph — what the user habitually works on. Only surfaced
        // for questions that actually ask, since it's background knowledge
        // rather than current state.
        {
            let lower = current_intent.to_lowercase();
            let asks_about_work = ["working on", "work on", "projects", "usually",
                                   "what do i", "my projects", "been doing"]
                .iter()
                .any(|p| lower.contains(p));
            if asks_about_work {
                let store = self.entities.lock().await;
                if let Ok(projects) = store.top(EntityKind::Project, 4) {
                    if !projects.is_empty() {
                        let names: Vec<String> =
                            projects.iter().map(|p| p.name.clone()).collect();
                        items.push(serde_json::json!({
                            "source": "known_projects",
                            "content": format!(
                                "Projects the user works in, most frequent first: {}.",
                                names.join(", ")
                            ),
                        }));
                    }
                }
                // What tends to be open alongside the current project.
                if let Some(project) = &snapshot.project {
                    if let Ok(related) = store.related(&project.name, EntityKind::Project, 5) {
                        if !related.is_empty() {
                            let names: Vec<String> = related
                                .iter()
                                .map(|e| format!("{} ({})", e.name, e.kind))
                                .collect();
                            items.push(serde_json::json!({
                                "source": "related_entities",
                                "content": format!(
                                    "Usually alongside {}: {}.",
                                    project.name,
                                    names.join(", ")
                                ),
                            }));
                        }
                    }
                }
            }
        }

        if context_source_enabled("MAVIS_CONTEXT_CALENDAR") {
            if let Some(event) = &snapshot.next_event {
                let content = if event.all_day {
                    format!("The user's next calendar event is \"{}\" (all day).", event.summary)
                } else if event.minutes_until < 60 {
                    format!(
                        "The user's next calendar event is \"{}\", in {} minutes.",
                        event.summary, event.minutes_until
                    )
                } else {
                    format!(
                        "The user's next calendar event is \"{}\", in about {} hours.",
                        event.summary,
                        event.minutes_until / 60
                    )
                };
                items.push(serde_json::json!({
                    "source": "calendar",
                    "content": content,
                }));
            }
        }

        // Only when the user is asking about it. Sent on every turn, the
        // clipboard was noise the model had to ignore — and whatever
        // happened to be copied (a launch command, a password) went into
        // every prompt.
        if context_source_enabled("MAVIS_CONTEXT_CLIPBOARD") && mentions_clipboard(current_intent) {
            if let Some(clipboard) = &snapshot.last_clipboard {
                // Flattened to one line first. A copied terminal session
                // is thousands of characters over dozens of lines, and
                // pasting it raw broke the prompt apart: every line after
                // the first read as its own context item, and a stray
                // word from a shell prompt became the answer.
                let flat = clipboard.split_whitespace().collect::<Vec<_>>().join(" ");
                if !flat.is_empty() {
                    let total = flat.chars().count();
                    let shown: String = flat.chars().take(200).collect();
                    let content = if total > 200 {
                        format!(
                            "The user's clipboard holds {} characters, beginning: \"{}…\"",
                            total, shown
                        )
                    } else {
                        format!("The user's clipboard contains: \"{}\"", shown)
                    };
                    items.push(serde_json::json!({
                        "source": "clipboard",
                        "content": content,
                    }));
                }
            }
        }

        if context_source_enabled("MAVIS_CONTEXT_BROWSER") {
            if let Some(tab) = &snapshot.browser_tab {
                items.push(serde_json::json!({
                    "source": "browser",
                    "content": format!("The user is browsing {} — \"{}\".", tab.domain, tab.title),
                }));
            }
        }

        // Conversation so far: everything still in the event ring. The
        // worker keeps as much of it as fits the model's context, newest
        // first; what came before is in the summary.
        //
        // What MAVIS said comes from PlanReady only: WorkerResponse carries
        // the same text, and both would put every reply in twice.
        //
        // The current utterance is skipped: it's the user message itself,
        // and depending on which task ran first it could also already be in
        // the ring.
        let waiting: Vec<String> = snapshot.earlier.iter().cloned().collect();
        if let Some(line) = earlier_in_conversation(&snapshot.summary, &waiting) {
            items.push(serde_json::json!({ "source": "earlier", "content": line }));
        }

        for event in snapshot.recent_events(snapshot.events.len()) {
            if event.id == current_id {
                continue;
            }
            let (source, content) = match event.event_type {
                EventType::UserIntent => (
                    "user",
                    event.payload.get("text").and_then(|v| v.as_str()).unwrap_or(""),
                ),
                EventType::PlanReady => ("mavis", ""),
                _ => continue,
            };
            // MAVIS's turns as it wrote them: tool calls stay tool calls, so
            // the model sees what it did apart from what it said.
            let said;
            let content = if source == "mavis" {
                said = mavis_turn(&event.payload);
                said.as_str()
            } else {
                content
            };

            if !content.is_empty() {
                items.push(serde_json::json!({
                    "source": source,
                    "content": content,
                }));
            }
        }

        items
    }

    async fn handle_worker_response(&self, event: Event) -> anyhow::Result<()> {
        let payload = &event.payload;
        let response_type = payload.get("type").and_then(|v| v.as_str()).unwrap_or("unknown");

        match response_type {
            "response" => {
                let content = payload
                    .get("result")
                    .and_then(|r| r.get("content"))
                    .and_then(|c| c.as_str())
                    .unwrap_or("I didn't understand that.");

                let plan_event = Event {
                    id: uuid::Uuid::new_v4(),
                    timestamp: chrono::Utc::now(),
                    source: "planner".to_string(),
                    event_type: EventType::PlanReady,
                    payload: serde_json::json!({
                        "plan": {"type": "say", "text": content}
                    }),
                };
                self.bus.publish(plan_event);
                self.maybe_summarise().await;
            }
            "command" => {
                let text = payload.get("text").and_then(|v| v.as_str()).unwrap_or_default();
                let action = payload.get("command").and_then(|v| v.as_str()).unwrap_or_default();
                let call = payload.get("call").and_then(|v| v.as_str()).unwrap_or(action);
                info!("Planner: model chose \"{}\" for \"{}\"", action, text);
                self.carry_out(action, text, call).await;
                self.maybe_summarise().await;
            }
            "summary" => {
                let sent = self.summarising.lock().unwrap_or_else(|p| p.into_inner()).take().unwrap_or(0);
                let text = payload.get("text").and_then(|v| v.as_str()).unwrap_or_default().trim();
                if !text.is_empty() {
                    let mut wm = self.working.write().await;
                    wm.summary = text.to_string();
                    let folded = sent.min(wm.earlier.len());
                    wm.earlier.drain(..folded);
                    info!("Planner: conversation summary updated ({} lines folded in)", folded);
                }
            }
            "summary_failed" => {
                self.summarising.lock().unwrap_or_else(|p| p.into_inner()).take();
            }
            "error" => {
                let error_msg = payload
                    .get("error")
                    .and_then(|e| e.as_str())
                    .unwrap_or("Unknown worker error");

                let plan_event = Event {
                    id: uuid::Uuid::new_v4(),
                    timestamp: chrono::Utc::now(),
                    source: "planner".to_string(),
                    event_type: EventType::PlanReady,
                    payload: serde_json::json!({
                        "plan": {"type": "say", "text": format!("Sorry, I encountered an error: {}", error_msg)}
                    }),
                };
                self.bus.publish(plan_event);
            }
            other => {
                info!("Planner: unhandled WorkerResponse type '{}'", other);
            }
        }

        Ok(())
    }
}
// ---------------------------------------------------------------------------
// Playing and searching
// ---------------------------------------------------------------------------
//
// "Play X" and "search for X": which app and which site, decided from what
// each installed app registers for and from config.toml. No app or site
// names live in code.

/// A site from config.toml: where a query goes, and optionally how to pick
/// the top result off its results page.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Site {
    pub name: String,
    search_url: String,
    result_marker: Option<String>,
    play_url: Option<String>,
}

impl Site {
    #[cfg(test)]
    pub fn new(name: &str, search_url: &str) -> Self {
        Self { name: name.to_string(), search_url: search_url.to_string(), ..Default::default() }
    }

    fn from_config(text: &str, section: &str) -> Option<Self> {
        let get = |key| crate::listen::read_setting(text, section, key).filter(|v| !v.is_empty());
        Some(Self {
            name: get("site_name").unwrap_or_default(),
            search_url: get("search_url").filter(|u| u.contains("{query}"))?,
            result_marker: get("result_marker"),
            play_url: get("play_url").filter(|u| u.contains("{id}")),
        })
    }

    fn search(&self, query: &str) -> String {
        self.search_url.replace("{query}", &crate::util::urlencode(query))
    }

    /// "sunbeam on tube" → "sunbeam", when the site is called that.
    fn strip_from<'a>(&self, query: &'a str) -> &'a str {
        let squash = |s: &str| s.to_lowercase().replace(' ', "");
        let name = squash(&self.name);
        match query.rfind(" on ") {
            Some(at) if !name.is_empty() && squash(&query[at + 4..]) == name => query[..at].trim(),
            _ => query,
        }
    }

    fn named(&self, spoken: &str) -> bool {
        let squash = |s: &str| s.to_lowercase().replace(' ', "");
        !self.name.is_empty() && squash(spoken) == squash(&self.name)
    }

    /// The top result's link, read off the results page. None without a
    /// marker in config, without network or curl, or with nothing found.
    async fn top_result(&self, search: &str) -> Option<String> {
        let (marker, play_url) = (self.result_marker.as_deref()?, self.play_url.as_deref()?);
        if cfg!(test) {
            return None;
        }
        let fetched = tokio::time::timeout(
            std::time::Duration::from_secs(6),
            tokio::process::Command::new("curl")
                .args(["-sL", "--max-time", "5", "-A", "Mozilla/5.0 (X11; Linux x86_64)"])
                .args(["-H", "Accept-Language: en", search])
                .output(),
        )
        .await;
        let page = match fetched {
            Ok(Ok(out)) if out.status.success() => String::from_utf8_lossy(&out.stdout).into_owned(),
            _ => {
                warn!("Media: couldn't reach {} — opening the search", self.name);
                return None;
            }
        };
        let Some(id) = id_after(&page, marker) else {
            warn!("Media: no result found on the {} page — opening the search", self.name);
            return None;
        };
        Some(play_url.replace("{id}", &id))
    }
}

/// The id right after `marker`: letters, digits, '-' and '_'.
fn id_after(page: &str, marker: &str) -> Option<String> {
    let at = page.find(marker)? + marker.len();
    let id: String = page[at..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    (!id.is_empty()).then_some(id)
}

/// Sites from config, and the app last asked to play something.
#[derive(Debug, Default)]
pub struct Media {
    pub play_site: Option<Site>,
    pub web_site: Option<Site>,
    last_app_file: Option<std::path::PathBuf>,
}

impl Media {
    /// `[media]` and `[web]` in config.toml; the last app is kept in `data_dir`.
    pub fn configured(data_dir: &std::path::Path) -> Self {
        let text = crate::listen::config_text().unwrap_or_default();
        let media = Self {
            play_site: Site::from_config(&text, "media"),
            web_site: Site::from_config(&text, "web"),
            last_app_file: Some(data_dir.join("media_app")),
        };
        if media.play_site.is_none() {
            info!("Media: no [media] search_url in config.toml — \"play\" in a browser is off");
        }
        media
    }

    #[cfg(test)]
    pub fn with_sites(play_site: Option<Site>, web_site: Option<Site>, last_app_file: Option<std::path::PathBuf>) -> Self {
        Self { play_site, web_site, last_app_file }
    }

    fn last_app(&self) -> Option<String> {
        let text = std::fs::read_to_string(self.last_app_file.as_ref()?).ok()?;
        Some(text.trim().to_string()).filter(|s| !s.is_empty())
    }

    fn remember(&self, app: &str) {
        if let Some(path) = &self.last_app_file {
            if let Err(e) = std::fs::write(path, app) {
                warn!("Media: couldn't remember {}: {}", app, e);
            }
        }
    }
}

/// An app that can play or show something, by its own registration.
pub fn can_play(app: &AppEntry) -> bool {
    app.opens_links || plays_files(app) || own_scheme(app).is_some()
}

fn plays_files(app: &AppEntry) -> bool {
    app.opens.iter().any(|m| m.starts_with("audio/") || m.starts_with("video/"))
}

/// A link type of the app's own ("x-scheme-handler/<name>"), not the web's,
/// for an app that registers nothing else. Editors and terminals register
/// their own link types too, beside files; handing them a search ran
/// `<terminal> <name>:search:…`.
fn own_scheme(app: &AppEntry) -> Option<&str> {
    if app.opens.iter().any(|m| !m.starts_with("x-scheme-handler/")) {
        return None;
    }
    app.opens
        .iter()
        .filter_map(|m| m.strip_prefix("x-scheme-handler/"))
        .find(|s| !matches!(*s, "http" | "https" | "ftp" | "file" | "mailto"))
}

/// Replace a planner placeholder ("play" or "search") with real actions.
pub async fn resolve_media(plan: &mut serde_json::Value, apps: &[AppEntry], media: &Media) {
    let Some(actions) = plan.as_array_mut() else { return };
    let Some(at) = actions.iter().position(|a| matches!(a["type"].as_str(), Some("play" | "search"))) else {
        return;
    };
    let step = actions.remove(at);
    let query = step["query"].as_str().unwrap_or_default();
    let find = |name: &str| apps.iter().find(|a| a.name == name);
    let named = step["app"].as_str().and_then(find);

    let concrete = if step["type"] == "search" {
        match &media.web_site {
            Some(site) => in_browser(site.search(query), named),
            None => note("Nothing was searched: no search site is set. It goes under [web] in config.toml."),
        }
    } else {
        let search_only = step["search_only"].as_bool() == Some(true);
        // "search tunely for X": a site or an app, whichever it names.
        let on = step["on"].as_str();
        let on_app = on.and_then(|o| find_app(o, apps)).filter(|a| can_play(a));
        let stripped = media.play_site.as_ref().map_or(query, |s| s.strip_from(query));
        // The site named ("on tube", "search tube for"): a browser.
        let site_named = stripped != query || on.is_some();
        let (target, used_last) = match (named.or(on_app), on) {
            (Some(app), _) => (Some(app), false),
            // "search the web for X", "search google for X": not the play
            // site and not an app, so a web search.
            (None, Some(o)) if !media.play_site.as_ref().is_some_and(|s| s.named(o)) => {
                let found = match &media.web_site {
                    Some(site) => in_browser(site.search(query), None),
                    None => note(&format!("Nothing was searched: you don't know how to search {}.", o)),
                };
                return splice(actions, at, found);
            }
            _ => {
                let last = media
                    .last_app()
                    .and_then(|n| find(&n))
                    .filter(|a| !site_named || a.opens_links);
                (last, last.is_some())
            }
        };
        let query = stripped;
        let (actions_out, worked) = play_in(query, target, search_only, media).await;
        if worked && !used_last {
            if let Some(app) = target {
                media.remember(&app.name);
            }
        }
        actions_out
    };
    splice(actions, at, concrete);
}

fn splice(actions: &mut Vec<serde_json::Value>, at: usize, with: Vec<serde_json::Value>) {
    for (i, a) in with.into_iter().enumerate() {
        actions.insert(at + i, a);
    }
}

/// Something that didn't go as asked, for MAVIS to tell.
fn note(text: &str) -> Vec<serde_json::Value> {
    vec![serde_json::json!({"type": "note", "text": text})]
}

/// Launch `app`, with a link or a file after its own arguments.
fn launch_with(app: &AppEntry, arg: Option<String>) -> serde_json::Value {
    // Flatpak's @@ markers bracket file arguments; ours goes after.
    let mut parts = crate::util::split_command_line(&app.exec).into_iter().filter(|p| !p.starts_with("@@"));
    let bin = parts.next().unwrap_or_default();
    let args: Vec<String> = parts.chain(arg).collect();
    serde_json::json!({"type": "app", "target": bin, "args": args})
}

fn in_browser(url: String, app: Option<&AppEntry>) -> Vec<serde_json::Value> {
    match app {
        Some(app) => vec![launch_with(app, Some(url))],
        None => vec![serde_json::json!({"type": "app", "target": url})],
    }
}

/// The actions for playing `query` in `app` (None: the default browser),
/// and whether it found something to play.
async fn play_in(query: &str, app: Option<&AppEntry>, search_only: bool, media: &Media) -> (Vec<serde_json::Value>, bool) {
    // A browser, or no app at all: the configured site.
    if app.is_none_or(|a| a.opens_links) {
        let Some(site) = &media.play_site else {
            return (note("Nothing was played: no site is set for playing. It goes under [media] in config.toml."), false);
        };
        let search = site.search(query);
        if !search_only {
            if let Some(link) = site.top_result(&search).await {
                info!("Media: playing {}", link);
                return (in_browser(link, app), true);
            }
            let mut opened = in_browser(search, app);
            opened.extend(note(&format!("Nothing could be played straight away, so {}'s search for {} was opened instead.", site.name, query)));
            return (opened, true);
        }
        return (in_browser(search, app), true);
    }
    let app = app.expect("checked above");

    // A media player: a matching file from the user's music or videos.
    if plays_files(app) && !search_only {
        if let Some(file) = find_media_file(query, &media_dirs()) {
            let title = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            info!("Media: playing {} in {}", title, app.name);
            return (vec![launch_with(app, Some(file.to_string_lossy().into_owned()))], true);
        }
    }

    // An app with its own link type: hand it a search.
    if let Some(scheme) = own_scheme(app) {
        let uri = format!("{}:search:{}", scheme, crate::util::urlencode(query).replace('+', "%20"));
        return (vec![launch_with(app, Some(uri))], true);
    }

    if plays_files(app) {
        return (note(&format!("Nothing was played: {} isn't in their music or videos.", query)), false);
    }
    let mut opened = vec![launch_with(app, None)];
    opened.extend(note(&format!("{} was opened, but you can't search inside it, so nothing is playing.", app.name)));
    (opened, false)
}

/// The user's Music and Videos folders, from user-dirs.dirs, else ~/Music
/// and ~/Videos.
fn media_dirs() -> Vec<std::path::PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else { return Vec::new() };
    let config = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| home.join(".config"));
    let text = std::fs::read_to_string(config.join("user-dirs.dirs")).unwrap_or_default();
    let from_file = |key: &str| {
        text.lines().find_map(|l| {
            let v = l.strip_prefix(key)?.strip_prefix('=')?.trim().trim_matches('"');
            Some(std::path::PathBuf::from(v.replace("$HOME", &home.to_string_lossy())))
        })
    };
    vec![
        from_file("XDG_MUSIC_DIR").unwrap_or_else(|| home.join("Music")),
        from_file("XDG_VIDEOS_DIR").unwrap_or_else(|| home.join("Videos")),
    ]
}

/// The first media file whose name has every word of the query, apart from
/// small words: "sunbeam by the band" finds "The Band - Sunbeam.mp3".
fn find_media_file(query: &str, dirs: &[std::path::PathBuf]) -> Option<std::path::PathBuf> {
    const SMALL: &[&str] = &["by", "the", "a", "an", "of", "song", "music", "video", "from"];
    const MEDIA: &[&str] = &[
        "mp3", "flac", "ogg", "opus", "m4a", "aac", "wav", "wma", "mp4", "mkv", "webm", "avi", "mov", "m4v",
    ];
    let words = |s: &str| -> Vec<String> {
        s.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(String::from)
            .collect()
    };
    let wanted: Vec<String> = words(query).into_iter().filter(|w| !SMALL.contains(&w.as_str())).collect();
    if wanted.is_empty() {
        return None;
    }
    let mut stack: Vec<(std::path::PathBuf, u8)> = dirs.iter().map(|d| (d.clone(), 0)).collect();
    let mut seen = 0;
    let mut found: Vec<std::path::PathBuf> = Vec::new();
    'walk: while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            if seen > 20_000 {
                break 'walk;
            }
            let path = entry.path();
            if path.is_dir() {
                if depth < 5 {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            if !MEDIA.contains(&ext.as_str()) {
                continue;
            }
            let have = words(&path.file_stem().unwrap_or_default().to_string_lossy());
            if wanted.iter().all(|w| have.contains(w)) {
                found.push(path);
            }
        }
    }
    found.sort();
    found.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------
    // strip_address — regression tests for the UTF-8 panic
    // ---------------------------------------------------------------

    /// Each of these panicked before the `get(..)` fix. Whisper emits
    /// non-ASCII often enough that this was reachable from ordinary use.
    #[test]
    fn strip_address_survives_non_ascii() {
        let inputs = [
            "ééé mavis open webby",
            "日本語",
            "é",
            "ée",
            "\u{2019}\u{2019}\u{2019} mavis",
            "café mavis",
            "naïve mavis",
            "—— mavis hello",
            "🙂 mavis",
            "🙂",
        ];
        for input in inputs {
            // The assertion is simply that this returns at all.
            let _ = strip_address(input);
        }
    }

    #[test]
    fn strip_address_never_panics_on_any_prefix_of_multibyte_text() {
        let s = "🙂é日本語 mavis open webby";
        for end in 0..=s.len() {
            if let Some(slice) = s.get(..end) {
                let _ = strip_address(slice);
            }
        }
    }

    #[test]
    fn strip_address_still_strips_the_wake_word() {
        assert_eq!(strip_address("mavis, open webby"), "open webby");
        assert_eq!(strip_address("MAVIS open webby"), "open webby");
        assert_eq!(strip_address("hey mavis, volume up"), "volume up");
        assert_eq!(strip_address("ok mavis. volume up"), "volume up");
        assert_eq!(strip_address("okay mavis volume up"), "volume up");
        assert_eq!(strip_address("  mavis   open webby"), "open webby");
    }

    #[test]
    fn strip_address_leaves_unaddressed_text_alone() {
        assert_eq!(strip_address("open webby"), "open webby");
        assert_eq!(strip_address("what time is it"), "what time is it");
        // Short strings must not be mistaken for a prefix.
        assert_eq!(strip_address("mav"), "mav");
        assert_eq!(strip_address(""), "");
    }

    // ---------------------------------------------------------------
    // System intents
    // ---------------------------------------------------------------

    // ---------------------------------------------------------------
    // Meta-instruction deflection
    // ---------------------------------------------------------------

    // ---------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------

    #[test]
    fn urlencode_escapes_correctly() {
        use crate::util::urlencode;
        assert_eq!(urlencode("hello world"), "hello+world");
        assert_eq!(urlencode("a&b=c"), "a%26b%3Dc");
        assert_eq!(urlencode("safe-_.~"), "safe-_.~");
    }

    #[test]
    fn urlencode_handles_non_ascii() {
        use crate::util::urlencode;
        // Must encode per-byte and never panic.
        assert_eq!(urlencode("é"), "%C3%A9");
        let _ = urlencode("日本語 🙂");
    }

    #[test]
    fn strip_env_prefix_removes_assignments() {
        assert_eq!(
            strip_env_prefix("MAVIS_CONTEXT_ACTIVE_WINDOW=1 RUST_LOG=info cargo run"),
            "cargo run"
        );
        assert_eq!(strip_env_prefix("cargo run"), "cargo run");
        assert_eq!(strip_env_prefix("  FOO=bar baz"), "baz");
        // Lowercase keys are not env assignments (e.g. a filename).
        assert_eq!(strip_env_prefix("file=name.txt open"), "file=name.txt open");
    }

    #[test]
    fn strip_env_prefix_survives_non_ascii() {
        for s in ["日本語", "FOO=日本語 bar", "é=1 x", ""] {
            let _ = strip_env_prefix(s);
        }
    }

    #[test]
    fn normalize_collapses_punctuation_and_case() {
        assert_eq!(normalize("Code - OSS"), "code oss");
        assert_eq!(normalize("  Webby  "), "webby");
        assert_eq!(normalize(""), "");
    }

    use crate::sentinel::change::{Change, ChangeKind};

    /// Planner on temp stores with one pending pulled-in package.
    fn planner_with_pending_change(
        tag: &str,
    ) -> (Planner, tokio::sync::broadcast::Receiver<Event>, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "mavis_planner_{}_{}_{}",
            tag,
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let sentinel = SentinelStore::new(&dir.join("sentinel.db")).unwrap();
        let when = chrono::DateTime::from_timestamp(chrono::Utc::now().timestamp() - 3600, 0).unwrap();
        sentinel
            .record_all(&[Change::new(
                ChangeKind::PackageInstalled {
                    name: "alphawm".into(),
                    version: "1.0".into(),
                    requested: false,
                },
                "pacman",
                when,
            )])
            .unwrap();

        let bus = Arc::new(EventBus::new());
        let rx = bus.subscribe();
        let planner = Planner::new(
            bus,
            Arc::new(RwLock::new(WorkingMemory::new())),
            vec![AppEntry { name: "Webby".into(), exec: "webby".into(), opens_links: true, ..Default::default() }],
            Arc::new(Mutex::new(RecallStore::new(&dir.join("recall.db")).unwrap())),
            Arc::new(Mutex::new(LongTermMemory::new(&dir.join("long_term.db")).unwrap())),
            Arc::new(Mutex::new(EntityStore::new(&dir.join("entities.db")).unwrap())),
            Some(Arc::new(std::sync::Mutex::new(sentinel))),
        );
        (planner, rx, dir)
    }

    fn utterance(text: &str) -> Event {
        Event {
            id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            source: "stt".into(),
            event_type: EventType::UserIntent,
            payload: serde_json::json!({ "text": text, "source": "voice" }),
        }
    }

    fn drain(rx: &mut tokio::sync::broadcast::Receiver<Event>) -> Vec<Event> {
        let mut out = Vec::new();
        while let Ok(e) = rx.try_recv() {
            out.push(e);
        }
        out
    }

    /// The facts handed to the model with a request, by source.
    fn facts(request: &Event, source: &str) -> Vec<String> {
        request.payload["working_memory"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter(|i| i["source"] == source)
                    .filter_map(|i| i["content"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Pending system changes reach the model as news with the request,
    /// for it to tell first. Once.
    #[tokio::test]
    async fn pending_changes_are_said_before_the_reply() {
        let (planner, mut rx, dir) = planner_with_pending_change("lead");
        planner.handle_event(utterance("open webby")).await.unwrap();
        let events = drain(&mut rx);
        assert!(!events.iter().any(|e| e.event_type == EventType::PlanReady), "nothing said in fixed words");
        let asked = events.iter().find(|e| e.event_type == EventType::WorkerRequest).unwrap();
        let news = facts(asked, "news");
        assert!(news.len() == 1 && news[0].contains("alphawm"), "{:?}", news);

        planner.handle_event(utterance("open webby")).await.unwrap();
        let asked = drain(&mut rx).into_iter().find(|e| e.event_type == EventType::WorkerRequest).unwrap();
        assert!(facts(&asked, "news").is_empty(), "told once");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn window(app: &str, title: &str, ws: u64, focused: bool) -> WindowInfo {
        WindowInfo {
            app_name: app.into(),
            window_title: title.into(),
            pid: None,
            workspace_id: Some(ws),
            is_focused: focused,
            id: Some(ws * 100 + app.len() as u64),
        }
    }

    /// The model said "four" with five apps listed, and confused
    /// workspaces. The counts are now worked out before it sees them.
    #[test]
    fn open_windows_are_counted_for_the_model() {
        let windows = vec![
            window("term", "cargo run", 1, true),
            window("term", "htop", 1, false),
            window("files", "Home", 1, false),
            window("webby", "News", 2, false),
            window("notes", "todo", 2, false),
        ];
        let line = describe_open_windows(&windows, Some(1)).unwrap();
        assert!(line.starts_with("5 windows are open, from 4 applications:"), "{}", line);
        assert!(line.contains("term (2 windows, workspace 1)"), "{}", line);
        assert!(line.contains("files (workspace 1)"), "{}", line);
        assert!(line.contains("Windows are on 2 workspaces: 1 and 2."), "{}", line);
        assert!(line.contains("The user is on workspace 1."), "{}", line);
    }

    #[test]
    fn the_focused_window_is_described_without_knowing_the_app() {
        let w = window("anything", "MAVIS_X=1 cargo run", 1, true);
        assert_eq!(describe_active_window(&w), "The focused window is anything — \"cargo run\".");
        let w = window("editor", "main.rs - mavis_core", 1, true);
        assert!(describe_active_window(&w).ends_with("Its title names the file main.rs."));
        // Read aloud, "2>&1 | tee ~/log" was symbols.
        let w = window("term", "cargo run 2>&1 | tee ~/mavis-test.log", 1, true);
        assert_eq!(describe_active_window(&w), "The focused window is term — \"cargo run\".");
        let w = window("term", "make && make install", 1, true);
        assert_eq!(describe_active_window(&w), "The focused window is term — \"make\".");
        // A page title's separator is not a redirection.
        let w = window("webby", "News | Site", 1, true);
        assert_eq!(describe_active_window(&w), "The focused window is webby — \"News | Site\".");
    }

    fn kinds(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    fn desktop_apps() -> Vec<AppEntry> {
        vec![
            AppEntry { name: "termy".into(), exec: "termy".into(), kinds: kinds(&["terminal", "emulator", "system"]), ..Default::default() },
            AppEntry { name: "Filey File Manager".into(), exec: "filey".into(), kinds: kinds(&["file", "manager", "system"]), ..Default::default() },
            AppEntry { name: "Surfy".into(), exec: "surfy".into(), opens_links: true, kinds: kinds(&["web", "browser", "network"]), ..Default::default() },
            AppEntry { name: "Webby".into(), exec: "webby".into(), opens_links: true, kinds: kinds(&["web", "browser", "network"]), ..Default::default() },
        ]
    }

    #[test]
    fn windows_are_named_after_their_apps() {
        let apps = vec![
            AppEntry { name: "Bulk Rename".into(), exec: "filey --bulk-rename".into(), ..Default::default() },
            AppEntry { name: "Filey File Manager".into(), exec: "filey".into(), ..Default::default() },
            AppEntry { name: "Moovie".into(), exec: "moovie".into(), ..Default::default() },
        ];
        assert_eq!(with_app_name(&window("org.kde.moovie", "film", 1, false), &apps).app_name, "Moovie");
        assert_eq!(with_app_name(&window("filey", "Home", 1, false), &apps).app_name, "Filey File Manager");
        assert_eq!(with_app_name(&window("unknownapp", "x", 1, false), &apps).app_name, "unknownapp");
    }

    /// Two entries share one binary; a misheard name tied between them and
    /// opened nothing.
    #[test]
    fn a_misheard_name_prefers_the_app_named_so() {
        let apps = vec![
            AppEntry { name: "Bulk Rename".into(), exec: "filey --bulk-rename".into(), ..Default::default() },
            AppEntry { name: "Filey File Manager".into(), exec: "filey".into(), ..Default::default() },
        ];
        assert_eq!(misheard_app("filly", &apps).unwrap().name, "Filey File Manager");
    }

    #[test]
    fn just_the_name_with_a_question_mark_is_still_just_the_name() {
        assert_eq!(strip_address("Mavis?"), "");
        assert_eq!(strip_address("Mavis? what time is it"), "what time is it");
    }

    #[test]
    fn a_misheard_name_is_still_an_address() {
        assert_eq!(strip_address("MOVIS, what changed recently?"), "what changed recently?");
        assert_eq!(strip_address("Mavus. open notes"), "open notes");
        // Not set off by punctuation, or not close enough: left alone.
        assert_eq!(strip_address("movies tonight"), "movies tonight");
        assert_eq!(strip_address("Magic, show me"), "Magic, show me");
    }

    #[tokio::test]
    async fn stop_pauses_media_only_when_mavis_is_silent() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let (planner, mut rx, dir) = planner_with_pending_change("stop_media");
        let speaking = Arc::new(AtomicBool::new(false));
        let planner = planner.with_speaking(speaking.clone());

        planner.handle_event(model_did("stop", "stop")).await.unwrap();
        assert!(drain(&mut rx).iter().any(|e| e.payload.to_string().contains("media_pause")));

        speaking.store(true, Ordering::SeqCst);
        planner.handle_event(model_did("stop", "MAVIS, stop.")).await.unwrap();
        let events = drain(&mut rx);
        assert!(events.iter().any(|e| e.event_type == EventType::TtsInterrupt));
        assert!(!events.iter().any(|e| e.payload.to_string().contains("media_pause")), "only MAVIS stops");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn closing_an_app_is_asked_about_first() {
        let plan = serde_json::json!([
            {"type": "close", "app": "Surfy Web Browser", "windows": [11, 12]},
            {"type": "say", "text": "Closed Surfy Web Browser."},
        ]);
        let a = crate::safety::risk::assess_plan(&plan);
        assert_eq!(a.verdict, crate::safety::risk::Verdict::Confirm);
        assert_eq!(a.what, "close Surfy Web Browser");
    }

    /// MAVIS's earlier turns reach the model as it wrote them: a tool call
    /// stays a tool call, a reply stays a reply.
    #[test]
    fn earlier_turns_are_as_the_model_wrote_them() {
        let call = "<tool_call>\n{\"name\": \"undo\", \"arguments\": {}}\n</tool_call>";
        let did = serde_json::json!({"plan": [{"type": "undo"}], "did": call});
        assert_eq!(mavis_turn(&did), call);
        let said = serde_json::json!({"plan": {"type": "say", "text": "Hello."}});
        assert_eq!(mavis_turn(&said), "Hello.");
        assert_eq!(mavis_turn(&serde_json::json!({"plan": []})), "");
    }

    /// What happened is told at the end of the plan, once the rest worked;
    /// first when the session is about to end.
    #[test]
    fn the_outcome_is_told_after_the_plan() {
        let plan = with_tell(serde_json::json!([{"type": "system", "op": "volume_up"}]), "volume_up");
        assert_eq!(plan[1], serde_json::json!({"type": "tell", "fact": "Done: volume_up"}));
        let plan = with_tell(serde_json::json!([{"type": "system", "op": "logout"}]), "logout");
        assert_eq!(plan[0]["type"], "tell");
        let plan = with_tell(serde_json::json!([{"type": "app"}, {"type": "note", "text": "Only the search page."}]), "play x");
        assert_eq!(plan[1]["fact"], "Only the search page.", "a note is told instead of done");
        assert_eq!(with_tell(serde_json::json!([{"type": "undo"}]), "undo"), serde_json::json!([{"type": "undo"}]));
    }

    #[test]
    fn what_came_before_is_the_summary_then_the_lines_after_it() {
        let waiting = vec!["User: ccc".to_string(), "MAVIS: ddd".to_string()];
        assert_eq!(
            earlier_in_conversation("aaa bbb.", &waiting).unwrap(),
            "Earlier in this conversation: aaa bbb. Then: User: ccc / MAVIS: ddd"
        );
        assert_eq!(earlier_in_conversation("aaa.", &[]).unwrap(), "Earlier in this conversation: aaa.");
        assert_eq!(earlier_in_conversation("", &[]), None);
    }

    /// The ring holds 50 events. Turns that leave it are folded into a
    /// summary by the model, so the conversation carries on.
    #[tokio::test]
    async fn turns_that_leave_the_ring_are_summarised_not_dropped() {
        let (planner, mut rx, dir) = planner_with_pending_change("compression");
        {
            let mut wm = planner.working.write().await;
            wm.push_event(utterance("aaa bbb"));
            for i in 0..60 {
                wm.push_event(utterance(&format!("filler {}", i)));
            }
            assert_eq!(wm.events.len(), 50);
            assert_eq!(wm.earlier.front().map(String::as_str), Some("User: aaa bbb"), "kept for the summary");
        }
        drain(&mut rx);
        planner.maybe_summarise().await;
        let asked = drain(&mut rx).into_iter().find(|e| e.event_type == EventType::WorkerRequest).unwrap();
        assert_eq!(asked.payload["request_type"], "summarise");
        assert_eq!(asked.payload["lines"][0], "User: aaa bbb");
        planner.maybe_summarise().await;
        assert!(drain(&mut rx).is_empty(), "one summary at a time");

        let summary = Event {
            id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            source: "worker".into(),
            event_type: EventType::WorkerResponse,
            payload: serde_json::json!({"type": "summary", "text": "The user said aaa bbb."}),
        };
        planner.handle_event(summary).await.unwrap();
        assert!(planner.working.read().await.earlier.is_empty(), "folded in");
        let items = planner.build_working_memory("what was I saying?", uuid::Uuid::new_v4(), None).await;
        let earlier = items.iter().find(|i| i["source"] == "earlier").expect("the summary");
        assert!(earlier["content"].as_str().unwrap().contains("The user said aaa bbb."), "{}", earlier);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn model_did(action: &str, said: &str) -> Event {
        Event {
            id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            source: "worker".into(),
            event_type: EventType::WorkerResponse,
            payload: serde_json::json!({ "type": "command", "command": action, "text": said }),
        }
    }

    /// Whatever the model picks is checked before anything runs.
    /// Answers about the system come from the Sentinel's record when the
    /// model asks for them.
    #[tokio::test]
    async fn system_changes_are_answered_from_the_record() {
        let (planner, mut rx, dir) = planner_with_pending_change("changes");
        let said = |rx: &mut tokio::sync::broadcast::Receiver<Event>| -> Vec<String> {
            drain(rx)
                .into_iter()
                .filter(|e| e.event_type == EventType::WorkerRequest)
                .flat_map(|e| facts(&e, "result"))
                .collect()
        };
        planner.handle_event(model_did("system_changes 7", "what changed lately")).await.unwrap();
        let answer = said(&mut rx);
        assert_eq!(answer.len(), 1, "{:?}", answer);
        assert!(answer[0].contains("alphawm"), "{}", answer[0]);
        planner.handle_event(model_did("update_details", "and that update?")).await.unwrap();
        assert!(said(&mut rx)[0].contains("alphawm"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// With a question open the model sees it and may answer it; its
    /// answer goes to the gate.
    #[tokio::test]
    async fn an_open_question_is_answered_through_the_model() {
        let (planner, mut rx, dir) = planner_with_pending_change("question");
        let audit = crate::safety::audit::AuditLog::new(&dir.join("audit.db")).unwrap();
        let mut gate = crate::safety::PermissionGate::new(
            Arc::new(EventBus::new()),
            Arc::new(Mutex::new(audit)),
            planner.awaiting.clone(),
        );
        gate.ask_for_test(serde_json::json!([{ "type": "shell", "command": "rm notes.txt" }])).await;

        planner.handle_event(utterance("yeah go on")).await.unwrap();
        let asked = drain(&mut rx).into_iter().find(|e| e.event_type == EventType::WorkerRequest).unwrap();
        let actions = asked.payload["actions"].to_string();
        assert!(actions.contains("confirm") && actions.contains("cancel"), "{}", actions);
        let waiting = facts(&asked, "waiting");
        assert!(waiting.len() == 1 && waiting[0].contains("run rm notes.txt"), "{:?}", waiting);

        planner.handle_event(model_did("confirm", "yeah go on")).await.unwrap();
        let answer = drain(&mut rx).into_iter().find(|e| e.event_type == EventType::Confirmation).unwrap();
        assert_eq!(answer.payload["approved"], true);

        gate.answer_for_test(false).await;
        planner.handle_event(utterance("hello")).await.unwrap();
        let asked = drain(&mut rx).into_iter().find(|e| e.event_type == EventType::WorkerRequest).unwrap();
        assert!(!asked.payload["actions"].to_string().contains("\"confirm"), "closed again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn stop_goes_quiet_without_a_word() {
        let (planner, mut rx, dir) = planner_with_pending_change("stop");
        planner.handle_event(model_did("stop", "stop")).await.unwrap();
        let events = drain(&mut rx);
        assert!(events.iter().any(|e| e.event_type == EventType::TtsInterrupt));
        assert!(!events.iter().any(|e| e.payload.to_string().contains("\"say\"")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Whatever the model picks is checked before anything runs. What can't
    /// be done goes back to the model as a result to tell.
    #[tokio::test]
    async fn the_models_action_is_checked_before_it_runs() {
        let (planner, mut rx, dir) = planner_with_pending_change("act");
        let cases = [
            ("undo", "undo that"),
            ("bluetooth_off", "kill the bluetooth"),
            ("logout", "sign me out now"),
            ("volume_set 40", "forty percent volume"),
            ("volume_set 140", "louder"),
            ("open About Desk", "what is open on workspace one"),
            ("format_disk", "wipe it"),
            ("volume_up 5", "louder"),
            ("volume_set <80>.", "turn it down to 80"),
        ];
        // Each turn: the plan that runs, or the result handed back to the model.
        let mut turns: Vec<Result<serde_json::Value, String>> = Vec::new();
        for (action, said) in cases {
            planner.handle_event(model_did(action, said)).await.unwrap();
            let events = drain(&mut rx);
            let result = events.iter().find(|e| e.event_type == EventType::WorkerRequest).and_then(|e| {
                e.payload["working_memory"]
                    .as_array()?
                    .iter()
                    .find(|i| i["source"] == "result")
                    .and_then(|i| i["content"].as_str().map(String::from))
            });
            let plan = events.iter().find(|e| e.event_type == EventType::PlanReady).map(|e| e.payload["plan"].clone());
            turns.push(match (plan, result) {
                (Some(plan), None) => Ok(plan),
                (None, Some(result)) => Err(result),
                other => panic!("{}: {:?}", action, other),
            });
        }
        let plan = |i: usize| turns[i].clone().unwrap();
        let told = |i: usize| turns[i].clone().unwrap_err();
        assert_eq!(plan(0), serde_json::json!([{"type": "undo"}]), "undo's result is told once known");
        assert_eq!((plan(1)[0]["op"].as_str(), plan(1)[1]["type"].as_str()), (Some("bluetooth_off"), Some("tell")));
        assert_eq!(plan(2)[0]["type"], "tell", "told before the session ends");
        assert_eq!((plan(3)[0]["value"].as_u64(), plan(3)[1]["fact"].as_str()), (Some(40), Some("Done: volume_set 40")));
        assert!(told(4).contains("between 0 and 100"));
        assert!(told(5).contains("never named"), "an app nobody named");
        assert!(told(6).contains("isn't one of your actions"));
        assert!(told(7).contains("isn't a form"), "an argument it doesn't take");
        assert_eq!(plan(8)[0]["value"].as_u64(), Some(80), "the menu's form is read too");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What other parts of MAVIS need said (a question, a warning) goes to
    /// the model to tell, never as a fixed sentence.
    #[tokio::test]
    async fn a_fact_from_elsewhere_is_told_by_the_model() {
        let (planner, mut rx, dir) = planner_with_pending_change("tell");
        let tell = Event {
            id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            source: "stt".into(),
            event_type: EventType::Tell,
            payload: serde_json::json!({"fact": "the mic is too quiet"}),
        };
        planner.handle_event(tell).await.unwrap();
        let events = drain(&mut rx);
        assert!(!events.iter().any(|e| e.event_type == EventType::PlanReady));
        let asked = events.iter().find(|e| e.event_type == EventType::WorkerRequest).unwrap();
        assert_eq!(facts(asked, "result"), ["the mic is too quiet"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A name is learned when the model hears one given, and only a name
    /// that was actually said.
    #[tokio::test]
    async fn a_name_is_learned_only_as_said() {
        let (planner, mut rx, dir) = planner_with_pending_change("name");
        planner.handle_event(model_did("user_name robin", "you can call me robin")).await.unwrap();
        assert_eq!(planner.working.read().await.user_name.as_deref(), Some("Robin"));
        drain(&mut rx);
        planner.handle_event(model_did("user_name Alex", "you can call me whatever")).await.unwrap();
        assert_eq!(planner.working.read().await.user_name.as_deref(), Some("Robin"), "never invented");
        let asked = drain(&mut rx).into_iter().find(|e| e.event_type == EventType::WorkerRequest).unwrap();
        assert!(facts(&asked, "result")[0].contains("isn't a name they said"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A failed action is told by the model, with the reason.
    #[tokio::test]
    async fn a_failure_is_told_in_the_models_words() {
        let (planner, mut rx, dir) = planner_with_pending_change("failed");
        planner.handle_event(utterance("brighter please")).await.unwrap();
        drain(&mut rx);
        let done = Event {
            id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            source: "executor".into(),
            event_type: EventType::ActionComplete,
            payload: serde_json::json!({"action_type": "system", "success": false, "error": "no backlight found\ndetails"}),
        };
        planner.handle_event(done).await.unwrap();
        let request = drain(&mut rx).into_iter().find(|e| e.event_type == EventType::WorkerRequest).unwrap();
        assert_eq!(request.payload["text"], "brighter please");
        assert!(request.payload.get("actions").is_none(), "only a reply");
        let wm = request.payload["working_memory"].to_string();
        assert!(wm.contains("no backlight found") && !wm.contains("details"), "{}", wm);
        let _ = std::fs::remove_dir_all(&dir);
    }

    mod media {
            use super::*;

        fn app(name: &str, exec: &str, opens: &[&str]) -> AppEntry {
            AppEntry {
                name: name.into(),
                exec: exec.into(),
                opens_links: opens.iter().any(|m| m.ends_with("/https")),
                opens: opens.iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            }
        }

        fn sites() -> (Site, Site) {
            let mut video = Site::new("Tube", "https://tube.test/results?q={query}");
            video.result_marker = Some("\"id\":\"".into());
            video.play_url = Some("https://tube.test/watch?v={id}".into());
            (video, Site::new("", "https://find.test/?q={query}"))
        }

        fn apps() -> Vec<AppEntry> {
            vec![
                app("Webby", "webby --new", &["text/html", "x-scheme-handler/https"]),
                app("Tunes", "tunes", &["x-scheme-handler/tunes"]),
                app("Player", "player", &["audio/mpeg", "video/mp4"]),
                app("Notes", "notes", &[]),
            ]
        }

        async fn resolved(plan: serde_json::Value, media: &Media) -> serde_json::Value {
            let mut plan = plan;
            resolve_media(&mut plan, &apps(), media).await;
            plan
        }

        #[test]
        fn config_gives_the_sites() {
            let text = "[media]\nsite_name = \"Tube\"\nsearch_url = \"https://tube.test/r?q={query}\"\n\
                        result_marker = '\"id\":\"'\nplay_url = \"https://tube.test/w?v={id}\"\n\
                        [web]\nsearch_url = \"https://find.test/?q={query}\"\n";
            let site = Site::from_config(text, "media").unwrap();
            assert_eq!(site.name, "Tube");
            assert_eq!(site.result_marker.as_deref(), Some("\"id\":\""));
            assert_eq!(site.search("lo fi"), "https://tube.test/r?q=lo+fi");
            assert!(Site::from_config(text, "web").is_some());
            assert!(Site::from_config("[media]\nsearch_url = \"no placeholder\"", "media").is_none());
        }

        #[test]
        fn the_shipped_config_has_both_sites() {
            let text = crate::listen::config_text().unwrap();
            let play = Site::from_config(&text, "media").unwrap();
            assert!(play.result_marker.is_some() && play.play_url.is_some());
            assert!(Site::from_config(&text, "web").is_some());
        }

        #[test]
        fn the_top_result_is_read_after_the_marker() {
            assert_eq!(id_after(r#"{"ad":"x"},{"id":"u9Dg-g7t2l4","t""#, "\"id\":\"").as_deref(), Some("u9Dg-g7t2l4"));
            assert_eq!(id_after("<html>consent</html>", "\"id\":\""), None);
        }

        #[tokio::test]
        async fn a_browser_gets_the_site_and_the_site_name_is_not_searched() {
            let (video, web) = sites();
            let media = Media::with_sites(Some(video), Some(web), None);
            let plan = resolved(serde_json::json!([{"type": "play", "query": "sunbeam on tube", "app": "Webby"}]), &media).await;
            assert_eq!(plan[0]["target"], "webby");
            assert_eq!(plan[0]["args"], serde_json::json!(["--new", "https://tube.test/results?q=sunbeam"]));
            assert_eq!(plan[1]["type"], "note", "the search page instead of playing is told");

            // No app, nothing remembered: the default browser.
            let plan = resolved(serde_json::json!([{"type": "play", "query": "sunbeam"}]), &media).await;
            assert_eq!(plan[0]["target"], "https://tube.test/results?q=sunbeam");

            let plan = resolved(serde_json::json!([{"type": "search", "query": "rust traits"}]), &media).await;
            assert_eq!(plan[0]["target"], "https://find.test/?q=rust+traits");
            assert_eq!(plan.as_array().unwrap().len(), 1, "a search as asked has nothing to tell");
        }

        #[tokio::test]
        async fn an_app_with_its_own_link_type_gets_a_search() {
            let media = Media::with_sites(Some(sites().0), None, None);
            let plan = resolved(serde_json::json!([{"type": "play", "query": "sunbeam by the band", "app": "Tunes"}]), &media).await;
            assert_eq!(plan[0]["args"], serde_json::json!(["tunes:search:sunbeam%20by%20the%20band"]));
            // Named the way a site would be: "search tunes for sunbeam".
            let plan = resolved(serde_json::json!([{"type": "play", "query": "sunbeam", "on": "tunes", "search_only": true}]), &media).await;
            assert_eq!(plan[0]["target"], "tunes");
            let plan = resolved(serde_json::json!([{"type": "play", "query": "sunbeam", "on": "elsewhere", "search_only": true}]), &media).await;
            assert_eq!(plan[0]["type"], "note");
            assert!(plan[0]["text"].as_str().unwrap().contains("elsewhere"));
        }

        #[tokio::test]
        async fn naming_the_site_or_the_web_uses_a_browser() {
            let dir = std::env::temp_dir().join(format!("mavis-site-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            let (video, web) = sites();
            let media = Media::with_sites(Some(video), Some(web), Some(dir.join("media_app")));
            resolved(serde_json::json!([{"type": "play", "query": "sunbeam", "app": "Tunes"}]), &media).await;
            // Tunes is remembered, but "on tube" means the site, in a browser.
            let plan = resolved(serde_json::json!([{"type": "play", "query": "sunrise on tube"}]), &media).await;
            assert_eq!(plan[0]["target"], "https://tube.test/results?q=sunrise", "{}", plan);
            // "search the web for X": a web search, not "I don't know how".
            let plan = resolved(serde_json::json!([{"type": "play", "query": "cats", "on": "the web", "search_only": true}]), &media).await;
            assert_eq!(plan[0]["target"], "https://find.test/?q=cats");
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn only_link_only_apps_get_a_search_link() {
            let term = app("Term", "term", &["text/plain", "inode/directory", "x-scheme-handler/term"]);
            assert!(own_scheme(&term).is_none());
            assert_eq!(own_scheme(&apps()[1]), Some("tunes"));
        }

        #[tokio::test]
        async fn an_app_that_cannot_search_is_opened_and_says_so() {
            let media = Media::with_sites(Some(sites().0), None, None);
            let plan = resolved(serde_json::json!([{"type": "play", "query": "sunbeam", "app": "Notes"}]), &media).await;
            assert_eq!(plan[0]["args"], serde_json::json!([]));
            assert_eq!(plan[1]["type"], "note");
            assert!(plan[1]["text"].as_str().unwrap().contains("Notes"));
        }

        #[tokio::test]
        async fn the_app_last_asked_for_is_used_next_time() {
            let dir = std::env::temp_dir().join(format!("mavis-media-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            let media = Media::with_sites(Some(sites().0), None, Some(dir.join("media_app")));
            resolved(serde_json::json!([{"type": "play", "query": "sunbeam", "app": "Webby"}]), &media).await;
            let plan = resolved(serde_json::json!([{"type": "play", "query": "sunrise"}]), &media).await;
            assert_eq!(plan[0]["target"], "webby", "{}", plan);
            // An app that couldn't play it isn't remembered.
            resolved(serde_json::json!([{"type": "play", "query": "x", "app": "Notes"}]), &media).await;
            let plan = resolved(serde_json::json!([{"type": "play", "query": "sunrise"}]), &media).await;
            assert_eq!(plan[0]["target"], "webby");
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_media_file_is_found_by_its_words() {
            let dir = std::env::temp_dir().join(format!("mavis-music-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(dir.join("Pop")).unwrap();
            std::fs::write(dir.join("Pop/The Band - Sunbeam.mp3"), b"").unwrap();
            std::fs::write(dir.join("Sunbeam notes.txt"), b"").unwrap();
            let found = find_media_file("sunbeam by the band", std::slice::from_ref(&dir)).unwrap();
            assert!(found.ends_with("Pop/The Band - Sunbeam.mp3"));
            assert!(find_media_file("sunrise", std::slice::from_ref(&dir)).is_none());
            let _ = std::fs::remove_dir_all(&dir);
        }
    }


    #[test]
    fn what_a_plan_says_is_read_from_lists_too() {
        let plan = serde_json::json!([{"type": "workspace", "index": 2}, {"type": "say", "text": "Workspace 2."}]);
        assert_eq!(plan_says(&plan), "Workspace 2.");
        assert_eq!(plan_says(&serde_json::json!({"type": "say", "text": "Yes?"})), "Yes?");
    }

    /// The model may only name apps or sites the user mentioned.
    #[test]
    fn targets_must_come_from_what_was_said() {
        let mut apps = desktop_apps();
        apps.push(AppEntry { name: "About Desk".into(), exec: "desk-about".into(), ..Default::default() });
        apps.push(AppEntry { name: "Snapkeep".into(), exec: "snapkeep-launcher".into(), ..Default::default() });
        assert!(grounded("webby", "kill the browser", &apps), "described by kind");
        assert!(grounded("Snapkeep", "shut snap keep", &apps), "said as two words");
        assert!(!grounded("About Desk", "Which window is on the second workspace?", &apps));
        assert!(!grounded("Webby", "Tell me about the latest updates.", &apps));
    }

    /// Every action the model is shown can be carried out, and the menu
    /// names each once.
    #[test]
    fn the_action_menu_matches_the_catalogue() {
        let menu = action_menu(false);
        assert_eq!(menu.len(), ACTIONS.len() - 2, "confirm and cancel only with a question open");
        assert_eq!(action_menu(true).len(), ACTIONS.len());
        assert!(menu.contains(&"volume_set <0-100>: set the speaker volume in percent".to_string()));
        let mut names: Vec<&str> = ACTIONS.iter().map(|a| a.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), ACTIONS.len(), "names are unique");
    }

    #[test]
    fn opening_by_name_by_kind_or_not_at_all() {
        let apps = desktop_apps();
        assert_eq!(open_plan("webby", &apps).unwrap()[0]["target"], "webby");
        assert_eq!(open_plan("tube.test", &apps).unwrap()[0]["target"], "https://tube.test");
        assert!(open_plan("zqxw", &apps).unwrap_err().contains("zqxw"), "what wasn't found is told");
    }

    #[test]
    fn closing_by_name_by_kind_or_the_focused_window() {
        let apps = desktop_apps();
        let mut ours = window("termy", "MAVIS", 1, true);
        ours.pid = Some(4242);
        let open = [window("webby", "News", 2, false), ours.clone()];
        let spared: std::collections::HashSet<u32> = [4242].into_iter().collect();
        assert!(matches!(close_app(Some("Webby"), &open, &apps, &spared), Close::Windows { .. }));
        assert!(matches!(close_app(None, &open, &apps, &spared), Close::Mine(_)), "never MAVIS's own window");
        assert!(matches!(close_app(Some("nothing here"), &open, &apps, &spared), Close::NotOpen(_)));
        assert_eq!(fullscreen_plan(Some("webby"), &open, &apps).unwrap()[0]["type"], "fullscreen");
        assert_eq!(fullscreen_plan(None, &open, &apps).unwrap()[0]["window"], ours.id.unwrap());
    }

    #[test]
    fn a_fact_to_remember_must_be_what_was_said() {
        assert!(restates("aaa bbb ccc ddd", "xxx bbb ccc ddd"));
        assert!(!restates("aaa bbb ccc ddd", "xxx yyy zzz"));
    }
}