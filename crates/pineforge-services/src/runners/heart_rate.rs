use embassy_futures::select::{Either, select};
use embassy_sync::channel::DynamicSender;
use embassy_time::{Duration, Ticker, Timer};
use embedded_hal_async::i2c::I2c;
#[cfg(feature = "diagnostics")]
use pineforge_state::HeartRateRawSample;
use pineforge_state::{
    AppEvent, HeartRateCommand, HeartRateSensorKind, HeartRateSession, PpgAnalysis, PpgProcessor,
};

use crate::{
    drivers::hrs3300::{Hrs3300, Hrs3300Kind},
    log::{log_info, log_warn},
    ports::HeartRatePorts,
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
pub struct HeartRateRunner<'a, I2C> {
    sensor: Hrs3300<I2C>,
    ports: HeartRatePorts<'a>,
}

impl<'a, I2C> HeartRateRunner<'a, I2C>
where
    I2C: I2c,
{
    #[must_use]
    pub const fn new(i2c: I2C, ports: HeartRatePorts<'a>) -> Self {
        Self {
            sensor: Hrs3300::new(i2c),
            ports,
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
        events: &DynamicSender<'_, AppEvent>,
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
                    events
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
                events
                    .send(AppEvent::HeartRateStateUpdated(session.stop()))
                    .await;
            }
        }
    }

    pub async fn run(mut self) {
        self.ports.motion_ready.wait().await;
        let available = self.initialize().await;

        let mut session = HeartRateSession::new();
        self.ports
            .events
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
                let command = self.ports.commands.receive().await;
                Self::apply(
                    &self.ports.events,
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
                    self.ports
                        .events
                        .send(AppEvent::HeartRateStateUpdated(session.fail()))
                        .await;
                    enabled = false;
                    oneshot = false;
                }
                continue;
            }
            self.ports
                .events
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
                    &self.ports.events,
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
                self.ports.commands.receive(),
                Timer::after(gap(interval_seconds)),
            )
            .await
            {
                Self::apply(
                    &self.ports.events,
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
                log_info!("HRS3300 detected");
                HeartRateSensorKind::Hrs3300
            }
            Ok(Hrs3300Kind::Unknown(id)) => {
                log_warn!("Unknown heart-rate sensor ID: {=u8:#x}", id);
                HeartRateSensorKind::Unknown(id)
            }
            Err(_) => {
                log_warn!("Heart-rate sensor probe failed");
                HeartRateSensorKind::Unavailable
            }
        };
        self.ports
            .events
            .send(AppEvent::HeartRateSensorDetected(kind))
            .await;

        if kind != HeartRateSensorKind::Hrs3300 {
            return false;
        }
        Timer::after(SETTLING_DELAY).await;
        if self.sensor.configure().await.is_err() {
            log_warn!("Heart-rate sensor configuration failed");
            self.ports
                .events
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
            log_warn!("Heart-rate sensor power-up failed");
            self.power_down().await;
            self.ports
                .events
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
        self.ports
            .events
            .send(AppEvent::HeartRateStateUpdated(session.collecting()))
            .await;
        loop {
            match select(self.ports.commands.receive(), sample_ticker.next()).await {
                Either::First(command) => return Some(command),
                Either::Second(()) => {
                    if let Ok(sample) = self.sensor.read_sample().await {
                        sample_count = sample_count.saturating_add(1);
                        let analysis = ppg.push(sample.hrs, sample.als);
                        // A rate that survived validation is the only thing a
                        // phone is told. Ambient light, no signal and the
                        // collecting window are states to draw on the watch, not
                        // measurements to report - and the standard
                        // characteristic has no way to say "I am still looking".
                        if let PpgAnalysis::HeartRate { bpm } = analysis {
                            self.ports.bpm.send(u8::try_from(bpm).unwrap_or(u8::MAX));
                        }
                        if !matches!(analysis, PpgAnalysis::Collecting { .. }) {
                            self.ports
                                .events
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
                            self.ports
                                .events
                                .send(AppEvent::HeartRateAnalysisUpdated(analysis))
                                .await;
                        }
                        #[cfg(feature = "diagnostics")]
                        {
                            samples_until_ui_update -= 1;
                        }
                        #[cfg(feature = "diagnostics")]
                        if samples_until_ui_update == 0 {
                            self.ports
                                .events
                                .send(AppEvent::HeartRateRawSampleUpdated(HeartRateRawSample {
                                    hrs: sample.hrs,
                                    als: sample.als,
                                }))
                                .await;
                            samples_until_ui_update = UI_SAMPLE_DIVISOR;
                        }
                    } else {
                        log_warn!("Heart-rate sample failed");
                    }
                }
            }
        }
    }

    async fn power_down(&mut self) {
        if self.sensor.power_down().await.is_err() {
            log_warn!("Heart-rate sensor power-down failed");
        }
    }
}

/// What the crate boundary bought.
///
/// The runner is driven here with no watch, no executor task and no `ipc`
/// statics: the I²C bus is a table of register answers, the channels belong to
/// the test, and the future is polled on the thread that built it. None of that
/// was reachable while this code lived in the firmware binary - `cargo test`
/// cannot build a `no_main` crate that pulls in `embassy-nrf`, so the lifecycle
/// below was only ever checked by wearing the result.
#[cfg(test)]
mod tests {
    use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
    use embedded_hal_async::i2c::{ErrorKind, ErrorType, I2c, Operation};
    use pineforge_state::HeartRateState;

    use super::{HeartRateCommand, HeartRateRunner};
    use crate::ports::{HeartRatePorts, ReadySignal};

    /// A bus that answers every read with one byte and remembers every write.
    ///
    /// Enough for the lifecycle: what the runner does with an identification
    /// byte is the question, and the registers it writes on the way are the
    /// driver's business rather than this test's.
    struct FakeBus {
        id: u8,
    }

    #[derive(Debug)]
    struct FakeError;

    impl embedded_hal_async::i2c::Error for FakeError {
        fn kind(&self) -> ErrorKind {
            ErrorKind::Other
        }
    }

    impl ErrorType for FakeBus {
        type Error = FakeError;
    }

    impl I2c for FakeBus {
        async fn transaction(
            &mut self,
            _address: u8,
            operations: &mut [Operation<'_>],
        ) -> Result<(), Self::Error> {
            for operation in operations {
                if let Operation::Read(buffer) = operation {
                    buffer.fill(self.id);
                }
            }
            Ok(())
        }
    }

    /// Drives the runner until the collector has seen what it came for.
    ///
    /// The runner never returns - it is a firmware task - so it is raced
    /// against the assertions rather than awaited. `select` drops the losing
    /// future, which is exactly what the executor does when a task is stopped.
    fn drive(id: u8, wanted: usize) -> std::vec::Vec<pineforge_state::AppEvent> {
        // Locals, not statics, and that is the whole of what makes this safe to
        // run beside itself. `cargo test` runs these in parallel threads; with
        // one shared channel both invocations pushed into it and each collected
        // whatever arrived first, so the two tests swapped results - which is
        // exactly how it failed, each asserting the other's sensor.
        //
        // Nothing here needs `'static`. `HeartRatePorts` borrows for as long as
        // the runner lives, and the runner does not outlive this call.
        let events: Channel<CriticalSectionRawMutex, pineforge_state::AppEvent, 8> = Channel::new();
        let commands: Channel<CriticalSectionRawMutex, HeartRateCommand, 2> = Channel::new();
        let motion_ready = ReadySignal::new();
        let bpm: embassy_sync::watch::Watch<CriticalSectionRawMutex, u8, 1> =
            embassy_sync::watch::Watch::new();

        // The bring-up order the PineTime wants, granted immediately: this test
        // is about what the runner does with the bus, not about who gets it
        // first.
        motion_ready.signal(());

        let runner = HeartRateRunner::new(
            FakeBus { id },
            HeartRatePorts {
                events: events.dyn_sender(),
                commands: commands.dyn_receiver(),
                bpm: bpm.dyn_sender(),
                motion_ready: &motion_ready,
            },
        );

        let collect = async {
            let mut seen = std::vec::Vec::new();
            while seen.len() < wanted {
                seen.push(events.receive().await);
            }
            seen
        };

        match futures_executor::block_on(embassy_futures::select::select(runner.run(), collect)) {
            embassy_futures::select::Either::First(()) => {
                panic!("the runner returned instead of waiting for a command")
            }
            embassy_futures::select::Either::Second(seen) => seen,
        }
    }

    /// A watch whose sensor answers with the wrong identification byte still
    /// reaches a defined state, and says so, rather than sitting in whatever
    /// the session was constructed with.
    #[test]
    fn a_sensor_that_is_not_there_is_reported_and_leaves_the_session_stopped() {
        let seen = drive(0x00, 2);

        assert_eq!(
            seen[0],
            pineforge_state::AppEvent::HeartRateSensorDetected(
                pineforge_state::HeartRateSensorKind::Unknown(0x00)
            ),
            "the identification byte the bus gave back is not what was reported"
        );
        assert_eq!(
            seen[1],
            pineforge_state::AppEvent::HeartRateStateUpdated(HeartRateState::Disabled),
            "a runner with no sensor still owes the UI a state"
        );
    }

    /// The sensor this watch has, recognised. The events either side are the
    /// same two, which is the point: what changes is the kind, not whether the
    /// runner reports at all.
    #[test]
    fn the_sensor_the_pinetime_carries_is_recognised() {
        let seen = drive(0x21, 1);

        assert_eq!(
            seen[0],
            pineforge_state::AppEvent::HeartRateSensorDetected(
                pineforge_state::HeartRateSensorKind::Hrs3300
            )
        );
    }
}
