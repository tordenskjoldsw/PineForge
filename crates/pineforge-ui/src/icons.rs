//! Monochrome 24 x 24 pictograms, one bit per pixel.
//!
//! Bitmaps rather than drawing code: a gear built from arcs would pull in a
//! rasteriser the firmware needs nowhere else, while a row of bits costs four
//! bytes and blits with a loop. They are tinted at draw time, so one bitmap
//! serves a tile in both its normal and its pressed colours.

use crate::canvas::{Canvas, CanvasError};
use embedded_graphics::{
    draw_target::DrawTarget, pixelcolor::Rgb565, prelude::*, primitives::Rectangle,
};

pub const ICON_SIZE: i32 = 24;

/// One row per pixel row, most significant bit leftmost.
pub type Icon = [u32; ICON_SIZE as usize];

/// Settings.
pub const GEAR: Icon = [
    0b0000_0000_0000_0000_0000_0000,
    0b0000_0000_0011_1100_0000_0000,
    0b0000_0000_0001_1000_0000_0000,
    0b0000_0100_0001_1000_0010_0000,
    0b0000_1100_1111_1111_0011_0000,
    0b0001_1111_1111_1111_1111_1000,
    0b0000_0111_1111_1111_1110_0000,
    0b0000_0111_1111_1111_1110_0000,
    0b0000_1111_1110_0111_1111_0000,
    0b0000_1111_1000_0001_1111_0000,
    0b0100_1111_1000_0001_1111_0010,
    0b0111_1111_0000_0000_1111_1110,
    0b0111_1111_0000_0000_1111_1110,
    0b0100_1111_1000_0001_1111_0010,
    0b0000_1111_1000_0001_1111_0000,
    0b0000_1111_1110_0111_1111_0000,
    0b0000_0111_1111_1111_1110_0000,
    0b0000_0111_1111_1111_1110_0000,
    0b0001_1111_1111_1111_1111_1000,
    0b0000_1100_1111_1111_0011_0000,
    0b0000_0100_0001_1000_0010_0000,
    0b0000_0000_0001_1000_0000_0000,
    0b0000_0000_0011_1100_0000_0000,
    0b0000_0000_0000_0000_0000_0000,
];

/// Firmware and recovery.
pub const CHIP: Icon = [
    0b0000_0000_0000_0000_0000_0000,
    0b0000_0000_0000_0000_0000_0000,
    0b0000_0000_0000_0000_0000_0000,
    0b0000_0000_1100_0111_0000_0000,
    0b0000_0000_1100_0111_0000_0000,
    0b0000_0000_1100_0111_0000_0000,
    0b0000_0011_1111_1111_1100_0000,
    0b0000_0011_1111_1111_1100_0000,
    0b0001_1111_1111_1111_1111_1000,
    0b0001_1111_1000_0001_1111_1000,
    0b0000_0011_1000_0001_1100_0000,
    0b0000_0011_1000_0001_1100_0000,
    0b0000_0011_1000_0001_1100_0000,
    0b0001_1111_1000_0001_1111_1000,
    0b0001_1111_1000_0001_1111_1000,
    0b0001_1111_1111_1111_1111_1000,
    0b0000_0011_1111_1111_1100_0000,
    0b0000_0011_1111_1111_1100_0000,
    0b0000_0000_1100_0111_0000_0000,
    0b0000_0000_1100_0111_0000_0000,
    0b0000_0000_1100_0111_0000_0000,
    0b0000_0000_0000_0000_0000_0000,
    0b0000_0000_0000_0000_0000_0000,
    0b0000_0000_0000_0000_0000_0000,
];

/// Touch diagnostics.
#[cfg(feature = "diagnostics")]
pub const CROSSHAIR: Icon = [
    0b0000_0000_0000_0000_0000_0000,
    0b0000_0000_0000_0000_0000_0000,
    0b0000_0000_0111_1110_0000_0000,
    0b0000_0001_1111_1111_1000_0000,
    0b0000_0111_1000_0001_1110_0000,
    0b0000_1110_0000_0000_0111_0000,
    0b0000_1100_0000_1100_0011_0000,
    0b0001_1000_0000_1100_0001_1000,
    0b0001_1000_0000_1100_0001_1000,
    0b0011_0000_0000_1100_0000_1100,
    0b0011_0000_0001_1110_0000_1100,
    0b0011_0011_1111_1111_1100_1100,
    0b0011_0011_1111_1111_1100_1100,
    0b0011_0000_0001_1110_0000_1100,
    0b0011_0000_0000_1100_0000_1100,
    0b0001_1000_0000_1100_0001_1000,
    0b0001_1000_0000_1100_0001_1000,
    0b0000_1100_0000_1100_0011_0000,
    0b0000_1110_0000_0000_0111_0000,
    0b0000_0111_1000_0001_1110_0000,
    0b0000_0001_1111_1111_1000_0000,
    0b0000_0000_0111_1110_0000_0000,
    0b0000_0000_0000_0000_0000_0000,
    0b0000_0000_0000_0000_0000_0000,
];

/// Draws the set bits of an icon in one colour, leaving the rest untouched.
///
/// Clear bits are skipped rather than painted in the background colour, so an
/// icon can sit on a filled tile without knowing what it is standing on.
pub fn draw_icon(
    icon: &Icon,
    top_left: Point,
    color: Rgb565,
    canvas: &mut Canvas<'_>,
) -> Result<(), CanvasError> {
    let area = Rectangle::new(
        top_left,
        Size::new(
            u32::try_from(ICON_SIZE).unwrap_or(0),
            u32::try_from(ICON_SIZE).unwrap_or(0),
        ),
    );
    if area.intersection(&canvas.bounding_box()).size == Size::zero() {
        return Ok(());
    }

    canvas.draw_iter(icon.iter().enumerate().flat_map(|(row, bits)| {
        let y = top_left.y + i32::try_from(row).unwrap_or(0);
        (0..ICON_SIZE).filter_map(move |column| {
            let shift = u32::try_from(ICON_SIZE - 1 - column).unwrap_or(0);
            (bits >> shift & 1 == 1).then(|| Pixel(Point::new(top_left.x + column, y), color))
        })
    }))
}
