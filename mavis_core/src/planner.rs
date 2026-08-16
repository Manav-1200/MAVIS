use crate::event_bus::EventBus;
use crate::memory::working::WorkingMemory;
use crate::models::event::{Event, EventType};
use log::{info, warn};
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct Planner {
    bus: Arc<EventBus>,
    working: Arc<RwLock<WorkingMemory>>,
}

impl Planner {
    pub fn new(bus: Arc<EventBus>, working: Arc<RwLock<WorkingMemory>>) -> Self {
        Self { bus, working }
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

        let working_memory = self.build_working_memory().await;

        let worker_req = Event {
            id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            source: "planner".to_string(),
            event_type: EventType::WorkerRequest,
            payload: serde_json::json!({
                "request_type": "chat",
                "messages": [
                    {"role": "system", "content": "You are MAVIS, a helpful desktop AI companion. Keep responses concise and actionable."},
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

    async fn build_working_memory(&self) -> Vec<serde_json::Value> {
        let snapshot = self.working.read().await;
        let mut items = Vec::new();

        if let Some(intent) = &snapshot.current_intent {
            items.push(serde_json::json!({
                "source": "current_intent",
                "content": intent,
            }));
        }

        for event in snapshot.recent_events(5) {
            let (source, content) = match event.event_type {
                EventType::UserIntent => (
                    "user",
                    event.payload.get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string()
                ),
                EventType::WorkerResponse => (
                    "mavis",
                    event.payload.get("result")
                        .and_then(|r| r.get("content"))
                        .and_then(|c| c.as_str())
                        .unwrap_or("")
                        .to_string()
                ),
                EventType::PlanReady => (
                    "plan",
                    event.payload.get("plan")
                        .and_then(|p| p.get("text"))
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string()
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