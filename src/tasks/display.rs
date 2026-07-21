use core::cell::RefCell;

use defmt::info;
use embassy_embedded_hal::shared_bus::blocking::spi::SpiDevice;
use embassy_nrf::{
    gpio::{Level, Output, OutputDrive},
    spim,
};
use embassy_sync::blocking_mutex::NoopMutex;
use embassy_time::{Delay, Duration, Instant, Timer, with_deadline};
use mipidsi::interface::SpiInterface;
use mipidsi::options::{ColorInversion, Orientation};
use static_cell::StaticCell;

#[cfg(feature = "diagnostics")]
use crate::ui::test_screen::TestScreen;
use crate::{
    board::{
        peripherals::{DisplayResources, Irqs},
        pins,
    },
    boot::watchdog::BootloaderWatchdog,
    drivers::backlight::Backlight,
    services::events::UI_EVENTS,
    ui::{
        screen::Screen,
        transition::{SlideBuffer, draw_slide_reveal},
        watchface::TerminalWatchface,
    },
};
use pineforge_state::{
    AppEffect, AppEvent, AppState, DisplayPowerConfig, DisplayPowerPolicy, DisplayPowerState,
    ScreenId,
};

static SPI_BUS: StaticCell<NoopMutex<RefCell<spim::Spim<'static>>>> = StaticCell::new();
static DISPLAY_BUFFER: StaticCell<[u8; 512]> = StaticCell::new();
static SLIDE_BUFFER: StaticCell<SlideBuffer> = StaticCell::new();

const ACTIVE_BRIGHTNESS: u8 = 4;
const DIMMED_BRIGHTNESS: u8 = 1;
const WAKE_INPUT_GUARD: Duration = Duration::from_millis(500);

/// Owns the display and backlight and renders events received from the UI bus.
// Keeping setup and the event loop in one function makes peripheral ownership
// explicit for this single-owner task.
#[allow(clippy::too_many_lines)]
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
    backlight.set_level(ACTIVE_BRIGHTNESS);

    let started_at = Instant::now();
    let mut next_tick = started_at + Duration::from_secs(1);
    let mut watchface = TerminalWatchface::default();
    #[cfg(feature = "diagnostics")]
    let mut touch_test = TestScreen::default();
    let slide_buffer = SLIDE_BUFFER.init(SlideBuffer::new());
    let mut app = AppState::new(ScreenId::Watchface);
    let mut power = DisplayPowerPolicy::new(0, DisplayPowerConfig::DEFAULT);
    let mut ignore_input_until = started_at;
    let _ = watchface.draw_full(&mut display, || watchdog.pet());

    loop {
        let event = if power.state() == DisplayPowerState::Off {
            UI_EVENTS.receive().await
        } else {
            with_deadline(next_tick, UI_EVENTS.receive())
                .await
                .unwrap_or_else(|_| {
                    let now = Instant::now();
                    while next_tick <= now {
                        next_tick += Duration::from_secs(1);
                    }
                    AppEvent::Tick {
                        uptime_seconds: now.duration_since(started_at).as_secs(),
                    }
                })
        };
        let now = Instant::now();
        let now_millis = now.duration_since(started_at).as_millis();

        if event.is_user_activity() {
            if now < ignore_input_until {
                continue;
            }
            let was_off = power.state() == DisplayPowerState::Off;
            if power.on_activity(now_millis).is_some() {
                if was_off {
                    let uptime = AppEvent::Tick {
                        uptime_seconds: now.duration_since(started_at).as_secs(),
                    };
                    match app.active_screen() {
                        ScreenId::Watchface => {
                            let _ = watchface.handle_event(uptime);
                            let _ = display.wake(&mut delay);
                            let _ = watchface.draw_full(&mut display, || watchdog.pet());
                        }
                        #[cfg(feature = "diagnostics")]
                        ScreenId::TouchTest => {
                            let _ = touch_test.handle_event(uptime);
                            let _ = display.wake(&mut delay);
                            let _ = touch_test.draw_full(&mut display, || watchdog.pet());
                        }
                    }
                    backlight.set_level(ACTIVE_BRIGHTNESS);
                    ignore_input_until = Instant::now() + WAKE_INPUT_GUARD;
                    next_tick = Instant::now() + Duration::from_secs(1);
                    continue;
                }
                backlight.set_level(ACTIVE_BRIGHTNESS);
            }
        } else if let Some(next) = power.advance(now_millis) {
            match next {
                DisplayPowerState::Active => backlight.set_level(ACTIVE_BRIGHTNESS),
                DisplayPowerState::Dimmed => backlight.set_level(DIMMED_BRIGHTNESS),
                DisplayPowerState::Off => {
                    backlight.set_level(0);
                    let _ = display.sleep(&mut delay);
                }
            }
        }

        let action = match app.active_screen() {
            ScreenId::Watchface => watchface.handle_event(event),
            #[cfg(feature = "diagnostics")]
            ScreenId::TouchTest => touch_test.handle_event(event),
        };
        let effect = app.transition(action);

        if power.state() == DisplayPowerState::Off {
            continue;
        }

        match effect {
            AppEffect::RequestRollback => {
                info!("Rollback requested; resetting unconfirmed image");
                Timer::after_millis(250).await;
                cortex_m::peripheral::SCB::sys_reset();
            }
            AppEffect::Navigate(direction) => match app.active_screen() {
                ScreenId::Watchface => {
                    let result = draw_slide_reveal(
                        &watchface,
                        &mut display,
                        slide_buffer,
                        direction,
                        || watchdog.pet(),
                    );
                    #[cfg(feature = "diagnostics")]
                    if let Ok(metrics) = result {
                        touch_test.record_transition(direction, metrics);
                    }
                    #[cfg(not(feature = "diagnostics"))]
                    let _ = result;
                }
                #[cfg(feature = "diagnostics")]
                ScreenId::TouchTest => {
                    if let Ok(metrics) = draw_slide_reveal(
                        &touch_test,
                        &mut display,
                        slide_buffer,
                        direction,
                        || watchdog.pet(),
                    ) {
                        touch_test.record_transition(direction, metrics);
                        let _ = touch_test.draw_metrics(&mut display);
                        watchdog.pet();
                    }
                }
            },
            AppEffect::None => match app.active_screen() {
                ScreenId::Watchface => {
                    let _ = watchface.draw_dirty(&mut display, || watchdog.pet());
                }
                #[cfg(feature = "diagnostics")]
                ScreenId::TouchTest => {
                    let _ = touch_test.draw_dirty(&mut display, || watchdog.pet());
                }
            },
        }
    }
}
