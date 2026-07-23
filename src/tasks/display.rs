use defmt::info;
use embassy_futures::select::{Either3, Either4, select3, select4};
use embassy_nrf::gpio::{Level, Output, OutputDrive};
use embassy_time::{Delay, Duration, Instant, Timer};
use mipidsi::interface::SpiInterface;
use mipidsi::options::{ColorInversion, Orientation};
use static_cell::StaticCell;

#[cfg(feature = "diagnostics")]
use crate::services::events::HEART_RATE_COMMANDS;
#[cfg(feature = "diagnostics")]
use crate::ui::heart_rate::HeartRateScreen;
#[cfg(feature = "diagnostics")]
use crate::ui::test_screen::TestScreen;
use crate::{
    board::{buses::DisplaySpi, peripherals::DisplayResources, pins},
    boot::watchdog::BootloaderWatchdog,
    drivers::backlight::Backlight,
    services::events::{
        SETTINGS_COMMANDS, UI_EVENTS, display_settings_receiver, system_power_receiver,
    },
    ui::{
        screen::Screen,
        settings::DisplaySettingsScreen,
        transition::{SlideBuffer, draw_slide_reveal},
        watchface::TerminalWatchface,
    },
};
#[cfg(feature = "diagnostics")]
use pineforge_state::HeartRateCommand;
use pineforge_state::{AppEffect, AppEvent, AppState, DisplaySettings, ScreenId, SystemPowerState};

static DISPLAY_BUFFER: StaticCell<[u8; 512]> = StaticCell::new();
static SLIDE_BUFFER: StaticCell<SlideBuffer> = StaticCell::new();

const DIMMED_BRIGHTNESS: u8 = 1;
const WAKE_INPUT_GUARD: Duration = Duration::from_millis(500);

enum DisplayEvent {
    Ui(AppEvent),
    Power(SystemPowerState),
    Settings(DisplaySettings),
}

/// Owns the display and backlight and renders events received from the UI bus.
// Keeping setup and the event loop in one function makes peripheral ownership
// explicit for this single-owner task.
#[allow(clippy::too_many_lines)]
#[embassy_executor::task]
pub async fn run(resources: DisplayResources, spi: DisplaySpi, watchdog: BootloaderWatchdog) {
    let mut backlight = Backlight::new(
        Output::new(resources.backlight_low, Level::High, OutputDrive::Standard),
        Output::new(resources.backlight_mid, Level::High, OutputDrive::Standard),
        Output::new(resources.backlight_high, Level::High, OutputDrive::Standard),
    );
    backlight.set_level(1);

    let dc = Output::new(resources.dc, Level::Low, OutputDrive::Standard);
    let reset = Output::new(resources.reset, Level::Low, OutputDrive::Standard);
    let interface = SpiInterface::new(spi, dc, DISPLAY_BUFFER.init([0; 512]));
    let mut delay = Delay;
    let mut display = mipidsi::Builder::new(mipidsi::models::ST7789, interface)
        .display_size(pins::DISPLAY_WIDTH, pins::DISPLAY_HEIGHT)
        .invert_colors(ColorInversion::Inverted)
        .reset_pin(reset)
        .init(&mut delay)
        .unwrap();
    display.set_orientation(Orientation::new()).unwrap();
    watchdog.pet();
    let mut settings = DisplaySettings::DEFAULT;
    backlight.set_level(settings.brightness());

    let started_at = Instant::now();
    let mut next_tick = started_at + Duration::from_secs(1);
    let mut watchface = TerminalWatchface::default();
    let mut display_settings = DisplaySettingsScreen::default();
    #[cfg(feature = "diagnostics")]
    let mut touch_test = TestScreen::default();
    #[cfg(feature = "diagnostics")]
    let mut heart_rate = HeartRateScreen::default();
    let slide_buffer = SLIDE_BUFFER.init(SlideBuffer::new());
    let mut app = AppState::new(ScreenId::Watchface);
    let mut power_receiver = system_power_receiver();
    let mut settings_receiver = display_settings_receiver();
    let mut power = power_receiver.get().await;
    let mut ignore_input_until = started_at;
    let _ = watchface.draw_full(&mut display, || watchdog.pet());
    match power {
        SystemPowerState::Interactive => backlight.set_level(settings.brightness()),
        SystemPowerState::Idle => backlight.set_level(DIMMED_BRIGHTNESS),
        SystemPowerState::Sleeping => {
            backlight.set_level(0);
            let _ = display.sleep(&mut delay);
        }
    }

    loop {
        let display_event = if power == SystemPowerState::Sleeping {
            match select3(
                UI_EVENTS.receive(),
                power_receiver.changed(),
                settings_receiver.changed(),
            )
            .await
            {
                Either3::First(event) => DisplayEvent::Ui(event),
                Either3::Second(state) => DisplayEvent::Power(state),
                Either3::Third(snapshot) => DisplayEvent::Settings(snapshot),
            }
        } else {
            match select4(
                UI_EVENTS.receive(),
                Timer::at(next_tick),
                power_receiver.changed(),
                settings_receiver.changed(),
            )
            .await
            {
                Either4::First(event) => DisplayEvent::Ui(event),
                Either4::Second(()) => {
                    let now = Instant::now();
                    while next_tick <= now {
                        next_tick += Duration::from_secs(1);
                    }
                    DisplayEvent::Ui(AppEvent::Tick {
                        uptime_seconds: now.duration_since(started_at).as_secs(),
                    })
                }
                Either4::Third(state) => DisplayEvent::Power(state),
                Either4::Fourth(snapshot) => DisplayEvent::Settings(snapshot),
            }
        };
        let now = Instant::now();

        let event = match display_event {
            DisplayEvent::Ui(event) => event,
            DisplayEvent::Settings(snapshot) => {
                settings = snapshot;
                if power == SystemPowerState::Interactive {
                    backlight.set_level(settings.brightness());
                }
                AppEvent::DisplaySettingsUpdated(snapshot)
            }
            DisplayEvent::Power(next) => {
                if next == power {
                    continue;
                }
                let was_sleeping = power == SystemPowerState::Sleeping;
                power = next;
                match next {
                    SystemPowerState::Interactive if was_sleeping => {
                        let uptime = AppEvent::Tick {
                            uptime_seconds: now.duration_since(started_at).as_secs(),
                        };
                        match app.active_screen() {
                            ScreenId::Watchface => {
                                let _ = watchface.handle_event(uptime);
                                let _ = display.wake(&mut delay);
                                let _ = watchface.draw_full(&mut display, || watchdog.pet());
                            }
                            ScreenId::DisplaySettings => {
                                let _ = display_settings.handle_event(uptime);
                                let _ = display.wake(&mut delay);
                                let _ = display_settings.draw_full(&mut display, || watchdog.pet());
                            }
                            #[cfg(feature = "diagnostics")]
                            ScreenId::TouchTest => {
                                let _ = touch_test.handle_event(uptime);
                                let _ = display.wake(&mut delay);
                                let _ = touch_test.draw_full(&mut display, || watchdog.pet());
                            }
                            #[cfg(feature = "diagnostics")]
                            ScreenId::HeartRate => {
                                let _ = heart_rate.handle_event(uptime);
                                let _ = display.wake(&mut delay);
                                let _ = heart_rate.draw_full(&mut display, || watchdog.pet());
                            }
                        }
                        backlight.set_level(settings.brightness());
                        ignore_input_until = Instant::now() + WAKE_INPUT_GUARD;
                        next_tick = Instant::now() + Duration::from_secs(1);
                    }
                    SystemPowerState::Interactive => backlight.set_level(settings.brightness()),
                    SystemPowerState::Idle => backlight.set_level(DIMMED_BRIGHTNESS),
                    SystemPowerState::Sleeping => {
                        #[cfg(feature = "diagnostics")]
                        if app.active_screen() == ScreenId::HeartRate {
                            let _ = heart_rate.handle_event(AppEvent::HeartRateStateUpdated(
                                pineforge_state::HeartRateState::Disabled,
                            ));
                        }
                        backlight.set_level(0);
                        let _ = display.sleep(&mut delay);
                    }
                }
                continue;
            }
        };

        if power == SystemPowerState::Sleeping
            || (event.is_user_activity() && now < ignore_input_until)
        {
            continue;
        }

        let action = match app.active_screen() {
            ScreenId::Watchface => watchface.handle_event(event),
            ScreenId::DisplaySettings => display_settings.handle_event(event),
            #[cfg(feature = "diagnostics")]
            ScreenId::TouchTest => touch_test.handle_event(event),
            #[cfg(feature = "diagnostics")]
            ScreenId::HeartRate => heart_rate.handle_event(event),
        };
        let effect = app.transition(action);

        match effect {
            AppEffect::ApplySettings(updated) => {
                SETTINGS_COMMANDS.send(updated).await;
                let _ = display_settings.draw_dirty(&mut display, || watchdog.pet());
            }
            AppEffect::RequestRollback => {
                info!("Rollback requested; resetting unconfirmed image");
                Timer::after_millis(250).await;
                cortex_m::peripheral::SCB::sys_reset();
            }
            AppEffect::Navigate(direction) => {
                #[cfg(feature = "diagnostics")]
                if app.active_screen() == ScreenId::HeartRate {
                    heart_rate.begin_measurement();
                }
                #[cfg(feature = "diagnostics")]
                HEART_RATE_COMMANDS
                    .send(if app.active_screen() == ScreenId::HeartRate {
                        HeartRateCommand::Start
                    } else {
                        HeartRateCommand::Stop
                    })
                    .await;
                match app.active_screen() {
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
                    ScreenId::DisplaySettings => {
                        let _ = draw_slide_reveal(
                            &display_settings,
                            &mut display,
                            slide_buffer,
                            direction,
                            || watchdog.pet(),
                        );
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
                    #[cfg(feature = "diagnostics")]
                    ScreenId::HeartRate => {
                        let _ = draw_slide_reveal(
                            &heart_rate,
                            &mut display,
                            slide_buffer,
                            direction,
                            || watchdog.pet(),
                        );
                    }
                }
            }
            AppEffect::None => match app.active_screen() {
                ScreenId::Watchface => {
                    let _ = watchface.draw_dirty(&mut display, || watchdog.pet());
                }
                ScreenId::DisplaySettings => {
                    let _ = display_settings.draw_dirty(&mut display, || watchdog.pet());
                }
                #[cfg(feature = "diagnostics")]
                ScreenId::TouchTest => {
                    let _ = touch_test.draw_dirty(&mut display, || watchdog.pet());
                }
                #[cfg(feature = "diagnostics")]
                ScreenId::HeartRate => {
                    let _ = heart_rate.draw_dirty(&mut display, || watchdog.pet());
                }
            },
        }
    }
}
