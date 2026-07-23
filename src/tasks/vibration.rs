use embassy_nrf::gpio::{Level, Output, OutputDrive};
use embassy_time::Timer;

use crate::{
    board::peripherals::VibrationResources, drivers::vibration::VibrationMotor,
    services::events::VIBRATION_COMMANDS,
};

/// Owns the vibration motor and plays requested haptic patterns.
#[embassy_executor::task]
pub async fn run(resources: VibrationResources) {
    // The gate is active low, so the motor must idle high from the start.
    let mut motor = VibrationMotor::new(Output::new(
        resources.motor,
        Level::High,
        OutputDrive::Standard,
    ));

    loop {
        let pattern = VIBRATION_COMMANDS.receive().await;
        let (on_millis, pause_millis, pulses) = pattern.pulses();
        for pulse in 0..pulses {
            motor.on();
            Timer::after_millis(on_millis).await;
            motor.off();
            if pulse + 1 < pulses {
                Timer::after_millis(pause_millis).await;
            }
        }
    }
}
