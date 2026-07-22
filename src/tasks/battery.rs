use embassy_futures::select::{Either, select};
use embassy_nrf::{
    gpio::{Input, Pull},
    saadc::{ChannelConfig, Config, Gain, Reference, Resolution, Saadc, Time},
};
use embassy_time::{Duration, Instant, Timer};
use pineforge_state::{AppEvent, BatteryCapacityEstimator, BatteryStatus, battery_millivolts};

use crate::{
    board::peripherals::{BatteryResources, Irqs},
    services::events::UI_EVENTS,
};

#[cfg(feature = "diagnostics")]
const fn sample_interval(_power_present: bool) -> Duration {
    Duration::from_secs(30)
}

#[cfg(not(feature = "diagnostics"))]
const fn sample_interval(power_present: bool) -> Duration {
    if power_present {
        Duration::from_secs(60)
    } else {
        Duration::from_secs(10 * 60)
    }
}

const SAMPLES_PER_READING: i32 = 4;

/// Owns the battery ADC and charger-status input for the lifetime of the app.
#[embassy_executor::task]
pub async fn run(resources: BatteryResources) {
    let mut charge_status = Input::new(resources.charge_status, Pull::None);
    let power_present = Input::new(resources.power_present, Pull::None);

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
    let externally_powered = power_present.is_low();
    let mut status = BatteryStatus {
        millivolts,
        percent: estimator.observe(millivolts, externally_powered, 0),
        charging: externally_powered && charge_status.is_low(),
        power_present: externally_powered,
    };
    let mut last_observation = Instant::now();
    publish(status).await;

    loop {
        match select(
            Timer::after(sample_interval(status.power_present)),
            charge_status.wait_for_any_edge(),
        )
        .await
        {
            Either::First(()) => {
                let millivolts = sample_millivolts(&mut adc).await;
                let observation_time = Instant::now();
                let elapsed_seconds = observation_time.duration_since(last_observation).as_secs();
                last_observation = observation_time;
                let externally_powered = power_present.is_low();
                status = BatteryStatus {
                    millivolts,
                    percent: estimator.observe(millivolts, externally_powered, elapsed_seconds),
                    charging: externally_powered && charge_status.is_low(),
                    power_present: externally_powered,
                };
            }
            Either::Second(()) => {
                // Update power flags promptly without turning charger-induced
                // terminal-voltage changes into capacity observations.
                Timer::after_millis(30).await;
                status.power_present = power_present.is_low();
                status.charging = status.power_present && charge_status.is_low();
            }
        }
        publish(status).await;
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
    UI_EVENTS.send(AppEvent::BatteryUpdated(status)).await;
}
