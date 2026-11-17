// mavis_core/src/system/dbus.rs
// System controls: notifications, media (MPRIS), volume, brightness,
// Bluetooth, Wi-Fi, and the session (lock, log out, suspend, power off).
// Each control tries the usual tools in turn; none is required.

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
        // The executor runs its own actions and marks them done, so it can
        // say when one fails; this handles the rest (e.g. notifications).
        if event.event_type == EventType::SystemAction && event.payload.get("done").is_none() {
            if let Err(e) = run_system_op(&event.payload).await {
                warn!("DBusIntegration: system action failed: {}", e);
            }
        }
        Ok(())
    }
}

/// Carry out one system action. The error says what went wrong in words
/// fit to be spoken ("brightnessctl isn't installed").
pub async fn run_system_op(payload: &serde_json::Value) -> Result<String> {
    let op = payload.get("op").and_then(|v| v.as_str()).unwrap_or("unknown");
    let level = payload.get("value").and_then(|v| v.as_u64()).map(|v| v.min(150));
    let session = std::env::var("XDG_SESSION_ID").unwrap_or_else(|_| "self".into());

    match op {
        "notify" => {
            let title = payload.get("title").and_then(|v| v.as_str()).unwrap_or("MAVIS");
            let message = payload.get("message").and_then(|v| v.as_str()).unwrap_or("");
            send_notification(title, message).await
        }
        "media_play_pause" => media_control("play-pause").await,
        "media_pause" => media_control("pause").await,
        "media_play" => media_control("play").await,
        "media_next" => media_control("next").await,
        "media_previous" => media_control("previous").await,
        "volume_up" => first_of(&[&["pactl", "set-sink-volume", "@DEFAULT_SINK@", "+5%"], &["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%+"]]).await,
        "volume_down" => first_of(&[&["pactl", "set-sink-volume", "@DEFAULT_SINK@", "-5%"], &["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%-"]]).await,
        "volume_mute" => first_of(&[&["pactl", "set-sink-mute", "@DEFAULT_SINK@", "toggle"], &["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]]).await,
        "volume_set" => {
            let pct = format!("{}%", level.ok_or_else(|| anyhow::anyhow!("no volume level given"))?);
            first_of(&[&["pactl", "set-sink-volume", "@DEFAULT_SINK@", &pct], &["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", &pct]]).await
        }
        "brightness_up" => first_of(&[&["brightnessctl", "set", "+10%"]]).await,
        "brightness_down" => first_of(&[&["brightnessctl", "set", "10%-"]]).await,
        "brightness_set" => {
            let pct = format!("{}%", level.ok_or_else(|| anyhow::anyhow!("no brightness level given"))?.min(100));
            first_of(&[&["brightnessctl", "set", &pct]]).await
        }
        "bluetooth_on" => first_of(&[&["bluetoothctl", "power", "on"], &["rfkill", "unblock", "bluetooth"]]).await,
        "bluetooth_off" => first_of(&[&["bluetoothctl", "power", "off"], &["rfkill", "block", "bluetooth"]]).await,
        "wifi_on" => first_of(&[&["nmcli", "radio", "wifi", "on"], &["rfkill", "unblock", "wifi"]]).await,
        "wifi_off" => first_of(&[&["nmcli", "radio", "wifi", "off"], &["rfkill", "block", "wifi"]]).await,
        "lock" => first_of(&[&["loginctl", "lock-session"]]).await,
        "logout" => first_of(&[&["loginctl", "terminate-session", &session]]).await,
        "suspend" => first_of(&[&["systemctl", "suspend"]]).await,
        "poweroff" => first_of(&[&["systemctl", "poweroff"]]).await,
        "reboot" => first_of(&[&["systemctl", "reboot"]]).await,
        other => {
            warn!("DBusIntegration: unknown system op '{}'", other);
            Err(anyhow::anyhow!("I don't know how to do {} yet", other.replace('_', " ")))
        }
    }
}

/// Run the first of these commands that's installed. A missing tool moves
/// on to the next; one that runs and fails is the answer.
async fn first_of(commands: &[&[&str]]) -> Result<String> {
    for command in commands {
        let (program, args) = command.split_first().expect("a command");
        match Command::new(program).args(args).output().await {
            Ok(out) if out.status.success() => return Ok(format!("{} {}", program, args.join(" "))),
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let reason = stderr.lines().next().unwrap_or("").trim();
                return Err(anyhow::anyhow!("{} failed{}", program, if reason.is_empty() { String::new() } else { format!(": {}", reason) }));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(anyhow::anyhow!("{} couldn't start: {}", program, e)),
        }
    }
    let names: Vec<&str> = commands.iter().map(|c| c[0]).collect();
    Err(anyhow::anyhow!("{} isn't installed", names.join(" or ")))
}

async fn send_notification(title: &str, body: &str) -> Result<String> {
    first_of(&[&["notify-send", title, body]]).await
}

async fn media_control(action: &str) -> Result<String> {
    match Command::new("playerctl").arg(action).output().await {
        Ok(o) if o.status.success() => Ok(format!("Media {} via playerctl", action)),
        _ => {
            let method = match action {
                "play-pause" => "PlayPause",
                "next" => "Next",
                "previous" => "Previous",
                "pause" => "Pause",
                "play" => "Play",
                _ => action,
            };
            let member = format!("org.mpris.MediaPlayer2.Player.{}", method);
            first_of(&[&["dbus-send", "--type=method_call", "--dest=org.mpris.MediaPlayer2", "/org/mpris/MediaPlayer2", &member]]).await
        }
    }
}