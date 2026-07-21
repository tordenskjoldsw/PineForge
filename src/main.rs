#![no_std]
#![no_main]
// Embassy's entry-point macro captures a non-Send executor token by design;
// the Cortex-M thread executor never moves this future between threads.
#![allow(clippy::future_not_send)]

use defmt::info;
use defmt_rtt as _;
use embassy_nrf::gpio::{Input, Level, Output, OutputDrive, Pull};
use panic_probe as _;

mod board;
mod boot;
mod drivers;
mod services;
mod tasks;
mod ui;

use board::peripherals::{BatteryResources, ButtonResources, DisplayResources, TouchResources};
use boot::watchdog::BootloaderWatchdog;

#[embassy_executor::main]
async fn main(spawner: embassy_executor::Spawner) {
    info!("PineForge touch test starting; image remains unconfirmed");

    let mut config = embassy_nrf::config::Config::default();
    config.hfclk_source = embassy_nrf::config::HfclkSource::ExternalXtal;
    config.lfclk_source = embassy_nrf::config::LfclkSource::ExternalXtal;
    let p = embassy_nrf::init(config);

    // MCUBoot enters the image by branching to its reset handler rather than
    // performing a hardware reset. Restore the reset-state expectation that
    // global interrupts are enabled so Embassy's RTC time driver can wake.
    #[allow(unsafe_code)]
    unsafe {
        cortex_m::interrupt::enable();
    }

    // Pet synchronously before any peripheral setup can block or fail.
    let watchdog = BootloaderWatchdog::take_over();
    watchdog.pet();
    spawner.spawn(defmt::unwrap!(tasks::watchdog::run(watchdog)));
    spawner.spawn(defmt::unwrap!(boot::rollback::safety_timeout()));

    spawner.spawn(defmt::unwrap!(tasks::battery::run(BatteryResources {
        adc: p.SAADC,
        voltage: p.P0_31,
        charge_status: p.P0_12,
    })));

    let button = ButtonResources {
        input: p.P0_13,
        enable: p.P0_15,
    };
    let button_enable = Output::new(button.enable, Level::High, OutputDrive::Standard);
    let button_input = Input::new(button.input, Pull::Down);
    spawner.spawn(defmt::unwrap!(boot::rollback::side_button(
        button_input,
        button_enable
    )));

    spawner.spawn(defmt::unwrap!(tasks::display::run(
        DisplayResources {
            spi: p.TWISPI0,
            sck: p.P0_02,
            miso: p.P0_04,
            mosi: p.P0_03,
            dc: p.P0_18,
            cs: p.P0_25,
            reset: p.P0_26,
            backlight_low: p.P0_14,
            backlight_mid: p.P0_22,
            backlight_high: p.P0_23,
        },
        watchdog
    )));
    spawner.spawn(defmt::unwrap!(tasks::input::run(TouchResources {
        i2c: p.TWISPI1,
        sda: p.P0_06,
        scl: p.P0_07,
        reset: p.P0_10,
        interrupt: p.P0_28,
    })));
}
