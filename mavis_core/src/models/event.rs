// mavis_core/src/models/event.rs
// The canonical Event type. Everything that happens in MAVIS is an Event.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub source: String,
    pub event_type: EventType,
    pub payload: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum EventType {
    SystemWake,
    UserIntent,
    ContextUpdate,
    PlanReady,
    ActionComplete,
    WorkerRequest,
    WorkerResponse,
    UiStateChange,
    SystemAction,
    /// Signal to kill current TTS playback and drain the queue.
    /// Emitted by the intent router when the user speaks during TTS.
    TtsInterrupt,
    /// A plan that has passed the permission gate. The executor listens
    /// for this rather than PlanReady, so nothing runs unreviewed.
    PlanApproved,
    /// Browser tab/URL update from the extension's native messaging host.
    /// Deliberately separate from ContextUpdate: that one does a full
    /// replace of active_window/clipboard, and reusing it here would wipe
    /// those out on every tab switch since browser payloads don't carry them.
    BrowserUpdate,
    /// Something changed on the machine itself — packages installed or
    /// removed, and later privilege surfaces. Published by the Sentinel
    /// (Phase 8.5). Carries the changes and their peak severity so a
    /// listener can decide how loudly to surface them.
    SystemChange,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_serialization() {
        let event = Event {
            id: Uuid::new_v4(),
            timestamp: Utc::now(),
            source: "test".to_string(),
            event_type: EventType::SystemWake,
            payload: serde_json::json!({"key": "value"}),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("SystemWake"));
    }

    #[test]
    fn test_tts_interrupt_variant() {
        let event = Event {
            id: Uuid::new_v4(),
            timestamp: Utc::now(),
            source: "test".to_string(),
            event_type: EventType::TtsInterrupt,
            payload: serde_json::json!({}),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("TtsInterrupt"));
    }
}
