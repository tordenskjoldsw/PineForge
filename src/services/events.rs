use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    channel::Channel,
    signal::Signal,
    watch::{Receiver, Watch},
};

#[cfg(feature = "diagnostics")]
use pineforge_state::HeartRateCommand;
use pineforge_state::{
    AppEvent, BatteryStatus, DisplaySettings, PowerCommand, SystemPowerState, VibrationPattern,
    WallClockReference,
};

pub static UI_EVENTS: Channel<CriticalSectionRawMutex, AppEvent, 8> = Channel::new();

pub static POWER_COMMANDS: Channel<CriticalSectionRawMutex, PowerCommand, 8> = Channel::new();

/// Enforces `PineTime`'s proven touch -> motion -> heart-rate bus bring-up.
pub static TOUCH_READY: Signal<CriticalSectionRawMutex, ()> = Signal::new();
pub static MOTION_READY: Signal<CriticalSectionRawMutex, ()> = Signal::new();
#[cfg(feature = "diagnostics")]
pub static HEART_RATE_COMMANDS: Channel<CriticalSectionRawMutex, HeartRateCommand, 2> =
    Channel::new();

/// Latest logical system state for display, motion, and future services.
pub static SYSTEM_POWER: Watch<CriticalSectionRawMutex, SystemPowerState, 3> = Watch::new();

pub type SystemPowerReceiver = Receiver<'static, CriticalSectionRawMutex, SystemPowerState, 3>;

/// Reserves one of the fixed display, motion, and heart-rate subscriptions.
pub fn system_power_receiver() -> SystemPowerReceiver {
    SYSTEM_POWER
        .receiver()
        .expect("system power receiver capacity is fixed by architecture")
}

/// Latest validated display settings, published by the settings service.
pub static DISPLAY_SETTINGS: Watch<CriticalSectionRawMutex, DisplaySettings, 2> = Watch::new();

pub type DisplaySettingsReceiver = Receiver<'static, CriticalSectionRawMutex, DisplaySettings, 2>;

/// Reserves one of the fixed display and power subscriptions.
pub fn display_settings_receiver() -> DisplaySettingsReceiver {
    DISPLAY_SETTINGS
        .receiver()
        .expect("display settings receiver capacity is fixed by architecture")
}

/// Validated settings snapshots requested by UI screens.
pub static SETTINGS_COMMANDS: Channel<CriticalSectionRawMutex, DisplaySettings, 2> = Channel::new();

/// Haptic requests for the vibration task. Senders use `try_send` so a busy
/// motor drops feedback instead of ever stalling the UI.
pub static VIBRATION_COMMANDS: Channel<CriticalSectionRawMutex, VibrationPattern, 4> =
    Channel::new();

/// Latest battery measurement for the BLE battery service.
pub static BATTERY_STATUS: Watch<CriticalSectionRawMutex, BatteryStatus, 2> = Watch::new();

pub type BatteryStatusReceiver = Receiver<'static, CriticalSectionRawMutex, BatteryStatus, 2>;

/// Reserves one of the fixed BLE and future consumer subscriptions.
pub fn battery_status_receiver() -> BatteryStatusReceiver {
    BATTERY_STATUS
        .receiver()
        .expect("battery status receiver capacity is fixed by architecture")
}

/// Wall-clock anchor written by the BLE Current Time Service.
pub static WALL_CLOCK: Watch<CriticalSectionRawMutex, WallClockReference, 2> = Watch::new();

pub type WallClockReceiver = Receiver<'static, CriticalSectionRawMutex, WallClockReference, 2>;

/// Reserves one of the fixed display and future consumer subscriptions.
pub fn wall_clock_receiver() -> WallClockReceiver {
    WALL_CLOCK
        .receiver()
        .expect("wall clock receiver capacity is fixed by architecture")
}
