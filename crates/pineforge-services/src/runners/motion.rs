use embassy_futures::select::{Either, select};
use embassy_sync::watch::DynAnonReceiver;
use embassy_time::{Duration, Timer};
use embedded_hal_async::i2c::I2c;
#[cfg(feature = "diagnostics")]
use pineforge_state::FeatureEngineStatus;
use pineforge_state::{
    AccelerationSample, AccelerometerKind, AppEvent, DisplaySettings, PowerCommand,
    RaiseToWakeDetector, SystemPowerState, WakeGesture,
};

use crate::{
    drivers::bma42x::{AccelerationPowerMode, Bma42x, FeatureEngineError},
    log::{log_info, log_warn},
    ports::MotionPorts,
};

const ACTIVE_UPDATE_INTERVAL: Duration = Duration::from_millis(100);
const ACTIVE_STEP_DIVISOR: u8 = 10;
const IDLE_UPDATE_INTERVAL: Duration = Duration::from_secs(1);

/// Recovers the board-owned shared bus after the `BMA4xx` soft reset.
pub trait BusRecovery {
    fn recover(&self);
}

/// Owns the motion sensor lifecycle independently from the executor task.
pub struct AccelerometerRunner<'a, I2C, RECOVERY> {
    accelerometer: Bma42x<I2C>,
    bus_recovery: RECOVERY,
    ports: MotionPorts<'a>,
}

impl<'a, I2C, RECOVERY> AccelerometerRunner<'a, I2C, RECOVERY>
where
    I2C: I2c,
    RECOVERY: BusRecovery,
{
    #[must_use]
    pub const fn new(i2c: I2C, bus_recovery: RECOVERY, ports: MotionPorts<'a>) -> Self {
        Self {
            accelerometer: Bma42x::new(i2c),
            bus_recovery,
            ports,
        }
    }

    pub async fn run(mut self) {
        self.ports.touch_ready.wait().await;
        let initialized = self.initialize().await;
        self.ports.motion_ready.signal(());
        let Some(()) = initialized else {
            return;
        };

        let power = self.ports.power.get().await;
        if self.apply_power_mode(power).await.is_err() {
            return;
        }

        self.run_motion(power).await;
    }

    async fn run_motion(&mut self, mut power: SystemPowerState) -> ! {
        // Lifted out of `self` so the wait below borrows nothing the body it
        // races needs. The signal is a shared reference, so this is a copy.
        let reset = self.ports.reset_steps;
        let mut ticks_until_step_update = 1;
        let mut raise_to_wake = RaiseToWakeDetector::new();
        loop {
            // The day boundary outranks whatever this state was waiting for.
            // Serving it late means a day's steps folded into the next day's
            // total, and on a sleeping watch "late" is however long it is until
            // something else happens - which can be hours.
            //
            // Losing the body's future to the race costs nothing: every step it
            // could have been part-way through is a sensor read whose value the
            // next one carries anyway, and the reset publishes a count of its
            // own the moment it lands.
            if matches!(
                select(
                    reset.wait(),
                    self.step(&mut power, &mut ticks_until_step_update, &mut raise_to_wake),
                )
                .await,
                Either::First(())
            ) {
                self.reset_step_count().await;
            }
        }
    }

    /// One pass of whatever the current power state waits on.
    ///
    /// Split from the loop above so the reset can be raced against it as a
    /// whole, rather than added as a third arm to each of the three selects
    /// below - which would have been the same signal written out three times
    /// and forgotten from one of them.
    async fn step(
        &mut self,
        power: &mut SystemPowerState,
        ticks_until_step_update: &mut u8,
        raise_to_wake: &mut RaiseToWakeDetector,
    ) {
        match *power {
            SystemPowerState::Interactive => {
                match select(
                    self.ports.power.changed(),
                    Timer::after(ACTIVE_UPDATE_INTERVAL),
                )
                .await
                {
                    Either::First(next) => {
                        *power = next;
                        let _ = self.apply_power_mode(*power).await;
                        *ticks_until_step_update = 1;
                    }
                    Either::Second(()) => {
                        if raise_to_wake_enabled(&mut self.ports.settings) {
                            self.update_raise_to_wake(raise_to_wake, false).await;
                        } else {
                            raise_to_wake.reset();
                        }
                        #[cfg(feature = "diagnostics")]
                        self.publish_acceleration().await;
                        *ticks_until_step_update -= 1;
                        if *ticks_until_step_update == 0 {
                            self.publish_step_count().await;
                            *ticks_until_step_update = ACTIVE_STEP_DIVISOR;
                        }
                    }
                }
            }
            SystemPowerState::Idle => {
                let track_raise = raise_to_wake_enabled(&mut self.ports.settings);
                let update_interval = if track_raise {
                    ACTIVE_UPDATE_INTERVAL
                } else {
                    IDLE_UPDATE_INTERVAL
                };
                match select(self.ports.power.changed(), Timer::after(update_interval)).await {
                    Either::First(next) => {
                        *power = next;
                        let _ = self.apply_power_mode(*power).await;
                        *ticks_until_step_update = 1;
                    }
                    Either::Second(()) => {
                        if track_raise {
                            self.update_raise_to_wake(raise_to_wake, false).await;
                        } else {
                            raise_to_wake.reset();
                        }
                        #[cfg(feature = "diagnostics")]
                        self.publish_acceleration().await;
                        if track_raise {
                            *ticks_until_step_update -= 1;
                            if *ticks_until_step_update == 0 {
                                self.publish_step_count().await;
                                *ticks_until_step_update = ACTIVE_STEP_DIVISOR;
                            }
                        } else {
                            self.publish_step_count().await;
                        }
                    }
                }
            }
            SystemPowerState::Sleeping => {
                let wake_gestures = self
                    .ports
                    .settings
                    .try_get()
                    .unwrap_or(DisplaySettings::DEFAULT)
                    .wake_gestures();
                if wake_gestures.contains(WakeGesture::RaiseWrist) {
                    match select(
                        self.ports.power.changed(),
                        Timer::after(ACTIVE_UPDATE_INTERVAL),
                    )
                    .await
                    {
                        Either::First(next) => *power = next,
                        Either::Second(()) => {
                            self.update_raise_to_wake(raise_to_wake, true).await;
                            return;
                        }
                    }
                } else {
                    raise_to_wake.reset();
                    *power = self.ports.power.changed().await;
                }
                let _ = self.apply_power_mode(*power).await;
                *ticks_until_step_update = 1;
                self.publish_step_count().await;
            }
        }
    }

    async fn initialize(&mut self) -> Option<()> {
        if self.accelerometer.reset().await.is_err() {
            log_warn!("Accelerometer reset failed");
        }
        self.bus_recovery.recover();
        let result = self.accelerometer.probe().await.map_or_else(
            |_| {
                log_warn!("Accelerometer probe failed");
                AccelerometerKind::Unavailable
            },
            |kind| {
                match kind {
                    AccelerometerKind::Bma421 => log_info!("BMA421 detected"),
                    AccelerometerKind::Bma425 => log_info!("BMA425 detected"),
                    AccelerometerKind::Unknown(chip_id) => {
                        log_warn!("Unknown accelerometer chip ID: {=u8:#x}", chip_id);
                    }
                    AccelerometerKind::Unavailable => {}
                }
                kind
            },
        );
        self.ports
            .events
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
                    log_warn!("Accelerometer feature-engine status: {=u8:#x}", status);
                }
                FeatureEngineError::Bus(_)
                | FeatureEngineError::UnsupportedSensor
                | FeatureEngineError::NotInitialized => {
                    log_warn!("Accelerometer feature-engine initialization failed");
                }
            }
            #[cfg(feature = "diagnostics")]
            self.ports
                .events
                .send(AppEvent::FeatureEngineUpdated(FeatureEngineStatus::Failed))
                .await;
            return None;
        }
        #[cfg(feature = "diagnostics")]
        self.ports
            .events
            .send(AppEvent::FeatureEngineUpdated(FeatureEngineStatus::Ready))
            .await;
        if self.accelerometer.enable_step_counter().await.is_err() {
            log_warn!("Accelerometer step-counter enable failed");
            return None;
        }
        self.publish_step_count().await;
        Some(())
    }

    async fn update_raise_to_wake(&mut self, detector: &mut RaiseToWakeDetector, wake: bool) {
        if let Ok(sample) = self.accelerometer.read_acceleration().await {
            let raised = detector.push(pinetime_axes(sample));
            if wake && raised {
                let _ = self
                    .ports
                    .power_commands
                    .try_send(PowerCommand::UserActivity);
            }
        }
    }

    async fn apply_power_mode(&mut self, state: SystemPowerState) -> Result<(), ()> {
        if self
            .accelerometer
            .set_power_mode(acceleration_mode(state))
            .await
            .is_err()
        {
            log_warn!("Accelerometer power transition failed");
            return Err(());
        }

        if self
            .accelerometer
            .set_data_ready_interrupt(false)
            .await
            .is_err()
        {
            log_warn!("Accelerometer interrupt routing failed");
            return Err(());
        }

        Ok(())
    }

    async fn publish_step_count(&mut self) {
        if let Ok(steps) = self.accelerometer.read_step_count().await {
            // Both, and in this order. The screens take it as an event because
            // a repaint is owed; anything that only wants the number - the
            // phone reading the motion service - takes the latest value
            // instead, and must not be able to block the sensor loop waiting
            // for a reader that is asleep.
            self.ports.steps.send(steps);
            self.ports.events.send(AppEvent::StepsUpdated(steps)).await;
        } else {
            log_warn!("Accelerometer step-counter read failed");
        }
    }

    /// Puts the counter back to zero when the day it was counting has ended.
    ///
    /// Published immediately afterwards rather than left until the next
    /// ordinary read: the zero is the whole point of the reset. A companion
    /// keeping a daily total watches for exactly that value to know the
    /// previous day's count is finished, and on a sleeping watch the next read
    /// may be hours away.
    async fn reset_step_count(&mut self) {
        if self.accelerometer.reset_step_counter().await.is_err() {
            log_warn!("Accelerometer step-counter reset failed");
            return;
        }
        log_info!("Step counter reset for the new day");
        self.publish_step_count().await;
    }

    #[cfg(feature = "diagnostics")]
    async fn publish_acceleration(&mut self) {
        if let Ok(sample) = self.accelerometer.read_acceleration().await {
            self.ports
                .events
                .send(AppEvent::AccelerationUpdated(sample))
                .await;
        } else {
            log_warn!("Accelerometer sample failed");
        }
    }
}

/// Whether the persisted wake gestures include the tilt.
///
/// Read through an anonymous receiver rather than a subscription: the answer is
/// only ever wanted at the moment it is asked, and taking one of the settings
/// `Watch`'s fixed subscriber slots to get it would cost a slot the display,
/// power and BLE tasks are counted into.
fn raise_to_wake_enabled(settings: &mut DynAnonReceiver<'_, DisplaySettings>) -> bool {
    settings
        .try_get()
        .unwrap_or(DisplaySettings::DEFAULT)
        .wake_gestures()
        .contains(WakeGesture::RaiseWrist)
}

/// The `BMA42x` is mounted with X and Y exchanged relative to the watch body.
const fn pinetime_axes(sample: AccelerationSample) -> AccelerationSample {
    AccelerationSample {
        x: sample.y,
        y: sample.x,
        z: sample.z,
    }
}

const fn acceleration_mode(state: SystemPowerState) -> AccelerationPowerMode {
    match state {
        SystemPowerState::Interactive => AccelerationPowerMode::Active,
        SystemPowerState::Idle | SystemPowerState::Sleeping => AccelerationPowerMode::LowPower,
    }
}
