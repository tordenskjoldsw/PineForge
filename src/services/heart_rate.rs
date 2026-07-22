use defmt::{info, warn};
use embassy_futures::select::{Either, select};
use embassy_time::{Duration, Ticker, Timer};
use embedded_hal_async::i2c::I2c;
use pineforge_state::{
    AppEvent, HeartRateRawSample, HeartRateSensorKind, PpgAnalysis, PpgProcessor, SystemPowerState,
};

use crate::{
    drivers::hrs3300::{Hrs3300, Hrs3300Kind},
    services::events::{
        SENSOR_BUS_READY, SensorBusClient, SystemPowerReceiver, UI_EVENTS, system_power_receiver,
    },
};

const SAMPLE_INTERVAL: Duration = Duration::from_millis(100);
const SETTLING_DELAY: Duration = Duration::from_millis(100);
const UI_SAMPLE_DIVISOR: u8 = 10;

/// Owns the HRS3300 lifecycle and acquisition cadence independently from the
/// concrete Embassy task and from UI rendering.
pub struct HeartRateRunner<I2C> {
    sensor: Hrs3300<I2C>,
}

impl<I2C> HeartRateRunner<I2C>
where
    I2C: I2c,
{
    #[must_use]
    pub const fn new(i2c: I2C) -> Self {
        Self {
            sensor: Hrs3300::new(i2c),
        }
    }

    pub async fn run(mut self) {
        Self::wait_for_sensor_bus_clients().await;
        if !self.initialize().await {
            return;
        }

        let mut power_receiver = system_power_receiver();
        let mut power = power_receiver.get().await;
        loop {
            if power == SystemPowerState::Sleeping {
                power = power_receiver.changed().await;
                continue;
            }

            power = self.measure_until_sleep(&mut power_receiver).await;
        }
    }

    async fn wait_for_sensor_bus_clients() {
        let mut motion_ready = false;
        let mut touch_ready = false;
        while !motion_ready || !touch_ready {
            match SENSOR_BUS_READY.receive().await {
                SensorBusClient::Motion => motion_ready = true,
                SensorBusClient::Touch => touch_ready = true,
            }
        }
    }

    async fn initialize(&mut self) -> bool {
        let kind = match self.sensor.probe_and_disable().await {
            Ok(Hrs3300Kind::Hrs3300) => {
                info!("HRS3300 detected");
                HeartRateSensorKind::Hrs3300
            }
            Ok(Hrs3300Kind::Unknown(id)) => {
                warn!("Unknown heart-rate sensor ID: {=u8:#x}", id);
                HeartRateSensorKind::Unknown(id)
            }
            Err(_) => {
                warn!("Heart-rate sensor probe failed");
                HeartRateSensorKind::Unavailable
            }
        };
        UI_EVENTS
            .send(AppEvent::HeartRateSensorDetected(kind))
            .await;

        if kind != HeartRateSensorKind::Hrs3300 {
            return false;
        }
        Timer::after(SETTLING_DELAY).await;
        if self.sensor.configure().await.is_err() {
            warn!("Heart-rate sensor configuration failed");
            UI_EVENTS
                .send(AppEvent::HeartRateSensorDetected(
                    HeartRateSensorKind::Unavailable,
                ))
                .await;
            return false;
        }
        true
    }

    async fn measure_until_sleep(
        &mut self,
        power_receiver: &mut SystemPowerReceiver,
    ) -> SystemPowerState {
        if self.sensor.power_up().await.is_err() {
            warn!("Heart-rate sensor power-up failed");
            self.power_down().await;
            Timer::after(Duration::from_secs(1)).await;
            return power_receiver.get().await;
        }

        match select(power_receiver.changed(), Timer::after(SETTLING_DELAY)).await {
            Either::First(power) if power == SystemPowerState::Sleeping => {
                self.power_down().await;
                return power;
            }
            Either::First(_) | Either::Second(()) => {}
        }

        let mut samples_until_ui_update = 1;
        let mut sample_ticker = Ticker::every(SAMPLE_INTERVAL);
        let mut ppg = PpgProcessor::new();
        loop {
            match select(power_receiver.changed(), sample_ticker.next()).await {
                Either::First(power) if power == SystemPowerState::Sleeping => {
                    self.power_down().await;
                    return power;
                }
                Either::First(_) => {}
                Either::Second(()) => {
                    if let Ok(sample) = self.sensor.read_sample().await {
                        let analysis = ppg.push(sample.hrs, sample.als);
                        if !matches!(analysis, PpgAnalysis::Collecting { .. }) {
                            UI_EVENTS
                                .send(AppEvent::HeartRateAnalysisUpdated(analysis))
                                .await;
                        }
                        samples_until_ui_update -= 1;
                        if samples_until_ui_update == 0 {
                            UI_EVENTS
                                .send(AppEvent::HeartRateRawSampleUpdated(HeartRateRawSample {
                                    hrs: sample.hrs,
                                    als: sample.als,
                                }))
                                .await;
                            samples_until_ui_update = UI_SAMPLE_DIVISOR;
                        }
                    } else {
                        warn!("Heart-rate sample failed");
                    }
                }
            }
        }
    }

    async fn power_down(&mut self) {
        if self.sensor.power_down().await.is_err() {
            warn!("Heart-rate sensor power-down failed");
        }
    }
}
