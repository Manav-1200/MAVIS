// mavis_core/src/platform/mod.rs
//! Platform abstraction layer — Linux / Windows / macOS
//!
//! All system I/O goes through these traits. Platform-specific impls live
//! in submodules. The Context Engine requests capabilities; the platform
//! layer provides them or returns None if unavailable.

mod linux;
mod windows;
mod macos;

pub use crate::context_snapshot::{AppEntry, ProjectInfo, WindowInfo};

// ---------------------------------------------------------------------------
// Capabilities
// ---------------------------------------------------------------------------

pub trait WindowTracker {
    /// Returns the currently focused window: (app_name, window_title, pid)
    fn active_window(&self) -> Result<(String, String, u32), PlatformError>;
    /// Returns every open window, including which workspace each sits on
    /// and which is focused. Answers "what applications are open" —
    /// active_window alone only ever knows about the focused one.
    fn open_windows(&self) -> Result<Vec<WindowInfo>, PlatformError>;
    /// Best-effort project detection for a window's process tree.
    /// Default None: only Linux implements this (via /proc), and the
    /// default keeps the other platform stubs unchanged.
    fn current_project(&self, _pid: u32) -> Option<ProjectInfo> {
        None
    }
}

pub trait ClipboardReader {
    /// Read current clipboard text. Returns None if not text or empty.
    fn read_text(&self) -> Result<Option<String>, PlatformError>;
}

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

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
    /// Every application installed on this machine. Used to turn "open
    /// firefox" into a launch. Default empty so a platform without an
    /// implementation degrades to "no apps found" rather than failing.
    fn installed_apps(&self) -> Vec<AppEntry> {
        Vec::new()
    }
    fn windows(&self) -> Option<&dyn WindowTracker>;
    fn clipboard(&self) -> Option<&dyn ClipboardReader>;
}