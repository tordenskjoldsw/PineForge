//! CST816S touch controller support based on `InfiniTime`'s proven setup.

use embedded_hal::{delay::DelayNs, digital::OutputPin};
use embedded_hal_async::i2c::I2c;

const ADDRESS: u8 = 0x15;
const TOUCH_DATA_START: u8 = 0x01;
const MOTION_MASK: u8 = 0b0000_0101;
const IRQ_CONTROL: u8 = 0b0111_0000;

#[derive(Clone, Copy, Debug, defmt::Format, PartialEq, Eq)]
pub enum Gesture {
    None,
    SlideDown,
    SlideUp,
    SlideLeft,
    SlideRight,
    SingleTap,
    DoubleTap,
    LongPress,
    Unknown(u8),
}

impl From<u8> for Gesture {
    fn from(value: u8) -> Self {
        match value {
            0x00 => Self::None,
            0x01 => Self::SlideDown,
            0x02 => Self::SlideUp,
            0x03 => Self::SlideLeft,
            0x04 => Self::SlideRight,
            0x05 => Self::SingleTap,
            0x0b => Self::DoubleTap,
            0x0c => Self::LongPress,
            value => Self::Unknown(value),
        }
    }
}

#[derive(Clone, Copy, Debug, defmt::Format, PartialEq, Eq)]
pub struct TouchInfo {
    pub x: u16,
    pub y: u16,
    pub touching: bool,
    pub gesture: Gesture,
}

#[derive(Debug)]
pub enum Error<I2cError, PinError> {
    I2c(I2cError),
    Pin(PinError),
    InvalidCoordinates,
}

pub struct Cst816s<I2C, RST> {
    i2c: I2C,
    reset: RST,
}

impl<I2C, RST> Cst816s<I2C, RST>
where
    I2C: I2c,
    RST: OutputPin,
{
    #[must_use]
    pub const fn new(i2c: I2C, reset: RST) -> Self {
        Self { i2c, reset }
    }

    pub async fn setup(
        &mut self,
        delay: &mut impl DelayNs,
    ) -> Result<(), Error<I2C::Error, RST::Error>> {
        self.reset.set_low().map_err(Error::Pin)?;
        delay.delay_ms(5);
        self.reset.set_high().map_err(Error::Pin)?;
        delay.delay_ms(50);

        // InfiniTime performs these reads to wake controllers that do not
        // answer immediately after reset.
        let _ = self.read_register(0x15).await;
        delay.delay_ms(5);
        let _ = self.read_register(0xa7).await;
        delay.delay_ms(5);

        self.write_register(0xec, MOTION_MASK).await?;
        self.write_register(0xfa, IRQ_CONTROL).await?;
        // Disable the controller's five-second automatic reset, which would
        // otherwise interrupt long touches and drawing gestures.
        self.write_register(0xfb, 0).await?;
        Ok(())
    }

    pub async fn read_touch(&mut self) -> Result<TouchInfo, Error<I2C::Error, RST::Error>> {
        // InfiniTime reads registers 1..=6. This includes zero-point reports,
        // which are required to observe the finger being released.
        let mut data = [0_u8; 6];
        self.i2c
            .write_read(ADDRESS, &[TOUCH_DATA_START], &mut data)
            .await
            .map_err(Error::I2c)?;

        let x = (u16::from(data[2] & 0x0f) << 8) | u16::from(data[3]);
        let y = (u16::from(data[4] & 0x0f) << 8) | u16::from(data[5]);
        if x >= 240 || y >= 240 {
            return Err(Error::InvalidCoordinates);
        }

        Ok(TouchInfo {
            x,
            y,
            touching: data[1] & 0x0f != 0,
            gesture: Gesture::from(data[0]),
        })
    }

    async fn read_register(&mut self, register: u8) -> Result<u8, Error<I2C::Error, RST::Error>> {
        let mut value = 0;
        self.i2c
            .write_read(ADDRESS, &[register], core::slice::from_mut(&mut value))
            .await
            .map_err(Error::I2c)?;
        Ok(value)
    }

    async fn write_register(
        &mut self,
        register: u8,
        value: u8,
    ) -> Result<(), Error<I2C::Error, RST::Error>> {
        self.i2c
            .write(ADDRESS, &[register, value])
            .await
            .map_err(Error::I2c)
    }
}
