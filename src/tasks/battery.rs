use embassy_futures::select::{Either, select};
use embassy_nrf::{
    gpio::{Input, Pull},
    saadc::{ChannelConfig, Config, Gain, Reference, Resolution, Saadc, Time},
};
use embassy_time::{Duration, Timer};
use pineforge_state::{AppEvent, BatteryStatus, battery_millivolts, battery_percent};

use crate::{
    board::peripherals::{BatteryResources, Irqs},
    services::events::UI_EVENTS,
};

const SAMPLE_INTERVAL: Duration = Duration::from_secs(30);
const SAMPLES_PER_READING: i32 = 4;

/// Owns the battery ADC and charger-status input for the lifetime of the app.
#[embassy_executor::task]
pub async fn run(resources: BatteryResources) {
    let mut charge_status = Input::new(resources.charge_status, Pull::None);

    let mut channel = ChannelConfig::single_ended(resources.voltage);
    channel.reference = Reference::Internal;
    channel.gain = Gain::Gain1_4;
    // The PineTime's 1 MOhm + 1 MOhm divider is a high-impedance source.
    channel.time = Time::_40US;

    let mut config = Config::default();
    config.resolution = Resolution::_12bit;
    let mut adc = Saadc::new(resources.adc, Irqs, config, [channel]);
    adc.calibrate().await;

    // nRF52832 anomaly 86 can place an invalid first result in RAM when
    // sampling starts after offset calibration. Consume exactly that result;
    // subsequent one-shot samples are valid and stop the peripheral cleanly.
    let mut post_calibration_sample = [0_i16; 1];
    adc.sample(&mut post_calibration_sample).await;

    let mut millivolts = sample_millivolts(&mut adc).await;
    publish(millivolts, charge_status.is_low()).await;

    loop {
        match select(
            Timer::after(SAMPLE_INTERVAL),
            charge_status.wait_for_any_edge(),
        )
        .await
        {
            Either::First(()) => millivolts = sample_millivolts(&mut adc).await,
            Either::Second(()) => {
                // Ignore short charger-contact transients before publishing.
                Timer::after_millis(30).await;
            }
        }
        publish(millivolts, charge_status.is_low()).await;
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

async fn publish(millivolts: u16, charging: bool) {
    let status = BatteryStatus {
        millivolts,
        percent: battery_percent(millivolts),
        // PineTime's charge-status signal is active-low.
        charging,
    };
    UI_EVENTS.send(AppEvent::BatteryUpdated(status)).await;
}
