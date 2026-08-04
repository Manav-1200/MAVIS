// mavis_core/src/ui/states.rs
// Orb state machine.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrbState {
    Idle,
    Listening,
    Thinking,
    Speaking,
    Working,
    Notification,
    Error,
    Sleeping,
}