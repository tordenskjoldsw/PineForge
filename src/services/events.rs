#[cfg(feature = "diagnostics")]
use embassy_sync::signal::Signal;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};

use pineforge_state::AppEvent;
#[cfg(feature = "diagnostics")]
use pineforge_state::SystemPowerState;

pub static UI_EVENTS: Channel<CriticalSectionRawMutex, AppEvent, 8> = Channel::new();

#[cfg(feature = "diagnostics")]
pub static SYSTEM_POWER: Signal<CriticalSectionRawMutex, SystemPowerState> = Signal::new();
