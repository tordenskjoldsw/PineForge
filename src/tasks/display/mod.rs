use defmt::{error, info};
use embassy_futures::select::{Either3, Either4, select3, select4};
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
        MUSIC_CONTROL, NOTIFICATIONS, POWER_COMMANDS, SETTINGS_COMMANDS, UI_EVENTS,
        VIBRATION_ALARM, VIBRATION_COMMANDS, VibrationAlarmSignal, WALL_CLOCK,
        display_settings_receiver, music_state_receiver, system_power_receiver,
        wall_clock_receiver,
    },
};
#[cfg(feature = "ui-animations")]
use pineforge_state::Navigation;
use pineforge_state::{
    AppEffect, AppEvent, AppState, ClockSnapshot, DisplaySettings, HeartRateCommand, Modal,
    ModalOutcome, ModalState, MusicControl, MusicState, Notification, PowerCommand, ScreenId,
    SystemPowerState, TimerOutcome, VibrationPattern, panel_backlight,
};
use pineforge_ui::{
    about::BuildInfo,
    canvas::Canvas,
    modal,
    registry::Screens,
    status::{StatusCorner, wears_status},
    theme,
};

mod panel;

use panel::ScrollPanel;
#[cfg(feature = "ui-animations")]
use pineforge_ui::{scratch::UiScratch, transition::draw_slide_reveal};

/// Bytes the display interface gathers before it hands them to the SPI bus.
///
/// Eight `EasyDMA` transfers, and the eight is what the number is for. The
/// nRF52832 caps a transfer at 255 bytes, so embassy splits anything longer -
/// but the expensive part is not the split. Embassy's errata-109 workaround
/// runs once per *write call*, not per chunk: a zero-length start, an
/// interrupt, and a spin on an atomic before the real data moves. `InfiniTime`
/// pays none of this, which is most of why our bus runs at two thirds of its
/// nominal rate.
///
/// So the lever is how many write calls a frame takes. At 512 bytes a full
/// panel took 225 of them; at 2040 it takes 57. Going from 512 to 510 was
/// tried first, on the theory that the two-byte remainder of 255 + 255 + 2 was
/// the cost - it changed the call count by one and moved nothing.
const DISPLAY_BUFFER_BYTES: usize = 8 * 255;
static DISPLAY_BUFFER: StaticCell<[u8; DISPLAY_BUFFER_BYTES]> = StaticCell::new();
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

/// The FORGE stopwatch shows tenths; other screens only need the one-second
/// cadence used by the wall clock and retained readings.
const STOPWATCH_REFRESH: Duration = Duration::from_millis(100);

fn refresh_interval(active: ScreenId) -> Duration {
    if active == ScreenId::Stopwatch {
        STOPWATCH_REFRESH
    } else {
        Duration::from_secs(1)
    }
}

/// Absolute Embassy deadline corresponding to the timer's task-relative
/// monotonic value. `MAX` is a pending timer without allocating another
/// optional future when no countdown is running.
fn timer_deadline(screens: &Screens, started_at: Instant) -> Instant {
    screens
        .timer
        .deadline_millis()
        .map_or(Instant::MAX, |millis| {
            started_at + Duration::from_millis(millis)
        })
}

/// Advances the retained countdown and raises its one-shot alarm event.
fn expire_timer(screens: &mut Screens, started_at: Instant, now: Instant) -> Option<AppEvent> {
    if matches!(
        screens
            .timer
            .observe(now.duration_since(started_at).as_millis()),
        TimerOutcome::Expired
    ) {
        VIBRATION_ALARM.signal(VibrationAlarmSignal::Start);
        Some(AppEvent::TimerExpired)
    } else {
        None
    }
}

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

/// Puts what the phone is playing in front of the screen that shows it, and
/// reports only that it moved.
///
/// Filed here for the same reason a notification is: the record is two 40-byte
/// text buffers, and carrying one through the event channel would cost that
/// against the channel's whole capacity for something that changes a few times
/// an hour.
///
/// The uptime is taken here rather than by the screen, and it has to be this
/// base - the one the tick uses. The BLE task cannot supply it: its clock reads
/// from a different zero, and the elapsed time is the difference between the
/// two readings.
fn play(screens: &mut Screens, state: &MusicState, started_at: Instant) -> DisplayEvent {
    let uptime_seconds = Instant::now().duration_since(started_at).as_secs();
    let _ = screens.music.apply(state, uptime_seconds);
    DisplayEvent::Ui(AppEvent::MusicUpdated)
}

/// The panel, once it is up and pointed the right way round.
///
/// Declared as the whole of the controller's frame memory rather than as the
/// 240x240 that is fitted, which is what lets [`ScrollPanel`] reach the rows the
/// panel is not showing. Nothing else may address them: the wrapper is what
/// every other part of this task draws through, and it presents the watch.
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
) -> Option<ScrollPanel<Panel>> {
    let interface = SpiInterface::new(spi, dc, DISPLAY_BUFFER.init([0; DISPLAY_BUFFER_BYTES]));
    let panel = mipidsi::Builder::new(mipidsi::models::ST7789, interface)
        .display_size(pins::DISPLAY_WIDTH, panel::MEMORY_ROWS)
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
    // Stated rather than inherited: the window has to be free to travel the
    // whole of frame memory, and a reset default is a poor thing to rest an
    // animation on.
    if let Err(error) = panel.set_vertical_scroll_region(0, 0) {
        error!(
            "Display scroll region rejected, continuing without a screen: {}",
            defmt::Debug2Format(&error)
        );
        return None;
    }
    let mut panel = ScrollPanel::new(panel);
    // The staging rows come up holding whatever the controller powered on with,
    // and the first slide would carry that into view ahead of the screen it is
    // bringing in. Paid once, at boot, for 80 rows.
    if let Err(error) = panel.clear_memory(theme::BACKGROUND) {
        error!(
            "Display clear failed, continuing without a screen: {}",
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
    let image_confirmed = crate::boot::confirm::is_validated();
    screens.firmware.set_confirmed(image_confirmed);
    screens.about.set_image(image_confirmed);
    // Which build this is, from the one crate that can know. `pineforge-ui` has
    // its own package version and compiling it in there named that instead -
    // which is how a watch running 0.2.1 came to report 0.1.0.
    let build = BuildInfo {
        version: env!("PINEFORGE_VERSION"),
        commit: env!("PINEFORGE_COMMIT"),
        date: env!("PINEFORGE_DATE"),
        bootloader: "MCUBOOT",
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
    let mut music = music_state_receiver();
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
        let _ = display.inner().sleep(&mut delay);
    }

    loop {
        // A notification is received even while asleep: it has to be in the
        // inbox by the time the watch is woken, and leaving it queued would
        // block the one behind it.
        let display_event = if power == SystemPowerState::Sleeping {
            match select3(
                select4(
                    UI_EVENTS.receive(),
                    power_receiver.changed(),
                    settings_receiver.changed(),
                    NOTIFICATIONS.receive(),
                ),
                music.changed(),
                Timer::at(timer_deadline(&screens, started_at)),
            )
            .await
            {
                Either3::Second(state) => play(&mut screens, &state, started_at),
                Either3::Third(()) => {
                    let Some(event) = expire_timer(&mut screens, started_at, Instant::now()) else {
                        continue;
                    };
                    DisplayEvent::Ui(event)
                }
                Either3::First(Either4::First(event)) => DisplayEvent::Ui(event),
                Either3::First(Either4::Second(state)) => DisplayEvent::Power(state),
                Either3::First(Either4::Third(snapshot)) => DisplayEvent::Settings(snapshot),
                Either3::First(Either4::Fourth(notification)) => file(&mut screens, notification),
            }
        } else {
            // Nested because there is no `select6`, and the notification and
            // the music record are the inputs least entangled with the other
            // four - both are filed straight into the screen that holds them.
            let scheduled = next_tick.min(timer_deadline(&screens, started_at));
            match select3(
                select4(
                    UI_EVENTS.receive(),
                    Timer::at(scheduled),
                    power_receiver.changed(),
                    settings_receiver.changed(),
                ),
                NOTIFICATIONS.receive(),
                music.changed(),
            )
            .await
            {
                Either3::Second(notification) => file(&mut screens, notification),
                Either3::Third(state) => play(&mut screens, &state, started_at),
                Either3::First(Either4::First(event)) => DisplayEvent::Ui(event),
                Either3::First(Either4::Second(())) => {
                    let now = Instant::now();
                    if timer_deadline(&screens, started_at) <= now {
                        let Some(event) = expire_timer(&mut screens, started_at, now) else {
                            continue;
                        };
                        DisplayEvent::Ui(event)
                    } else {
                        let interval = refresh_interval(app.active_screen());
                        while next_tick <= now {
                            next_tick += interval;
                        }
                        if app.active_screen() == ScreenId::Stopwatch {
                            DisplayEvent::Ui(AppEvent::StopwatchTick(
                                now.duration_since(started_at).as_millis(),
                            ))
                        } else if app.active_screen() == ScreenId::Timer {
                            DisplayEvent::Ui(AppEvent::TimerTick(
                                now.duration_since(started_at).as_millis(),
                            ))
                        } else {
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
                    }
                }
                Either3::First(Either4::Third(state)) => DisplayEvent::Power(state),
                Either3::First(Either4::Fourth(snapshot)) => DisplayEvent::Settings(snapshot),
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
                        let _ = display.inner().wake(&mut delay);
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
                            let uptime_millis = now.duration_since(started_at).as_millis();
                            let tick = if app.active_screen() == ScreenId::Stopwatch {
                                AppEvent::StopwatchTick(uptime_millis)
                            } else if app.active_screen() == ScreenId::Timer {
                                if let Some(event) = expire_timer(&mut screens, started_at, now) {
                                    let _ = UI_EVENTS.try_send(event);
                                }
                                AppEvent::TimerTick(uptime_millis)
                            } else {
                                let uptime_seconds = now.duration_since(started_at).as_secs();
                                AppEvent::Tick {
                                    uptime_seconds,
                                    wall_time: wall_clock_reference
                                        .map(|reference| reference.wall_time_at(now.as_secs())),
                                    date: wall_clock_reference
                                        .map(|reference| reference.date_at(now.as_secs())),
                                }
                            };
                            let _ = screens.handle(app.active_screen(), tick);
                            screens.enter(app.active_screen(), settings);
                            // The panel kept its picture: sleeping the ST7789
                            // stops its scan, not its memory, so what was on it
                            // when the watch went dark is still on it now.
                            // Waking therefore owes only what moved in the
                            // meantime - the clock, and whatever readings
                            // arrived while nothing was painting them.
                            // `InfiniTime` does not repaint on wake at all.
                            //
                            // Only the face is taken this way. Every other
                            // screen was just handed the current settings by
                            // `enter` above, and a leaf that re-pointed itself
                            // has no dirty region to report - so those keep the
                            // full repaint, which is also the rarer case: the
                            // watch sleeps on its face.
                            let _ = if app.active_screen() == ScreenId::Watchface {
                                screens.draw_dirty(
                                    app.active_screen(),
                                    &mut Canvas::new(&mut display),
                                    &mut || watchdog.pet(),
                                )
                            } else {
                                screens.draw_full(
                                    app.active_screen(),
                                    &status,
                                    &mut Canvas::new(&mut display),
                                    &mut || watchdog.pet(),
                                )
                            };
                        }
                        backlight.set_level(panel_backlight(
                            power,
                            lamp_lit(&app, &screens),
                            settings,
                        ));
                        ignore_input_until = Instant::now() + WAKE_INPUT_GUARD;
                        next_tick = Instant::now() + refresh_interval(app.active_screen());
                    }
                    SystemPowerState::Interactive | SystemPowerState::Idle => backlight
                        .set_level(panel_backlight(power, lamp_lit(&app, &screens), settings)),
                    SystemPowerState::Sleeping => {
                        backlight.set_level(0);
                        let _ = display.inner().sleep(&mut delay);
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
        let reading_moved = event.is_reading() && screens.absorb(app.active_screen(), event);

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
        let timer_alarm_showing = matches!(modals.current(), Some(Modal::TimerExpired));
        let modal_outcome = modals.handle(event);
        if timer_alarm_showing && matches!(modal_outcome, ModalOutcome::Dismissed { .. }) {
            VIBRATION_ALARM.signal(VibrationAlarmSignal::Cancel);
        }
        match modal_outcome {
            ModalOutcome::Show(showing) | ModalOutcome::Refresh(showing) => {
                let _ = display.inner().wake(&mut delay);
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
        if event.is_reading() && Screens::holds_readings(app.active_screen()) {
            if reading_moved {
                let _ = screens.draw_dirty(
                    app.active_screen(),
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
            // Navigation resolves first, with one exception it has to ask about:
            // a screen that pages along the axis it was entered on claims the
            // gesture while it still has a page that way. Nothing else answers
            // yes, and a screen that does still gives the gesture up at the end
            // of its pages - which is where it becomes the way out.
            AppEvent::Swipe(direction) if screens.claims(app.active_screen(), direction) => {
                app.transition(screens.handle(app.active_screen(), event))
            }
            AppEvent::Swipe(direction) => match app.navigate(direction) {
                AppEffect::None => app.transition(screens.handle(app.active_screen(), event)),
                navigated => navigated,
            },
            // At the root there is no screen to leave, so the press means the
            // other thing a watch button means: put the panel out now rather
            // than waiting out the timeout. Reached only when the watch was
            // already awake - the guard above swallows the press that wakes it,
            // so the button cannot turn the screen off in the act of turning it
            // on.
            AppEvent::BackPressed => match app.back() {
                AppEffect::None => {
                    let _ = POWER_COMMANDS.try_send(PowerCommand::SleepNow);
                    AppEffect::None
                }
                left => left,
            },
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
                screens.about.set_image(confirmed);
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
            AppEffect::MeasureHeartRate | AppEffect::StopHeartRate => {
                // The service owns the sensor and decides what a request means
                // while one is already running; this only forwards it.
                let command = if matches!(effect, AppEffect::MeasureHeartRate) {
                    HeartRateCommand::MeasureNow
                } else {
                    HeartRateCommand::Stop
                };
                HEART_RATE_COMMANDS.send(command).await;
                let _ = VIBRATION_COMMANDS.try_send(VibrationPattern::Tap);
            }
            AppEffect::MusicControl(control) => {
                // The phone owns the player; the watch only asks. A full queue
                // means the radio has not sent the last request yet, and
                // dropping one is a button that did nothing - recoverable by
                // pressing it again, where stalling the repaint here would not
                // be.
                let _ = MUSIC_CONTROL.try_send(control);
                let _ = VIBRATION_COMMANDS.try_send(VibrationPattern::Tap);
                let _ = screens.draw_dirty(
                    app.active_screen(),
                    &mut Canvas::new(&mut display),
                    &mut || watchdog.pet(),
                );
            }
            AppEffect::StopwatchControl(control) => {
                screens
                    .stopwatch
                    .control(control, now.duration_since(started_at).as_millis());
                let _ = VIBRATION_COMMANDS.try_send(VibrationPattern::Tap);
                let _ = screens.draw_dirty(
                    app.active_screen(),
                    &mut Canvas::new(&mut display),
                    &mut || watchdog.pet(),
                );
            }
            AppEffect::TimerControl(control) => {
                let outcome = screens
                    .timer
                    .control(control, now.duration_since(started_at).as_millis());
                if matches!(outcome, TimerOutcome::Expired) {
                    VIBRATION_ALARM.signal(VibrationAlarmSignal::Start);
                    let _ = UI_EVENTS.try_send(AppEvent::TimerExpired);
                } else {
                    let _ = VIBRATION_COMMANDS.try_send(VibrationPattern::Tap);
                }
                // Starting aligns the first decrement with the touch that set
                // the deadline; pausing or cancelling drops the old cadence.
                next_tick = now + refresh_interval(app.active_screen());
                let _ = screens.draw_dirty(
                    app.active_screen(),
                    &mut Canvas::new(&mut display),
                    &mut || watchdog.pet(),
                );
            }
            AppEffect::SetTime(time) => {
                let snapshot = wall_clock_reference.map_or(ClockSnapshot::DEFAULT, |reference| {
                    ClockSnapshot::from_reference(reference, now.as_secs())
                });
                let reference = ClockSnapshot::new(snapshot.date, time).reference_at(now.as_secs());
                wall_clock_reference = Some(reference);
                WALL_CLOCK.sender().send(reference);
                let _ = VIBRATION_COMMANDS.try_send(VibrationPattern::Tap);
                let _ = screens.draw_dirty(
                    app.active_screen(),
                    &mut Canvas::new(&mut display),
                    &mut || watchdog.pet(),
                );
            }
            AppEffect::SetDate(date) => {
                let snapshot = wall_clock_reference.map_or(ClockSnapshot::DEFAULT, |reference| {
                    ClockSnapshot::from_reference(reference, now.as_secs())
                });
                let reference = ClockSnapshot::new(date, snapshot.time).reference_at(now.as_secs());
                wall_clock_reference = Some(reference);
                WALL_CLOCK.sender().send(reference);
                let _ = VIBRATION_COMMANDS.try_send(VibrationPattern::Tap);
                let _ = screens.draw_dirty(
                    app.active_screen(),
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
                if let Some(reference) = wall_clock.try_changed() {
                    wall_clock_reference = Some(reference);
                }
                let snapshot = wall_clock_reference
                    .map(|reference| ClockSnapshot::from_reference(reference, now.as_secs()));
                screens.enter_clock(
                    app.active_screen(),
                    snapshot.map(|value| value.time),
                    snapshot.map(|value| value.date),
                );
                screens.enter(app.active_screen(), settings);
                if app.active_screen() == ScreenId::Stopwatch {
                    let _ = screens.handle(
                        ScreenId::Stopwatch,
                        AppEvent::StopwatchTick(now.duration_since(started_at).as_millis()),
                    );
                } else if app.active_screen() == ScreenId::Timer
                    && let Some(event) = expire_timer(&mut screens, started_at, now)
                {
                    let _ = UI_EVENTS.try_send(event);
                }
                next_tick = now + refresh_interval(app.active_screen());
                // Asks what is playing, the way `InfiniTime` asks it. Nothing
                // may be expected of the answer: Gadgetbridge drops this event
                // rather than replying to it, so the screen fills in when the
                // phone's media session next changes and not before. It is
                // sent because it is the protocol's own question and because a
                // companion that does answer it costs nothing to support.
                //
                // It sits here rather than in `enter` because `enter` returns
                // nothing and is called on every navigation; widening it for
                // one screen would be the larger change.
                if app.active_screen() == ScreenId::Music {
                    let _ = MUSIC_CONTROL.try_send(MusicControl::Open);
                }
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
                    // A transition composes through `surface` rather than
                    // `draw_full`, so it is the one paint that has to say so
                    // itself. Without this, sliding onto the face would leave
                    // every reading it just drew still looking owed.
                    screens.painted();
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
            AppEffect::PageTurn(motion) => {
                // The stack has not moved, so nothing is entered and no screen
                // is left; only what one screen shows has changed. It is drawn
                // like a navigation because that is what it looks like, and
                // because a page that slides in is the whole point of paging
                // along the axis the panel can slide on.
                #[cfg(not(feature = "ui-animations"))]
                let _ = motion;
                #[cfg(feature = "ui-animations")]
                {
                    let active = app.active_screen();
                    let result = screens.surface(active, &status, &mut |surface| {
                        draw_slide_reveal(
                            surface,
                            &mut display,
                            ui_scratch,
                            Navigation::forward(motion),
                            &mut || watchdog.pet(),
                        )
                    });
                    // Composed through `surface` rather than `draw_dirty`, so
                    // the readings it drew have to be marked as shown here.
                    screens.painted();
                    let _ = result;
                }
                #[cfg(not(feature = "ui-animations"))]
                let _ = screens.draw_dirty(
                    app.active_screen(),
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
