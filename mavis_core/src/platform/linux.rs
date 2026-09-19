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
    fn installed_apps(&self) -> Vec<AppEntry> {
        scan_linux_apps()
    }
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

/// Which window-query tool actually works here. Resolved once at startup:
/// previously every poll spawned `niri`, then `swaymsg`, then `hyprctl`,
/// then up to three `xdotool` processes, most of which fail on any given
/// desktop. On GNOME that was a dozen doomed process spawns every 2 seconds.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Compositor {
    Niri,
    Sway,
    Hyprland,
    X11,
    None,
}

struct LinuxWindowTracker {
    compositor: Compositor,
}

impl LinuxWindowTracker {
    fn new(wayland: bool) -> Self {
        let compositor = Self::detect(wayland);
        info!("LinuxWindowTracker: using {:?}", compositor);
        Self { compositor }
    }

    /// Probe each backend once, at startup.
    ///
    /// Environment variables are used only to decide what to *try first* —
    /// never as proof. `NIRI_SOCKET` and friends leak into other sessions
    /// (observed: a GNOME login inheriting `NIRI_SOCKET`, which made MAVIS
    /// commit to niri and then fail every poll forever). Each candidate is
    /// confirmed by actually running it.
    fn detect(wayland: bool) -> Compositor {
        if wayland {
            // Order the attempts by what the environment hints at, then
            // verify. A hint that turns out wrong just costs one failed
            // spawn at startup rather than one every 2 seconds thereafter.
            let mut candidates: Vec<Compositor> = Vec::new();
            if std::env::var("NIRI_SOCKET").is_ok() {
                candidates.push(Compositor::Niri);
            }
            if std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok() {
                candidates.push(Compositor::Hyprland);
            }
            if std::env::var("SWAYSOCK").is_ok() {
                candidates.push(Compositor::Sway);
            }
            for c in [Compositor::Niri, Compositor::Hyprland, Compositor::Sway] {
                if !candidates.contains(&c) {
                    candidates.push(c);
                }
            }

            for candidate in candidates {
                let works = match candidate {
                    Compositor::Niri => {
                        Self::run_cmd(&["niri", "msg", "--json", "windows"])
                            .filter(|o| o.starts_with('[') || o.starts_with('{'))
                            .is_some()
                    }
                    Compositor::Hyprland => {
                        Self::run_cmd(&["hyprctl", "activewindow", "-j"])
                            .filter(|o| o.starts_with('{'))
                            .is_some()
                    }
                    Compositor::Sway => Self::run_cmd(&["swaymsg", "-t", "get_tree"])
                        .filter(|o| o.starts_with('{'))
                        .is_some(),
                    _ => false,
                };
                if works {
                    return candidate;
                }
            }
        }

        if std::env::var("DISPLAY").is_ok()
            && Self::run_cmd(&["xdotool", "getactivewindow"])
                .filter(|o| !o.is_empty())
                .is_some()
        {
            return Compositor::X11;
        }

        // GNOME Wayland and other unsupported compositors land here. Window
        // tracking is simply unavailable rather than retried forever.
        warn!("No supported window tracker found; window context disabled");
        Compositor::None
    }

    /// Run a command and return its stdout, or None if it failed.
    ///
    /// Checks the exit status: previously a command that existed but errored
    /// (niri outside a niri session, say) returned `Some("")`, which reads
    /// as success to every caller.
    fn run_cmd(args: &[&str]) -> Option<String> {
        let output = Command::new(args[0]).args(&args[1..]).output().ok()?;
        if !output.status.success() {
            return None;
        }
        let s = String::from_utf8(output.stdout).ok()?.trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
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
        match self.compositor {
            Compositor::Niri => Self::run_cmd(&["niri", "msg", "--json", "windows"])
                .and_then(|j| Self::parse_niri_windows(&j)),
            Compositor::Sway => Self::run_cmd(&["swaymsg", "-t", "get_tree"])
                .and_then(|j| Self::parse_sway_tree(&j)),
            Compositor::Hyprland => Self::run_cmd(&["hyprctl", "activewindow", "-j"])
                .and_then(|j| Self::parse_hypr_active(&j)),
            Compositor::X11 => Self::xdotool_active(),
            Compositor::None => None,
        }
        .ok_or_else(|| PlatformError("No window tracker available".into()))
    }

    fn open_windows(&self) -> Result<Vec<WindowInfo>, PlatformError> {
        if self.compositor == Compositor::Niri {
            if let Some(json) = Self::run_cmd(&["niri", "msg", "--json", "windows"]) {
                if let Some(list) = Self::parse_niri_all_windows(&json) {
                    if !list.is_empty() {
                        return Ok(list);
                    }
                }
            }
        }
        if self.compositor == Compositor::None {
            return Ok(Vec::new());
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
        use std::sync::Mutex;
        use std::time::{Duration, Instant};
        // Even with a single /proc pass this is the most expensive thing in
        // the poll. The answer changes when the user switches window, not
        // every 2 s, so cache per-pid and refresh at most every 30 s.
        static CACHE: Mutex<Option<(u32, Instant, Option<ProjectInfo>)>> = Mutex::new(None);
        if let Ok(cache) = CACHE.lock() {
            if let Some((cached_pid, at, result)) = cache.as_ref() {
                if *cached_pid == pid && at.elapsed() < Duration::from_secs(30) {
                    return result.clone();
                }
            }
        }

        let home = std::env::var("HOME").unwrap_or_default();
        let tree = build_process_tree();
        let mut candidates = vec![pid];
        candidates.extend(descendants_from_tree(&tree, pid));

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
                let found = Some(ProjectInfo {
                    name,
                    path: root.to_string_lossy().to_string(),
                    git_branch: read_git_branch(&root),
                });
                if let Ok(mut cache) = CACHE.lock() {
                    *cache = Some((pid, Instant::now(), found.clone()));
                }
                return found;
            }
        }
        // Cache the negative too — otherwise a browser window (which never
        // has a project) would redo the whole walk on every poll.
        if let Ok(mut cache) = CACHE.lock() {
            *cache = Some((pid, Instant::now(), None));
        }
        None
    }

    fn subscribe_changes(&self) -> Result<mpsc::Receiver<WindowEvent>, PlatformError> {
        let (tx, rx) = mpsc::channel(32);
        let tracker = LinuxWindowTracker { compositor: self.compositor };
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

/// Snapshot of every process's parent, built from one pass over /proc.
///
/// This replaces a recursive `collect_children` that rescanned all of /proc
/// for each descendant: at depth 3 with a few hundred processes that meant
/// well over a hundred full directory walks and tens of thousands of file
/// reads — every 2 seconds. It was survivable on a light compositor and
/// took down a GNOME session, which runs far more processes.
///
/// One scan, O(processes). The map is then walked in memory.
fn build_process_tree() -> std::collections::HashMap<u32, Vec<u32>> {
    let mut tree: std::collections::HashMap<u32, Vec<u32>> = std::collections::HashMap::new();
    let entries = match std::fs::read_dir("/proc") {
        Ok(e) => e,
        Err(_) => return tree,
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let pid: u32 = match name.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };
        let stat = match std::fs::read_to_string(format!("/proc/{}/stat", pid)) {
            Ok(s) => s,
            Err(_) => continue,
        };
        // The comm field can contain spaces and is parenthesised, so split
        // on the LAST ") " to reach the fields after it; ppid is the 2nd.
        if let Some((_, after)) = stat.rsplit_once(") ") {
            let mut fields = after.split_whitespace();
            let _state = fields.next();
            if let Some(ppid) = fields.next().and_then(|p| p.parse::<u32>().ok()) {
                tree.entry(ppid).or_default().push(pid);
            }
        }
    }
    tree
}

/// Descendants of `pid` from a prebuilt tree, breadth-first, bounded in both
/// depth and total count so a pathological process tree can't stall the poll.
fn descendants_from_tree(
    tree: &std::collections::HashMap<u32, Vec<u32>>,
    pid: u32,
) -> Vec<u32> {
    const MAX_DEPTH: u8 = 3;
    const MAX_NODES: usize = 64;

    let mut out = Vec::new();
    let mut frontier = vec![pid];
    for _ in 0..MAX_DEPTH {
        let mut next = Vec::new();
        for p in frontier {
            if let Some(children) = tree.get(&p) {
                for &c in children {
                    if out.len() >= MAX_NODES {
                        return out;
                    }
                    out.push(c);
                    next.push(c);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
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

// ---------------------------------------------------------------------------
// Installed application discovery (freedesktop .desktop files)
// ---------------------------------------------------------------------------

/// Drop freedesktop field codes (%U, %F, %i ...) from an Exec line.
fn strip_field_codes(exec: &str) -> String {
    exec.split_whitespace()
        .filter(|t| !(t.len() == 2 && t.starts_with('%')))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parse one .desktop file. Returns None for anything that isn't a
/// launchable application: hidden entries, non-Application types, and
/// sub-actions (the `[Desktop Action ...]` sections that give a browser
/// its "New Incognito Window" entries — those aren't separate apps).
fn parse_desktop_file(content: &str) -> Option<AppEntry> {
    let mut name: Option<String> = None;
    let mut exec: Option<String> = None;
    let mut in_entry = false;

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        if let Some(v) = line.strip_prefix("Name=") {
            if name.is_none() {
                name = Some(v.to_string());
            }
        } else if let Some(v) = line.strip_prefix("Exec=") {
            if exec.is_none() {
                exec = Some(v.to_string());
            }
        } else if let Some(v) = line.strip_prefix("NoDisplay=") {
            if v.eq_ignore_ascii_case("true") {
                return None;
            }
        } else if let Some(v) = line.strip_prefix("Hidden=") {
            if v.eq_ignore_ascii_case("true") {
                return None;
            }
        } else if let Some(v) = line.strip_prefix("Type=") {
            if v != "Application" {
                return None;
            }
        }
    }

    match (name, exec) {
        (Some(n), Some(e)) if !n.is_empty() && !e.is_empty() => Some(AppEntry {
            name: n,
            exec: strip_field_codes(&e),
        }),
        _ => None,
    }
}

fn scan_linux_apps() -> Vec<AppEntry> {
    // Read the spec-defined locations *in addition to* the usual defaults,
    // not instead of them. XDG_DATA_DIRS varies between desktop sessions and
    // is sometimes narrower than reality, so relying on it alone can find
    // fewer applications than a plain hardcoded list would.
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();

    if let Ok(data_dirs) = std::env::var("XDG_DATA_DIRS") {
        for d in data_dirs.split(':').filter(|d| !d.is_empty()) {
            dirs.push(std::path::PathBuf::from(d).join("applications"));
        }
    }
    if let Ok(data_home) = std::env::var("XDG_DATA_HOME") {
        dirs.push(std::path::PathBuf::from(data_home).join("applications"));
    }

    for fallback in [
        "/usr/share/applications",
        "/usr/local/share/applications",
        "/var/lib/flatpak/exports/share/applications",
        "/snap/bin",
    ] {
        dirs.push(std::path::PathBuf::from(fallback));
    }
    if let Ok(home) = std::env::var("HOME") {
        let home = std::path::PathBuf::from(&home);
        dirs.push(home.join(".local/share/applications"));
        dirs.push(home.join(".local/share/flatpak/exports/share/applications"));
        dirs.push(home.join(".nix-profile/share/applications"));
    }

    dirs.sort();
    dirs.dedup();

    let mut apps = Vec::new();
    // Dedupe by .desktop filename: the same app often appears in several
    // directories, and a user override should shadow the system copy.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for dir in &dirs {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            let stem = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };
            if !seen.insert(stem) {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Some(app) = parse_desktop_file(&content) {
                    apps.push(app);
                }
            }
        }
    }
    info!(
        "LinuxProvider: discovered {} applications across {} directories",
        apps.len(),
        dirs.len()
    );
    apps
}