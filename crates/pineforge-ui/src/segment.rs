//! Numerals built from rectangles, at whatever size the caller needs.
//!
//! This is the FORGE watchface's numeral, lifted out of it so a second screen
//! can be set in the same type. It was worth lifting for the reason it was
//! worth writing: at the sizes this firmware wants a number to be read across a
//! room, a glyph atlas is the expensive way to get one. The face's own digits
//! are 70 by 80 pixels, which as a font would cost around 23 KB - against seven
//! rectangles and a table of which are lit.
//!
//! Rectangles are also what the panel is fastest at: a filled rectangle is one
//! address window and one stream of pixels. And they make the partial redraw
//! trivial, because a digit that changed repaints its own cell and nothing
//! around it.
//!
//! Unlit strokes are drawn too, in the surface colour. They cost four more
//! rectangles per digit and they are what makes a number read as an instrument
//! rather than as a font that happens to be square - which is the whole look
//! this module exists to share.

use embedded_graphics::{
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
};

use crate::canvas::{Canvas, CanvasError};
use crate::theme;

/// How large a numeral is, and how heavy its strokes are.
///
/// Carried as a value rather than baked in as constants, which is the whole
/// difference from the version that lived inside the watchface: the face wants
/// digits that fill half the panel, and a screen showing five of them wants
/// digits a third of that. Both want the same shape.
#[derive(Clone, Copy)]
pub struct SegmentSize {
    pub width: i32,
    pub height: i32,
    pub stroke: i32,
}

impl SegmentSize {
    #[must_use]
    pub const fn new(width: i32, height: i32, stroke: i32) -> Self {
        Self {
            width,
            height,
            stroke,
        }
    }

    /// Where the two strokes of a vertical pair meet.
    const fn waist(self) -> i32 {
        self.height / 2
    }

    /// The cell a numeral occupies, which is what a partial redraw repaints.
    #[must_use]
    pub const fn cell(self, x: i32, y: i32) -> Rectangle {
        Rectangle::new(
            Point::new(x, y),
            Size::new(self.width.unsigned_abs(), self.height.unsigned_abs()),
        )
    }

    /// The seven strokes of a numeral whose cell begins at `x`, `y`.
    ///
    /// Returned as a fixed array rather than drawn here so that the lit and
    /// unlit passes cannot disagree about where a stroke is.
    fn strokes(self, x: i32, y: i32) -> [Rectangle; 7] {
        let rect = |x: i32, y: i32, width: i32, height: i32| {
            Rectangle::new(
                Point::new(x, y),
                Size::new(width.unsigned_abs(), height.unsigned_abs()),
            )
        };
        let right = x + self.width - self.stroke;
        let middle = y + self.waist() - self.stroke / 2;
        // From the middle stroke down to the bottom, so the lower pair meets
        // both.
        let lower = self.height - self.waist() + self.stroke / 2;
        [
            rect(x, y, self.width, self.stroke),       // top
            rect(right, y, self.stroke, self.waist()), // upper right
            rect(right, middle, self.stroke, lower),   // lower right
            rect(x, y + self.height - self.stroke, self.width, self.stroke), // bottom
            rect(x, middle, self.stroke, lower),       // lower left
            rect(x, y, self.stroke, self.waist()),     // upper left
            rect(x, middle, self.width, self.stroke),  // middle
        ]
    }
}

/// Which strokes each numeral lights, in the order [`SegmentSize::strokes`]
/// returns them.
///
/// The conventional seven-segment order, so the table reads the same as every
/// other one: top, upper right, lower right, bottom, lower left, upper left,
/// middle. Bit 0 is the top stroke, and the digit grouping is only the usual
/// four-from-the-right - it does not line up with the strokes.
const LIT: [u8; 10] = [
    0b011_1111, // 0
    0b000_0110, // 1
    0b101_1011, // 2
    0b100_1111, // 3
    0b110_0110, // 4
    0b110_1101, // 5
    0b111_1101, // 6
    0b000_0111, // 7
    0b111_1111, // 8
    0b110_1111, // 9
];

/// What a cell shows: a numeral, or the place one would occupy.
///
/// A blank is not an empty cell. It is every stroke unlit, which is what an
/// instrument does with a place it is not using - and it keeps a right-aligned
/// number from shifting under the eye as it gains a digit. Padding with zeroes
/// instead would say the count begins with a zero, which it does not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    Digit(u8),
    Blank,
}

fn fill(canvas: &mut Canvas<'_>, area: Rectangle, color: Rgb565) -> Result<(), CanvasError> {
    if area.size.width == 0 || area.size.height == 0 {
        return Ok(());
    }
    area.into_styled(PrimitiveStyle::with_fill(color))
        .draw(canvas)
}

/// Paints one cell over its whole area.
///
/// Opaque by construction: the cell is filled before the strokes go on, so a
/// caller repainting a single digit never has to know what was there before.
pub fn draw_cell(
    canvas: &mut Canvas<'_>,
    size: SegmentSize,
    x: i32,
    y: i32,
    cell: Cell,
    lit_color: Rgb565,
) -> Result<(), CanvasError> {
    fill(canvas, size.cell(x, y), theme::BACKGROUND)?;
    let lit = match cell {
        Cell::Digit(value) => LIT[usize::from(value.min(9))],
        Cell::Blank => 0,
    };
    for (index, stroke) in size.strokes(x, y).into_iter().enumerate() {
        let color = if lit & (1 << index) != 0 {
            lit_color
        } else {
            theme::SURFACE
        };
        fill(canvas, stroke, color)?;
    }
    Ok(())
}

/// A number as `N` cells, right-aligned, with unused places left blank.
///
/// Saturates rather than wrapping: a count too large for the places given shows
/// every nine, which is wrong by a knowable amount and reads as a limit. Taking
/// the low digits instead would show a small number for a large one.
#[must_use]
pub fn right_aligned<const N: usize>(value: u32) -> [Cell; N] {
    let mut cells = [Cell::Blank; N];
    let mut left = value;
    for place in (0..N).rev() {
        #[allow(clippy::cast_possible_truncation)]
        let digit = (left % 10) as u8;
        cells[place] = Cell::Digit(digit);
        left /= 10;
        if left == 0 {
            break;
        }
    }
    if left > 0 {
        cells = [Cell::Digit(9); N];
    }
    cells
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A blank is a place, not an absence: the number stays where it is as it
    /// gains and loses digits.
    #[test]
    fn a_short_number_keeps_its_places() {
        assert_eq!(
            right_aligned::<5>(742),
            [
                Cell::Blank,
                Cell::Blank,
                Cell::Digit(7),
                Cell::Digit(4),
                Cell::Digit(2)
            ]
        );
    }

    #[test]
    fn a_number_that_fills_every_place_has_no_blanks() {
        assert_eq!(
            right_aligned::<5>(10_000),
            [
                Cell::Digit(1),
                Cell::Digit(0),
                Cell::Digit(0),
                Cell::Digit(0),
                Cell::Digit(0)
            ]
        );
    }

    /// Zero is a digit, not an empty display.
    #[test]
    fn zero_shows_a_zero() {
        assert_eq!(right_aligned::<5>(0)[4], Cell::Digit(0));
        assert_eq!(right_aligned::<5>(0)[3], Cell::Blank);
    }

    /// A count past the places given reads as a limit rather than as a small
    /// number, which is what taking the low digits would have shown.
    #[test]
    fn an_overlong_number_saturates_rather_than_wrapping() {
        assert_eq!(right_aligned::<5>(123_456), [Cell::Digit(9); 5]);
    }

    /// The strokes have to cover the cell they claim, or a partial redraw
    /// leaves a seam where the previous digit showed through.
    #[test]
    fn the_strokes_stay_inside_their_cell() {
        let size = SegmentSize::new(40, 54, 7);
        let cell = size.cell(10, 20);
        for stroke in size.strokes(10, 20) {
            assert_eq!(
                stroke.intersection(&cell).size,
                stroke.size,
                "a stroke left its cell"
            );
        }
    }
}
