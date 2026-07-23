//! XT25F32B SPI-NOR transport for the `PineTime` external flash.
//!
//! Long erase and program waits happen inside the chip while the shared bus
//! is released; this driver polls the status register in short transactions
//! with timer yields between, mirroring the sensor-bus philosophy.

use embassy_time::Timer;
use embedded_hal::spi::{Operation, SpiDevice};

const RELEASE_DEEP_POWER_DOWN: u8 = 0xab;
const READ_JEDEC_ID: u8 = 0x9f;
const READ_DATA: u8 = 0x03;
const WRITE_ENABLE: u8 = 0x06;
const PAGE_PROGRAM: u8 = 0x02;
const SECTOR_ERASE: u8 = 0x20;
const READ_STATUS: u8 = 0x05;
const STATUS_WRITE_IN_PROGRESS: u8 = 0x01;

/// Manufacturer and device identification of the stock `PineTime` chip.
pub const EXPECTED_JEDEC_ID: [u8; 3] = [0x0b, 0x40, 0x16];
pub const PAGE_SIZE: usize = 256;

/// Sector erase completes in at most 300 ms; give a generous bound.
const BUSY_POLL_LIMIT_MILLIS: u32 = 1_000;

#[derive(Debug)]
pub enum Error<SpiError> {
    Spi(SpiError),
    BusyTimeout,
    VerifyFailed,
}

pub struct Xt25f32<SPI> {
    spi: SPI,
}

impl<SPI: SpiDevice> Xt25f32<SPI> {
    #[must_use]
    pub const fn new(spi: SPI) -> Self {
        Self { spi }
    }

    /// Wakes the chip and returns its JEDEC identification.
    ///
    /// `InfiniTime` puts the flash into deep power-down on shutdown, so the
    /// release command is mandatory before the chip answers anything.
    pub async fn init(&mut self) -> Result<[u8; 3], Error<SPI::Error>> {
        self.spi
            .write(&[RELEASE_DEEP_POWER_DOWN])
            .map_err(Error::Spi)?;
        // Datasheet tRES1: the chip needs ~30 us before accepting commands.
        Timer::after_micros(100).await;

        let mut id = [0_u8; 3];
        self.spi
            .transaction(&mut [Operation::Write(&[READ_JEDEC_ID]), Operation::Read(&mut id)])
            .map_err(Error::Spi)?;
        Ok(id)
    }

    pub fn read(&mut self, address: u32, buffer: &mut [u8]) -> Result<(), Error<SPI::Error>> {
        self.spi
            .transaction(&mut [
                Operation::Write(&command(READ_DATA, address)),
                Operation::Read(buffer),
            ])
            .map_err(Error::Spi)
    }

    pub async fn erase_sector(&mut self, address: u32) -> Result<(), Error<SPI::Error>> {
        self.spi.write(&[WRITE_ENABLE]).map_err(Error::Spi)?;
        self.spi
            .write(&command(SECTOR_ERASE, address))
            .map_err(Error::Spi)?;
        self.wait_while_busy().await
    }

    /// Programs up to one page; the data must not cross a page boundary.
    pub async fn program(&mut self, address: u32, data: &[u8]) -> Result<(), Error<SPI::Error>> {
        debug_assert!(data.len() <= PAGE_SIZE);
        self.spi.write(&[WRITE_ENABLE]).map_err(Error::Spi)?;
        self.spi
            .transaction(&mut [
                Operation::Write(&command(PAGE_PROGRAM, address)),
                Operation::Write(data),
            ])
            .map_err(Error::Spi)?;
        self.wait_while_busy().await
    }

    /// Programs one record and confirms it by reading it back.
    pub async fn program_verified<const N: usize>(
        &mut self,
        address: u32,
        data: &[u8; N],
    ) -> Result<(), Error<SPI::Error>> {
        self.program(address, data).await?;
        let mut readback = [0_u8; N];
        self.read(address, &mut readback)?;
        if readback == *data {
            Ok(())
        } else {
            Err(Error::VerifyFailed)
        }
    }

    async fn wait_while_busy(&mut self) -> Result<(), Error<SPI::Error>> {
        for _ in 0..BUSY_POLL_LIMIT_MILLIS {
            let mut status = [0_u8; 1];
            self.spi
                .transaction(&mut [
                    Operation::Write(&[READ_STATUS]),
                    Operation::Read(&mut status),
                ])
                .map_err(Error::Spi)?;
            if status[0] & STATUS_WRITE_IN_PROGRESS == 0 {
                return Ok(());
            }
            Timer::after_millis(1).await;
        }
        Err(Error::BusyTimeout)
    }
}

const fn command(opcode: u8, address: u32) -> [u8; 4] {
    let address = address.to_be_bytes();
    [opcode, address[1], address[2], address[3]]
}
