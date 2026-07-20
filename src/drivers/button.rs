use embassy_nrf::gpio::Input;
use embassy_time::{Duration, Timer};

/// Debounced active-high `PineTime` side button.
#[allow(dead_code)]
pub struct Button<'d> {
    input: Input<'d>,
    debounce: Duration,
}

#[allow(dead_code)]
impl<'d> Button<'d> {
    #[must_use]
    pub const fn new(input: Input<'d>, debounce: Duration) -> Self {
        Self { input, debounce }
    }

    pub async fn wait_for_press(&mut self) {
        self.input.wait_for_high().await;
        Timer::after(self.debounce).await;
        while self.input.is_high() {
            Timer::after_millis(10).await;
        }
    }
}
