//! A monotonic stopwatch that loses no time while the display task sleeps.

/// What one of the stopwatch controls asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopwatchControl {
    Start,
    Pause,
    Reset,
}

/// Stopwatch state anchored to monotonic uptime.
///
/// Time is derived rather than counted. No task has to wake while the panel is
/// asleep: the next observation subtracts the same monotonic clock from the
/// start anchor and includes the whole gap automatically.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StopwatchState {
    accumulated_millis: u64,
    started_at_millis: Option<u64>,
}

impl StopwatchState {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            accumulated_millis: 0,
            started_at_millis: None,
        }
    }

    #[must_use]
    pub const fn is_running(self) -> bool {
        self.started_at_millis.is_some()
    }

    #[must_use]
    pub const fn elapsed_millis(self, now_millis: u64) -> u64 {
        match self.started_at_millis {
            Some(started) => self
                .accumulated_millis
                .saturating_add(now_millis.saturating_sub(started)),
            None => self.accumulated_millis,
        }
    }

    #[must_use]
    pub const fn can_reset(self) -> bool {
        !self.is_running() && self.accumulated_millis != 0
    }

    /// Applies a control at one reading of the monotonic clock.
    ///
    /// Returns whether the state changed. Invalid repeats are deliberately
    /// inert: a second pause cannot add time, and reset while running cannot
    /// silently turn a stop control into a lap control.
    pub const fn apply(&mut self, control: StopwatchControl, now_millis: u64) -> bool {
        match control {
            StopwatchControl::Start if !self.is_running() => {
                self.started_at_millis = Some(now_millis);
                true
            }
            StopwatchControl::Pause => {
                let Some(started) = self.started_at_millis else {
                    return false;
                };
                self.accumulated_millis = self
                    .accumulated_millis
                    .saturating_add(now_millis.saturating_sub(started));
                self.started_at_millis = None;
                true
            }
            StopwatchControl::Reset if self.can_reset() => {
                self.accumulated_millis = 0;
                true
            }
            StopwatchControl::Start | StopwatchControl::Reset => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_time_is_derived_from_the_start_anchor() {
        let mut stopwatch = StopwatchState::new();
        assert!(stopwatch.apply(StopwatchControl::Start, 1_250));
        assert_eq!(stopwatch.elapsed_millis(4_750), 3_500);
    }

    #[test]
    fn a_sleep_gap_is_part_of_the_elapsed_time() {
        let mut stopwatch = StopwatchState::new();
        let _ = stopwatch.apply(StopwatchControl::Start, 100);

        // Nothing observes the state for an hour. The next observation still
        // sees that hour; no periodic task had to count it.
        assert_eq!(stopwatch.elapsed_millis(3_600_100), 3_600_000);
    }

    #[test]
    fn pause_and_resume_preserve_the_accumulated_time() {
        let mut stopwatch = StopwatchState::new();
        let _ = stopwatch.apply(StopwatchControl::Start, 1_000);
        assert!(stopwatch.apply(StopwatchControl::Pause, 3_500));
        assert_eq!(stopwatch.elapsed_millis(9_000), 2_500);

        assert!(stopwatch.apply(StopwatchControl::Start, 10_000));
        assert_eq!(stopwatch.elapsed_millis(11_250), 3_750);
    }

    #[test]
    fn reset_is_available_only_while_paused() {
        let mut stopwatch = StopwatchState::new();
        assert!(!stopwatch.apply(StopwatchControl::Reset, 0));
        let _ = stopwatch.apply(StopwatchControl::Start, 0);
        assert!(!stopwatch.apply(StopwatchControl::Reset, 1_000));
        let _ = stopwatch.apply(StopwatchControl::Pause, 1_000);
        assert!(stopwatch.can_reset());
        assert!(stopwatch.apply(StopwatchControl::Reset, 1_000));
        assert_eq!(stopwatch.elapsed_millis(2_000), 0);
    }

    #[test]
    fn clock_regression_cannot_subtract_elapsed_time() {
        let mut stopwatch = StopwatchState::new();
        let _ = stopwatch.apply(StopwatchControl::Start, 10_000);
        assert_eq!(stopwatch.elapsed_millis(9_000), 0);
    }
}
