//! macOS platform provider — stub, compiles, returns errors gracefully.

use super::*;
use anyhow::anyhow;

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
    fn active_window(&self) -> anyhow::Result<(String, String, u32)> {
        Err(anyhow!("macOS window tracking not yet implemented"))
    }
    fn subscribe(&self) -> anyhow::Result<tokio::sync::mpsc::Receiver<WindowEvent>> {
        Err(anyhow!("macOS window tracking not yet implemented"))
    }
}

impl ClipboardReader for MacOsClipboard {
    fn read_text(&self) -> anyhow::Result<Option<String>> {
        Err(anyhow!("macOS clipboard not yet implemented"))
    }
    fn subscribe(&self) -> anyhow::Result<tokio::sync::mpsc::Receiver<String>> {
        Err(anyhow!("macOS clipboard not yet implemented"))
    }
}

impl ScreenGrabber for MacOsScreen {
    fn capture_focused(&self) -> anyhow::Result<Screenshot> {
        Err(anyhow!("macOS screen capture not yet implemented"))
    }
}

impl TtsPlayer for MacOsTts {
    fn speak(&self, _text: &str) -> anyhow::Result<()> {
        Err(anyhow!("macOS TTS not yet implemented"))
    }
    fn stop(&self) -> anyhow::Result<()> {
        Err(anyhow!("macOS TTS not yet implemented"))
    }
}

impl PlatformProvider for MacOsProvider {
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