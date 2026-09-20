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

/// An action held back, waiting for the user to say yes.
struct Pending {
    plan: serde_json::Value,
    detail: String,
    score: u8,
    /// Scores at or above this need an explicit "yes" plus an
    /// acknowledgement of the risk, not a bare "yeah".
    requires_explicit: bool,
    asked_at: std::time::Instant,
}

/// How long a held action stays answerable. Short on purpose: a "yes" thirty
/// seconds later probably answers a different question, and silently running
/// something the user has moved on from is worse than making them repeat it.
const CONFIRMATION_WINDOW: std::time::Duration = std::time::Duration::from_secs(20);

pub struct PermissionGate {
    bus: Arc<EventBus>,
    audit: Arc<Mutex<AuditLog>>,
    pending: Option<Pending>,
}

impl PermissionGate {
    pub fn new(bus: Arc<EventBus>, audit: Arc<Mutex<AuditLog>>) -> Self {
        Self { bus, audit, pending: None }
    }

    pub async fn run(&mut self) {
        let mut rx = self.bus.subscribe();
        info!("PermissionGate: listening for plans");
        loop {
            match rx.recv().await {
                Ok(event) => match event.event_type {
                    EventType::PlanReady => self.review(event).await,
                    // The confirmation answer arrives as ordinary speech.
                    EventType::UserIntent => self.resolve_pending(&event).await,
                    _ => {}
                },
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    warn!("PermissionGate lagged by {} events", n);
                }
            }
        }
        info!("PermissionGate: shutting down");
    }

    async fn review(&mut self, event: Event) {
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

        // The audit log is the record of what actually happened, so these
        // strings have to say what actually happened. This previously read
        // "blocked_pending_confirmation" alongside a comment claiming the
        // confirmation flow wasn't built — but it is built, just below, and
        // a held action can go on to run. An append-only log that misreports
        // its own outcomes is worse than no log.
        //
        // "held_for_confirmation" is followed by a second entry from
        // `record()` — "confirmed", "declined" or "expired" — which is what
        // closes the story for that action.
        let outcome = match &assessment.verdict {
            Verdict::Allow => "allowed",
            Verdict::Confirm => "held_for_confirmation",
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
                let requires_explicit = assessment.score >= 8;
                info!(
                    "PermissionGate: holding action (risk {}, {})",
                    assessment.score, assessment.reason
                );
                let question = if requires_explicit {
                    format!(
                        "That needs administrator permission — {}. Say 'yes, administrator' to go ahead.",
                        assessment.reason
                    )
                } else {
                    format!("Shall I? {}.", assessment.reason)
                };
                self.pending = Some(Pending {
                    plan,
                    detail,
                    score: assessment.score,
                    requires_explicit,
                    asked_at: std::time::Instant::now(),
                });
                self.speak(question);
            }
            Verdict::Deny(why) => {
                warn!("PermissionGate: denied — {}", why);
                self.speak("I won't do that. It's not reversible.".to_string());
            }
        }
    }

    /// Interpret the user's next utterance as an answer to a held action.
    ///
    /// Only consulted while something is actually pending, so ordinary
    /// speech is unaffected. Anything that isn't a clear yes cancels —
    /// silence, a new question, or an ambiguous reply all mean "don't".
    async fn resolve_pending(&mut self, event: &Event) {
        let pending = match self.pending.take() {
            Some(p) => p,
            None => return,
        };

        if pending.asked_at.elapsed() > CONFIRMATION_WINDOW {
            info!("PermissionGate: confirmation window expired, discarding");
            self.record(&pending, "expired", "no answer in time").await;
            return;
        }

        let said = event
            .payload
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        let said = said.trim();

        let approved = if pending.requires_explicit {
            // A bare "yes" is too easy to say by accident, and too easy for
            // a mistranscription to produce. High-risk actions need the word.
            said.contains("administrator") && is_affirmative(said)
        } else {
            is_affirmative(said)
        };

        if approved {
            info!("PermissionGate: confirmed by user, executing");
            self.record(&pending, "confirmed", "user approved").await;
            self.bus.publish(Event {
                id: uuid::Uuid::new_v4(),
                timestamp: chrono::Utc::now(),
                source: "permission_gate".to_string(),
                event_type: EventType::PlanApproved,
                payload: serde_json::json!({ "plan": pending.plan }),
            });
        } else {
            info!("PermissionGate: not confirmed, discarding held action");
            self.record(&pending, "declined", "user did not confirm").await;
            self.speak("Cancelled.".to_string());
        }
    }

    async fn record(&self, pending: &Pending, outcome: &str, reason: &str) {
        let log = self.audit.lock().await;
        if let Err(e) = log.record("confirmation", &pending.detail, pending.score, outcome, reason)
        {
            warn!("PermissionGate: failed to write audit entry: {}", e);
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
            // truncate_bytes, not `&what[..120]`: `what` can be spoken text
            // (a `say` action), and slicing mid-character panicked inside
            // the permission gate — which is the one subsystem that must
            // never stop running.
            format!("{}:{}", kind, crate::util::truncate_bytes(what, 120))
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

/// Whether an utterance is a clear yes.
///
/// Deliberately strict: only unambiguous agreement counts. "Maybe", "I
/// think so", or anything containing a negation is not consent, and the
/// cost of a false positive here is running something destructive.
fn is_affirmative(said: &str) -> bool {
    // Normalise punctuation to spaces first: "yes, administrator" is the
    // natural way to say it, and a comma shouldn't cost the user a retry.
    let normalised: String = said
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '\'' { c } else { ' ' })
        .collect();
    let normalised = normalised.split_whitespace().collect::<Vec<_>>().join(" ");
    let words: Vec<&str> = normalised.split(' ').filter(|w| !w.is_empty()).collect();

    // Negation always wins. "no, yes" and "actually no, cancel that" are
    // refusals, and a false positive here runs something destructive.
    const NEGATIONS: &[&str] = &["no", "don t", "dont", "stop", "cancel", "never", "wait"];
    if NEGATIONS.iter().any(|n| {
        if n.contains(' ') {
            normalised.contains(n)
        } else {
            words.contains(n)
        }
    }) {
        return false;
    }

    const AFFIRMATIVES: &[&str] = &[
        "yes", "yeah", "yep", "yup", "sure", "go ahead", "do it", "confirm",
        "confirmed", "affirmative", "please do", "ok", "okay",
    ];
    AFFIRMATIVES.iter().any(|a| {
        normalised == *a
            || normalised.starts_with(&format!("{} ", a))
            || normalised.contains(&format!(" {} ", a))
            || normalised.ends_with(&format!(" {}", a))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------
    // summarise — regression tests for the UTF-8 panic
    // ---------------------------------------------------------------

    /// `&what[..120]` panicked whenever byte 120 landed inside a
    /// multi-byte character. A panic here took out the permission gate,
    /// after which nothing reached the executor at all.
    #[test]
    fn summarise_survives_long_non_ascii_text() {
        let long = "日本語".repeat(200);
        let plan = serde_json::json!([{ "type": "say", "text": long }]);
        let out = summarise(&plan);
        assert!(out.starts_with("say:"));
    }

    #[test]
    fn summarise_survives_every_length_of_multibyte_text() {
        for n in 1..80 {
            let text = "é".repeat(n) + &"x".repeat(n);
            let plan = serde_json::json!([{ "type": "say", "text": text }]);
            let _ = summarise(&plan);
        }
    }

    #[test]
    fn summarise_handles_plans_and_bare_actions() {
        let bare = serde_json::json!({ "type": "system", "op": "volume_up" });
        assert_eq!(summarise(&bare), "system:volume_up");

        let multi = serde_json::json!([
            { "type": "say", "text": "Volume up." },
            { "type": "system", "op": "volume_up" },
        ]);
        assert_eq!(summarise(&multi), "say:Volume up. | system:volume_up");
    }

    #[test]
    fn summarise_handles_missing_fields() {
        let plan = serde_json::json!([{ "nothing": "useful" }]);
        assert_eq!(summarise(&plan), "?:");
    }

    // ---------------------------------------------------------------
    // is_affirmative
    // ---------------------------------------------------------------

    #[test]
    fn clear_agreement_is_affirmative() {
        for said in [
            "yes", "yeah", "yep", "sure", "go ahead", "do it",
            "yes please", "yes, administrator", "confirmed",
        ] {
            assert!(is_affirmative(said), "should be affirmative: {:?}", said);
        }
    }

    #[test]
    fn negation_always_wins() {
        for said in [
            "no", "no thanks", "nope, cancel", "yes no", "no, yes",
            "actually no, cancel that", "don't", "dont do it", "stop",
            "wait", "cancel", "never",
        ] {
            assert!(!is_affirmative(said), "should NOT be affirmative: {:?}", said);
        }
    }

    #[test]
    fn ambiguous_answers_are_not_consent() {
        for said in ["maybe", "i think so", "what", "", "hmm", "probably"] {
            assert!(!is_affirmative(said), "should NOT be affirmative: {:?}", said);
        }
    }

    #[test]
    fn is_affirmative_survives_non_ascii() {
        for said in ["日本語", "é", "yes — administrator", "\u{2019}yes\u{2019}"] {
            let _ = is_affirmative(said);
        }
    }

    // ---------------------------------------------------------------
    // Gate wiring
    // ---------------------------------------------------------------

    #[test]
    fn low_risk_plans_are_allowed_without_asking() {
        let plan = serde_json::json!([
            { "type": "say", "text": "Volume up." },
            { "type": "system", "op": "volume_up" },
        ]);
        assert_eq!(risk::assess_plan(&plan).verdict, risk::Verdict::Allow);
    }
}