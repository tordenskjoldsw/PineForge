use core::cell::RefCell;

use embassy_embedded_hal::adapter::{BlockingAsync, YieldingAsync};
use embassy_nrf::{twim, twim::Twim};
use embassy_sync::blocking_mutex::{Mutex, raw::NoopRawMutex};
use embassy_time::Duration;
use embedded_hal::i2c::{ErrorType, I2c, Operation};
use static_cell::StaticCell;

use crate::board::peripherals::{Irqs, SensorBusResources};

pub type SensorBus = Mutex<NoopRawMutex, RefCell<Twim<'static>>>;
pub type TouchI2c = BlockingAsync<TimedI2cDevice>;
pub type MotionI2c = YieldingAsync<BlockingAsync<TimedI2cDevice>>;
#[cfg(feature = "diagnostics")]
pub type HeartRateI2c = YieldingAsync<BlockingAsync<TimedI2cDevice>>;

static SENSOR_BUS: StaticCell<SensorBus> = StaticCell::new();
const SENSOR_BUS_BUFFER_SIZE: usize = 32;
static SENSOR_BUS_BUFFER: StaticCell<[u8; SENSOR_BUS_BUFFER_SIZE]> = StaticCell::new();
const PINETIME_TWIM_FREQUENCY: twim::Frequency = twim::Frequency::from_bits(0x0620_0000);
const TRANSACTION_TIMEOUT: Duration = Duration::from_millis(10);

/// Serialized device handle that applies `PineTime`'s mandatory transaction
/// timeout before adapting the blocking TWIM API to async sensor drivers.
pub struct TimedI2cDevice {
    bus: &'static SensorBus,
}

impl TimedI2cDevice {
    const fn new(bus: &'static SensorBus) -> Self {
        Self { bus }
    }
}

impl ErrorType for TimedI2cDevice {
    type Error = twim::Error;
}

impl I2c for TimedI2cDevice {
    fn transaction(
        &mut self,
        address: u8,
        operations: &mut [Operation<'_>],
    ) -> Result<(), Self::Error> {
        self.bus.lock(|bus| {
            bus.borrow_mut()
                .blocking_transaction_timeout(address, operations, TRANSACTION_TIMEOUT)
        })
    }
}

/// Initializes the shared bus used by touch, motion, and heart-rate sensors.
pub fn init_sensor_bus(resources: SensorBusResources) -> &'static SensorBus {
    let mut config = twim::Config::default();
    // Exact 400 kHz violates the nRF52832 TWIM timing on PineTime. InfiniTime
    // uses this Nordic register value for approximately 390 kHz instead.
    config.frequency = PINETIME_TWIM_FREQUENCY;
    let bus = Twim::new(
        resources.i2c,
        Irqs,
        resources.sda,
        resources.scl,
        config,
        SENSOR_BUS_BUFFER.init([0; SENSOR_BUS_BUFFER_SIZE]),
    );
    SENSOR_BUS.init(Mutex::new(RefCell::new(bus)))
}

/// Creates the latency-sensitive touch device. Each transaction is blocking,
/// matching the proven CST816S access path while keeping its async trait API.
pub fn touch_device(bus: &'static SensorBus) -> TouchI2c {
    BlockingAsync::new(TimedI2cDevice::new(bus))
}

/// Creates the motion device. Its runner provides explicit timer and signal
/// yield points between short, serialized transactions.
pub fn motion_device(bus: &'static SensorBus) -> MotionI2c {
    YieldingAsync::new(BlockingAsync::new(TimedI2cDevice::new(bus)))
}

/// Creates the heart-rate device used by the diagnostics probe. Heart-rate
/// sampling is not latency-sensitive, so every short transaction yields.
#[cfg(feature = "diagnostics")]
pub fn heart_rate_device(bus: &'static SensorBus) -> HeartRateI2c {
    YieldingAsync::new(BlockingAsync::new(TimedI2cDevice::new(bus)))
}
