use defmt::{info, warn};
use embassy_nrf::gpio::{Input, Level, Output, OutputDrive, Pull};
use embassy_time::Delay;

use crate::{
    board::{buses::TouchI2c, peripherals::TouchResources},
    drivers::touch::{Cst816s, Gesture},
    services::events::{POWER_COMMANDS, TOUCH_READY, UI_EVENTS},
};
use pineforge_state::{AppEvent, PowerCommand, SwipeDirection, SwipeRecognizer};

/// Owns the touch controller and publishes hardware-independent UI events.
#[embassy_executor::task]
pub async fn run(resources: TouchResources, i2c: TouchI2c) {
    let mut interrupt = Input::new(resources.interrupt, Pull::Up);
    let reset = Output::new(resources.reset, Level::High, OutputDrive::Standard);
    let mut touch = Cst816s::new(i2c, reset);
    let mut delay = Delay;
    let mut swipe_recognizer = SwipeRecognizer::new();
    // Set once a gesture has claimed the touch in flight, so the rest of that
    // finger's reports stay off the screen until it lifts.
    let mut gesture_claimed = false;

    if touch.setup(&mut delay).await.is_err() {
        warn!("Touch controller setup failed");
    }
    TOUCH_READY.signal(());

    loop {
        interrupt.wait_for_falling_edge().await;

        if let Ok(event) = touch.read_touch().await {
            info!(
                "Touch x={} y={} pressed={} gesture={:?}",
                event.x, event.y, event.touching, event.gesture
            );
            let controller_gesture = match event.gesture {
                Gesture::SlideLeft => Some(SwipeDirection::Left),
                Gesture::SlideRight => Some(SwipeDirection::Right),
                Gesture::SlideUp => Some(SwipeDirection::Up),
                Gesture::SlideDown => Some(SwipeDirection::Down),
                _ => None,
            };
            let swipe = swipe_recognizer.update(
                i32::from(event.x),
                i32::from(event.y),
                event.touching,
                controller_gesture,
            );

            POWER_COMMANDS.send(PowerCommand::UserActivity).await;
            // A gesture consumes the touch it was recognised from. The minimum
            // swipe distance still fits inside one menu row, so delivering the
            // release as well would activate the control the finger started on -
            // swiping back out of the firmware screen would reboot the watch.
            if let Some(direction) = swipe {
                if !gesture_claimed {
                    gesture_claimed = true;
                    UI_EVENTS.send(AppEvent::TouchCancelled).await;
                }
                UI_EVENTS.send(AppEvent::Swipe(direction)).await;
            } else if !gesture_claimed {
                UI_EVENTS
                    .send(AppEvent::Touch {
                        x: i32::from(event.x),
                        y: i32::from(event.y),
                        pressed: event.touching,
                    })
                    .await;
            }
            if !event.touching {
                gesture_claimed = false;
            }
        } else {
            // A lost report may hide the release event; stale tracking state
            // would otherwise suppress or misdirect the next swipe, or leave the
            // next finger's touches suppressed as a claimed gesture.
            swipe_recognizer.reset();
            gesture_claimed = false;
            warn!("Touch report read failed");
        }
    }
}
