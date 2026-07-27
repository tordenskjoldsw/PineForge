//! System modals: full-screen prompts that override the screen stack.
//!
//! A modal outranks whatever screen is active, and the rules for raising and
//! dismissing one are product policy, not display-driver work. Keeping them
//! here makes them testable on a host and keeps the display task down to the
//! hardware effects each outcome implies.

use crate::{AppEvent, BleState, DfuFailReason, StorageState};

/// A full-screen prompt shown above the active screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modal {
    /// First-boot flash format. Outranks every other modal: it blocks the
    /// storage the other flows depend on, and it cannot be dismissed.
    StorageFormat(u8),
    /// Pairing passkey to read off the watch and enter on the phone.
    Pairing(u32),
    /// Firmware transfer progress.
    DfuProgress(u8),
    /// A firmware transfer failed. Terminal, so the user can dismiss it -
    /// otherwise the sealed watch would be stuck until a reboot.
    DfuFailed(DfuFailReason),
}

/// What the display task must do with an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModalOutcome {
    /// No modal is involved; the event belongs to the active screen.
    None,
    /// Put this modal up. The event does not reach the screen.
    Show(Modal),
    /// The prompt already showing is the same one, with a new value in it - a
    /// transfer that advanced a percent.
    ///
    /// Told apart from [`Self::Show`] because repainting a whole screen for a
    /// number that moved is what makes a progress bar flicker, and a transfer
    /// does it a hundred times. Only the part that changed has to be drawn.
    Refresh(Modal),
    /// A modal is showing and this event is suppressed entirely, so nothing
    /// redraws over the prompt.
    Suppressed,
    /// A modal is showing, and this event is not modal but must still reach
    /// the active screen so it is current when the modal closes. The screen
    /// updates its state without drawing.
    UpdateBehind,
    /// The modal closed and the active screen needs a full redraw.
    ///
    /// `deliver` says whether the closing event also belongs to the screen. A
    /// dismissing touch or swipe does not: a modal consumes the input that
    /// dismisses it, so it can never also navigate the screen underneath.
    Dismissed { deliver: bool },
}

/// Owns which system modal is showing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModalState {
    current: Option<Modal>,
}

impl ModalState {
    #[must_use]
    pub const fn new() -> Self {
        Self { current: None }
    }

    /// The modal to redraw after waking, if one outlived sleep.
    #[must_use]
    pub const fn current(&self) -> Option<Modal> {
        self.current
    }

    /// Applies an event and reports what the display task should do with it.
    pub const fn handle(&mut self, event: AppEvent) -> ModalOutcome {
        let outcome = self.outcome_for(event);
        match outcome {
            ModalOutcome::Show(modal) | ModalOutcome::Refresh(modal) => self.current = Some(modal),
            ModalOutcome::Dismissed { .. } => self.current = None,
            ModalOutcome::None | ModalOutcome::Suppressed | ModalOutcome::UpdateBehind => {}
        }
        outcome
    }

    /// Raising a modal, as [`ModalOutcome::Refresh`] when the same prompt is
    /// already up and only its value moved.
    const fn raise(self, modal: Modal) -> ModalOutcome {
        let same = matches!(
            (self.current, modal),
            (Some(Modal::StorageFormat(_)), Modal::StorageFormat(_))
                | (Some(Modal::Pairing(_)), Modal::Pairing(_))
                | (Some(Modal::DfuProgress(_)), Modal::DfuProgress(_))
                | (Some(Modal::DfuFailed(_)), Modal::DfuFailed(_))
        );
        if same {
            ModalOutcome::Refresh(modal)
        } else {
            ModalOutcome::Show(modal)
        }
    }

    const fn outcome_for(self, event: AppEvent) -> ModalOutcome {
        // A format in progress outranks every other modal, so its arms come
        // first: a pairing or transfer that starts behind it must not raise a
        // prompt over the one flow that cannot be interrupted.
        if let AppEvent::StorageUpdated(StorageState::Formatting(percent)) = event {
            return self.raise(Modal::StorageFormat(percent));
        }
        if matches!(self.current, Some(Modal::StorageFormat(_))) {
            return match event {
                AppEvent::StorageUpdated(StorageState::Ready | StorageState::Failed) => {
                    ModalOutcome::Dismissed { deliver: false }
                }
                // BLE comes up behind the format screen; let the status line
                // reflect it instead of a stale "off" once the modal closes.
                AppEvent::BleUpdated(_) => ModalOutcome::UpdateBehind,
                _ => ModalOutcome::Suppressed,
            };
        }

        // A BLE state that has its own modal always raises or refreshes it.
        if let AppEvent::BleUpdated(state) = event {
            match state {
                BleState::Pairing(passkey) => return self.raise(Modal::Pairing(passkey)),
                BleState::DfuProgress(percent) => return self.raise(Modal::DfuProgress(percent)),
                BleState::DfuFailed(reason) => return self.raise(Modal::DfuFailed(reason)),
                BleState::Off | BleState::Advertising | BleState::Connected => {}
            }
        }

        let Some(modal) = self.current else {
            return ModalOutcome::None;
        };
        match event {
            // Only a terminal BLE state clears a pairing prompt or a transfer,
            // and the screen behind wants that state for its status line.
            AppEvent::BleUpdated(_) => ModalOutcome::Dismissed { deliver: true },
            // A failure screen is terminal, so a tap or swipe dismisses it.
            // The input is consumed here and never also navigates.
            _ if matches!(modal, Modal::DfuFailed(_)) && event.is_user_activity() => {
                ModalOutcome::Dismissed { deliver: false }
            }
            // Sensor ticks and everything else stay off the prompt.
            _ => ModalOutcome::Suppressed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BatteryStatus, SwipeDirection};

    const TICK: AppEvent = AppEvent::Tick {
        uptime_seconds: 1,
        wall_time: None,
        date: None,
    };
    const SWIPE: AppEvent = AppEvent::Swipe(SwipeDirection::Down);
    const BATTERY: AppEvent = AppEvent::BatteryUpdated(BatteryStatus {
        millivolts: 3_900,
        percent: 80,
        charging: false,
        power_present: false,
    });

    #[test]
    fn events_pass_through_while_no_modal_shows() {
        let mut modals = ModalState::new();
        assert_eq!(modals.handle(TICK), ModalOutcome::None);
        assert_eq!(modals.handle(SWIPE), ModalOutcome::None);
        assert_eq!(
            modals.handle(AppEvent::BleUpdated(BleState::Connected)),
            ModalOutcome::None
        );
        assert_eq!(modals.current(), None);
    }

    #[test]
    fn a_pairing_passkey_takes_over_and_suppresses_everything_else() {
        let mut modals = ModalState::new();
        assert_eq!(
            modals.handle(AppEvent::BleUpdated(BleState::Pairing(123_456))),
            ModalOutcome::Show(Modal::Pairing(123_456))
        );
        assert_eq!(modals.current(), Some(Modal::Pairing(123_456)));
        assert_eq!(modals.handle(TICK), ModalOutcome::Suppressed);
        assert_eq!(modals.handle(BATTERY), ModalOutcome::Suppressed);
        // A swipe must not dismiss a prompt that is still waiting on the phone.
        assert_eq!(modals.handle(SWIPE), ModalOutcome::Suppressed);
        assert_eq!(
            modals.handle(AppEvent::BleUpdated(BleState::Connected)),
            ModalOutcome::Dismissed { deliver: true }
        );
        assert_eq!(modals.current(), None);
    }

    /// A percentage that moved is a refresh, not a new prompt.
    ///
    /// The distinction is what keeps the progress screen from being repainted
    /// whole a hundred times over a transfer, which is visible as flicker: each
    /// full repaint clears the panel for the length of an SPI frame.
    #[test]
    fn transfer_progress_refreshes_rather_than_dismisses() {
        let mut modals = ModalState::new();
        assert_eq!(
            modals.handle(AppEvent::BleUpdated(BleState::DfuProgress(10))),
            ModalOutcome::Show(Modal::DfuProgress(10))
        );
        assert_eq!(
            modals.handle(AppEvent::BleUpdated(BleState::DfuProgress(90))),
            ModalOutcome::Refresh(Modal::DfuProgress(90))
        );
        // A stray touch cannot kill a transfer in flight.
        assert_eq!(modals.handle(SWIPE), ModalOutcome::Suppressed);
        assert_eq!(
            modals.handle(AppEvent::BleUpdated(BleState::Connected)),
            ModalOutcome::Dismissed { deliver: true }
        );
    }

    #[test]
    fn a_failure_screen_is_dismissed_by_the_user_and_consumes_the_input() {
        let mut modals = ModalState::new();
        assert_eq!(
            modals.handle(AppEvent::BleUpdated(BleState::DfuFailed(
                DfuFailReason::EraseFailed
            ))),
            ModalOutcome::Show(Modal::DfuFailed(DfuFailReason::EraseFailed))
        );
        assert_eq!(modals.handle(TICK), ModalOutcome::Suppressed);
        // Terminal, so the sealed watch is never stuck: the swipe dismisses it
        // and is not delivered, so it cannot also navigate.
        assert_eq!(
            modals.handle(SWIPE),
            ModalOutcome::Dismissed { deliver: false }
        );
        assert_eq!(modals.current(), None);
    }

    #[test]
    fn the_back_button_dismisses_only_what_a_swipe_dismisses() {
        let mut modals = ModalState::new();
        let _ = modals.handle(AppEvent::BleUpdated(BleState::Pairing(123_456)));
        // The button is an input like any other, so it cannot cancel a prompt
        // that is still waiting on the phone.
        assert_eq!(
            modals.handle(AppEvent::BackPressed),
            ModalOutcome::Suppressed
        );

        let _ = modals.handle(AppEvent::BleUpdated(BleState::DfuFailed(
            DfuFailReason::EraseFailed,
        )));
        // The terminal failure screen is the one a user may close, and the
        // press is consumed there instead of also popping the screen behind.
        assert_eq!(
            modals.handle(AppEvent::BackPressed),
            ModalOutcome::Dismissed { deliver: false }
        );
        assert_eq!(modals.current(), None);
    }

    #[test]
    fn a_format_outranks_the_modals_that_start_behind_it() {
        let mut modals = ModalState::new();
        assert_eq!(
            modals.handle(AppEvent::StorageUpdated(StorageState::Formatting(20))),
            ModalOutcome::Show(Modal::StorageFormat(20))
        );
        // BLE advertising and pairing come up during the first-boot format;
        // neither may raise a prompt over it, but the screen behind stays
        // current so its status line is right once the format finishes.
        assert_eq!(
            modals.handle(AppEvent::BleUpdated(BleState::Advertising)),
            ModalOutcome::UpdateBehind
        );
        assert_eq!(
            modals.handle(AppEvent::BleUpdated(BleState::Pairing(1))),
            ModalOutcome::UpdateBehind
        );
        assert_eq!(modals.current(), Some(Modal::StorageFormat(20)));
        assert_eq!(modals.handle(TICK), ModalOutcome::Suppressed);
        assert_eq!(modals.handle(SWIPE), ModalOutcome::Suppressed);
        assert_eq!(
            modals.handle(AppEvent::StorageUpdated(StorageState::Formatting(80))),
            ModalOutcome::Refresh(Modal::StorageFormat(80))
        );
        assert_eq!(
            modals.handle(AppEvent::StorageUpdated(StorageState::Ready)),
            ModalOutcome::Dismissed { deliver: false }
        );
        assert_eq!(modals.current(), None);
    }

    #[test]
    fn a_failed_format_also_closes_its_modal() {
        let mut modals = ModalState::new();
        let _ = modals.handle(AppEvent::StorageUpdated(StorageState::Formatting(5)));
        assert_eq!(
            modals.handle(AppEvent::StorageUpdated(StorageState::Failed)),
            ModalOutcome::Dismissed { deliver: false }
        );
    }

    /// Only the *same* prompt refreshes. A different one replacing it has to
    /// be drawn whole, or the screen underneath it would show through.
    #[test]
    fn a_different_prompt_taking_over_is_drawn_whole() {
        let mut modals = ModalState::new();
        let _ = modals.handle(AppEvent::BleUpdated(BleState::DfuProgress(40)));

        assert_eq!(
            modals.handle(AppEvent::BleUpdated(BleState::DfuFailed(
                DfuFailReason::EraseFailed
            ))),
            ModalOutcome::Show(Modal::DfuFailed(DfuFailReason::EraseFailed))
        );
        // And a prompt that comes back after being dismissed is new again.
        let _ = modals.handle(SWIPE);
        assert_eq!(
            modals.handle(AppEvent::BleUpdated(BleState::DfuProgress(1))),
            ModalOutcome::Show(Modal::DfuProgress(1))
        );
    }

    #[test]
    fn a_format_interrupts_a_showing_modal() {
        let mut modals = ModalState::new();
        let _ = modals.handle(AppEvent::BleUpdated(BleState::Pairing(42)));
        assert_eq!(
            modals.handle(AppEvent::StorageUpdated(StorageState::Formatting(1))),
            ModalOutcome::Show(Modal::StorageFormat(1))
        );
        assert_eq!(modals.current(), Some(Modal::StorageFormat(1)));
    }
}
