//! The watch's current readings, shared by every watchface.
//!
//! A watchface renders this; it does not collect it. That split is what makes
//! a second face cost only its drawing code: the values arrive once, from the
//! event stream, no matter which face is showing them.
//!
//! Applying an event reports which fields moved, so a face can redraw just the
//! part of its layout that changed without tracking previous values itself.

#[cfg(feature = "diagnostics")]
use crate::{
    AccelerationSample, AccelerometerKind, FeatureEngineStatus, HeartRateRawSample,
    HeartRateSensorKind, PpgAnalysis,
};
use crate::{AppEvent, BatteryStatus, BleState, CalendarDate, WallTime};
use crate::{HeartRateState, NotificationCategory};

/// A part of the watch state, addressable by a face's layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchField {
    /// The time of day, or the uptime standing in for it before the first
    /// synchronization.
    Clock,
    Date,
    Battery,
    Steps,
    HeartRate,
    Ble,
    Notifications,
    /// Accelerometer readings, carried in diagnostics builds only.
    Motion,
}

impl WatchField {
    const fn mask(self) -> u16 {
        1 << self as u16
    }
}

/// The set of fields a single event moved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WatchFields(u16);

impl WatchFields {
    pub const NONE: Self = Self(0);

    #[must_use]
    pub const fn of(field: WatchField) -> Self {
        Self(field.mask())
    }

    #[must_use]
    pub const fn with(self, field: WatchField) -> Self {
        Self(self.0 | field.mask())
    }

    #[must_use]
    pub const fn contains(self, field: WatchField) -> bool {
        self.0 & field.mask() != 0
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// Every reading a watchface can show, fed from the event stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WatchState {
    clock_seconds: u64,
    previous_clock_seconds: u64,
    date: Option<CalendarDate>,
    battery: Option<BatteryStatus>,
    steps: Option<u32>,
    ble: BleState,
    heart_rate: HeartRateState,
    notifications: u32,
    last_category: Option<NotificationCategory>,
    #[cfg(feature = "diagnostics")]
    accelerometer: Option<AccelerometerKind>,
    #[cfg(feature = "diagnostics")]
    acceleration: Option<AccelerationSample>,
    #[cfg(feature = "diagnostics")]
    feature_engine: Option<FeatureEngineStatus>,
    #[cfg(feature = "diagnostics")]
    heart_rate_sensor: Option<HeartRateSensorKind>,
    #[cfg(feature = "diagnostics")]
    heart_rate_raw: Option<HeartRateRawSample>,
    #[cfg(feature = "diagnostics")]
    heart_rate_analysis: Option<PpgAnalysis>,
    changed: WatchFields,
}

impl Default for WatchState {
    fn default() -> Self {
        Self::new()
    }
}

impl WatchState {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            clock_seconds: 0,
            previous_clock_seconds: 0,
            date: None,
            battery: None,
            steps: None,
            ble: BleState::Off,
            heart_rate: HeartRateState::Disabled,
            notifications: 0,
            last_category: None,
            #[cfg(feature = "diagnostics")]
            accelerometer: None,
            #[cfg(feature = "diagnostics")]
            acceleration: None,
            #[cfg(feature = "diagnostics")]
            feature_engine: None,
            #[cfg(feature = "diagnostics")]
            heart_rate_sensor: None,
            #[cfg(feature = "diagnostics")]
            heart_rate_raw: None,
            #[cfg(feature = "diagnostics")]
            heart_rate_analysis: None,
            changed: WatchFields::NONE,
        }
    }

    /// Applies an event and returns the fields it moved.
    ///
    /// Sensor events arrive on a timer whether or not the reading changed, so
    /// an event that moves nothing reports nothing and costs no redraw.
    pub fn apply(&mut self, event: AppEvent) -> WatchFields {
        self.changed = self.apply_event(event);
        self.changed
    }

    /// The fields the most recent [`Self::apply`] moved.
    #[must_use]
    pub const fn changed(&self) -> WatchFields {
        self.changed
    }

    fn apply_event(&mut self, event: AppEvent) -> WatchFields {
        match event {
            AppEvent::Tick {
                uptime_seconds,
                wall_time,
                date,
            } => {
                self.previous_clock_seconds = self.clock_seconds;
                // Until a phone synchronizes the clock, the uptime stands in
                // for the time of day.
                self.clock_seconds = wall_time.map_or(uptime_seconds, WallTime::total_seconds);
                let mut moved = if self.clock_seconds == self.previous_clock_seconds {
                    WatchFields::NONE
                } else {
                    WatchFields::of(WatchField::Clock)
                };
                if self.date != date {
                    self.date = date;
                    moved = moved.with(WatchField::Date);
                }
                moved
            }
            AppEvent::BatteryUpdated(status) => {
                Self::moved(&mut self.battery, Some(status), WatchField::Battery)
            }
            AppEvent::StepsUpdated(steps) => {
                Self::moved(&mut self.steps, Some(steps), WatchField::Steps)
            }
            AppEvent::BleUpdated(state) => Self::moved(&mut self.ble, state, WatchField::Ble),
            AppEvent::HeartRateStateUpdated(state) => {
                Self::moved(&mut self.heart_rate, state, WatchField::HeartRate)
            }
            // The tally is the inbox's, not a running count kept here: a face
            // that counted arrivals itself would keep showing notifications the
            // user had already cleared.
            AppEvent::NotificationsChanged(summary) => {
                let moved =
                    self.notifications != summary.count || self.last_category != summary.latest;
                self.notifications = summary.count;
                self.last_category = summary.latest;
                if moved {
                    WatchFields::of(WatchField::Notifications)
                } else {
                    WatchFields::NONE
                }
            }
            #[cfg(feature = "diagnostics")]
            AppEvent::AccelerometerDetected(kind) => {
                Self::moved(&mut self.accelerometer, Some(kind), WatchField::Motion)
            }
            #[cfg(feature = "diagnostics")]
            AppEvent::AccelerationUpdated(sample) => {
                Self::moved(&mut self.acceleration, Some(sample), WatchField::Motion)
            }
            #[cfg(feature = "diagnostics")]
            AppEvent::FeatureEngineUpdated(status) => {
                Self::moved(&mut self.feature_engine, Some(status), WatchField::Motion)
            }
            #[cfg(feature = "diagnostics")]
            AppEvent::HeartRateSensorDetected(kind) => Self::moved(
                &mut self.heart_rate_sensor,
                Some(kind),
                WatchField::HeartRate,
            ),
            #[cfg(feature = "diagnostics")]
            AppEvent::HeartRateRawSampleUpdated(raw) => {
                Self::moved(&mut self.heart_rate_raw, Some(raw), WatchField::HeartRate)
            }
            #[cfg(feature = "diagnostics")]
            AppEvent::HeartRateAnalysisUpdated(analysis) => Self::moved(
                &mut self.heart_rate_analysis,
                Some(analysis),
                WatchField::HeartRate,
            ),
            // Readings this build does not show, and events that belong to
            // navigation, power, or the settings screen.
            _ => WatchFields::NONE,
        }
    }

    fn moved<T: PartialEq>(field: &mut T, next: T, moved: WatchField) -> WatchFields {
        if *field == next {
            return WatchFields::NONE;
        }
        *field = next;
        WatchFields::of(moved)
    }

    #[must_use]
    pub const fn clock_seconds(&self) -> u64 {
        self.clock_seconds
    }

    /// The clock before the last tick, so a face can redraw only the digits
    /// that changed. A face draws from a shared reference and cannot remember
    /// what it drew, so the previous value belongs here.
    #[must_use]
    pub const fn previous_clock_seconds(&self) -> u64 {
        self.previous_clock_seconds
    }

    /// The date of the last synchronization, absent until one arrives.
    #[must_use]
    pub const fn date(&self) -> Option<CalendarDate> {
        self.date
    }

    #[must_use]
    pub const fn battery(&self) -> Option<BatteryStatus> {
        self.battery
    }

    #[must_use]
    pub const fn steps(&self) -> Option<u32> {
        self.steps
    }

    #[must_use]
    pub const fn ble(&self) -> BleState {
        self.ble
    }

    #[must_use]
    pub const fn heart_rate(&self) -> HeartRateState {
        self.heart_rate
    }

    #[must_use]
    pub const fn notifications(&self) -> u32 {
        self.notifications
    }

    #[must_use]
    pub const fn last_category(&self) -> Option<NotificationCategory> {
        self.last_category
    }

    #[cfg(feature = "diagnostics")]
    #[must_use]
    pub const fn accelerometer(&self) -> Option<AccelerometerKind> {
        self.accelerometer
    }

    #[cfg(feature = "diagnostics")]
    #[must_use]
    pub const fn acceleration(&self) -> Option<AccelerationSample> {
        self.acceleration
    }

    #[cfg(feature = "diagnostics")]
    #[must_use]
    pub const fn feature_engine(&self) -> Option<FeatureEngineStatus> {
        self.feature_engine
    }

    #[cfg(feature = "diagnostics")]
    #[must_use]
    pub const fn heart_rate_sensor(&self) -> Option<HeartRateSensorKind> {
        self.heart_rate_sensor
    }

    #[cfg(feature = "diagnostics")]
    #[must_use]
    pub const fn heart_rate_raw(&self) -> Option<HeartRateRawSample> {
        self.heart_rate_raw
    }

    #[cfg(feature = "diagnostics")]
    #[must_use]
    pub const fn heart_rate_analysis(&self) -> Option<PpgAnalysis> {
        self.heart_rate_analysis
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SwipeDirection;

    const BATTERY: BatteryStatus = BatteryStatus {
        millivolts: 3_900,
        percent: 80,
        charging: false,
        power_present: false,
    };

    const fn tick(uptime_seconds: u64) -> AppEvent {
        AppEvent::Tick {
            uptime_seconds,
            wall_time: None,
            date: None,
        }
    }

    #[test]
    fn a_reading_that_did_not_move_reports_nothing() {
        let mut state = WatchState::new();

        assert_eq!(
            state.apply(AppEvent::BatteryUpdated(BATTERY)),
            WatchFields::of(WatchField::Battery)
        );
        // The battery service publishes on a timer whether or not the value
        // moved; an unchanged reading must not cost a redraw.
        assert!(state.apply(AppEvent::BatteryUpdated(BATTERY)).is_empty());
        assert_eq!(state.battery(), Some(BATTERY));

        assert_eq!(
            state.apply(AppEvent::StepsUpdated(0)),
            WatchFields::of(WatchField::Steps)
        );
        assert!(state.apply(AppEvent::StepsUpdated(0)).is_empty());
        assert_eq!(
            state.apply(AppEvent::StepsUpdated(1)),
            WatchFields::of(WatchField::Steps)
        );
    }

    #[test]
    fn a_tick_moves_the_clock_and_the_date_independently() {
        let mut state = WatchState::new();

        assert_eq!(state.apply(tick(1)), WatchFields::of(WatchField::Clock));
        assert_eq!(state.clock_seconds(), 1);
        assert_eq!(state.previous_clock_seconds(), 0);
        // A repeated second - the wake path emits one - moves nothing.
        assert!(state.apply(tick(1)).is_empty());

        let date = CalendarDate {
            year: 2026,
            month: 7,
            day: 25,
        };
        let synchronized = AppEvent::Tick {
            uptime_seconds: 2,
            wall_time: Some(WallTime {
                hour: 8,
                minute: 0,
                second: 0,
            }),
            date: Some(date),
        };
        assert_eq!(
            state.apply(synchronized),
            WatchFields::NONE
                .with(WatchField::Clock)
                .with(WatchField::Date)
        );
        assert_eq!(state.clock_seconds(), 28_800);
        assert_eq!(state.date(), Some(date));
    }

    #[test]
    fn events_that_belong_elsewhere_move_nothing() {
        let mut state = WatchState::new();

        for event in [
            AppEvent::Swipe(SwipeDirection::Down),
            AppEvent::Touch {
                x: 10,
                y: 10,
                pressed: true,
            },
            AppEvent::StorageUpdated(crate::StorageState::Ready),
        ] {
            assert!(state.apply(event).is_empty());
        }
        assert_eq!(state.changed(), WatchFields::NONE);
    }

    /// One of every event the firmware raises.
    ///
    /// Written out by hand on purpose: adding a variant should make the caller
    /// come here and decide which side it is on, which is the whole value of
    /// the check below.
    fn every_event() -> impl Iterator<Item = AppEvent> {
        [
            AppEvent::Touch {
                x: 10,
                y: 10,
                pressed: true,
            },
            AppEvent::Swipe(SwipeDirection::Down),
            AppEvent::TouchCancelled,
            AppEvent::BackPressed,
            tick(1),
            AppEvent::DisplaySettingsUpdated(crate::DisplaySettings::DEFAULT),
            AppEvent::StorageUpdated(crate::StorageState::Ready),
            AppEvent::BatteryUpdated(BATTERY),
            AppEvent::StepsUpdated(1),
            AppEvent::BleUpdated(BleState::Connected),
            AppEvent::NotificationsChanged(crate::NotificationSummary {
                count: 1,
                latest: Some(NotificationCategory::Sms),
            }),
            AppEvent::HeartRateStateUpdated(HeartRateState::Measuring),
            AppEvent::HeartRateSensorDetected(crate::HeartRateSensorKind::Hrs3300),
            AppEvent::HeartRateAnalysisUpdated(crate::PpgAnalysis::HeartRate { bpm: 60 }),
            #[cfg(feature = "diagnostics")]
            AppEvent::AccelerometerDetected(crate::AccelerometerKind::Bma421),
            #[cfg(feature = "diagnostics")]
            AppEvent::AccelerationUpdated(crate::AccelerationSample { x: 1, y: 2, z: 3 }),
            #[cfg(feature = "diagnostics")]
            AppEvent::FeatureEngineUpdated(crate::FeatureEngineStatus::Ready),
            #[cfg(feature = "diagnostics")]
            AppEvent::HeartRateRawSampleUpdated(crate::HeartRateRawSample { hrs: 1, als: 2 }),
        ]
        .into_iter()
    }

    /// The rule the display task routes by, and the one whose breach froze the
    /// battery on the face.
    ///
    /// Anything this state absorbs has to be reachable when the panel is off,
    /// because the event is consumed from the channel either way - a reading
    /// that is dropped is not shown late, it is never shown. So it must be
    /// applied before the task decides whether to paint, and
    /// [`AppEvent::is_reading`] is what tells it which those are. A reading
    /// that moves a field here while reporting `false` is exactly the bug that
    /// left a percentage sitting where it was at boot.
    #[test]
    fn every_event_this_state_absorbs_is_one_the_display_task_will_route_here() {
        for event in every_event() {
            // The tick is the one event the display task raises itself rather
            // than receives, and it raises it only while awake and only when it
            // is about to repaint. It cannot be dropped by a gate the way an
            // arriving reading can, because nothing produces it while that gate
            // is shut - so it is deliberately not a reading, and this is the
            // only exemption the rule has.
            if matches!(event, AppEvent::Tick { .. }) {
                assert!(!event.is_reading());
                continue;
            }
            // A fresh state each time, so the assertion is about this event
            // rather than the order they happen to be listed in.
            let mut state = WatchState::new();
            if !state.apply(event).is_empty() {
                assert!(
                    event.is_reading(),
                    "{event:?} moves the watch state but is not routed to it"
                );
            }
        }
    }

    /// The other direction, so the classification cannot be made trivially true
    /// by calling everything a reading: input has to reach the active screen,
    /// and a reading that short-circuits input would strand navigation.
    #[test]
    fn input_and_navigation_are_never_readings() {
        for event in every_event().filter(|event| event.is_reading()) {
            assert!(
                !event.is_user_activity(),
                "{event:?} is both a reading and user input"
            );
        }
        assert!(every_event().any(AppEvent::is_reading));
        assert!(every_event().any(|event| !event.is_reading()));
    }

    #[test]
    fn fields_are_addressed_independently() {
        let fields = WatchFields::of(WatchField::Clock).with(WatchField::Ble);

        assert!(fields.contains(WatchField::Clock));
        assert!(fields.contains(WatchField::Ble));
        assert!(!fields.contains(WatchField::Battery));
        assert!(!fields.is_empty());
        assert!(WatchFields::NONE.is_empty());
    }

    /// Builds the event the inbox sends when what is pending changes.
    fn pending(count: u32, latest: Option<NotificationCategory>) -> AppEvent {
        AppEvent::NotificationsChanged(crate::NotificationSummary { count, latest })
    }

    #[test]
    fn the_face_shows_what_is_pending_rather_than_what_has_arrived() {
        let mut state = WatchState::new();

        assert_eq!(
            state.apply(pending(1, Some(NotificationCategory::Sms))),
            WatchFields::of(WatchField::Notifications)
        );
        // Two of the same category are still two notifications.
        assert_eq!(
            state.apply(pending(2, Some(NotificationCategory::Sms))),
            WatchFields::of(WatchField::Notifications)
        );
        assert_eq!(state.notifications(), 2);
        assert_eq!(state.last_category(), Some(NotificationCategory::Sms));

        // The regression a running tally would produce: dismissing on the
        // notification screen has to bring the face's count back down, and
        // clearing the inbox has to clear the row entirely.
        assert_eq!(
            state.apply(pending(1, Some(NotificationCategory::Sms))),
            WatchFields::of(WatchField::Notifications)
        );
        assert_eq!(state.notifications(), 1);
        assert_eq!(
            state.apply(pending(0, None)),
            WatchFields::of(WatchField::Notifications)
        );
        assert_eq!(state.notifications(), 0);
        assert_eq!(state.last_category(), None);
    }

    #[test]
    fn an_inbox_that_did_not_move_costs_no_redraw() {
        let mut state = WatchState::new();
        let _ = state.apply(pending(2, Some(NotificationCategory::Email)));

        assert_eq!(
            state.apply(pending(2, Some(NotificationCategory::Email))),
            WatchFields::NONE
        );
    }
}
