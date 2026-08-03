use embassy_time::{Duration, Timer};
use embedded_hal_async::i2c::I2c;
use pineforge_state::AccelerationSample;
use pineforge_state::{AccelerometerKind, accelerometer_kind};

const ADDRESS: u8 = 0x18;
const CHIP_ID_REGISTER: u8 = 0x00;
const ACCEL_DATA_REGISTER: u8 = 0x12;
const ACCEL_CONFIG_REGISTER: u8 = 0x40;
const ACCEL_RANGE_REGISTER: u8 = 0x41;
const POWER_CONTROL_REGISTER: u8 = 0x7d;
const INT1_IO_CONTROL_REGISTER: u8 = 0x53;
const INTERRUPT_MAP_DATA_REGISTER: u8 = 0x58;
const INTERNAL_STATUS_REGISTER: u8 = 0x2a;
const FEATURE_CONFIG_ADDRESS_LSB_REGISTER: u8 = 0x5b;
const FEATURE_CONFIG_ADDRESS_MSB_REGISTER: u8 = 0x5c;
const FEATURE_CONFIG_DATA_REGISTER: u8 = 0x5e;
const INIT_CONTROL_REGISTER: u8 = 0x59;
const POWER_CONFIG_REGISTER: u8 = 0x7c;
const COMMAND_REGISTER: u8 = 0x7e;

const ACCEL_100_HZ_NORMAL_AVG4: u8 = 0x28;
const ACCEL_RANGE_2G: u8 = 0x00;
const ACCEL_ENABLE: u8 = 1 << 2;
const REGISTER_WRITE_DELAY: Duration = Duration::from_millis(1);
const INT1_EDGE_ACTIVE_HIGH_PUSH_PULL: u8 = 0x0b;
const INT1_DATA_READY: u8 = 1 << 2;
const ADVANCED_POWER_SAVE: u8 = 1;
const ASIC_INITIALIZED: u8 = 1;
const SOFT_RESET_COMMAND: u8 = 0xb6;
const FEATURE_CONFIG_CHUNK_SIZE: usize = 16;
const FEATURE_CONFIG_SIZE: usize = 70;
const STEP_COUNTER_CONFIG_OFFSET: usize = 0x3a;
const STEP_COUNTER_ENABLE: u8 = 0x10;
const STEP_DETECTOR_ENABLE: u8 = 0x08;
const STEP_COUNTER_OUTPUT_REGISTER: u8 = 0x1e;

const BMA421_FEATURE_CONFIG: &[u8; 6_144] = include_bytes!(concat!(env!("OUT_DIR"), "/bma421.bin"));
const BMA425_FEATURE_CONFIG: &[u8; 6_144] = include_bytes!(concat!(env!("OUT_DIR"), "/bma425.bin"));

#[derive(Debug)]
pub enum FeatureEngineError<E> {
    Bus(E),
    UnsupportedSensor,
    NotInitialized,
    InitializationFailed(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccelerationPowerMode {
    Active,
    LowPower,
    /// Clears the sensor's enable bit, so it stops converting entirely.
    ///
    /// Nothing asks for this. `services::motion::acceleration_mode` maps every
    /// system power state onto `Active` or `LowPower`, so the accelerometer
    /// runs from boot until reset and the power-down path in
    /// [`Bma42x::set_power_mode`] is never taken.
    ///
    /// That was invisible until this enum stopped being compared against
    /// `Off` to pick a branch: the comparison constructed the variant, which
    /// was enough to keep dead-code analysis quiet about a state the firmware
    /// cannot reach. Kept rather than deleted because the driver's half of the
    /// work is done and correct; what is missing is the decision about whether
    /// a sleeping watch with raise-wrist off should stop the sensor, which is
    /// a power measurement rather than a code change.
    #[expect(dead_code)]
    Off,
}

/// Minimal, non-mutating `BMA42x` identification driver.
pub struct Bma42x<I2C> {
    i2c: I2C,
    feature_config_start: Option<u16>,
}

impl<I2C> Bma42x<I2C>
where
    I2C: I2c,
{
    #[must_use]
    pub const fn new(i2c: I2C) -> Self {
        Self {
            i2c,
            feature_config_start: None,
        }
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
            .map_err(FeatureEngineError::Bus)?;

        let low_nibble = self
            .read_register(FEATURE_CONFIG_ADDRESS_LSB_REGISTER)
            .await
            .map_err(FeatureEngineError::Bus)?;
        let upper_bits = self
            .read_register(FEATURE_CONFIG_ADDRESS_MSB_REGISTER)
            .await
            .map_err(FeatureEngineError::Bus)?;
        self.feature_config_start =
            Some((u16::from(upper_bits) << 4) | u16::from(low_nibble & 0x0f));
        Ok(())
    }

    pub async fn enable_step_counter(&mut self) -> Result<(), FeatureEngineError<I2C::Error>> {
        let start = self
            .feature_config_start
            .ok_or(FeatureEngineError::NotInitialized)?;
        let offset = STEP_COUNTER_CONFIG_OFFSET;
        debug_assert!(offset + 1 < FEATURE_CONFIG_SIZE);

        let power = self
            .read_register(POWER_CONFIG_REGISTER)
            .await
            .map_err(FeatureEngineError::Bus)?;
        self.write_register(POWER_CONFIG_REGISTER, power & !ADVANCED_POWER_SAVE)
            .await
            .map_err(FeatureEngineError::Bus)?;

        self.set_feature_address(start + u16::try_from(offset / 2).unwrap_or(0))
            .await
            .map_err(FeatureEngineError::Bus)?;
        let mut config = [0; 2];
        self.i2c
            .write_read(ADDRESS, &[FEATURE_CONFIG_DATA_REGISTER], &mut config)
            .await
            .map_err(FeatureEngineError::Bus)?;
        config[1] |= STEP_COUNTER_ENABLE;
        config[1] &= !STEP_DETECTOR_ENABLE;
        self.set_feature_address(start + u16::try_from(offset / 2).unwrap_or(0))
            .await
            .map_err(FeatureEngineError::Bus)?;
        self.write_feature_chunk(&config)
            .await
            .map_err(FeatureEngineError::Bus)?;

        self.write_register(POWER_CONFIG_REGISTER, power)
            .await
            .map_err(FeatureEngineError::Bus)
    }

    pub async fn read_step_count(&mut self) -> Result<u32, I2C::Error> {
        let mut data = [0; 4];
        self.i2c
            .write_read(ADDRESS, &[STEP_COUNTER_OUTPUT_REGISTER], &mut data)
            .await?;
        Ok(u32::from_le_bytes(data))
    }

    /// Applies the requested power mode without enabling the optional feature engine.
    pub async fn set_power_mode(&mut self, mode: AccelerationPowerMode) -> Result<(), I2C::Error> {
        let power = self.read_register(POWER_CONTROL_REGISTER).await?;
        let config = match mode {
            // Powering down is the whole operation. The configuration
            // registers keep their values while the sensor is off and are
            // written again by the arm below when it comes back up.
            AccelerationPowerMode::Off => {
                return self
                    .write_register(POWER_CONTROL_REGISTER, power & !ACCEL_ENABLE)
                    .await;
            }
            // Both live modes leave the sensor converting at the same rate;
            // what separates them is how often the runner reads it.
            AccelerationPowerMode::Active | AccelerationPowerMode::LowPower => {
                ACCEL_100_HZ_NORMAL_AVG4
            }
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
    pub async fn set_data_ready_interrupt(&mut self, enabled: bool) -> Result<(), I2C::Error> {
        if enabled {
            self.write_register(INT1_IO_CONTROL_REGISTER, INT1_EDGE_ACTIVE_HIGH_PUSH_PULL)
                .await?;
        }
        let mapping = self.read_register(INTERRUPT_MAP_DATA_REGISTER).await?;
        let mapping = if enabled {
            mapping | INT1_DATA_READY
        } else {
            mapping & !INT1_DATA_READY
        };
        self.write_register(INTERRUPT_MAP_DATA_REGISTER, mapping)
            .await
    }

    /// Reads and clears the hardware interrupt status for the current sample.
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

    async fn set_feature_address(&mut self, address: u16) -> Result<(), I2C::Error> {
        self.write_register(
            FEATURE_CONFIG_ADDRESS_LSB_REGISTER,
            u8::try_from(address & 0x0f).unwrap_or(0),
        )
        .await?;
        self.write_register(
            FEATURE_CONFIG_ADDRESS_MSB_REGISTER,
            u8::try_from(address >> 4).unwrap_or(u8::MAX),
        )
        .await
    }
}

const fn decode_axis(lsb: u8, msb: u8) -> i16 {
    i16::from_le_bytes([lsb, msb]) >> 4
}
