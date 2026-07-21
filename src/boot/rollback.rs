use defmt::info;
use embassy_nrf::gpio::{Input, Output};
use embassy_time::Timer;
use pineforge_state::TEST_IMAGE_TIMEOUT_SECONDS;

/// Resets the unconfirmed test image when the `PineTime` side button is
/// pressed. The enable output is owned by this task so the button circuit
/// remains powered for the entire firmware lifetime.
#[embassy_executor::task]
pub async fn side_button(mut input: Input<'static>, enable: Output<'static>) {
    let _enable = enable;

    loop {
        input.wait_for_high().await;
        Timer::after_millis(30).await;

        if input.is_high() {
            info!("Side-button rollback requested");
            cortex_m::peripheral::SCB::sys_reset();
        }

        input.wait_for_low().await;
    }
}

/// Guarantees that an unconfirmed hardware-test image cannot run forever.
#[embassy_executor::task]
pub async fn safety_timeout() {
    Timer::after_secs(TEST_IMAGE_TIMEOUT_SECONDS).await;
    info!("Safety timeout reached; resetting for MCUBoot rollback");
    cortex_m::peripheral::SCB::sys_reset();
}
