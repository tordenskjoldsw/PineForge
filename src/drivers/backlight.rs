use embassy_nrf::gpio::{Level, Output};

/// Three-bit `PineTime` backlight controller.
pub struct Backlight<'d> {
    low: Output<'d>,
    mid: Output<'d>,
    high: Output<'d>,
    level: u8,
}

impl<'d> Backlight<'d> {
    #[must_use]
    pub const fn new(low: Output<'d>, mid: Output<'d>, high: Output<'d>) -> Self {
        Self {
            low,
            mid,
            high,
            level: 0,
        }
    }

    #[must_use]
    #[allow(dead_code)]
    pub const fn level(&self) -> u8 {
        self.level
    }

    pub fn set_level(&mut self, level: u8) {
        let level = level.min(7);
        self.level = level;
        self.low.set_level(bit_level(level, 0));
        self.mid.set_level(bit_level(level, 1));
        self.high.set_level(bit_level(level, 2));
    }
}

const fn bit_level(value: u8, bit: u8) -> Level {
    if value & (1 << bit) == 0 {
        Level::High
    } else {
        // Each brightness control drives an active-low FET gate.
        Level::Low
    }
}
