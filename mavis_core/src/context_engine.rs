// Central nervous system. Owns Working Memory and routes all events.

use crate::event_bus::EventBus;
use crate::memory::manager::MemoryManager;
use crate::models::event::{Event, EventType};
use anyhow::Result;
use log::{info, warn};
use std::sync::Arc;

pub struct ContextEngine {
    memory: MemoryManager,
    bus: Arc<EventBus>,
}

impl ContextEngine {
    pub fn new(bus: Arc<EventBus>) -> Self {
        Self {
            memory: MemoryManager::new(),
            bus,
        }
    }

    pub async fn process_event(&mut self, event: Event) -> Result<()> {
        // Every event lands in working memory first.
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
                info!("ContextEngine: UserIntent received — routing to AI worker");
                let intent = event
                    .payload
                    .get("intent")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");

                {
                    let mut wm = self.memory.working.write().await;
                    wm.set_intent(intent.to_string());
                }

                // Gather context for the worker
                let context = {
                    let wm = self.memory.working.read().await;
                    serde_json::json!({
                        "recent_events": wm.recent_context(10),
                        "current_intent": wm.current_intent,
                        "ui_state": wm.ui_state,
                    })
                };

                let worker_req = Event {
                    id: uuid::Uuid::new_v4(),
                    timestamp: chrono::Utc::now(),
                    source: "context_engine".to_string(),
                    event_type: EventType::WorkerRequest,
                    payload: serde_json::json!({
                        "type": "intent_analysis",
                        "intent": intent,
                        "context": context,
                    }),
                };
                self.bus.publish(worker_req);
            }

            EventType::WorkerResponse => {
                info!("ContextEngine: WorkerResponse received — routing");
                self.route_worker_response(event).await?;
            }

            EventType::ContextUpdate => {
                info!("ContextEngine: ContextUpdate merged into working memory");
                // Already recorded above; deeper semantic merging goes here later.
            }

            EventType::PlanReady => {
                info!("ContextEngine: PlanReady — forwarding to executor");
                if let Some(plan) = event.payload.get("plan") {
                    let mut wm = self.memory.working.write().await;
                    wm.set_active_plan(plan.clone());
                }
                // Re-publish so Executor (and UI) can react.
                self.bus.publish(event);
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
                // Another subsystem published a direct worker request; observe only.
                info!("ContextEngine: observed WorkerRequest from {}", event.source);
            }
        }

        Ok(())
    }

    /// Parse a WorkerResponse and emit the appropriate downstream event.
    async fn route_worker_response(&self, event: Event) -> Result<()> {
        let payload = &event.payload;
        let response_type = payload.get("type").and_then(|v| v.as_str());

        match response_type {
            Some("plan") => {
                let plan_event = Event {
                    id: uuid::Uuid::new_v4(),
                    timestamp: chrono::Utc::now(),
                    source: "context_engine".to_string(),
                    event_type: EventType::PlanReady,
                    payload: serde_json::json!({
                        "plan": payload.get("plan"),
                        "metadata": payload.get("metadata"),
                    }),
                };
                self.bus.publish(plan_event);
            }
            Some("context") => {
                let ctx_event = Event {
                    id: uuid::Uuid::new_v4(),
                    timestamp: chrono::Utc::now(),
                    source: "context_engine".to_string(),
                    event_type: EventType::ContextUpdate,
                    payload: payload.get("data").cloned().unwrap_or_else(|| payload.clone()),
                };
                self.bus.publish(ctx_event);
            }
            Some(other) => {
                warn!("ContextEngine: unknown worker response type '{}'", other);
            }
            None => {
                warn!("ContextEngine: WorkerResponse missing 'type' field");
            }
        }

        Ok(())
    }

    /// Diagnostics / UI snapshot accessor.
    pub async fn working_memory_snapshot(&self) -> crate::memory::working::WorkingMemory {
        self.memory.working.read().await.clone()
    }
}