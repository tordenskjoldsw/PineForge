use defmt::{info, warn};
use embassy_futures::select::{Either, select};
use embassy_time::{Duration, Ticker, Timer};
use embedded_hal_async::i2c::I2c;
#[cfg(feature = "diagnostics")]
use pineforge_state::HeartRateRawSample;
use pineforge_state::{
    AppEvent, HeartRateCommand, HeartRateSensorKind, HeartRateSession, PpgAnalysis, PpgProcessor,
};

use crate::{
    drivers::hrs3300::{Hrs3300, Hrs3300Kind},
    ipc::{HEART_RATE_COMMANDS, MOTION_READY, UI_EVENTS},
};

const SAMPLE_INTERVAL: Duration = Duration::from_millis(100);
/// The shortest gap before retrying a failed continuous measurement.
///
/// A working continuous measurement does not leave the acquisition loop: every
/// newly validated BPM value is published while the sensor stays powered. This
/// delay only bounds the retry loop when power-up fails immediately.
const CONTINUOUS_GAP: Duration = Duration::from_secs(1);

/// How long to wait before the next background reading.
///
/// Zero is the continuous preset and reaches this wait only after a failed
/// acquisition; a working continuous acquisition does not end between values.
const fn gap(interval_seconds: u32) -> Duration {
    if interval_seconds == 0 {
        CONTINUOUS_GAP
    } else {
        Duration::from_secs(interval_seconds as u64)
    }
}
const SETTLING_DELAY: Duration = Duration::from_millis(100);
#[cfg(feature = "diagnostics")]
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

    /// Applies one command, leaving the loop's state where the command says.
    ///
    /// Four places take a command - the idle wait, the sleep wait, the one that
    /// interrupted a reading, and the interval wait - and all four mean the
    /// same thing by it. Written once so a fifth cannot mean something subtly
    /// different, and because saying it four times was already what pushed this
    /// loop past being readable.
    async fn apply(
        command: HeartRateCommand,
        enabled: &mut bool,
        oneshot: &mut bool,
        interval_seconds: &mut u32,
        session: &mut HeartRateSession,
    ) {
        match command {
            HeartRateCommand::Configure {
                enabled: next,
                interval_seconds: interval,
            } => {
                *enabled = next;
                *interval_seconds = interval;
                if !*enabled {
                    UI_EVENTS
                        .send(AppEvent::HeartRateStateUpdated(session.stop()))
                        .await;
                }
            }
            HeartRateCommand::MeasureNow => *oneshot = true,
            // The reading in flight, if any, has already been abandoned by this
            // command arriving; all that is left is to stop asking for another
            // and to say so. The periodic setting is deliberately untouched.
            HeartRateCommand::Stop => {
                *oneshot = false;
                UI_EVENTS
                    .send(AppEvent::HeartRateStateUpdated(session.stop()))
                    .await;
            }
        }
    }

    pub async fn run(mut self) {
        MOTION_READY.wait().await;
        let available = self.initialize().await;

        let mut session = HeartRateSession::new();
        UI_EVENTS
            .send(AppEvent::HeartRateStateUpdated(session.state()))
            .await;
        let mut enabled = false;
        // Set by a one-shot request and cleared once it has been served. It is
        // what lets the pulse app take a reading without disturbing the
        // periodic setting either side of it.
        let mut oneshot = false;
        let mut interval_seconds = 300;
        loop {
            if !enabled && !oneshot {
                let command = HEART_RATE_COMMANDS.receive().await;
                Self::apply(
                    command,
                    &mut enabled,
                    &mut oneshot,
                    &mut interval_seconds,
                    &mut session,
                )
                .await;
                // Whichever way it was asked for, a sensor that never answered
                // at boot cannot produce a reading.
                if (enabled || oneshot) && !available {
                    UI_EVENTS
                        .send(AppEvent::HeartRateStateUpdated(session.fail()))
                        .await;
                    enabled = false;
                    oneshot = false;
                }
                continue;
            }
            UI_EVENTS
                .send(AppEvent::HeartRateStateUpdated(session.start()))
                .await;
            // A one-shot remains finite even when the stored interval happens
            // to be zero while background measurement is disabled. Continuous
            // acquisition is specifically the enabled zero-interval preset.
            let continuous = enabled && interval_seconds == 0;
            let command = self.measure(&mut session, continuous).await;
            self.power_down().await;
            // Served, whatever it produced. A reading that found no signal is
            // still an answer to the question that was asked.
            oneshot = false;
            if let Some(command) = command {
                // A second request mid-reading sets the one-shot again, so the
                // loop starts over rather than handing back a number gathered
                // before the request.
                Self::apply(
                    command,
                    &mut enabled,
                    &mut oneshot,
                    &mut interval_seconds,
                    &mut session,
                )
                .await;
                continue;
            }
            if !enabled {
                continue;
            }
            if let Either::First(command) = select(
                HEART_RATE_COMMANDS.receive(),
                Timer::after(gap(interval_seconds)),
            )
            .await
            {
                Self::apply(
                    command,
                    &mut enabled,
                    &mut oneshot,
                    &mut interval_seconds,
                    &mut session,
                )
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

    async fn measure(
        &mut self,
        session: &mut HeartRateSession,
        continuous: bool,
    ) -> Option<HeartRateCommand> {
        if self.sensor.power_up().await.is_err() {
            warn!("Heart-rate sensor power-up failed");
            self.power_down().await;
            UI_EVENTS
                .send(AppEvent::HeartRateStateUpdated(session.fail()))
                .await;
            return None;
        }

        Timer::after(SETTLING_DELAY).await;

        #[cfg(feature = "diagnostics")]
        let mut samples_until_ui_update = 1;
        let mut sample_ticker = Ticker::every(SAMPLE_INTERVAL);
        let mut ppg = PpgProcessor::new();
        let mut sample_count = 0_u16;
        UI_EVENTS
            .send(AppEvent::HeartRateStateUpdated(session.collecting()))
            .await;
        loop {
            match select(HEART_RATE_COMMANDS.receive(), sample_ticker.next()).await {
                Either::First(command) => return Some(command),
                Either::Second(()) => {
                    if let Ok(sample) = self.sensor.read_sample().await {
                        sample_count = sample_count.saturating_add(1);
                        let analysis = ppg.push(sample.hrs, sample.als);
                        if !matches!(analysis, PpgAnalysis::Collecting { .. }) {
                            UI_EVENTS
                                .send(AppEvent::HeartRateStateUpdated(session.apply(analysis)))
                                .await;
                        }
                        // Periodic and on-demand sessions answer one question:
                        // stop at the first valid value, or give up after
                        // twenty seconds. `CONT` instead behaves like
                        // InfiniTime's foreground acquisition: keep the sensor
                        // and processor running so each later validated window
                        // can replace the BPM value on the watchface.
                        if !continuous
                            && (matches!(analysis, PpgAnalysis::HeartRate { .. })
                                || sample_count >= 200)
                        {
                            return None;
                        }
                        if !matches!(analysis, PpgAnalysis::Collecting { .. }) {
                            UI_EVENTS
                                .send(AppEvent::HeartRateAnalysisUpdated(analysis))
                                .await;
                        }
                        #[cfg(feature = "diagnostics")]
                        {
                            samples_until_ui_update -= 1;
                        }
                        #[cfg(feature = "diagnostics")]
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
