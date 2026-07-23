use embassy_nrf::gpio::Output;

/// `PineTime` vibration motor behind an active-low FET gate.
pub struct VibrationMotor<'d> {
    pin: Output<'d>,
}

impl<'d> VibrationMotor<'d> {
    #[must_use]
    pub const fn new(pin: Output<'d>) -> Self {
        Self { pin }
    }

    pub fn on(&mut self) {
        self.pin.set_low();
    }

    pub fn off(&mut self) {
        self.pin.set_high();
    }
}
