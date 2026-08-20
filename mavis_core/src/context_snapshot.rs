// mavis_core/src/context_snapshot.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowInfo {
    pub app_name: String,
    pub window_title: String,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSnapshot {
    pub active_window: Option<WindowInfo>,
    pub clipboard_text: Option<String>,
    pub captured_at: u64,
}

impl ContextSnapshot {
    pub fn is_empty(&self) -> bool {
        self.active_window.is_none() && self.clipboard_text.is_none()
    }
}