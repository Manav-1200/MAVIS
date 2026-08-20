// Entry point. Initializes subsystems, wires them together, runs until shutdown.

use anyhow::Result;
use log::{info, warn};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::sync::mpsc;

mod bridge;
mod context_engine;
mod event_bus;
mod executor;
mod memory;
mod models;
mod planner;
mod platform;
mod stt;
mod system;
mod ui;
mod tts;

use event_bus::EventBus;
use models::event::{Event, EventType};

const WORKER_SOCKET: &str = "/tmp/mavis_worker.sock";

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    info!("MAVIS starting...");

    let bus = Arc::new(EventBus::new());
    let (_shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);

    // Platform layer — Linux / Windows / macOS abstraction
    let platform = Arc::new(platform::Platform::detect().build_provider());
    info!("Platform: initialized");

    // Memory Manager — shared between ContextEngine and Planner
    let data_dir = std::path::Path::new("../memory");
    let memory = memory::manager::MemoryManager::new(data_dir)?;
    let working_memory = memory.working.clone();

    // Context Engine
    let bus_pub = Arc::clone(&bus);
    let bus_sub = Arc::clone(&bus);
    let mut ctx_engine = context_engine::ContextEngine::new(bus_pub, memory)?;
    let ctx_handle = tokio::spawn(async move {
        let mut rx = bus_sub.subscribe();
        info!("ContextEngine: listening for events");
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Err(e) = ctx_engine.process_event(event).await {
                        warn!("ContextEngine error: {}", e);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    warn!("ContextEngine lagged by {} events", n);
                }
            }
        }
        info!("ContextEngine: shutting down");
    });

    // Planner
    let bus_clone = Arc::clone(&bus);
    let mut planner = planner::Planner::new(bus_clone, working_memory);
    let planner_handle = tokio::spawn(async move {
        planner.run().await;
    });

    // Executor
    let bus_clone = Arc::clone(&bus);
    let mut executor = executor::Executor::new(bus_clone);
    let exec_handle = tokio::spawn(async move {
        executor.run().await;
    });

    // DBus Integration
    let bus_clone = Arc::clone(&bus);
    let mut dbus = system::dbus::DbusIntegration::new(bus_clone);
    let dbus_handle = tokio::spawn(async move {
        dbus.run().await;
    });

    // Hotkey Manager
    let bus_clone = Arc::clone(&bus);
    let mut hotkeys = system::hotkeys::HotkeyManager::new(bus_clone);
    let hotkeys_handle = tokio::spawn(async move {
        hotkeys.run().await;
    });

    // File Watcher
    let bus_clone = Arc::clone(&bus);
    let mut watcher = system::watcher::FileWatcher::new(bus_clone);
    let watcher_handle = tokio::spawn(async move {
        watcher.run().await;
    });

    // Worker Bridge
    let bus_clone = Arc::clone(&bus);
    let bridge_handle = tokio::spawn(async move {
        let bridge = bridge::worker_bridge::WorkerBridge::new();
        if let Err(e) = bridge.run(bus_clone).await {
            warn!("WorkerBridge error: {}", e);
        }
    });

    // STT Pipeline
    let bus_for_stt_start = Arc::clone(&bus);
    let (stt_handle, mut utterance_rx) = stt::SttManager::new(stt::SttConfig::default()).start(bus_for_stt_start);
    let bus_for_stt = Arc::clone(&bus);

    // STT mute controller — mutes mic while TTS is speaking to prevent feedback loop
    let tts_active_clone = stt_handle.tts_active.clone();
    let bus_for_mute = Arc::clone(&bus);
    let mute_handle = tokio::spawn(async move {
        let mut rx = bus_for_mute.subscribe();
        while let Ok(event) = rx.recv().await {
            if let EventType::UiStateChange = event.event_type {
                if let Some(state) = event.payload.get("state").and_then(|v| v.as_str()) {
                    match state {
                        "speaking" => tts_active_clone.store(true, Ordering::SeqCst),
                        "idle" | "error" | "listening" | "thinking" | "working" => {
                            tts_active_clone.store(false, Ordering::SeqCst);
                        }
                        _ => {}
                    }
                }
            }
        }
    });

    let stt_task = tokio::spawn(async move {
        use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixStream;
        use tokio::time::{sleep, timeout, Duration};

        while let Some(audio) = utterance_rx.recv().await {
            // Wait for worker socket to exist (up to 30s)
            let socket_ready = timeout(Duration::from_secs(30), async {
                while !std::path::Path::new(WORKER_SOCKET).exists() {
                    info!("STT: waiting for worker socket...");
                    sleep(Duration::from_millis(500)).await;
                }
            }).await;

            if socket_ready.is_err() {
                warn!("STT: worker socket never appeared — dropping utterance");
                continue;
            }

            let bytes: Vec<u8> = audio.iter().flat_map(|s| s.to_le_bytes()).collect();
            let audio_b64 = B64.encode(&bytes);

            let request = serde_json::json!({
                "type": "WorkerRequest",
                "payload": {
                    "request_type": "stt",
                    "audio": audio_b64,
                }
            });
            let req_str = request.to_string();
            let req_bytes = req_str.as_bytes();

            // Try to connect and send with 60s timeout per attempt (model load can be slow)
            let mut response_text: Option<String> = None;
            for attempt in 1..=5 {
                let result = timeout(Duration::from_secs(60), async {
                    let mut stream = UnixStream::connect(WORKER_SOCKET).await?;
                    let len = req_bytes.len() as u32;
                    stream.write_all(&len.to_le_bytes()).await?;
                    stream.write_all(req_bytes).await?;
                    stream.flush().await?;

                    let mut len_buf = [0u8; 4];
                    stream.read_exact(&mut len_buf).await?;
                    let resp_len = u32::from_le_bytes(len_buf) as usize;
                    let mut resp_buf = vec![0u8; resp_len];
                    stream.read_exact(&mut resp_buf).await?;

                    let resp_str = String::from_utf8_lossy(&resp_buf);
                    if let Ok(resp_json) = serde_json::from_str::<serde_json::Value>(&resp_str) {
                        if let Some(text) = resp_json
                            .get("payload")
                            .and_then(|p| p.get("result"))
                            .and_then(|r| r.get("text"))
                            .and_then(|t| t.as_str())
                        {
                            return Ok::<_, std::io::Error>(Some(text.to_string()));
                        }
                    }
                    Ok(None)
                }).await;

                match result {
                    Ok(Ok(Some(text))) => {
                        response_text = Some(text);
                        break;
                    }
                    Ok(Ok(None)) => {
                        warn!("STT: empty response (attempt {}/5)", attempt);
                        sleep(Duration::from_millis(500)).await;
                    }
                    Ok(Err(e)) => {
                        warn!("STT: I/O error (attempt {}/5): {}", attempt, e);
                        sleep(Duration::from_millis(500)).await;
                    }
                    Err(_) => {
                        warn!("STT: timeout (attempt {}/5) — model may still be loading", attempt);
                        sleep(Duration::from_millis(500)).await;
                    }
                }
            }

            if let Some(text) = response_text {
                if !text.is_empty() {
                    info!("STT transcription: {}", text);
                    let event = Event {
                        id: uuid::Uuid::new_v4(),
                        timestamp: chrono::Utc::now(),
                        source: "stt".to_string(),
                        event_type: EventType::UserIntent,
                        payload: serde_json::json!({
                            "source": "voice",
                            "text": text,
                        }),
                    };
                    let _ = bus_for_stt.publish(event);
                } else {
                    info!("STT: empty transcription (silence or no speech)");
                    let _ = bus_for_stt.publish(Event {
                        id: uuid::Uuid::new_v4(),
                        timestamp: chrono::Utc::now(),
                        source: "stt".to_string(),
                        event_type: EventType::UiStateChange,
                        payload: serde_json::json!({ "state": "idle" }),
                    });
                }
            } else {
                warn!("STT: failed to get transcription after retries");
                let _ = bus_for_stt.publish(Event {
                    id: uuid::Uuid::new_v4(),
                    timestamp: chrono::Utc::now(),
                    source: "stt".to_string(),
                    event_type: EventType::UiStateChange,
                    payload: serde_json::json!({ "state": "error" }),
                });
            }
        }
        info!("STT pipeline shut down");
    });

    // Orb UI
    let bus_clone = Arc::clone(&bus);
    let orb_handle = tokio::spawn(async move {
        let orb = ui::Orb::new();
        info!("Orb: initialized");

        let mut rx = bus_clone.subscribe();
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let EventType::UiStateChange = event.event_type {
                        if let Some(state_str) =
                            event.payload.get("state").and_then(|v| v.as_str())
                        {
                            let state = match state_str {
                                "idle" => ui::OrbState::Idle,
                                "listening" => ui::OrbState::Listening,
                                "thinking" => ui::OrbState::Thinking,
                                "speaking" => ui::OrbState::Speaking,
                                "working" => ui::OrbState::Working,
                                "error" => ui::OrbState::Error,
                                "asleep" => ui::OrbState::Asleep,
                                _ => ui::OrbState::Idle,
                            };
                            orb.set_state(state);
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    warn!("Orb lagged by {} events", n);
                }
            }
        }
        orb.shutdown();
        info!("Orb: shutting down");
    });

    // Platform context polling — Phase 6 (active window, clipboard, etc.)
    let platform_ctx = Arc::clone(&platform);
    let bus_ctx = Arc::clone(&bus);
    let _ctx_poll_handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(tokio::time::Duration::from_secs(2));
        loop {
            ticker.tick().await;
            if let Some(tracker) = platform_ctx.windows() {
                if let Ok((app, title, pid)) = tracker.active_window() {
                    log::debug!("Window: {} | {} | {}", app, title, pid);
                    // Phase 6: inject into ContextEngine working memory
                }
            }
        }
    });

    // Startup event
    bus.publish(Event {
        id: uuid::Uuid::new_v4(),
        timestamp: chrono::Utc::now(),
        source: "mavis_core".to_string(),
        event_type: EventType::SystemWake,
        payload: serde_json::json!({ "status": "ready" }),
    });

    info!("MAVIS runtime ready. Press Ctrl+C to shutdown.");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("Shutdown signal received.");
        }
        _ = shutdown_rx.recv() => {
            info!("Shutdown requested via event bus.");
        }
    }

    // CRITICAL: close the bus first. This wakes up every rx.recv().await immediately.
    bus.close();
    info!("EventBus closed — signaling all subsystems to shut down");

    // Stop STT first so the audio thread exits cleanly
    stt_handle.stop();

    // Join everything concurrently so total shutdown is capped at 5s, not 5s × N.
    let timeout = std::time::Duration::from_secs(5);
    let _ = tokio::join!(
        tokio::time::timeout(timeout, ctx_handle),
        tokio::time::timeout(timeout, planner_handle),
        tokio::time::timeout(timeout, exec_handle),
        tokio::time::timeout(timeout, dbus_handle),
        tokio::time::timeout(timeout, hotkeys_handle),
        tokio::time::timeout(timeout, watcher_handle),
        tokio::time::timeout(timeout, bridge_handle),
        tokio::time::timeout(timeout, stt_task),
        tokio::time::timeout(timeout, mute_handle),
        tokio::time::timeout(timeout, orb_handle),
    );

    info!("MAVIS shutdown complete.");
    Ok(())
}