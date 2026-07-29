//! Which setting a picker edits, and what choosing one of its presets means.
//!
//! This is product logic rather than drawing: reading a preset out of a record
//! and writing one back is the same decision whatever the screen looks like,
//! and it is the part that can be got wrong silently. It lives here so it can
//! be tested on the host, which is how the stale-snapshot bug below is kept
//! from coming back.

use crate::{
    BRIGHTNESS_LEVELS, BRIGHTNESS_NAMES, DIM_TIMEOUT_NAMES, DIM_TIMEOUTS_MILLIS, DisplaySettings,
    HEART_RATE_INTERVALS_SECONDS, HEART_RATE_MODE_NAMES, OFF_TIMEOUT_NAMES, OFF_TIMEOUTS_MILLIS,
    WAKE_GESTURE_NAMES, WAKE_GESTURES, WATCHFACES,
};

/// The faces' names, in the order a picker lists them.
const fn watchface_names<const N: usize>() -> [&'static str; N] {
    let mut names = [""; N];
    let mut index = 0;
    while index < N {
        names[index] = WATCHFACES[index].name;
        index += 1;
    }
    names
}

pub const WATCHFACE_NAMES: [&str; WATCHFACES.len()] = watchface_names();

/// A setting a picker can offer the presets of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Brightness,
    DimTimeout,
    OffTimeout,
    HeartRate,
    WakeGesture,
    Watchface,
}

impl Setting {
    /// The names of this setting's presets, in the order they are offered.
    #[must_use]
    pub const fn names(self) -> &'static [&'static str] {
        match self {
            Self::Brightness => &BRIGHTNESS_NAMES,
            Self::DimTimeout => &DIM_TIMEOUT_NAMES,
            Self::OffTimeout => &OFF_TIMEOUT_NAMES,
            Self::HeartRate => &HEART_RATE_MODE_NAMES,
            Self::WakeGesture => &WAKE_GESTURE_NAMES,
            Self::Watchface => &WATCHFACE_NAMES,
        }
    }

    /// What the screen offering these presets is called.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Brightness => "BRIGHTNESS",
            Self::DimTimeout => "DIM AFTER",
            Self::OffTimeout => "SCREEN OFF",
            Self::HeartRate => "HEART RATE",
            Self::WakeGesture => "WAKE GESTURE",
            Self::Watchface => "WATCHFACE",
        }
    }

    /// Which single-choice preset a record currently holds, if it is one of
    /// them.
    ///
    /// A value the table does not list reports `None` rather than the first
    /// preset, so a record written by another build cannot make a picker claim
    /// a preset it is not using.
    #[must_use]
    pub fn selected(self, settings: DisplaySettings) -> Option<usize> {
        match self {
            Self::Brightness => BRIGHTNESS_LEVELS
                .iter()
                .position(|level| *level == settings.brightness()),
            Self::DimTimeout => DIM_TIMEOUTS_MILLIS
                .iter()
                .position(|millis| *millis == settings.dim_after_millis()),
            Self::OffTimeout => OFF_TIMEOUTS_MILLIS
                .iter()
                .position(|millis| *millis == settings.off_after_millis()),
            // Off is the first preset; every other one names an interval, so
            // the record's two fields collapse onto one row here.
            Self::HeartRate => {
                if settings.heart_rate_enabled() {
                    HEART_RATE_INTERVALS_SECONDS
                        .iter()
                        .position(|seconds| *seconds == settings.heart_rate_interval_seconds())
                        .map(|interval| interval + 1)
                } else {
                    Some(0)
                }
            }
            Self::WakeGesture => None,
            Self::Watchface => WATCHFACES
                .iter()
                .position(|face| face.id == settings.watchface()),
        }
    }

    /// A bit per row that is currently selected.
    ///
    /// Ordinary pickers set one bit. Wake sources are independent switches and
    /// can therefore set any combination of their three bits.
    #[must_use]
    pub fn selection_mask(self, settings: DisplaySettings) -> u32 {
        if self == Self::WakeGesture {
            let mut mask = 0;
            for (entry, gesture) in WAKE_GESTURES.iter().enumerate() {
                if settings.wake_gestures().contains(*gesture) {
                    mask |= 1_u32 << entry;
                }
            }
            mask
        } else {
            self.selected(settings).map_or(0, |entry| 1_u32 << entry)
        }
    }

    /// `settings` after activating its `entry`-th row, or `None` when nothing
    /// would change.
    ///
    /// Most settings choose one preset. A wake row instead toggles that source
    /// without changing the other wake sources.
    ///
    /// Takes the whole record and returns a whole record on purpose. Every
    /// picker must start from the record that is *current*, not from one it
    /// captured earlier: a picker applying a preset to a stale copy silently
    /// reverts every setting somebody changed in between.
    #[must_use]
    pub fn apply(self, settings: DisplaySettings, entry: usize) -> Option<DisplaySettings> {
        let updated = match self {
            Self::Brightness => settings.with_brightness(*BRIGHTNESS_LEVELS.get(entry)?),
            Self::DimTimeout => settings.with_dim_timeout(*DIM_TIMEOUTS_MILLIS.get(entry)?),
            Self::OffTimeout => settings.with_off_timeout(*OFF_TIMEOUTS_MILLIS.get(entry)?),
            // Choosing off leaves the interval where it was, so turning
            // measurement back on returns to the rate that was picked rather
            // than to a default.
            Self::HeartRate => match entry.checked_sub(1) {
                None => settings.with_heart_rate_enabled(false),
                Some(interval) => settings
                    .with_heart_rate_enabled(true)
                    .with_heart_rate_interval(*HEART_RATE_INTERVALS_SECONDS.get(interval)?),
            },
            Self::WakeGesture => settings.toggle_wake_gesture(*WAKE_GESTURES.get(entry)?),
            Self::Watchface => settings.with_watchface(WATCHFACES.get(entry)?.id),
        };
        (updated != settings).then_some(updated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Applies a setting's preset by name, the way a picker's row does.
    fn choose(settings: DisplaySettings, setting: Setting, name: &str) -> DisplaySettings {
        let entry = setting
            .names()
            .iter()
            .position(|preset| *preset == name)
            .expect("the preset is offered");
        setting.apply(settings, entry).unwrap_or(settings)
    }

    #[test]
    fn a_preset_that_is_taken_is_the_one_reported_back() {
        for setting in [
            Setting::Brightness,
            Setting::DimTimeout,
            Setting::OffTimeout,
            Setting::HeartRate,
            Setting::Watchface,
        ] {
            for entry in 0..setting.names().len() {
                // `None` means the record did not move: either the preset was
                // already current, or a setter refused it. An off timeout that
                // is not later than dimming is refused, so this cannot assert
                // that every entry is reachable from every record.
                let Some(updated) = setting.apply(DisplaySettings::DEFAULT, entry) else {
                    continue;
                };
                assert_eq!(
                    setting.selected(updated),
                    Some(entry),
                    "{setting:?} lost track of entry {entry}"
                );
            }
        }
    }

    #[test]
    fn wake_rows_toggle_independently_and_report_every_marker() {
        let single_and_tilt = choose(
            DisplaySettings::DEFAULT,
            Setting::WakeGesture,
            "RAISE WRIST",
        );
        assert_eq!(Setting::WakeGesture.selection_mask(single_and_tilt), 0b101);

        let tilt_only = choose(single_and_tilt, Setting::WakeGesture, "SINGLE TAP");
        assert_eq!(Setting::WakeGesture.selection_mask(tilt_only), 0b100);

        let none = choose(tilt_only, Setting::WakeGesture, "RAISE WRIST");
        assert_eq!(Setting::WakeGesture.selection_mask(none), 0);
    }

    /// The regression this module exists for.
    ///
    /// Two pickers changed one after the other, each starting from the record
    /// the previous one produced. Both changes have to survive.
    #[test]
    fn one_picker_does_not_discard_another_pickers_change() {
        let settings = DisplaySettings::DEFAULT;
        let bright = choose(settings, Setting::Brightness, "LOW");
        let both = choose(bright, Setting::HeartRate, "5 min");

        assert_eq!(
            both.brightness(),
            bright.brightness(),
            "the heart rate picker reverted the brightness"
        );
        assert!(both.heart_rate_enabled());
    }

    /// What the bug looked like, so the shape of it is on the record.
    ///
    /// A picker that kept its own snapshot applied its preset to the record as
    /// it was when the picker last saw it. Everything changed in between was
    /// written back to the older value.
    #[test]
    fn a_stale_snapshot_is_what_loses_a_change() {
        let stale = DisplaySettings::DEFAULT;
        // LOW rather than FULL: FULL is already the default, so choosing
        // it would not be a change to lose.
        let current = choose(stale, Setting::Brightness, "LOW");
        assert_ne!(current.brightness(), stale.brightness());

        // The heart rate picker still holding `stale` produces a record whose
        // brightness is the old one - this is the data loss, and it is why the
        // display task keeps one canonical record instead of one per screen.
        let from_stale = choose(stale, Setting::HeartRate, "5 min");
        assert_eq!(from_stale.brightness(), stale.brightness());
        assert_ne!(from_stale.brightness(), current.brightness());

        // Applied to the current record instead, both changes stand.
        let from_current = choose(current, Setting::HeartRate, "5 min");
        assert_eq!(from_current.brightness(), current.brightness());
        assert!(from_current.heart_rate_enabled());
    }

    /// Off and the intervals share one list, so the two record fields it
    /// stands for have to survive the round trip in both directions.
    #[test]
    fn turning_measurement_off_and_on_keeps_the_interval_that_was_picked() {
        let every_ten = choose(DisplaySettings::DEFAULT, Setting::HeartRate, "10 min");
        assert!(every_ten.heart_rate_enabled());
        assert_eq!(every_ten.heart_rate_interval_seconds(), 600);

        // Off leaves the interval alone, so coming back does not land on a
        // default the user never chose.
        let off = choose(every_ten, Setting::HeartRate, "OFF");
        assert!(!off.heart_rate_enabled());
        assert_eq!(off.heart_rate_interval_seconds(), 600);
        assert_eq!(Setting::HeartRate.selected(off), Some(0));

        // Continuous is an interval like any other, not a mode beside them.
        let continuous = choose(off, Setting::HeartRate, "CONT");
        assert!(continuous.heart_rate_enabled());
        assert_eq!(continuous.heart_rate_interval_seconds(), 0);
    }

    #[test]
    fn a_value_outside_the_presets_marks_nothing() {
        // Dimming and the screen turning off are coupled, so setting a dim
        // timeout can move the off timeout to a value that is still a preset.
        // Every setting must report its own preset back regardless.
        let settings = choose(DisplaySettings::DEFAULT, Setting::DimTimeout, "30 s");
        assert!(Setting::DimTimeout.selected(settings).is_some());
        assert!(Setting::OffTimeout.selected(settings).is_some());
    }

    #[test]
    fn an_entry_past_the_last_preset_changes_nothing() {
        for setting in [Setting::Brightness, Setting::Watchface] {
            assert_eq!(
                setting.apply(DisplaySettings::DEFAULT, setting.names().len()),
                None
            );
        }
    }
}
