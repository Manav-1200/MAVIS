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
        "brightness_up" => brightness(&["+10%"], Step::Up).await,
        "brightness_down" => brightness(&["10%-"], Step::Down).await,
        "brightness_set" => {
            let pct = level.ok_or_else(|| anyhow::anyhow!("no brightness level given"))?.min(100);
            brightness(&[&format!("{}%", pct)], Step::To(pct)).await
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

enum Step {
    Up,
    Down,
    To(u64),
}

/// brightnessctl if it's there; otherwise systemd-logind, which lets the
/// logged-in user set the screen's brightness with nothing else installed.
async fn brightness(args: &[&str], step: Step) -> Result<String> {
    let mut command = vec!["brightnessctl", "set"];
    command.extend_from_slice(args);
    match first_of(&[&command]).await {
        Err(e) if e.to_string().ends_with("isn't installed") => {
            let Some((device, now, max)) = backlight() else { return Err(e) };
            let tenth = (max / 10).max(1);
            let target = match step {
                Step::Up => (now + tenth).min(max),
                // Never quite off: a black screen can't be turned back up by eye.
                Step::Down => now.saturating_sub(tenth).max(max / 20),
                Step::To(pct) => (max * pct / 100).max(max / 20),
            };
            let value = target.to_string();
            first_of(&[&[
                "busctl", "call", "org.freedesktop.login1", "/org/freedesktop/login1/session/auto",
                "org.freedesktop.login1.Session", "SetBrightness", "ssu", "backlight", &device, &value,
            ]])
            .await
        }
        other => other,
    }
}

/// The first backlight: its name, brightness and maximum.
fn backlight() -> Option<(String, u64, u64)> {
    let entry = std::fs::read_dir("/sys/class/backlight").ok()?.flatten().next()?;
    let read = |f: &str| std::fs::read_to_string(entry.path().join(f)).ok()?.trim().parse::<u64>().ok();
    Some((entry.file_name().to_string_lossy().to_string(), read("brightness")?, read("max_brightness")?))
}

// ---------------------------------------------------------------------
// Media players, over D-Bus (MPRIS) with busctl, which comes with systemd.
// ---------------------------------------------------------------------

const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";
const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
const MPRIS_PLAYER: &str = "org.mpris.MediaPlayer2.Player";

async fn busctl(args: &[&str]) -> Result<std::process::Output> {
    let run = Command::new("busctl").arg("--user").args(args).output();
    match tokio::time::timeout(std::time::Duration::from_secs(2), run).await {
        Ok(Ok(out)) => Ok(out),
        Ok(Err(_)) => Err(anyhow::anyhow!("busctl isn't installed, so media players can't be reached")),
        Err(_) => Err(anyhow::anyhow!("the media players didn't answer in time")),
    }
}

/// Every media player on the session bus, by its bus name.
pub async fn media_players() -> Result<Vec<String>> {
    let out = busctl(&["list", "--no-legend", "--no-pager"]).await?;
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter(|n| n.starts_with(MPRIS_PREFIX))
        .map(String::from)
        .collect())
}

/// "Playing", "Paused" or "Stopped".
pub async fn player_status(player: &str) -> Option<String> {
    let out = busctl(&["get-property", player, MPRIS_PATH, MPRIS_PLAYER, "PlaybackStatus"]).await.ok()?;
    // `s "Playing"`
    let text = String::from_utf8_lossy(&out.stdout);
    text.split('"').nth(1).map(String::from)
}

/// Ask a player to Pause, Play, PlayPause, Next or Previous.
pub async fn player_call(player: &str, method: &str) -> bool {
    busctl(&["call", player, MPRIS_PATH, MPRIS_PLAYER, method]).await.is_ok_and(|o| o.status.success())
}

/// A player's name as people say it: "org.mpris.MediaPlayer2.vlc.instance42" → "vlc".
fn player_name(player: &str) -> &str {
    let name = player.trim_start_matches(MPRIS_PREFIX);
    name.split('.').next().unwrap_or(name)
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

/// Play, pause, skip: sent to the player that's playing, or the first one.
async fn media_control(action: &str) -> Result<String> {
    let method = match action {
        "play-pause" => "PlayPause",
        "next" => "Next",
        "previous" => "Previous",
        "pause" => "Pause",
        "play" => "Play",
        other => return Err(anyhow::anyhow!("no media action called {}", other)),
    };
    let players = media_players().await?;
    let mut target = None;
    for player in &players {
        if player_status(player).await.as_deref() == Some("Playing") {
            target = Some(player);
            break;
        }
    }
    let Some(player) = target.or(players.first()) else {
        return Err(anyhow::anyhow!("no media player is open"));
    };
    if player_call(player, method).await {
        Ok(format!("{} sent to {}", method, player_name(player)))
    } else {
        Err(anyhow::anyhow!("{} didn't accept {}", player_name(player), method))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_player_is_named_as_people_say_it() {
        assert_eq!(player_name("org.mpris.MediaPlayer2.tunes.instance42"), "tunes");
    }
}