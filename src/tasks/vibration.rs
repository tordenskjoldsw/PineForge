use embassy_futures::select::{Either, select};
use embassy_nrf::gpio::{Level, Output, OutputDrive};
use embassy_time::Timer;

use crate::{
    board::peripherals::VibrationResources,
    drivers::vibration::VibrationMotor,
    ipc::{VIBRATION_ALARM, VIBRATION_COMMANDS, VibrationAlarmSignal},
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
        let pattern = match select(VIBRATION_ALARM.wait(), VIBRATION_COMMANDS.receive()).await {
            Either::First(VibrationAlarmSignal::Start) => pineforge_state::VibrationPattern::Alarm,
            Either::First(VibrationAlarmSignal::Cancel) => continue,
            Either::Second(pattern) => pattern,
        };
        let (on_millis, pause_millis, pulses) = pattern.pulses();
        let cancellable = matches!(pattern, pineforge_state::VibrationPattern::Alarm);
        'pattern: for pulse in 0..pulses {
            motor.on();
            if cancellable {
                match select(Timer::after_millis(on_millis), VIBRATION_ALARM.wait()).await {
                    Either::First(()) | Either::Second(VibrationAlarmSignal::Start) => {}
                    Either::Second(VibrationAlarmSignal::Cancel) => {
                        motor.off();
                        break 'pattern;
                    }
                }
            } else {
                Timer::after_millis(on_millis).await;
            }
            motor.off();
            if pulse + 1 < pulses {
                if cancellable {
                    match select(Timer::after_millis(pause_millis), VIBRATION_ALARM.wait()).await {
                        Either::First(()) | Either::Second(VibrationAlarmSignal::Start) => {}
                        Either::Second(VibrationAlarmSignal::Cancel) => break 'pattern,
                    }
                } else {
                    Timer::after_millis(pause_millis).await;
                }
            }
        }
    }
}
