use embassy_nrf::gpio::{Input, Pull};
use pineforge_services::{ports::HeartRatePorts, runners::heart_rate::HeartRateRunner};

use crate::{
    board::{buses::HeartRateI2c, peripherals::HeartRateResources},
    ipc::{HEART_RATE_BPM, HEART_RATE_COMMANDS, MOTION_READY, UI_EVENTS},
};

/// Binds the `PineTime` resources and this firmware's bus to the
/// executor-independent heart-rate runner. The interrupt stays owned as an
/// input until interrupt-driven acquisition is implemented.
#[embassy_executor::task]
pub async fn run(resources: HeartRateResources, i2c: HeartRateI2c) {
    let _interrupt = Input::new(resources.interrupt, Pull::None);
    let ports = HeartRatePorts {
        events: UI_EVENTS.dyn_sender(),
        commands: HEART_RATE_COMMANDS.dyn_receiver(),
        bpm: HEART_RATE_BPM.dyn_sender(),
        motion_ready: &MOTION_READY,
    };
    HeartRateRunner::new(i2c, ports).run().await;
}
