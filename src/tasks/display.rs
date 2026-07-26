use defmt::info;
use embassy_futures::select::{Either3, Either4, select3, select4};
use embassy_nrf::gpio::{Level, Output, OutputDrive};
use embassy_time::{Delay, Duration, Instant, Timer};
use mipidsi::interface::SpiInterface;
use mipidsi::options::{ColorInversion, Orientation};
use static_cell::StaticCell;

use crate::services::events::HEART_RATE_COMMANDS;
#[cfg(feature = "diagnostics")]
use crate::ui::test_screen::TestScreen;
#[cfg(feature = "ui-animations")]
use crate::ui::{scratch::UiScratch, transition::draw_slide_reveal};
use crate::{
    board::{buses::DisplaySpi, peripherals::DisplayResources, pins},
    boot::watchdog::BootloaderWatchdog,
    drivers::backlight::Backlight,
    services::events::{
        POWER_COMMANDS, SETTINGS_COMMANDS, UI_EVENTS, VIBRATION_COMMANDS,
        display_settings_receiver, system_power_receiver, wall_clock_receiver,
    },
    ui::{
        canvas::{Canvas, CanvasError},
        dfu::{draw_dfu_failed, draw_dfu_progress, draw_storage_progress},
        firmware::FirmwareScreen,
        launcher::LauncherScreen,
        pairing::draw_pairing,
        screen::{Paint, Screen},
        settings::DisplaySettingsScreen,
        status::{StatusCorner, WithStatus, wears_status},
        watchface::WatchfaceScreen,
        watchface_select::WatchfaceSelectScreen,
    },
};
use pineforge_state::{
    AppEffect, AppEvent, AppState, DisplaySettings, HeartRateCommand, Modal, ModalOutcome,
    ModalState, PowerCommand, ScreenAction, ScreenId, SystemPowerState, VibrationPattern,
};

static DISPLAY_BUFFER: StaticCell<[u8; 512]> = StaticCell::new();
#[cfg(feature = "ui-animations")]
static UI_SCRATCH: StaticCell<UiScratch> = StaticCell::new();

const DIMMED_BRIGHTNESS: u8 = 1;
const WAKE_INPUT_GUARD: Duration = Duration::from_millis(500);

enum DisplayEvent {
    Ui(AppEvent),
    Power(SystemPowerState),
    Settings(DisplaySettings),
}

/// Every screen instance, dispatching to whichever one the navigation state
/// says is active.
///
/// Screens are held for the lifetime of the task so their model state survives
/// navigation and sleep; only the active one receives events and draws.
struct Screens {
    watchface: WatchfaceScreen,
    launcher: LauncherScreen,
    settings: DisplaySettingsScreen,
    watchface_select: WatchfaceSelectScreen,
    firmware: FirmwareScreen,
    #[cfg(feature = "diagnostics")]
    touch_test: TestScreen,
}

impl Screens {
    fn handle(&mut self, active: ScreenId, event: AppEvent) -> ScreenAction {
        match active {
            ScreenId::Watchface => self.watchface.handle_event(event),
            ScreenId::Launcher => self.launcher.handle_event(event),
            ScreenId::DisplaySettings => self.settings.handle_event(event),
            ScreenId::WatchfaceSelect => self.watchface_select.handle_event(event),
            ScreenId::Firmware => self.firmware.handle_event(event),
            #[cfg(feature = "diagnostics")]
            ScreenId::TouchTest => self.touch_test.handle_event(event),
        }
    }

    /// Paints a screen and, unless it is a watchface, the status corner over
    /// it. The two are drawn together so a transition composing this stripe by
    /// stripe carries the corner with it instead of adding it afterwards.
    fn draw_full(
        &self,
        active: ScreenId,
        status: &StatusCorner,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        match active {
            ScreenId::Watchface => self.watchface.draw_full(canvas, keep_alive),
            ScreenId::Launcher => {
                WithStatus::new(&self.launcher, status).draw_full(canvas, keep_alive)
            }
            ScreenId::DisplaySettings => {
                WithStatus::new(&self.settings, status).draw_full(canvas, keep_alive)
            }
            ScreenId::WatchfaceSelect => {
                WithStatus::new(&self.watchface_select, status).draw_full(canvas, keep_alive)
            }
            ScreenId::Firmware => {
                WithStatus::new(&self.firmware, status).draw_full(canvas, keep_alive)
            }
            #[cfg(feature = "diagnostics")]
            ScreenId::TouchTest => {
                WithStatus::new(&self.touch_test, status).draw_full(canvas, keep_alive)
            }
        }
    }

    fn draw_dirty(
        &self,
        active: ScreenId,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        match active {
            ScreenId::Watchface => self.watchface.draw_dirty(canvas, keep_alive),
            ScreenId::Launcher => self.launcher.draw_dirty(canvas, keep_alive),
            ScreenId::DisplaySettings => self.settings.draw_dirty(canvas, keep_alive),
            ScreenId::WatchfaceSelect => self.watchface_select.draw_dirty(canvas, keep_alive),
            ScreenId::Firmware => self.firmware.draw_dirty(canvas, keep_alive),
            #[cfg(feature = "diagnostics")]
            ScreenId::TouchTest => self.touch_test.draw_dirty(canvas, keep_alive),
        }
    }
}

/// Draws the system modal that owns the screen.
fn draw_modal(
    canvas: &mut Canvas<'_>,
    modal: Modal,
    keep_alive: &mut dyn FnMut(),
) -> Result<(), CanvasError> {
    match modal {
        Modal::StorageFormat(percent) => draw_storage_progress(canvas, percent, keep_alive),
        Modal::Pairing(passkey) => draw_pairing(canvas, passkey, keep_alive),
        Modal::DfuProgress(percent) => draw_dfu_progress(canvas, percent, keep_alive),
        Modal::DfuFailed(reason) => draw_dfu_failed(canvas, reason, keep_alive),
    }
}

/// Whether showing this modal counts as user activity.
///
/// A prompt or a transfer in flight renews the idle timer so it stays readable.
/// The terminal failure screen deliberately does not: it survives sleep and is
/// redrawn on wake, so it must not hold the backlight on indefinitely.
const fn renews_activity(modal: Modal) -> bool {
    !matches!(modal, Modal::DfuFailed(_))
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
    let mut screens = Screens {
        watchface: WatchfaceScreen::default(),
        launcher: LauncherScreen::default(),
        settings: DisplaySettingsScreen::default(),
        watchface_select: WatchfaceSelectScreen::default(),
        firmware: FirmwareScreen::default(),
        #[cfg(feature = "diagnostics")]
        touch_test: TestScreen::default(),
    };
    screens
        .firmware
        .set_confirmed(crate::boot::confirm::is_validated());
    #[cfg(feature = "ui-animations")]
    let ui_scratch = UI_SCRATCH.init(UiScratch::new());
    let mut app = AppState::new(ScreenId::Watchface);
    // Pairing, firmware updates, and the first-boot format are system modals
    // above the screen stack; this owns which one is up and which events may
    // still reach the screen behind it.
    let mut modals = ModalState::new();
    // Bluetooth and charge arrive whatever screen is up, so the corner is fed
    // from the event stream rather than by the active screen. It keeps its
    // values while the watch sleeps, so the redraw on wake shows the state as
    // it was last reported instead of an empty corner.
    let mut status = StatusCorner::new();
    let mut power_receiver = system_power_receiver();
    let mut settings_receiver = display_settings_receiver();
    let mut wall_clock = wall_clock_receiver();
    let mut wall_clock_reference = None;
    let mut power = power_receiver.get().await;
    let mut ignore_input_until = started_at;
    let _ = screens.draw_full(
        app.active_screen(),
        &status,
        &mut Canvas::new(&mut display),
        &mut || watchdog.pet(),
    );
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
                    if let Some(reference) = wall_clock.try_changed() {
                        wall_clock_reference = Some(reference);
                    }
                    // The wall-clock reference is anchored to absolute uptime,
                    // so it must be sampled with the same base; the displayed
                    // uptime stays relative to this task's start.
                    let uptime_seconds = now.duration_since(started_at).as_secs();
                    DisplayEvent::Ui(AppEvent::Tick {
                        uptime_seconds,
                        wall_time: wall_clock_reference
                            .map(|reference| reference.wall_time_at(now.as_secs())),
                        date: wall_clock_reference
                            .map(|reference| reference.date_at(now.as_secs())),
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
                        let _ = display.wake(&mut delay);
                        // A modal outlives sleep and still owns the screen.
                        if let Some(modal) = modals.current() {
                            let _ = draw_modal(&mut Canvas::new(&mut display), modal, &mut || {
                                watchdog.pet();
                            });
                        } else {
                            if let Some(reference) = wall_clock.try_changed() {
                                wall_clock_reference = Some(reference);
                            }
                            let uptime_seconds = now.duration_since(started_at).as_secs();
                            let tick = AppEvent::Tick {
                                uptime_seconds,
                                wall_time: wall_clock_reference
                                    .map(|reference| reference.wall_time_at(now.as_secs())),
                                date: wall_clock_reference
                                    .map(|reference| reference.date_at(now.as_secs())),
                            };
                            let _ = screens.handle(app.active_screen(), tick);
                            let _ = screens.draw_full(
                                app.active_screen(),
                                &status,
                                &mut Canvas::new(&mut display),
                                &mut || watchdog.pet(),
                            );
                        }
                        backlight.set_level(settings.brightness());
                        ignore_input_until = Instant::now() + WAKE_INPUT_GUARD;
                        next_tick = Instant::now() + Duration::from_secs(1);
                    }
                    SystemPowerState::Interactive => backlight.set_level(settings.brightness()),
                    SystemPowerState::Idle => backlight.set_level(DIMMED_BRIGHTNESS),
                    SystemPowerState::Sleeping => {
                        backlight.set_level(0);
                        let _ = display.sleep(&mut delay);
                    }
                }
                continue;
            }
        };

        let status_changed = match event {
            AppEvent::BatteryUpdated(reading) => status.set_battery(reading),
            AppEvent::BleUpdated(state) => status.set_ble(state),
            _ => false,
        };

        if let AppEvent::DisplaySettingsUpdated(updated) = event {
            HEART_RATE_COMMANDS
                .send(HeartRateCommand::Configure {
                    enabled: updated.heart_rate_enabled(),
                    interval_seconds: updated.heart_rate_interval_seconds(),
                })
                .await;
            // The persisted choice arrives as a settings snapshot, at boot and
            // whenever it changes. A face swap only has to be painted when the
            // watchface is what the user is looking at; otherwise the next
            // navigation redraws it anyway.
            if screens.watchface.select(updated.watchface())
                && app.active_screen() == ScreenId::Watchface
            {
                let _ = screens.draw_full(
                    ScreenId::Watchface,
                    &status,
                    &mut Canvas::new(&mut display),
                    &mut || watchdog.pet(),
                );
            }
        }
        if matches!(event, AppEvent::HeartRateStateUpdated(_))
            && app.active_screen() != ScreenId::Watchface
        {
            let _ = screens.watchface.handle_event(event);
        }

        // System modals rank above the screen stack, so they claim the event
        // first; only `None` leaves it to the active screen.
        match modals.handle(event) {
            ModalOutcome::Show(modal) => {
                let _ = display.wake(&mut delay);
                backlight.set_level(settings.brightness());
                if renews_activity(modal) {
                    POWER_COMMANDS.send(PowerCommand::UserActivity).await;
                }
                let _ = draw_modal(&mut Canvas::new(&mut display), modal, &mut || {
                    watchdog.pet();
                });
                continue;
            }
            ModalOutcome::Suppressed => continue,
            ModalOutcome::UpdateBehind => {
                let _ = screens.handle(app.active_screen(), event);
                continue;
            }
            ModalOutcome::Dismissed { deliver } => {
                if deliver {
                    let _ = screens.handle(app.active_screen(), event);
                }
                let _ = screens.draw_full(
                    app.active_screen(),
                    &status,
                    &mut Canvas::new(&mut display),
                    &mut || watchdog.pet(),
                );
                continue;
            }
            ModalOutcome::None => {}
        }

        if power == SystemPowerState::Sleeping
            || (event.is_user_activity() && now < ignore_input_until)
        {
            continue;
        }

        // The corner redraws on its own, without the screen underneath: it
        // paints its own background, and a battery reading is no reason to
        // repaint a menu.
        if status_changed && wears_status(app.active_screen()) {
            let _ = status.draw(&mut Canvas::new(&mut display));
        }

        // Navigation resolves against the contract first. A swipe that leads
        // nowhere still belongs to the screen, which may use it for its own
        // content; a back press that leads nowhere is at the root and is simply
        // absorbed, so it never reaches a screen as content.
        let effect = match event {
            AppEvent::Swipe(direction) => match app.navigate(direction) {
                AppEffect::None => app.transition(screens.handle(app.active_screen(), event)),
                navigated => navigated,
            },
            AppEvent::BackPressed => app.back(),
            _ => app.transition(screens.handle(app.active_screen(), event)),
        };

        match effect {
            AppEffect::ApplySettings(updated) => {
                // Haptic confirmation for the activated button; a busy motor
                // drops the tick rather than stalling rendering.
                let _ = VIBRATION_COMMANDS.try_send(VibrationPattern::Tap);
                SETTINGS_COMMANDS.send(updated).await;
                let _ = screens.draw_dirty(
                    app.active_screen(),
                    &mut Canvas::new(&mut display),
                    &mut || watchdog.pet(),
                );
            }
            AppEffect::ConfirmFirmware => {
                // Making the image permanent takes effect immediately; a reset
                // no longer rolls back afterwards.
                let confirmed = crate::boot::confirm::confirm();
                info!("Firmware confirmation requested; confirmed={}", confirmed);
                screens.firmware.set_confirmed(confirmed);
                let _ = VIBRATION_COMMANDS.try_send(VibrationPattern::Double);
                let _ = screens.draw_full(
                    app.active_screen(),
                    &status,
                    &mut Canvas::new(&mut display),
                    &mut || watchdog.pet(),
                );
            }
            AppEffect::Reboot | AppEffect::RequestRollback => {
                info!("Restart requested from software");
                // The haptic tick is the acknowledgement the user gets; the
                // delay lets the motor and the RTT buffer finish before the
                // core is reset out from under them.
                let _ = VIBRATION_COMMANDS.try_send(VibrationPattern::Double);
                Timer::after_millis(250).await;
                cortex_m::peripheral::SCB::sys_reset();
            }
            AppEffect::Navigate(navigation) => {
                #[cfg(not(feature = "ui-animations"))]
                let _ = navigation;
                #[cfg(feature = "ui-animations")]
                match app.active_screen() {
                    ScreenId::Watchface => {
                        let result = draw_slide_reveal(
                            &screens.watchface,
                            &mut display,
                            ui_scratch,
                            navigation,
                            &mut || watchdog.pet(),
                        );
                        #[cfg(feature = "diagnostics")]
                        if let Ok(metrics) = result {
                            screens
                                .touch_test
                                .record_transition(navigation.direction, metrics);
                        }
                        #[cfg(not(feature = "diagnostics"))]
                        let _ = result;
                    }
                    ScreenId::Launcher => {
                        let _ = draw_slide_reveal(
                            &WithStatus::new(&screens.launcher, &status),
                            &mut display,
                            ui_scratch,
                            navigation,
                            &mut || watchdog.pet(),
                        );
                    }
                    ScreenId::DisplaySettings => {
                        let _ = draw_slide_reveal(
                            &WithStatus::new(&screens.settings, &status),
                            &mut display,
                            ui_scratch,
                            navigation,
                            &mut || watchdog.pet(),
                        );
                    }
                    ScreenId::WatchfaceSelect => {
                        let _ = draw_slide_reveal(
                            &WithStatus::new(&screens.watchface_select, &status),
                            &mut display,
                            ui_scratch,
                            navigation,
                            &mut || watchdog.pet(),
                        );
                    }
                    ScreenId::Firmware => {
                        let _ = draw_slide_reveal(
                            &WithStatus::new(&screens.firmware, &status),
                            &mut display,
                            ui_scratch,
                            navigation,
                            &mut || watchdog.pet(),
                        );
                    }
                    #[cfg(feature = "diagnostics")]
                    ScreenId::TouchTest => {
                        if let Ok(metrics) = draw_slide_reveal(
                            &WithStatus::new(&screens.touch_test, &status),
                            &mut display,
                            ui_scratch,
                            navigation,
                            &mut || watchdog.pet(),
                        ) {
                            screens
                                .touch_test
                                .record_transition(navigation.direction, metrics);
                            let _ = screens
                                .touch_test
                                .draw_metrics(&mut Canvas::new(&mut display));
                            watchdog.pet();
                        }
                    }
                }
                #[cfg(not(feature = "ui-animations"))]
                let _ = screens.draw_full(
                    app.active_screen(),
                    &status,
                    &mut Canvas::new(&mut display),
                    &mut || watchdog.pet(),
                );
            }
            AppEffect::None => {
                let _ = screens.draw_dirty(
                    app.active_screen(),
                    &mut Canvas::new(&mut display),
                    &mut || watchdog.pet(),
                );
            }
        }
    }
}
