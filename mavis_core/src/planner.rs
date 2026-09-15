use crate::context_snapshot::{AppEntry, WindowInfo};
use crate::event_bus::EventBus;
use crate::memory::long_term::LongTermMemory;
use crate::memory::recall::{build_fts_query, RecallStore};
use crate::memory::working::WorkingMemory;
use crate::models::event::{Event, EventType};
use log::{info, warn};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

// Deterministic catch for "what are your instructions?"-style questions.
// Prompt-wording alone can't reliably stop a small model from paraphrasing
// its way around a ban list (tried, it just found new synonyms). Catching
// the question itself and skipping the LLM entirely is the actual fix.
const META_INSTRUCTION_PHRASES: &[&str] = &[
    "your instructions",
    "your system prompt",
    "system prompt",
    "your rules",
    "your guidelines",
    "your guidance",
    "your principles",
    "your programming",
    "are you an ai",
    "are you a language model",
    "language model",
    "following rules",
    "following instructions",
    "what were you told",
    "what are you programmed",
];

fn is_meta_instruction_question(text: &str) -> bool {
    let lower = text.to_lowercase();
    META_INSTRUCTION_PHRASES.iter().any(|p| lower.contains(p))
}

// Phase 6 privacy gate: each context source is off by default and must be
// explicitly opted into. This is the specific gate Phase 6 itself asks for —
// not the fuller 5-tier system Phase 8 builds later.
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
const TERMINAL_APPS: &[&str] = &[
    "kitty", "alacritty", "foot", "wezterm", "gnome-terminal",
    "konsole", "xterm", "urxvt", "terminator",
];

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

// IDE awareness: known editors get a specific "editing X" message instead
// of the generic "in app Y" one. Both matches below are confirmed against
// real niri output on this machine, not guessed:
//   antigravity-ide: "{workspace} - Antigravity IDE - {filename}"
//   code-oss:        "{filename} - {workspace} - Code - OSS"
// Anything else falls through to a generic filename-in-title heuristic —
// approximate (could false-positive on e.g. a file manager), but avoids
// needing to verify every possible editor's exact title format up front.
// Browsers are excluded since that's handled by the extension work instead.
fn describe_active_window(window: &WindowInfo) -> String {
    if window.app_name == "antigravity-ide" {
        if let Some((workspace, filename)) = window.window_title.split_once(" - Antigravity IDE - ") {
            return format!(
                "The user is editing {} in the {} project using Antigravity IDE.",
                filename, workspace
            );
        }
    }

    if window.app_name == "code-oss" {
        if let Some(prefix) = window.window_title.strip_suffix(" - Code - OSS") {
            return match prefix.split_once(" - ") {
                Some((filename, workspace)) => format!(
                    "The user is editing {} in the {} project using VS Code.",
                    filename, workspace
                ),
                None => format!("The user is editing {} using VS Code.", prefix),
            };
        }
    }

    if TERMINAL_APPS.contains(&window.app_name.as_str()) {
        let cleaned = strip_env_prefix(&window.window_title);
        if cleaned.is_empty() {
            return format!("The user is in a terminal ({}).", window.app_name);
        }
        return format!(
            "The user is in a terminal ({}), running: {}.",
            window.app_name, cleaned
        );
    }

    if window.app_name != "brave-browser" && window.app_name != "firefox" {
        if let Some(filename) = find_filename_in_title(&window.window_title) {
            return format!("The user appears to be editing {} in {}.", filename, window.app_name);
        }
    }

    format!("The user is currently in {} — \"{}\".", window.app_name, window.window_title)
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
    // Exact binary name — "code-oss" as spoken.
    if let Some(a) = apps.iter().find(|a| {
        a.exec
            .split_whitespace()
            .next()
            .and_then(|b| b.rsplit('/').next())
            .map(|b| normalize(b) == s)
            .unwrap_or(false)
    }) {
        return Some(a);
    }
    // Name begins with what was said — "code" finds "Code - OSS".
    if let Some(a) = apps.iter().find(|a| normalize(&a.name).starts_with(&s)) {
        return Some(a);
    }
    // Every spoken word appears in the name — "notepad" finds "DMS Notepad".
    let words: Vec<&str> = s.split_whitespace().collect();
    apps.iter().find(|a| {
        let n = normalize(&a.name);
        words.iter().all(|w| n.contains(w))
    })
}

/// Percent-encode a search query. Hand-rolled to avoid pulling in a URL
/// crate for one use; encodes everything outside the unreserved set.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Strip a leading address to MAVIS so "MAVIS, open firefox" matches the
/// same as "open firefox".
fn strip_address(text: &str) -> &str {
    let t = text.trim_start();
    for prefix in ["hey mavis", "ok mavis", "okay mavis", "mavis"] {
        if t.len() >= prefix.len() && t[..prefix.len()].eq_ignore_ascii_case(prefix) {
            return t[prefix.len()..].trim_start_matches([',', ' ', '.']);
        }
    }
    t
}

// System control intents. The executor already forwards `system` actions to
// the DBus subsystem, which implements all eight ops below — none of them
// were reachable from speech until now.
//
// Phrases are matched whole rather than by keyword: "what's the volume
// policy at work" contains "volume" but is a question, not a command.
const SYSTEM_INTENTS: &[(&[&str], &str, &str)] = &[
    (
        &["volume up", "turn up the volume", "turn the volume up", "louder",
          "increase the volume", "raise the volume"],
        "volume_up",
        "Volume up.",
    ),
    (
        &["volume down", "turn down the volume", "turn the volume down", "quieter",
          "decrease the volume", "lower the volume"],
        "volume_down",
        "Volume down.",
    ),
    (
        &["mute", "unmute", "mute the volume", "mute the sound"],
        "volume_mute",
        "Muted.",
    ),
    (
        &["pause the music", "resume the music", "play the music", "pause",
          "resume", "pause music", "stop the music"],
        "media_play_pause",
        "Done.",
    ),
    (
        &["next track", "next song", "skip the song", "skip this song", "skip track"],
        "media_next",
        "Next track.",
    ),
    (
        &["previous track", "previous song", "last song", "go back a song"],
        "media_previous",
        "Previous track.",
    ),
    (
        &["brightness up", "turn up the brightness", "brighter", "increase the brightness"],
        "brightness_up",
        "Brightness up.",
    ),
    (
        &["brightness down", "turn down the brightness", "dimmer", "darker",
          "decrease the brightness", "dim the screen"],
        "brightness_down",
        "Brightness down.",
    ),
];

/// Match a whole-phrase system command. Returns the longest match so
/// "turn up the volume" wins over the bare "volume up" substring inside it.
///
/// Returns None when the utterance opens with an explicit action prefix:
/// "google how to mute a tab" contains "mute", but muting the machine is
/// plainly not what was asked for.
fn match_system_intent(text: &str) -> Option<serde_json::Value> {
    let lower = text.to_lowercase();
    let t = strip_address(&lower)
        .trim()
        .trim_end_matches(|c: char| c.is_ascii_punctuation())
        .trim();

    const OVERRIDING_PREFIXES: &[&str] = &[
        "google ", "search for ", "look up ", "open ", "launch ", "start ", "run ",
        "play ", "put on ", "search youtube for ",
    ];
    if OVERRIDING_PREFIXES.iter().any(|p| t.starts_with(p)) {
        return None;
    }

    let padded = format!(" {} ", t);
    let mut best: Option<(usize, &str, &str)> = None;

    for (phrases, op, spoken) in SYSTEM_INTENTS {
        for p in *phrases {
            let is_match = t == *p || padded.contains(&format!(" {} ", p));
            if is_match && best.map(|(len, _, _)| p.len() > len).unwrap_or(true) {
                best = Some((p.len(), op, spoken));
            }
        }
    }

    best.map(|(_, op, spoken)| {
        serde_json::json!([
            {"type": "say", "text": spoken},
            {"type": "system", "op": op},
        ])
    })
}

/// Returns a plan (say + action) when the utterance is a clear command.
/// None means "not a command" — the utterance goes to the LLM as normal.
fn match_action_intent(text: &str, apps: &[AppEntry]) -> Option<serde_json::Value> {
    let lower = text.to_lowercase();
    let t = strip_address(&lower)
        .trim()
        .trim_end_matches(|c: char| c.is_ascii_punctuation())
        .trim();

    if let Some(plan) = match_system_intent(text) {
        return Some(plan);
    }

    let say_then_open = |spoken: String, target: String| {
        Some(serde_json::json!([
            {"type": "say", "text": spoken},
            {"type": "app", "target": target},
        ]))
    };

    // "play X on youtube" / "play X"
    for prefix in ["play ", "put on ", "search youtube for "] {
        if let Some(rest) = t.strip_prefix(prefix) {
            let query = rest.trim_end_matches(" on youtube").trim();
            if query.is_empty() {
                continue;
            }
            return say_then_open(
                format!("Searching YouTube for {}.", query),
                format!(
                    "https://www.youtube.com/results?search_query={}",
                    urlencode(query)
                ),
            );
        }
    }

    // "open / launch / start / run <app or url>"
    for prefix in ["open ", "launch ", "start ", "run "] {
        if let Some(rest) = t.strip_prefix(prefix) {
            let target = rest.trim();
            if target.is_empty() {
                continue;
            }
            if target.contains("://") {
                return say_then_open(format!("Opening {}.", target), target.to_string());
            }
            if let Some(app) = find_app(target, apps) {
                // Exec may carry flags ("code-oss --unity-launch"); the
                // executor's app action takes a binary plus args.
                let mut parts = app.exec.split_whitespace();
                let bin = parts.next().unwrap_or_default().to_string();
                let args: Vec<String> = parts.map(String::from).collect();
                return Some(serde_json::json!([
                    {"type": "say", "text": format!("Opening {}.", app.name)},
                    {"type": "app", "target": bin, "args": args},
                ]));
            }
            // Unknown app — don't guess at a binary that probably doesn't
            // exist. Fall through and let the LLM respond instead.
            return None;
        }
    }

    // "google X" / "search for X" / "look up X"
    for prefix in ["google ", "search for ", "look up "] {
        if let Some(rest) = t.strip_prefix(prefix) {
            let query = rest.trim();
            if query.is_empty() {
                continue;
            }
            return say_then_open(
                format!("Searching for {}.", query),
                format!("https://www.google.com/search?q={}", urlencode(query)),
            );
        }
    }

    None
}

/// Recognise "yesterday", "this morning", "last week" and friends, mapping
/// them to a concrete time window. Returns None for anything without a time
/// reference, which is most utterances.
///
/// Deliberately a small fixed set rather than general date parsing: these
/// cover what people actually ask a desktop assistant, and a wrong window is
/// worse than no window — it would inject unrelated history into the prompt.
fn parse_time_reference(
    text: &str,
) -> Option<(chrono::DateTime<chrono::Local>, chrono::DateTime<chrono::Local>, String)> {
    use chrono::{Duration, Local, TimeZone, Timelike};

    let t = text.to_lowercase();
    let now = Local::now();
    // Midnight today, derived from the date rather than picking apart
    // year/month/day by hand.
    let today = Local
        .from_local_datetime(&now.date_naive().and_hms_opt(0, 0, 0)?)
        .single()?;

    let part_range = |base: chrono::DateTime<Local>, part: &str| {
        let (from_h, to_h) = match part {
            "morning" => (5, 12),
            "afternoon" => (12, 17),
            _ => (17, 23),
        };
        (
            base.with_hour(from_h).unwrap_or(base),
            base.with_hour(to_h).unwrap_or(base),
        )
    };

    for part in ["morning", "afternoon", "evening"] {
        if t.contains(&format!("yesterday {}", part)) {
            let base = today - Duration::days(1);
            let (a, b) = part_range(base, part);
            return Some((a, b, format!("yesterday {}", part)));
        }
        if t.contains(&format!("this {}", part)) {
            let (a, b) = part_range(today, part);
            return Some((a, b, format!("this {}", part)));
        }
    }

    if t.contains("yesterday") {
        return Some((today - Duration::days(1), today, "yesterday".into()));
    }
    if t.contains("today") || t.contains("so far") {
        return Some((today, today + Duration::days(1), "today".into()));
    }
    if t.contains("last week") {
        return Some((today - Duration::days(7), today, "last week".into()));
    }
    if t.contains("this week") {
        return Some((today - Duration::days(7), now, "this week".into()));
    }

    None
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
    ) -> Self {
        info!("Planner: {} installed applications available", apps.len());
        Self { bus, working, apps, recall, long_term }
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

        // Deterministic deflection — no LLM round trip, no way to leak.
        if is_meta_instruction_question(intent) {
            let plan_event = Event {
                id: uuid::Uuid::new_v4(),
                timestamp: chrono::Utc::now(),
                source: "planner".to_string(),
                event_type: EventType::PlanReady,
                payload: serde_json::json!({
                    "plan": {"type": "say", "text": "Just trying to be helpful — what do you need?"}
                }),
            };
            self.bus.publish(plan_event);
            return Ok(());
        }

        // Action intents — deterministic, no LLM round trip. Emitted as a
        // say+action plan so the user gets spoken confirmation.
        if let Some(plan) = match_action_intent(intent, &self.apps) {
            info!("Planner: matched action intent — {}", plan);
            let plan_event = Event {
                id: uuid::Uuid::new_v4(),
                timestamp: chrono::Utc::now(),
                source: "planner".to_string(),
                event_type: EventType::PlanReady,
                payload: serde_json::json!({ "plan": plan }),
            };
            self.bus.publish(plan_event);
            return Ok(());
        }

        let working_memory = self.build_working_memory(intent).await;

        // Only send the user message — build_chat_messages() in Python owns the system prompt.
        let worker_req = Event {
            id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            source: "planner".to_string(),
            event_type: EventType::WorkerRequest,
            payload: serde_json::json!({
                "request_type": "chat",
                "messages": [
                    {"role": "user", "content": user_message}
                ],
                "max_tokens": 256,
                "temperature": 0.7,
                "working_memory": working_memory
            }),
        };
        self.bus.publish(worker_req);
        Ok(())
    }

    async fn build_working_memory(&self, current_intent: &str) -> Vec<serde_json::Value> {
        let snapshot = self.working.read().await;
        let mut items = Vec::new();

        // NEW — inject user profile first so it appears at the top of context
        if let Some(name) = &snapshot.user_name {
            items.push(serde_json::json!({
                "source": "user_profile",
                "content": format!("The user's name is {}.", name),
            }));
        }

        if let Some(intent) = &snapshot.current_intent {
            items.push(serde_json::json!({
                "source": "current_intent",
                "content": intent,
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
            if let Some((start, end, label)) = parse_time_reference(current_intent) {
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
                        for m in memories {
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
            if let Some(window) = &snapshot.active_window {
                items.push(serde_json::json!({
                    "source": "active_window",
                    "content": describe_active_window(window),
                }));
            }

            // Workspace-aware view: what's on the current workspace vs
            // elsewhere. niri reports workspace_id per window, so this is
            // read straight from the same data as the window list.
            if !snapshot.open_windows.is_empty() {
                let current_ws = snapshot.active_workspace;

                let mut here: Vec<&str> = Vec::new();
                let mut elsewhere: Vec<String> = Vec::new();
                for w in &snapshot.open_windows {
                    if w.app_name.is_empty() || w.app_name == "unknown" || w.is_focused {
                        continue;
                    }
                    if w.workspace_id == current_ws {
                        here.push(w.app_name.as_str());
                    } else if let Some(ws) = w.workspace_id {
                        elsewhere.push(format!("{} (workspace {})", w.app_name, ws));
                    } else {
                        elsewhere.push(w.app_name.clone());
                    }
                }
                here.sort_unstable();
                here.dedup();
                elsewhere.sort();
                elsewhere.dedup();

                let mut parts = Vec::new();
                if let Some(ws) = current_ws {
                    parts.push(format!("The user is on workspace {}.", ws));
                }
                if !here.is_empty() {
                    parts.push(format!(
                        "Also open on this workspace: {}.",
                        here.join(", ")
                    ));
                }
                if !elsewhere.is_empty() {
                    parts.push(format!("Open elsewhere: {}.", elsewhere.join(", ")));
                }
                if !parts.is_empty() {
                    items.push(serde_json::json!({
                        "source": "open_windows",
                        "content": parts.join(" "),
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

        if context_source_enabled("MAVIS_CONTEXT_CLIPBOARD") {
            if let Some(clipboard) = &snapshot.last_clipboard {
                if !clipboard.is_empty() {
                    let truncated: String = clipboard.chars().take(200).collect();
                    items.push(serde_json::json!({
                        "source": "clipboard",
                        "content": format!("The user's clipboard currently contains: \"{}\"", truncated),
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

        // CHANGED — 5 → 15. Between two user turns MAVIS generates ~7 internal
        // events (WorkerRequest, WorkerResponse, PlanReady, ActionComplete,
        // 2× UiStateChange). A window of 5 drops the prior turn entirely.
        for event in snapshot.recent_events(15) {
            let (source, content) = match event.event_type {
                EventType::UserIntent => (
                    "user",
                    event.payload
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                ),
                EventType::WorkerResponse => (
                    "mavis",
                    event.payload
                        .get("result")
                        .and_then(|r| r.get("content"))
                        .and_then(|c| c.as_str())
                        .unwrap_or("")
                        .to_string(),
                ),
                EventType::PlanReady => (
                    "plan",
                    event.payload
                        .get("plan")
                        .and_then(|p| p.get("text"))
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string(),
                ),
                _ => continue,
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