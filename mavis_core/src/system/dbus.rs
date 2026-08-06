// mavis_core/src/system/dbus.rs
// DBus integration: handles SystemAction events for notifications,
// media control (MPRIS), brightness, and volume.

use crate::event_bus::EventBus;
use crate::models::event::{Event, EventType};
use anyhow::Result;
use log::{info, warn};
use std::sync::Arc;
use tokio::process::Command;

pub struct DbusIntegration {
    bus: Arc<EventBus>,
}

impl DbusIntegration {
    pub fn new(bus: Arc<EventBus>) -> Self {
        Self { bus }
    }

    pub async fn run(&mut self) {
        let mut rx = self.bus.subscribe();
        info!("DBusIntegration: listening for events");
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Err(e) = self.handle_event(event).await {
                        warn!("DBusIntegration error: {}", e);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    warn!("DBusIntegration lagged by {} events", n);
                }
            }
        }
        info!("DBusIntegration: shutting down");
    }

    async fn handle_event(&self, event: Event) -> Result<()> {
        if event.event_type == EventType::SystemAction {
            if let Err(e) = self.execute_system_action(&event.payload).await {
                warn!("DBusIntegration: system action failed: {}", e);
            }
        }
        Ok(())
    }

    async fn execute_system_action(&self, payload: &serde_json::Value) -> Result<String> {
        let op = payload.get("op").and_then(|v| v.as_str()).unwrap_or("unknown");

        match op {
            "notify" => {
                let title = payload.get("title").and_then(|v| v.as_str()).unwrap_or("MAVIS");
                let message = payload.get("message").and_then(|v| v.as_str()).unwrap_or("");
                self.send_notification(title, message).await
            }
            "media_play_pause" => self.media_control("play-pause").await,
            "media_next" => self.media_control("next").await,
            "media_previous" => self.media_control("previous").await,
            "brightness_up" => self.brightness_control("up").await,
            "brightness_down" => self.brightness_control("down").await,
            "volume_up" => self.volume_control("up").await,
            "volume_down" => self.volume_control("down").await,
            "volume_mute" => self.volume_control("mute").await,
            other => {
                warn!("DBusIntegration: unknown system op '{}'", other);
                Err(anyhow::anyhow!("unknown system op: {}", other))
            }
        }
    }

    async fn send_notification(&self, title: &str, body: &str) -> Result<String> {
        let output = Command::new("notify-send")
            .arg(title)
            .arg(body)
            .output()
            .await
            .map_err(|e| anyhow::anyhow!("notify-send failed: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("notify-send error: {}", stderr.trim()));
        }
        Ok(format!("Notification: {} — {}", title, body))
    }

    async fn media_control(&self, action: &str) -> Result<String> {
        let output = Command::new("playerctl")
            .arg(action)
            .output()
            .await;

        match output {
            Ok(o) if o.status.success() => Ok(format!("Media {} via playerctl", action)),
            _ => {
                let method = match action {
                    "play-pause" => "PlayPause",
                    "next" => "Next",
                    "previous" => "Previous",
                    _ => action,
                };
                let output = Command::new("dbus-send")
                    .args(&[
                        "--type=method_call",
                        "--dest=org.mpris.MediaPlayer2",
                        "/org/mpris/MediaPlayer2",
                        &format!("org.mpris.MediaPlayer2.Player.{}", method),
                    ])
                    .output()
                    .await
                    .map_err(|e| anyhow::anyhow!("media control failed: {}", e))?;

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Err(anyhow::anyhow!("dbus-send error: {}", stderr.trim()));
                }
                Ok(format!("Media {} via dbus-send", action))
            }
        }
    }

    async fn brightness_control(&self, direction: &str) -> Result<String> {
        let cmd = match direction {
            "up" => "brightnessctl set +10%",
            "down" => "brightnessctl set 10%-",
            _ => return Err(anyhow::anyhow!("invalid brightness direction: {}", direction)),
        };

        let output = Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .output()
            .await
            .map_err(|e| anyhow::anyhow!("brightnessctl failed: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("brightnessctl error: {}", stderr.trim()));
        }
        Ok(format!("Brightness {}", direction))
    }

    async fn volume_control(&self, action: &str) -> Result<String> {
        let cmd = match action {
            "up" => "pactl set-sink-volume @DEFAULT_SINK@ +5%",
            "down" => "pactl set-sink-volume @DEFAULT_SINK@ -5%",
            "mute" => "pactl set-sink-mute @DEFAULT_SINK@ toggle",
            _ => return Err(anyhow::anyhow!("invalid volume action: {}", action)),
        };

        let output = Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .output()
            .await
            .map_err(|e| anyhow::anyhow!("volume control failed: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("volume control error: {}", stderr.trim()));
        }
        Ok(format!("Volume {}", action))
    }
}