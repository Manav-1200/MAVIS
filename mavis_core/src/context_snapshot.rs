// mavis_core/src/context_snapshot.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowInfo {
    pub app_name: String,
    pub window_title: String,
    pub pid: Option<u32>,
    /// Compositor workspace this window sits on (niri workspace_id).
    #[serde(default)]
    pub workspace_id: Option<u64>,
    #[serde(default)]
    pub is_focused: bool,
}

/// Project the user appears to be working in, resolved from the focused
/// window's process tree: walk children for a cwd outside $HOME, then walk
/// up from there to the nearest .git directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub name: String,
    pub path: String,
    pub git_branch: Option<String>,
}

/// Next upcoming calendar event, read from Evolution's local .ics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarEvent {
    pub summary: String,
    pub start: String,
    pub minutes_until: i64,
    pub all_day: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserTab {
    pub url: String,
    pub title: String,
    pub domain: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSnapshot {
    pub active_window: Option<WindowInfo>,
    #[serde(default)]
    pub open_windows: Vec<WindowInfo>,
    #[serde(default)]
    pub active_workspace: Option<u64>,
    #[serde(default)]
    pub project: Option<ProjectInfo>,
    #[serde(default)]
    pub next_event: Option<CalendarEvent>,
    pub clipboard_text: Option<String>,
    pub captured_at: u64,
}

impl ContextSnapshot {
    pub fn is_empty(&self) -> bool {
        self.active_window.is_none()
            && self.open_windows.is_empty()
            && self.clipboard_text.is_none()
    }
}