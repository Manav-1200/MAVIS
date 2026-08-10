//! Bridges WorkerRequest events to the Python worker over UDS.
//! Protocol: length-prefixed JSON, both directions.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use log::{error, info, warn};
use tokio::sync::broadcast;
use tokio::time::interval;

use crate::bridge::worker_lifecycle::WorkerLifecycle;
use crate::event_bus::EventBus;
use crate::models::event::{Event, EventType};

const HEALTH_INTERVAL: Duration = Duration::from_secs(30);
const IDLE_TIMEOUT: Duration = Duration::from_secs(300); // 5 min

pub struct WorkerBridge {
    lifecycle: WorkerLifecycle,
}

impl WorkerBridge {
    pub fn new() -> Self {
        Self {
            lifecycle: WorkerLifecycle::new("python3", "mavis.worker"),
        }
    }

    pub async fn run(mut self, bus: Arc<EventBus>) -> Result<()> {
        info!("WorkerBridge starting");
        let mut rx = bus.subscribe();
        let mut health_tick = interval(HEALTH_INTERVAL);

        loop {
            tokio::select! {
                bus_result = rx.recv() => {
                    match bus_result {
                        Ok(event) => {
                            match &event.event_type {
                                EventType::WorkerRequest => {
                                    if let Err(e) = self.handle_event(&event, &bus).await {
                                        error!("Worker request failed: {}", e);
                                        let resp = Event {
                                            id: uuid::Uuid::new_v4(),
                                            timestamp: chrono::Utc::now(),
                                            source: "worker_bridge".to_string(),
                                            event_type: EventType::WorkerResponse,
                                            payload: serde_json::json!({
                                                "request_id": event.id.to_string(),
                                                "error": e.to_string(),
                                                "type": "error"
                                            }),
                                        };
                                        let _ = bus.publish(resp);
                                    }
                                }
                                _ => {}
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!("WorkerBridge lagged by {} events", n);
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            info!("WorkerBridge: event bus closed");
                            break;
                        }
                    }
                }
                _ = health_tick.tick() => {
                    if self.lifecycle.is_running() {
                        if !self.lifecycle.health_check().await {
                            warn!("Worker unhealthy; recording crash");
                            self.lifecycle.record_crash();
                        } else if self.lifecycle.is_idle(IDLE_TIMEOUT) {
                            info!("Worker idle — requesting model unload");
                            let _ = self.lifecycle
                                .send_request(r#"{"type":"unload"}"#)
                                .await;
                        }
                    }
                }
            }
        }

        info!("WorkerBridge shutting down");
        self.lifecycle.shutdown().await;
        Ok(())
    }

    async fn handle_event(&mut self, event: &Event, bus: &Arc<EventBus>) -> Result<()> {
        // Worker expects {"type":"WorkerRequest","payload":{...}} not a full Event
        let request_json = serde_json::json!({
            "type": "WorkerRequest",
            "payload": event.payload
        })
        .to_string();

        let response_str = self.lifecycle.send_request(&request_json).await?;

        let resp_event: Event = serde_json::from_str(&response_str).unwrap_or_else(|_| Event {
            id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            source: "worker_bridge".to_string(),
            event_type: EventType::WorkerResponse,
            payload: serde_json::json!({
                "request_id": event.id.to_string(),
                "raw": response_str,
            }),
        });

        let _ = bus.publish(resp_event);
        Ok(())
    }
}