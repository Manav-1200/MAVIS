// Entry point. Initializes subsystems, wires them together, runs until shutdown.

use anyhow::Result;
use log::{info, warn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

mod bridge;
mod context_engine;
mod context_snapshot;
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

use context_snapshot::{ContextSnapshot, WindowInfo};
use event_bus::EventBus;
use models::event::{Event, EventType};

const WORKER_SOCKET: &str = "/tmp/mavis_worker.sock";

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    info!("MAVIS starting...");

    let bus = Arc::new(EventBus::new());
    let (_shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);

    // Shared TTS state — true while MAVIS is speaking. Mic is muted when true.
    let tts_active = Arc::new(AtomicBool::new(false));

    // Platform layer
    let platform = Arc::new(platform::Platform::detect().build_provider());
    info!("Platform: initialized");

    // Memory Manager
    let data_dir = std::path::Path::new("../memory");
    let memory = memory::manager::MemoryManager::new(data_dir)?;
    let memory_for_shutdown = memory.clone();
    let working_memory = memory.working.clone();
    info!("Memory: initialized (working events={})", memory.working.read().await.events.len());

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
    let tts_active_for_exec = tts_active.clone();
    let mut executor = executor::Executor::new(bus_clone, tts_active_for_exec);
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

    // Orb — created here so we can clone it for the energy task
    let orb = ui::Orb::new();
    let orb_for_energy = orb.clone();

    // STT Pipeline
    let bus_for_stt_start = Arc::clone(&bus);
    let (speech_start_tx, mut speech_start_rx) = mpsc::channel::<()>(8);
    let (stt_handle, mut utterance_rx, mut energy_rx) = stt::SttManager::new(stt::SttConfig::default())
        .start(bus_for_stt_start, Some(speech_start_tx));
    let bus_for_stt = Arc::clone(&bus);

    // Voice activity LED — pipe real-time VAD energy into the orb
    let energy_handle = tokio::spawn(async move {
        while let Some(energy) = energy_rx.recv().await {
            orb_for_energy.set_energy(energy);
        }
    });

    // STT mute controller
    let tts_active_for_mute = tts_active.clone();
    let bus_for_mute = Arc::clone(&bus);
    let mute_handle = tokio::spawn(async move {
        let mut rx = bus_for_mute.subscribe();
        while let Ok(event) = rx.recv().await {
            if let EventType::UiStateChange = event.event_type {
                if let Some(state) = event.payload.get("state").and_then(|v| v.as_str()) {
                    match state {
                        "speaking" => tts_active_for_mute.store(true, Ordering::SeqCst),
                        "idle" | "error" => {
                            tts_active_for_mute.store(false, Ordering::SeqCst);
                        }
                        _ => {}
                    }
                }
            }
        }
    });

    // Intent router — interrupt TTS when user speaks during playback
    let tts_active_for_router = tts_active.clone();
    let bus_for_router = Arc::clone(&bus);
    let router_handle = tokio::spawn(async move {
        let mut rx = bus_for_router.subscribe();
        while let Ok(event) = rx.recv().await {
            if event.event_type == EventType::UserIntent {
                if tts_active_for_router.load(Ordering::SeqCst) {
                    info!("IntentRouter: user spoke during TTS — interrupting");
                    let _ = bus_for_router.publish(Event {
                        id: uuid::Uuid::new_v4(),
                        timestamp: chrono::Utc::now(),
                        source: "intent_router".to_string(),
                        event_type: EventType::TtsInterrupt,
                        payload: serde_json::json!({}),
                    });
                    tts_active_for_router.store(false, Ordering::SeqCst);
                }
            }
        }
    });

    // Parallel LLM warm-up
    let warmup_handle = tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        use tokio::net::UnixStream;
        use tokio::time::{timeout, Duration};

        while speech_start_rx.recv().await.is_some() {
            let request = serde_json::json!({
                "type": "WorkerRequest",
                "payload": {
                    "request_type": "warmup",
                }
            });
            let req_bytes = request.to_string().into_bytes();

            let _ = timeout(Duration::from_secs(10), async {
                let mut stream = UnixStream::connect(WORKER_SOCKET).await?;
                stream.write_all(&(req_bytes.len() as u32).to_le_bytes()).await?;
                stream.write_all(&req_bytes).await?;
                stream.flush().await?;
                Ok::<_, std::io::Error>(())
            }).await;
        }
    });

    let stt_task = tokio::spawn(async move {
        use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixStream;
        use tokio::time::{sleep, timeout, Duration};

        while let Some(audio) = utterance_rx.recv().await {
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

            let mut response_text: Option<String> = None;
            for attempt in 1..=5 {
                let attempt_timeout = if attempt == 1 {
                    Duration::from_secs(300)
                } else {
                    Duration::from_secs(60)
                };

                let result = timeout(attempt_timeout, async {
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
                        warn!("STT: timeout (attempt {}/5)", attempt);
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

    // Orb UI task
    let bus_clone = Arc::clone(&bus);
    let orb_handle = tokio::spawn(async move {
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
                                "celebrating" => ui::OrbState::Celebrating,
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

    // Platform context polling
    let platform_ctx = Arc::clone(&platform);
    let bus_ctx = Arc::clone(&bus);
    let _ctx_poll_handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(tokio::time::Duration::from_secs(2));
        loop {
            ticker.tick().await;

            let mut snapshot = ContextSnapshot {
                active_window: None,
                clipboard_text: None,
                captured_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
            };

            if let Some(tracker) = platform_ctx.windows() {
                match tracker.active_window() {
                    Ok((app, title, pid)) => {
                        snapshot.active_window = Some(WindowInfo {
                            app_name: app,
                            window_title: title,
                            pid: Some(pid),
                        });
                    }
                    Err(e) => {
                        log::debug!("Window tracking error: {}", e);
                    }
                }
            }

            if let Some(clipboard) = platform_ctx.clipboard() {
                match clipboard.read_text() {
                    Ok(Some(text)) => {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() && trimmed.len() < 4096 {
                            snapshot.clipboard_text = Some(trimmed.to_string());
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        log::debug!("Clipboard read error: {}", e);
                    }
                }
            }

            if !snapshot.is_empty() {
                match serde_json::to_value(&snapshot) {
                    Ok(payload) => {
                        let event = Event {
                            id: uuid::Uuid::new_v4(),
                            timestamp: chrono::Utc::now(),
                            source: "platform".to_string(),
                            event_type: EventType::ContextUpdate,
                            payload,
                        };
                        bus_ctx.publish(event);
                    }
                    Err(e) => {
                        log::warn!("Failed to serialize context snapshot: {}", e);
                    }
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

    // Final save
    if let Err(e) = memory_for_shutdown.save_working().await {
        warn!("Final working memory save failed: {}", e);
    }

    bus.close();
    info!("EventBus closed — signaling all subsystems to shut down");

    stt_handle.stop();
    drop(stt_handle);

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
        tokio::time::timeout(timeout, warmup_handle),
        tokio::time::timeout(timeout, orb_handle),
        tokio::time::timeout(timeout, router_handle),
        tokio::time::timeout(timeout, energy_handle),
    );

    info!("MAVIS shutdown complete.");
    Ok(())
}