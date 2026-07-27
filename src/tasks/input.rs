use defmt::{info, warn};
use embassy_nrf::gpio::{Input, Level, Output, OutputDrive, Pull};
use embassy_time::Delay;

use crate::{
    board::{buses::TouchI2c, peripherals::TouchResources},
    drivers::touch::{Cst816s, Gesture},
    services::events::{POWER_COMMANDS, TOUCH_READY, UI_EVENTS},
};
use pineforge_state::{PowerCommand, SwipeDirection, TouchReport, TouchRouter};

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
        interrupt.wait_for_falling_edge().await;

        if let Ok(event) = touch.read_touch().await {
            info!(
                "Touch x={} y={} pressed={} gesture={:?}",
                event.x, event.y, event.touching, event.gesture
            );
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

            POWER_COMMANDS.send(PowerCommand::UserActivity).await;
            for event in events {
                UI_EVENTS.send(event).await;
            }
        } else {
            router.lost_report();
            warn!("Touch report read failed");
        }
    }
}
