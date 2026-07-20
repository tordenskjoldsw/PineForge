use defmt::{info, warn};
use embassy_nrf::{
    gpio::{Input, Level, Output, OutputDrive, Pull},
    twim,
};
use embassy_time::Delay;
use static_cell::StaticCell;

use crate::{
    board::peripherals::{Irqs, TouchResources},
    drivers::touch::Cst816s,
    services::events::{UI_EVENTS, UiEvent},
};

static TWIM_BUFFER: StaticCell<[u8; 16]> = StaticCell::new();

/// Owns the touch controller and publishes hardware-independent UI events.
#[embassy_executor::task]
pub async fn run(resources: TouchResources) {
    let mut config = twim::Config::default();
    config.frequency = twim::Frequency::K400;
    let i2c = twim::Twim::new(
        resources.i2c,
        Irqs,
        resources.sda,
        resources.scl,
        config,
        TWIM_BUFFER.init([0; 16]),
    );
    let mut interrupt = Input::new(resources.interrupt, Pull::Up);
    let reset = Output::new(resources.reset, Level::High, OutputDrive::Standard);
    let mut touch = Cst816s::new(i2c, reset);
    let mut delay = Delay;

    if touch.setup(&mut delay).is_err() {
        warn!("Touch controller setup failed");
    }

    loop {
        interrupt.wait_for_falling_edge().await;

        if let Ok(event) = touch.read_touch() {
            info!(
                "Touch x={} y={} pressed={} gesture={:?}",
                event.x, event.y, event.touching, event.gesture
            );
            UI_EVENTS
                .send(UiEvent::Touch {
                    x: i32::from(event.x),
                    y: i32::from(event.y),
                    pressed: event.touching,
                })
                .await;
        } else {
            warn!("Touch report read failed");
        }
    }
}
