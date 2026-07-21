use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_nrf::{twim, twim::Twim};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use static_cell::StaticCell;

use crate::board::peripherals::{Irqs, SensorBusResources};

pub type SensorBus = Mutex<CriticalSectionRawMutex, Twim<'static>>;
pub type SensorI2c = I2cDevice<'static, CriticalSectionRawMutex, Twim<'static>>;

static SENSOR_BUS: StaticCell<SensorBus> = StaticCell::new();
#[cfg(feature = "diagnostics")]
const SENSOR_BUS_BUFFER_SIZE: usize = 32;
#[cfg(not(feature = "diagnostics"))]
const SENSOR_BUS_BUFFER_SIZE: usize = 16;
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
    SENSOR_BUS.init(Mutex::new(bus))
}
