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

/// Speaker volume and screen brightness as they are now, for the model,
/// which otherwise guesses.
pub async fn current_levels() -> Option<String> {
    let mut parts = Vec::new();
    let volume = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        Command::new("wpctl").args(["get-volume", "@DEFAULT_AUDIO_SINK@"]).output(),
    )
    .await;
    if let Ok(Ok(out)) = volume {
        // "Volume: 0.50" or "Volume: 0.50 [MUTED]"
        let text = String::from_utf8_lossy(&out.stdout);
        if let Some(v) = text.split_whitespace().nth(1).and_then(|v| v.parse::<f64>().ok()) {
            let muted = if text.contains("MUTED") { ", muted" } else { "" };
            parts.push(format!("speaker volume is {}%{}", (v * 100.0).round(), muted));
        }
    }
    if let Some((_, now, max)) = backlight().filter(|(_, _, max)| *max > 0) {
        parts.push(format!("screen brightness is {}%", now * 100 / max));
    }
    if let Some(battery) = battery() {
        parts.push(battery);
    }
    (!parts.is_empty()).then(|| {
        let mut line = parts.join(" and ");
        line.replace_range(..1, &line[..1].to_uppercase());
        format!("{}.", line)
    })
}

/// "the battery is at 64% and charging", from the first battery the kernel lists.
fn battery() -> Option<String> {
    let entries = std::fs::read_dir("/sys/class/power_supply").ok()?;
    for entry in entries.flatten() {
        let dir = entry.path();
        let read = |name: &str| std::fs::read_to_string(dir.join(name)).ok().map(|s| s.trim().to_string());
        if read("type").as_deref() != Some("Battery") {
            continue;
        }
        let capacity = read("capacity")?;
        let status = read("status").unwrap_or_default().to_lowercase();
        let state = match status.as_str() {
            "charging" | "discharging" | "full" => format!(" and {}", status),
            _ => String::new(),
        };
        return Some(format!("the battery is at {}%{}", capacity, state));
    }
    None
}

/// What the media players say is playing: "Title by Artist (in Player,
/// playing)", the playing one first. None when no player reports a title.
/// Err when playerctl, which asks them, isn't installed.
pub async fn now_playing() -> Result<Option<String>> {
    let run = |args: Vec<String>| async move {
        tokio::time::timeout(std::time::Duration::from_secs(2), Command::new("playerctl").args(args).output()).await
    };
    let listed = match run(vec!["-l".into()]).await {
        Ok(Ok(out)) => out,
        Ok(Err(_)) => return Err(anyhow::anyhow!("playerctl isn't installed")),
        Err(_) => return Ok(None),
    };
    let mut found: Vec<(bool, String)> = Vec::new();
    for player in String::from_utf8_lossy(&listed.stdout).lines().map(str::trim).filter(|p| !p.is_empty()) {
        let format = "{{title}}\t{{artist}}\t{{status}}";
        let Ok(Ok(out)) = run(vec!["-p".into(), player.into(), "metadata".into(), "--format".into(), format.into()]).await else {
            continue;
        };
        let line = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let mut fields = line.split('\t');
        let title = fields.next().unwrap_or_default().trim();
        if !out.status.success() || title.is_empty() {
            continue;
        }
        let artist = fields.next().unwrap_or_default().trim();
        let status = fields.next().unwrap_or_default().trim().to_lowercase();
        let by = if artist.is_empty() { String::new() } else { format!(" by {}", artist) };
        let name = player.split('.').next().unwrap_or(player);
        found.push((status == "playing", format!("{}{} (in {}, {})", title, by, name, status)));
    }
    // Playing before paused or stopped.
    found.sort_by_key(|(playing, _)| !playing);
    Ok((!found.is_empty()).then(|| found.into_iter().map(|(_, f)| f).collect::<Vec<_>>().join("; ")))
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