use defmt::{error, info};
use embassy_futures::select::{Either, Either4, select, select4};
use embassy_nrf::gpio::{Level, Output, OutputDrive};
use embassy_time::{Delay, Duration, Instant, Timer};
use mipidsi::interface::SpiInterface;
use mipidsi::options::{ColorInversion, Orientation};
use static_cell::StaticCell;

use crate::ipc::HEART_RATE_COMMANDS;
use crate::{
    board::{buses::DisplaySpi, peripherals::DisplayResources, pins},
    boot::watchdog::BootloaderWatchdog,
    drivers::backlight::Backlight,
    ipc::{
        NOTIFICATIONS, POWER_COMMANDS, SETTINGS_COMMANDS, UI_EVENTS, VIBRATION_COMMANDS,
        display_settings_receiver, system_power_receiver, wall_clock_receiver,
    },
};
use pineforge_state::{
    AppEffect, AppEvent, AppState, DisplaySettings, HeartRateCommand, ModalOutcome, ModalState,
    Notification, PowerCommand, ScreenId, SystemPowerState, VibrationPattern, panel_backlight,
};
use pineforge_ui::{
    about::BuildInfo,
    canvas::Canvas,
    modal,
    registry::Screens,
    screen::Screen,
    status::{StatusCorner, wears_status},
};
#[cfg(feature = "ui-animations")]
use pineforge_ui::{scratch::UiScratch, transition::draw_slide_reveal};

static DISPLAY_BUFFER: StaticCell<[u8; 512]> = StaticCell::new();
#[cfg(feature = "ui-animations")]
static UI_SCRATCH: StaticCell<UiScratch> = StaticCell::new();

/// Backstop for how long input is disowned after waking.
///
/// What actually has to be swallowed is the *rest of the touch that did the
/// waking* - its press woke the watch and its lift must not then activate
/// whatever it landed on. That end is recognised directly, by the release the
/// router always emits, so this timer only covers the case where no release is
/// coming: a wake from the side button leaves no touch in flight to end.
///
/// It used to be the only rule, at half a second, and half a second is a long
/// time to be deaf. A swipe is a thing people do immediately after lighting up
/// the screen, and every one of them inside that window was dropped.
const WAKE_INPUT_GUARD: Duration = Duration::from_millis(250);

/// Whether the lamp app is showing and lit.
///
/// The one question the backlight decision has to put to a screen. Every other
/// input to it - the power state, the settings - the task already holds.
fn lamp_lit(app: &AppState, screens: &Screens) -> bool {
    app.active_screen() == ScreenId::Flashlight && screens.flashlight.is_lit()
}

enum DisplayEvent {
    Ui(AppEvent),
    Power(SystemPowerState),
    Settings(DisplaySettings),
}

/// Puts an arriving notification in the inbox and reports what is now pending.
///
/// Filed where it is received rather than carried through [`DisplayEvent`]: a
/// notification is well over a hundred bytes of text buffers, and a variant
/// holding one would add that to the task's future for the whole life of the
/// firmware. Moving it straight into the inbox keeps it on the stack instead,
/// and the event that comes back out carries only the tally.
fn file(screens: &mut Screens, notification: Notification) -> DisplayEvent {
    DisplayEvent::Ui(AppEvent::NotificationsChanged(
        screens.notifications.file(notification),
    ))
}

/// The panel, once it is up and pointed the right way round.
type Panel = mipidsi::Display<
    SpiInterface<'static, DisplaySpi, Output<'static>>,
    mipidsi::models::ST7789,
    Output<'static>,
>;

/// Brings the panel up, or reports that this watch has to run blind.
///
/// `None` rather than a panic, and that is the whole point of the return type.
/// Panicking here traps the core, nothing pets the watchdog from the trap, and
/// seven seconds later the bootloader starts the same image into the same
/// failure - forever, with no window in which anything can be fixed. Ending the
/// display task instead leaves every other task running, so the watch still
/// advertises and can still be recovered over DFU. A blind watch is bad; a
/// bootlooping watch is a brick.
fn init_panel(
    dc: Output<'static>,
    reset: Output<'static>,
    spi: DisplaySpi,
    delay: &mut Delay,
) -> Option<Panel> {
    let interface = SpiInterface::new(spi, dc, DISPLAY_BUFFER.init([0; 512]));
    let panel = mipidsi::Builder::new(mipidsi::models::ST7789, interface)
        .display_size(pins::DISPLAY_WIDTH, pins::DISPLAY_HEIGHT)
        .invert_colors(ColorInversion::Inverted)
        .reset_pin(reset)
        .init(delay);

    let mut panel = match panel {
        Ok(panel) => panel,
        Err(error) => {
            error!(
                "Display init failed, continuing without a screen: {}",
                defmt::Debug2Format(&error)
            );
            return None;
        }
    };
    if let Err(error) = panel.set_orientation(Orientation::new()) {
        error!(
            "Display orientation rejected, continuing without a screen: {}",
            defmt::Debug2Format(&error)
        );
        return None;
    }
    Some(panel)
}

/// Owns the display and backlight and renders events received from the UI bus.
// Keeping the event loop in one function makes peripheral ownership explicit
// for this single-owner task; only the panel bring-up, which shares nothing
// with the loop, is lifted out above.
#[allow(clippy::too_many_lines)]
#[embassy_executor::task]
pub async fn run(resources: DisplayResources, spi: DisplaySpi, watchdog: BootloaderWatchdog) {
    let mut backlight = Backlight::new(
        Output::new(resources.backlight_low, Level::High, OutputDrive::Standard),
        Output::new(resources.backlight_mid, Level::High, OutputDrive::Standard),
        Output::new(resources.backlight_high, Level::High, OutputDrive::Standard),
    );
    backlight.set_level(1);

    let mut delay = Delay;
    let Some(mut display) = init_panel(
        Output::new(resources.dc, Level::Low, OutputDrive::Standard),
        Output::new(resources.reset, Level::Low, OutputDrive::Standard),
        spi,
        &mut delay,
    ) else {
        backlight.set_level(0);
        return;
    };
    watchdog.pet();
    let mut settings = DisplaySettings::DEFAULT;
    // Interactive and unlit by construction: nothing has been navigated to yet.
    backlight.set_level(panel_backlight(
        SystemPowerState::Interactive,
        false,
        settings,
    ));

    let started_at = Instant::now();
    let mut next_tick = started_at + Duration::from_secs(1);
    let mut screens = Screens::new();
    screens
        .firmware
        .set_confirmed(crate::boot::confirm::is_validated());
    // Which build this is, from the one crate that can know. `pineforge-ui` has
    // its own package version and compiling it in there named that instead -
    // which is how a watch running 0.2.1 came to report 0.1.0.
    let build = BuildInfo {
        version: env!("PINEFORGE_VERSION"),
        commit: env!("PINEFORGE_COMMIT"),
        date: env!("PINEFORGE_DATE"),
    };
    screens.about.set_build(build);
    screens.firmware.set_version(build.version);
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
    // The one piece of corner state that is not a reading: it is settled at
    // boot and only ever moves when the user confirms, below.
    status.set_unconfirmed(!crate::boot::confirm::is_validated());
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
    backlight.set_level(panel_backlight(power, lamp_lit(&app, &screens), settings));
    if power == SystemPowerState::Sleeping {
        let _ = display.sleep(&mut delay);
    }

    loop {
        // A notification is received even while asleep: it has to be in the
        // inbox by the time the watch is woken, and leaving it queued would
        // block the one behind it.
        let display_event = if power == SystemPowerState::Sleeping {
            match select4(
                UI_EVENTS.receive(),
                power_receiver.changed(),
                settings_receiver.changed(),
                NOTIFICATIONS.receive(),
            )
            .await
            {
                Either4::First(event) => DisplayEvent::Ui(event),
                Either4::Second(state) => DisplayEvent::Power(state),
                Either4::Third(snapshot) => DisplayEvent::Settings(snapshot),
                Either4::Fourth(notification) => file(&mut screens, notification),
            }
        } else {
            // Nested because there is no `select5`, and the notification is the
            // input least entangled with the other four.
            match select(
                select4(
                    UI_EVENTS.receive(),
                    Timer::at(next_tick),
                    power_receiver.changed(),
                    settings_receiver.changed(),
                ),
                NOTIFICATIONS.receive(),
            )
            .await
            {
                Either::Second(notification) => file(&mut screens, notification),
                Either::First(Either4::First(event)) => DisplayEvent::Ui(event),
                Either::First(Either4::Second(())) => {
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
                Either::First(Either4::Third(state)) => DisplayEvent::Power(state),
                Either::First(Either4::Fourth(snapshot)) => DisplayEvent::Settings(snapshot),
            }
        };
        let now = Instant::now();

        let event = match display_event {
            DisplayEvent::Ui(event) => event,
            DisplayEvent::Settings(snapshot) => {
                settings = snapshot;
                backlight.set_level(panel_backlight(power, lamp_lit(&app, &screens), settings));
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
                        if let Some(showing) = modals.current() {
                            let _ =
                                modal::draw(&mut Canvas::new(&mut display), showing, &mut || {
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
                            screens.enter(app.active_screen(), settings);
                            let _ = screens.draw_full(
                                app.active_screen(),
                                &status,
                                &mut Canvas::new(&mut display),
                                &mut || watchdog.pet(),
                            );
                        }
                        backlight.set_level(panel_backlight(
                            power,
                            lamp_lit(&app, &screens),
                            settings,
                        ));
                        ignore_input_until = Instant::now() + WAKE_INPUT_GUARD;
                        next_tick = Instant::now() + Duration::from_secs(1);
                    }
                    SystemPowerState::Interactive | SystemPowerState::Idle => backlight
                        .set_level(panel_backlight(power, lamp_lit(&app, &screens), settings)),
                    SystemPowerState::Sleeping => {
                        backlight.set_level(0);
                        let _ = display.sleep(&mut delay);
                    }
                }
                continue;
            }
        };

        // ---- Ingest. Every model this task owns absorbs the event here,
        // before anything below decides whether to paint. Nothing in this
        // section may be skipped: an event is consumed from the channel
        // whatever happens next, so a model that misses one never sees it. ----

        let status_changed = match event {
            AppEvent::BatteryUpdated(reading) => status.set_battery(reading),
            AppEvent::BleUpdated(state) => status.set_ble(state),
            _ => false,
        };

        // A reading is a fact about the watch rather than something addressed
        // to whichever screen is up, and the face is the only screen that keeps
        // one. So it is applied once, here, instead of being routed through the
        // active screen.
        //
        // This is what used to be lost. The dispatch to the active screen sits
        // below the sleep gate, and the one path that fed the face directly
        // covered two event kinds and only while the face was *not* showing.
        // So the ordinary case - face showing, watch asleep - dropped every
        // reading it received. With sleep at twenty seconds and a battery
        // sample every ten minutes, that was nearly all of them: the percentage
        // sat where it was at boot, and a charger on the pad never turned `BAT`
        // into `CHG`, because that reading arrives exactly when the panel is
        // off.
        let reading_moved = event.is_reading() && {
            let _ = screens.watchface.handle_event(event);
            screens.watchface.moved()
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
        // The stack's high-water mark, on the one screen that can show it. A
        // watch with no debugger attached has no other way to read it, and the
        // reading is what the RAM budget is meant to be set from.
        #[cfg(feature = "diagnostics")]
        if matches!(event, AppEvent::Tick { .. }) {
            screens
                .touch_test
                .set_stack(crate::boot::stack::used(), crate::boot::stack::capacity());
        }

        // System modals rank above the screen stack, so they claim the event
        // first; only `None` leaves it to the active screen.
        let modal_outcome = modals.handle(event);
        match modal_outcome {
            ModalOutcome::Show(showing) | ModalOutcome::Refresh(showing) => {
                let _ = display.wake(&mut delay);
                backlight.set_level(panel_backlight(
                    SystemPowerState::Interactive,
                    lamp_lit(&app, &screens),
                    settings,
                ));
                if showing.renews_activity() {
                    POWER_COMMANDS.send(PowerCommand::UserActivity).await;
                }
                // A value that moved repaints what moved; a prompt that is new
                // on the panel is drawn whole.
                let canvas = &mut Canvas::new(&mut display);
                let keep_alive = &mut || watchdog.pet();
                let _ = if matches!(modal_outcome, ModalOutcome::Refresh(_)) {
                    modal::refresh(canvas, showing, keep_alive)
                } else {
                    modal::draw(canvas, showing, keep_alive)
                };
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
                // The screen coming back may be a settings leaf that missed an
                // update while the modal covered it. Re-entering costs nothing
                // and keeps the leaf's record current by construction rather
                // than by an argument about which events a modal can hide.
                screens.enter(app.active_screen(), settings);
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

        // ---- Render. From here down every path is about the panel, and every
        // one of them may be skipped: a dark screen is not repainted, and an
        // event that reaches no screen paints nothing. The models above are
        // already current whichever way this goes. ----

        if power == SystemPowerState::Sleeping {
            continue;
        }

        // The lamp holds the watch awake for as long as it is open, not only
        // while it is lit - a light that went out on its own timeout partway
        // through being used would be worse than no light. `InfiniTime` takes
        // the same lock for the lifetime of its flashlight screen.
        //
        // A tick arrives every second while awake, so renewing here keeps the
        // idle deadline permanently out of reach. Never awaited: the request is
        // idempotent, and a full queue already means activity is being reported.
        if app.active_screen().keeps_awake() {
            let _ = POWER_COMMANDS.try_send(PowerCommand::UserActivity);
        }

        if now < ignore_input_until {
            // The lift that ends the waking touch is what the guard was really
            // waiting for, so seeing it retires the guard early instead of
            // staying deaf for the rest of the window. The router emits this
            // release for every touch, gesture or not, so there is always one
            // to see when a touch is what woke the watch.
            if matches!(event, AppEvent::Touch { pressed: false, .. }) {
                ignore_input_until = now;
            }
            if event.is_user_activity() {
                continue;
            }
        }

        // The corner redraws on its own, without the screen underneath: it
        // paints its own background, and a battery reading is no reason to
        // repaint a menu.
        if status_changed && wears_status(app.active_screen()) {
            let _ = status.draw(&mut Canvas::new(&mut display));
        }

        // A reading never navigates, and the face already took it above. When
        // the face is what is showing, the repaint it earned is all that is
        // left to do - and handing it to the screen a second time would find
        // nothing moved and cancel exactly that repaint.
        //
        // Any other screen keeps the ordinary path below. A notification tally
        // is a reading to the face and a reason to repaint to the inbox, and
        // only the screen itself can know that.
        if event.is_reading() && app.active_screen() == ScreenId::Watchface {
            if reading_moved {
                let _ = screens.draw_dirty(
                    ScreenId::Watchface,
                    &mut Canvas::new(&mut display),
                    &mut || watchdog.pet(),
                );
            }
            continue;
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
                // The corner carries the same fact, so it clears here rather
                // than at the next boot. The full repaint below takes it.
                status.set_unconfirmed(!confirmed);
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
                // Before anything paints: the screen now on top may be a
                // settings leaf, and a leaf must edit the record that is
                // current rather than one it kept from an earlier visit.
                screens.enter(app.active_screen(), settings);
                #[cfg(not(feature = "ui-animations"))]
                let _ = navigation;
                #[cfg(feature = "ui-animations")]
                {
                    let active = app.active_screen();
                    // The registry lends the surface the panel would show,
                    // corner and all, and the borrow ends with the statement so
                    // the metrics below can take the screens mutably.
                    let result = screens.surface(active, &status, &mut |surface| {
                        draw_slide_reveal(
                            surface,
                            &mut display,
                            ui_scratch,
                            navigation,
                            &mut || watchdog.pet(),
                        )
                    });
                    #[cfg(feature = "diagnostics")]
                    if let Ok(metrics) = result {
                        screens
                            .touch_test
                            .record_transition(navigation.direction, metrics);
                        // The screen that shows the numbers is the one that has
                        // to be told they changed.
                        if active == ScreenId::TouchTest {
                            let _ = screens
                                .touch_test
                                .draw_metrics(&mut Canvas::new(&mut display));
                            watchdog.pet();
                        }
                    }
                    #[cfg(not(feature = "diagnostics"))]
                    let _ = result;
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

        // Settled after the effect rather than inside it, because two different
        // things reach here: navigating onto or off the lamp, and the tap that
        // lights it. Three GPIO writes is cheaper than an argument about which
        // arm owns the backlight.
        backlight.set_level(panel_backlight(power, lamp_lit(&app, &screens), settings));
    }
}
