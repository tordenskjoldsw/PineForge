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
            UI_EVENTS
                .send(AppEvent::Touch {
                    x: i32::from(event.x),
                    y: i32::from(event.y),
                    pressed: event.touching,
                })
                .await;
            if let Some(direction) = swipe {
                UI_EVENTS.send(AppEvent::Swipe(direction)).await;
            }
        } else {
            // A lost report may hide the release event; stale tracking state
            // would otherwise suppress or misdirect the next swipe.
            swipe_recognizer.reset();
            warn!("Touch report read failed");
        }
    }
}
