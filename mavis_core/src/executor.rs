// mavis_core/src/executor.rs
// Executes plans: shell commands, app launching, notifications, TTS.
// Listens for PlanApproved + TtsInterrupt, emits ActionComplete + UiStateChange.
// Only approved plans arrive here — the permission gate stands between this
// and the planner, so nothing runs without being scored and recorded.
//
// CHANGELOG 2026-08-25 (Phase 5):
//   - TTS queue: non-blocking say() with sequential playback
//   - TTS interruption: kill current playback + drain queue on TtsInterrupt
//   - Audio playback: pw-play > paplay > aplay (PipeWire-first for Arch)
//   - Piper model + .onnx.json validated before synthesis
//   - Kokoro WAV bytes and Piper WAV file both route through unified play_audio()
//   - spawn_audio_player respects MAVIS_AUDIO_DEVICE env var
//   - TTS queue emits Celebrating state briefly after the last queued item finishes

use crate::event_bus::EventBus;
use crate::models::event::{Event, EventType};
use crate::safety::rollback::{Rollback, UNDO_WINDOW};
use anyhow::Result;
use log::{debug, error, info, warn};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};

pub struct Executor {
    bus: Arc<EventBus>,
    tts_queue: TtsQueue,
    rollback: Rollback,
}

impl Executor {
    /// `backup_root` is where files are copied before a destructive
    /// command, so it can be undone.
    pub fn new(bus: Arc<EventBus>, tts_active: Arc<AtomicBool>, backup_root: PathBuf) -> Self {
        let tts_queue = TtsQueue::new(bus.clone(), tts_active);
        let rollback = Rollback::new(backup_root);
        // Copies left by an earlier run are past their window by now.
        rollback.prune();
        Self { bus, tts_queue, rollback }
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
        match event.event_type {
            EventType::PlanApproved => self.execute_plan(event).await,
            EventType::TtsInterrupt => {
                // A pause is provisional: MAVIS goes quiet at once, but the
                // reply is kept in case what interrupted it turns out to be
                // its own voice coming back through the microphone. The
                // transcript decides — TtsResume carries on, a second
                // TtsInterrupt without `pause` ends it for good.
                let pause = event
                    .payload
                    .get("pause")
                    .and_then(|p| p.as_bool())
                    .unwrap_or(false);
                if pause {
                    info!("Executor: pausing playback");
                    self.tts_queue.pause().await;
                } else {
                    info!("Executor: TTS interrupt received — draining queue");
                    self.tts_queue.interrupt().await;
                }
                Ok(())
            }
            EventType::TtsResume => {
                info!("Executor: resuming playback");
                self.tts_queue.resume().await;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    async fn execute_plan(&self, event: Event) -> Result<()> {
        self.emit_ui_state("working").await;

        let plan_value = event.payload.get("plan").cloned().unwrap_or(serde_json::Value::Null);
        let actions = Self::extract_actions(&plan_value);

        if actions.is_empty() {
            warn!("Executor: approved plan contained no executable actions");
            self.emit_ui_state("idle").await;
            return Ok(());
        }

        info!("Executor: executing plan with {} action(s)", actions.len());

        let mut tts_queued = false;

        for (idx, action) in actions.iter().enumerate() {
            let action_type = action.get("type").and_then(|v| v.as_str()).unwrap_or("unknown");
            let description = action.get("description").and_then(|v| v.as_str()).unwrap_or("");
            info!(
                "Executor: action {} [{}] — {}",
                idx,
                action_type,
                if description.is_empty() { "(no description)" } else { description }
            );

            let result = if action_type == "say" {
                tts_queued = true;
                self.run_say(action).await
            } else if action_type == "undo" {
                tts_queued = true;
                self.run_undo().await
            } else {
                self.execute_action(action).await
            };

            let success = result.is_ok();
            let output = result.as_ref().ok().cloned().unwrap_or_default();
            let error_msg = result.as_ref().err().map(|e| e.to_string()).unwrap_or_default();

            if !success {
                error!("Executor: action {} failed: {}", idx, error_msg);
            }

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
                self.tts_queue.interrupt().await;
                return Ok(());
            }
        }

        if !tts_queued {
            self.emit_ui_state("idle").await;
        }

        Ok(())
    }

    /// Same reading of a plan as the permission gate, so what was scored
    /// is what runs.
    fn extract_actions(plan: &serde_json::Value) -> Vec<serde_json::Value> {
        crate::safety::risk::actions(plan)
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
                self.run_say_text(text).await
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
        self.save_for_undo(command).await;
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

    /// Copy the files a destructive command names, and drop the copy once
    /// the undo window has passed.
    async fn save_for_undo(&self, command: &str) {
        let (rollback, command) = (self.rollback.clone(), command.to_string());
        let saved = tokio::task::spawn_blocking(move || rollback.snapshot(&command)).await;
        if let Ok(Some(count)) = saved {
            info!("Executor: saved {} path(s) — say \"undo\" within five minutes to restore", count);
            let rollback = self.rollback.clone();
            tokio::spawn(async move {
                sleep(UNDO_WINDOW + Duration::from_secs(1)).await;
                let _ = tokio::task::spawn_blocking(move || rollback.prune()).await;
            });
        }
    }

    /// Put back what the last destructive command changed, and say so.
    async fn run_undo(&self) -> Result<String> {
        let rollback = self.rollback.clone();
        let restored = tokio::task::spawn_blocking(move || rollback.undo()).await?;
        let message = match restored.as_deref() {
            Ok([]) => "There's nothing I can undo.".to_string(),
            Ok([one]) => {
                let name = Path::new(one).file_name().map(|n| n.to_string_lossy().to_string());
                format!("Restored {}.", name.unwrap_or_else(|| one.clone()))
            }
            Ok(many) => format!("Restored {} items.", many.len()),
            Err(e) => {
                warn!("Executor: undo failed: {}", e);
                "I couldn't undo that.".to_string()
            }
        };
        self.run_say_text(&message).await?;
        Ok(message)
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

    async fn run_say(&self, action: &serde_json::Value) -> Result<String> {
        let text = action
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("say action missing 'text'"))?;
        self.run_say_text(text).await
    }

    async fn run_say_text(&self, text: &str) -> Result<String> {
        info!("Executor: queue TTS: {}", text);
        self.tts_queue.say(text);
        Ok(format!("Queued TTS: {}", text))
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

// ---------------------------------------------------------------------
// TTS Queue — sequential playback with interruption support
// ---------------------------------------------------------------------

struct TtsQueue {
    queue_tx: mpsc::UnboundedSender<String>,
    kill_tx: mpsc::Sender<()>,
    current_pid: Arc<AtomicU32>,
    queue_depth: Arc<AtomicUsize>,
    /// Playback is suspended, waiting to be resumed or killed.
    paused: Arc<AtomicBool>,
}

impl TtsQueue {
    fn new(bus: Arc<EventBus>, tts_active: Arc<AtomicBool>) -> Self {
        let (queue_tx, mut queue_rx) = mpsc::unbounded_channel::<String>();
        let (kill_tx, mut kill_rx) = mpsc::channel::<()>(1);
        let current_pid = Arc::new(AtomicU32::new(0));
        let pid_for_task = current_pid.clone();
        let bus_clone = bus.clone();
        let tts_active_clone = tts_active.clone();
        let queue_depth = Arc::new(AtomicUsize::new(0));
        let queue_depth_for_task = queue_depth.clone();
        let paused = Arc::new(AtomicBool::new(false));
        let paused_for_task = paused.clone();

        tokio::spawn(async move {
            while let Some(text) = queue_rx.recv().await {
                while kill_rx.try_recv().is_ok() {}

                tts_active_clone.store(true, Ordering::SeqCst);
                // A new item is never born paused, whatever happened to
                // the last one.
                paused_for_task.store(false, Ordering::SeqCst);
                Self::emit_state(&bus_clone, "speaking").await;

                let interrupted = Self::play_text(&text, &pid_for_task, &mut kill_rx).await;
                let was_last = queue_depth_for_task.fetch_sub(1, Ordering::SeqCst) == 1;

                if interrupted {
                    let mut drained = 0;
                    while queue_rx.try_recv().is_ok() {
                        drained += 1;
                    }
                    if drained > 0 {
                        queue_depth_for_task.fetch_sub(drained, Ordering::SeqCst);
                    }
                    tts_active_clone.store(false, Ordering::SeqCst);
                    Self::emit_state(&bus_clone, "idle").await;
                    continue;
                }

                if was_last {
                    Self::emit_state(&bus_clone, "celebrating").await;
                    sleep(Duration::from_millis(1200)).await;
                }

                tts_active_clone.store(false, Ordering::SeqCst);
                Self::emit_state(&bus_clone, "idle").await;
            }

            tts_active_clone.store(false, Ordering::SeqCst);
            Self::emit_state(&bus_clone, "idle").await;
        });

        Self {
            queue_tx,
            kill_tx,
            current_pid,
            queue_depth,
            paused,
        }
    }

    fn say(&self, text: &str) {
        self.queue_depth.fetch_add(1, Ordering::SeqCst);
        let _ = self.queue_tx.send(text.to_string());
    }

    async fn interrupt(&self) {
        let pid = self.current_pid.load(Ordering::SeqCst);
        if pid != 0 {
            // CONT first: a paused player is stopped, and a stopped
            // process doesn't act on SIGTERM until it runs again.
            let _ = Command::new("kill").arg("-CONT").arg(pid.to_string()).output().await;
            let _ = Command::new("kill")
                .arg("-15")
                .arg(pid.to_string())
                .output()
                .await;
        }
        self.paused.store(false, Ordering::SeqCst);
        let _ = self.kill_tx.try_send(());
    }

    /// Silence playback without losing it. SIGSTOP suspends the player
    /// mid-sample; SIGCONT picks up exactly where it stopped.
    async fn pause(&self) {
        let pid = self.current_pid.load(Ordering::SeqCst);
        if pid == 0 || self.paused.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = Command::new("kill").arg("-STOP").arg(pid.to_string()).output().await;
    }

    async fn resume(&self) {
        let pid = self.current_pid.load(Ordering::SeqCst);
        if pid == 0 || !self.paused.swap(false, Ordering::SeqCst) {
            return;
        }
        let _ = Command::new("kill").arg("-CONT").arg(pid.to_string()).output().await;
    }

    async fn play_text(
        text: &str,
        current_pid: &Arc<AtomicU32>,
        kill_rx: &mut mpsc::Receiver<()>,
    ) -> bool {
        let use_kokoro = std::env::var("MAVIS_TTS_ENGINE")
            .map(|v| v.eq_ignore_ascii_case("kokoro"))
            .unwrap_or(false);

        let result = if use_kokoro {
            Self::play_kokoro(text, current_pid, kill_rx).await
        } else {
            Self::play_piper(text, current_pid, kill_rx).await
        };

        match result {
            Ok(interrupted) => interrupted,
            Err(e) => {
                warn!("TTS playback error: {}", e);
                false
            }
        }
    }

    async fn play_kokoro(
        text: &str,
        current_pid: &Arc<AtomicU32>,
        kill_rx: &mut mpsc::Receiver<()>,
    ) -> Result<bool> {
        let wav_path = match synthesize_via_worker(text).await {
            Ok(bytes) => {
                let path = crate::util::runtime_dir().join("mavis_tts_kokoro.wav");
                tokio::fs::write(&path, &bytes).await?;
                path
            }
            Err(e) => {
                warn!("Kokoro synthesis failed ({}), falling back to Piper", e);
                return Self::play_piper(text, current_pid, kill_rx).await;
            }
        };

        let mut child = spawn_audio_player(&wav_path).await?;
        if let Some(pid) = child.id() {
            current_pid.store(pid, Ordering::SeqCst);
        }

        let interrupted = tokio::select! {
            result = child.wait() => {
                if let Err(e) = result {
                    warn!("Audio playback error: {}", e);
                }
                false
            }
            _ = kill_rx.recv() => {
                let _ = child.kill().await;
                true
            }
        };

        current_pid.store(0, Ordering::SeqCst);
        let _ = tokio::fs::remove_file(&wav_path).await;
        Ok(interrupted)
    }

    async fn play_piper(
        text: &str,
        current_pid: &Arc<AtomicU32>,
        kill_rx: &mut mpsc::Receiver<()>,
    ) -> Result<bool> {
        let home = std::env::var("HOME").unwrap_or_default();
        let voice_model = std::env::var("MAVIS_VOICE_MODEL")
            .unwrap_or_else(|_| format!("{}/.local/share/piper-voices/en_US-lessac-medium.onnx", home));
        let model_path = Path::new(&voice_model);
        let json_path = model_path.with_extension("onnx.json");

        if !model_path.exists() || !json_path.exists() {
            fallback_say_blocking(text).await?;
            return Ok(false);
        }

        let wav_path = crate::util::runtime_dir().join("mavis_tts_piper.wav");
        run_piper_to_file(text, &voice_model, &wav_path).await?;

        let mut child = spawn_audio_player(&wav_path).await?;
        if let Some(pid) = child.id() {
            current_pid.store(pid, Ordering::SeqCst);
        }

        let interrupted = tokio::select! {
            result = child.wait() => {
                if let Err(e) = result {
                    warn!("Audio playback error: {}", e);
                }
                false
            }
            _ = kill_rx.recv() => {
                let _ = child.kill().await;
                true
            }
        };

        current_pid.store(0, Ordering::SeqCst);
        let _ = tokio::fs::remove_file(&wav_path).await;
        Ok(interrupted)
    }

    async fn emit_state(bus: &Arc<EventBus>, state: &str) {
        let event = Event {
            id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            source: "executor".to_string(),
            event_type: EventType::UiStateChange,
            payload: serde_json::json!({ "state": state }),
        };
        let _ = bus.publish(event);
    }
}

// ---------------------------------------------------------------------
// Audio helpers
// ---------------------------------------------------------------------

/// Spawn the first available audio player backend. Returns the Child handle.
/// Respects MAVIS_AUDIO_DEVICE env var for pw-play and paplay.
async fn spawn_audio_player(path: &Path) -> Result<tokio::process::Child> {
    if !path.exists() {
        return Err(anyhow::anyhow!("WAV file does not exist: {:?}", path));
    }

    // MAVIS_AUDIO_OUTPUT, not MAVIS_AUDIO_DEVICE: the latter names the
    // *microphone*, and passing it here asked the player to output to a
    // capture device, which silently broke playback for anyone who set it.
    // The flags differ per player too — pw-play takes --target, not
    // --device, which was the second half of the same bug.
    let device_arg = std::env::var("MAVIS_AUDIO_OUTPUT").ok();

    let backends: [(&str, Vec<String>); 3] = [
        ("pw-play", {
            let mut args = vec![path.to_string_lossy().to_string()];
            if let Some(ref dev) = device_arg {
                args.extend_from_slice(&["--target".to_string(), dev.clone()]);
            }
            args
        }),
        ("paplay", {
            let mut args = vec![path.to_string_lossy().to_string()];
            if let Some(ref dev) = device_arg {
                args.extend_from_slice(&["--device".to_string(), dev.clone()]);
            }
            args
        }),
        ("aplay", {
            let mut args = vec![];
            if let Some(ref dev) = device_arg {
                args.extend_from_slice(&["-D".to_string(), dev.clone()]);
            }
            args.push(path.to_string_lossy().to_string());
            args
        }),
    ];

    for (cmd, args) in backends {
        debug!("Trying audio backend: {}", cmd);
        match Command::new(cmd).args(&args).spawn() {
            Ok(child) => {
                info!("Audio playback spawned via {} (pid={:?}, device={:?})", cmd, child.id(), device_arg);
                return Ok(child);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                debug!("{} not found in PATH, skipping", cmd);
            }
            Err(e) => {
                warn!("{} spawn error: {}", cmd, e);
            }
        }
    }

    Err(anyhow::anyhow!(
        "All audio playback backends failed. \
         Install one of: pw-play (pipewire), paplay (pulseaudio), aplay (alsa-utils)."
    ))
}

/// Run piper synthesis to a WAV file (no playback).
async fn run_piper_to_file(text: &str, voice_model: &str, wav_path: &Path) -> Result<()> {
    let mut child = Command::new("piper")
        .args(&[
            "--model",
            voice_model,
            "--output_file",
            &wav_path.to_string_lossy(),
            "--length-scale",
            "1.15",
            "--sentence-silence",
            "0.25",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to spawn piper: {}", e))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .await
            .map_err(|e| anyhow::anyhow!("failed to write to piper stdin: {}", e))?;
    }

    let output = child
        .wait_with_output()
        .await
        .map_err(|e| anyhow::anyhow!("piper process error: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("piper synthesis failed: {}", stderr));
    }

    if !wav_path.exists() {
        return Err(anyhow::anyhow!("Piper did not produce an output WAV file"));
    }

    Ok(())
}

/// Fallback TTS via spd-say or espeak. Blocks until speech has finished.
///
/// Each binary is tried in turn. Previously a missing `spd-say` returned
/// an error straight out of the loop, so `espeak` was never tried — the
/// common case on minimal installs, which ship espeak-ng without
/// speech-dispatcher. `spd-say` gets `--wait`: without it, it hands the
/// text to speech-dispatcher and exits at once, so the microphone reopened
/// while MAVIS was still talking and heard itself.
async fn fallback_say_blocking(text: &str) -> Result<()> {
    let candidates: [(&str, &[&str]); 3] =
        [("spd-say", &["--wait"]), ("espeak-ng", &[]), ("espeak", &[])];
    for (tts, flags) in candidates {
        let mut child = match Command::new(tts)
            .args(flags)
            .arg(text)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => continue, // not installed — try the next one
        };
        match child.wait().await {
            Ok(status) if status.success() => {
                info!("TTS via {}: {}", tts, text);
                return Ok(());
            }
            Ok(status) => warn!("Executor: {} exited with {}, trying next", tts, status),
            Err(e) => warn!("Executor: {} wait error: {}, trying next", tts, e),
        }
    }
    info!("Executor: no TTS binary found, logging only: {}", text);
    Ok(())
}

/// Synthesize text via the Python worker (Kokoro). Returns raw WAV bytes.
async fn synthesize_via_worker(text: &str) -> Result<Vec<u8>> {
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;
    use tokio::time::{timeout, Duration};

    const WORKER_SOCKET: &str = "/tmp/mavis_worker.sock";
    let voice = std::env::var("MAVIS_KOKORO_VOICE").unwrap_or_else(|_| "af_heart".to_string());

    let request = serde_json::json!({
        "type": "WorkerRequest",
        "payload": {
            "request_type": "tts",
            "text": text,
            "voice": voice,
        }
    });
    let req_str = request.to_string();
    let req_bytes = req_str.as_bytes();

    let response_str = timeout(Duration::from_secs(20), async {
        let mut stream = UnixStream::connect(WORKER_SOCKET).await?;
        stream.write_all(&(req_bytes.len() as u32).to_le_bytes()).await?;
        stream.write_all(req_bytes).await?;
        stream.flush().await?;

        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await?;
        let resp_len = u32::from_le_bytes(len_buf) as usize;
        let mut resp_buf = vec![0u8; resp_len];
        stream.read_exact(&mut resp_buf).await?;
        Ok::<_, std::io::Error>(String::from_utf8_lossy(&resp_buf).to_string())
    })
    .await
    .map_err(|_| anyhow::anyhow!("TTS worker request timed out"))??;

    let resp_json: serde_json::Value = serde_json::from_str(&response_str)?;

    if let Some(err) = resp_json.get("payload").and_then(|p| p.get("error")) {
        anyhow::bail!("worker TTS error: {}", err);
    }

    let audio_b64 = resp_json
        .get("payload")
        .and_then(|p| p.get("result"))
        .and_then(|r| r.get("audio"))
        .and_then(|a| a.as_str())
        .ok_or_else(|| anyhow::anyhow!("worker TTS response missing audio field"))?;

    B64.decode(audio_b64)
        .map_err(|e| anyhow::anyhow!("failed to decode TTS audio: {}", e))
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

    /// A destructive command run through the executor can be undone.
    #[tokio::test]
    async fn a_deleted_file_is_restored_by_undo() {
        let dir = std::env::temp_dir().join(format!(
            "mavis_exec_undo_{}_{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("notes.txt");
        std::fs::write(&file, "important").unwrap();

        let executor = Executor::new(
            Arc::new(EventBus::new()),
            Arc::new(AtomicBool::new(false)),
            dir.join(".mavis-backup"),
        );
        executor.run_shell(&format!("rm '{}'", file.display())).await.unwrap();
        assert!(!file.exists(), "the command really ran");

        assert_eq!(executor.run_undo().await.unwrap(), "Restored notes.txt.");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "important");
        assert_eq!(executor.run_undo().await.unwrap(), "There's nothing I can undo.");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_extract_actions_invalid() {
        let plan = serde_json::json!("just a string");
        let actions = Executor::extract_actions(&plan);
        assert!(actions.is_empty());
    }
}