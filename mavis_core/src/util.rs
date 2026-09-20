// mavis_core/src/util.rs
// Small shared helpers that several subsystems need.

use crate::event_bus::EventBus;
use log::{error, info, warn};
use std::sync::Arc;

/// How many times a subsystem may panic before MAVIS stops restarting it.
/// A deterministic panic would otherwise spin forever; five attempts is
/// enough to ride out something transient without hiding a real bug.
pub const MAX_SUBSYSTEM_RESTARTS: u32 = 5;

/// Pause between restarts, so a panic that recurs immediately can't burn
/// a core.
pub const RESTART_BACKOFF: std::time::Duration = std::time::Duration::from_millis(500);

/// Run a long-lived subsystem under supervision.
///
/// A panic inside a `tokio::spawn`ed task does not stop the process and
/// does not stop any other task — it just ends *that* task. Nothing logs
/// it at the application level and nothing restarts it. The visible
/// result is a MAVIS that still listens, still transcribes, still
/// animates the orb, and never answers again. That is precisely the
/// "it doesn't respond" failure: a single unlucky transcription panicked
/// the planner on a UTF-8 boundary and the subsystem was simply gone for
/// the rest of the session.
///
/// The individual panics are fixed, but "one bug silently amputates a
/// subsystem" is the structural problem, so every event-loop subsystem
/// runs through here. `factory` rebuilds the subsystem from its inputs —
/// all of which are cheap `Arc`/`Clone` handles — so a restart gets a
/// fresh, consistent object rather than resuming from whatever
/// half-updated state the panic left behind.
///
/// Clean exits (the bus closing at shutdown) are not restarts.
pub fn supervise<F, Fut>(
    name: &'static str,
    bus: Arc<EventBus>,
    factory: F,
) -> tokio::task::JoinHandle<()>
where
    F: Fn() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        let mut restarts: u32 = 0;
        loop {
            match tokio::spawn(factory()).await {
                // Normal return: the subsystem saw the bus close.
                Ok(()) => return,
                Err(join_err) if join_err.is_panic() => {
                    // Shutting down anyway — a panic on the way out is not
                    // worth restarting for.
                    if !bus.is_open() {
                        warn!("{}: panicked during shutdown, not restarting", name);
                        return;
                    }
                    restarts += 1;
                    error!(
                        "{}: PANICKED — restarting ({}/{}). This is a bug; the \
                         panic message above is the one worth reporting.",
                        name, restarts, MAX_SUBSYSTEM_RESTARTS
                    );
                    if restarts >= MAX_SUBSYSTEM_RESTARTS {
                        error!(
                            "{}: panicked {} times, giving up. MAVIS keeps running \
                             without it — restart MAVIS to bring it back.",
                            name, restarts
                        );
                        return;
                    }
                    tokio::time::sleep(RESTART_BACKOFF).await;
                }
                // Cancelled (runtime shutting down).
                Err(_) => {
                    info!("{}: cancelled", name);
                    return;
                }
            }
        }
    })
}

/// Truncate a string to at most `max_bytes`, never splitting a character.
///
/// `&s[..n]` panics when byte `n` lands inside a multi-byte character, and
/// every string MAVIS handles is external input: Whisper transcriptions,
/// clipboard contents, window titles. Whisper emits curly apostrophes
/// (U+2019 — the STT engine already normalises them), em-dashes, ellipses
/// and accented loanwords, any of which can straddle a truncation point.
///
/// Three separate call sites were slicing by raw byte index. A panic in a
/// spawned task ends that task silently, so the visible symptom was not a
/// crash but a subsystem that stopped responding for the rest of the run.
///
/// Returns whole characters, so the result is always <= `max_bytes`.
pub fn truncate_bytes(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    // is_char_boundary(0) is always true, so this terminates.
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_shorter_than_limit_is_unchanged() {
        assert_eq!(truncate_bytes("hello", 20), "hello");
    }

    #[test]
    fn ascii_longer_than_limit_is_cut() {
        assert_eq!(truncate_bytes("hello world", 5), "hello");
    }

    #[test]
    fn exact_length_is_unchanged() {
        assert_eq!(truncate_bytes("hello", 5), "hello");
    }

    #[test]
    fn empty_string() {
        assert_eq!(truncate_bytes("", 10), "");
        assert_eq!(truncate_bytes("", 0), "");
    }

    /// The case that panicked: byte 20 falls inside a 3-byte character.
    #[test]
    fn does_not_split_multibyte_character() {
        let s = "aaaaaaaaaaaaaaaaaaa日本語more";
        let out = truncate_bytes(s, 20);
        assert_eq!(out, "aaaaaaaaaaaaaaaaaaa");
        assert!(out.len() <= 20);
    }

    /// Whisper's curly apostrophe is 3 bytes and shows up constantly.
    #[test]
    fn handles_curly_apostrophe_at_boundary() {
        let s = "that\u{2019}s the clipboard contents here";
        for limit in 0..s.len() + 4 {
            let out = truncate_bytes(s, limit);
            assert!(out.len() <= limit.min(s.len()));
            assert!(s.starts_with(out));
        }
    }

    /// Every limit over a string that is entirely multi-byte.
    #[test]
    fn every_limit_is_safe_for_multibyte_text() {
        for s in ["日本語", "ééé", "café — naïve…", "🙂🙂🙂"] {
            for limit in 0..s.len() + 4 {
                let out = truncate_bytes(s, limit);
                assert!(s.starts_with(out), "{:?} limit {}", s, limit);
            }
        }
    }

    #[test]
    fn zero_limit_yields_empty() {
        assert_eq!(truncate_bytes("日本語", 0), "");
    }

    // -----------------------------------------------------------------
    // supervise
    // -----------------------------------------------------------------

    use crate::event_bus::EventBus;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    /// The core guarantee: a panicking subsystem comes back.
    #[tokio::test]
    async fn supervise_restarts_a_panicking_subsystem() {
        let bus = Arc::new(EventBus::new());
        let attempts = Arc::new(AtomicU32::new(0));
        let attempts_for_factory = Arc::clone(&attempts);

        let handle = supervise("TestSubsystem", Arc::clone(&bus), move || {
            let attempts = Arc::clone(&attempts_for_factory);
            async move {
                let n = attempts.fetch_add(1, Ordering::SeqCst);
                if n < 2 {
                    panic!("simulated subsystem panic #{}", n);
                }
                // Third attempt succeeds and returns cleanly.
            }
        });

        handle.await.expect("supervisor itself must never panic");
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            3,
            "should have run 3 times: two panics plus one clean run"
        );
    }

    /// A subsystem that panics every time must eventually be given up on,
    /// rather than spinning forever.
    #[tokio::test]
    async fn supervise_gives_up_after_the_restart_limit() {
        let bus = Arc::new(EventBus::new());
        let attempts = Arc::new(AtomicU32::new(0));
        let attempts_for_factory = Arc::clone(&attempts);

        let handle = supervise("AlwaysPanics", Arc::clone(&bus), move || {
            let attempts = Arc::clone(&attempts_for_factory);
            async move {
                attempts.fetch_add(1, Ordering::SeqCst);
                panic!("always fails");
            }
        });

        handle.await.expect("supervisor itself must never panic");
        assert_eq!(attempts.load(Ordering::SeqCst), MAX_SUBSYSTEM_RESTARTS);
    }

    /// A clean return (the bus closing at shutdown) is not a restart.
    #[tokio::test]
    async fn supervise_does_not_restart_a_clean_exit() {
        let bus = Arc::new(EventBus::new());
        let attempts = Arc::new(AtomicU32::new(0));
        let attempts_for_factory = Arc::clone(&attempts);

        let handle = supervise("CleanExit", Arc::clone(&bus), move || {
            let attempts = Arc::clone(&attempts_for_factory);
            async move {
                attempts.fetch_add(1, Ordering::SeqCst);
            }
        });

        handle.await.expect("supervisor itself must never panic");
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    /// Panicking while the bus is already closed must not trigger a
    /// restart loop during shutdown.
    #[tokio::test]
    async fn supervise_does_not_restart_once_the_bus_is_closed() {
        let bus = Arc::new(EventBus::new());
        bus.close();
        let attempts = Arc::new(AtomicU32::new(0));
        let attempts_for_factory = Arc::clone(&attempts);

        let handle = supervise("ShuttingDown", Arc::clone(&bus), move || {
            let attempts = Arc::clone(&attempts_for_factory);
            async move {
                attempts.fetch_add(1, Ordering::SeqCst);
                panic!("panic on the way out");
            }
        });

        handle.await.expect("supervisor itself must never panic");
        assert_eq!(attempts.load(Ordering::SeqCst), 1, "must not restart");
    }
}