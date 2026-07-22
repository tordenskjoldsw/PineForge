use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    channel::Channel,
    signal::Signal,
    watch::{Receiver, Watch},
};

#[cfg(feature = "diagnostics")]
use pineforge_state::HeartRateCommand;
use pineforge_state::{AppEvent, PowerCommand, SystemPowerState};

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
