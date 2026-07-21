use defmt::{info, warn};
use embassy_futures::select::{Either, select};
use embassy_nrf::gpio::{Input, Pull};
use pineforge_state::{AccelerometerKind, AppEvent, SystemPowerState};

use crate::{
    board::buses::SensorI2c,
    board::peripherals::AccelerometerResources,
    drivers::bma42x::{AccelerationPowerMode, Bma42x},
    services::events::{SYSTEM_POWER, UI_EVENTS},
};

const ACTIVE_UI_DIVISOR: u8 = 5;
const IDLE_UI_DIVISOR: u8 = 12;

/// Identifies the shared-bus accelerometer and streams diagnostic raw samples.
#[embassy_executor::task]
pub async fn run(resources: AccelerometerResources, i2c: SensorI2c) {
    let mut accelerometer = Bma42x::new(i2c);
    let mut interrupt = Input::new(resources.interrupt, Pull::Down);
    let result = accelerometer.probe().await.map_or_else(
        |_| {
            warn!("Accelerometer probe failed");
            AccelerometerKind::Unavailable
        },
        |kind| {
            match kind {
                AccelerometerKind::Bma421 => info!("BMA421 detected"),
                AccelerometerKind::Bma425 => info!("BMA425 detected"),
                AccelerometerKind::Unknown(chip_id) => {
                    warn!("Unknown accelerometer chip ID: {=u8:#x}", chip_id);
                }
                AccelerometerKind::Unavailable => {}
            }
            kind
        },
    );
    UI_EVENTS
        .send(AppEvent::AccelerometerDetected(result))
        .await;

    if !matches!(
        result,
        AccelerometerKind::Bma421 | AccelerometerKind::Bma425
    ) {
        return;
    }
    let mut power = SystemPowerState::Interactive;
    if accelerometer
        .set_power_mode(acceleration_mode(power))
        .await
        .is_err()
    {
        warn!("Accelerometer configuration failed");
        return;
    }
    if accelerometer.enable_data_ready_interrupt().await.is_err() {
        warn!("Accelerometer interrupt configuration failed");
        return;
    }

    let mut samples_until_update = 1;
    loop {
        if power == SystemPowerState::Sleeping {
            power = SYSTEM_POWER.wait().await;
            apply_power_mode(&mut accelerometer, power).await;
            samples_until_update = 1;
            continue;
        }

        match select(SYSTEM_POWER.wait(), interrupt.wait_for_rising_edge()).await {
            Either::First(next) => {
                power = next;
                apply_power_mode(&mut accelerometer, power).await;
                samples_until_update = 1;
            }
            Either::Second(()) => {
                match accelerometer.acknowledge_data_ready().await {
                    Ok(true) => {}
                    Ok(false) => continue,
                    Err(_) => {
                        warn!("Accelerometer interrupt acknowledgement failed");
                        continue;
                    }
                }
                samples_until_update -= 1;
                if samples_until_update == 0 {
                    if let Ok(sample) = accelerometer.read_acceleration().await {
                        UI_EVENTS.send(AppEvent::AccelerationUpdated(sample)).await;
                    } else {
                        warn!("Accelerometer sample failed");
                    }
                    samples_until_update = ui_divisor(power);
                }
            }
        }
    }
}

const fn ui_divisor(state: SystemPowerState) -> u8 {
    match state {
        SystemPowerState::Interactive => ACTIVE_UI_DIVISOR,
        SystemPowerState::Idle => IDLE_UI_DIVISOR,
        SystemPowerState::Sleeping => 1,
    }
}

const fn acceleration_mode(state: SystemPowerState) -> AccelerationPowerMode {
    match state {
        SystemPowerState::Interactive => AccelerationPowerMode::Active,
        SystemPowerState::Idle => AccelerationPowerMode::LowPower,
        SystemPowerState::Sleeping => AccelerationPowerMode::Off,
    }
}

async fn apply_power_mode(accelerometer: &mut Bma42x<SensorI2c>, state: SystemPowerState) {
    if accelerometer
        .set_power_mode(acceleration_mode(state))
        .await
        .is_err()
    {
        warn!("Accelerometer power transition failed");
    }
}
