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

#[cfg(feature = "diagnostics")]
use board::peripherals::HeartRateResources;
use board::peripherals::{
    BatteryResources, ButtonResources, DisplayFlashBusResources, DisplayResources,
    SensorBusResources, TouchResources, VibrationResources,
};
use boot::watchdog::BootloaderWatchdog;

#[embassy_executor::main]
async fn main(spawner: embassy_executor::Spawner) {
    info!("PineForge touch test starting; image remains unconfirmed");

    let mut config = embassy_nrf::config::Config::default();
    config.hfclk_source = embassy_nrf::config::HfclkSource::ExternalXtal;
    config.lfclk_source = embassy_nrf::config::LfclkSource::ExternalXtal;
    // MPSL reserves the highest interrupt priorities for the radio protocol
    // stack; keep Embassy's GPIOTE and time driver out of its way.
    #[cfg(feature = "ble")]
    {
        config.gpiote_interrupt_priority = embassy_nrf::interrupt::Priority::P2;
        config.time_interrupt_priority = embassy_nrf::interrupt::Priority::P2;
    }
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
    spawner.spawn(defmt::unwrap!(services::power::run()));

    let sensor_bus = board::buses::init_sensor_bus(SensorBusResources {
        i2c: p.TWISPI1,
        sda: p.P0_06,
        scl: p.P0_07,
    });
    spawner.spawn(defmt::unwrap!(tasks::accelerometer::run(
        board::buses::motion_device(sensor_bus),
        sensor_bus,
    )));
    #[cfg(feature = "diagnostics")]
    spawner.spawn(defmt::unwrap!(tasks::heart_rate::run(
        HeartRateResources { interrupt: p.P0_30 },
        board::buses::heart_rate_device(sensor_bus)
    )));

    spawner.spawn(defmt::unwrap!(tasks::battery::run(BatteryResources {
        adc: p.SAADC,
        voltage: p.P0_31,
        charge_status: p.P0_12,
        power_present: p.P0_19,
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

    let display_flash_bus = board::buses::init_display_flash_bus(DisplayFlashBusResources {
        spi: p.TWISPI0,
        sck: p.P0_02,
        miso: p.P0_04,
        mosi: p.P0_03,
    });
    spawner.spawn(defmt::unwrap!(tasks::display::run(
        DisplayResources {
            dc: p.P0_18,
            reset: p.P0_26,
            backlight_low: p.P0_14,
            backlight_mid: p.P0_22,
            backlight_high: p.P0_23,
        },
        board::buses::display_device(display_flash_bus, p.P0_25),
        watchdog
    )));
    spawner.spawn(defmt::unwrap!(services::settings::run(
        board::buses::flash_device(display_flash_bus, p.P0_05)
    )));
    spawner.spawn(defmt::unwrap!(tasks::vibration::run(VibrationResources {
        motor: p.P0_16,
    })));
    #[cfg(feature = "ble")]
    spawner.spawn(defmt::unwrap!(tasks::ble::run(
        board::peripherals::BleResources {
            rtc0: p.RTC0,
            timer0: p.TIMER0,
            temp: p.TEMP,
            rng: p.RNG,
            ppi_ch17: p.PPI_CH17,
            ppi_ch18: p.PPI_CH18,
            ppi_ch19: p.PPI_CH19,
            ppi_ch20: p.PPI_CH20,
            ppi_ch21: p.PPI_CH21,
            ppi_ch22: p.PPI_CH22,
            ppi_ch23: p.PPI_CH23,
            ppi_ch24: p.PPI_CH24,
            ppi_ch25: p.PPI_CH25,
            ppi_ch26: p.PPI_CH26,
            ppi_ch27: p.PPI_CH27,
            ppi_ch28: p.PPI_CH28,
            ppi_ch29: p.PPI_CH29,
            ppi_ch30: p.PPI_CH30,
            ppi_ch31: p.PPI_CH31,
        },
        spawner
    )));
    spawner.spawn(defmt::unwrap!(tasks::input::run(
        TouchResources {
            reset: p.P0_10,
            interrupt: p.P0_28,
        },
        board::buses::touch_device(sensor_bus)
    )));
}
