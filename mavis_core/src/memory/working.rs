// Working Memory: session-scoped context window.
// Holds recent events, current intent, active plan, and UI state.

use crate::models::event::{Event, EventType};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

const MAX_EVENTS: usize = 50;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WorkingMemory {
    pub events: VecDeque<Event>,
    pub current_intent: Option<String>,
    pub active_plan: Option<serde_json::Value>,
    pub ui_state: Option<String>,
}

impl WorkingMemory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push an event into the window, dropping the oldest if over capacity.
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

    pub fn set_active_plan(&mut self, plan: serde_json::Value) {
        self.active_plan = Some(plan);
    }

    pub fn clear_active_plan(&mut self) {
        self.active_plan = None;
    }

    /// Borrow the most recent n events in chronological order.
    pub fn recent_events(&self, n: usize) -> Vec<&Event> {
        let skip = self.events.len().saturating_sub(n);
        self.events.iter().skip(skip).collect()
    }

    /// Clone the most recent n events in chronological order.
    pub fn recent_context(&self, n: usize) -> Vec<Event> {
        let skip = self.events.len().saturating_sub(n);
        self.events.iter().skip(skip).cloned().collect()
    }

    pub fn is_busy(&self) -> bool {
        self.active_plan.is_some()
    }
}