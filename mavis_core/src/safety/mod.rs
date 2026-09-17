// mavis_core/src/safety/mod.rs
// Permission gate: sits between the planner and the executor.
//
// The planner publishes PlanReady; this subscribes, scores the plan, records
// it, and republishes as PlanApproved — which is what the executor now
// listens for. Nothing reaches the executor without passing through here.

pub mod audit;
pub mod risk;

use crate::event_bus::EventBus;
use crate::models::event::{Event, EventType};
use audit::AuditLog;
use log::{info, warn};
use risk::{assess_plan, Verdict};
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct PermissionGate {
    bus: Arc<EventBus>,
    audit: Arc<Mutex<AuditLog>>,
}

impl PermissionGate {
    pub fn new(bus: Arc<EventBus>, audit: Arc<Mutex<AuditLog>>) -> Self {
        Self { bus, audit }
    }

    pub async fn run(&mut self) {
        let mut rx = self.bus.subscribe();
        info!("PermissionGate: listening for plans");
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if event.event_type == EventType::PlanReady {
                        self.review(event).await;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    warn!("PermissionGate lagged by {} events", n);
                }
            }
        }
        info!("PermissionGate: shutting down");
    }

    async fn review(&self, event: Event) {
        let plan = match event.payload.get("plan") {
            Some(p) => p.clone(),
            None => return,
        };

        let assessment = assess_plan(&plan);
        let detail = summarise(&plan);
        let action_type = plan
            .as_array()
            .and_then(|a| a.first())
            .or(Some(&plan))
            .and_then(|a| a.get("type"))
            .and_then(|t| t.as_str())
            .unwrap_or("unknown")
            .to_string();

        let outcome = match &assessment.verdict {
            Verdict::Allow => "allowed",
            // Confirmation flow isn't built yet, so anything needing it is
            // refused rather than run unreviewed. Deliberately the safe
            // default: shell actions aren't reachable from voice today, so
            // in practice nothing legitimate is blocked by this.
            Verdict::Confirm => "blocked_pending_confirmation",
            Verdict::Deny(_) => "denied",
        };

        {
            let log = self.audit.lock().await;
            if let Err(e) = log.record(
                &action_type,
                &detail,
                assessment.score,
                outcome,
                &assessment.reason,
            ) {
                warn!("PermissionGate: failed to write audit entry: {}", e);
            }
        }

        match assessment.verdict {
            Verdict::Allow => {
                self.bus.publish(Event {
                    id: uuid::Uuid::new_v4(),
                    timestamp: chrono::Utc::now(),
                    source: "permission_gate".to_string(),
                    event_type: EventType::PlanApproved,
                    payload: serde_json::json!({ "plan": plan }),
                });
            }
            Verdict::Confirm => {
                info!(
                    "PermissionGate: holding action (risk {}, {})",
                    assessment.score, assessment.reason
                );
                self.speak(format!(
                    "That needs confirmation — {}. I can't do that yet.",
                    assessment.reason
                ));
            }
            Verdict::Deny(why) => {
                warn!("PermissionGate: denied — {}", why);
                self.speak("I won't do that. It's not reversible.".to_string());
            }
        }
    }

    /// Refusals still reach the user as speech, via an already-approved plan
    /// so they don't loop back through this gate.
    fn speak(&self, text: String) {
        self.bus.publish(Event {
            id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            source: "permission_gate".to_string(),
            event_type: EventType::PlanApproved,
            payload: serde_json::json!({ "plan": {"type": "say", "text": text} }),
        });
    }
}

/// One-line description of a plan, for the audit log.
fn summarise(plan: &serde_json::Value) -> String {
    let actions: Vec<serde_json::Value> = match plan.as_array() {
        Some(a) => a.clone(),
        None => vec![plan.clone()],
    };
    actions
        .iter()
        .map(|a| {
            let kind = a.get("type").and_then(|v| v.as_str()).unwrap_or("?");
            let what = a
                .get("command")
                .or_else(|| a.get("target"))
                .or_else(|| a.get("op"))
                .or_else(|| a.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{}:{}", kind, &what[..what.len().min(120)])
        })
        .collect::<Vec<_>>()
        .join(" | ")
}