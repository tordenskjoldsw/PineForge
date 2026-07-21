use defmt::{info, warn};
use embassy_time::{Duration, Timer};
use pineforge_state::{AccelerometerKind, AppEvent};

use crate::{board::buses::SensorI2c, drivers::bma42x::Bma42x, services::events::UI_EVENTS};

const UI_UPDATE_INTERVAL: Duration = Duration::from_millis(200);

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
    if accelerometer.enable_acceleration().await.is_err() {
        warn!("Accelerometer configuration failed");
        return;
    }

    loop {
        if let Ok(sample) = accelerometer.read_acceleration().await {
            UI_EVENTS.send(AppEvent::AccelerationUpdated(sample)).await;
        } else {
            warn!("Accelerometer sample failed");
        }
        Timer::after(UI_UPDATE_INTERVAL).await;
    }
}
