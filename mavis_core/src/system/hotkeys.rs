// mavis_core/src/system/hotkeys.rs
// Wayland-compatible hotkey activation via Unix domain socket.
// Bind a key in Niri to: echo '{"intent":"toggle_listen"}' | nc -U /tmp/mavis_hotkey.sock

use crate::event_bus::EventBus;
use crate::models::event::{Event, EventType};
use anyhow::Result;
use log::{info, warn};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::UnixListener;

const HOTKEY_SOCKET: &str = "/tmp/mavis_hotkey.sock";

pub struct HotkeyManager {
    bus: Arc<EventBus>,
}

impl HotkeyManager {
    pub fn new(bus: Arc<EventBus>) -> Self {
        Self { bus }
    }

    pub async fn run(&mut self) {
        let _ = tokio::fs::remove_file(HOTKEY_SOCKET).await;

        let listener = match UnixListener::bind(HOTKEY_SOCKET) {
            Ok(l) => l,
            Err(e) => {
                warn!("HotkeyManager: failed to bind socket at {}: {}", HOTKEY_SOCKET, e);
                return;
            }
        };

        let mut rx = self.bus.subscribe();
        info!("HotkeyManager: listening on {}", HOTKEY_SOCKET);

        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok((stream, _addr)) => {
                            let bus = Arc::clone(&self.bus);
                            tokio::spawn(async move {
                                if let Err(e) = handle_connection(stream, bus).await {
                                    warn!("HotkeyManager: connection error: {}", e);
                                }
                            });
                        }
                        Err(e) => {
                            warn!("HotkeyManager: accept error: {}", e);
                        }
                    }
                }
                bus_event = rx.recv() => {
                    match bus_event {
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            info!("HotkeyManager: bus closed, shutting down");
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

async fn handle_connection(stream: tokio::net::UnixStream, bus: Arc<EventBus>) -> Result<()> {
    let reader = BufReader::new(stream);
    let mut lines = reader.lines();

    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }

        let intent = if line.starts_with('{') {
            match serde_json::from_str::<serde_json::Value>(&line) {
                Ok(json) => json
                    .get("intent")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                Err(_) => line.trim().to_string(),
            }
        } else {
            line.trim().to_string()
        };

        info!("HotkeyManager: received intent '{}'", intent);

        let event = Event {
            id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            source: "hotkey_manager".to_string(),
            event_type: EventType::UserIntent,
            payload: serde_json::json!({
                "intent": intent,
                "trigger": "hotkey",
            }),
        };
        bus.publish(event);
    }

    Ok(())
}