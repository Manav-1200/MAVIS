//! Windows platform provider — stub, compiles, returns errors gracefully.

use super::*;

pub struct WindowsProvider;

impl WindowsProvider {
    pub fn new() -> Self {
        Self
    }
}

struct WindowsWindowTracker;
struct WindowsClipboard;
struct WindowsScreen;

impl WindowTracker for WindowsWindowTracker {
    fn active_window(&self) -> Result<(String, String, u32), PlatformError> {
        Err(PlatformError("Windows window tracking not yet implemented".into()))
    }
    fn open_windows(&self) -> Result<Vec<WindowInfo>, PlatformError> {
        Err(PlatformError("Windows window tracking not yet implemented".into()))
    }
    fn subscribe_changes(&self) -> Result<tokio::sync::mpsc::Receiver<WindowEvent>, PlatformError> {
        Err(PlatformError("Windows window tracking not yet implemented".into()))
    }
}

impl ClipboardReader for WindowsClipboard {
    fn read_text(&self) -> Result<Option<String>, PlatformError> {
        Err(PlatformError("Windows clipboard not yet implemented".into()))
    }
    fn subscribe_changes(&self) -> Result<tokio::sync::mpsc::Receiver<String>, PlatformError> {
        Err(PlatformError("Windows clipboard not yet implemented".into()))
    }
}

impl ScreenGrabber for WindowsScreen {
    fn capture_focused(&self) -> Result<Screenshot, PlatformError> {
        Err(PlatformError("Windows screen capture not yet implemented".into()))
    }
}

impl PlatformProvider for WindowsProvider {
    fn installed_apps(&self) -> Vec<AppEntry> {
        scan_windows_apps()
    }
    fn audio(&self) -> Option<&dyn AudioCapture> {
        None
    }
    fn windows(&self) -> Option<&dyn WindowTracker> {
        None
    }
    fn clipboard(&self) -> Option<&dyn ClipboardReader> {
        None
    }
    fn screen(&self) -> Option<&dyn ScreenGrabber> {
        None
    }
}
// ---------------------------------------------------------------------------
// Installed application discovery (Start Menu shortcuts)
// ---------------------------------------------------------------------------
//
// UNTESTED: written against documented Start Menu layout, but nobody has
// run this on Windows yet. Treat as a starting point, not verified code.
//
// No .lnk parsing needed: the shortcut filename is the app name users see,
// and `cmd /c start "" "<path.lnk>"` lets the shell resolve the target,
// which also preserves working directory and arguments baked into it.

fn scan_windows_apps() -> Vec<AppEntry> {
    let mut roots: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(pd) = std::env::var("ProgramData") {
        roots.push(
            std::path::PathBuf::from(pd).join("Microsoft\\Windows\\Start Menu\\Programs"),
        );
    }
    if let Ok(ad) = std::env::var("APPDATA") {
        roots.push(
            std::path::PathBuf::from(ad).join("Microsoft\\Windows\\Start Menu\\Programs"),
        );
    }

    let mut apps = Vec::new();
    for root in roots {
        collect_shortcuts(&root, &mut apps, 0);
    }
    apps
}

/// Start Menu entries are nested in vendor subfolders, so recurse — but
/// bounded, to avoid pathological trees.
fn collect_shortcuts(dir: &std::path::Path, out: &mut Vec<AppEntry>, depth: u8) {
    if depth > 3 {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_shortcuts(&path, out, depth + 1);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("lnk") {
            continue;
        }
        let name = match path.file_stem().and_then(|s| s.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        // Uninstallers and help links clutter the Start Menu; they are not
        // things a user means when they say "open X".
        let lower = name.to_lowercase();
        if lower.contains("uninstall") || lower.contains("readme") || lower.contains("help") {
            continue;
        }
        out.push(AppEntry {
            name,
            exec: format!("cmd /c start \"\" \"{}\"", path.to_string_lossy()),
        });
    }
}