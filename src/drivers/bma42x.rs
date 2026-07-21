use embassy_time::{Duration, Timer};
use embedded_hal_async::i2c::I2c;
use pineforge_state::{AccelerationSample, AccelerometerKind, accelerometer_kind};

const ADDRESS: u8 = 0x18;
const CHIP_ID_REGISTER: u8 = 0x00;
const ACCEL_DATA_REGISTER: u8 = 0x12;
const ACCEL_CONFIG_REGISTER: u8 = 0x40;
const ACCEL_RANGE_REGISTER: u8 = 0x41;
const POWER_CONTROL_REGISTER: u8 = 0x7d;
const INT1_IO_CONTROL_REGISTER: u8 = 0x53;
const INTERRUPT_MAP_DATA_REGISTER: u8 = 0x58;
const INTERRUPT_STATUS_1_REGISTER: u8 = 0x1d;
const INTERNAL_STATUS_REGISTER: u8 = 0x2a;
const FEATURE_CONFIG_ADDRESS_LSB_REGISTER: u8 = 0x5b;
const FEATURE_CONFIG_ADDRESS_MSB_REGISTER: u8 = 0x5c;
const FEATURE_CONFIG_DATA_REGISTER: u8 = 0x5e;
const INIT_CONTROL_REGISTER: u8 = 0x59;
const POWER_CONFIG_REGISTER: u8 = 0x7c;
const COMMAND_REGISTER: u8 = 0x7e;

const ACCEL_25_HZ_NORMAL_AVG4: u8 = 0x26;
const ACCEL_12_5_HZ_NORMAL_AVG4: u8 = 0x25;
const ACCEL_RANGE_2G: u8 = 0x00;
const ACCEL_ENABLE: u8 = 1 << 2;
const REGISTER_WRITE_DELAY: Duration = Duration::from_millis(1);
const INT1_EDGE_ACTIVE_HIGH_PUSH_PULL: u8 = 0x0b;
const INT1_DATA_READY: u8 = 1 << 2;
const ACCEL_DATA_READY_STATUS: u8 = 1 << 7;
const ADVANCED_POWER_SAVE: u8 = 1;
const ASIC_INITIALIZED: u8 = 1;
const SOFT_RESET_COMMAND: u8 = 0xb6;
const FEATURE_CONFIG_CHUNK_SIZE: usize = 16;

const BMA421_FEATURE_CONFIG: &[u8; 6_144] = include_bytes!(concat!(env!("OUT_DIR"), "/bma421.bin"));
const BMA425_FEATURE_CONFIG: &[u8; 6_144] = include_bytes!(concat!(env!("OUT_DIR"), "/bma425.bin"));

#[derive(Debug)]
pub enum FeatureEngineError<E> {
    Bus(E),
    UnsupportedSensor,
    InitializationFailed(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccelerationPowerMode {
    Active,
    LowPower,
    Off,
}

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

    pub async fn reset(&mut self) -> Result<(), I2C::Error> {
        self.write_register(COMMAND_REGISTER, SOFT_RESET_COMMAND)
            .await?;
        Timer::after(Duration::from_millis(2)).await;
        Ok(())
    }

    /// Uploads and verifies Bosch's feature-engine configuration stream.
    pub async fn initialize_feature_engine(
        &mut self,
        kind: AccelerometerKind,
    ) -> Result<(), FeatureEngineError<I2C::Error>> {
        let config = match kind {
            AccelerometerKind::Bma421 => BMA421_FEATURE_CONFIG,
            AccelerometerKind::Bma425 => BMA425_FEATURE_CONFIG,
            AccelerometerKind::Unknown(_) | AccelerometerKind::Unavailable => {
                return Err(FeatureEngineError::UnsupportedSensor);
            }
        };

        let power = self
            .read_register(POWER_CONFIG_REGISTER)
            .await
            .map_err(FeatureEngineError::Bus)?;
        self.write_register(POWER_CONFIG_REGISTER, power & !ADVANCED_POWER_SAVE)
            .await
            .map_err(FeatureEngineError::Bus)?;
        self.write_register(INIT_CONTROL_REGISTER, 0)
            .await
            .map_err(FeatureEngineError::Bus)?;

        for (chunk_index, chunk) in config.chunks(FEATURE_CONFIG_CHUNK_SIZE).enumerate() {
            let byte_index = chunk_index * FEATURE_CONFIG_CHUNK_SIZE;
            let word_address = u16::try_from(byte_index / 2).unwrap_or(u16::MAX);
            self.write_register(
                FEATURE_CONFIG_ADDRESS_LSB_REGISTER,
                u8::try_from(word_address & 0x0f).unwrap_or(0),
            )
            .await
            .map_err(FeatureEngineError::Bus)?;
            self.write_register(
                FEATURE_CONFIG_ADDRESS_MSB_REGISTER,
                u8::try_from(word_address >> 4).unwrap_or(u8::MAX),
            )
            .await
            .map_err(FeatureEngineError::Bus)?;
            self.write_feature_chunk(chunk)
                .await
                .map_err(FeatureEngineError::Bus)?;
        }

        self.write_register(INIT_CONTROL_REGISTER, 1)
            .await
            .map_err(FeatureEngineError::Bus)?;
        Timer::after(Duration::from_millis(150)).await;
        let status = self
            .read_register(INTERNAL_STATUS_REGISTER)
            .await
            .map_err(FeatureEngineError::Bus)?;
        if status & 0x0f != ASIC_INITIALIZED {
            return Err(FeatureEngineError::InitializationFailed(status));
        }

        let power = self
            .read_register(POWER_CONFIG_REGISTER)
            .await
            .map_err(FeatureEngineError::Bus)?;
        self.write_register(POWER_CONFIG_REGISTER, power | ADVANCED_POWER_SAVE)
            .await
            .map_err(FeatureEngineError::Bus)
    }

    /// Applies the requested power mode without enabling the optional feature engine.
    pub async fn set_power_mode(&mut self, mode: AccelerationPowerMode) -> Result<(), I2C::Error> {
        let power = self.read_register(POWER_CONTROL_REGISTER).await?;
        if mode == AccelerationPowerMode::Off {
            self.write_register(POWER_CONTROL_REGISTER, power & !ACCEL_ENABLE)
                .await?;
            return Ok(());
        }

        let config = match mode {
            AccelerationPowerMode::Active => ACCEL_25_HZ_NORMAL_AVG4,
            AccelerationPowerMode::LowPower => ACCEL_12_5_HZ_NORMAL_AVG4,
            AccelerationPowerMode::Off => unreachable!(),
        };
        self.write_register(ACCEL_CONFIG_REGISTER, config).await?;
        self.write_register(ACCEL_RANGE_REGISTER, ACCEL_RANGE_2G)
            .await?;
        self.write_register(POWER_CONTROL_REGISTER, power | ACCEL_ENABLE)
            .await?;
        Timer::after(Duration::from_millis(2)).await;
        Ok(())
    }

    pub async fn read_acceleration(&mut self) -> Result<AccelerationSample, I2C::Error> {
        let mut data = [0; 6];
        self.i2c
            .write_read(ADDRESS, &[ACCEL_DATA_REGISTER], &mut data)
            .await?;
        Ok(AccelerationSample {
            x: decode_axis(data[0], data[1]),
            y: decode_axis(data[2], data[3]),
            z: decode_axis(data[4], data[5]),
        })
    }

    /// Routes the non-latched accelerometer data-ready signal to INT1.
    pub async fn enable_data_ready_interrupt(&mut self) -> Result<(), I2C::Error> {
        self.write_register(INT1_IO_CONTROL_REGISTER, INT1_EDGE_ACTIVE_HIGH_PUSH_PULL)
            .await?;
        let mapping = self.read_register(INTERRUPT_MAP_DATA_REGISTER).await?;
        self.write_register(INTERRUPT_MAP_DATA_REGISTER, mapping | INT1_DATA_READY)
            .await
    }

    /// Reads and clears the hardware interrupt status for the current sample.
    pub async fn acknowledge_data_ready(&mut self) -> Result<bool, I2C::Error> {
        let status = self.read_register(INTERRUPT_STATUS_1_REGISTER).await?;
        Ok(status & ACCEL_DATA_READY_STATUS != 0)
    }

    async fn read_register(&mut self, register: u8) -> Result<u8, I2C::Error> {
        let mut value = 0;
        self.i2c
            .write_read(ADDRESS, &[register], core::slice::from_mut(&mut value))
            .await?;
        Ok(value)
    }

    async fn write_register(&mut self, register: u8, value: u8) -> Result<(), I2C::Error> {
        self.i2c.write(ADDRESS, &[register, value]).await?;
        Timer::after(REGISTER_WRITE_DELAY).await;
        Ok(())
    }

    async fn write_feature_chunk(&mut self, chunk: &[u8]) -> Result<(), I2C::Error> {
        let mut transaction = [0; FEATURE_CONFIG_CHUNK_SIZE + 1];
        transaction[0] = FEATURE_CONFIG_DATA_REGISTER;
        transaction[1..=chunk.len()].copy_from_slice(chunk);
        self.i2c
            .write(ADDRESS, &transaction[..=chunk.len()])
            .await?;
        Timer::after(REGISTER_WRITE_DELAY).await;
        Ok(())
    }
}

const fn decode_axis(lsb: u8, msb: u8) -> i16 {
    i16::from_le_bytes([lsb, msb]) >> 4
}
