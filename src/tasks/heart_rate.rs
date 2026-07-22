use embassy_nrf::gpio::{Input, Pull};
use embassy_time::Timer;
use pineforge_state::{AppEvent, HeartRateSensorKind};

use crate::{
    board::{buses::HeartRateI2c, peripherals::HeartRateResources},
    drivers::hrs3300::{Hrs3300, Hrs3300Kind},
    services::events::UI_EVENTS,
};

/// Performs the diagnostics-only hardware probe. Keeping the interrupt pin as
/// an input establishes ownership without enabling sampling or wake behavior.
#[embassy_executor::task]
pub async fn run(resources: HeartRateResources, i2c: HeartRateI2c) {
    let _interrupt = Input::new(resources.interrupt, Pull::None);
    let mut sensor = Hrs3300::new(i2c);
    let kind = match sensor.probe_and_disable().await {
        Ok(Hrs3300Kind::Hrs3300) => HeartRateSensorKind::Hrs3300,
        Ok(Hrs3300Kind::Unknown(id)) => HeartRateSensorKind::Unknown(id),
        Err(_) => HeartRateSensorKind::Unavailable,
    };
    UI_EVENTS
        .send(AppEvent::HeartRateSensorDetected(kind))
        .await;

    if kind != HeartRateSensorKind::Hrs3300 {
        return;
    }

    Timer::after_millis(100).await;
    let sample = async {
        sensor.configure().await?;
        sensor.power_up().await?;
        Timer::after_millis(100).await;
        sensor.read_hrs().await
    }
    .await;
    let power_down = sensor.power_down().await;

    match (sample, power_down) {
        (Ok(raw), Ok(())) => {
            UI_EVENTS
                .send(AppEvent::HeartRateRawSampleUpdated(raw))
                .await;
        }
        _ => {
            UI_EVENTS
                .send(AppEvent::HeartRateSensorDetected(
                    HeartRateSensorKind::Unavailable,
                ))
                .await;
        }
    }
}
