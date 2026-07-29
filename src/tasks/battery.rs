use embassy_futures::select::{Either3, select3};
use embassy_nrf::{
    gpio::{Input, Pull},
    saadc::{ChannelConfig, Config, Gain, Reference, Resolution, Saadc, Time},
};
use embassy_time::{Duration, Instant, Timer};
use pineforge_state::{
    AppEvent, BatteryCapacityEstimator, BatteryStatus, ChargerPins, PowerCommand,
    battery_millivolts, battery_sample_interval_seconds,
};

use crate::{
    board::peripherals::{BatteryResources, Irqs},
    ipc::{BATTERY_STATUS, POWER_COMMANDS, UI_EVENTS},
};

const SAMPLES_PER_READING: i32 = 4;

/// The measurement rate, which is battery policy and lives with the rest of it.
const fn sample_interval(power_present: bool) -> Duration {
    Duration::from_secs(battery_sample_interval_seconds(power_present))
}

/// Owns the battery ADC and the two charger inputs for the lifetime of the app.
///
/// The two pins answer different questions: P0.12 is whether current is
/// flowing, P0.19 is whether the watch is on the pad at all. Charging requires
/// both, which is not redundancy - P0.12 has no pull, so an unpowered charger
/// can leave it floating, and external power is what says the reading means
/// anything. `InfiniTime` reads P0.12 alone and gets away with it; this is the
/// more conservative of the two and costs nothing now that both pins are
/// watched rather than one watched and one merely read.
///
/// What shows on the face says which pin answered: `CHG` is both, `PWR` is
/// external power without charge current - a watch that is already full - and
/// `BAT` is no external power.
#[embassy_executor::task]
pub async fn run(resources: BatteryResources) {
    let mut charge_status = Input::new(resources.charge_status, Pull::None);
    // Watched, not just read. Which of the two pins moves when a watch is set
    // down on the pad depends on how full it is: a flat one starts charging and
    // moves both, but a nearly full one may never enter constant current at
    // all, so the charge indication stays put and external power is the only
    // thing that changes. Waiting on the charge pin alone therefore missed
    // exactly the case that happens most - the watch you top up before bed.
    let mut power_present = Input::new(resources.power_present, Pull::None);

    let mut channel = ChannelConfig::single_ended(resources.voltage);
    channel.reference = Reference::Internal;
    channel.gain = Gain::Gain1_4;
    // The PineTime's 1 MOhm + 1 MOhm divider is a high-impedance source.
    channel.time = Time::_40US;

    let mut config = Config::default();
    config.resolution = Resolution::_12bit;
    let mut adc = Saadc::new(resources.adc, Irqs, config, [channel]);
    let mut estimator = BatteryCapacityEstimator::new();
    let millivolts = sample_millivolts(&mut adc).await;
    let mut pins = read_pins(&charge_status, &power_present);
    let mut status = BatteryStatus {
        millivolts,
        percent: estimator.observe(millivolts, pins.power_present, 0),
        charging: pins.charging(),
        power_present: pins.power_present,
    };
    let mut last_observation = Instant::now();
    publish(status).await;

    loop {
        match select3(
            Timer::after(sample_interval(status.power_present)),
            charge_status.wait_for_any_edge(),
            power_present.wait_for_any_edge(),
        )
        .await
        {
            Either3::First(()) => {
                let millivolts = sample_millivolts(&mut adc).await;
                let observation_time = Instant::now();
                let elapsed_seconds = observation_time.duration_since(last_observation).as_secs();
                last_observation = observation_time;
                pins = read_pins(&charge_status, &power_present);
                status = BatteryStatus {
                    millivolts,
                    percent: estimator.observe(millivolts, pins.power_present, elapsed_seconds),
                    charging: pins.charging(),
                    power_present: pins.power_present,
                };
            }
            // Either pin moving means the same thing: the charger situation
            // changed, so both flags are re-read. Reading both rather than the
            // one that woke us is what makes a simultaneous change safe - the
            // second pin's edge is already spent by the time we re-arm, but its
            // level is not.
            Either3::Second(()) | Either3::Third(()) => {
                // Update power flags promptly without turning charger-induced
                // terminal-voltage changes into capacity observations.
                Timer::after_millis(30).await;
                let before = pins;
                pins = read_pins(&charge_status, &power_present);
                status.power_present = pins.power_present;
                status.charging = pins.charging();

                // Setting a watch down on the pad lights it up, the way
                // `InfiniTime` does it - its charger interrupt calls
                // `GoToRunning()`, which is most of why the charge state feels
                // immediate there and did not here. Waking is also the only
                // acknowledgement a sealed watch can give that the charger was
                // noticed at all.
                //
                // Only on a real change, so a charge indication that chatters
                // while the charger settles cannot hold the screen on. Never
                // awaited: a full queue means the user is already busy being
                // active, which is the state this asks for anyway.
                if pins != before {
                    let _ = POWER_COMMANDS.try_send(PowerCommand::UserActivity);
                }
            }
        }
        publish(status).await;
    }
}

/// Reads both charger pins as one observation.
///
/// Together rather than one at a time, because what they mean is a property of
/// the pair: the level of the pin that did not wake us is as much a part of the
/// answer as the one that did.
fn read_pins(charge: &Input<'static>, power: &Input<'static>) -> ChargerPins {
    ChargerPins {
        power_present: power.is_low(),
        charge_indicated: charge.is_low(),
    }
}

async fn sample_millivolts(adc: &mut Saadc<'_, 1>) -> u16 {
    let mut sum = 0_i32;
    for _ in 0..SAMPLES_PER_READING {
        let mut sample = [0_i16; 1];
        adc.sample(&mut sample).await;
        sum += i32::from(sample[0]);
    }
    let average = i16::try_from(sum / SAMPLES_PER_READING).unwrap_or_default();
    battery_millivolts(average)
}

async fn publish(status: BatteryStatus) {
    // Documented duplication until the UI rewrite: the display consumes the
    // event stream, the BLE battery service the watch snapshot.
    BATTERY_STATUS.sender().send(status);
    UI_EVENTS.send(AppEvent::BatteryUpdated(status)).await;
}
