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