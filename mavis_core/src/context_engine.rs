// mavis_core/src/context_engine.rs
// Central nervous system. Owns Working Memory and routes all events.

use crate::context_snapshot::ContextSnapshot;
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
}

impl ContextEngine {
    pub fn new(bus: Arc<EventBus>, memory: MemoryManager) -> Result<Self> {
        Ok(Self {
            memory,
            bus,
            last_save: Mutex::new(Instant::now() - Duration::from_secs(10)),
        })
    }

    pub async fn process_event(&mut self, event: Event) -> Result<()> {
        self.maybe_persist(&event).await;

        {
            let mut wm = self.memory.working.write().await;
            wm.push_event(event.clone());
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
                        if wm.user_name.as_ref() != Some(&name) {
                            wm.set_user_name(name.clone());
                            info!("ContextEngine: stored user name: {}", name);
                        }
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
                            let mut wm = self.memory.working.write().await;
                            wm.active_window = snapshot.active_window;
                            wm.last_clipboard = snapshot.clipboard_text;
                            wm.context_timestamp = Some(snapshot.captured_at);
                            info!(
                                "ContextEngine: context injected — app={}, clipboard={}",
                                wm.active_window.as_ref().map(|w| w.app_name.as_str()).unwrap_or("none"),
                                wm.last_clipboard.as_ref().map(|s| &s[..s.len().min(20)]).unwrap_or("none")
                            );
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
                    let mut wm = self.memory.working.write().await;
                    wm.set_active_plan(plan.clone());
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

/// Common words that are NOT names. Prevents false positives like
/// "No, I'm here" → "Here".
const NAME_DENYLIST: &[&str] = &[
    "here", "there", "sure", "ok", "fine", "good", "ready", "back",
    "home", "done", "right", "well", "yeah", "yes", "no", "maybe",
    "nothing", "something", "anything", "everything", "someone",
    "everyone", "nobody", "anybody", "today", "tomorrow", "yesterday",
    "talking", "speaking", "listening", "waiting", "coming", "going",
    "not", "in", "looking", "mavis",
];
fn extract_user_name(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let patterns = [
        ("my name is ", 11),
        ("i am ", 5),
        ("i'm ", 4),
        ("call me ", 8),
        ("name is ", 8),
    ];

    for (prefix, prefix_len) in patterns {
        if let Some(pos) = lower.find(prefix) {
            let start = pos + prefix_len;
            let rest = &text[start..];
            let name: String = rest
                .trim_start()
                .split_whitespace()
                .next()?
                .trim_end_matches(|c: char| c.is_ascii_punctuation())
                .to_string();
            if name.is_empty() || name.len() >= 30 {
                continue;
            }
            if NAME_DENYLIST.contains(&name.to_lowercase().as_str()) {
                continue;
            }
            let mut chars = name.chars();
            let capitalized = match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
                None => name,
            };
            return Some(capitalized);
        }
    }
    None
}