// mavis_core/src/context_engine.rs
// Central nervous system. Owns Working Memory and routes all events.

use crate::context_snapshot::{BrowserTab, ContextSnapshot};
use crate::memory::entities::EntityKind;
use crate::event_bus::EventBus;
use crate::memory::manager::MemoryManager;
use crate::models::event::{Event, EventType};
use anyhow::Result;
use log::{info, warn};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

pub struct ContextEngine {
    memory: MemoryManager,
    bus: Arc<EventBus>,
    last_save: Mutex<Instant>,
    /// Last observed app/project/file combination. Context updates arrive
    /// every 2 s, so recording every one would count idle seconds rather
    /// than actual work — entities are only recorded when this changes.
    last_entity_signature: Mutex<Option<String>>,
    /// Last context summary logged, so the 2 s poll logs changes only.
    last_context_log: Mutex<Option<String>>,
}

impl ContextEngine {
    pub fn new(bus: Arc<EventBus>, memory: MemoryManager) -> Result<Self> {
        Ok(Self {
            memory,
            bus,
            last_save: Mutex::new(Instant::now() - Duration::from_secs(10)),
            last_entity_signature: Mutex::new(None),
            last_context_log: Mutex::new(None),
        })
    }

    pub async fn process_event(&mut self, event: Event) -> Result<()> {
        self.maybe_persist(&event).await;

        {
            let mut wm = self.memory.working.write().await;
            // Only conversational events go in the ring. ContextUpdate fires
            // every 2 s and UiStateChange on every transition, so storing
            // them filled all 50 slots with polling noise inside two minutes
            // and evicted the actual conversation — which is the only thing
            // the planner reads back. The context itself isn't lost: it
            // lives in dedicated fields (active_window, clipboard, project).
            if matches!(
                event.event_type,
                EventType::UserIntent
                    | EventType::WorkerResponse
                    | EventType::PlanReady
                    | EventType::ActionComplete
                    | EventType::SystemWake
            ) {
                wm.push_event(event.clone());
            }
        }

        let event_type = event.event_type.clone();
        match event_type {
            EventType::SystemWake => {
                info!("ContextEngine: MAVIS woke up — payload: {}", event.payload);
            }

            EventType::UserIntent => {
                info!("ContextEngine: UserIntent received — updating working memory");
                let intent = event
                    .payload
                    .get("text")
                    .and_then(|v| v.as_str())
                    .or_else(|| event.payload.get("intent").and_then(|v| v.as_str()))
                    .unwrap_or("unknown");

                {
                    let mut wm = self.memory.working.write().await;
                    wm.set_intent(intent.to_string());

                    if let Some(name) = extract_user_name(intent) {
                        if wm.user_name.as_ref() != Some(&name) && wm.set_user_name(name.clone()) {
                            info!("ContextEngine: stored user name: {}", name);
                        }
                    }
                }

                // Persist to searchable memory. Low-importance chatter is
                // dropped inside record() rather than filtered here.
                {
                    let recall = self.memory.recall.lock().await;
                    if let Err(e) =
                        recall.record("user", intent, &event.timestamp.to_rfc3339())
                    {
                        warn!("ContextEngine: failed to record memory: {}", e);
                    }
                }
            }

            EventType::WorkerResponse => {
                info!("ContextEngine: WorkerResponse received — routing");
                self.route_worker_response(event).await?;
            }

            EventType::ContextUpdate => {
                if let Some(payload) = event.payload.as_object() {
                    match serde_json::from_value::<ContextSnapshot>(serde_json::Value::Object(payload.clone())) {
                        Ok(snapshot) => {
                            let summary = {
                                let mut wm = self.memory.working.write().await;
                                wm.active_window = snapshot.active_window;
                                wm.open_windows = snapshot.open_windows;
                                wm.active_workspace = snapshot.active_workspace;
                                wm.project = snapshot.project;
                                wm.next_event = snapshot.next_event;
                                wm.last_clipboard = snapshot.clipboard_text;
                                wm.context_timestamp = Some(snapshot.captured_at);
                                // Length, not content: the log is often
                                // pasted into bug reports, and the clipboard
                                // is exactly where passwords end up.
                                format!(
                                    "app={}, project={}, clipboard={}",
                                    wm.active_window
                                        .as_ref()
                                        .map(|w| w.app_name.as_str())
                                        .unwrap_or("none"),
                                    wm.project.as_ref().map(|p| p.name.as_str()).unwrap_or("none"),
                                    wm.last_clipboard
                                        .as_ref()
                                        .map(|c| format!("{} chars", c.chars().count()))
                                        .unwrap_or_else(|| "empty".to_string()),
                                )
                            };
                            // This fires every 2 s; logging each one buried
                            // everything else. Log only what changed.
                            {
                                let mut last = self.last_context_log.lock().await;
                                if last.as_deref() != Some(summary.as_str()) {
                                    info!("ContextEngine: context changed — {}", summary);
                                    *last = Some(summary);
                                }
                            }
                            self.observe_entities(&event.timestamp.to_rfc3339()).await;
                        }
                        Err(e) => {
                            warn!("ContextEngine: failed to parse ContextUpdate payload: {}", e);
                        }
                    }
                } else {
                    warn!("ContextEngine: ContextUpdate payload is not an object");
                }
            }

            EventType::PlanReady => {
                info!("ContextEngine: PlanReady — updating working memory");
                if let Some(plan) = event.payload.get("plan") {
                    {
                        let mut wm = self.memory.working.write().await;
                        wm.set_active_plan(plan.clone());
                    }
                    // What MAVIS actually said, for later recall.
                    if let Some(text) = plan.get("text").and_then(|t| t.as_str()) {
                        let recall = self.memory.recall.lock().await;
                        if let Err(e) =
                            recall.record("mavis", text, &event.timestamp.to_rfc3339())
                        {
                            warn!("ContextEngine: failed to record memory: {}", e);
                        }
                    }
                }
            }

            EventType::ActionComplete => {
                info!("ContextEngine: ActionComplete — clearing active state");
                {
                    let mut wm = self.memory.working.write().await;
                    wm.clear_active_plan();
                    wm.clear_intent();
                }
            }

            EventType::UiStateChange => {
                let state = event.payload.get("state").and_then(|v| v.as_str());
                if let Some(s) = state {
                    let mut wm = self.memory.working.write().await;
                    wm.ui_state = Some(s.to_string());
                }
                info!("ContextEngine: UI state updated to {:?}", state);
            }

            EventType::WorkerRequest => {
                info!(
                    "ContextEngine: observed WorkerRequest from {}",
                    event.source
                );
                let snapshot = self.working_memory_snapshot().await;
                info!(
                    "ContextEngine: working memory — intent={:?}, has_plan={}, events={}",
                    snapshot.current_intent,
                    snapshot.active_plan.is_some(),
                    snapshot.events.len()
                );
            }

            EventType::SystemAction => {
                info!("ContextEngine: SystemAction observed — {}", event.payload);
            }

            EventType::TtsInterrupt => {
                info!("ContextEngine: TTS interrupt observed");
            }

            EventType::PlanApproved => {
                // The gate's verdict, not a new decision — working memory
                // already recorded the plan when it was proposed.
                info!("ContextEngine: plan approved for execution");
            }

            EventType::BrowserUpdate => {
                if let Some(data) = event.payload.as_object() {
                    match serde_json::from_value::<BrowserTab>(
                        serde_json::Value::Object(data.clone()),
                    ) {
                        Ok(tab) => {
                            info!("ContextEngine: browser tab updated — {}", tab.domain);
                            let mut wm = self.memory.working.write().await;
                            wm.browser_tab = Some(tab);
                        }
                        Err(e) => {
                            warn!("ContextEngine: failed to parse BrowserUpdate payload: {}", e);
                        }
                    }
                }
            }

            // Phase 8.5 — the Sentinel saw the machine change under us.
            //
            // Deliberately not pushed into working memory: the sentinel's
            // own store is the record, and a big update would otherwise
            // evict the actual conversation from the ring, which is the
            // exact failure the polling filter above exists to prevent.
            EventType::SystemChange => {
                let count = event
                    .payload
                    .get("count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let severity = event
                    .payload
                    .get("severity")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                info!(
                    "ContextEngine: Sentinel reported {} system change(s), peak severity {}",
                    count, severity
                );
            }
        }

        // Phase 5 — debounced save of working memory to disk
        self.maybe_save().await;

        Ok(())
    }

    async fn route_worker_response(&self, event: Event) -> Result<()> {
        let payload = &event.payload;
        let response_type = payload.get("type").and_then(|v| v.as_str());

        match response_type {
            Some("context") => {
                let ctx_event = Event {
                    id: uuid::Uuid::new_v4(),
                    timestamp: chrono::Utc::now(),
                    source: "context_engine".to_string(),
                    event_type: EventType::ContextUpdate,
                    payload: payload
                        .get("data")
                        .cloned()
                        .unwrap_or_else(|| payload.clone()),
                };
                self.bus.publish(ctx_event);
            }
            Some(other) => {
                info!(
                    "ContextEngine: passing WorkerResponse type '{}' to Planner",
                    other
                );
            }
            None => {
                warn!("ContextEngine: WorkerResponse missing 'type' field");
            }
        }

        Ok(())
    }

    /// Record the projects, apps and files currently in play, and link them
    /// to each other. Only fires when the combination changes — otherwise a
    /// 2 s poll would inflate every count by 30 a minute.
    async fn observe_entities(&self, when: &str) {
        let (app, project, file) = {
            let wm = self.memory.working.read().await;
            let app = wm.active_window.as_ref().map(|w| w.app_name.clone());
            let project = wm.project.as_ref().map(|p| p.name.clone());
            // Filenames only come from editors, where the title parsing is
            // verified; anything else would be guesswork.
            let file = wm.active_window.as_ref().and_then(|w| {
                w.window_title
                    .split_whitespace()
                    .find(|t| t.contains('.') && !t.contains('/') && t.len() > 3)
                    .map(|t| t.trim_matches(|c: char| c.is_ascii_punctuation()).to_string())
            });
            (app, project, file)
        };

        let signature = format!("{:?}|{:?}|{:?}", app, project, file);
        {
            let mut last = self.last_entity_signature.lock().await;
            if last.as_deref() == Some(signature.as_str()) {
                return;
            }
            *last = Some(signature);
        }

        let store = self.memory.entities.lock().await;
        if let Some(a) = &app {
            let _ = store.observe(a, EntityKind::App, when);
        }
        if let Some(p) = &project {
            let _ = store.observe(p, EntityKind::Project, when);
        }
        if let Some(f) = &file {
            let _ = store.observe(f, EntityKind::File, when);
        }
        // Co-occurrence is the useful part: which app, project and file are
        // in play together.
        if let (Some(p), Some(a)) = (&project, &app) {
            let _ = store.link((p, EntityKind::Project), (a, EntityKind::App), when);
        }
        if let (Some(p), Some(f)) = (&project, &file) {
            let _ = store.link((p, EntityKind::Project), (f, EntityKind::File), when);
        }
    }

    async fn maybe_persist(&self, event: &Event) {
        match event.event_type {
            EventType::UserIntent | EventType::ActionComplete | EventType::PlanReady => {
                let store = self.memory.episodic.lock().await;
                if let Err(e) = store.record(event) {
                    warn!("Failed to persist event to episodic memory: {}", e);
                }
            }
            _ => {}
        }
    }

    pub async fn working_memory_snapshot(&self) -> crate::memory::working::WorkingMemory {
        self.memory.working.read().await.clone()
    }

    /// Save working memory to disk at most once per second.
    async fn maybe_save(&self) {
        let mut last = self.last_save.lock().await;
        if last.elapsed() > Duration::from_secs(1) {
            if let Err(e) = self.memory.save_working().await {
                warn!("ContextEngine: failed to save working memory snapshot: {}", e);
            }
            *last = Instant::now();
        }
    }
}

// ---------------------------------------------------------------------
// Name extraction — simple pattern matching, no NLP dependency.
// ---------------------------------------------------------------------

/// Learn a name only when it is unmistakably being given.
///
/// No list of "words that aren't names". Earlier versions matched "I'm …"
/// and then kept a denylist of what followed — here, sorry, looking,
/// using… — which grew with every false match and could never be finished.
/// Instead, the *shape* of the sentence decides:
///
/// 1. An explicit introduction: "my name is X", "call me X", "I'm called X".
///    "I'm X" / "I am X" don't count — they introduce a state ("I'm using
///    the terminal") far more often than a name.
/// 2. X ends its clause: followed by nothing, punctuation, or "and". Names
///    come last ("my name is Manav", "call me Azazel, please");
///    other words keep going ("call me back later", "my name is not
///    important").
/// 3. Not a question: "what's my name is what I asked?" introduces nothing.
/// 4. For "call me", X must be capitalised as said. Whisper capitalises
///    proper nouns; "call me maybe" / "call me back" come out lowercase.
///    "my name is" is unambiguous enough to accept lowercase, which matters
///    for typed input.
///
/// MAVIS then addresses the user by it, so a wrong one is heard and can be
/// corrected by saying it again.
fn extract_user_name(text: &str) -> Option<String> {
    // Work on the clause containing the phrase, so a question elsewhere in
    // the utterance doesn't disqualify an introduction.
    for sentence in text.split_inclusive(['.', '!', '?']) {
        if sentence.trim_end().ends_with('?') {
            continue;
        }
        let words: Vec<&str> = sentence.split_whitespace().collect();
        let lower: Vec<String> = words
            .iter()
            .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'').to_lowercase())
            .collect();

        for i in 0..words.len() {
            let (next, needs_capital) = match lower[i].as_str() {
                "my" if lower.get(i + 1).is_some_and(|w| w == "name")
                    && lower.get(i + 2).is_some_and(|w| w == "is") =>
                {
                    (i + 3, false)
                }
                "call" if lower.get(i + 1).is_some_and(|w| w == "me") => (i + 2, true),
                "called" if i > 0 && matches!(lower[i - 1].as_str(), "i'm" | "am") => {
                    (i + 1, false)
                }
                _ => continue,
            };
            let Some(raw) = words.get(next) else { continue };
            let name = raw.trim_matches(|c: char| !c.is_alphabetic() && c != '\'' && c != '-');
            if !crate::memory::working::is_plausible_name(name) {
                continue;
            }
            // Clause end: last word, trailing punctuation, or "and" next.
            let ends_clause = next + 1 == words.len()
                || raw.ends_with([',', '.', '!', ';', ':'])
                || lower.get(next + 1).is_some_and(|w| w == "and");
            if !ends_clause {
                continue;
            }
            let capitalised = name.chars().next().is_some_and(|c| c.is_uppercase());
            if needs_capital && !capitalised {
                continue;
            }
            let mut chars = name.chars();
            let first = chars.next()?;
            return Some(first.to_uppercase().collect::<String>() + chars.as_str());
        }
    }
    None
}