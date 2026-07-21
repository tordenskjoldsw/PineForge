use defmt::{info, warn};
use embassy_futures::select::{Either, select};
use embassy_time::{Duration, Timer};
use embedded_hal_async::{digital::Wait, i2c::I2c};
use pineforge_state::{AccelerometerKind, AppEvent, FeatureEngineStatus, SystemPowerState};

use crate::{
    drivers::bma42x::{AccelerationPowerMode, Bma42x, FeatureEngineError},
    services::events::{SYSTEM_POWER, UI_EVENTS},
};

const ACTIVE_UI_DIVISOR: u8 = 20;
const ACTIVE_STEP_DIVISOR: u8 = 100;
const IDLE_UPDATE_INTERVAL: Duration = Duration::from_secs(1);

/// Owns the motion sensor lifecycle independently from the executor task.
pub struct AccelerometerRunner<I2C, IRQ> {
    accelerometer: Bma42x<I2C>,
    interrupt: IRQ,
}

impl<I2C, IRQ> AccelerometerRunner<I2C, IRQ>
where
    I2C: I2c,
    IRQ: Wait,
{
    #[must_use]
    pub const fn new(i2c: I2C, interrupt: IRQ) -> Self {
        Self {
            accelerometer: Bma42x::new(i2c),
            interrupt,
        }
    }

    pub async fn run(mut self) {
        let Some(()) = self.initialize().await else {
            return;
        };

        let mut power = SystemPowerState::Interactive;
        if self.apply_power_mode(power).await.is_err() {
            return;
        }

        let mut samples_until_update = 1;
        let mut samples_until_step_update = 1;
        loop {
            match power {
                SystemPowerState::Interactive => {
                    match select(SYSTEM_POWER.wait(), self.interrupt.wait_for_rising_edge()).await {
                        Either::First(next) => {
                            power = next;
                            let _ = self.apply_power_mode(power).await;
                            samples_until_update = 1;
                            samples_until_step_update = 1;
                        }
                        Either::Second(result) => {
                            if result.is_err() {
                                warn!("Accelerometer interrupt wait failed");
                                continue;
                            }
                            match self.accelerometer.acknowledge_data_ready().await {
                                Ok(true) => {}
                                Ok(false) => continue,
                                Err(_) => {
                                    warn!("Accelerometer interrupt acknowledgement failed");
                                    continue;
                                }
                            }
                            samples_until_update -= 1;
                            if samples_until_update == 0 {
                                self.publish_acceleration().await;
                                samples_until_update = ACTIVE_UI_DIVISOR;
                            }
                            samples_until_step_update -= 1;
                            if samples_until_step_update == 0 {
                                self.publish_step_count().await;
                                samples_until_step_update = ACTIVE_STEP_DIVISOR;
                            }
                        }
                    }
                }
                SystemPowerState::Idle => {
                    match select(SYSTEM_POWER.wait(), Timer::after(IDLE_UPDATE_INTERVAL)).await {
                        Either::First(next) => {
                            power = next;
                            let _ = self.apply_power_mode(power).await;
                            samples_until_update = 1;
                            samples_until_step_update = 1;
                        }
                        Either::Second(()) => {
                            self.publish_acceleration().await;
                            self.publish_step_count().await;
                        }
                    }
                }
                SystemPowerState::Sleeping => {
                    power = SYSTEM_POWER.wait().await;
                    let _ = self.apply_power_mode(power).await;
                    samples_until_update = 1;
                    samples_until_step_update = 1;
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
            UI_EVENTS
                .send(AppEvent::FeatureEngineUpdated(FeatureEngineStatus::Failed))
                .await;
            return None;
        }
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
            .set_data_ready_interrupt(state == SystemPowerState::Interactive)
            .await
            .is_err()
        {
            warn!("Accelerometer interrupt routing failed");
            return Err(());
        }

        // A power signal can cancel the GPIO future while INT1 is asserted.
        // Clear that pending status before arming the next rising-edge wait.
        if self.accelerometer.acknowledge_data_ready().await.is_err() {
            warn!("Accelerometer interrupt rearm failed");
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
