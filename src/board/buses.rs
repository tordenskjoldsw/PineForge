use core::cell::RefCell;

use embassy_embedded_hal::{
    adapter::{BlockingAsync, YieldingAsync},
    shared_bus::blocking::spi::SpiDevice,
};
use embassy_nrf::{
    Peri,
    gpio::{Flex, Level, Output, OutputDrive, Pull},
    peripherals::{P0_05, P0_25},
    spim,
    spim::Spim,
    twim,
    twim::Twim,
};
use embassy_sync::blocking_mutex::{Mutex, raw::NoopRawMutex};
use embassy_time::Duration;
use embedded_hal::i2c::{ErrorType, I2c, Operation};
use nrf_pac::twim::vals::Enable;
use static_cell::StaticCell;

use crate::{
    board::peripherals::{DisplayFlashBusResources, Irqs, SensorBusResources},
    services::motion::BusRecovery,
};

pub struct SensorBus {
    inner: Mutex<NoopRawMutex, RefCell<Twim<'static>>>,
}
pub type SharedSpiBus = Mutex<NoopRawMutex, RefCell<Spim<'static>>>;
pub type DisplaySpi = SpiDevice<'static, NoopRawMutex, Spim<'static>, Output<'static>>;
pub type FlashSpi = SpiDevice<'static, NoopRawMutex, Spim<'static>, Output<'static>>;
pub type TouchI2c = BlockingAsync<TimedI2cDevice>;
pub type MotionI2c = YieldingAsync<BlockingAsync<TimedI2cDevice>>;
pub type HeartRateI2c = YieldingAsync<BlockingAsync<TimedI2cDevice>>;

static SENSOR_BUS: StaticCell<SensorBus> = StaticCell::new();
static DISPLAY_FLASH_BUS: StaticCell<SharedSpiBus> = StaticCell::new();
const SENSOR_BUS_BUFFER_SIZE: usize = 32;
static SENSOR_BUS_BUFFER: StaticCell<[u8; SENSOR_BUS_BUFFER_SIZE]> = StaticCell::new();
const PINETIME_TWIM_FREQUENCY: twim::Frequency = twim::Frequency::from_bits(0x0620_0000);
const TRANSACTION_TIMEOUT: Duration = Duration::from_millis(10);
const BUS_CLEAR_PULSES: usize = 16;
const FIVE_MICROSECONDS_AT_64_MHZ: u32 = 320;

impl SensorBus {
    const fn new(bus: Twim<'static>) -> Self {
        Self {
            inner: Mutex::new(RefCell::new(bus)),
        }
    }

    fn recover_after_motion_reset(&self) {
        self.inner.lock(|_| reset_twim1());
    }
}

impl BusRecovery for &'static SensorBus {
    fn recover(&self) {
        self.recover_after_motion_reset();
    }
}

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
        self.bus.inner.lock(|bus| {
            bus.borrow_mut()
                .blocking_transaction_timeout(address, operations, TRANSACTION_TIMEOUT)
        })
    }
}

/// Initializes the shared bus used by touch, motion, and heart-rate sensors.
pub fn init_sensor_bus(mut resources: SensorBusResources) -> &'static SensorBus {
    clear_sensor_bus(resources.scl.reborrow());

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
    SENSOR_BUS.init(SensorBus::new(bus))
}

/// Releases a slave that retained SDA across MCUBoot/application hand-off.
///
/// `InfiniTime` performs the same 16 open-drain SCL pulses before initializing
/// `PineTime`'s shared TWI controller.
fn clear_sensor_bus(scl: embassy_nrf::Peri<'_, embassy_nrf::peripherals::P0_07>) {
    let mut scl = Flex::new(scl);
    scl.set_high();
    scl.set_as_input_output(Pull::None, OutputDrive::Standard0Disconnect1);
    for _ in 0..BUS_CLEAR_PULSES {
        scl.set_low();
        cortex_m::asm::delay(FIVE_MICROSECONDS_AT_64_MHZ);
        scl.set_high();
        cortex_m::asm::delay(FIVE_MICROSECONDS_AT_64_MHZ);
    }
}

/// Restores TWIM1 after the BMA421 soft reset destabilizes the shared bus.
///
/// This mirrors `InfiniTime`'s mandatory `Sleep()` + `Init()` boundary between
/// the BMA reset and all subsequent touch, motion, and HRS transactions.
fn reset_twim1() {
    let registers = nrf_pac::TWIM1;
    registers
        .enable()
        .write(|value| value.set_enable(Enable::Disabled));
    registers.events_lastrx().write_value(0);
    registers.events_stopped().write_value(0);
    registers.events_lasttx().write_value(0);
    registers.events_error().write_value(0);
    registers.events_rxstarted().write_value(0);
    registers.events_suspended().write_value(0);
    registers.events_txstarted().write_value(0);
    registers
        .enable()
        .write(|value| value.set_enable(Enable::Enabled));
}

/// Initializes the SPI bus shared by the ST7789 LCD and the external flash.
///
/// Both devices tolerate mode 3, so a single configuration serves the bus.
/// Every `SpiDevice` transaction is blocking and completes within one
/// executor poll, which keeps the `NoopRawMutex` sharing sound.
pub fn init_display_flash_bus(resources: DisplayFlashBusResources) -> &'static SharedSpiBus {
    let mut config = spim::Config::default();
    config.frequency = spim::Frequency::M8;
    config.mode = spim::MODE_3;
    let spi = Spim::new(
        resources.spi,
        Irqs,
        resources.sck,
        resources.miso,
        resources.mosi,
        config,
    );
    DISPLAY_FLASH_BUS.init(Mutex::new(RefCell::new(spi)))
}

/// Creates the LCD device with its dedicated chip select.
pub fn display_device(bus: &'static SharedSpiBus, cs: Peri<'static, P0_25>) -> DisplaySpi {
    SpiDevice::new(bus, Output::new(cs, Level::High, OutputDrive::Standard))
}

/// Creates the external-flash device with its dedicated chip select.
pub fn flash_device(bus: &'static SharedSpiBus, cs: Peri<'static, P0_05>) -> FlashSpi {
    SpiDevice::new(bus, Output::new(cs, Level::High, OutputDrive::Standard))
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

/// Creates the heart-rate device used by the measurement runner. Heart-rate
/// sampling is not latency-sensitive, so every short transaction yields.
pub fn heart_rate_device(bus: &'static SensorBus) -> HeartRateI2c {
    YieldingAsync::new(BlockingAsync::new(TimedI2cDevice::new(bus)))
}
