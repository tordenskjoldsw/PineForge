//! A countdown timer derived from one monotonic deadline.

pub const TIMER_DEFAULT_MINUTES: u8 = 5;
pub const TIMER_MIN_MINUTES: u8 = 1;
pub const TIMER_MAX_MINUTES: u8 = 99;
const MILLIS_PER_MINUTE: u64 = 60_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerControl {
    Decrease,
    Increase,
    Start,
    Pause,
    Reset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerPhase {
    Ready,
    Running,
    Paused,
    Expired,
}

/// What applying a timestamp or control did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerOutcome {
    Unchanged,
    Changed,
    /// The deadline was crossed by this observation and the alarm is owed.
    Expired,
}

/// Countdown state whose running value is always derived from a deadline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimerState {
    selected_minutes: u8,
    remaining_millis: u64,
    deadline_millis: Option<u64>,
    phase: TimerPhase,
}

impl TimerState {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            selected_minutes: TIMER_DEFAULT_MINUTES,
            remaining_millis: minutes_millis(TIMER_DEFAULT_MINUTES),
            deadline_millis: None,
            phase: TimerPhase::Ready,
        }
    }

    #[must_use]
    pub const fn phase(self) -> TimerPhase {
        self.phase
    }

    #[must_use]
    pub const fn selected_minutes(self) -> u8 {
        self.selected_minutes
    }

    #[must_use]
    pub const fn deadline_millis(self) -> Option<u64> {
        self.deadline_millis
    }

    #[must_use]
    pub const fn remaining_millis(self, now_millis: u64) -> u64 {
        match self.deadline_millis {
            Some(deadline) => deadline.saturating_sub(now_millis),
            None => self.remaining_millis,
        }
    }

    /// Seconds for display, rounded up so a fresh five-minute timer reads
    /// `05:00` rather than losing a second at the first subsecond observation.
    #[must_use]
    pub const fn remaining_seconds(self, now_millis: u64) -> u64 {
        self.remaining_millis(now_millis).saturating_add(999) / 1_000
    }

    /// Observes monotonic time and reports the deadline exactly once.
    pub const fn observe(&mut self, now_millis: u64) -> TimerOutcome {
        let Some(deadline) = self.deadline_millis else {
            return TimerOutcome::Unchanged;
        };
        if now_millis < deadline {
            return TimerOutcome::Unchanged;
        }
        self.deadline_millis = None;
        self.remaining_millis = 0;
        self.phase = TimerPhase::Expired;
        TimerOutcome::Expired
    }

    pub const fn apply(&mut self, control: TimerControl, now_millis: u64) -> TimerOutcome {
        if matches!(self.observe(now_millis), TimerOutcome::Expired) {
            return TimerOutcome::Expired;
        }

        match control {
            TimerControl::Decrease if matches!(self.phase, TimerPhase::Ready) => {
                let next = self.selected_minutes.saturating_sub(1);
                if next < TIMER_MIN_MINUTES {
                    return TimerOutcome::Unchanged;
                }
                self.set_minutes(next);
                TimerOutcome::Changed
            }
            TimerControl::Increase if matches!(self.phase, TimerPhase::Ready) => {
                let next = self.selected_minutes.saturating_add(1);
                if next > TIMER_MAX_MINUTES {
                    return TimerOutcome::Unchanged;
                }
                self.set_minutes(next);
                TimerOutcome::Changed
            }
            TimerControl::Start if matches!(self.phase, TimerPhase::Ready | TimerPhase::Paused) => {
                self.deadline_millis = Some(now_millis.saturating_add(self.remaining_millis));
                self.phase = TimerPhase::Running;
                TimerOutcome::Changed
            }
            TimerControl::Pause if matches!(self.phase, TimerPhase::Running) => {
                self.remaining_millis = self.remaining_millis(now_millis);
                self.deadline_millis = None;
                self.phase = TimerPhase::Paused;
                TimerOutcome::Changed
            }
            TimerControl::Reset if !matches!(self.phase, TimerPhase::Ready) => {
                self.deadline_millis = None;
                self.remaining_millis = minutes_millis(self.selected_minutes);
                self.phase = TimerPhase::Ready;
                TimerOutcome::Changed
            }
            TimerControl::Decrease
            | TimerControl::Increase
            | TimerControl::Start
            | TimerControl::Pause
            | TimerControl::Reset => TimerOutcome::Unchanged,
        }
    }

    const fn set_minutes(&mut self, minutes: u8) {
        self.selected_minutes = minutes;
        self.remaining_millis = minutes_millis(minutes);
    }
}

impl Default for TimerState {
    fn default() -> Self {
        Self::new()
    }
}

const fn minutes_millis(minutes: u8) -> u64 {
    minutes as u64 * MILLIS_PER_MINUTE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_five_minutes_and_adjusts_only_while_ready() {
        let mut timer = TimerState::new();
        assert_eq!(timer.remaining_seconds(0), 300);
        assert_eq!(
            timer.apply(TimerControl::Increase, 0),
            TimerOutcome::Changed
        );
        assert_eq!(timer.selected_minutes(), 6);
        let _ = timer.apply(TimerControl::Start, 1_000);
        assert_eq!(
            timer.apply(TimerControl::Increase, 2_000),
            TimerOutcome::Unchanged
        );
    }

    #[test]
    fn adjustment_stops_at_one_and_ninety_nine_minutes() {
        let mut timer = TimerState::new();
        for _ in 0..TIMER_MAX_MINUTES {
            let _ = timer.apply(TimerControl::Decrease, 0);
        }
        assert_eq!(timer.selected_minutes(), TIMER_MIN_MINUTES);
        assert_eq!(
            timer.apply(TimerControl::Decrease, 0),
            TimerOutcome::Unchanged
        );
        for _ in 0..TIMER_MAX_MINUTES {
            let _ = timer.apply(TimerControl::Increase, 0);
        }
        assert_eq!(timer.selected_minutes(), TIMER_MAX_MINUTES);
        assert_eq!(
            timer.apply(TimerControl::Increase, 0),
            TimerOutcome::Unchanged
        );
    }

    #[test]
    fn display_seconds_round_up_until_the_exact_deadline() {
        let mut timer = TimerState::new();
        let _ = timer.apply(TimerControl::Start, 1_234);
        assert_eq!(timer.remaining_seconds(1_234), 300);
        assert_eq!(timer.remaining_seconds(1_235), 300);
        assert_eq!(timer.remaining_seconds(2_234), 299);
        assert_eq!(timer.remaining_seconds(301_234), 0);
    }

    #[test]
    fn pause_and_resume_preserve_the_remaining_duration() {
        let mut timer = TimerState::new();
        let _ = timer.apply(TimerControl::Start, 1_000);
        let _ = timer.apply(TimerControl::Pause, 31_000);
        assert_eq!(timer.remaining_seconds(90_000), 270);
        let _ = timer.apply(TimerControl::Start, 100_000);
        assert_eq!(timer.deadline_millis(), Some(370_000));
    }

    #[test]
    fn a_sleep_gap_expires_once_and_reset_restores_the_selection() {
        let mut timer = TimerState::new();
        let _ = timer.apply(TimerControl::Decrease, 0);
        let _ = timer.apply(TimerControl::Start, 100);
        assert_eq!(timer.observe(240_099), TimerOutcome::Unchanged);
        assert_eq!(timer.observe(240_100), TimerOutcome::Expired);
        assert_eq!(timer.observe(300_000), TimerOutcome::Unchanged);
        assert_eq!(timer.phase(), TimerPhase::Expired);
        assert_eq!(
            timer.apply(TimerControl::Reset, 300_000),
            TimerOutcome::Changed
        );
        assert_eq!(timer.remaining_seconds(300_000), 240);
    }

    #[test]
    fn reset_cancels_a_running_timer() {
        let mut timer = TimerState::new();
        let _ = timer.apply(TimerControl::Start, 0);
        assert_eq!(
            timer.apply(TimerControl::Reset, 5_000),
            TimerOutcome::Changed
        );
        assert_eq!(timer.phase(), TimerPhase::Ready);
        assert_eq!(timer.deadline_millis(), None);
    }

    #[test]
    fn an_input_at_the_deadline_cannot_silence_the_alarm() {
        let mut timer = TimerState::new();
        let _ = timer.apply(TimerControl::Start, 0);
        assert_eq!(
            timer.apply(TimerControl::Pause, 300_000),
            TimerOutcome::Expired
        );
    }
}
