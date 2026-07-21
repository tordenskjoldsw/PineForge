use embedded_hal_async::i2c::I2c;
use pineforge_state::{AccelerometerKind, accelerometer_kind};

const ADDRESS: u8 = 0x18;
const CHIP_ID_REGISTER: u8 = 0x00;

/// Minimal, non-mutating `BMA42x` identification driver.
pub struct Bma42x<I2C> {
    i2c: I2C,
}

impl<I2C> Bma42x<I2C>
where
    I2C: I2c,
{
    #[must_use]
    pub const fn new(i2c: I2C) -> Self {
        Self { i2c }
    }

    pub async fn probe(&mut self) -> Result<AccelerometerKind, I2C::Error> {
        let mut chip_id = 0;
        self.i2c
            .write_read(
                ADDRESS,
                &[CHIP_ID_REGISTER],
                core::slice::from_mut(&mut chip_id),
            )
            .await?;
        Ok(accelerometer_kind(chip_id))
    }
}
