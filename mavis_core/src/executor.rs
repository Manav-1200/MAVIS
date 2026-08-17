// mavis_core/src/executor.rs
// Executes plans: shell commands, app launching, notifications, TTS.
// Listens for PlanReady, emits ActionComplete + UiStateChange.

use crate::event_bus::EventBus;
use crate::models::event::{Event, EventType};
use anyhow::Result;
use log::{error, info, warn};
use std::sync::Arc;
use tokio::process::Command;

pub struct Executor {
    bus: Arc<EventBus>,
}

impl Executor {
    pub fn new(bus: Arc<EventBus>) -> Self {
        Self { bus }
    }

    pub async fn run(&mut self) {
        let mut rx = self.bus.subscribe();
        info!("Executor: listening for events");
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Err(e) = self.handle_event(event).await {
                        warn!("Executor error: {}", e);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    warn!("Executor lagged by {} events", n);
                }
            }
        }
        info!("Executor: shutting down");
    }

    async fn handle_event(&self, event: Event) -> Result<()> {
        if event.event_type == EventType::PlanReady {
            if let Err(e) = self.execute_plan(event).await {
                warn!("Executor: plan execution failed: {}", e);
                self.emit_ui_state("error").await;
            }
        }
        Ok(())
    }

    async fn execute_plan(&self, event: Event) -> Result<()> {
        self.emit_ui_state("working").await;

        let plan_value = event.payload.get("plan").cloned().unwrap_or(serde_json::Value::Null);
        let actions = Self::extract_actions(&plan_value);

        if actions.is_empty() {
            warn!("Executor: PlanReady contained no executable actions");
            self.emit_ui_state("idle").await;
            return Ok(());
        }

        info!("Executor: executing plan with {} action(s)", actions.len());

        for (idx, action) in actions.iter().enumerate() {
            let action_type = action.get("type").and_then(|v| v.as_str()).unwrap_or("unknown");
            let description = action.get("description").and_then(|v| v.as_str()).unwrap_or("");
            info!(
                "Executor: action {} [{}] — {}",
                idx,
                action_type,
                if description.is_empty() { "(no description)" } else { description }
            );

            let result = self.execute_action(action).await;

            let success = result.is_ok();
            let output = result.as_ref().ok().cloned().unwrap_or_default();
            let error_msg = result.as_ref().err().map(|e| e.to_string()).unwrap_or_default();

            if !success {
                error!("Executor: action {} failed: {}", idx, error_msg);
            }

            // Emit ActionComplete so ContextEngine persists it and clears state
            let completion_event = Event {
                id: uuid::Uuid::new_v4(),
                timestamp: chrono::Utc::now(),
                source: "executor".to_string(),
                event_type: EventType::ActionComplete,
                payload: serde_json::json!({
                    "action_index": idx,
                    "action_type": action_type,
                    "description": description,
                    "success": success,
                    "output": output,
                    "error": error_msg,
                }),
            };
            self.bus.publish(completion_event);

            if !success {
                self.emit_ui_state("error").await;
                // Stop plan execution on first failure (safer default)
                return Ok(());
            }
        }

        self.emit_ui_state("idle").await;
        Ok(())
    }

    fn extract_actions(plan: &serde_json::Value) -> Vec<serde_json::Value> {
        if let Some(arr) = plan.as_array() {
            return arr.clone();
        }
        if let Some(obj) = plan.as_object() {
            if let Some(actions) = obj.get("actions").and_then(|v| v.as_array()) {
                return actions.clone();
            }
            // Single action object
            return vec![plan.clone()];
        }
        Vec::new()
    }

    async fn execute_action(&self, action: &serde_json::Value) -> Result<String> {
        let action_type = action.get("type").and_then(|v| v.as_str()).unwrap_or("unknown");

        match action_type {
            "shell" => {
                let cmd = action
                    .get("command")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("shell action missing 'command'"))?;
                self.run_shell(cmd).await
            }
            "app" => {
                let target = action
                    .get("target")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("app action missing 'target'"))?;
                let args: Vec<String> = action
                    .get("args")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                self.run_app(target, args).await
            }
            "notify" => {
                let title = action.get("title").and_then(|v| v.as_str()).unwrap_or("MAVIS");
                let message = action.get("message").and_then(|v| v.as_str()).unwrap_or("");
                self.run_notify(title, message).await
            }
            "say" => {
                let text = action
                    .get("text")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("say action missing 'text'"))?;
                self.run_say(text).await
            }
            "system" => {
                let op = action.get("op").and_then(|v| v.as_str()).unwrap_or("unknown");
                let system_event = Event {
                    id: uuid::Uuid::new_v4(),
                    timestamp: chrono::Utc::now(),
                    source: "executor".to_string(),
                    event_type: EventType::SystemAction,
                    payload: action.clone(),
                };
                self.bus.publish(system_event);
                Ok(format!("Delegated system action '{}' to DBus subsystem", op))
            }
            other => {
                warn!("Executor: unknown action type '{}'", other);
                Err(anyhow::anyhow!("unknown action type: {}", other))
            }
        }
    }

    async fn run_shell(&self, command: &str) -> Result<String> {
        info!("Executor: shell exec: {}", command);
        let output = Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
            .await
            .map_err(|e| anyhow::anyhow!("failed to spawn shell: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            let code = output.status.code().map_or("signal".to_string(), |c| c.to_string());
            return Err(anyhow::anyhow!("exit {}: {}", code, stderr.trim()));
        }

        let result = if stderr.is_empty() {
            stdout
        } else {
            format!("{}\n{}", stdout.trim(), stderr.trim())
        };

        Ok(result.trim().to_string())
    }

    async fn run_app(&self, target: &str, args: Vec<String>) -> Result<String> {
        info!("Executor: app launch: {} {:?}", target, args);
        let mut cmd = Command::new(target);
        cmd.args(&args);
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());

        match cmd.spawn() {
            Ok(child) => {
                let pid = child.id().map_or("?".to_string(), |p| p.to_string());
                info!("Executor: spawned {} (pid: {})", target, pid);
                Ok(format!("Launched {} (pid: {})", target, pid))
            }
            Err(direct_err) => {
                // Fallback to xdg-open for URLs, files, or unknown binaries
                let looks_like_url = target.contains("://");
                let looks_like_path = target.starts_with('/') || target.starts_with('~');
                if looks_like_url || looks_like_path {
                    let mut xdg = Command::new("xdg-open");
                    xdg.arg(target);
                    xdg.stdin(std::process::Stdio::null());
                    xdg.stdout(std::process::Stdio::null());
                    xdg.stderr(std::process::Stdio::null());
                    match xdg.spawn() {
                        Ok(child) => {
                            let pid = child.id().map_or("?".to_string(), |p| p.to_string());
                            Ok(format!("Opened {} via xdg-open (pid: {})", target, pid))
                        }
                        Err(xdg_err) => Err(anyhow::anyhow!(
                            "failed to launch {} ({}), xdg-open also failed ({})",
                            target,
                            direct_err,
                            xdg_err
                        )),
                    }
                } else {
                    Err(anyhow::anyhow!("failed to launch {}: {}", target, direct_err))
                }
            }
        }
    }

    async fn run_notify(&self, title: &str, message: &str) -> Result<String> {
        info!("Executor: notify: {} — {}", title, message);
        let output = Command::new("notify-send")
            .arg(title)
            .arg(message)
            .output()
            .await
            .map_err(|e| anyhow::anyhow!("notify-send failed: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("notify-send error: {}", stderr.trim()));
        }
        Ok(format!("Notification: {} — {}", title, message))
    }

    async fn run_say(&self, text: &str) -> Result<String> {
        info!("Executor: say: {}", text);

        // Try Piper first (best quality, most natural)
        let home = std::env::var("HOME").unwrap_or_default();
        let voice_model = std::env::var("MAVIS_VOICE_MODEL")
            .unwrap_or_else(|_| format!("{}/.local/share/piper-voices/en_US-lessac-medium.onnx", home));

        if std::path::Path::new(&voice_model).exists() {
            use std::process::{Command as StdCommand, Stdio as StdStdio};
            use std::io::Write;

            let mut piper = match StdCommand::new("piper")
                .args(&[
                    "--model", &voice_model,
                    "--output_file", "-",
                    "--length-scale", "1.15",
                    "--sentence-silence", "0.25",
                ])
                .stdin(StdStdio::piped())
                .stdout(StdStdio::piped())
                .stderr(StdStdio::null())
                .spawn()
            {
                Ok(child) => child,
                Err(_) => {
                    return self.fallback_say(text).await;
                }
            };

            if let Some(mut stdin) = piper.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }

            let _ = StdCommand::new("aplay")
                .args(&["-r", "22050", "-f", "S16_LE", "-c", "1", "-t", "raw", "-"])
                .stdin(piper.stdout.take().unwrap())
                .stdout(StdStdio::null())
                .stderr(StdStdio::null())
                .spawn();

            return Ok(format!("TTS (Piper): {}", text));
        }

        self.fallback_say(text).await
    }

    async fn fallback_say(&self, text: &str) -> Result<String> {
        for tts in ["spd-say", "espeak"] {
            let mut cmd = Command::new(tts);
            cmd.arg(text);
            cmd.stdin(std::process::Stdio::null());
            cmd.stdout(std::process::Stdio::null());
            cmd.stderr(std::process::Stdio::null());
            if let Ok(child) = cmd.spawn() {
                let pid = child.id().map_or("?".to_string(), |p| p.to_string());
                return Ok(format!("TTS via {} (pid: {}): {}", tts, pid, text));
            }
        }
        info!("Executor: no TTS binary found, logging only");
        Ok(format!("(say) {}", text))
    }

    async fn emit_ui_state(&self, state: &str) {
        let event = Event {
            id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            source: "executor".to_string(),
            event_type: EventType::UiStateChange,
            payload: serde_json::json!({ "state": state }),
        };
        self.bus.publish(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_actions_array() {
        let plan = serde_json::json!([{"type": "shell", "command": "echo hi"}]);
        let actions = Executor::extract_actions(&plan);
        assert_eq!(actions.len(), 1);
    }

    #[test]
    fn test_extract_actions_object_with_actions() {
        let plan = serde_json::json!({
            "actions": [{"type": "say", "text": "hello"}]
        });
        let actions = Executor::extract_actions(&plan);
        assert_eq!(actions.len(), 1);
    }

    #[test]
    fn test_extract_actions_single_object() {
        let plan = serde_json::json!({"type": "notify", "message": "test"});
        let actions = Executor::extract_actions(&plan);
        assert_eq!(actions.len(), 1);
    }

    #[test]
    fn test_extract_actions_invalid() {
        let plan = serde_json::json!("just a string");
        let actions = Executor::extract_actions(&plan);
        assert!(actions.is_empty());
    }
}