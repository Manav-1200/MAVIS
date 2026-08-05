#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrbState {
    Idle,
    Listening,
    Thinking,
    Speaking,
    Working,
    Error,
    Asleep,
}
