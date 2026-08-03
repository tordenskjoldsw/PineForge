use embedded_hal_async::i2c::I2c;

const ADDRESS: u8 = 0x44;
const DEVICE_ID: u8 = 0x21;

const REG_ID: u8 = 0x00;
const REG_ENABLE: u8 = 0x01;
const REG_DATA_START: u8 = 0x08;
const REG_LED_DRIVER: u8 = 0x0c;
const REG_RESOLUTION: u8 = 0x16;
const REG_HRS_GAIN: u8 = 0x17;
const DATA_REGISTER_COUNT: usize = 8;
const HRS_ENABLE: u8 = 0x80;
const CONFIG_DISABLED_50_MS: u8 = 0x50;
const LED_DRIVE_12_5_MA: u8 = 0x2f;
const RESOLUTION_15_BIT: u8 = 0x77;
const HRS_GAIN_1X: u8 = 0x00;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hrs3300Kind {
    Hrs3300,
    Unknown(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hrs3300Sample {
    pub hrs: u16,
    pub als: u16,
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

    /// Applies `InfiniTime`'s proven `PineTime` acquisition configuration while
    /// leaving the conversion engine disabled.
    pub async fn configure(&mut self) -> Result<(), I2C::Error> {
        self.write_register(REG_ENABLE, CONFIG_DISABLED_50_MS)
            .await?;
        self.write_register(REG_LED_DRIVER, LED_DRIVE_12_5_MA)
            .await?;
        self.write_register(REG_RESOLUTION, RESOLUTION_15_BIT)
            .await?;
        self.write_register(REG_HRS_GAIN, HRS_GAIN_1X).await
    }

    pub async fn power_up(&mut self) -> Result<(), I2C::Error> {
        let enable = self.read_register(REG_ENABLE).await? | HRS_ENABLE;
        self.write_register(REG_ENABLE, enable).await?;
        self.write_register(REG_LED_DRIVER, LED_DRIVE_12_5_MA).await
    }

    pub async fn power_down(&mut self) -> Result<(), I2C::Error> {
        self.disable().await
    }

    /// Reads and decodes `InfiniTime`'s coherent heart-signal and ambient-light
    /// sample block in one transaction.
    pub async fn read_sample(&mut self) -> Result<Hrs3300Sample, I2C::Error> {
        let mut registers = [0; DATA_REGISTER_COUNT];
        self.i2c
            .write_read(ADDRESS, &[REG_DATA_START], &mut registers)
            .await?;

        let hrs = (u16::from(registers[1]) << 8)
            | (u16::from(registers[2] & 0x0f) << 4)
            | u16::from(registers[7] & 0x0f);
        let als = (u16::from(registers[5] & 0x3f) << 11)
            | (u16::from(registers[0]) << 3)
            | u16::from(registers[6] & 0x07);
        Ok(Hrs3300Sample { hrs, als })
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
