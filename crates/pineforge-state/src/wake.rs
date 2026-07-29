//! Hardware-independent raise-to-wake recognition.
//!
//! The accelerometer service supplies samples in the `PineTime`'s wrist-oriented
//! axes. This keeps the thresholds and transition history host-testable instead
//! of burying a product decision in an I²C task.

use crate::AccelerationSample;

// InfiniTime keeps eight 10 Hz samples. Its current orientation is the mean of
// the newest two and its previous orientation is the mean of the oldest two,
// giving a deliberate wrist turn roughly 0.6 seconds to develop.
const HISTORY_LENGTH: usize = 8;
const CURRENT_WINDOW_START: usize = HISTORY_LENGTH - 2;
const MAX_VIEW_X: i32 = 384;
const MAX_STABLE_DELTA: i32 = 112;
const MAX_VIEW_Y: i32 = -64;
const FACE_DOWN_Y: i32 = -724;

#[derive(Clone, Copy)]
struct Vector {
    x: i32,
    y: i32,
    z: i32,
}

/// Detects a stable wrist rotation of more than roughly 45 degrees into the
/// viewing orientation used by `InfiniTime` on `PineTime`.
pub struct RaiseToWakeDetector {
    samples: [AccelerationSample; HISTORY_LENGTH],
    count: usize,
}

impl RaiseToWakeDetector {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            samples: [AccelerationSample { x: 0, y: 0, z: 0 }; HISTORY_LENGTH],
            count: 0,
        }
    }

    pub const fn reset(&mut self) {
        self.count = 0;
    }

    /// Adds one 10 Hz sample and reports a completed raise gesture.
    pub fn push(&mut self, sample: AccelerationSample) -> bool {
        self.samples.copy_within(1..HISTORY_LENGTH, 0);
        self.samples[HISTORY_LENGTH - 1] = sample;
        self.count = self.count.saturating_add(1).min(HISTORY_LENGTH);
        if self.count < HISTORY_LENGTH {
            return false;
        }

        let previous = mean(self.samples[0], self.samples[1]);
        let current = mean(
            self.samples[CURRENT_WINDOW_START],
            self.samples[CURRENT_WINDOW_START + 1],
        );
        let current_delta = delta(
            self.samples[CURRENT_WINDOW_START],
            self.samples[CURRENT_WINDOW_START + 1],
        );

        if current.x.abs() > MAX_VIEW_X
            || current.y >= MAX_VIEW_Y
            || current_delta.y.abs() > MAX_STABLE_DELTA
            || (current.y < FACE_DOWN_Y && current_delta.z.abs() > MAX_STABLE_DELTA)
        {
            return false;
        }

        // Signed angle comparison without trigonometry. For the Y/Z vectors,
        // a negative cross product is a raise in the PineTime's mounted
        // orientation. |cross| > dot means the rotation exceeded 45 degrees;
        // a non-positive dot already means at least 90 degrees.
        let cross = i64::from(previous.y) * i64::from(current.z)
            - i64::from(previous.z) * i64::from(current.y);
        let dot = i64::from(previous.y) * i64::from(current.y)
            + i64::from(previous.z) * i64::from(current.z);
        cross < 0 && (dot <= 0 || -cross > dot)
    }
}

impl Default for RaiseToWakeDetector {
    fn default() -> Self {
        Self::new()
    }
}

const fn mean(a: AccelerationSample, b: AccelerationSample) -> Vector {
    Vector {
        x: i32::midpoint(a.x as i32, b.x as i32),
        y: i32::midpoint(a.y as i32, b.y as i32),
        z: i32::midpoint(a.z as i32, b.z as i32),
    }
}

const fn delta(a: AccelerationSample, b: AccelerationSample) -> Vector {
    Vector {
        x: a.x as i32 - b.x as i32,
        y: a.y as i32 - b.y as i32,
        z: a.z as i32 - b.z as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn sample(x: i16, y: i16, z: i16) -> AccelerationSample {
        AccelerationSample { x, y, z }
    }

    #[test]
    fn a_raise_into_the_viewing_angle_wakes() {
        let mut detector = RaiseToWakeDetector::new();
        for value in [
            sample(0, -1_024, 0),
            sample(0, -1_024, 0),
            sample(0, -1_024, 0),
            sample(0, -1_024, 0),
            sample(0, -1_024, 0),
            sample(0, -1_024, 0),
            sample(0, -512, 887),
        ] {
            assert!(!detector.push(value));
        }
        assert!(detector.push(sample(0, -512, 887)));
    }

    #[test]
    fn a_gradual_raise_across_infinitimes_history_window_wakes() {
        let mut detector = RaiseToWakeDetector::new();
        for value in [
            sample(0, -1_024, 0),
            sample(0, -1_000, 200),
            sample(0, -950, 380),
            sample(0, -850, 570),
            sample(0, -724, 724),
            sample(0, -630, 800),
            sample(0, -560, 850),
        ] {
            assert!(!detector.push(value));
        }
        assert!(detector.push(sample(0, -512, 887)));
    }

    #[test]
    fn holding_the_same_angle_does_not_wake() {
        let mut detector = RaiseToWakeDetector::new();
        for _ in 0..12 {
            assert!(!detector.push(sample(0, -512, 887)));
        }
    }

    #[test]
    fn lowering_the_wrist_is_not_a_raise() {
        let mut detector = RaiseToWakeDetector::new();
        for value in [
            sample(0, -512, 887),
            sample(0, -512, 887),
            sample(0, -512, 887),
            sample(0, -512, 887),
            sample(0, -512, 887),
            sample(0, -512, 887),
            sample(0, -1_024, 0),
            sample(0, -1_024, 0),
        ] {
            assert!(!detector.push(value));
        }
    }

    #[test]
    fn an_unstable_or_sideways_wrist_is_rejected() {
        let mut unstable = RaiseToWakeDetector::new();
        for value in [
            sample(0, -1_024, 0),
            sample(0, -1_024, 0),
            sample(0, -900, 300),
            sample(0, -800, 500),
            sample(0, -700, 650),
            sample(0, -600, 800),
            sample(0, -300, 900),
            sample(0, -700, 500),
        ] {
            assert!(!unstable.push(value));
        }

        let mut sideways = RaiseToWakeDetector::new();
        for value in [
            sample(700, -1_024, 0),
            sample(700, -1_024, 0),
            sample(700, -900, 300),
            sample(700, -800, 500),
            sample(700, -700, 650),
            sample(700, -600, 800),
            sample(700, -512, 887),
            sample(700, -512, 887),
        ] {
            assert!(!sideways.push(value));
        }
    }
}
