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

/// Thickness of a transition stripe, across whichever axis it spans.
///
/// This is the whole cost of the slide animation, and it trades RAM against
/// render time: the screen is composed once per stripe, so halving the
/// thickness halves the buffer and doubles the number of clipped full draws a
/// transition performs. 12 pixels divides the 240-pixel span evenly into 20
/// stripes and leaves the static RAM budget with headroom for the screens still
/// to come; 8 remains available if a later feature needs the space more than the
/// animation needs the speed.
pub const STRIPE_THICKNESS: u32 = 12;
const MAX_SCREEN_EDGE: usize = 240;
const SCRATCH_PIXELS: usize = STRIPE_THICKNESS as usize * MAX_SCREEN_EDGE;

/// The display task's reusable 5.6 KiB rendering workspace.
///
/// It currently acts as a clipped RGB565 target for slide transitions. Keeping
/// the storage in this operation-neutral type makes its exclusivity explicit:
/// later transient UI operations extend this owner rather than reserve another
/// large static buffer.
///
/// A stripe is stored row-major at its own width, so the same allocation serves
/// an upright stripe of a horizontal slide and a flat one of a vertical slide.
pub struct UiScratch {
    pub(crate) pixels: [Rgb565; SCRATCH_PIXELS],
    area: Rectangle,
    stride: usize,
}

impl UiScratch {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pixels: [Rgb565::BLACK; SCRATCH_PIXELS],
            area: Rectangle::new(Point::zero(), Size::zero()),
            stride: 0,
        }
    }

    pub(crate) fn prepare_stripe(&mut self, area: Rectangle) {
        let width = area.size.width as usize;
        let height = area.size.height as usize;
        debug_assert!(width.saturating_mul(height) <= SCRATCH_PIXELS);
        self.area = area;
        self.stride = width;
    }

    /// Row stride of the prepared stripe, needed to read it back out.
    pub(crate) const fn stride(&self) -> usize {
        self.stride
    }

    fn pixel_index(&self, point: Point) -> Option<usize> {
        if !self.area.contains(point) {
            return None;
        }

        let x = usize::try_from(point.x - self.area.top_left.x).ok()?;
        let y = usize::try_from(point.y - self.area.top_left.y).ok()?;
        Some(y * self.stride + x)
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
            let start = y * self.stride + x_start;
            self.pixels[start..start + width].fill(color);
        }
        Ok(())
    }
}
