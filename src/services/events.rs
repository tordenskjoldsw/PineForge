use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    channel::Channel,
    watch::{Receiver, Watch},
};

use pineforge_state::{AppEvent, PowerCommand, SystemPowerState};

pub static UI_EVENTS: Channel<CriticalSectionRawMutex, AppEvent, 8> = Channel::new();

pub static POWER_COMMANDS: Channel<CriticalSectionRawMutex, PowerCommand, 8> = Channel::new();

/// Latest logical system state for display, motion, and future services.
pub static SYSTEM_POWER: Watch<CriticalSectionRawMutex, SystemPowerState, 2> = Watch::new();

pub type SystemPowerReceiver = Receiver<'static, CriticalSectionRawMutex, SystemPowerState, 2>;

/// Reserves one of the fixed display/motion subscriptions.
pub fn system_power_receiver() -> SystemPowerReceiver {
    SYSTEM_POWER
        .receiver()
        .expect("system power receiver capacity is fixed by architecture")
}
