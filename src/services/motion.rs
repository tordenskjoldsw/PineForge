use defmt::{info, warn};
use embassy_futures::select::{Either, select};
use embassy_time::{Duration, Timer};
use embedded_hal_async::i2c::I2c;
#[cfg(feature = "diagnostics")]
use pineforge_state::FeatureEngineStatus;
use pineforge_state::{AccelerometerKind, AppEvent, SystemPowerState};

use crate::{
    drivers::bma42x::{AccelerationPowerMode, Bma42x, FeatureEngineError},
    services::events::{SYSTEM_POWER, UI_EVENTS},
};

const ACTIVE_UPDATE_INTERVAL: Duration = Duration::from_millis(100);
const ACTIVE_STEP_DIVISOR: u8 = 10;
const IDLE_UPDATE_INTERVAL: Duration = Duration::from_secs(1);

/// Owns the motion sensor lifecycle independently from the executor task.
pub struct AccelerometerRunner<I2C> {
    accelerometer: Bma42x<I2C>,
}

impl<I2C> AccelerometerRunner<I2C>
where
    I2C: I2c,
{
    #[must_use]
    pub const fn new(i2c: I2C) -> Self {
        Self {
            accelerometer: Bma42x::new(i2c),
        }
    }

    pub async fn run(mut self) {
        let Some(()) = self.initialize().await else {
            return;
        };

        let power = SystemPowerState::Interactive;
        if self.apply_power_mode(power).await.is_err() {
            return;
        }

        self.run_motion(power).await;
    }

    async fn run_motion(&mut self, mut power: SystemPowerState) -> ! {
        let mut ticks_until_step_update = 1;
        loop {
            match power {
                SystemPowerState::Interactive => {
                    match select(SYSTEM_POWER.wait(), Timer::after(ACTIVE_UPDATE_INTERVAL)).await {
                        Either::First(next) => {
                            power = next;
                            let _ = self.apply_power_mode(power).await;
                            ticks_until_step_update = 1;
                        }
                        Either::Second(()) => {
                            #[cfg(feature = "diagnostics")]
                            self.publish_acceleration().await;
                            ticks_until_step_update -= 1;
                            if ticks_until_step_update == 0 {
                                self.publish_step_count().await;
                                ticks_until_step_update = ACTIVE_STEP_DIVISOR;
                            }
                        }
                    }
                }
                SystemPowerState::Idle => {
                    match select(SYSTEM_POWER.wait(), Timer::after(IDLE_UPDATE_INTERVAL)).await {
                        Either::First(next) => {
                            power = next;
                            let _ = self.apply_power_mode(power).await;
                            ticks_until_step_update = 1;
                        }
                        Either::Second(()) => {
                            #[cfg(feature = "diagnostics")]
                            self.publish_acceleration().await;
                            self.publish_step_count().await;
                        }
                    }
                }
                SystemPowerState::Sleeping => {
                    power = SYSTEM_POWER.wait().await;
                    let _ = self.apply_power_mode(power).await;
                    ticks_until_step_update = 1;
                    self.publish_step_count().await;
                }
            }
        }
    }

    async fn initialize(&mut self) -> Option<()> {
        if self.accelerometer.reset().await.is_err() {
            warn!("Accelerometer reset failed");
        }
        let result = self.accelerometer.probe().await.map_or_else(
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
        #[cfg(feature = "diagnostics")]
        UI_EVENTS
            .send(AppEvent::AccelerometerDetected(result))
            .await;

        if !matches!(
            result,
            AccelerometerKind::Bma421 | AccelerometerKind::Bma425
        ) {
            return None;
        }

        if let Err(error) = self.accelerometer.initialize_feature_engine(result).await {
            match error {
                FeatureEngineError::InitializationFailed(status) => {
                    warn!("Accelerometer feature-engine status: {=u8:#x}", status);
                }
                FeatureEngineError::Bus(_)
                | FeatureEngineError::UnsupportedSensor
                | FeatureEngineError::NotInitialized => {
                    warn!("Accelerometer feature-engine initialization failed");
                }
            }
            #[cfg(feature = "diagnostics")]
            UI_EVENTS
                .send(AppEvent::FeatureEngineUpdated(FeatureEngineStatus::Failed))
                .await;
            return None;
        }
        #[cfg(feature = "diagnostics")]
        UI_EVENTS
            .send(AppEvent::FeatureEngineUpdated(FeatureEngineStatus::Ready))
            .await;
        if self.accelerometer.enable_step_counter().await.is_err() {
            warn!("Accelerometer step-counter enable failed");
            return None;
        }
        self.publish_step_count().await;
        Some(())
    }

    async fn apply_power_mode(&mut self, state: SystemPowerState) -> Result<(), ()> {
        if self
            .accelerometer
            .set_power_mode(acceleration_mode(state))
            .await
            .is_err()
        {
            warn!("Accelerometer power transition failed");
            return Err(());
        }

        if self
            .accelerometer
            .set_data_ready_interrupt(false)
            .await
            .is_err()
        {
            warn!("Accelerometer interrupt routing failed");
            return Err(());
        }

        Ok(())
    }

    async fn publish_step_count(&mut self) {
        if let Ok(steps) = self.accelerometer.read_step_count().await {
            UI_EVENTS.send(AppEvent::StepsUpdated(steps)).await;
        } else {
            warn!("Accelerometer step-counter read failed");
        }
    }

    #[cfg(feature = "diagnostics")]
    async fn publish_acceleration(&mut self) {
        if let Ok(sample) = self.accelerometer.read_acceleration().await {
            UI_EVENTS.send(AppEvent::AccelerationUpdated(sample)).await;
        } else {
            warn!("Accelerometer sample failed");
        }
    }
}

const fn acceleration_mode(state: SystemPowerState) -> AccelerationPowerMode {
    match state {
        SystemPowerState::Interactive => AccelerationPowerMode::Active,
        SystemPowerState::Idle | SystemPowerState::Sleeping => AccelerationPowerMode::LowPower,
    }
}
