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
    /// The permission gate asked "Shall I?" and is waiting for an answer.
    Asking,
}

impl OrbState {
    /// The state to show for a UiStateChange. While a question is open,
    /// "idle" would be a lie — MAVIS is waiting on the user — so the orb
    /// stays on Asking between the question being spoken and the answer.
    pub fn from_event(state: &str, asking: &mut bool) -> Self {
        match state {
            "asking" => *asking = true,
            "answered" => *asking = false,
            _ => {}
        }
        match state {
            "listening" => OrbState::Listening,
            "thinking" => OrbState::Thinking,
            "speaking" => OrbState::Speaking,
            "working" => OrbState::Working,
            "error" => OrbState::Error,
            "asleep" => OrbState::Asleep,
            _ if *asking => OrbState::Asking,
            "celebrating" => OrbState::Celebrating,
            _ => OrbState::Idle,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_map_as_before_when_nothing_is_being_asked() {
        let mut asking = false;
        assert_eq!(OrbState::from_event("speaking", &mut asking), OrbState::Speaking);
        assert_eq!(OrbState::from_event("celebrating", &mut asking), OrbState::Celebrating);
        assert_eq!(OrbState::from_event("idle", &mut asking), OrbState::Idle);
        assert_eq!(OrbState::from_event("something new", &mut asking), OrbState::Idle);
    }

    /// The question is spoken, then playback ends with "idle" — the orb
    /// must keep showing that an answer is wanted until there is one.
    #[test]
    fn the_orb_keeps_asking_until_answered() {
        let mut asking = false;
        assert_eq!(OrbState::from_event("asking", &mut asking), OrbState::Asking);
        assert_eq!(OrbState::from_event("speaking", &mut asking), OrbState::Speaking);
        assert_eq!(OrbState::from_event("celebrating", &mut asking), OrbState::Asking);
        assert_eq!(OrbState::from_event("idle", &mut asking), OrbState::Asking);
        assert_eq!(OrbState::from_event("listening", &mut asking), OrbState::Listening);
        assert_eq!(OrbState::from_event("answered", &mut asking), OrbState::Idle);
        assert_eq!(OrbState::from_event("idle", &mut asking), OrbState::Idle);
    }
}