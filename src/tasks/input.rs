use defmt::{info, warn};
use embassy_nrf::gpio::{Input, Level, Output, OutputDrive, Pull};
use embassy_time::{Delay, Duration, with_timeout};

use crate::{
    board::{buses::TouchI2c, peripherals::TouchResources},
    drivers::touch::{Cst816s, Gesture},
    ipc::{DISPLAY_SETTINGS, POWER_COMMANDS, SYSTEM_POWER, TOUCH_READY, UI_EVENTS},
};
use pineforge_state::{
    DisplaySettings, PowerCommand, SwipeDirection, SystemPowerState, TouchReport, TouchRouter,
};

/// How long a touch already in flight may go without a report before it is
/// treated as one whose lift was lost.
///
/// The controller pulses periodically while a finger is down, so this is two
/// orders of magnitude past the gap between reports. Erring long is deliberate:
/// cutting a slow drag short costs a gesture the user repeats, while recovering
/// a moment late costs nothing at all - the stale touch has to be cleared
/// before the *next* one, not before the screen is repainted.
const TOUCH_STALE_TIMEOUT: Duration = Duration::from_millis(500);

/// Owns the touch controller and publishes hardware-independent UI events.
///
/// What a report *means* is not decided here. Which travel counts as a gesture,
/// and which of a finger's reports a screen should still hear about once one
/// does, live in [`TouchRouter`] - both are rules that fail silently, and both
/// did. This task reads the controller and forwards what the router returns.
#[embassy_executor::task]
pub async fn run(resources: TouchResources, i2c: TouchI2c) {
    let mut interrupt = Input::new(resources.interrupt, Pull::Up);
    let reset = Output::new(resources.reset, Level::High, OutputDrive::Standard);
    let mut touch = Cst816s::new(i2c, reset);
    let mut delay = Delay;
    let mut router = TouchRouter::new();

    if touch.setup(&mut delay).await.is_err() {
        warn!("Touch controller setup failed");
    }
    TOUCH_READY.signal(());

    loop {
        // A touch in flight is held to a deadline; an idle panel is not. The
        // controller's interrupt is a bare low pulse with nothing latching it,
        // so a pulse that arrives while this task is anywhere else - notably
        // blocked handing the previous event to a display task busy painting a
        // transition - is gone without trace. When the lost one was the finger
        // lifting, the router keeps measuring the next swipe's travel from a
        // finger that is no longer there. Nothing in the report stream can
        // reveal that, so silence is the only evidence there is.
        if router.is_tracking() {
            if with_timeout(TOUCH_STALE_TIMEOUT, interrupt.wait_for_falling_edge())
                .await
                .is_err()
            {
                warn!("Touch went quiet; forgetting the touch in flight");
                router.lost_report();
                continue;
            }
        } else {
            interrupt.wait_for_falling_edge().await;
        }

        if let Ok(event) = touch.read_touch().await {
            info!(
                "Touch x={} y={} pressed={} gesture={:?}",
                event.x, event.y, event.touching, event.gesture
            );

            // A sleeping panel accepts only configured wake gestures. The
            // CST816S identifies taps itself, so double-tap does not have to
            // wake on the first touch just to let software count the second.
            // None of the wake touch is forwarded to the screen underneath.
            if SYSTEM_POWER.try_get() == Some(SystemPowerState::Sleeping) {
                let wake_gestures = DISPLAY_SETTINGS
                    .try_get()
                    .unwrap_or(DisplaySettings::DEFAULT)
                    .wake_gestures();
                let taps = match event.gesture {
                    Gesture::SingleTap => 1,
                    Gesture::DoubleTap => 2,
                    _ => 0,
                };
                if wake_gestures.accepts_taps(taps) {
                    let _ = POWER_COMMANDS.try_send(PowerCommand::UserActivity);
                }
                router.lost_report();
                continue;
            }

            let report = TouchReport {
                x: i32::from(event.x),
                y: i32::from(event.y),
                touching: event.touching,
                gesture: match event.gesture {
                    Gesture::SlideLeft => Some(SwipeDirection::Left),
                    Gesture::SlideRight => Some(SwipeDirection::Right),
                    Gesture::SlideUp => Some(SwipeDirection::Up),
                    Gesture::SlideDown => Some(SwipeDirection::Down),
                    _ => None,
                },
            };
            let events = router.report(report);

            // Never awaited. An activity ping is idempotent - the power service
            // only needs to know the user is there, and eight of them queued
            // say that as well as nine do. Blocking here would put this task
            // outside the interrupt wait for as long as the queue stayed full,
            // and every report arriving in that window would be lost.
            let _ = POWER_COMMANDS.try_send(PowerCommand::UserActivity);
            for event in events {
                UI_EVENTS.send(event).await;
            }
        } else {
            router.lost_report();
            warn!("Touch report read failed");
        }
    }
}
