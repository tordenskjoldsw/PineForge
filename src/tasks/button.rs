use defmt::info;
use embassy_futures::select::{Either, select};
use embassy_nrf::gpio::{Input, Output};
use embassy_time::{Duration, Timer};

use crate::services::events::{POWER_COMMANDS, UI_EVENTS};
use pineforge_state::{AppEvent, PowerCommand};

const DEBOUNCE: Duration = Duration::from_millis(30);
/// How long the button must stay down to reset instead of navigating back.
///
/// Long enough that leaving a screen never resets by accident, short enough to
/// reach for when the watch stops responding.
const RESET_HOLD: Duration = Duration::from_secs(2);

/// Owns the `PineTime` side button: a press navigates back, holding it resets.
///
/// The reset is the recovery path for a sealed watch, so it must not depend on
/// anything the press-to-go-back path touches. It therefore fires from the
/// timer while the button is still down, rather than on release, and the back
/// event is only offered to the UI - never awaited - so a stalled display task
/// cannot park this task and take the reset with it.
#[embassy_executor::task]
pub async fn run(mut input: Input<'static>, enable: Output<'static>) {
    // Owned for the firmware's lifetime so the button circuit stays powered.
    let _enable = enable;

    loop {
        input.wait_for_high().await;
        Timer::after(DEBOUNCE).await;
        if !input.is_high() {
            continue;
        }

        match select(input.wait_for_low(), Timer::after(RESET_HOLD)).await {
            Either::First(()) => {
                info!("Side button pressed: back");
                // Renews the idle timer first, so a press on a sleeping watch
                // wakes it. The wake guard then swallows the back event, and
                // the press that wakes the watch never also leaves a screen.
                let _ = POWER_COMMANDS.try_send(PowerCommand::UserActivity);
                let _ = UI_EVENTS.try_send(AppEvent::BackPressed);
            }
            Either::Second(()) => {
                info!("Side button held: resetting");
                cortex_m::peripheral::SCB::sys_reset();
            }
        }
    }
}
