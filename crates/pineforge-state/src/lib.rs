#![no_std]

//! Pure application state transitions for `PineForge`.
//!
//! This crate deliberately has no Embassy or hardware dependencies so its
//! transitions can be exercised on a host.

use heapless::Vec;

mod ppg;
pub use ppg::{PpgAnalysis, PpgProcessor};

mod bond;
pub use bond::{
    BOND_PAYLOAD_MAX, BOND_RECORD_LEN, BOND_SCHEMA_LEN, BondRecord, bond_schema_tag, frame_bond,
    parse_bond,
};

mod clock;
pub use clock::{CalendarDate, WallClockReference, WallTime, parse_cts};

mod dfu;
pub use dfu::{DFU_SLOT_SIZE, DfuEngine, DfuStep, crc16_update};

mod list;
pub use list::{ListOutcome, ListSlots, PageAxis, PagedList};

mod modal;
pub use modal::{Modal, ModalOutcome, ModalState};

mod notification;
pub use notification::{
    NOTIFICATION_BODY_MAX, NOTIFICATION_CAPACITY, NOTIFICATION_TITLE_MAX, Notification,
    NotificationCategory, NotificationInbox, NotificationSummary, Wrapped, parse_new_alert, wrap,
};

mod settings;
pub use setting::{Setting, WATCHFACE_NAMES};
pub use settings::{
    BRIGHTNESS_LEVELS, BRIGHTNESS_NAMES, DIM_TIMEOUT_NAMES, DIM_TIMEOUTS_MILLIS, DecodeError,
    DisplaySettings, HEART_RATE_ENABLED_NAMES, HEART_RATE_INTERVAL_NAMES,
    HEART_RATE_INTERVALS_SECONDS, OFF_TIMEOUT_NAMES, OFF_TIMEOUTS_MILLIS, SETTINGS_RECORD_LEN,
    SettingsError, SettingsSlot, SlotDecision, crc32, select_slot,
};
mod setting;
mod watch;
pub use watch::{WatchField, WatchFields, WatchState};

mod watchface;
pub use watchface::{WATCHFACES, WatchfaceDescriptor, WatchfaceId};

mod touch;
pub use touch::{SwipeRecognizer, TouchEvents, TouchReport, TouchRouter};

mod storage;
pub use storage::{
    STORAGE_BASE, STORAGE_DATA_SECTOR_COUNT, STORAGE_END, STORAGE_FORMAT_VERSION,
    STORAGE_HEADER_LEN, STORAGE_PROGRESS_OFFSET, STORAGE_READY_HEADER_OFFSET, STORAGE_SECTOR_COUNT,
    STORAGE_SECTOR_SIZE, StorageHeader, decode_storage_header, encode_storage_header,
    storage_header_version,
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
    /// Firmware update in progress; payload is the transfer percent (0-100).
    DfuProgress(u8),
    /// A flash operation during the firmware update failed; the reason is the
    /// watch's only diagnostic surface on a sealed device.
    DfuFailed(DfuFailReason),
}

/// Why a DFU flash operation could not complete.
///
/// The sealed watch has no debug port, so this is surfaced on-screen: it turns
/// the generic "flash write failed" into a concrete, actionable cause.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DfuFailReason {
    /// The external flash answered with an unexpected JEDEC id, so it was never
    /// marked writable. The bytes are the id that was actually read.
    FlashUnrecognized([u8; 3]),
    /// The external flash did not respond to initialization at all.
    FlashInitFailed,
    /// A sector erase reported an error mid-transfer.
    EraseFailed,
    /// A page program reported an error mid-transfer.
    ProgramFailed,
    /// A verified write read back different bytes than were programmed.
    VerifyFailed,
    /// The host stopped sending mid-transfer without closing the connection.
    /// The partial image is discarded and the engine returns to idle, so the
    /// next attempt needs neither a reconnect nor a restart.
    TimedOut,
    /// The running image has not been confirmed, so it refused to overwrite the
    /// rollback image sitting in the secondary slot.
    ///
    /// Not a failure of the transfer - it never started. It is here because the
    /// alternative is a refusal only the phone hears about, which from the
    /// watch is indistinguishable from a connection that quietly died.
    NotConfirmed,
}

/// State of the one-time external storage initialization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageState {
    Formatting(u8),
    Ready,
    Failed,
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

/// How urgent the battery is, for a status symbol to colour.
///
/// The thresholds are product policy and live here rather than in the drawing
/// code, so the answer is the same wherever charge is shown and can be tested
/// on a host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChargeLevel {
    Good,
    Low,
    Critical,
}

impl BatteryStatus {
    /// Charge urgency. Anything on the charger reads as healthy: the number is
    /// climbing, so warning about it would be noise.
    #[must_use]
    pub const fn level(self) -> ChargeLevel {
        if self.power_present {
            return ChargeLevel::Good;
        }
        match self.percent {
            0..20 => ChargeLevel::Critical,
            20..=50 => ChargeLevel::Low,
            _ => ChargeLevel::Good,
        }
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeartRateCommand {
    Configure {
        enabled: bool,
        interval_seconds: u32,
    },
}

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

pub struct HeartRateSession {
    state: HeartRateState,
}

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
    /// The application launcher: tiles opening the screens below the root.
    Launcher,
    /// The notifications that have arrived and not been dismissed, one per page.
    Notifications,
    DisplaySettings,
    /// Settings leaves, each offering the presets of one setting.
    Brightness,
    DimTimeout,
    OffTimeout,
    HeartRate,
    HeartRateInterval,
    /// Picks the watchface, reached from the settings root.
    WatchfaceSelect,
    /// Firmware confirmation and a software reboot.
    Firmware,
    #[cfg(feature = "diagnostics")]
    TouchTest,
}

impl ScreenId {
    /// How many screens this build has.
    pub const COUNT: usize = if cfg!(feature = "diagnostics") {
        12
    } else {
        11
    };

    /// Every screen, so anything that has to hold for all of them can be
    /// written once - the registry that owns them, and the tests that check
    /// each one paints its whole surface.
    ///
    /// Listed rather than derived, because deriving it needs a macro crate this
    /// firmware does not otherwise want. Forgetting to list a new screen is
    /// caught rather than trusted: [`Self::position`] below is exhaustive, so a
    /// new variant stops the build until it is placed, the array's length is
    /// [`Self::COUNT`], and the test at the foot of this file proves the two
    /// agree.
    pub const ALL: [Self; Self::COUNT] = [
        Self::Watchface,
        Self::Launcher,
        Self::Notifications,
        Self::DisplaySettings,
        Self::Brightness,
        Self::DimTimeout,
        Self::OffTimeout,
        Self::HeartRate,
        Self::HeartRateInterval,
        Self::WatchfaceSelect,
        Self::Firmware,
        #[cfg(feature = "diagnostics")]
        Self::TouchTest,
    ];

    /// This screen's slot in [`Self::ALL`].
    ///
    /// Exists to be exhaustive, not to be called: a new variant stops this
    /// matching, and that is the whole mechanism. It is compiled only under
    /// test so it costs the firmware nothing - the check still runs, because
    /// the tests are a CI gate for both feature sets.
    #[cfg(test)]
    const fn position(self) -> usize {
        match self {
            Self::Watchface => 0,
            Self::Launcher => 1,
            Self::Notifications => 2,
            Self::DisplaySettings => 3,
            Self::Brightness => 4,
            Self::DimTimeout => 5,
            Self::OffTimeout => 6,
            Self::HeartRate => 7,
            Self::HeartRateInterval => 8,
            Self::WatchfaceSelect => 9,
            Self::Firmware => 10,
            #[cfg(feature = "diagnostics")]
            Self::TouchTest => 11,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppEvent {
    Touch {
        x: i32,
        y: i32,
        pressed: bool,
    },
    Swipe(SwipeDirection),
    /// The touch in flight turned out to be a gesture, so whoever was tracking
    /// it must drop it without acting on it.
    ///
    /// A swipe of the minimum distance still fits inside one menu row, so the
    /// release that ends the gesture would otherwise land inside the control the
    /// finger started on and activate it. The gesture consumes its own touch,
    /// the same way a modal consumes the input that dismisses it.
    TouchCancelled,
    /// The user asked to leave the active screen.
    ///
    /// The side button raises this, but nothing about it is a button: the back
    /// gesture keeps working, and a screen with an on-screen back control
    /// returns [`ScreenAction::Back`] to the same effect.
    BackPressed,
    Tick {
        uptime_seconds: u64,
        wall_time: Option<WallTime>,
        /// The calendar date of the last synchronization, absent until one
        /// arrives over BLE.
        date: Option<CalendarDate>,
    },
    BatteryUpdated(BatteryStatus),
    #[cfg(feature = "diagnostics")]
    AccelerometerDetected(AccelerometerKind),
    #[cfg(feature = "diagnostics")]
    AccelerationUpdated(AccelerationSample),
    #[cfg(feature = "diagnostics")]
    FeatureEngineUpdated(FeatureEngineStatus),
    HeartRateSensorDetected(HeartRateSensorKind),
    #[cfg(feature = "diagnostics")]
    HeartRateRawSampleUpdated(HeartRateRawSample),
    HeartRateAnalysisUpdated(PpgAnalysis),
    HeartRateStateUpdated(HeartRateState),
    StepsUpdated(u32),
    /// What is pending in the notification inbox changed - one arrived, or one
    /// was dismissed.
    ///
    /// The text does not ride on this event. `AppEvent` is `Copy` and sits in a
    /// fixed-capacity channel, so carrying two string buffers would cost the
    /// channel its capacity times a notification; the messages travel on their
    /// own channel and this reports what a tally needs.
    NotificationsChanged(NotificationSummary),
    DisplaySettingsUpdated(DisplaySettings),
    BleUpdated(BleState),
    StorageUpdated(StorageState),
}

impl AppEvent {
    #[must_use]
    pub const fn is_user_activity(self) -> bool {
        matches!(
            self,
            Self::Touch { .. } | Self::Swipe(_) | Self::BackPressed
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwipeDirection {
    Left,
    Right,
    Up,
    Down,
}

impl SwipeDirection {
    /// The gesture that undoes this one.
    #[must_use]
    pub const fn opposite(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
            Self::Up => Self::Down,
            Self::Down => Self::Up,
        }
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
    /// Restart the watch from software. On an unconfirmed image this is a
    /// rollback; on a confirmed one it is a plain reboot.
    Reboot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppEffect {
    None,
    Navigate(Navigation),
    RequestRollback,
    ApplySettings(DisplaySettings),
    ConfirmFirmware,
    Reboot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavigationDirection {
    Forward,
    Backward,
}

/// A navigation the display has to render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Navigation {
    /// Whether the screen stack grew or shrank.
    pub direction: NavigationDirection,
    /// The way the content travels, so the animation follows the finger: a
    /// screen opened by swiping down slides down, and leaves upwards again.
    ///
    /// A navigation with no gesture behind it - a tap on a launcher tile, an
    /// explicit back - falls back to the conventional horizontal push, so the
    /// renderer never has to invent an axis.
    pub motion: SwipeDirection,
}

impl Navigation {
    /// The axis a gestureless navigation moves along.
    const DEFAULT_FORWARD_MOTION: SwipeDirection = SwipeDirection::Left;

    #[must_use]
    pub const fn forward(motion: SwipeDirection) -> Self {
        Self {
            direction: NavigationDirection::Forward,
            motion,
        }
    }

    #[must_use]
    pub const fn backward(motion: SwipeDirection) -> Self {
        Self {
            direction: NavigationDirection::Backward,
            motion,
        }
    }
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

    #[must_use]
    pub const fn x(self) -> i32 {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> i32 {
        self.y
    }

    #[must_use]
    pub const fn width(self) -> i32 {
        self.width
    }

    #[must_use]
    pub const fn height(self) -> i32 {
        self.height
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
        if event == AppEvent::TouchCancelled {
            // Not an activation: the press is abandoned exactly as if the finger
            // had been dragged out of the button.
            if self.state != ButtonState::Pressed {
                return ButtonOutcome::None;
            }
            self.state = ButtonState::Idle;
            return ButtonOutcome::Redraw;
        }
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
            // A press captures the button, and lifting completes it - wherever
            // the controller happens to say the finger ended up.
            //
            // Requiring the release to land inside as well is what made a tile
            // light up and then do nothing: the CST816S does not promise a
            // meaningful position on the report that ends a touch, and one that
            // read as outside dropped the activation on the floor. Every
            // pointer stack works this way, LVGL included, which is why
            // InfiniTime does not have this failure - the object under the
            // press owns the interaction until it is dragged out of, and the
            // release position never re-selects a target.
            (ButtonState::Pressed, false, _) => {
                self.state = ButtonState::Idle;
                ButtonOutcome::Activated
            }
            // Dragged out while still down: the press is abandoned, which is
            // the one way to change your mind without lifting.
            (ButtonState::Pressed, true, false) => {
                self.state = ButtonState::Idle;
                ButtonOutcome::Redraw
            }
            _ => ButtonOutcome::None,
        }
    }
}

/// A screen on the stack, with the gesture that opened it.
///
/// The root has no entry gesture: nothing opened it, and nothing pops it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ScreenEntry {
    screen: ScreenId,
    entered_by: Option<SwipeDirection>,
}

/// Where a swipe leads from a given screen.
///
/// This table is the whole forward navigation contract. Screens deliberately
/// know nothing about it, so a gesture can be re-routed here without touching
/// any rendering code.
const fn route(from: ScreenId, swipe: SwipeDirection) -> Option<ScreenId> {
    match (from, swipe) {
        (ScreenId::Watchface, SwipeDirection::Up) => Some(ScreenId::Launcher),
        // Pulling down brings what arrived down with it, the way a phone's
        // shade does.
        (ScreenId::Watchface, SwipeDirection::Down) => Some(ScreenId::Notifications),
        // Left is spoken for by quick settings; the screen is not built yet, so
        // the gesture leads nowhere rather than somewhere temporary that would
        // have to be unlearned.
        _ => None,
    }
}

/// Owns application-wide navigation state.
///
/// Peripheral state remains owned by its Embassy task; this controller only
/// contains deterministic product state.
pub struct AppState {
    screens: Vec<ScreenEntry, SCREEN_STACK_CAPACITY>,
}

impl AppState {
    #[must_use]
    pub fn new(root: ScreenId) -> Self {
        let mut screens = Vec::new();
        screens
            .push(ScreenEntry {
                screen: root,
                entered_by: None,
            })
            .expect("the empty screen stack always has room for its root");
        Self { screens }
    }

    #[must_use]
    pub fn active_screen(&self) -> ScreenId {
        self.active_entry().screen
    }

    fn active_entry(&self) -> ScreenEntry {
        *self
            .screens
            .last()
            .expect("AppState always retains a root screen")
    }

    /// Resolves a swipe against the navigation contract.
    ///
    /// A screen is left by the opposite of the gesture that opened it: pull
    /// the settings down, push them back up. That rule holds for every screen
    /// without any of them knowing how it was reached, so the back gesture can
    /// never drift out of step with the forward one.
    ///
    /// Returns [`AppEffect::None`] when the swipe navigates nowhere; the
    /// gesture then belongs to the active screen.
    pub fn navigate(&mut self, swipe: SwipeDirection) -> AppEffect {
        // A screen opened by a tap has no entry gesture of its own, so it
        // borrows the default one: it slid in the way a forward navigation
        // always does, and it leaves the same way reversed. That makes one
        // direction the way back out of everything, whether a gesture or a tile
        // opened it.
        let entered_by = self
            .active_entry()
            .entered_by
            .unwrap_or(Navigation::DEFAULT_FORWARD_MOTION);
        if swipe == entered_by.opposite() && self.screens.len() > 1 {
            return self.pop();
        }
        if let Some(target) = route(self.active_screen(), swipe) {
            return self.push(target, Some(swipe));
        }
        AppEffect::None
    }

    /// Leaves the active screen without a gesture behind it.
    ///
    /// This is the side button's entire contribution to navigation: the same
    /// pop the back gesture performs, so no screen can tell the two apart. The
    /// root absorbs it, which is what makes the button safe to press anywhere -
    /// it never leaves the user on nothing.
    pub fn back(&mut self) -> AppEffect {
        self.pop()
    }

    /// Applies a high-level action returned by the active screen.
    pub fn transition(&mut self, action: ScreenAction) -> AppEffect {
        match action {
            ScreenAction::None => AppEffect::None,
            ScreenAction::RequestRollback => AppEffect::RequestRollback,
            ScreenAction::ApplySettings(settings) => AppEffect::ApplySettings(settings),
            ScreenAction::ConfirmFirmware => AppEffect::ConfirmFirmware,
            ScreenAction::Reboot => AppEffect::Reboot,
            ScreenAction::Back => self.pop(),
            ScreenAction::Push(screen) => self.push(screen, None),
        }
    }

    /// The root is never popped, so the stack always has an active screen.
    ///
    /// A screen leaves the way it arrived, reversed, whether it is dismissed by
    /// a gesture or by an explicit back.
    fn pop(&mut self) -> AppEffect {
        if self.screens.len() <= 1 {
            return AppEffect::None;
        }

        let motion = self
            .screens
            .pop()
            .and_then(|entry| entry.entered_by)
            .unwrap_or(Navigation::DEFAULT_FORWARD_MOTION)
            .opposite();
        AppEffect::Navigate(Navigation::backward(motion))
    }

    fn push(&mut self, screen: ScreenId, entered_by: Option<SwipeDirection>) -> AppEffect {
        if self.active_screen() == screen {
            return AppEffect::None;
        }

        let entry = ScreenEntry { screen, entered_by };
        if self.screens.push(entry).is_ok() {
            let motion = entered_by.unwrap_or(Navigation::DEFAULT_FORWARD_MOTION);
            AppEffect::Navigate(Navigation::forward(motion))
        } else {
            AppEffect::None
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
    fn charge_urgency_has_fixed_thresholds() {
        let at = |percent, power_present| {
            BatteryStatus {
                millivolts: 3_800,
                percent,
                charging: power_present,
                power_present,
            }
            .level()
        };

        assert_eq!(at(100, false), ChargeLevel::Good);
        assert_eq!(at(51, false), ChargeLevel::Good);
        assert_eq!(at(50, false), ChargeLevel::Low);
        assert_eq!(at(20, false), ChargeLevel::Low);
        assert_eq!(at(19, false), ChargeLevel::Critical);
        assert_eq!(at(0, false), ChargeLevel::Critical);
        // On the charger the number is climbing, so nothing is urgent.
        assert_eq!(at(0, true), ChargeLevel::Good);
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

    #[test]
    fn a_screen_is_left_by_the_opposite_of_the_gesture_that_opened_it() {
        let mut app = AppState::new(ScreenId::Watchface);

        assert_eq!(
            app.navigate(SwipeDirection::Up),
            AppEffect::Navigate(Navigation::forward(SwipeDirection::Up))
        );
        assert_eq!(app.active_screen(), ScreenId::Launcher);
        // The gesture that opened it does not open it again, and the horizontal
        // directions belong to the launcher's own paging.
        assert_eq!(app.navigate(SwipeDirection::Up), AppEffect::None);
        assert_eq!(app.navigate(SwipeDirection::Left), AppEffect::None);
        assert_eq!(app.navigate(SwipeDirection::Right), AppEffect::None);
        assert_eq!(app.active_screen(), ScreenId::Launcher);

        assert_eq!(
            app.navigate(SwipeDirection::Down),
            AppEffect::Navigate(Navigation::backward(SwipeDirection::Down))
        );
        assert_eq!(app.active_screen(), ScreenId::Watchface);
    }

    /// Closes the one gap `position` alone cannot: it makes a new variant fail
    /// to compile, but it cannot make anyone put that variant in `ALL`. Here a
    /// screen that was given a slot and then left out of the list shows up as
    /// an unfilled slot.
    #[test]
    fn every_screen_is_listed_exactly_once() {
        let mut filled = [false; ScreenId::COUNT];
        for screen in ScreenId::ALL {
            let slot = screen.position();
            assert!(
                !filled[slot],
                "{screen:?} shares a slot with another screen"
            );
            filled[slot] = true;
        }
        assert!(
            filled.iter().all(|slot| *slot),
            "a screen has a slot but is missing from ALL"
        );
    }

    #[test]
    fn the_root_ignores_gestures_that_route_nowhere() {
        let mut app = AppState::new(ScreenId::Watchface);

        // Left belongs to quick settings, which does not exist yet; right is
        // the way back, and the root cannot be popped.
        for swipe in [SwipeDirection::Left, SwipeDirection::Right] {
            assert_eq!(app.navigate(swipe), AppEffect::None);
            assert_eq!(app.active_screen(), ScreenId::Watchface);
        }
    }

    /// What is left for the notification screen's own use once navigation has
    /// taken its share.
    ///
    /// The screen browses and dismisses with gestures, so which ones reach it
    /// at all is not an implementation detail of the screen - it is decided
    /// here, and a re-route would silently take a gesture away from it.
    #[test]
    fn the_notification_screen_keeps_the_gestures_it_browses_with() {
        let mut app = AppState::new(ScreenId::Watchface);
        assert_eq!(
            app.navigate(SwipeDirection::Down),
            AppEffect::Navigate(Navigation::forward(SwipeDirection::Down))
        );
        assert_eq!(app.active_screen(), ScreenId::Notifications);

        // Down browses to the next notification and right dismisses one, so
        // navigation must leave both alone.
        for swipe in [SwipeDirection::Down, SwipeDirection::Right] {
            assert_eq!(app.navigate(swipe), AppEffect::None);
            assert_eq!(app.active_screen(), ScreenId::Notifications);
        }

        // Up is the way out, because down is the way in.
        assert_eq!(
            app.navigate(SwipeDirection::Up),
            AppEffect::Navigate(Navigation::backward(SwipeDirection::Up))
        );
        assert_eq!(app.active_screen(), ScreenId::Watchface);
    }

    #[test]
    fn the_button_leaves_a_screen_the_way_its_gesture_would() {
        let mut app = AppState::new(ScreenId::Watchface);
        let _ = app.navigate(SwipeDirection::Up);
        assert_eq!(app.active_screen(), ScreenId::Launcher);

        // The launcher was opened by a swipe up, so it leaves downwards -
        // pressing the button looks exactly like swiping it away.
        assert_eq!(
            app.back(),
            AppEffect::Navigate(Navigation::backward(SwipeDirection::Down))
        );
        assert_eq!(app.active_screen(), ScreenId::Watchface);
    }

    #[test]
    fn the_button_also_leaves_a_screen_no_gesture_can_leave() {
        let mut app = AppState::new(ScreenId::Watchface);
        let _ = app.transition(ScreenAction::Push(ScreenId::DisplaySettings));

        // Nothing opened it by gesture, so before the button this screen could
        // only be left by an on-screen control.
        assert_eq!(
            app.back(),
            AppEffect::Navigate(Navigation::backward(
                Navigation::DEFAULT_FORWARD_MOTION.opposite(),
            ))
        );
        assert_eq!(app.active_screen(), ScreenId::Watchface);
    }

    #[test]
    fn the_root_absorbs_the_button() {
        let mut app = AppState::new(ScreenId::Watchface);

        assert_eq!(app.back(), AppEffect::None);
        assert_eq!(app.active_screen(), ScreenId::Watchface);
    }

    #[test]
    fn the_button_renews_the_idle_timer_like_any_other_input() {
        assert!(AppEvent::BackPressed.is_user_activity());
    }

    #[test]
    fn a_reboot_is_an_explicit_effect() {
        let mut app = AppState::new(ScreenId::Watchface);

        assert_eq!(app.transition(ScreenAction::Reboot), AppEffect::Reboot);
        // Restarting is a hardware effect, not a navigation: the stack is
        // untouched so a refused reset leaves the user where they were.
        assert_eq!(app.active_screen(), ScreenId::Watchface);
    }

    #[test]
    fn every_route_is_reversible_by_its_opposite() {
        for (swipe, target) in [
            (SwipeDirection::Up, ScreenId::Launcher),
            (SwipeDirection::Down, ScreenId::Notifications),
        ] {
            let mut app = AppState::new(ScreenId::Watchface);
            assert_eq!(
                app.navigate(swipe),
                AppEffect::Navigate(Navigation::forward(swipe))
            );
            assert_eq!(app.active_screen(), target);
            // The animation follows the finger in both directions.
            assert_eq!(
                app.navigate(swipe.opposite()),
                AppEffect::Navigate(Navigation::backward(swipe.opposite()))
            );
            assert_eq!(app.active_screen(), ScreenId::Watchface);
        }
    }

    #[test]
    fn a_screen_pushed_without_a_gesture_is_left_by_the_default_back_gesture() {
        let mut app = AppState::new(ScreenId::Watchface);

        assert_eq!(
            app.transition(ScreenAction::Push(ScreenId::DisplaySettings)),
            AppEffect::Navigate(Navigation::forward(Navigation::DEFAULT_FORWARD_MOTION))
        );
        // A tile opened it, so it borrows the default entry gesture: only the
        // reverse of that leaves, and the other three belong to the screen.
        for swipe in [
            SwipeDirection::Up,
            SwipeDirection::Down,
            SwipeDirection::Left,
        ] {
            assert_eq!(app.navigate(swipe), AppEffect::None);
        }
        assert_eq!(app.active_screen(), ScreenId::DisplaySettings);
        assert_eq!(
            app.navigate(Navigation::DEFAULT_FORWARD_MOTION.opposite()),
            AppEffect::Navigate(Navigation::backward(
                Navigation::DEFAULT_FORWARD_MOTION.opposite(),
            ))
        );
        assert_eq!(app.active_screen(), ScreenId::Watchface);

        let _ = app.transition(ScreenAction::Push(ScreenId::DisplaySettings));
        assert_eq!(
            app.transition(ScreenAction::Back),
            AppEffect::Navigate(Navigation::backward(
                Navigation::DEFAULT_FORWARD_MOTION.opposite(),
            ))
        );
        assert_eq!(app.active_screen(), ScreenId::Watchface);
    }

    #[test]
    fn opposites_are_symmetric() {
        for swipe in [
            SwipeDirection::Up,
            SwipeDirection::Down,
            SwipeDirection::Left,
            SwipeDirection::Right,
        ] {
            assert_ne!(swipe.opposite(), swipe);
            assert_eq!(swipe.opposite().opposite(), swipe);
        }
    }

    #[cfg(feature = "diagnostics")]
    #[test]
    fn push_and_back_change_the_active_screen() {
        let mut app = AppState::new(ScreenId::Watchface);

        assert_eq!(
            app.transition(ScreenAction::Push(ScreenId::TouchTest)),
            AppEffect::Navigate(Navigation::forward(Navigation::DEFAULT_FORWARD_MOTION))
        );
        assert_eq!(app.active_screen(), ScreenId::TouchTest);
        assert_eq!(
            app.transition(ScreenAction::Back),
            AppEffect::Navigate(Navigation::backward(
                Navigation::DEFAULT_FORWARD_MOTION.opposite(),
            ))
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
                AppEffect::Navigate(Navigation::forward(Navigation::DEFAULT_FORWARD_MOTION))
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

    /// The regression behind "the tile lights up and nothing happens".
    ///
    /// The report that ends a touch does not promise a sensible position, and
    /// activation used to require one inside the button. A release that read as
    /// somewhere else left the press captured and the outcome dropped, so the
    /// user pressed twice and blamed the swipe.
    #[test]
    fn a_press_is_completed_by_the_lift_wherever_it_is_reported() {
        let mut button = Button::new(ButtonBounds::new(10, 20, 100, 40));
        let _ = button.handle_event(AppEvent::Touch {
            x: 20,
            y: 30,
            pressed: true,
        });

        // The origin is the value a CST816S is most likely to leave behind, and
        // it is nowhere near this button.
        assert_eq!(
            button.handle_event(AppEvent::Touch {
                x: 0,
                y: 0,
                pressed: false,
            }),
            ButtonOutcome::Activated
        );
        assert_eq!(button.state(), ButtonState::Idle);
    }

    /// Capture does not mean a button can be activated without being pressed:
    /// a lift with no press behind it belongs to nobody.
    #[test]
    fn a_lift_without_a_press_activates_nothing() {
        let mut button = Button::new(ButtonBounds::new(10, 20, 100, 40));

        assert_eq!(
            button.handle_event(AppEvent::Touch {
                x: 20,
                y: 30,
                pressed: false,
            }),
            ButtonOutcome::None
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
    fn a_cancelled_touch_abandons_the_press_without_activating() {
        let mut button = Button::new(ButtonBounds::new(10, 20, 100, 40));
        let _ = button.handle_event(AppEvent::Touch {
            x: 20,
            y: 30,
            pressed: true,
        });

        assert_eq!(
            button.handle_event(AppEvent::TouchCancelled),
            ButtonOutcome::Redraw
        );
        assert_eq!(button.state(), ButtonState::Idle);
        // The release that ends the gesture arrives inside the button and must
        // find nothing left to activate.
        assert_eq!(
            button.handle_event(AppEvent::Touch {
                x: 20,
                y: 30,
                pressed: false,
            }),
            ButtonOutcome::None
        );
        // A cancel with no press in flight is not a redraw either.
        assert_eq!(
            button.handle_event(AppEvent::TouchCancelled),
            ButtonOutcome::None
        );
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
