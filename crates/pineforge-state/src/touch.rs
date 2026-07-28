//! Turning touch controller reports into the events a screen sees.
//!
//! Two things happen between the panel and a screen, and both are the kind of
//! rule that fails quietly: deciding when a finger's travel amounts to a
//! gesture, and deciding which of that finger's reports a screen should still
//! be told about once it does.
//!
//! Both used to live in the input task, where nothing could test them. Two
//! faults came out of that in one afternoon - the first tap after a swipe was
//! eaten as a repeat of that swipe, and a finger lifting after a gesture was
//! never reported at all, so anything tracking contact stayed stuck believing
//! it was still down. Both are regression-tested at the foot of this file.

use heapless::Vec;

use crate::{AppEvent, SwipeDirection};

/// The events one report turns into, in order.
///
/// Two at most: a gesture withdraws the press it grew out of before announcing
/// itself, and nothing else emits more than one.
pub type TouchEvents = Vec<AppEvent, 2>;

/// One report from the touch controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TouchReport {
    pub x: i32,
    pub y: i32,
    /// Whether a finger is in contact. A report with this false is the finger
    /// lifting.
    pub touching: bool,
    /// What the controller's gesture register says, if it names a direction.
    /// Treated with suspicion; see [`SwipeRecognizer::update`].
    pub gesture: Option<SwipeDirection>,
}

/// Decides what a screen is told about each touch report.
///
/// Owns the gesture recognition and the rule that follows from it: a gesture
/// consumes the touch it was recognised from, so the moves after it stay off
/// the screen. Otherwise the minimum swipe distance - which fits inside a single
/// menu row - would let a swipe activate the control the finger started on, and
/// swiping back out of the firmware screen would reboot the watch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TouchRouter {
    recognizer: SwipeRecognizer,
    /// Set once a gesture has claimed the touch in flight.
    gesture_claimed: bool,
}

impl TouchRouter {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            recognizer: SwipeRecognizer::new(),
            gesture_claimed: false,
        }
    }

    /// What this report means for the screen.
    pub fn report(&mut self, report: TouchReport) -> TouchEvents {
        let mut events = TouchEvents::new();
        let swipe = self
            .recognizer
            .update(report.x, report.y, report.touching, report.gesture);

        if let Some(direction) = swipe {
            if !self.gesture_claimed {
                self.gesture_claimed = true;
                // Withdraws the press already delivered, so the control the
                // finger came down on is released rather than activated.
                let _ = events.push(AppEvent::TouchCancelled);
            }
            let _ = events.push(AppEvent::Swipe(direction));
        } else if !self.gesture_claimed || !report.touching {
            // The lift goes out even when a gesture claimed this touch. It can
            // activate nothing - the cancel above already released everything -
            // and withholding it leaves anything that tracks contact waiting
            // for an event that would never come.
            let _ = events.push(AppEvent::Touch {
                x: report.x,
                y: report.y,
                pressed: report.touching,
            });
        }

        if !report.touching {
            self.gesture_claimed = false;
        }
        events
    }

    /// A report that could not be read.
    ///
    /// The lost report may have been the finger lifting, and stale tracking
    /// would then suppress or misdirect the next touch entirely. Forgetting the
    /// one in flight costs at worst a gesture the user has to repeat.
    pub const fn lost_report(&mut self) {
        self.recognizer.reset();
        self.gesture_claimed = false;
    }
}

const SWIPE_MIN_DISTANCE: i32 = 40;

/// Combines controller-provided gestures with a coordinate-based fallback.
///
/// The fallback exists to catch swipes the `CST816S` gesture register missed,
/// so it must not be *stricter* than the hardware it stands in for. It used to
/// demand the travelled axis beat the other one by half again, which left a
/// dead wedge around the diagonals: a swipe of 50 across and 40 down satisfied
/// neither axis and was dropped, and fingers cross a round screen diagonally
/// all the time. Now the longer axis simply wins and only the minimum distance
/// has to be cleared, which is what keeps a tap from reading as a swipe.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SwipeRecognizer {
    start: Option<(i32, i32)>,
    emitted: bool,
}

impl SwipeRecognizer {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            start: None,
            emitted: false,
        }
    }

    pub fn update(
        &mut self,
        x: i32,
        y: i32,
        pressed: bool,
        controller_gesture: Option<SwipeDirection>,
    ) -> Option<SwipeDirection> {
        // Whether a finger was already down when this report arrived. A touch's
        // very first report cannot carry a gesture of its own - see below.
        let tracking = self.start.is_some();

        if !pressed {
            // Fast flicks can deliver only a press and a release report, so the
            // release is checked for a swipe the moves in between never showed.
            //
            // From the distance the finger covered, and not from the
            // controller's gesture register: the register still holds the
            // previous swipe at this point, and on a release there is nothing
            // to tell the two apart. The distance can - a tap has not moved.
            let gesture = self
                .start
                .and_then(|start| Self::direction_from_delta(x - start.0, y - start.1))
                .filter(|_| !self.emitted);
            self.reset();
            return gesture;
        }

        let start = *self.start.get_or_insert((x, y));
        if self.emitted {
            return None;
        }
        // The controller's gesture register holds its last value until a new
        // gesture replaces it. So a gesture arriving together with the finger's
        // *first* report describes the previous touch, not this one - the finger
        // has only just landed and cannot have travelled anywhere yet.
        //
        // Trusting it turned the first tap after any swipe into a repeat of that
        // swipe: the tap was consumed as a gesture and never reached the screen,
        // so a tile had to be pressed twice. Ignoring it here costs nothing - a
        // real slide is still reported from the second report onwards, and the
        // coordinate fallback below covers the same ground.
        if let Some(gesture) = controller_gesture.filter(|_| tracking) {
            self.emitted = true;
            return Some(gesture);
        }

        let direction = Self::direction_from_delta(x - start.0, y - start.1);
        self.emitted = direction.is_some();
        direction
    }

    const fn direction_from_delta(delta_x: i32, delta_y: i32) -> Option<SwipeDirection> {
        let horizontal = delta_x.abs();
        let vertical = delta_y.abs();
        // An exact tie names no direction, so it is the one case refused
        // outright rather than resolved by an arbitrary preference.
        if horizontal > vertical && horizontal >= SWIPE_MIN_DISTANCE {
            Some(if delta_x < 0 {
                SwipeDirection::Left
            } else {
                SwipeDirection::Right
            })
        } else if vertical > horizontal && vertical >= SWIPE_MIN_DISTANCE {
            Some(if delta_y < 0 {
                SwipeDirection::Up
            } else {
                SwipeDirection::Down
            })
        } else {
            None
        }
    }

    pub const fn reset(&mut self) {
        self.start = None;
        self.emitted = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A finger landing, with whatever the gesture register happens to hold.
    fn down(x: i32, y: i32, gesture: Option<SwipeDirection>) -> TouchReport {
        TouchReport {
            x,
            y,
            touching: true,
            gesture,
        }
    }

    fn up(x: i32, y: i32, gesture: Option<SwipeDirection>) -> TouchReport {
        TouchReport {
            x,
            y,
            touching: false,
            gesture,
        }
    }

    fn touch(x: i32, y: i32, pressed: bool) -> AppEvent {
        AppEvent::Touch { x, y, pressed }
    }

    #[test]
    fn swipe_recognizer_falls_back_to_dominant_coordinate_motion() {
        let mut recognizer = SwipeRecognizer::new();

        assert_eq!(recognizer.update(120, 190, true, None), None);
        assert_eq!(
            recognizer.update(118, 145, true, None),
            Some(SwipeDirection::Up)
        );
        assert_eq!(recognizer.update(116, 90, true, None), None);
        assert_eq!(recognizer.update(0, 0, false, None), None);
    }

    #[test]
    fn swipe_recognizer_prefers_hardware_and_preserves_taps() {
        let mut recognizer = SwipeRecognizer::new();

        assert_eq!(recognizer.update(100, 100, true, None), None);
        assert_eq!(recognizer.update(105, 103, true, None), None);
        assert_eq!(recognizer.update(105, 103, false, None), None);
        assert_eq!(recognizer.update(180, 100, true, None), None);
        // Reported by the controller before the coordinates alone would carry
        // it, and taken because a finger is already down.
        assert_eq!(
            recognizer.update(170, 100, true, Some(SwipeDirection::Left)),
            Some(SwipeDirection::Left)
        );
        assert_eq!(
            recognizer.update(100, 100, true, Some(SwipeDirection::Left)),
            None
        );
    }

    /// The regression behind "the first tap after a swipe does nothing".
    ///
    /// The `CST816S` leaves its last gesture standing in the register, so the
    /// report that lands a new finger still reads as the previous swipe. Taken
    /// at face value, the tap became another swipe and never reached a screen -
    /// which is why a launcher tile had to be pressed twice after swiping up.
    #[test]
    fn a_gesture_arriving_with_the_first_report_belongs_to_the_previous_touch() {
        let mut recognizer = SwipeRecognizer::new();

        // A swipe up, taken from the coordinates.
        assert_eq!(recognizer.update(120, 190, true, None), None);
        assert_eq!(
            recognizer.update(120, 140, true, Some(SwipeDirection::Up)),
            Some(SwipeDirection::Up)
        );
        assert_eq!(
            recognizer.update(120, 140, false, Some(SwipeDirection::Up)),
            None
        );

        // A tap. The register still says "up" and must be disbelieved: nothing
        // has moved, so there is no gesture to report.
        assert_eq!(
            recognizer.update(60, 60, true, Some(SwipeDirection::Up)),
            None
        );
        assert_eq!(
            recognizer.update(60, 61, false, Some(SwipeDirection::Up)),
            None
        );
    }

    /// A lone release with a stale gesture and no press behind it - what a
    /// dropped report leaves - must not be read as a swipe either.
    #[test]
    fn a_release_without_a_press_carries_no_gesture() {
        let mut recognizer = SwipeRecognizer::new();

        assert_eq!(
            recognizer.update(60, 60, false, Some(SwipeDirection::Right)),
            None
        );
    }

    #[test]
    fn swipe_recognizer_derives_fast_flicks_from_the_release_report() {
        let mut recognizer = SwipeRecognizer::new();

        assert_eq!(recognizer.update(200, 120, true, None), None);
        assert_eq!(recognizer.update(190, 118, true, None), None);
        assert_eq!(
            recognizer.update(60, 120, false, None),
            Some(SwipeDirection::Left)
        );
        assert_eq!(recognizer.update(100, 100, true, None), None);
        assert_eq!(recognizer.update(100, 100, false, None), None);
    }

    /// The regression for "swipes get swallowed".
    ///
    /// A finger crossing a round screen rarely travels along an axis. The old
    /// rule wanted the moved axis to beat the other by half again, so a swipe
    /// of 50 across and 40 down cleared the minimum distance on both axes and
    /// was still dropped by both arms of the test - the fallback meant to
    /// recover missed hardware gestures was refusing more than the hardware.
    #[test]
    fn a_diagonal_swipe_resolves_to_its_longer_axis() {
        let cases = [
            ((50, 40), SwipeDirection::Right),
            ((-50, 40), SwipeDirection::Left),
            ((40, 50), SwipeDirection::Down),
            ((40, -50), SwipeDirection::Up),
        ];
        for ((delta_x, delta_y), expected) in cases {
            let mut recognizer = SwipeRecognizer::new();
            assert_eq!(recognizer.update(120, 120, true, None), None);
            assert_eq!(
                recognizer.update(120 + delta_x, 120 + delta_y, true, None),
                Some(expected),
                "delta ({delta_x}, {delta_y})"
            );
        }
    }

    #[test]
    fn a_short_move_is_still_a_tap_whichever_way_it_leans() {
        // Relaxing the axis rule must not lower the bar that separates a swipe
        // from a tap: the minimum distance still has to be cleared.
        for (delta_x, delta_y) in [(30, 25), (25, 30), (39, 0), (0, 39), (39, 39)] {
            let mut recognizer = SwipeRecognizer::new();
            assert_eq!(recognizer.update(100, 100, true, None), None);
            assert_eq!(
                recognizer.update(100 + delta_x, 100 + delta_y, true, None),
                None,
                "delta ({delta_x}, {delta_y})"
            );
        }
    }

    #[test]
    fn an_exactly_diagonal_swipe_names_no_direction() {
        let mut recognizer = SwipeRecognizer::new();
        assert_eq!(recognizer.update(100, 100, true, None), None);
        assert_eq!(recognizer.update(160, 160, true, None), None);
    }

    #[test]
    fn swipe_recognizer_reset_discards_stale_tracking() {
        let mut recognizer = SwipeRecognizer::new();

        assert_eq!(recognizer.update(100, 100, true, None), None);
        assert_eq!(
            recognizer.update(180, 100, true, None),
            Some(SwipeDirection::Right)
        );
        // The release report was lost, e.g. due to invalid coordinates.
        recognizer.reset();
        assert_eq!(recognizer.update(120, 200, true, None), None);
        assert_eq!(
            recognizer.update(120, 120, true, None),
            Some(SwipeDirection::Up)
        );
    }

    #[test]
    fn a_tap_reaches_the_screen_as_a_press_and_a_release() {
        let mut router = TouchRouter::new();

        assert_eq!(
            router.report(down(60, 60, None)).as_slice(),
            [touch(60, 60, true)]
        );
        assert_eq!(
            router.report(up(60, 61, None)).as_slice(),
            [touch(60, 61, false)]
        );
    }

    #[test]
    fn a_gesture_withdraws_the_press_it_grew_out_of() {
        let mut router = TouchRouter::new();
        let _ = router.report(down(120, 190, None));

        assert_eq!(
            router.report(down(120, 140, None)).as_slice(),
            [
                AppEvent::TouchCancelled,
                AppEvent::Swipe(SwipeDirection::Up)
            ]
        );
        // The moves that follow stay off the screen: the minimum swipe distance
        // fits inside one menu row, so delivering them would let the swipe
        // operate the control it started on.
        assert!(router.report(down(120, 100, None)).is_empty());
    }

    /// The regression for a finger that lifts after a gesture.
    ///
    /// The lift used to be suppressed along with the moves, so the last thing
    /// any screen heard was "pressed" and nothing ever took it back. Anything
    /// tracking contact stayed stuck until the next touch happened to clear it.
    #[test]
    fn a_finger_lifting_is_reported_even_when_a_gesture_took_the_touch() {
        let mut router = TouchRouter::new();
        let _ = router.report(down(120, 190, None));
        let _ = router.report(down(120, 140, None));

        assert_eq!(
            router.report(up(120, 130, None)).as_slice(),
            [touch(120, 130, false)]
        );
    }

    /// The regression for the first tap after a swipe.
    ///
    /// The `CST816S` leaves its last gesture standing in the register, so the
    /// report that lands the next finger still reads as the previous swipe.
    /// Taken at face value the tap became another swipe and never reached a
    /// screen - which is why a launcher tile had to be pressed twice.
    #[test]
    fn the_tap_after_a_swipe_is_a_tap() {
        let mut router = TouchRouter::new();
        let _ = router.report(down(120, 190, None));
        let _ = router.report(down(120, 140, Some(SwipeDirection::Up)));
        let _ = router.report(up(120, 140, Some(SwipeDirection::Up)));

        // The register still says "up". Nothing has moved, so nothing happened.
        assert_eq!(
            router
                .report(down(60, 60, Some(SwipeDirection::Up)))
                .as_slice(),
            [touch(60, 60, true)]
        );
        assert_eq!(
            router
                .report(up(60, 61, Some(SwipeDirection::Up)))
                .as_slice(),
            [touch(60, 61, false)]
        );
    }

    #[test]
    fn a_lost_report_leaves_nothing_of_the_touch_in_flight() {
        let mut router = TouchRouter::new();
        let _ = router.report(down(120, 190, None));
        let _ = router.report(down(120, 140, None));
        // The lift was the report that went missing.
        router.lost_report();

        // A fresh finger is tracked from scratch rather than suppressed as the
        // tail of a gesture that already ended.
        assert_eq!(
            router.report(down(60, 60, None)).as_slice(),
            [touch(60, 60, true)]
        );
    }
}
