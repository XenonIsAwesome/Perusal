//! CI status widget model and polling cadence.
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CiState { Running, Passed, Failed, Pending, Canceled, None }

#[derive(Debug, PartialEq)]
pub enum Look { Hidden, Breathing { period_ms: u32 }, StaticRing, Check, Cross, Slash }

/// Colour is chosen by the theme from the state; this decides shape and motion.
pub fn look(s: CiState, reduced_motion: bool) -> Look {
    match (s, reduced_motion) {
        (CiState::None, _) => Look::Hidden,
        (CiState::Running, false) => Look::Breathing { period_ms: 2000 }, (CiState::Pending, false) => Look::Breathing { period_ms: 3500 },
        (CiState::Running | CiState::Pending, true) => Look::StaticRing,
        (CiState::Passed, _) => Look::Check, (CiState::Failed, _) => Look::Cross, (CiState::Canceled, _) => Look::Slash,
    }
}

/// 15 s while running and visible, 60 s otherwise, paused when the window is unfocused.
pub fn poll_interval(s: CiState, tab_visible: bool, window_focused: bool) -> Option<Duration> {
    if !window_focused { return None; }
    Some(Duration::from_secs(if s == CiState::Running && tab_visible { 15 } else { 60 }))
}

#[cfg(test)]
mod tests { use super::*;
    #[test] fn looks_and_polling() {
        assert_eq!(look(CiState::Running, false), Look::Breathing { period_ms: 2000 }); assert_eq!(look(CiState::Pending, false), Look::Breathing { period_ms: 3500 });
        assert_eq!(look(CiState::Running, true), Look::StaticRing); assert_eq!(look(CiState::None, false), Look::Hidden);
        assert_eq!(look(CiState::Passed, false), Look::Check); assert_eq!(look(CiState::Failed, true), Look::Cross); assert_eq!(look(CiState::Canceled, false), Look::Slash);
        assert_eq!(poll_interval(CiState::Running, true, true), Some(Duration::from_secs(15))); assert_eq!(poll_interval(CiState::Running, false, true), Some(Duration::from_secs(60)));
        assert_eq!(poll_interval(CiState::Passed, true, true), Some(Duration::from_secs(60))); assert_eq!(poll_interval(CiState::Running, true, false), None);
    } }
