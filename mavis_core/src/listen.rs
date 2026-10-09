// mavis_core/src/listen.rs
// How MAVIS listens: always, or only after you ask it to (push to talk).
// In push mode the microphone is ignored until a hotkey or an orb tap
// opens a short window; one utterance is heard, then it closes again.

use log::info;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long an opened window waits for you to start speaking.
pub const OPEN_FOR: Duration = Duration::from_secs(8);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListenMode {
    /// Always listening — the default.
    Always,
    /// Listening only after a hotkey press or an orb tap.
    Push,
}

impl ListenMode {
    /// `MAVIS_LISTEN_MODE`, else `listen_mode` under `[voice]` in
    /// `config/config.toml`, else Always.
    pub fn configured() -> Self {
        let from_env = std::env::var("MAVIS_LISTEN_MODE").ok();
        let from_file = std::fs::read_to_string("../config/config.toml")
            .ok()
            .and_then(|text| read_setting(&text, "voice", "listen_mode"));
        Self::parse(from_env.or(from_file).as_deref())
    }

    fn parse(value: Option<&str>) -> Self {
        match value.map(|v| v.trim().to_lowercase()).as_deref() {
            Some("push") | Some("push_to_talk") | Some("push-to-talk") => ListenMode::Push,
            _ => ListenMode::Always,
        }
    }
}

/// `key = "value"` inside `[section]` of a TOML file. Just enough to read
/// one setting without a TOML dependency; anything unusual reads as absent.
pub fn read_setting(text: &str, section: &str, key: &str) -> Option<String> {
    let mut current = String::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = name.trim().to_string();
        } else if current == section {
            if let Some((k, v)) = line.split_once('=') {
                if k.trim() == key {
                    return Some(v.trim().trim_matches('"').to_string());
                }
            }
        }
    }
    None
}

/// Shared between the audio thread, the hotkey handler and the orb.
pub struct ListenGate {
    mode: ListenMode,
    opened_at: Mutex<Option<Instant>>,
    /// The utterance just shipped was asked for, so the worker shouldn't
    /// second-guess it as noise.
    explicit: AtomicBool,
}

impl ListenGate {
    pub fn new(mode: ListenMode) -> Self {
        Self {
            mode,
            opened_at: Mutex::new(None),
            explicit: AtomicBool::new(false),
        }
    }

    pub fn mode(&self) -> ListenMode {
        self.mode
    }

    /// Open a window, or close it if one is open — a hotkey is a toggle.
    /// Returns whether a window is now open. Always mode ignores it.
    pub fn toggle(&self) -> bool {
        if self.mode == ListenMode::Always {
            return false;
        }
        let mut opened = self.opened_at.lock().unwrap_or_else(|p| p.into_inner());
        let open = opened.is_some_and(|at| at.elapsed() < OPEN_FOR);
        *opened = if open { None } else { Some(Instant::now()) };
        info!("Listen: {}", if open { "closed" } else { "listening for one utterance" });
        !open
    }

    /// Whether the microphone should be heard right now.
    pub fn hearing(&self) -> bool {
        match self.mode {
            ListenMode::Always => true,
            ListenMode::Push => self
                .opened_at
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .is_some_and(|at| at.elapsed() < OPEN_FOR),
        }
    }

    /// An utterance was heard and sent on. In push mode that uses up the window.
    pub fn shipped(&self) {
        if self.mode == ListenMode::Push {
            *self.opened_at.lock().unwrap_or_else(|p| p.into_inner()) = None;
            self.explicit.store(true, Ordering::SeqCst);
        }
    }

    /// Whether the utterance being transcribed was asked for. Read once.
    pub fn take_explicit(&self) -> bool {
        self.explicit.swap(false, Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_is_read_from_its_section_only() {
        let text = "[ui]\nlisten_mode = \"push\"\n\n[voice]\nenabled = true\nlisten_mode = \"push\"  # set by hand\n";
        assert_eq!(read_setting(text, "voice", "listen_mode").as_deref(), Some("push"));
        assert_eq!(read_setting("[voice]\nenabled = true\n", "voice", "listen_mode"), None);
        assert_eq!(read_setting("listen_mode = \"push\"\n", "voice", "listen_mode"), None, "outside any section");
    }

    #[test]
    fn unknown_values_mean_always() {
        assert_eq!(ListenMode::parse(Some("push")), ListenMode::Push);
        assert_eq!(ListenMode::parse(Some(" Push-To-Talk ")), ListenMode::Push);
        assert_eq!(ListenMode::parse(Some("always")), ListenMode::Always);
        assert_eq!(ListenMode::parse(Some("sometimes")), ListenMode::Always);
        assert_eq!(ListenMode::parse(None), ListenMode::Always);
    }

    #[test]
    fn always_mode_always_hears_and_ignores_the_toggle() {
        let gate = ListenGate::new(ListenMode::Always);
        assert!(gate.hearing());
        assert!(!gate.toggle());
        gate.shipped();
        assert!(gate.hearing());
        assert!(!gate.take_explicit());
    }

    #[test]
    fn push_mode_hears_one_utterance_per_press() {
        let gate = ListenGate::new(ListenMode::Push);
        assert!(!gate.hearing(), "deaf until asked");
        assert!(gate.toggle());
        assert!(gate.hearing());
        gate.shipped();
        assert!(!gate.hearing(), "one utterance, then closed");
        assert!(gate.take_explicit());
        assert!(!gate.take_explicit(), "read once");
    }

    #[test]
    fn a_second_press_closes_the_window() {
        let gate = ListenGate::new(ListenMode::Push);
        assert!(gate.toggle());
        assert!(!gate.toggle());
        assert!(!gate.hearing());
    }

    #[test]
    fn an_unused_window_lapses() {
        let gate = ListenGate::new(ListenMode::Push);
        *gate.opened_at.lock().unwrap() = Some(Instant::now() - OPEN_FOR - Duration::from_millis(10));
        assert!(!gate.hearing());
        assert!(gate.toggle(), "a lapsed window reopens on the next press");
    }
}