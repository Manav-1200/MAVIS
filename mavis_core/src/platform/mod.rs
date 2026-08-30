//! Platform abstraction layer — Linux / Windows / macOS
//!
//! All system I/O goes through these traits. Platform-specific impls live
//! in submodules. The Context Engine requests capabilities; the platform
//! layer provides them or returns None if unavailable.

mod linux;
mod windows;
mod macos;

// ---------------------------------------------------------------------------
// Capabilities
// ---------------------------------------------------------------------------

pub trait AudioCapture {
    /// Start microphone capture. Returns a stream handle.
    fn start_input(&self, config: AudioConfig) -> Result<Box<dyn AudioStream>, PlatformError>;
}

pub trait AudioStream: Send {
    fn play(&self) -> Result<(), PlatformError>;
    fn pause(&self) -> Result<(), PlatformError>;
}

pub trait WindowTracker {
    /// Returns the currently focused window: (app_name, window_title, pid)
    fn active_window(&self) -> Result<(String, String, u32), PlatformError>;
    /// Subscribe to window focus changes. Returns a channel receiver.
    fn subscribe_changes(&self) -> Result<tokio::sync::mpsc::Receiver<WindowEvent>, PlatformError>;
}

pub trait ClipboardReader {
    /// Read current clipboard text. Returns None if not text or empty.
    fn read_text(&self) -> Result<Option<String>, PlatformError>;
    /// Subscribe to clipboard changes.
    fn subscribe_changes(&self) -> Result<tokio::sync::mpsc::Receiver<String>, PlatformError>;
}

pub trait ScreenGrabber {
    /// Capture the focused monitor or window. Returns raw bytes + dimensions.
    /// `data` is PNG on Linux, RGBA on other platforms (Phase 12).
    fn capture_focused(&self) -> Result<Screenshot, PlatformError>;
}

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

pub struct AudioConfig {
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_format: SampleFormat,
}

pub enum SampleFormat {
    F32,
    I16,
}

pub struct Screenshot {
    pub width: u32,
    pub height: u32,
    /// Platform-specific bytes: PNG on Linux, RGBA elsewhere (future).
    pub data: Vec<u8>,
}

pub struct WindowEvent {
    pub app_name: String,
    pub window_title: String,
    pub pid: u32,
}

#[derive(Debug)]
pub struct PlatformError(pub String);

impl std::fmt::Display for PlatformError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PlatformError: {}", self.0)
    }
}

impl std::error::Error for PlatformError {}

// ---------------------------------------------------------------------------
// Platform factory
// ---------------------------------------------------------------------------

pub enum Platform {
    Linux,
    Windows,
    MacOs,
}

impl Platform {
    pub fn detect() -> Self {
        #[cfg(target_os = "linux")]
        return Platform::Linux;
        #[cfg(target_os = "windows")]
        return Platform::Windows;
        #[cfg(target_os = "macos")]
        return Platform::MacOs;
    }

    /// Build the platform provider for this OS.
    pub fn build_provider(&self) -> Box<dyn PlatformProvider> {
        match self {
            Platform::Linux => Box::new(linux::LinuxProvider::new()),
            Platform::Windows => Box::new(windows::WindowsProvider::new()),
            Platform::MacOs => Box::new(macos::MacOsProvider::new()),
        }
    }
}

/// Aggregates all platform capabilities. Individual methods return None if
/// the capability is unavailable on this DE / OS / permission tier.
pub trait PlatformProvider: Send + Sync {
    fn audio(&self) -> Option<&dyn AudioCapture>;
    fn windows(&self) -> Option<&dyn WindowTracker>;
    fn clipboard(&self) -> Option<&dyn ClipboardReader>;
    fn screen(&self) -> Option<&dyn ScreenGrabber>;
}