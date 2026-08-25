// mavis_core/src/ui/states.rs

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrbState {
    Idle,
    Listening,
    Thinking,
    Speaking,
    Working,
    Error,
    Asleep,
    /// Brief celebratory state after successful plan completion.
    Celebrating,
}