// mavis_core/src/safety/mod.rs
// Permission gate: sits between the planner and the executor.
//
// The planner publishes PlanReady; this subscribes, scores the plan, records
// it, and republishes as PlanApproved — which is what the executor now
// listens for. Nothing reaches the executor without passing through here.

pub mod audit;
pub mod risk;
pub mod rollback;

use crate::event_bus::EventBus;
use crate::models::event::{Event, EventType};
use audit::AuditLog;
use log::{info, warn};
use risk::{assess_plan, Verdict};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// An action held back, waiting for the user to say yes.
struct Pending {
    plan: serde_json::Value,
    detail: String,
    score: u8,
    /// Can't be undone: asked twice, the second time saying so.
    requires_explicit: bool,
    /// The second question has been asked.
    asked_twice: bool,
    asked_at: Instant,
    /// What was asked, e.g. "close Webby".
    what: String,
}

/// How long a held action stays answerable. Short on purpose: a "yes" thirty
/// seconds later probably answers a different question, and silently running
/// something the user has moved on from is worse than making them repeat it.
pub const CONFIRMATION_WINDOW: Duration = Duration::from_secs(if cfg!(test) { 2 } else { 20 });

/// Whether a question is open, shared with the planner. The utterance
/// that answers "Shall I?" belongs to the gate; without this the planner
/// answered it too, as if "yes" were a question.
#[derive(Default)]
pub struct Awaiting {
    asked_at: Option<Instant>,
    /// The question as MAVIS asked it, for the model to see.
    question: Option<String>,
}

pub type SharedAwaiting = Arc<std::sync::Mutex<Awaiting>>;

impl Awaiting {
    /// A question is open and still answerable.
    pub fn is_open(&self) -> bool {
        self.asked_at.is_some_and(|at| at.elapsed() <= CONFIRMATION_WINDOW)
    }

    /// The open question, while it can still be answered.
    pub fn question(&self) -> Option<String> {
        if self.is_open() { self.question.clone() } else { None }
    }
}

pub struct PermissionGate {
    bus: Arc<EventBus>,
    audit: Arc<Mutex<AuditLog>>,
    pending: Option<Pending>,
    awaiting: SharedAwaiting,
}

impl PermissionGate {
    pub fn new(bus: Arc<EventBus>, audit: Arc<Mutex<AuditLog>>, awaiting: SharedAwaiting) -> Self {
        Self { bus, audit, pending: None, awaiting }
    }

    pub async fn run(&mut self) {
        let mut rx = self.bus.subscribe();
        info!("PermissionGate: listening for plans");
        loop {
            // A held action lapses on time, not whenever the user next speaks.
            let lapse = self.pending.as_ref().map(|p| p.asked_at + CONFIRMATION_WINDOW);
            let event = match lapse {
                Some(deadline) => {
                    match tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), rx.recv()).await {
                        Ok(event) => event,
                        Err(_) => {
                            self.expire().await;
                            continue;
                        }
                    }
                }
                None => rx.recv().await,
            };
            match event {
                Ok(event) => match event.event_type {
                    EventType::PlanReady => self.review(event).await,
                    // The answer, as the model understood what was said.
                    EventType::Confirmation => {
                        let approved = event.payload.get("approved").and_then(|v| v.as_bool()).unwrap_or(false);
                        self.answered(approved).await
                    }
                    EventType::OrbTap => self.tapped(&event).await,
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
        let action_type = risk::actions(&plan)
            .first()
            .and_then(|a| a.get("type"))
            .and_then(|t| t.as_str())
            .unwrap_or("unknown")
            .to_string();

        // The audit log is the record of what actually happened, so these
        // strings have to say what actually happened. "held_for_confirmation"
        // is followed by a second entry from `record()` — "confirmed",
        // "declined", "expired" or "superseded" — which closes the story
        // for that action.
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
                // Only one question can be open: a "yes" has to mean the
                // thing MAVIS asked last. The older one is closed on record.
                if let Some(old) = self.pending.take() {
                    self.record(&old, "superseded", "a newer action asked for confirmation").await;
                }
                let requires_explicit = assessment.score >= 8;
                info!(
                    "PermissionGate: holding action (risk {}, {})",
                    assessment.score, assessment.reason
                );
                // What would run, not only why it needs asking.
                let question = format!("whether you should {}, since it {}", assessment.what, assessment.reason);
                let asked_at = Instant::now();
                self.pending = Some(Pending {
                    plan,
                    detail,
                    score: assessment.score,
                    requires_explicit,
                    asked_twice: false,
                    asked_at,
                    what: assessment.what.clone(),
                });
                self.set_awaiting(Some(asked_at), Some(&question));
                self.show("asking");
                self.tell(format!("Nothing has run yet. Ask the user {}.", question));
            }
            Verdict::Deny(why) => {
                warn!("PermissionGate: denied — {}", why);
                self.tell(format!("You won't do that, and nothing ran: {}.", why));
            }
        }
    }

    /// Nobody answered in time.
    async fn expire(&mut self) {
        if let Some(pending) = self.pending.take() {
            info!("PermissionGate: confirmation window expired, discarding");
            self.record(&pending, "expired", "no answer in time").await;
            self.set_awaiting(None, None);
            self.show("answered");
        }
    }

    /// The user answered the open question; the model judged yes or no.
    /// High-risk actions need two yeses, the second after being told it
    /// can't be undone.
    async fn answered(&mut self, approved: bool) {
        let Some(mut pending) = self.pending.take() else { return };
        if pending.asked_at.elapsed() > CONFIRMATION_WINDOW {
            self.pending = Some(pending);
            self.expire().await;
            return;
        }
        if !approved {
            info!("PermissionGate: not confirmed, discarding held action");
            self.set_awaiting(None, None);
            self.show("answered");
            self.record(&pending, "declined", "user did not confirm").await;
            self.tell("They said no, so it was cancelled and nothing ran.".to_string());
            return;
        }
        if pending.requires_explicit && !pending.asked_twice {
            let question = format!("whether they're sure you should {}, since it can't be undone", pending.what);
            pending.asked_twice = true;
            pending.asked_at = Instant::now();
            self.set_awaiting(Some(pending.asked_at), Some(&question));
            self.pending = Some(pending);
            self.tell(format!("Nothing has run yet. Ask the user once more {}.", question));
            return;
        }
        info!("PermissionGate: confirmed by user, executing");
        self.set_awaiting(None, None);
        self.show("answered");
        self.record(&pending, "confirmed", "user approved").await;
        self.bus.publish(Event {
            id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            source: "permission_gate".to_string(),
            event_type: EventType::PlanApproved,
            payload: serde_json::json!({ "plan": pending.plan }),
        });
    }

    /// Hold a plan as `review` would. For tests in other modules.
    #[cfg(test)]
    pub async fn ask_for_test(&mut self, plan: serde_json::Value) {
        let event = Event {
            id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            source: "test".into(),
            event_type: EventType::PlanReady,
            payload: serde_json::json!({ "plan": plan }),
        };
        self.review(event).await;
    }

    #[cfg(test)]
    pub async fn answer_for_test(&mut self, approved: bool) {
        self.answered(approved).await;
    }

    /// A tap on the orb is a yes, except for what can't be undone: a stray
    /// click must not run that.
    async fn tapped(&mut self, _tap: &Event) {
        match &self.pending {
            Some(p) if p.requires_explicit => self.tell("A tap isn't enough for this one. Ask them to answer out loud.".to_string()),
            Some(_) => {
                info!("PermissionGate: answered by a tap on the orb");
                self.answered(true).await;
            }
            None => {}
        }
    }

    async fn record(&self, pending: &Pending, outcome: &str, reason: &str) {
        let log = self.audit.lock().await;
        if let Err(e) = log.record("confirmation", &pending.detail, pending.score, outcome, reason)
        {
            warn!("PermissionGate: failed to write audit entry: {}", e);
        }
    }

    fn set_awaiting(&self, asked_at: Option<Instant>, question: Option<&str>) {
        let mut awaiting = self.awaiting.lock().unwrap_or_else(|p| p.into_inner());
        awaiting.asked_at = asked_at;
        awaiting.question = question.map(String::from);
    }

    /// Tell the orb a question is open ("asking") or closed ("answered").
    fn show(&self, state: &str) {
        self.bus.publish(Event {
            id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            source: "permission_gate".to_string(),
            event_type: EventType::UiStateChange,
            payload: serde_json::json!({ "state": state }),
        });
    }

    /// Questions and refusals are told by the model, in its own words.
    fn tell(&self, fact: String) {
        self.bus.publish(Event {
            id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            source: "permission_gate".to_string(),
            event_type: EventType::Tell,
            payload: serde_json::json!({ "fact": fact }),
        });
    }
}

/// One-line description of a plan, for the audit log.
fn summarise(plan: &serde_json::Value) -> String {
    risk::actions(plan)
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
    // Gate wiring
    // ---------------------------------------------------------------

    struct Rig {
        gate: PermissionGate,
        rx: tokio::sync::broadcast::Receiver<Event>,
        audit: Arc<Mutex<AuditLog>>,
        awaiting: SharedAwaiting,
    }

    fn rig(tag: &str) -> Rig {
        let path = std::env::temp_dir().join(format!(
            "mavis_gate_{}_{}_{}.db",
            tag,
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let bus = Arc::new(EventBus::new());
        let rx = bus.subscribe();
        let audit = Arc::new(Mutex::new(AuditLog::new(&path).unwrap()));
        let awaiting = SharedAwaiting::default();
        let gate = PermissionGate::new(bus, audit.clone(), awaiting.clone());
        Rig { gate, rx, audit, awaiting }
    }

    fn event(event_type: EventType, payload: serde_json::Value) -> Event {
        Event {
            id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            source: "test".into(),
            event_type,
            payload,
        }
    }

    fn plan_ready(command: &str) -> Event {
        event(EventType::PlanReady, serde_json::json!({ "plan": [{ "type": "shell", "command": command }] }))
    }

    fn answer(approved: bool) -> Event {
        event(EventType::Confirmation, serde_json::json!({ "approved": approved }))
    }

    /// Everything the gate published since the last call: (facts handed to
    /// the model to tell, shell commands approved to run, orb states).
    fn published(rx: &mut tokio::sync::broadcast::Receiver<Event>) -> (Vec<String>, Vec<String>, Vec<String>) {
        let (mut spoken, mut approved, mut shown) = (Vec::new(), Vec::new(), Vec::new());
        while let Ok(e) = rx.try_recv() {
            match e.event_type {
                EventType::PlanApproved => {
                    for action in risk::actions(&e.payload["plan"]) {
                        if let Some(text) = action["text"].as_str() {
                            spoken.push(text.to_string());
                        }
                        if let Some(command) = action["command"].as_str() {
                            approved.push(command.to_string());
                        }
                    }
                }
                EventType::Tell => spoken.push(e.payload["fact"].as_str().unwrap_or("").to_string()),
                EventType::UiStateChange => shown.push(e.payload["state"].as_str().unwrap_or("").to_string()),
                _ => {}
            }
        }
        (spoken, approved, shown)
    }

    async fn outcomes(audit: &Arc<Mutex<AuditLog>>) -> Vec<String> {
        let mut all: Vec<String> = audit.lock().await.recent(20).unwrap().into_iter().map(|e| e.outcome).collect();
        all.reverse();
        all
    }

    /// Dry run: the question says what would run, not only why it's asking.
    #[tokio::test]
    async fn a_held_action_is_read_out_before_it_runs() {
        let mut r = rig("echo");
        r.gate.review(plan_ready("rm notes.txt")).await;

        let (spoken, approved, shown) = published(&mut r.rx);
        assert_eq!(spoken.len(), 1);
        assert!(spoken[0].contains("run rm notes.txt, since it modifies or deletes data"), "{}", spoken[0]);
        assert!(approved.is_empty(), "nothing runs before the answer");
        assert_eq!(shown, ["asking"]);
    }

    #[tokio::test]
    async fn a_clear_yes_runs_it_and_closes_the_question() {
        let mut r = rig("yes");
        r.gate.review(plan_ready("rm notes.txt")).await;
        published(&mut r.rx);

        assert!(r.awaiting.lock().unwrap().question().unwrap().contains("run rm notes.txt"));
        r.gate.answered(true).await;

        let (_, approved, shown) = published(&mut r.rx);
        assert_eq!(approved, ["rm notes.txt"]);
        assert_eq!(shown, ["answered"]);
        assert_eq!(outcomes(&r.audit).await, ["held_for_confirmation", "confirmed"]);
        assert!(r.awaiting.lock().unwrap().question().is_none(), "closed");
    }

    #[tokio::test]
    async fn a_no_cancels() {
        let mut r = rig("no");
        r.gate.review(plan_ready("rm notes.txt")).await;
        published(&mut r.rx);
        r.gate.answered(false).await;

        let (spoken, approved, _) = published(&mut r.rx);
        assert!(approved.is_empty());
        assert_eq!(spoken.len(), 1);
        assert!(spoken[0].contains("cancelled"), "{}", spoken[0]);
        assert_eq!(outcomes(&r.audit).await, ["held_for_confirmation", "declined"]);
    }

    #[tokio::test]
    async fn high_risk_actions_are_asked_twice() {
        let mut r = rig("admin");
        r.gate.review(plan_ready("sudo pacman -Syu")).await;
        let (spoken, _, _) = published(&mut r.rx);
        assert!(spoken[0].contains("run sudo pacman -Syu"), "{}", spoken[0]);
        r.gate.answered(true).await;
        let (spoken, approved, _) = published(&mut r.rx);
        assert!(approved.is_empty(), "one yes is not enough");
        assert!(spoken[0].contains("once more") && spoken[0].contains("can't be undone"), "{}", spoken[0]);
        r.gate.answered(true).await;
        assert_eq!(published(&mut r.rx).1, ["sudo pacman -Syu"]);
    }

    /// A second held plan must not replace the first without a trace.
    #[tokio::test]
    async fn a_replaced_question_is_closed_on_record() {
        let mut r = rig("superseded");
        r.gate.review(plan_ready("rm a.txt")).await;
        r.gate.review(plan_ready("rm b.txt")).await;
        assert_eq!(
            outcomes(&r.audit).await,
            ["held_for_confirmation", "held_for_confirmation", "superseded"]
        );
        published(&mut r.rx);
        r.gate.answered(true).await;
        assert_eq!(published(&mut r.rx).1, ["rm b.txt"], "yes means the question asked last");
    }

    /// A question nobody answers lapses on time, not whenever the user
    /// next happens to speak.
    #[tokio::test]
    async fn an_unanswered_question_lapses_by_itself() {
        let Rig { mut gate, mut rx, audit, awaiting } = rig("lapse");
        let bus = Arc::clone(&gate.bus);
        let running = tokio::spawn(async move { gate.run().await });
        tokio::time::sleep(Duration::from_millis(100)).await;
        bus.publish(plan_ready("rm notes.txt"));

        tokio::time::sleep(CONFIRMATION_WINDOW + Duration::from_millis(700)).await;
        let (_, approved, shown) = published(&mut rx);
        assert!(approved.is_empty());
        assert_eq!(shown, ["asking", "answered"]);
        assert_eq!(outcomes(&audit).await, ["held_for_confirmation", "expired"]);
        assert!(awaiting.lock().unwrap().question().is_none(), "no question left open");
        running.abort();
    }

    /// The whole path on one bus: ask, wait, run on a yes, undo.
    /// Nothing touches the file until the answer arrives.
    #[tokio::test]
    async fn ask_confirm_run_and_undo_end_to_end() {
        async fn until(what: &str, done: impl Fn() -> bool) {
            for _ in 0..100 {
                if done() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            panic!("timed out waiting until {}", what);
        }

        let dir = std::env::temp_dir().join(format!(
            "mavis_gate_e2e_{}_{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("notes.txt");
        std::fs::write(&file, "important").unwrap();

        let Rig { mut gate, audit, .. } = rig("e2e");
        let bus = Arc::clone(&gate.bus);
        let mut executor = crate::executor::Executor::new(
            Arc::clone(&bus),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            dir.join(".mavis-backup"),
        );
        let gate_task = tokio::spawn(async move { gate.run().await });
        let executor_task = tokio::spawn(async move { executor.run().await });
        tokio::time::sleep(Duration::from_millis(100)).await;

        bus.publish(plan_ready(&format!("rm '{}'", file.display())));
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(file.exists(), "held: nothing runs before the answer");

        bus.publish(answer(true));
        until("the file is deleted", || !file.exists()).await;

        bus.publish(event(EventType::PlanReady, serde_json::json!({ "plan": { "type": "undo" } })));
        until("the file is back", || file.exists()).await;
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "important");

        assert_eq!(outcomes(&audit).await, ["held_for_confirmation", "confirmed", "allowed"]);
        gate_task.abort();
        executor_task.abort();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_tap_on_the_orb_answers_yes() {
        let mut r = rig("tap");
        r.gate.review(plan_ready("rm notes.txt")).await;
        published(&mut r.rx);
        let tap = event(EventType::OrbTap, serde_json::json!({}));
        r.gate.tapped(&tap).await;
        assert_eq!(published(&mut r.rx).1, ["rm notes.txt"]);
        assert_eq!(outcomes(&r.audit).await, ["held_for_confirmation", "confirmed"]);
    }

    /// A stray click must not run an administrator-level command.
    #[tokio::test]
    async fn a_tap_is_not_enough_for_high_risk_actions() {
        let mut r = rig("tap_admin");
        r.gate.review(plan_ready("sudo pacman -Syu")).await;
        published(&mut r.rx);
        r.gate.tapped(&event(EventType::OrbTap, serde_json::json!({}))).await;
        let (spoken, approved, _) = published(&mut r.rx);
        assert!(approved.is_empty());
        assert_eq!(spoken.len(), 1);
        assert!(spoken[0].contains("out loud"), "{}", spoken[0]);
        assert!(r.awaiting.lock().unwrap().question().is_some(), "still open for a spoken answer");
    }

    #[tokio::test]
    async fn a_tap_with_no_question_open_does_nothing() {
        let mut r = rig("tap_idle");
        r.gate.tapped(&event(EventType::OrbTap, serde_json::json!({}))).await;
        let (spoken, approved, shown) = published(&mut r.rx);
        assert!(spoken.is_empty() && approved.is_empty() && shown.is_empty());
    }

    /// A refused command stays refused however the plan is wrapped.
    #[tokio::test]
    async fn a_wrapped_plan_cannot_slip_past_a_refusal() {
        let mut r = rig("wrapped");
        let plan = serde_json::json!({ "actions": [{ "type": "shell", "command": "rm -rf /" }] });
        r.gate.review(event(EventType::PlanReady, serde_json::json!({ "plan": plan }))).await;
        let (spoken, approved, _) = published(&mut r.rx);
        assert_eq!(spoken, ["You won't do that, and nothing ran: recursive deletion rooted at /."]);
        assert!(approved.is_empty());
        r.gate.answered(true).await;
        assert!(published(&mut r.rx).1.is_empty(), "there is nothing to say yes to");
    }

    #[test]
    fn low_risk_plans_are_allowed_without_asking() {
        let plan = serde_json::json!([
            { "type": "say", "text": "Volume up." },
            { "type": "system", "op": "volume_up" },
        ]);
        assert_eq!(risk::assess_plan(&plan).verdict, risk::Verdict::Allow);
    }
}