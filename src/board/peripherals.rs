//! Concrete peripheral bundles owned by long-running firmware tasks.

use embassy_nrf::{Peri, bind_interrupts, peripherals, saadc, spim, twim};

bind_interrupts!(pub struct Irqs {
    TWISPI0 => spim::InterruptHandler<peripherals::TWISPI0>;
    TWISPI1 => twim::InterruptHandler<peripherals::TWISPI1>;
    SAADC => saadc::InterruptHandler;
});

pub struct DisplayResources {
    pub spi: Peri<'static, peripherals::TWISPI0>,
    pub sck: Peri<'static, peripherals::P0_02>,
    pub miso: Peri<'static, peripherals::P0_04>,
    pub mosi: Peri<'static, peripherals::P0_03>,
    pub dc: Peri<'static, peripherals::P0_18>,
    pub cs: Peri<'static, peripherals::P0_25>,
    pub reset: Peri<'static, peripherals::P0_26>,
    pub backlight_low: Peri<'static, peripherals::P0_14>,
    pub backlight_mid: Peri<'static, peripherals::P0_22>,
    pub backlight_high: Peri<'static, peripherals::P0_23>,
}

pub struct TouchResources {
    pub reset: Peri<'static, peripherals::P0_10>,
    pub interrupt: Peri<'static, peripherals::P0_28>,
}

pub struct SensorBusResources {
    pub i2c: Peri<'static, peripherals::TWISPI1>,
    pub sda: Peri<'static, peripherals::P0_06>,
    pub scl: Peri<'static, peripherals::P0_07>,
}

#[cfg(feature = "diagnostics")]
pub struct AccelerometerResources {
    pub interrupt: Peri<'static, peripherals::P0_08>,
}

pub struct ButtonResources {
    pub input: Peri<'static, peripherals::P0_13>,
    pub enable: Peri<'static, peripherals::P0_15>,
}

pub struct BatteryResources {
    pub adc: Peri<'static, peripherals::SAADC>,
    pub voltage: Peri<'static, peripherals::P0_31>,
    pub charge_status: Peri<'static, peripherals::P0_12>,
}
