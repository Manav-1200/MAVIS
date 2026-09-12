//! macOS platform provider — stub, compiles, returns errors gracefully.

use super::*;

pub struct MacOsProvider;

impl MacOsProvider {
    pub fn new() -> Self {
        Self
    }
}

struct MacOsWindowTracker;
struct MacOsClipboard;
struct MacOsScreen;

impl WindowTracker for MacOsWindowTracker {
    fn active_window(&self) -> Result<(String, String, u32), PlatformError> {
        Err(PlatformError("macOS window tracking not yet implemented".into()))
    }
    fn open_windows(&self) -> Result<Vec<WindowInfo>, PlatformError> {
        Err(PlatformError("macOS window tracking not yet implemented".into()))
    }
    fn subscribe_changes(&self) -> Result<tokio::sync::mpsc::Receiver<WindowEvent>, PlatformError> {
        Err(PlatformError("macOS window tracking not yet implemented".into()))
    }
}

impl ClipboardReader for MacOsClipboard {
    fn read_text(&self) -> Result<Option<String>, PlatformError> {
        Err(PlatformError("macOS clipboard not yet implemented".into()))
    }
    fn subscribe_changes(&self) -> Result<tokio::sync::mpsc::Receiver<String>, PlatformError> {
        Err(PlatformError("macOS clipboard not yet implemented".into()))
    }
}

impl ScreenGrabber for MacOsScreen {
    fn capture_focused(&self) -> Result<Screenshot, PlatformError> {
        Err(PlatformError("macOS screen capture not yet implemented".into()))
    }
}

impl PlatformProvider for MacOsProvider {
    fn installed_apps(&self) -> Vec<AppEntry> {
        scan_macos_apps()
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
// Installed application discovery (.app bundles)
// ---------------------------------------------------------------------------
//
// UNTESTED: written against the standard bundle layout, but nobody has run
// this on macOS yet. Treat as a starting point, not verified code.
//
// The bundle directory name minus ".app" is the name users say, and
// `open -a "<Name>"` is the documented way to launch by name.

fn scan_macos_apps() -> Vec<AppEntry> {
    let mut dirs: Vec<std::path::PathBuf> = vec![
        std::path::PathBuf::from("/Applications"),
        std::path::PathBuf::from("/System/Applications"),
        std::path::PathBuf::from("/Applications/Utilities"),
    ];
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(std::path::PathBuf::from(home).join("Applications"));
    }

    let mut apps = Vec::new();
    for dir in dirs {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("app") {
                continue;
            }
            if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                apps.push(AppEntry {
                    name: name.to_string(),
                    exec: format!("open -a \"{}\"", name),
                });
            }
        }
    }
    apps
}