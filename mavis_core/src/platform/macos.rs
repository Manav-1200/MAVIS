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
struct MacOsTts;

impl WindowTracker for MacOsWindowTracker {
    fn active_window(&self) -> Result<(String, String, u32), PlatformError> {
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

impl TtsPlayer for MacOsTts {
    fn speak(&self, _text: &str) -> Result<(), PlatformError> {
        Err(PlatformError("macOS TTS not yet implemented".into()))
    }
    fn stop(&self) -> Result<(), PlatformError> {
        Err(PlatformError("macOS TTS not yet implemented".into()))
    }
}

impl PlatformProvider for MacOsProvider {
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
    fn tts(&self) -> Option<&dyn TtsPlayer> {
        None
    }
}