#![no_std]

//! Pure application state transitions for `PineForge`.
//!
//! This crate deliberately has no Embassy or hardware dependencies so its
//! transitions can be exercised on a host.

use heapless::Vec;

#[cfg(feature = "diagnostics")]
mod ppg;
#[cfg(feature = "diagnostics")]
pub use ppg::{PpgAnalysis, PpgProcessor};

mod bond;
pub use bond::{BOND_PAYLOAD_MAX, BOND_RECORD_LEN, frame_bond, parse_bond};

mod clock;
pub use clock::{WallClockReference, WallTime, parse_cts};

mod dfu;
pub use dfu::{DFU_SLOT_SIZE, DfuEngine, DfuStep, crc16_update};

mod settings;
pub use settings::{
    BRIGHTNESS_LEVELS, DIM_TIMEOUTS_MILLIS, DecodeError, DisplaySettings, OFF_TIMEOUTS_MILLIS,
    SETTINGS_RECORD_LEN, SettingsError, SettingsSlot, SlotDecision, crc32, select_slot,
};

pub const SCREEN_STACK_CAPACITY: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PowerConfig {
    dim_after_millis: u64,
    off_after_millis: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerConfigError {
    ZeroDimTimeout,
    OffNotAfterDim,
}

impl PowerConfig {
    pub const DEFAULT: Self = Self {
        dim_after_millis: 10_000,
        off_after_millis: 20_000,
    };

    pub const fn new(
        dim_after_millis: u64,
        off_after_millis: u64,
    ) -> Result<Self, PowerConfigError> {
        if dim_after_millis == 0 {
            return Err(PowerConfigError::ZeroDimTimeout);
        }
        if off_after_millis <= dim_after_millis {
            return Err(PowerConfigError::OffNotAfterDim);
        }
        Ok(Self {
            dim_after_millis,
            off_after_millis,
        })
    }

    #[must_use]
    pub const fn dim_after_millis(self) -> u64 {
        self.dim_after_millis
    }

    #[must_use]
    pub const fn off_after_millis(self) -> u64 {
        self.off_after_millis
    }
}

/// Hardware-independent activity state shared with power-aware services.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemPowerState {
    Interactive,
    Idle,
    Sleeping,
}

/// Requests accepted by the system power coordinator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerCommand {
    UserActivity,
}

/// Deterministic system-power policy, independent from clocks and hardware.
pub struct SystemPowerPolicy {
    config: PowerConfig,
    state: SystemPowerState,
    last_activity_millis: u64,
}

impl SystemPowerPolicy {
    #[must_use]
    pub const fn new(now_millis: u64, config: PowerConfig) -> Self {
        Self {
            config,
            state: SystemPowerState::Interactive,
            last_activity_millis: now_millis,
        }
    }

    #[must_use]
    pub const fn state(&self) -> SystemPowerState {
        self.state
    }

    /// Replaces the timeout configuration; the caller re-evaluates via
    /// [`Self::advance`] so a shortened timeout takes effect immediately.
    pub const fn set_config(&mut self, config: PowerConfig) {
        self.config = config;
    }

    /// Records user activity and returns a state change, if any.
    pub fn on_activity(&mut self, now_millis: u64) -> Option<SystemPowerState> {
        self.last_activity_millis = now_millis;
        self.set_state(SystemPowerState::Interactive)
    }

    /// Advances inactivity policy and returns a state change, if any.
    pub fn advance(&mut self, now_millis: u64) -> Option<SystemPowerState> {
        let idle = now_millis.saturating_sub(self.last_activity_millis);
        let next = if idle >= self.config.off_after_millis() {
            SystemPowerState::Sleeping
        } else if idle >= self.config.dim_after_millis() {
            SystemPowerState::Idle
        } else {
            SystemPowerState::Interactive
        };
        self.set_state(next)
    }

    /// Returns the next absolute inactivity deadline, or `None` while sleeping.
    #[must_use]
    pub const fn next_deadline_millis(&self) -> Option<u64> {
        match self.state {
            SystemPowerState::Interactive => Some(
                self.last_activity_millis
                    .saturating_add(self.config.dim_after_millis()),
            ),
            SystemPowerState::Idle => Some(
                self.last_activity_millis
                    .saturating_add(self.config.off_after_millis()),
            ),
            SystemPowerState::Sleeping => None,
        }
    }

    fn set_state(&mut self, next: SystemPowerState) -> Option<SystemPowerState> {
        if self.state == next {
            None
        } else {
            self.state = next;
            Some(next)
        }
    }
}

/// Connection state of the BLE stack, published for status display.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BleState {
    #[default]
    Off,
    Advertising,
    /// Central connected; a passkey is being shown for pairing.
    Pairing(u32),
    Connected,
}

/// Haptic patterns playable by the vibration service.
///
/// Callers describe intent; the timing lives here so future features such as
/// notifications and alarms reuse the same vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VibrationPattern {
    /// Single short tick confirming a button activation.
    Tap,
    /// Two pulses for stronger confirmations.
    Double,
    /// One long pulse for alerts.
    Long,
}

impl VibrationPattern {
    /// Returns `(on_millis, pause_millis, pulse_count)`.
    #[must_use]
    pub const fn pulses(self) -> (u64, u64, u8) {
        match self {
            Self::Tap => (25, 0, 1),
            Self::Double => (50, 100, 2),
            Self::Long => (150, 0, 1),
        }
    }
}

/// Latest battery measurement and stabilized capacity state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatteryStatus {
    pub millivolts: u16,
    pub percent: u8,
    pub charging: bool,
    pub power_present: bool,
}

/// Directional capacity estimate based on `InfiniTime`'s battery policy.
///
/// Charger terminal voltage must not make capacity fall while externally
/// powered, and voltage recovery must not make it rise while discharging.
pub struct BatteryCapacityEstimator {
    basis_points: Option<u16>,
}

impl BatteryCapacityEstimator {
    const BASIS_POINTS_PER_PERCENT: u16 = 100;
    const MAX_BASIS_POINTS_PER_MINUTE: u64 = 100;

    #[must_use]
    pub const fn new() -> Self {
        Self { basis_points: None }
    }

    pub fn observe(&mut self, millivolts: u16, power_present: bool, elapsed_seconds: u64) -> u8 {
        let measured = u16::from(battery_percent(millivolts)) * Self::BASIS_POINTS_PER_PERCENT;
        let basis_points = self.basis_points.map_or(measured, |previous| {
            let target = if power_present {
                previous.max(measured)
            } else {
                previous.min(measured)
            };
            let allowance = elapsed_seconds.saturating_mul(Self::MAX_BASIS_POINTS_PER_MINUTE) / 60;
            let allowance = u16::try_from(allowance).unwrap_or(u16::MAX);

            if target >= previous {
                target.min(previous.saturating_add(allowance))
            } else {
                target.max(previous.saturating_sub(allowance))
            }
        });
        self.basis_points = Some(basis_points);
        u8::try_from(basis_points / Self::BASIS_POINTS_PER_PERCENT).unwrap_or(100)
    }
}

impl Default for BatteryCapacityEstimator {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccelerometerKind {
    Bma421,
    Bma425,
    Unknown(u8),
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccelerationSample {
    pub x: i16,
    pub y: i16,
    pub z: i16,
}

#[cfg(feature = "diagnostics")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureEngineStatus {
    Ready,
    Failed,
}

#[cfg(feature = "diagnostics")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeartRateSensorKind {
    Hrs3300,
    Unknown(u8),
    Unavailable,
}

#[cfg(feature = "diagnostics")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeartRateRawSample {
    pub hrs: u16,
    pub als: u16,
}

#[cfg(feature = "diagnostics")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeartRateCommand {
    Start,
    Stop,
}

#[cfg(feature = "diagnostics")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeartRateState {
    Disabled,
    Starting,
    Collecting,
    Measuring,
    Result(u16),
    NoSignal,
    AmbientLight,
    Error,
}

#[cfg(feature = "diagnostics")]
pub struct HeartRateSession {
    state: HeartRateState,
}

#[cfg(feature = "diagnostics")]
impl HeartRateSession {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: HeartRateState::Disabled,
        }
    }

    #[must_use]
    pub const fn state(&self) -> HeartRateState {
        self.state
    }

    pub const fn start(&mut self) -> HeartRateState {
        self.state = HeartRateState::Starting;
        self.state
    }

    pub const fn collecting(&mut self) -> HeartRateState {
        self.state = HeartRateState::Collecting;
        self.state
    }

    pub const fn apply(&mut self, analysis: PpgAnalysis) -> HeartRateState {
        self.state = match analysis {
            PpgAnalysis::Collecting { .. } => HeartRateState::Collecting,
            PpgAnalysis::HeartRate { bpm } => HeartRateState::Result(bpm),
            PpgAnalysis::NoSignal => HeartRateState::NoSignal,
            PpgAnalysis::AmbientLight => HeartRateState::AmbientLight,
        };
        self.state
    }

    pub const fn stop(&mut self) -> HeartRateState {
        self.state = HeartRateState::Disabled;
        self.state
    }

    pub const fn fail(&mut self) -> HeartRateState {
        self.state = HeartRateState::Error;
        self.state
    }
}

#[cfg(feature = "diagnostics")]
impl Default for HeartRateSession {
    fn default() -> Self {
        Self::new()
    }
}

#[must_use]
pub const fn accelerometer_kind(chip_id: u8) -> AccelerometerKind {
    match chip_id {
        0x11 => AccelerometerKind::Bma421,
        0x13 => AccelerometerKind::Bma425,
        value => AccelerometerKind::Unknown(value),
    }
}

/// Converts a 12-bit SAADC sample into battery millivolts.
///
/// `PineTime` divides the battery voltage by two. With the SAADC's 600 mV
/// internal reference and 1/4 gain, the ADC input range is 2400 mV and the
/// corresponding battery range is 4800 mV.
#[must_use]
pub fn battery_millivolts(raw: i16) -> u16 {
    let raw = u32::from(raw.max(0).cast_unsigned()).min(4095);
    u16::try_from((raw * 4_800 + 2_048) / 4_096).unwrap_or(u16::MAX)
}

/// Estimates remaining capacity from `InfiniTime`'s measured `PineTime`
/// discharge curve using piecewise-linear interpolation.
#[must_use]
pub fn battery_percent(millivolts: u16) -> u8 {
    const CURVE: [(u16, u8); 6] = [
        (3_500, 0),
        (3_616, 3),
        (3_723, 22),
        (3_776, 48),
        (3_979, 79),
        (4_180, 100),
    ];

    if millivolts <= CURVE[0].0 {
        return CURVE[0].1;
    }
    for window in CURVE.windows(2) {
        let [low, high] = window else {
            continue;
        };
        let (low_mv, low_percent) = *low;
        let (high_mv, high_percent) = *high;
        if millivolts <= high_mv {
            let position = u32::from(millivolts - low_mv);
            let span = u32::from(high_mv - low_mv);
            let percent_span = u32::from(high_percent - low_percent);
            let interpolated = u32::from(low_percent) + (position * percent_span + span / 2) / span;
            return u8::try_from(interpolated).unwrap_or(100);
        }
    }
    100
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenId {
    Watchface,
    DisplaySettings,
    #[cfg(feature = "diagnostics")]
    TouchTest,
    #[cfg(feature = "diagnostics")]
    HeartRate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppEvent {
    Touch {
        x: i32,
        y: i32,
        pressed: bool,
    },
    Swipe(SwipeDirection),
    Tick {
        uptime_seconds: u64,
        wall_time: Option<WallTime>,
    },
    BatteryUpdated(BatteryStatus),
    #[cfg(feature = "diagnostics")]
    AccelerometerDetected(AccelerometerKind),
    #[cfg(feature = "diagnostics")]
    AccelerationUpdated(AccelerationSample),
    #[cfg(feature = "diagnostics")]
    FeatureEngineUpdated(FeatureEngineStatus),
    #[cfg(feature = "diagnostics")]
    HeartRateSensorDetected(HeartRateSensorKind),
    #[cfg(feature = "diagnostics")]
    HeartRateRawSampleUpdated(HeartRateRawSample),
    #[cfg(feature = "diagnostics")]
    HeartRateAnalysisUpdated(PpgAnalysis),
    #[cfg(feature = "diagnostics")]
    HeartRateStateUpdated(HeartRateState),
    StepsUpdated(u32),
    DisplaySettingsUpdated(DisplaySettings),
    BleUpdated(BleState),
}

impl AppEvent {
    #[must_use]
    pub const fn is_user_activity(self) -> bool {
        matches!(self, Self::Touch { .. } | Self::Swipe(_))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwipeDirection {
    Left,
    Right,
    Up,
    Down,
}

const SWIPE_MIN_DISTANCE: i32 = 40;
const SWIPE_AXIS_DOMINANCE_NUMERATOR: i32 = 3;
const SWIPE_AXIS_DOMINANCE_DENOMINATOR: i32 = 2;

/// Combines controller-provided gestures with a coordinate-based fallback.
///
/// The fallback only accepts a clearly dominant axis, preserving taps and
/// diagonal drawing input while covering missed `CST816S` gesture reports.
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
        if !pressed {
            // Fast flicks can deliver only a press and a release report, so
            // the release coordinates are also checked for a missed swipe.
            let fallback = self
                .start
                .and_then(|start| Self::direction_from_delta(x - start.0, y - start.1));
            let gesture = controller_gesture.or(fallback).filter(|_| !self.emitted);
            self.reset();
            return gesture;
        }

        let start = *self.start.get_or_insert((x, y));
        if self.emitted {
            return None;
        }
        if let Some(gesture) = controller_gesture {
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
        if horizontal >= SWIPE_MIN_DISTANCE
            && horizontal * SWIPE_AXIS_DOMINANCE_DENOMINATOR
                >= vertical * SWIPE_AXIS_DOMINANCE_NUMERATOR
        {
            Some(if delta_x < 0 {
                SwipeDirection::Left
            } else {
                SwipeDirection::Right
            })
        } else if vertical >= SWIPE_MIN_DISTANCE
            && vertical * SWIPE_AXIS_DOMINANCE_DENOMINATOR
                >= horizontal * SWIPE_AXIS_DOMINANCE_NUMERATOR
        {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenAction {
    None,
    Push(ScreenId),
    Back,
    RequestRollback,
    ApplySettings(DisplaySettings),
    ConfirmFirmware,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppEffect {
    None,
    Navigate(NavigationDirection),
    RequestRollback,
    ApplySettings(DisplaySettings),
    ConfirmFirmware,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavigationDirection {
    Forward,
    Backward,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ButtonBounds {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl ButtonBounds {
    #[must_use]
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    const fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x
            && y >= self.y
            && x < self.x.saturating_add(self.width)
            && y < self.y.saturating_add(self.height)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonState {
    Idle,
    Pressed,
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonOutcome {
    None,
    Redraw,
    Activated,
}

/// Heap-free button interaction state with press-and-release activation.
pub struct Button {
    bounds: ButtonBounds,
    state: ButtonState,
}

impl Button {
    #[must_use]
    pub const fn new(bounds: ButtonBounds) -> Self {
        Self {
            bounds,
            state: ButtonState::Idle,
        }
    }

    #[must_use]
    pub const fn state(&self) -> ButtonState {
        self.state
    }

    pub fn set_enabled(&mut self, enabled: bool) -> ButtonOutcome {
        let next = if enabled {
            ButtonState::Idle
        } else {
            ButtonState::Disabled
        };
        if self.state == next {
            ButtonOutcome::None
        } else {
            self.state = next;
            ButtonOutcome::Redraw
        }
    }

    pub fn handle_event(&mut self, event: AppEvent) -> ButtonOutcome {
        let AppEvent::Touch { x, y, pressed } = event else {
            return ButtonOutcome::None;
        };
        if self.state == ButtonState::Disabled {
            return ButtonOutcome::None;
        }

        let inside = self.bounds.contains(x, y);
        match (self.state, pressed, inside) {
            (ButtonState::Idle, true, true) => {
                self.state = ButtonState::Pressed;
                ButtonOutcome::Redraw
            }
            (ButtonState::Pressed, false, true) => {
                self.state = ButtonState::Idle;
                ButtonOutcome::Activated
            }
            (ButtonState::Pressed, _, false) => {
                self.state = ButtonState::Idle;
                ButtonOutcome::Redraw
            }
            _ => ButtonOutcome::None,
        }
    }
}

/// Owns application-wide navigation state.
///
/// Peripheral state remains owned by its Embassy task; this controller only
/// contains deterministic product state.
pub struct AppState {
    screens: Vec<ScreenId, SCREEN_STACK_CAPACITY>,
}

impl AppState {
    #[must_use]
    pub fn new(root: ScreenId) -> Self {
        let mut screens = Vec::new();
        screens
            .push(root)
            .expect("the empty screen stack always has room for its root");
        Self { screens }
    }

    #[must_use]
    pub fn active_screen(&self) -> ScreenId {
        *self
            .screens
            .last()
            .expect("AppState always retains a root screen")
    }

    /// Applies a high-level action returned by the active screen.
    pub fn transition(&mut self, action: ScreenAction) -> AppEffect {
        match action {
            ScreenAction::None => AppEffect::None,
            ScreenAction::RequestRollback => AppEffect::RequestRollback,
            ScreenAction::ApplySettings(settings) => AppEffect::ApplySettings(settings),
            ScreenAction::ConfirmFirmware => AppEffect::ConfirmFirmware,
            ScreenAction::Back => {
                if self.screens.len() > 1 {
                    self.screens.pop();
                    AppEffect::Navigate(NavigationDirection::Backward)
                } else {
                    AppEffect::None
                }
            }
            ScreenAction::Push(screen) => {
                if self.active_screen() == screen {
                    return AppEffect::None;
                }

                if self.screens.push(screen).is_ok() {
                    AppEffect::Navigate(NavigationDirection::Forward)
                } else {
                    AppEffect::None
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_conversion_uses_the_full_12_bit_range() {
        assert_eq!(battery_millivolts(0), 0);
        assert_eq!(battery_millivolts(4095), 4_799);
        assert_eq!(battery_millivolts(3584), 4_200);
    }

    #[test]
    fn battery_conversion_clamps_out_of_range_samples() {
        assert_eq!(battery_millivolts(-1), 0);
        assert_eq!(battery_millivolts(i16::MAX), 4_799);
    }

    #[test]
    fn battery_percentage_interpolates_the_pinetime_curve() {
        assert_eq!(battery_percent(3_400), 0);
        assert_eq!(battery_percent(3_500), 0);
        assert_eq!(battery_percent(3_723), 22);
        assert_eq!(battery_percent(3_878), 64);
        assert_eq!(battery_percent(4_180), 100);
        assert_eq!(battery_percent(4_300), 100);
    }

    #[test]
    fn battery_capacity_moves_only_in_the_power_state_direction() {
        let mut estimator = BatteryCapacityEstimator::new();

        assert_eq!(estimator.observe(3_878, false, 0), 64);
        assert_eq!(estimator.observe(3_979, false, 3_600), 64);
        assert_eq!(estimator.observe(3_776, true, 3_600), 64);
        assert_eq!(estimator.observe(3_979, true, 3_600), 79);
        assert_eq!(estimator.observe(3_776, false, 3_600), 48);
    }

    #[test]
    fn battery_capacity_is_slew_limited_by_elapsed_time() {
        let mut estimator = BatteryCapacityEstimator::new();

        assert_eq!(estimator.observe(3_850, false, 0), 59);
        assert_eq!(estimator.observe(3_979, true, 30), 59);
        assert_eq!(estimator.observe(3_979, true, 30), 60);
        assert_eq!(estimator.observe(3_500, false, 30), 59);
        assert_eq!(estimator.observe(3_500, false, 30), 59);
    }

    #[test]
    fn system_power_policy_idles_sleeps_and_wakes() {
        let mut policy = SystemPowerPolicy::new(1_000, PowerConfig::DEFAULT);

        assert_eq!(policy.advance(10_999), None);
        assert_eq!(policy.next_deadline_millis(), Some(11_000));
        assert_eq!(policy.advance(11_000), Some(SystemPowerState::Idle));
        assert_eq!(policy.advance(20_999), None);
        assert_eq!(policy.next_deadline_millis(), Some(21_000));
        assert_eq!(policy.advance(21_000), Some(SystemPowerState::Sleeping));
        assert_eq!(policy.next_deadline_millis(), None);
        assert_eq!(
            policy.on_activity(25_000),
            Some(SystemPowerState::Interactive)
        );
        assert_eq!(policy.state(), SystemPowerState::Interactive);
        assert_eq!(policy.next_deadline_millis(), Some(35_000));
    }

    #[test]
    fn system_power_policy_uses_saturating_time_math() {
        let mut policy = SystemPowerPolicy::new(5_000, PowerConfig::DEFAULT);

        assert_eq!(policy.advance(4_000), None);
        assert_eq!(policy.state(), SystemPowerState::Interactive);
    }

    #[test]
    fn power_config_rejects_invalid_timeout_order() {
        assert_eq!(
            PowerConfig::new(0, 20_000),
            Err(PowerConfigError::ZeroDimTimeout)
        );
        assert_eq!(
            PowerConfig::new(20_000, 20_000),
            Err(PowerConfigError::OffNotAfterDim)
        );
    }

    #[cfg(feature = "diagnostics")]
    #[test]
    fn accelerometer_chip_ids_distinguish_pinetime_variants() {
        assert_eq!(accelerometer_kind(0x11), AccelerometerKind::Bma421);
        assert_eq!(accelerometer_kind(0x13), AccelerometerKind::Bma425);
        assert_eq!(accelerometer_kind(0xff), AccelerometerKind::Unknown(0xff));
    }

    #[cfg(feature = "diagnostics")]
    #[test]
    fn heart_rate_session_has_explicit_lifecycle() {
        let mut session = HeartRateSession::new();
        assert_eq!(session.state(), HeartRateState::Disabled);
        assert_eq!(session.start(), HeartRateState::Starting);
        assert_eq!(session.collecting(), HeartRateState::Collecting);
        assert_eq!(
            session.apply(PpgAnalysis::HeartRate { bpm: 72 }),
            HeartRateState::Result(72)
        );
        assert_eq!(session.stop(), HeartRateState::Disabled);
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
        assert_eq!(
            recognizer.update(170, 100, true, Some(SwipeDirection::Left)),
            Some(SwipeDirection::Left)
        );
        assert_eq!(
            recognizer.update(100, 100, true, Some(SwipeDirection::Left)),
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
    fn vibration_patterns_are_bounded_and_playable() {
        for pattern in [
            VibrationPattern::Tap,
            VibrationPattern::Double,
            VibrationPattern::Long,
        ] {
            let (on_millis, pause_millis, count) = pattern.pulses();
            assert!(on_millis > 0 && on_millis <= 500);
            assert!(count >= 1);
            assert!(count == 1 || pause_millis > 0);
        }
    }

    #[test]
    fn apply_settings_is_an_explicit_effect() {
        let mut app = AppState::new(ScreenId::Watchface);
        let settings = DisplaySettings::DEFAULT.cycle_brightness();

        assert_eq!(
            app.transition(ScreenAction::ApplySettings(settings)),
            AppEffect::ApplySettings(settings)
        );
        assert_eq!(app.active_screen(), ScreenId::Watchface);
    }

    #[test]
    fn confirm_firmware_is_an_explicit_effect() {
        let mut app = AppState::new(ScreenId::Watchface);
        assert_eq!(
            app.transition(ScreenAction::ConfirmFirmware),
            AppEffect::ConfirmFirmware
        );
        assert_eq!(app.active_screen(), ScreenId::Watchface);
    }

    #[test]
    fn root_cannot_be_popped() {
        let mut app = AppState::new(ScreenId::Watchface);

        assert_eq!(app.transition(ScreenAction::Back), AppEffect::None);
        assert_eq!(app.active_screen(), ScreenId::Watchface);
    }

    #[cfg(feature = "diagnostics")]
    #[test]
    fn push_and_back_change_the_active_screen() {
        let mut app = AppState::new(ScreenId::Watchface);

        assert_eq!(
            app.transition(ScreenAction::Push(ScreenId::TouchTest)),
            AppEffect::Navigate(NavigationDirection::Forward)
        );
        assert_eq!(app.active_screen(), ScreenId::TouchTest);
        assert_eq!(
            app.transition(ScreenAction::Back),
            AppEffect::Navigate(NavigationDirection::Backward)
        );
        assert_eq!(app.active_screen(), ScreenId::Watchface);
    }

    #[test]
    fn duplicate_push_is_ignored() {
        let mut app = AppState::new(ScreenId::Watchface);

        assert_eq!(
            app.transition(ScreenAction::Push(ScreenId::Watchface)),
            AppEffect::None
        );
    }

    #[cfg(feature = "diagnostics")]
    #[test]
    fn full_stack_rejects_another_screen_without_losing_state() {
        let mut app = AppState::new(ScreenId::Watchface);
        for screen in [
            ScreenId::TouchTest,
            ScreenId::Watchface,
            ScreenId::TouchTest,
        ] {
            assert_eq!(
                app.transition(ScreenAction::Push(screen)),
                AppEffect::Navigate(NavigationDirection::Forward)
            );
        }

        assert_eq!(
            app.transition(ScreenAction::Push(ScreenId::Watchface)),
            AppEffect::None
        );
        assert_eq!(app.active_screen(), ScreenId::TouchTest);
    }

    #[test]
    fn rollback_is_an_explicit_effect() {
        let mut app = AppState::new(ScreenId::Watchface);

        assert_eq!(
            app.transition(ScreenAction::RequestRollback),
            AppEffect::RequestRollback
        );
    }

    #[test]
    fn button_activates_only_after_press_and_release_inside() {
        let mut button = Button::new(ButtonBounds::new(10, 20, 100, 40));

        assert_eq!(
            button.handle_event(AppEvent::Touch {
                x: 20,
                y: 30,
                pressed: true,
            }),
            ButtonOutcome::Redraw
        );
        assert_eq!(button.state(), ButtonState::Pressed);
        assert_eq!(
            button.handle_event(AppEvent::Touch {
                x: 20,
                y: 30,
                pressed: false,
            }),
            ButtonOutcome::Activated
        );
        assert_eq!(button.state(), ButtonState::Idle);
    }

    #[test]
    fn dragging_outside_cancels_button_activation() {
        let mut button = Button::new(ButtonBounds::new(10, 20, 100, 40));
        let _ = button.handle_event(AppEvent::Touch {
            x: 20,
            y: 30,
            pressed: true,
        });

        assert_eq!(
            button.handle_event(AppEvent::Touch {
                x: 200,
                y: 30,
                pressed: true,
            }),
            ButtonOutcome::Redraw
        );
        assert_eq!(button.state(), ButtonState::Idle);
    }

    #[test]
    fn disabled_button_ignores_touch() {
        let mut button = Button::new(ButtonBounds::new(10, 20, 100, 40));
        assert_eq!(button.set_enabled(false), ButtonOutcome::Redraw);

        assert_eq!(
            button.handle_event(AppEvent::Touch {
                x: 20,
                y: 30,
                pressed: true,
            }),
            ButtonOutcome::None
        );
        assert_eq!(button.state(), ButtonState::Disabled);
    }
}
