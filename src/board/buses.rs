use core::cell::RefCell;

use embassy_embedded_hal::{
    adapter::{BlockingAsync, YieldingAsync},
    shared_bus::blocking::i2c::I2cDevice as BlockingI2cDevice,
};
use embassy_nrf::{twim, twim::Twim};
use embassy_sync::blocking_mutex::{Mutex, raw::NoopRawMutex};
use static_cell::StaticCell;

use crate::board::peripherals::{Irqs, SensorBusResources};

pub type SensorBus = Mutex<NoopRawMutex, RefCell<Twim<'static>>>;
type BlockingSensorI2c = BlockingI2cDevice<'static, NoopRawMutex, Twim<'static>>;
pub type TouchI2c = BlockingAsync<BlockingSensorI2c>;
pub type MotionI2c = YieldingAsync<BlockingAsync<BlockingSensorI2c>>;
#[cfg(feature = "diagnostics")]
pub type HeartRateI2c = YieldingAsync<BlockingAsync<BlockingSensorI2c>>;

static SENSOR_BUS: StaticCell<SensorBus> = StaticCell::new();
const SENSOR_BUS_BUFFER_SIZE: usize = 32;
static SENSOR_BUS_BUFFER: StaticCell<[u8; SENSOR_BUS_BUFFER_SIZE]> = StaticCell::new();

/// Initializes the shared 400 kHz bus used by touch and motion sensors.
pub fn init_sensor_bus(resources: SensorBusResources) -> &'static SensorBus {
    let mut config = twim::Config::default();
    config.frequency = twim::Frequency::K400;
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
    BlockingAsync::new(BlockingI2cDevice::new(bus))
}

/// Creates the motion device. Its runner provides explicit timer and signal
/// yield points between short, serialized transactions.
pub fn motion_device(bus: &'static SensorBus) -> MotionI2c {
    YieldingAsync::new(BlockingAsync::new(BlockingI2cDevice::new(bus)))
}

/// Creates the heart-rate device used by the diagnostics probe. Heart-rate
/// sampling is not latency-sensitive, so every short transaction yields.
#[cfg(feature = "diagnostics")]
pub fn heart_rate_device(bus: &'static SensorBus) -> HeartRateI2c {
    YieldingAsync::new(BlockingAsync::new(BlockingI2cDevice::new(bus)))
}
