use core::cell::RefCell;

use defmt::info;
use embassy_embedded_hal::shared_bus::blocking::spi::SpiDevice;
use embassy_nrf::{
    gpio::{Level, Output, OutputDrive},
    spim,
};
use embassy_sync::blocking_mutex::NoopMutex;
use embassy_time::{Delay, Timer};
use embedded_graphics::{draw_target::DrawTarget, pixelcolor::Rgb565, prelude::RgbColor};
use mipidsi::interface::SpiInterface;
use mipidsi::options::{ColorInversion, Orientation};
use static_cell::StaticCell;

use crate::{
    board::{
        peripherals::{DisplayResources, Irqs},
        pins,
    },
    boot::watchdog::BootloaderWatchdog,
    drivers::backlight::Backlight,
    services::events::UI_EVENTS,
    ui::{
        screen::{Screen, ScreenAction},
        test_screen::TestScreen,
    },
};

static SPI_BUS: StaticCell<NoopMutex<RefCell<spim::Spim<'static>>>> = StaticCell::new();
static DISPLAY_BUFFER: StaticCell<[u8; 512]> = StaticCell::new();

/// Owns the display and backlight and renders events received from the UI bus.
#[embassy_executor::task]
pub async fn run(resources: DisplayResources, watchdog: BootloaderWatchdog) {
    let mut backlight = Backlight::new(
        Output::new(resources.backlight_low, Level::High, OutputDrive::Standard),
        Output::new(resources.backlight_mid, Level::High, OutputDrive::Standard),
        Output::new(resources.backlight_high, Level::High, OutputDrive::Standard),
    );
    backlight.set_level(1);

    let mut spi_config = spim::Config::default();
    spi_config.frequency = spim::Frequency::M8;
    spi_config.mode = spim::MODE_3;
    let spi = spim::Spim::new(
        resources.spi,
        Irqs,
        resources.sck,
        resources.miso,
        resources.mosi,
        spi_config,
    );
    let dc = Output::new(resources.dc, Level::Low, OutputDrive::Standard);
    let cs = Output::new(resources.cs, Level::High, OutputDrive::Standard);
    let reset = Output::new(resources.reset, Level::Low, OutputDrive::Standard);
    let spi_bus = SPI_BUS.init(NoopMutex::new(RefCell::new(spi)));
    let interface = SpiInterface::new(
        SpiDevice::new(spi_bus, cs),
        dc,
        DISPLAY_BUFFER.init([0; 512]),
    );
    let mut delay = Delay;
    let mut display = mipidsi::Builder::new(mipidsi::models::ST7789, interface)
        .display_size(pins::DISPLAY_WIDTH, pins::DISPLAY_HEIGHT)
        .invert_colors(ColorInversion::Inverted)
        .reset_pin(reset)
        .init(&mut delay)
        .unwrap();
    display.set_orientation(Orientation::new()).unwrap();
    watchdog.pet();
    backlight.set_level(4);

    let _ = display.clear(Rgb565::BLACK);
    watchdog.pet();
    let mut screen = TestScreen::default();
    let _ = screen.draw(&mut display, || watchdog.pet());

    loop {
        let action = screen.handle_event(UI_EVENTS.receive().await);

        if action == ScreenAction::RequestRollback {
            info!("Rollback requested; resetting unconfirmed image");
            Timer::after_millis(250).await;
            cortex_m::peripheral::SCB::sys_reset();
        }

        let _ = screen.draw_update(&mut display, || watchdog.pet());
    }
}
