// mavis_core/src/event_bus.rs
// Central pub/sub event bus. All subsystems communicate here.
//
// This is the one object every subsystem touches, so it is also the one
// object that must never panic. It previously did, two ways:
//
//   1. `self.sender.lock().unwrap()` — a `std::sync::Mutex` is poisoned if
//      any thread panics while holding it. One panic anywhere (including
//      in the cpal audio callback thread, which publishes from outside the
//      tokio runtime) would make *every subsequent publish in the process*
//      panic, taking the whole system down in a cascade.
//
//   2. `subscribe()` used `.expect("EventBus already closed")`, so anything
//      subscribing after shutdown — or a subsystem restarting during
//      shutdown — panicked instead of shutting down normally.
//
// The lock is held only long enough to clone/send, and poison is recovered
// rather than propagated: a poisoned lock here means some other thread had
// a bug, not that the sender is unusable.

use crate::models::event::Event;
use std::sync::{Mutex, MutexGuard};
use tokio::sync::broadcast;

pub struct EventBus {
    sender: Mutex<Option<broadcast::Sender<Event>>>,
}

impl EventBus {
    pub fn new() -> Self {
        let (sender, _receiver) = broadcast::channel(256);
        Self {
            sender: Mutex::new(Some(sender)),
        }
    }

    /// Take the lock, recovering from poison.
    ///
    /// Poison means another thread panicked while holding this lock. The
    /// data behind it is a plain `Option<Sender>` that is only ever read or
    /// replaced wholesale, so there is no torn state to protect against —
    /// refusing to hand it back would turn one subsystem's bug into a
    /// total outage.
    fn guard(&self) -> MutexGuard<'_, Option<broadcast::Sender<Event>>> {
        match self.sender.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    pub fn publish(&self, event: Event) {
        if let Some(sender) = self.guard().as_ref() {
            // An Err here just means nobody is currently subscribed.
            let _ = sender.send(event);
        }
    }

    /// Subscribe to the bus.
    ///
    /// Never panics. If the bus is already closed the caller gets a
    /// receiver that is immediately closed, so it observes
    /// `RecvError::Closed` and takes its normal shutdown path.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        match self.guard().as_ref() {
            Some(sender) => sender.subscribe(),
            None => {
                let (tx, rx) = broadcast::channel(1);
                drop(tx);
                rx
            }
        }
    }

    /// Whether the bus is still open. Used by supervision to tell an
    /// intentional shutdown apart from a subsystem dying on its own.
    pub fn is_open(&self) -> bool {
        self.guard().is_some()
    }

    /// Close the bus. All existing receivers will get `RecvError::Closed`.
    pub fn close(&self) {
        *self.guard() = None;
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

    fn test_event() -> Event {
        Event {
            id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            source: "test".to_string(),
            event_type: EventType::SystemWake,
            payload: serde_json::json!({}),
        }
    }

    #[test]
    fn test_event_bus_publish_subscribe() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        bus.publish(test_event());
        let received = rx.try_recv().expect("should receive event");
        assert_eq!(received.source, "test");
    }

    #[test]
    fn test_event_bus_close() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();
        bus.close();
        assert!(rx.try_recv().is_err());
    }

    /// Subscribing after close used to panic; it must now yield a closed
    /// receiver so the subsystem shuts down through its normal path.
    #[test]
    fn subscribe_after_close_does_not_panic() {
        let bus = EventBus::new();
        bus.close();
        let mut rx = bus.subscribe();
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Closed)
        ));
    }

    #[test]
    fn publish_after_close_is_a_no_op() {
        let bus = EventBus::new();
        bus.close();
        bus.publish(test_event()); // must not panic
        assert!(!bus.is_open());
    }

    /// A panic in another thread must not disable the bus for everyone else.
    #[test]
    fn survives_a_poisoned_lock() {
        use std::sync::Arc;
        let bus = Arc::new(EventBus::new());

        let bus_panic = Arc::clone(&bus);
        let _ = std::thread::spawn(move || {
            let _guard = bus_panic.guard();
            panic!("simulated subsystem panic while holding the lock");
        })
        .join();

        // Previously: every call below panicked with PoisonError.
        let mut rx = bus.subscribe();
        bus.publish(test_event());
        assert_eq!(rx.try_recv().expect("bus still usable").source, "test");
        assert!(bus.is_open());
    }
}