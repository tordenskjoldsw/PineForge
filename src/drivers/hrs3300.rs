use embedded_hal_async::i2c::I2c;

const ADDRESS: u8 = 0x44;
const DEVICE_ID: u8 = 0x21;

const REG_ID: u8 = 0x00;
const REG_ENABLE: u8 = 0x01;
const REG_LED_DRIVER: u8 = 0x0c;
const HRS_ENABLE: u8 = 0x80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hrs3300Kind {
    Hrs3300,
    Unknown(u8),
}

/// Minimal HRS3300 transport driver. Signal acquisition and BPM processing
/// belong to the future heart-rate service rather than this register layer.
pub struct Hrs3300<I2C> {
    i2c: I2C,
}

impl<I2C> Hrs3300<I2C>
where
    I2C: I2c,
{
    pub const fn new(i2c: I2C) -> Self {
        Self { i2c }
    }

    /// Identifies the sensor and leaves its ADC and LED driver disabled.
    pub async fn probe_and_disable(&mut self) -> Result<Hrs3300Kind, I2C::Error> {
        let id = self.read_register(REG_ID).await?;
        self.disable().await?;
        Ok(if id == DEVICE_ID {
            Hrs3300Kind::Hrs3300
        } else {
            Hrs3300Kind::Unknown(id)
        })
    }

    async fn disable(&mut self) -> Result<(), I2C::Error> {
        let enable = self.read_register(REG_ENABLE).await? & !HRS_ENABLE;
        self.write_register(REG_ENABLE, enable).await?;
        self.write_register(REG_LED_DRIVER, 0).await
    }

    async fn read_register(&mut self, register: u8) -> Result<u8, I2C::Error> {
        let mut value = [0];
        self.i2c
            .write_read(ADDRESS, &[register], &mut value)
            .await?;
        Ok(value[0])
    }

    async fn write_register(&mut self, register: u8, value: u8) -> Result<(), I2C::Error> {
        self.i2c.write(ADDRESS, &[register, value]).await
    }
}
