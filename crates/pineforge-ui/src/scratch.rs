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
/// Rows a transition composes at a time.
///
/// Pure trade: `STRIPE_THICKNESS * 240 * 2` bytes of scratch against how many
/// stripes a slide takes. The picture is identical either way; only the number
/// of SPI rounds changes, 40 stripes here against the 20 that twelve would take.
///
/// It came down from 12 because the other side of that trade turned out to be
/// pairing. Every byte of static RAM is a byte the stack does not get -
/// `flip-link` puts the stack below the statics - and the deepest path this
/// firmware has is the elliptic-curve arithmetic of a key exchange, which the
/// controller runs in a high-priority interrupt nested on top of whatever was
/// drawing. Measured on hardware, that peak is 16,900 bytes.
///
/// At 12 the stack was 16,460 and every pairing attempt rebooted the watch. At
/// 10 it was 17,420, which worked with 3 % to spare. At 6 it is 19,336, which
/// leaves 14 %, and that margin is the point: 16,900 is one reading, and DFU, a
/// notification arriving mid-pairing and the alarm path are all outside it.
///
pub const STRIPE_THICKNESS: u32 = 6;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The one way the row walk above can be wrong, and nothing else sees it.
    ///
    /// Colours arrive across the caller's rectangle, not across the part of it
    /// that lands in the stripe. Advancing by the intersection's width instead
    /// pulls the next row's colours into this one - a shear, not a gap. It
    /// still writes every pixel, so the opacity checks that guard the screens
    /// stay green while the picture slides apart, and a transition is the only
    /// place it would ever show.
    #[test]
    fn a_rectangle_wider_than_the_stripe_keeps_its_rows_apart() {
        let mut scratch = UiScratch::new();
        scratch.prepare_stripe(Rectangle::new(Point::new(24, 0), Size::new(12, 240)));

        // Overhangs the stripe on both sides, the way a run of text does.
        let area = Rectangle::new(Point::new(20, 5), Size::new(20, 3));
        scratch
            .fill_contiguous(&area, (0..60).map(|index| Rgb565::new(0, index, 0)))
            .expect("the scratch accepts every write");

        for row in 0..3 {
            for column in 0..12 {
                // The stripe starts four columns into each row the caller
                // produced, and each row is twenty colours long.
                let expected = u8::try_from(row * 20 + 4 + column).unwrap();
                assert_eq!(
                    scratch.pixels[(5 + row) * scratch.stride() + column],
                    Rgb565::new(0, expected, 0),
                    "row {row}, column {column}"
                );
            }
        }
    }
}

fn as_usize(value: u32) -> usize {
    usize::try_from(value).unwrap_or(0)
}

/// How far `point` sits past `origin`, which is never negative for an
/// intersection measured against the rectangle it came from.
fn offset(point: i32, origin: i32) -> usize {
    usize::try_from(point - origin).unwrap_or(0)
}

/// Drops `count` colours, reporting whether any are left.
fn skip(colors: &mut impl Iterator<Item = Rgb565>, count: usize) -> bool {
    count == 0 || colors.nth(count - 1).is_some()
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

    /// Writes a rectangle of colours by walking rows, not points.
    ///
    /// Without this the crate's default stood in, and it is written for a target
    /// that has nothing better: it zips `area.points()` against the colours and
    /// hands each pair to [`Self::draw_iter`], so every pixel paid for a `Point`
    /// built and destructured, a `Rectangle::contains` against the stripe, two
    /// fallible casts, and a bounds-checked store. That is a lot of arithmetic
    /// to work out something the row already knows.
    ///
    /// This knows it once per row instead. Composing a screen is where the time
    /// goes - a measured transition spent 614 ms rastering against 176 ms
    /// sending - so the pixel loop here is the hot path of every navigation.
    ///
    /// The colours arrive row-major over `area`, which is the caller's
    /// rectangle and not the part of it that lands in the stripe. So the
    /// iterator is advanced across the whole row and only the middle of it is
    /// kept; taking the intersection's width instead would shear the image by
    /// pulling the next row's colours into this one.
    fn fill_contiguous<I>(&mut self, area: &Rectangle, colors: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Self::Color>,
    {
        let visible = area.intersection(&self.area);
        if visible.size.width == 0 || visible.size.height == 0 {
            return Ok(());
        }

        let row_length = as_usize(area.size.width);
        let before = offset(visible.top_left.x, area.top_left.x);
        let span = as_usize(visible.size.width);
        let after = row_length.saturating_sub(before + span);
        let rows_above = offset(visible.top_left.y, area.top_left.y);

        let mut colors = colors.into_iter();
        // A short iterator is allowed to end the fill early, so every advance
        // below can legitimately run out; what is already written stays.
        if !skip(&mut colors, rows_above * row_length) {
            return Ok(());
        }

        let x = offset(visible.top_left.x, self.area.top_left.x);
        let top = offset(visible.top_left.y, self.area.top_left.y);
        for row in 0..as_usize(visible.size.height) {
            if !skip(&mut colors, before) {
                return Ok(());
            }
            let start = (top + row) * self.stride + x;
            for slot in &mut self.pixels[start..start + span] {
                let Some(color) = colors.next() else {
                    return Ok(());
                };
                *slot = color;
            }
            if !skip(&mut colors, after) {
                return Ok(());
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
