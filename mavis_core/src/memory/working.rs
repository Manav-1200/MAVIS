// mavis_core/src/memory/working.rs
// Working memory: transient, fast-access context for the current session.

#![allow(dead_code)]

use crate::context_snapshot::{BrowserTab, CalendarEvent, ProjectInfo, WindowInfo};
use crate::models::event::Event;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

const MAX_EVENTS: usize = 50;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WorkingMemory {
    pub events: VecDeque<Event>,
    pub current_intent: Option<String>,
    pub active_plan: Option<serde_json::Value>,
    pub ui_state: Option<String>,
    pub active_window: Option<WindowInfo>,
    #[serde(default)]
    pub open_windows: Vec<WindowInfo>,
    #[serde(default)]
    pub active_workspace: Option<u64>,
    #[serde(default)]
    pub project: Option<ProjectInfo>,
    #[serde(default)]
    pub next_event: Option<CalendarEvent>,
    pub last_clipboard: Option<String>,
    pub context_timestamp: Option<u64>,
    pub user_name: Option<String>,
    /// Set when the name was learned by the current rules. Names stored
    /// by builds before 2026-09-22 don't have it: those came from "I'm …"
    /// patterns that produced "Using", "Sorry", "Here" — so they aren't
    /// trusted, and are dropped on load. The user says it once more.
    #[serde(default)]
    pub user_name_verified: bool,
    pub browser_tab: Option<BrowserTab>,
}

/// A name is one word of letters (apostrophes and hyphens allowed, as in
/// O'Neil or Anne-Marie) under 30 bytes. What makes a word a *name* is how
/// it was said — see `extract_user_name` — not whether it's missing from a
/// list of words that aren't names.
pub fn is_plausible_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() < 30
        && name.chars().next().is_some_and(|c| c.is_alphabetic())
        && name.chars().all(|c| c.is_alphabetic() || c == '\'' || c == '-')
}

impl WorkingMemory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_event(&mut self, event: Event) {
        self.events.push_back(event);
        if self.events.len() > MAX_EVENTS {
            self.events.pop_front();
        }
    }

    pub fn set_intent(&mut self, intent: String) {
        self.current_intent = Some(intent);
    }

    pub fn clear_intent(&mut self) {
        self.current_intent = None;
    }

    /// Stores the name only if it could plausibly be one — see
    /// `is_plausible_name`. Returns whether it was stored.
    pub fn set_user_name(&mut self, name: String) -> bool {
        if !is_plausible_name(&name) {
            return false;
        }
        self.user_name = Some(name);
        self.user_name_verified = true;
        true
    }

    /// Drop a restored name that wasn't learned by the current rules.
    /// A bad name is persisted in working_memory.json, so without this a
    /// false match from an older build ("The user's name is Using.") comes
    /// back on every run.
    pub fn sanitize(&mut self) -> Option<String> {
        if self.user_name.is_some() && !self.user_name_verified {
            return self.user_name.take();
        }
        None
    }

    pub fn set_active_plan(&mut self, plan: serde_json::Value) {
        self.active_plan = Some(plan);
    }

    pub fn clear_active_plan(&mut self) {
        self.active_plan = None;
    }

    pub fn recent_events(&self, n: usize) -> Vec<&Event> {
        let skip = self.events.len().saturating_sub(n);
        self.events.iter().skip(skip).collect()
    }

    pub fn recent_context(&self, n: usize) -> Vec<Event> {
        let skip = self.events.len().saturating_sub(n);
        self.events.iter().skip(skip).cloned().collect()
    }

    pub fn is_busy(&self) -> bool {
        self.active_plan.is_some()
    }

    // -----------------------------------------------------------------
    // Phase 5 — session state recovery
    // -----------------------------------------------------------------
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}