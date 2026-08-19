//! Windows platform provider — stub, compiles, returns errors gracefully.

use super::*;
use anyhow::anyhow;

pub struct WindowsProvider;

impl WindowsProvider {
    pub fn new() -> Self {
        Self
    }
}

struct WindowsWindowTracker;
struct WindowsClipboard;
struct WindowsScreen;
struct WindowsTts;

impl WindowTracker for WindowsWindowTracker {
    fn active_window(&self) -> anyhow::Result<(String, String, u32)> {
        Err(anyhow!("Windows window tracking not yet implemented"))
    }
    fn subscribe(&self) -> anyhow::Result<tokio::sync::mpsc::Receiver<WindowEvent>> {
        Err(anyhow!("Windows window tracking not yet implemented"))
    }
}

impl ClipboardReader for WindowsClipboard {
    fn read_text(&self) -> anyhow::Result<Option<String>> {
        Err(anyhow!("Windows clipboard not yet implemented"))
    }
    fn subscribe(&self) -> anyhow::Result<tokio::sync::mpsc::Receiver<String>> {
        Err(anyhow!("Windows clipboard not yet implemented"))
    }
}

impl ScreenGrabber for WindowsScreen {
    fn capture_focused(&self) -> anyhow::Result<Screenshot> {
        Err(anyhow!("Windows screen capture not yet implemented"))
    }
}

impl TtsPlayer for WindowsTts {
    fn speak(&self, _text: &str) -> anyhow::Result<()> {
        Err(anyhow!("Windows TTS not yet implemented"))
    }
    fn stop(&self) -> anyhow::Result<()> {
        Err(anyhow!("Windows TTS not yet implemented"))
    }
}

impl PlatformProvider for WindowsProvider {
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