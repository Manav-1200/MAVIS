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
    pub fn new(bus: Arc<EventBus>, data_dir: &std::path::Path) -> Result<Self> {
        Ok(Self {
            memory: MemoryManager::new(data_dir)?,
            bus,
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
                // STT pipeline sends transcribed text in "text"; other sources use "intent".
                let intent = event
                    .payload
                    .get("text")
                    .and_then(|v| v.as_str())
                    .or_else(|| event.payload.get("intent").and_then(|v| v.as_str()))
                    .unwrap_or("unknown");

                {
                    let mut wm = self.memory.working.write().await;
                    wm.set_intent(intent.to_string());
                }
                // Planner will pick this up and generate a PlanReady event.
            }

            EventType::WorkerResponse => {
                info!("ContextEngine: WorkerResponse received — routing");
                self.route_worker_response(event).await?;
            }

            EventType::ContextUpdate => {
                info!("ContextEngine: ContextUpdate merged into working memory");
            }

            EventType::PlanReady => {
                info!("ContextEngine: PlanReady — updating working memory");
                if let Some(plan) = event.payload.get("plan") {
                    let mut wm = self.memory.working.write().await;
                    wm.set_active_plan(plan.clone());
                }
                // Do NOT re-publish; Executor listens directly on the bus.
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
        }

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
                // Planner handles "plan" type responses.
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
}