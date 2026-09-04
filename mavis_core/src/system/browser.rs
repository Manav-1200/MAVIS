// mavis_core/src/system/browser.rs
// Browser tab/URL awareness via Unix domain socket. Mirrors hotkeys.rs's
// pattern exactly. The native messaging host (separate piece, browser-side)
// sends one JSON line per tab change: {"url": "...", "title": "..."}.
// Domain is derived here in Rust, not trusted from the extension, since
// it's simple string parsing and keeps the extension side dumb.

use crate::context_snapshot::BrowserTab;
use crate::event_bus::EventBus;
use crate::models::event::{Event, EventType};
use anyhow::Result;
use log::{info, warn};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::UnixListener;

const BROWSER_SOCKET: &str = "/tmp/mavis_browser.sock";

pub struct BrowserManager {
    bus: Arc<EventBus>,
}

impl BrowserManager {
    pub fn new(bus: Arc<EventBus>) -> Self {
        Self { bus }
    }

    pub async fn run(&mut self) {
        let _ = tokio::fs::remove_file(BROWSER_SOCKET).await;

        let listener = match UnixListener::bind(BROWSER_SOCKET) {
            Ok(l) => l,
            Err(e) => {
                warn!("BrowserManager: failed to bind socket at {}: {}", BROWSER_SOCKET, e);
                return;
            }
        };

        let mut rx = self.bus.subscribe();
        info!("BrowserManager: listening on {}", BROWSER_SOCKET);

        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok((stream, _addr)) => {
                            let bus = Arc::clone(&self.bus);
                            tokio::spawn(async move {
                                if let Err(e) = handle_connection(stream, bus).await {
                                    warn!("BrowserManager: connection error: {}", e);
                                }
                            });
                        }
                        Err(e) => {
                            warn!("BrowserManager: accept error: {}", e);
                        }
                    }
                }
                bus_event = rx.recv() => {
                    match bus_event {
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            info!("BrowserManager: bus closed, shutting down");
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

fn extract_domain(url: &str) -> String {
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    let end = without_scheme
        .find(['/', '?', '#'])
        .unwrap_or(without_scheme.len());
    without_scheme[..end].to_string()
}

async fn handle_connection(stream: tokio::net::UnixStream, bus: Arc<EventBus>) -> Result<()> {
    let reader = BufReader::new(stream);
    let mut lines = reader.lines();

    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }

        let parsed: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                warn!("BrowserManager: invalid JSON line: {}", e);
                continue;
            }
        };

        let url = parsed.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let title = parsed.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();

        if url.is_empty() {
            continue;
        }

        let domain = extract_domain(&url);
        info!("BrowserManager: tab update — {}", domain);

        let tab = BrowserTab { url, title, domain };
        let event = Event {
            id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            source: "browser_manager".to_string(),
            event_type: EventType::BrowserUpdate,
            payload: serde_json::to_value(&tab).unwrap_or(serde_json::Value::Null),
        };
        bus.publish(event);
    }

    Ok(())
}