//! Concrete peripheral bundles owned by long-running firmware tasks.

use embassy_nrf::{Peri, bind_interrupts, peripherals, saadc, spim, twim};

bind_interrupts!(pub struct Irqs {
    TWISPI0 => spim::InterruptHandler<peripherals::TWISPI0>;
    TWISPI1 => twim::InterruptHandler<peripherals::TWISPI1>;
    SAADC => saadc::InterruptHandler;
});

/// The SPI bus shared by the LCD and the external flash chip.
pub struct DisplayFlashBusResources {
    pub spi: Peri<'static, peripherals::TWISPI0>,
    pub sck: Peri<'static, peripherals::P0_02>,
    pub miso: Peri<'static, peripherals::P0_04>,
    pub mosi: Peri<'static, peripherals::P0_03>,
}

pub struct DisplayResources {
    pub dc: Peri<'static, peripherals::P0_18>,
    pub reset: Peri<'static, peripherals::P0_26>,
    pub backlight_low: Peri<'static, peripherals::P0_14>,
    pub backlight_mid: Peri<'static, peripherals::P0_22>,
    pub backlight_high: Peri<'static, peripherals::P0_23>,
}

pub struct TouchResources {
    pub reset: Peri<'static, peripherals::P0_10>,
    pub interrupt: Peri<'static, peripherals::P0_28>,
}

#[cfg(feature = "diagnostics")]
pub struct HeartRateResources {
    pub interrupt: Peri<'static, peripherals::P0_30>,
}

pub struct SensorBusResources {
    pub i2c: Peri<'static, peripherals::TWISPI1>,
    pub sda: Peri<'static, peripherals::P0_06>,
    pub scl: Peri<'static, peripherals::P0_07>,
}

pub struct VibrationResources {
    pub motor: Peri<'static, peripherals::P0_16>,
}

/// Peripherals consumed by the MPSL/SoftDevice-Controller BLE stack.
///
/// The radio itself is claimed directly by the controller blob; RTC0, TIMER0,
/// TEMP, and the listed PPI channels are reserved here so no other task can
/// take them.
#[cfg(feature = "ble")]
pub struct BleResources {
    pub rtc0: Peri<'static, peripherals::RTC0>,
    pub timer0: Peri<'static, peripherals::TIMER0>,
    pub temp: Peri<'static, peripherals::TEMP>,
    pub rng: Peri<'static, peripherals::RNG>,
    pub ppi_ch17: Peri<'static, peripherals::PPI_CH17>,
    pub ppi_ch18: Peri<'static, peripherals::PPI_CH18>,
    pub ppi_ch19: Peri<'static, peripherals::PPI_CH19>,
    pub ppi_ch20: Peri<'static, peripherals::PPI_CH20>,
    pub ppi_ch21: Peri<'static, peripherals::PPI_CH21>,
    pub ppi_ch22: Peri<'static, peripherals::PPI_CH22>,
    pub ppi_ch23: Peri<'static, peripherals::PPI_CH23>,
    pub ppi_ch24: Peri<'static, peripherals::PPI_CH24>,
    pub ppi_ch25: Peri<'static, peripherals::PPI_CH25>,
    pub ppi_ch26: Peri<'static, peripherals::PPI_CH26>,
    pub ppi_ch27: Peri<'static, peripherals::PPI_CH27>,
    pub ppi_ch28: Peri<'static, peripherals::PPI_CH28>,
    pub ppi_ch29: Peri<'static, peripherals::PPI_CH29>,
    pub ppi_ch30: Peri<'static, peripherals::PPI_CH30>,
    pub ppi_ch31: Peri<'static, peripherals::PPI_CH31>,
}

pub struct ButtonResources {
    pub input: Peri<'static, peripherals::P0_13>,
    pub enable: Peri<'static, peripherals::P0_15>,
}

pub struct BatteryResources {
    pub adc: Peri<'static, peripherals::SAADC>,
    pub voltage: Peri<'static, peripherals::P0_31>,
    pub charge_status: Peri<'static, peripherals::P0_12>,
    pub power_present: Peri<'static, peripherals::P0_19>,
}
