use defmt::{info, warn};
use embassy_nrf::gpio::{Input, Level, Output, OutputDrive, Pull};
use embassy_time::Delay;

use crate::{
    board::{buses::SensorI2c, peripherals::TouchResources},
    drivers::touch::{Cst816s, Gesture},
    services::events::UI_EVENTS,
};
use pineforge_state::{AppEvent, SwipeDirection};

/// Owns the touch controller and publishes hardware-independent UI events.
#[embassy_executor::task]
pub async fn run(resources: TouchResources, i2c: SensorI2c) {
    let mut interrupt = Input::new(resources.interrupt, Pull::Up);
    let reset = Output::new(resources.reset, Level::High, OutputDrive::Standard);
    let mut touch = Cst816s::new(i2c, reset);
    let mut delay = Delay;

    if touch.setup(&mut delay).await.is_err() {
        warn!("Touch controller setup failed");
    }

    loop {
        interrupt.wait_for_falling_edge().await;

        if let Ok(event) = touch.read_touch().await {
            info!(
                "Touch x={} y={} pressed={} gesture={:?}",
                event.x, event.y, event.touching, event.gesture
            );
            UI_EVENTS
                .send(AppEvent::Touch {
                    x: i32::from(event.x),
                    y: i32::from(event.y),
                    pressed: event.touching,
                })
                .await;
            let swipe = match event.gesture {
                Gesture::SlideLeft => Some(SwipeDirection::Left),
                Gesture::SlideRight => Some(SwipeDirection::Right),
                Gesture::SlideUp => Some(SwipeDirection::Up),
                Gesture::SlideDown => Some(SwipeDirection::Down),
                _ => None,
            };
            if let Some(direction) = swipe {
                UI_EVENTS.send(AppEvent::Swipe(direction)).await;
            }
        } else {
            warn!("Touch report read failed");
        }
    }
}
