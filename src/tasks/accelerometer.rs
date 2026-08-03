use pineforge_services::{ports::MotionPorts, runners::motion::AccelerometerRunner};

use crate::{
    board::buses::{MotionI2c, SensorBus},
    ipc::{
        DISPLAY_SETTINGS, MOTION_READY, POWER_COMMANDS, STEP_COUNT, STEP_RESET, TOUCH_READY,
        UI_EVENTS, motion_power,
    },
};

/// Binds concrete `PineTime` peripherals and this firmware's bus to the
/// executor-independent runner.
///
/// The endpoints are handed over here rather than reached for inside the
/// runner. That is what lets the runner live in a crate the host can build:
/// which channel a reading goes on is a fact about this firmware, and the
/// cadence that produces the reading is not.
#[embassy_executor::task]
pub async fn run(i2c: MotionI2c, bus: &'static SensorBus) {
    let ports = MotionPorts {
        events: UI_EVENTS.dyn_sender(),
        power_commands: POWER_COMMANDS.dyn_sender(),
        power: motion_power(),
        settings: DISPLAY_SETTINGS.dyn_anon_receiver(),
        steps: STEP_COUNT.dyn_sender(),
        reset_steps: &STEP_RESET,
        touch_ready: &TOUCH_READY,
        motion_ready: &MOTION_READY,
    };
    AccelerometerRunner::new(i2c, bus, ports).run().await;
}
