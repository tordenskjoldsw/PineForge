use embedded_graphics::{
    Pixel,
    draw_target::DrawTarget,
    geometry::{Dimensions, OriginDimensions, Size},
    pixelcolor::{Rgb565, raw::RawU16},
    prelude::RawData,
    primitives::Rectangle,
};
use embedded_hal::{delay::DelayNs, digital::OutputPin, spi::SpiBus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error<SpiError, PinError> {
    Spi(SpiError),
    Pin(PinError),
}

pub struct St7789<SPI, DC, CS, RST> {
    spi: SPI,
    dc: DC,
    cs: CS,
    reset: RST,
    width: u16,
    height: u16,
}

impl<SPI, DC, CS, RST> St7789<SPI, DC, CS, RST>
where
    SPI: SpiBus<u8>,
    DC: OutputPin,
    CS: OutputPin<Error = DC::Error>,
    RST: OutputPin<Error = DC::Error>,
{
    #[must_use]
    pub const fn new(spi: SPI, dc: DC, cs: CS, reset: RST, width: u16, height: u16) -> Self {
        Self {
            spi,
            dc,
            cs,
            reset,
            width,
            height,
        }
    }

    pub fn init(&mut self, delay: &mut impl DelayNs) -> Result<(), Error<SPI::Error, DC::Error>> {
        self.cs.set_high().map_err(Error::Pin)?;
        self.reset.set_high().map_err(Error::Pin)?;
        delay.delay_ms(10);
        self.reset.set_low().map_err(Error::Pin)?;
        delay.delay_ms(20);
        self.reset.set_high().map_err(Error::Pin)?;
        delay.delay_ms(120);

        self.command(0x01, &[])?;
        delay.delay_ms(150);
        self.command(0x11, &[])?;
        delay.delay_ms(120);
        self.command(0x3A, &[0x55])?;
        self.command(0x36, &[0x00])?;
        self.command(0x21, &[])?;
        self.command(0x13, &[])?;
        self.command(0x29, &[])?;
        delay.delay_ms(20);
        Ok(())
    }

    pub fn clear(&mut self, color: Rgb565) -> Result<(), Error<SPI::Error, DC::Error>> {
        self.fill_solid(&self.bounding_box(), color)
    }

    fn command(&mut self, command: u8, data: &[u8]) -> Result<(), Error<SPI::Error, DC::Error>> {
        self.cs.set_low().map_err(Error::Pin)?;
        self.dc.set_low().map_err(Error::Pin)?;
        self.spi.write(&[command]).map_err(Error::Spi)?;
        if !data.is_empty() {
            self.dc.set_high().map_err(Error::Pin)?;
            self.spi.write(data).map_err(Error::Spi)?;
        }
        self.cs.set_high().map_err(Error::Pin)
    }

    fn begin_pixels(&mut self, area: &Rectangle) -> Result<(), Error<SPI::Error, DC::Error>> {
        let x0 = u16::try_from(area.top_left.x).unwrap_or(0);
        let y0 = u16::try_from(area.top_left.y).unwrap_or(0);
        let width = u16::try_from(area.size.width).unwrap_or(u16::MAX);
        let height = u16::try_from(area.size.height).unwrap_or(u16::MAX);
        let x1 = x0.saturating_add(width.saturating_sub(1));
        let y1 = y0.saturating_add(height.saturating_sub(1));
        let [x0_hi, x0_lo] = x0.to_be_bytes();
        let [x1_hi, x1_lo] = x1.to_be_bytes();
        let [y0_hi, y0_lo] = y0.to_be_bytes();
        let [y1_hi, y1_lo] = y1.to_be_bytes();
        self.command(0x2A, &[x0_hi, x0_lo, x1_hi, x1_lo])?;
        self.command(0x2B, &[y0_hi, y0_lo, y1_hi, y1_lo])?;
        self.cs.set_low().map_err(Error::Pin)?;
        self.dc.set_low().map_err(Error::Pin)?;
        self.spi.write(&[0x2C]).map_err(Error::Spi)?;
        self.dc.set_high().map_err(Error::Pin)
    }

    fn finish_pixels(&mut self) -> Result<(), Error<SPI::Error, DC::Error>> {
        self.cs.set_high().map_err(Error::Pin)
    }

    fn write_run(
        &mut self,
        start: embedded_graphics::geometry::Point,
        data: &[u8],
    ) -> Result<(), Error<SPI::Error, DC::Error>> {
        if data.is_empty() {
            return Ok(());
        }

        let pixel_count = u32::try_from(data.len() / 2).unwrap_or(u32::MAX);
        self.begin_pixels(&Rectangle::new(start, Size::new(pixel_count, 1)))?;
        self.spi.write(data).map_err(Error::Spi)?;
        self.finish_pixels()
    }
}

impl<SPI, DC, CS, RST> OriginDimensions for St7789<SPI, DC, CS, RST> {
    fn size(&self) -> Size {
        Size::new(u32::from(self.width), u32::from(self.height))
    }
}

impl<SPI, DC, CS, RST> DrawTarget for St7789<SPI, DC, CS, RST>
where
    SPI: SpiBus<u8>,
    DC: OutputPin,
    CS: OutputPin<Error = DC::Error>,
    RST: OutputPin<Error = DC::Error>,
{
    type Color = Rgb565;
    type Error = Error<SPI::Error, DC::Error>;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        let mut run_start = None;
        let mut run_len = 0_usize;
        let mut run_data = [0_u8; 480];

        for Pixel(point, color) in pixels {
            if point.x < 0
                || point.y < 0
                || point.x >= i32::from(self.width)
                || point.y >= i32::from(self.height)
            {
                continue;
            }

            let continues_run =
                run_start.is_some_and(|start: embedded_graphics::geometry::Point| {
                    point.y == start.y
                        && point.x == start.x + i32::try_from(run_len).unwrap_or(i32::MAX)
                });
            if run_start.is_some() && (!continues_run || run_len == run_data.len() / 2) {
                self.write_run(
                    run_start.expect("a non-empty run has a start point"),
                    &run_data[..run_len * 2],
                )?;
                run_start = None;
                run_len = 0;
            }

            if run_start.is_none() {
                run_start = Some(point);
            }
            let raw = RawU16::from(color).into_inner().to_be_bytes();
            run_data[run_len * 2..run_len * 2 + 2].copy_from_slice(&raw);
            run_len += 1;
        }

        if let Some(start) = run_start {
            self.write_run(start, &run_data[..run_len * 2])?;
        }
        Ok(())
    }

    fn fill_solid(&mut self, area: &Rectangle, color: Self::Color) -> Result<(), Self::Error> {
        let area = area.intersection(&self.bounding_box());
        if area.size.width == 0 || area.size.height == 0 {
            return Ok(());
        }

        self.begin_pixels(&area)?;
        let raw = RawU16::from(color).into_inner().to_be_bytes();
        let mut row = [0_u8; 480];
        for chunk in row.chunks_exact_mut(2).take(area.size.width as usize) {
            chunk.copy_from_slice(&raw);
        }
        for _ in 0..area.size.height {
            self.spi
                .write(&row[..area.size.width as usize * 2])
                .map_err(Error::Spi)?;
        }
        self.finish_pixels()
    }
}
