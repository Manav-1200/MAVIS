// mavis_core/src/event_bus.rs
// Central pub/sub event bus. All subsystems communicate here.

use crate::models::event::Event;
use tokio::sync::broadcast;

pub struct EventBus {
    sender: broadcast::Sender<Event>,
}

impl EventBus {
    pub fn new() -> Self {
        let (sender, _receiver) = broadcast::channel(256);
        Self { sender }
    }

    pub fn publish(&self, event: Event) {
        let _ = self.sender.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.sender.subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::event::{Event, EventType};

    #[test]
    fn test_event_bus_publish_subscribe() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        let event = Event {
            id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            source: "test".to_string(),
            event_type: EventType::SystemWake,
            payload: serde_json::json!({}),
        };

        bus.publish(event);
        let received = rx.try_recv().expect("should receive event");
        assert_eq!(received.source, "test");
    }
}