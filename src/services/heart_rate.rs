use defmt::{info, warn};
use embassy_futures::select::{Either, Either3, select, select3};
use embassy_time::{Duration, Ticker, Timer};
use embedded_hal_async::i2c::I2c;
use pineforge_state::{
    AppEvent, HeartRateCommand, HeartRateRawSample, HeartRateSensorKind, HeartRateSession,
    PpgAnalysis, PpgProcessor, SystemPowerState,
};

use crate::{
    drivers::hrs3300::{Hrs3300, Hrs3300Kind},
    services::events::{
        HEART_RATE_COMMANDS, MOTION_READY, SystemPowerReceiver, UI_EVENTS, system_power_receiver,
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
        MOTION_READY.wait().await;
        if !self.initialize().await {
            return;
        }

        let mut power_receiver = system_power_receiver();
        let mut session = HeartRateSession::new();
        UI_EVENTS
            .send(AppEvent::HeartRateStateUpdated(session.state()))
            .await;
        loop {
            if HEART_RATE_COMMANDS.receive().await == HeartRateCommand::Start
                && power_receiver.get().await != SystemPowerState::Sleeping
            {
                UI_EVENTS
                    .send(AppEvent::HeartRateStateUpdated(session.start()))
                    .await;
                self.measure_until_stop(&mut power_receiver, &mut session)
                    .await;
                self.power_down().await;
                UI_EVENTS
                    .send(AppEvent::HeartRateStateUpdated(session.stop()))
                    .await;
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

    async fn measure_until_stop(
        &mut self,
        power_receiver: &mut SystemPowerReceiver,
        session: &mut HeartRateSession,
    ) {
        if self.sensor.power_up().await.is_err() {
            warn!("Heart-rate sensor power-up failed");
            self.power_down().await;
            UI_EVENTS
                .send(AppEvent::HeartRateStateUpdated(session.fail()))
                .await;
            return;
        }

        match select(power_receiver.changed(), Timer::after(SETTLING_DELAY)).await {
            Either::First(SystemPowerState::Sleeping) => {
                self.power_down().await;
                return;
            }
            Either::First(_) | Either::Second(()) => {}
        }

        let mut samples_until_ui_update = 1;
        let mut sample_ticker = Ticker::every(SAMPLE_INTERVAL);
        let mut ppg = PpgProcessor::new();
        UI_EVENTS
            .send(AppEvent::HeartRateStateUpdated(session.collecting()))
            .await;
        loop {
            match select3(
                HEART_RATE_COMMANDS.receive(),
                power_receiver.changed(),
                sample_ticker.next(),
            )
            .await
            {
                Either3::First(HeartRateCommand::Stop)
                | Either3::Second(SystemPowerState::Sleeping) => return,
                Either3::First(HeartRateCommand::Start) | Either3::Second(_) => {}
                Either3::Third(()) => {
                    if let Ok(sample) = self.sensor.read_sample().await {
                        let analysis = ppg.push(sample.hrs, sample.als);
                        if !matches!(analysis, PpgAnalysis::Collecting { .. }) {
                            UI_EVENTS
                                .send(AppEvent::HeartRateStateUpdated(session.apply(analysis)))
                                .await;
                        }
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
