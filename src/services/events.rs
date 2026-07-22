use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    channel::Channel,
    watch::{Receiver, Watch},
};

use pineforge_state::{AppEvent, PowerCommand, SystemPowerState};

#[cfg(feature = "diagnostics")]
#[derive(Clone, Copy)]
pub enum SensorBusClient {
    Motion,
    Touch,
}

pub static UI_EVENTS: Channel<CriticalSectionRawMutex, AppEvent, 8> = Channel::new();

pub static POWER_COMMANDS: Channel<CriticalSectionRawMutex, PowerCommand, 8> = Channel::new();

/// Startup barrier events for clients sharing `PineTime`'s single sensor bus.
#[cfg(feature = "diagnostics")]
pub static SENSOR_BUS_READY: Channel<CriticalSectionRawMutex, SensorBusClient, 2> = Channel::new();

/// Latest logical system state for display, motion, and future services.
pub static SYSTEM_POWER: Watch<CriticalSectionRawMutex, SystemPowerState, 3> = Watch::new();

pub type SystemPowerReceiver = Receiver<'static, CriticalSectionRawMutex, SystemPowerState, 3>;

/// Reserves one of the fixed display, motion, and heart-rate subscriptions.
pub fn system_power_receiver() -> SystemPowerReceiver {
    SYSTEM_POWER
        .receiver()
        .expect("system power receiver capacity is fixed by architecture")
}
