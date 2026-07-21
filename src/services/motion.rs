use defmt::{info, warn};
use embassy_futures::select::{Either, select};
use embedded_hal_async::{digital::Wait, i2c::I2c};
use pineforge_state::{AccelerometerKind, AppEvent, SystemPowerState};

use crate::{
    drivers::bma42x::{AccelerationPowerMode, Bma42x},
    services::events::{SYSTEM_POWER, UI_EVENTS},
};

const ACTIVE_UI_DIVISOR: u8 = 5;
const IDLE_UI_DIVISOR: u8 = 12;

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
            return;
        }

        let mut power = SystemPowerState::Interactive;
        if self
            .accelerometer
            .set_power_mode(acceleration_mode(power))
            .await
            .is_err()
        {
            warn!("Accelerometer configuration failed");
            return;
        }
        if self
            .accelerometer
            .enable_data_ready_interrupt()
            .await
            .is_err()
        {
            warn!("Accelerometer interrupt configuration failed");
            return;
        }

        let mut samples_until_update = 1;
        loop {
            if power == SystemPowerState::Sleeping {
                power = SYSTEM_POWER.wait().await;
                self.apply_power_mode(power).await;
                samples_until_update = 1;
                continue;
            }

            match select(SYSTEM_POWER.wait(), self.interrupt.wait_for_rising_edge()).await {
                Either::First(next) => {
                    power = next;
                    self.apply_power_mode(power).await;
                    samples_until_update = 1;
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
                        if let Ok(sample) = self.accelerometer.read_acceleration().await {
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

    async fn apply_power_mode(&mut self, state: SystemPowerState) {
        if self
            .accelerometer
            .set_power_mode(acceleration_mode(state))
            .await
            .is_err()
        {
            warn!("Accelerometer power transition failed");
            return;
        }

        // A power signal can cancel the GPIO future while INT1 is asserted.
        // Clear that pending status before arming the next rising-edge wait.
        if self.accelerometer.acknowledge_data_ready().await.is_err() {
            warn!("Accelerometer interrupt rearm failed");
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
