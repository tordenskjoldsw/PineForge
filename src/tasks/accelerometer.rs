use defmt::{info, warn};
use embassy_futures::select::{Either, select};
use embassy_time::{Duration, Timer};
use pineforge_state::{AccelerometerKind, AppEvent, SystemPowerState};

use crate::{
    board::buses::SensorI2c,
    drivers::bma42x::{AccelerationPowerMode, Bma42x},
    services::events::{SYSTEM_POWER, UI_EVENTS},
};

const ACTIVE_UPDATE_INTERVAL: Duration = Duration::from_millis(200);
const LOW_POWER_UPDATE_INTERVAL: Duration = Duration::from_secs(1);

/// Identifies the shared-bus accelerometer and streams diagnostic raw samples.
#[embassy_executor::task]
pub async fn run(i2c: SensorI2c) {
    let mut accelerometer = Bma42x::new(i2c);
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

    loop {
        if power == SystemPowerState::Sleeping {
            power = SYSTEM_POWER.wait().await;
            apply_power_mode(&mut accelerometer, power).await;
            continue;
        }

        let interval = if power == SystemPowerState::Interactive {
            ACTIVE_UPDATE_INTERVAL
        } else {
            LOW_POWER_UPDATE_INTERVAL
        };
        match select(SYSTEM_POWER.wait(), Timer::after(interval)).await {
            Either::First(next) => {
                power = next;
                apply_power_mode(&mut accelerometer, power).await;
            }
            Either::Second(()) => {
                if let Ok(sample) = accelerometer.read_acceleration().await {
                    UI_EVENTS.send(AppEvent::AccelerationUpdated(sample)).await;
                } else {
                    warn!("Accelerometer sample failed");
                }
            }
        }
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
