use defmt::{info, warn};
use pineforge_state::{AccelerometerKind, AppEvent};

use crate::{board::buses::SensorI2c, drivers::bma42x::Bma42x, services::events::UI_EVENTS};

/// Probes the shared-bus accelerometer without changing its configuration.
#[embassy_executor::task]
pub async fn probe(i2c: SensorI2c) {
    let result = Bma42x::new(i2c).probe().await.map_or_else(
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
}
