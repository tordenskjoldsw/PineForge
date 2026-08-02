#![no_std]

//! Pure application state transitions for `PineForge`.
//!
//! This crate deliberately has no Embassy or hardware dependencies so its
//! transitions can be exercised on a host.

use heapless::Vec;

mod ppg;
pub use ppg::{PpgAnalysis, PpgProcessor};

mod wake;
pub use wake::RaiseToWakeDetector;

mod bond;
pub use bond::{
    BOND_PAYLOAD_MAX, BOND_RECORD_LEN, BOND_SCHEMA_LEN, BondRecord, bond_schema_tag, frame_bond,
    parse_bond,
};

mod clock;
pub use clock::{
    CLOCK_RECORD_LEN, CLOCK_YEAR_MAX, CLOCK_YEAR_MIN, CalendarDate, ClockSnapshot, DateEditor,
    DateField, TimeEditor, TimeField, WallClockReference, WallTime, clock_sequence_is_newer,
    days_in_month, is_leap_year, parse_cts,
};

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

mod music;
pub use music::{
    MUSIC_MINUTES_MAX, MUSIC_TEXT_MAX, MusicControl, MusicPlayback, MusicState, parse_be_u32,
};

mod settings;
pub use setting::{Setting, WATCHFACE_NAMES};
pub use settings::{
    BRIGHTNESS_LEVELS, BRIGHTNESS_NAMES, DIM_TIMEOUT_NAMES, DIM_TIMEOUTS_MILLIS, DecodeError,
    DisplaySettings, HEART_RATE_INTERVALS_SECONDS, HEART_RATE_MODE_NAMES, OFF_TIMEOUT_NAMES,
    OFF_TIMEOUTS_MILLIS, SETTINGS_RECORD_LEN, SettingsError, SettingsSlot, SlotDecision,
    WAKE_GESTURE_NAMES, WAKE_GESTURES, WakeGesture, WakeGestures, crc32, select_slot,
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
    CLOCK_JOURNAL_A_ADDRESS, CLOCK_JOURNAL_B_ADDRESS, STORAGE_BASE, STORAGE_DATA_SECTOR_COUNT,
    STORAGE_END, STORAGE_FORMAT_VERSION, STORAGE_HEADER_LEN, STORAGE_PROGRESS_OFFSET,
    STORAGE_READY_HEADER_OFFSET, STORAGE_SECTOR_COUNT, STORAGE_SECTOR_SIZE, StorageHeader,
    decode_storage_header, encode_storage_header, storage_header_version,
};

mod stopwatch;
pub use stopwatch::{StopwatchControl, StopwatchState};

mod timer;
pub use timer::{
    TIMER_DEFAULT_MINUTES, TIMER_MAX_MINUTES, TIMER_MIN_MINUTES, TimerControl, TimerOutcome,
    TimerPhase, TimerState,
};

pub const SCREEN_STACK_CAPACITY: usize = 4;

/// Steps in a day, and what every gauge showing progress divides against.
///
/// Product policy, so it lives here rather than in whichever screen happens to
/// draw a bar: the steps app and the FORGE watchface both show progress toward
/// it, and two copies of the number would let them disagree about what a full
/// bar means.
pub const DAILY_STEP_GOAL: u32 = 10_000;

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
    /// Sleep now, because the user asked rather than because the timeout came.
    SleepNow,
}

/// Deterministic system-power policy, independent from clocks and hardware.
pub struct SystemPowerPolicy {
    config: PowerConfig,
    state: SystemPowerState,
    last_activity_millis: u64,
    /// Set when sleep was asked for rather than waited for, and cleared by the
    /// next activity. Held as a flag rather than by winding the idle clock back
    /// past the timeout, because that arithmetic saturates: a watch that has
    /// been up for less time than the timeout is long cannot be backdated far
    /// enough, and the next tick would decide it had just been used.
    sleep_requested: bool,
}

impl SystemPowerPolicy {
    #[must_use]
    pub const fn new(now_millis: u64, config: PowerConfig) -> Self {
        Self {
            config,
            state: SystemPowerState::Interactive,
            last_activity_millis: now_millis,
            sleep_requested: false,
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
        // Activity is what a requested sleep is waiting for. Clearing it here
        // rather than on wake means one rule governs both: whatever wakes the
        // watch also ends the request that put it to sleep.
        self.sleep_requested = false;
        self.set_state(SystemPowerState::Interactive)
    }

    /// Sleeps at once, at the user's request rather than the timeout's.
    ///
    /// Holds until the next activity, which is also what wakes the watch, so
    /// the panel stays dark through every tick in between.
    pub fn on_sleep_request(&mut self) -> Option<SystemPowerState> {
        self.sleep_requested = true;
        self.set_state(SystemPowerState::Sleeping)
    }

    /// Advances inactivity policy and returns a state change, if any.
    pub fn advance(&mut self, now_millis: u64) -> Option<SystemPowerState> {
        if self.sleep_requested {
            // Nothing the clock says can lighten a sleep that was asked for.
            return self.set_state(SystemPowerState::Sleeping);
        }
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

/// The brightest the panel goes.
///
/// Taken from the top of the presets rather than written out again, so a lamp
/// cannot ask for a level the settings screen has no way to return from.
pub const BRIGHTNESS_MAX: u8 = BRIGHTNESS_LEVELS[BRIGHTNESS_LEVELS.len() - 1];

/// What the backlight shows while the watch is idle but not yet asleep.
pub const DIMMED_BRIGHTNESS: u8 = 1;

/// How bright the panel should be, given everything that has a say.
///
/// One answer in one place. This used to be spelled out at eight call sites in
/// the display task - every power transition, every settings change, the wake
/// path, and the modal path - which is fine until a ninth thing has an opinion,
/// and a lamp is exactly that.
///
/// A lit lamp outranks both the brightness setting and idle dimming, because
/// the panel is the light: dimming it is not a power saving, it is the feature
/// failing. Sleep still wins, but the lamp screen holds the watch awake so that
/// case does not arise while it is open.
#[must_use]
pub const fn panel_backlight(
    power: SystemPowerState,
    lamp_lit: bool,
    settings: DisplaySettings,
) -> u8 {
    match power {
        SystemPowerState::Sleeping => 0,
        SystemPowerState::Idle => {
            if lamp_lit {
                BRIGHTNESS_MAX
            } else {
                DIMMED_BRIGHTNESS
            }
        }
        SystemPowerState::Interactive => {
            if lamp_lit {
                BRIGHTNESS_MAX
            } else {
                settings.brightness()
            }
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

/// Whether a board peripheral completed its start-up probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeripheralStatus {
    Ready,
    Unavailable,
}

/// What the external flash probe learned, including the ID needed to identify
/// an unexpected replacement without attaching a debugger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlashStatus {
    Ready([u8; 3]),
    Unrecognized([u8; 3]),
    Unavailable,
}

/// The deepest stack use observed since this boot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StackUsage {
    pub used: u16,
    pub capacity: u16,
}

/// Whether the bootloader may still roll this image back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FirmwareImageState {
    Trial,
    Confirmed,
}

/// A compact fault code suitable for a sealed watch's status screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemFault {
    Touch,
    Motion,
    HeartRate,
    Flash,
    Storage,
    Dfu(DfuFailReason),
}

/// The latest system facts collected by the About screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemStatus {
    pub touch: Option<PeripheralStatus>,
    pub motion: Option<AccelerometerKind>,
    pub heart_rate: Option<HeartRateSensorKind>,
    pub flash: Option<FlashStatus>,
    pub image: FirmwareImageState,
    pub ble: BleState,
    pub stack: Option<StackUsage>,
    pub last_fault: Option<SystemFault>,
}

impl SystemStatus {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            touch: None,
            motion: None,
            heart_rate: None,
            flash: None,
            image: FirmwareImageState::Trial,
            ble: BleState::Off,
            stack: None,
            last_fault: None,
        }
    }

    /// Applies one system reading and reports whether anything visible moved.
    pub fn apply(&mut self, event: AppEvent) -> bool {
        let before = *self;
        match event {
            AppEvent::TouchControllerUpdated(status) => {
                self.touch = Some(status);
                if status == PeripheralStatus::Unavailable {
                    self.last_fault = Some(SystemFault::Touch);
                }
            }
            AppEvent::AccelerometerDetected(kind) => {
                self.motion = Some(kind);
                if matches!(
                    kind,
                    AccelerometerKind::Unknown(_) | AccelerometerKind::Unavailable
                ) {
                    self.last_fault = Some(SystemFault::Motion);
                }
            }
            AppEvent::HeartRateSensorDetected(kind) => {
                self.heart_rate = Some(kind);
                if matches!(
                    kind,
                    HeartRateSensorKind::Unknown(_) | HeartRateSensorKind::Unavailable
                ) {
                    self.last_fault = Some(SystemFault::HeartRate);
                }
            }
            AppEvent::FlashUpdated(status) => {
                self.flash = Some(status);
                if !matches!(status, FlashStatus::Ready(_)) {
                    self.last_fault = Some(SystemFault::Flash);
                }
            }
            AppEvent::FirmwareImageUpdated(image) => self.image = image,
            AppEvent::StackUpdated(stack) => self.stack = Some(stack),
            AppEvent::BleUpdated(state) => {
                self.ble = state;
                if let BleState::DfuFailed(reason) = state {
                    self.last_fault = Some(SystemFault::Dfu(reason));
                }
            }
            AppEvent::StorageUpdated(StorageState::Failed) => {
                self.last_fault = Some(SystemFault::Storage);
            }
            _ => {}
        }
        *self != before
    }
}

impl Default for SystemStatus {
    fn default() -> Self {
        Self::new()
    }
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
    /// Repeating alert used when a countdown reaches zero.
    Alarm,
}

impl VibrationPattern {
    /// Returns `(on_millis, pause_millis, pulse_count)`.
    #[must_use]
    pub const fn pulses(self) -> (u64, u64, u8) {
        match self {
            Self::Tap => (25, 0, 1),
            Self::Double => (50, 100, 2),
            Self::Long => (150, 0, 1),
            // About five seconds if it is not acknowledged first.
            Self::Alarm => (250, 180, 12),
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

    /// Where the watch is drawing power from.
    #[must_use]
    pub const fn source(self) -> PowerSource {
        if self.charging {
            PowerSource::Charging
        } else if self.power_present {
            PowerSource::External
        } else {
            PowerSource::Battery
        }
    }
}

/// How long to wait between capacity measurements.
///
/// A measurement costs an ADC conversion and a wakeup, so the rate is a battery
/// decision rather than a display one, and it is deliberately slow: the
/// discharge curve is nearly flat and the estimator will not let the number
/// move faster than a point a minute anyway. `InfiniTime` settles on the same
/// ten minutes.
///
/// External power earns a faster rate because that is when the number is
/// actually moving and someone is plausibly watching it. Diagnostics builds go
/// faster still, which is what makes a suspect reading observable at all on a
/// sealed watch.
///
/// Note what this is not: a way to notice the charger. Waiting ten minutes to
/// see a pad would be a bug, and was one - the charger pins are watched for
/// changes, and this interval is only the backstop behind them.
#[must_use]
pub const fn battery_sample_interval_seconds(power_present: bool) -> u64 {
    if cfg!(feature = "diagnostics") {
        30
    } else if power_present {
        60
    } else {
        10 * 60
    }
}

/// The `PineTime`'s two charger pins, as levels already resolved to meaning.
///
/// Both are active low, and reading them is the firmware's job; deciding what a
/// pair of them means is this type's. Kept apart because the pins are the one
/// part no host can exercise, and the meaning is the part that was wrong.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChargerPins {
    /// P0.19 read low: the watch is on a pad that has power.
    pub power_present: bool,
    /// P0.12 read low: the charger reports current flowing.
    pub charge_indicated: bool,
}

impl ChargerPins {
    /// Whether the watch is charging.
    ///
    /// Both pins rather than the charge indication alone. P0.12 has no pull, so
    /// an unpowered charger can leave it floating, and external power is what
    /// makes the indication mean anything. `InfiniTime` reads P0.12 by itself
    /// and gets away with it; requiring both is the conservative reading and
    /// costs nothing, as long as external power is watched for changes rather
    /// than merely read - which is exactly what it was not.
    #[must_use]
    pub const fn charging(self) -> bool {
        self.power_present && self.charge_indicated
    }
}

/// Where the watch is drawing power from, as a face shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerSource {
    /// On the pad and taking current.
    Charging,
    /// On the pad, but not taking current - a watch that is already full.
    External,
    /// Running off its own battery.
    Battery,
}

impl PowerSource {
    /// The tag a face shows for this source.
    ///
    /// Here rather than in the drawing code because both faces showed it and
    /// both spelled the decision out for themselves, which is two places for
    /// one product answer to drift apart.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Charging => "CHG",
            Self::External => "PWR",
            Self::Battery => "BAT",
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
    /// Take one reading now, whether or not periodic measurement is on.
    ///
    /// The sensor was reachable only on an interval, which is not how anyone
    /// asks for their pulse. A one-shot leaves the periodic setting exactly as
    /// it found it.
    MeasureNow,
    /// Abandon the reading in flight.
    ///
    /// Distinct from disabling: someone who gives up on a measurement has not
    /// asked for periodic measurement to stop happening, and the setting is not
    /// this command's to change.
    Stop,
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
    WakeGesture,
    /// Picks the watchface, reached from the settings root.
    WatchfaceSelect,
    /// Firmware confirmation and a software reboot.
    Firmware,
    /// Which build this is: release, commit, and date.
    About,
    /// The watch used as a lamp: the panel itself is the light.
    Flashlight,
    /// An on-demand heart-rate reading.
    ///
    /// Named apart from [`Self::HeartRate`], which is the settings leaf that
    /// turns periodic measurement on and picks its interval. This is the app
    /// that takes one now and shows what came back.
    Pulse,
    /// The day's step count, and how far it is through the daily goal.
    Steps,
    /// What the phone is playing, and the transport that changes it.
    Music,
    /// A monotonic stopwatch with start, pause, resume, and reset.
    Stopwatch,
    /// A monotonic countdown that raises a system alarm at zero.
    Timer,
    /// Sets the local wall-clock hour and minute without a phone.
    Time,
    /// Sets the local calendar date without a phone.
    Date,
    /// Enables or disables Bluetooth advertising and connections.
    Bluetooth,
    #[cfg(feature = "diagnostics")]
    TouchTest,
}

impl ScreenId {
    /// How many screens this build has.
    pub const COUNT: usize = if cfg!(feature = "diagnostics") {
        22
    } else {
        21
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
        Self::WakeGesture,
        Self::WatchfaceSelect,
        Self::Firmware,
        Self::About,
        Self::Flashlight,
        Self::Pulse,
        Self::Steps,
        Self::Music,
        Self::Stopwatch,
        Self::Timer,
        Self::Time,
        Self::Date,
        Self::Bluetooth,
        #[cfg(feature = "diagnostics")]
        Self::TouchTest,
    ];

    /// Whether this screen holds the watch awake while it is showing.
    ///
    /// The lamp does, for as long as the app is open rather than only while it
    /// is lit: a light that went out on its own timeout partway through being
    /// used would be worse than no light. `InfiniTime` takes the same wake lock
    /// for the lifetime of its flashlight screen.
    ///
    /// The pulse app does too, and not as a convenience: the heart-rate service
    /// abandons a measurement the moment the system goes to sleep, and a
    /// reading takes longer than the default twenty-second timeout. Without the
    /// lock every measurement would be cut off just before it produced a
    /// number.
    ///
    /// These two and nothing else. Anything else holding the panel on
    /// indefinitely would be a battery bug wearing a feature's name.
    #[must_use]
    pub const fn keeps_awake(self) -> bool {
        matches!(self, Self::Flashlight | Self::Pulse)
    }

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
            Self::WakeGesture => 8,
            Self::WatchfaceSelect => 9,
            Self::Firmware => 10,
            Self::About => 11,
            Self::Flashlight => 12,
            Self::Pulse => 13,
            Self::Steps => 14,
            Self::Music => 15,
            Self::Stopwatch => 16,
            Self::Timer => 17,
            Self::Time => 18,
            Self::Date => 19,
            Self::Bluetooth => 20,
            #[cfg(feature = "diagnostics")]
            Self::TouchTest => 21,
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
    /// What the phone is playing changed.
    ///
    /// Carries nothing, for the reason the notification tally carries only a
    /// count: a `MusicState` is two 40-byte text buffers, and an `AppEvent` is
    /// `Copy` and sits in a fixed-capacity channel. The record itself is filed
    /// straight into the screen that shows it, exactly as an arriving
    /// notification is, and this says only that it moved.
    MusicUpdated,
    /// A high-resolution monotonic observation addressed to the stopwatch.
    StopwatchTick(u64),
    /// A monotonic observation addressed to the countdown screen.
    TimerTick(u64),
    /// The countdown crossed zero and its system modal is owed.
    TimerExpired,
    DisplaySettingsUpdated(DisplaySettings),
    BleUpdated(BleState),
    StorageUpdated(StorageState),
    TouchControllerUpdated(PeripheralStatus),
    FlashUpdated(FlashStatus),
    FirmwareImageUpdated(FirmwareImageState),
    StackUpdated(StackUsage),
}

impl AppEvent {
    #[must_use]
    pub const fn is_user_activity(self) -> bool {
        matches!(
            self,
            Self::Touch { .. } | Self::Swipe(_) | Self::BackPressed
        )
    }

    /// Whether this event carries a reading the shared [`WatchState`] holds.
    ///
    /// A reading is not addressed to whichever screen happens to be showing:
    /// it is a fact about the watch, and the face is the only screen that keeps
    /// one. So it has to reach that state whatever is on the panel and whether
    /// or not the panel is even on - the display task applies it before it
    /// decides what to paint, and painting is what sleep gates.
    ///
    /// Getting that order wrong is not a visibly wrong screen, it is a stale
    /// one: the reading is consumed from the channel either way, so a dropped
    /// one is simply never seen again. The watch slept twenty seconds after
    /// each touch and sampled the battery every ten minutes, so nearly every
    /// reading landed on a sleeping watch - which is how a percentage came to
    /// sit where it was at boot, and why a charger on the pad never turned
    /// `BAT` into `CHG`.
    ///
    /// [`WatchState`]: crate::WatchState
    #[must_use]
    pub const fn is_reading(self) -> bool {
        match self {
            Self::BatteryUpdated(_)
            | Self::StepsUpdated(_)
            | Self::BleUpdated(_)
            | Self::NotificationsChanged(_)
            | Self::HeartRateStateUpdated(_)
            | Self::HeartRateSensorDetected(_)
            | Self::HeartRateAnalysisUpdated(_)
            | Self::MusicUpdated
            | Self::AccelerometerDetected(_)
            | Self::TouchControllerUpdated(_)
            | Self::FlashUpdated(_)
            | Self::FirmwareImageUpdated(_)
            | Self::StackUpdated(_)
            | Self::StorageUpdated(_) => true,
            #[cfg(feature = "diagnostics")]
            Self::AccelerationUpdated(_)
            | Self::FeatureEngineUpdated(_)
            | Self::HeartRateRawSampleUpdated(_) => true,
            // Input and navigation are addressed to the active screen; a tick
            // is timing rather than a reading and only ever arrives awake;
            // settings and storage belong to screens that are handed them.
            Self::Touch { .. }
            | Self::Swipe(_)
            | Self::TouchCancelled
            | Self::BackPressed
            | Self::Tick { .. }
            | Self::DisplaySettingsUpdated(_) => false,
            Self::StopwatchTick(_) | Self::TimerTick(_) | Self::TimerExpired => false,
        }
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
    /// Ask for a heart-rate reading now.
    MeasureHeartRate,
    /// Give up on the reading in flight.
    StopHeartRate,
    /// Ask the phone to do something to what it is playing.
    MusicControl(MusicControl),
    StopwatchControl(StopwatchControl),
    TimerControl(TimerControl),
    SetTime(WallTime),
    SetDate(CalendarDate),
    /// The screen turned to another of its own pages, the way the gesture
    /// travelled. The stack has not moved, so this is not a navigation - but it
    /// looks like one to the eye and is drawn like one.
    Paged(SwipeDirection),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppEffect {
    None,
    Navigate(Navigation),
    /// Redraw the active screen as having moved this way; see
    /// [`ScreenAction::Paged`].
    PageTurn(SwipeDirection),
    RequestRollback,
    ApplySettings(DisplaySettings),
    ConfirmFirmware,
    Reboot,
    MeasureHeartRate,
    StopHeartRate,
    MusicControl(MusicControl),
    StopwatchControl(StopwatchControl),
    TimerControl(TimerControl),
    SetTime(WallTime),
    SetDate(CalendarDate),
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
        // Left is deliberately unassigned. It was once reserved for a quick
        // settings panel, which was dropped: the launcher is already one swipe
        // from the face and already carries the torch and the settings gear, so
        // a second grid of tiles beside it had nothing left to do. Whatever
        // claims this gesture should come from something the watch turns out to
        // need, not from holding it empty for a screen nobody specified.
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
            ScreenAction::MeasureHeartRate => AppEffect::MeasureHeartRate,
            ScreenAction::StopHeartRate => AppEffect::StopHeartRate,
            ScreenAction::MusicControl(control) => AppEffect::MusicControl(control),
            ScreenAction::StopwatchControl(control) => AppEffect::StopwatchControl(control),
            ScreenAction::TimerControl(control) => AppEffect::TimerControl(control),
            ScreenAction::SetTime(time) => AppEffect::SetTime(time),
            ScreenAction::SetDate(date) => AppEffect::SetDate(date),
            ScreenAction::Paged(motion) => AppEffect::PageTurn(motion),
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
    fn system_status_keeps_the_latest_fault_and_probe_results() {
        let mut status = SystemStatus::new();
        assert!(status.apply(AppEvent::TouchControllerUpdated(PeripheralStatus::Ready)));
        assert!(status.apply(AppEvent::FlashUpdated(FlashStatus::Ready([
            0x0b, 0x40, 0x16
        ]))));
        assert_eq!(status.last_fault, None);

        assert!(status.apply(AppEvent::AccelerometerDetected(
            AccelerometerKind::Unavailable
        )));
        assert_eq!(status.last_fault, Some(SystemFault::Motion));
        assert!(!status.apply(AppEvent::AccelerometerDetected(
            AccelerometerKind::Unavailable
        )));
    }

    #[test]
    fn a_dfu_failure_is_retained_after_the_connection_moves_on() {
        let mut status = SystemStatus::new();
        let _ = status.apply(AppEvent::BleUpdated(BleState::DfuFailed(
            DfuFailReason::VerifyFailed,
        )));
        let _ = status.apply(AppEvent::BleUpdated(BleState::Advertising));

        assert_eq!(status.ble, BleState::Advertising);
        assert_eq!(
            status.last_fault,
            Some(SystemFault::Dfu(DfuFailReason::VerifyFailed))
        );
    }

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

    /// A lamp outranks the setting and the idle dimming, but never sleep.
    #[test]
    fn a_lit_lamp_beats_the_brightness_setting_and_the_idle_dimming() {
        // Explicitly the dimmest preset. The default happens to be the
        // brightest, which would make a lamp indistinguishable from an ordinary
        // screen and prove nothing.
        let dim = DisplaySettings::DEFAULT.with_brightness(BRIGHTNESS_LEVELS[0]);
        assert!(dim.brightness() < BRIGHTNESS_MAX);

        assert_eq!(
            panel_backlight(SystemPowerState::Interactive, true, dim),
            BRIGHTNESS_MAX
        );
        // Dimming a lamp is not a power saving, it is the feature failing.
        assert_eq!(
            panel_backlight(SystemPowerState::Idle, true, dim),
            BRIGHTNESS_MAX
        );
        // Sleep still wins. The lamp screen keeps the watch awake so this does
        // not arise while it is open, but a dark panel must never be lit by a
        // flag left over from before.
        assert_eq!(panel_backlight(SystemPowerState::Sleeping, true, dim), 0);
    }

    /// Without a lamp the answer is exactly what it was before the lamp
    /// existed, which is what makes gathering the decision safe.
    #[test]
    fn without_a_lamp_the_panel_follows_the_setting_and_the_power_state() {
        for &level in &BRIGHTNESS_LEVELS {
            let settings = DisplaySettings::DEFAULT.with_brightness(level);
            assert_eq!(settings.brightness(), level, "a preset is always accepted");

            assert_eq!(
                panel_backlight(SystemPowerState::Interactive, false, settings),
                level
            );
            assert_eq!(
                panel_backlight(SystemPowerState::Idle, false, settings),
                DIMMED_BRIGHTNESS
            );
            assert_eq!(
                panel_backlight(SystemPowerState::Sleeping, false, settings),
                0
            );
        }
    }

    /// Holding the panel on is a privilege, and the list of screens with it is
    /// checked against every screen rather than asserted about one - anything
    /// that acquired it by accident would be a battery bug wearing a feature's
    /// name.
    ///
    /// The lamp, because it *is* the light. The pulse app, because the service
    /// abandons a measurement on sleep and a reading outlasts the default
    /// timeout.
    #[test]
    fn only_the_lamp_and_the_pulse_app_keep_the_watch_awake() {
        for screen in ScreenId::ALL {
            let expected = matches!(screen, ScreenId::Flashlight | ScreenId::Pulse);
            assert_eq!(
                screen.keeps_awake(),
                expected,
                "{screen:?} disagrees about holding the watch awake"
            );
        }
    }

    /// The brightest a lamp can ask for has to be a level the user can also
    /// choose, or leaving the lamp would strand the panel somewhere the
    /// settings screen cannot describe.
    #[test]
    fn the_lamp_brightness_is_one_the_settings_screen_offers() {
        assert!(BRIGHTNESS_LEVELS.contains(&BRIGHTNESS_MAX));
        assert_eq!(BRIGHTNESS_MAX, *BRIGHTNESS_LEVELS.iter().max().unwrap());
        // The backlight driver clamps at seven; asking for more would silently
        // become this anyway.
        assert!(BRIGHTNESS_MAX <= 7);
    }

    /// The rates are ordered rather than pinned to exact numbers, because the
    /// ordering is the property that matters and the numbers are tuning.
    #[test]
    fn measuring_is_faster_on_the_charger_than_off_it() {
        let powered = battery_sample_interval_seconds(true);
        let discharging = battery_sample_interval_seconds(false);

        assert!(powered <= discharging);
        // Slow enough that it cannot be what notices a charger, which is the
        // job the pin edges do. A rate fast enough to serve as that backstop
        // would be one nobody thought to check.
        assert!(powered >= 30);
    }

    /// All four pin combinations, including the two that only happen when
    /// something is wrong. The pair the firmware can never read correctly is a
    /// floating charge indication with no external power, and that one has to
    /// come out as `Battery` rather than as a watch that claims to be charging
    /// while lying on a desk.
    #[test]
    fn the_charger_pins_only_mean_charging_when_power_is_actually_present() {
        const fn pins(power_present: bool, charge_indicated: bool) -> ChargerPins {
            ChargerPins {
                power_present,
                charge_indicated,
            }
        }

        assert!(pins(true, true).charging());
        // On the pad and full: the charger has stopped pushing current. This is
        // the case that moves only the power pin, which is why watching the
        // charge pin alone was not enough to notice a watch being set down.
        assert!(!pins(true, false).charging());
        // Off the pad. The charge pin has no pull, so this pair is what a
        // floating input looks like and must never read as charging.
        assert!(!pins(false, true).charging());
        assert!(!pins(false, false).charging());
    }

    /// What the face shows, for every state the flags can be in.
    #[test]
    fn every_power_state_has_one_tag_and_the_faces_agree_on_it() {
        const fn status(charging: bool, power_present: bool) -> BatteryStatus {
            BatteryStatus {
                millivolts: 3_900,
                percent: 80,
                charging,
                power_present,
            }
        }

        assert_eq!(status(true, true).source(), PowerSource::Charging);
        assert_eq!(status(false, true).source(), PowerSource::External);
        assert_eq!(status(false, false).source(), PowerSource::Battery);
        // Charging without external power cannot be produced by `ChargerPins`,
        // but `BatteryStatus` is a plain record anyone can build, so the tag is
        // still defined rather than left to whichever branch happens to be
        // first.
        assert_eq!(status(true, false).source(), PowerSource::Charging);

        assert_eq!(PowerSource::Charging.label(), "CHG");
        assert_eq!(PowerSource::External.label(), "PWR");
        assert_eq!(PowerSource::Battery.label(), "BAT");
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

    /// A requested sleep has to survive the tick that follows it.
    ///
    /// The state is derived from the idle clock, so setting it without winding
    /// that clock back would leave `advance` - which runs at least once a
    /// second - deciding the watch had just been used and lighting the panel
    /// again. The button would appear not to work.
    #[test]
    fn a_requested_sleep_is_not_undone_by_the_next_tick() {
        let mut policy = SystemPowerPolicy::new(1_000, PowerConfig::DEFAULT);

        assert_eq!(policy.on_sleep_request(), Some(SystemPowerState::Sleeping));
        assert_eq!(policy.state(), SystemPowerState::Sleeping);
        assert_eq!(policy.next_deadline_millis(), None);

        assert_eq!(policy.advance(5_001), None);
        assert_eq!(policy.advance(6_000), None);
        assert_eq!(policy.state(), SystemPowerState::Sleeping);

        // And the watch still wakes normally afterwards.
        assert_eq!(
            policy.on_activity(7_000),
            Some(SystemPowerState::Interactive)
        );
    }

    /// Asking for sleep while already asleep reports no change, so the display
    /// task has nothing to act on and does not re-run its sleep sequence.
    #[test]
    fn requesting_sleep_twice_reports_one_change() {
        let mut policy = SystemPowerPolicy::new(0, PowerConfig::DEFAULT);

        assert_eq!(policy.on_sleep_request(), Some(SystemPowerState::Sleeping));
        assert_eq!(policy.on_sleep_request(), None);
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
            VibrationPattern::Alarm,
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
    fn manual_clock_values_are_explicit_effects() {
        let mut app = AppState::new(ScreenId::Watchface);
        let time = WallTime::new(18, 42, 0).unwrap();
        let date = CalendarDate::new(2028, 2, 29).unwrap();

        assert_eq!(
            app.transition(ScreenAction::SetTime(time)),
            AppEffect::SetTime(time)
        );
        assert_eq!(
            app.transition(ScreenAction::SetDate(date)),
            AppEffect::SetDate(date)
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

        // Left is unassigned; right is the way back, and the root cannot be
        // popped.
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
