//! Shared, single-owner scratch memory for transient UI rendering.
//!
//! The display task owns one instance. Renderers borrow it only for the
//! duration of an operation, so future image decoding, notification layout, or
//! other temporary work must reuse this allocation instead of adding another
//! permanent framebuffer-like static.

use core::convert::Infallible;

use embedded_graphics::{
    Pixel,
    draw_target::DrawTarget,
    geometry::{Dimensions, Point, Size},
    pixelcolor::{Rgb565, RgbColor},
    primitives::Rectangle,
};

pub const STRIPE_WIDTH: u32 = 16;
pub const STRIPE_WIDTH_USIZE: usize = 16;
const MAX_SCREEN_HEIGHT: u32 = 240;
const SCRATCH_PIXELS: usize = STRIPE_WIDTH_USIZE * 240;

/// The display task's reusable 7.5 KiB rendering workspace.
///
/// It currently acts as a clipped RGB565 target for slide transitions. Keeping
/// the storage in this operation-neutral type makes its exclusivity explicit:
/// later transient UI operations extend this owner rather than reserve another
/// large static buffer.
pub struct UiScratch {
    pub(crate) pixels: [Rgb565; SCRATCH_PIXELS],
    area: Rectangle,
}

impl UiScratch {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pixels: [Rgb565::BLACK; SCRATCH_PIXELS],
            area: Rectangle::new(Point::zero(), Size::zero()),
        }
    }

    pub(crate) fn prepare_stripe(&mut self, area: Rectangle) {
        debug_assert!(area.size.width <= STRIPE_WIDTH);
        debug_assert!(area.size.height <= MAX_SCREEN_HEIGHT);
        self.area = area;
    }

    fn pixel_index(&self, point: Point) -> Option<usize> {
        if !self.area.contains(point) {
            return None;
        }

        let x = usize::try_from(point.x - self.area.top_left.x).ok()?;
        let y = usize::try_from(point.y - self.area.top_left.y).ok()?;
        Some(y * STRIPE_WIDTH_USIZE + x)
    }
}

impl Default for UiScratch {
    fn default() -> Self {
        Self::new()
    }
}

impl Dimensions for UiScratch {
    fn bounding_box(&self) -> Rectangle {
        self.area
    }
}

impl DrawTarget for UiScratch {
    type Color = Rgb565;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(point, color) in pixels {
            if let Some(index) = self.pixel_index(point) {
                self.pixels[index] = color;
            }
        }
        Ok(())
    }

    fn fill_solid(&mut self, area: &Rectangle, color: Self::Color) -> Result<(), Self::Error> {
        let intersection = area.intersection(&self.area);
        let x_start = usize::try_from(intersection.top_left.x - self.area.top_left.x).unwrap_or(0);
        let y_start = usize::try_from(intersection.top_left.y - self.area.top_left.y).unwrap_or(0);
        let width = usize::try_from(intersection.size.width).unwrap_or(0);
        let height = usize::try_from(intersection.size.height).unwrap_or(0);

        for y in y_start..y_start + height {
            let start = y * STRIPE_WIDTH_USIZE + x_start;
            self.pixels[start..start + width].fill(color);
        }
        Ok(())
    }
}
