//! Linux platform — Wayland / X11 auto-detect
//!
//! Window tracking: tries niri, sway, hyprland, then xdotool.
//! Clipboard: wl-paste (Wayland) or xclip (X11).
//! Screen: grim (Wayland) or import (X11).

use super::*;
use log::{info, warn};
use serde_json::Value;
use std::process::Command;
use tokio::sync::mpsc;
use tokio::time::{interval, Duration};

pub struct LinuxProvider {
    windows: Option<LinuxWindowTracker>,
    clipboard: Option<LinuxClipboard>,
    screen: Option<LinuxScreen>,
}

impl LinuxProvider {
    pub fn new() -> Self {
        let wayland = std::env::var("WAYLAND_DISPLAY").is_ok();
        let x11 = std::env::var("DISPLAY").is_ok();
        info!("LinuxProvider: wayland={}, x11={}", wayland, x11);

        Self {
            windows: if wayland || x11 {
                Some(LinuxWindowTracker::new(wayland))
            } else {
                warn!("No display server; window tracking disabled");
                None
            },
            clipboard: if wayland || x11 {
                Some(LinuxClipboard::new(wayland))
            } else {
                None
            },
            screen: if wayland || x11 {
                Some(LinuxScreen::new(wayland))
            } else {
                None
            },
        }
    }
}

impl PlatformProvider for LinuxProvider {
    fn audio(&self) -> Option<&dyn AudioCapture> {
        None
    }
    fn windows(&self) -> Option<&dyn WindowTracker> {
        self.windows.as_ref().map(|w| w as &dyn WindowTracker)
    }
    fn clipboard(&self) -> Option<&dyn ClipboardReader> {
        self.clipboard.as_ref().map(|c| c as &dyn ClipboardReader)
    }
    fn screen(&self) -> Option<&dyn ScreenGrabber> {
        self.screen.as_ref().map(|s| s as &dyn ScreenGrabber)
    }
}

// ---------------------------------------------------------------------------
// Window Tracker
// ---------------------------------------------------------------------------

struct LinuxWindowTracker {
    wayland: bool,
}

impl LinuxWindowTracker {
    fn new(wayland: bool) -> Self {
        Self { wayland }
    }

    fn run_cmd(args: &[&str]) -> Option<String> {
        Command::new(args[0])
            .args(&args[1..])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
    }

    fn parse_niri_windows(json: &str) -> Option<(String, String, u32)> {
        let v: Value = serde_json::from_str(json).ok()?;
        let wins = v.as_array()?;
        for w in wins {
            if w.get("is_focused")?.as_bool()? {
                let title = w.get("title")?.as_str()?.to_string();
                let app = w.get("app_id")?.as_str().unwrap_or("unknown").to_string();
                let pid = w.get("pid")?.as_u64()? as u32;
                return Some((app, title, pid));
            }
        }
        None
    }

    /// Same niri call as above, but keeps every window instead of only the
    /// focused one — niri already returns the full list (with workspace and
    /// focus flags), we were discarding it.
    fn parse_niri_all_windows(json: &str) -> Option<Vec<WindowInfo>> {
        let v: Value = serde_json::from_str(json).ok()?;
        let wins = v.as_array()?;
        let mut out = Vec::new();
        for w in wins {
            out.push(WindowInfo {
                app_name: w
                    .get("app_id")
                    .and_then(|a| a.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                window_title: w.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string(),
                pid: w.get("pid").and_then(|p| p.as_u64()).map(|p| p as u32),
                workspace_id: w.get("workspace_id").and_then(|x| x.as_u64()),
                is_focused: w.get("is_focused").and_then(|x| x.as_bool()).unwrap_or(false),
            });
        }
        Some(out)
    }

    fn parse_sway_tree(json: &str) -> Option<(String, String, u32)> {
        let v: Value = serde_json::from_str(json).ok()?;
        Self::sway_find_focused(&v)
    }

    fn sway_find_focused(v: &Value) -> Option<(String, String, u32)> {
        if v.get("focused")?.as_bool()? {
            let app = v.get("app_id")?.as_str()?.to_string();
            let title = v.get("name")?.as_str()?.to_string();
            let pid = v.get("pid")?.as_u64()? as u32;
            return Some((app, title, pid));
        }
        for node in v.get("nodes")?.as_array()? {
            if let Some(r) = Self::sway_find_focused(node) {
                return Some(r);
            }
        }
        for node in v.get("floating_nodes")?.as_array()? {
            if let Some(r) = Self::sway_find_focused(node) {
                return Some(r);
            }
        }
        None
    }

    fn parse_hypr_active(json: &str) -> Option<(String, String, u32)> {
        let v: Value = serde_json::from_str(json).ok()?;
        let app = v.get("class")?.as_str()?.to_string();
        let title = v.get("title")?.as_str()?.to_string();
        let pid = v.get("pid")?.as_u64()? as u32;
        Some((app, title, pid))
    }

    fn xdotool_active() -> Option<(String, String, u32)> {
        let id = Self::run_cmd(&["xdotool", "getactivewindow"])?;
        let title = Self::run_cmd(&["xdotool", "getwindowname", &id]).unwrap_or_default();
        let pid = Self::run_cmd(&["xdotool", "getwindowpid", &id])
            .and_then(|s| s.parse().ok())?;
        Some(("unknown".to_string(), title, pid))
    }
}

impl WindowTracker for LinuxWindowTracker {
    fn active_window(&self) -> Result<(String, String, u32), PlatformError> {
        if self.wayland {
            if let Some(json) = Self::run_cmd(&["niri", "msg", "--json", "windows"]) {
                if let Some(r) = Self::parse_niri_windows(&json) {
                    return Ok(r);
                }
            }
            if let Some(json) = Self::run_cmd(&["swaymsg", "-t", "get_tree"]) {
                if let Some(r) = Self::parse_sway_tree(&json) {
                    return Ok(r);
                }
            }
            if let Some(json) = Self::run_cmd(&["hyprctl", "activewindow", "-j"]) {
                if let Some(r) = Self::parse_hypr_active(&json) {
                    return Ok(r);
                }
            }
        }

        if let Some(r) = Self::xdotool_active() {
            return Ok(r);
        }

        Err(PlatformError("No window tracker available for this compositor".into()))
    }

    fn open_windows(&self) -> Result<Vec<WindowInfo>, PlatformError> {
        if self.wayland {
            if let Some(json) = Self::run_cmd(&["niri", "msg", "--json", "windows"]) {
                if let Some(list) = Self::parse_niri_all_windows(&json) {
                    if !list.is_empty() {
                        return Ok(list);
                    }
                }
            }
        }
        // Other compositors don't have an equivalent full-list parser yet;
        // fall back to the focused window so callers still get something
        // rather than an error.
        self.active_window().map(|(app, title, pid)| {
            vec![WindowInfo {
                app_name: app,
                window_title: title,
                pid: Some(pid),
                workspace_id: None,
                is_focused: true,
            }]
        })
    }

    /// Resolve the project for a window by walking its process tree.
    /// Verified against real /proc output: a GUI editor's own cwd is
    /// usually just $HOME, but its shell children sit in the real project
    /// directory. Browser subprocesses report junk paths under /proc/*,
    /// which the filter below discards.
    fn current_project(&self, pid: u32) -> Option<ProjectInfo> {
        let home = std::env::var("HOME").unwrap_or_default();
        let mut candidates = vec![pid];
        candidates.extend(collect_descendants(pid, 0));

        for p in candidates {
            let cwd = match std::fs::read_link(format!("/proc/{}/cwd", p)) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let cwd_str = cwd.to_string_lossy().to_string();
            if cwd_str.starts_with("/proc/") || cwd_str == home || cwd_str == "/" {
                continue;
            }
            if let Some(root) = find_git_root(&cwd) {
                let name = root
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| root.to_string_lossy().to_string());
                return Some(ProjectInfo {
                    name,
                    path: root.to_string_lossy().to_string(),
                    git_branch: read_git_branch(&root),
                });
            }
        }
        None
    }

    fn subscribe_changes(&self) -> Result<mpsc::Receiver<WindowEvent>, PlatformError> {
        let (tx, rx) = mpsc::channel(32);
        let tracker = LinuxWindowTracker::new(self.wayland);
        let mut last = String::new();

        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_millis(500));
            loop {
                ticker.tick().await;
                match tracker.active_window() {
                    Ok((app, title, pid)) => {
                        let key = format!("{}:{}:{}", app, title, pid);
                        if key != last {
                            last = key;
                            let _ = tx.send(WindowEvent { app_name: app, window_title: title, pid }).await;
                        }
                    }
                    Err(_) => {}
                }
            }
        });

        Ok(rx)
    }
}

// ---------------------------------------------------------------------------
// Clipboard
// ---------------------------------------------------------------------------

struct LinuxClipboard {
    wayland: bool,
}

impl LinuxClipboard {
    fn new(wayland: bool) -> Self {
        Self { wayland }
    }

    fn read_cmd(&self) -> Option<String> {
        if self.wayland {
            LinuxWindowTracker::run_cmd(&["wl-paste", "--no-newline"])
        } else {
            LinuxWindowTracker::run_cmd(&["xclip", "-selection", "clipboard", "-o"])
        }
    }
}

impl ClipboardReader for LinuxClipboard {
    fn read_text(&self) -> Result<Option<String>, PlatformError> {
        match self.read_cmd() {
            Some(t) if !t.is_empty() => Ok(Some(t)),
            _ => Ok(None),
        }
    }

    fn subscribe_changes(&self) -> Result<mpsc::Receiver<String>, PlatformError> {
        let (tx, rx) = mpsc::channel(16);
        let cb = LinuxClipboard::new(self.wayland);
        let mut last = String::new();

        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_millis(500));
            loop {
                ticker.tick().await;
                if let Some(text) = cb.read_cmd() {
                    if text != last && !text.is_empty() {
                        last = text.clone();
                        let _ = tx.send(text).await;
                    }
                }
            }
        });

        Ok(rx)
    }
}

// ---------------------------------------------------------------------------
// Screen Grabber
// ---------------------------------------------------------------------------

struct LinuxScreen {
    wayland: bool,
}

impl LinuxScreen {
    fn new(wayland: bool) -> Self {
        Self { wayland }
    }
}

impl ScreenGrabber for LinuxScreen {
    fn capture_focused(&self) -> Result<Screenshot, PlatformError> {
        let tmp = "/tmp/mavis_screenshot.png";
        if self.wayland {
            std::process::Command::new("grim")
                .arg(tmp)
                .status()
                .map_err(|e| PlatformError(format!("grim failed: {}", e)))?;
        } else {
            std::process::Command::new("import")
                .args(&["-window", "root", tmp])
                .status()
                .map_err(|e| PlatformError(format!("import (ImageMagick) failed: {}", e)))?;
        }

        let data = std::fs::read(tmp).map_err(|e| PlatformError(format!("failed to read screenshot: {}", e)))?;
        let (width, height) = parse_png_dimensions(&data).unwrap_or((0, 0));
        Ok(Screenshot { width, height, data })
    }
}

fn parse_png_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    if data.len() < 24 || &data[0..8] != &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] {
        return None;
    }
    let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
    let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
    Some((w, h))
}

// ---------------------------------------------------------------------------
// Project detection helpers (Linux /proc)
// ---------------------------------------------------------------------------

/// Direct children of `pid`, read from /proc/*/stat. The stat format puts
/// the (possibly space-containing) comm field in parens, so we split on the
/// LAST ") " to get at the fields after it; ppid is the 2nd field there.
fn collect_children(pid: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir("/proc") {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let child_pid: u32 = match name.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };
        let stat = match std::fs::read_to_string(format!("/proc/{}/stat", child_pid)) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if let Some(after) = stat.rsplit_once(") ") {
            let mut fields = after.1.split_whitespace();
            let _state = fields.next();
            if let Some(ppid) = fields.next().and_then(|p| p.parse::<u32>().ok()) {
                if ppid == pid {
                    out.push(child_pid);
                }
            }
        }
    }
    out
}

/// Descendants up to 3 levels deep — enough to reach a shell inside a
/// terminal or IDE without scanning unbounded process trees every poll.
fn collect_descendants(pid: u32, depth: u8) -> Vec<u32> {
    if depth >= 3 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for child in collect_children(pid) {
        out.push(child);
        out.extend(collect_descendants(child, depth + 1));
    }
    out
}

/// Walk upward from `start` looking for a .git directory.
fn find_git_root(start: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut current = start.to_path_buf();
    for _ in 0..20 {
        if current.join(".git").is_dir() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
    None
}

/// Branch name from .git/HEAD, or a short commit hash if detached.
fn read_git_branch(git_root: &std::path::Path) -> Option<String> {
    let head = std::fs::read_to_string(git_root.join(".git").join("HEAD")).ok()?;
    let head = head.trim();
    if let Some(branch) = head.strip_prefix("ref: refs/heads/") {
        Some(branch.to_string())
    } else {
        Some(head.chars().take(8).collect())
    }
}