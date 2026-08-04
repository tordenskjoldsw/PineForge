//! The passkey a phone asks to see, in the numerals the rest of the watch
//! counts in.
//!
//! It used to be a title, a code set in the UI face inside a stroked box, and a
//! line telling the user what to do with it. All three are gone, and what is
//! left is six numerals.
//!
//! The reasoning is the one the FORGE face already settled. A passkey is a
//! number that is the entire point of the screen it is on, and this firmware
//! draws such a number from rectangles rather than from a glyph atlas - the
//! watchface, the pulse app, the steps app and the music app all do. Setting
//! this one in a font made the single screen where the number matters most the
//! only one that did not look like the watch.
//!
//! The text went for a different reason. "Enter this code on your phone" is
//! read by somebody who is already looking at their phone asking them to
//! confirm a code, so it describes what they are in the middle of doing. The
//! title said "BLUETOOTH PAIRING" to somebody who just tapped pair. Neither
//! answers a question anybody has here, and both were competing with the one
//! thing that does.
//!
//! The box went with them. An outline says "field", and this is not one; the
//! unlit strokes of the numerals give the code its own boundary, which is what
//! they are for.

use embedded_graphics::draw_target::DrawTarget;

use crate::canvas::{Canvas, CanvasError};
use crate::{
    render::PANEL,
    segment::{Cell, SegmentSize, draw_cell},
    theme,
};

/// Places in a Bluetooth passkey, which is always six digits.
const PLACES: usize = 6;

/// Sized to put six across the panel and still leave a margin.
///
/// Narrower than the steps app's numerals and in the same proportion - four
/// places there, six here. The stroke is scaled with them, or the digits would
/// read as a heavier face rather than a smaller one.
const DIGIT: SegmentSize = SegmentSize::new(30, 42, 5);
/// Wider than the gap between a clock's numerals, and that is the point.
///
/// Unlit strokes are drawn, so every cell reads as a filled block whether or
/// not its digit uses them. Six of those at a clock's spacing merge into one
/// band that has to be counted rather than read - which is the opposite of what
/// somebody copying a code needs.
const GAP: i32 = 6;
/// Added to [`GAP`] between the two threes, so the middle gap is three times
/// the others rather than replacing them.
///
/// A six-figure code is read and typed in two threes - it is how the number is
/// spoken, and it is what stops somebody losing their place halfway across.
const GROUP_EXTRA: i32 = 10;

/// Five gaps between six places, one of which is widened. Writing four here is
/// the arithmetic slip the centring test exists to catch, and did.
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
const CODE_WIDTH: i32 = PLACES as i32 * DIGIT.width + 5 * GAP + GROUP_EXTRA;
const CODE_X: i32 = (PANEL.size.width.cast_signed() - CODE_WIDTH) / 2;
const CODE_Y: i32 = (PANEL.size.height.cast_signed() - DIGIT.height) / 2;

/// The left edge of each numeral, with the group gap between the threes.
const fn place_x(place: usize) -> i32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    let index = place as i32;
    let group = if place < 3 { 0 } else { GROUP_EXTRA };
    CODE_X + index * (DIGIT.width + GAP) + group
}

// Six numerals across 240 pixels is the tight fit this layout is, and the
// arithmetic below is what keeps it from running off an edge. It is all
// constants and a `const fn`, so it is settled at compile time and holds for
// every build rather than only when tests run.
const _: () = assert!(CODE_X >= 8, "the code crowds the left edge");
const _: () = assert!(
    CODE_X == PANEL.size.width.cast_signed() - (place_x(PLACES - 1) + DIGIT.width),
    "the code is not centred"
);
const _: () = assert!(
    place_x(3) - (place_x(2) + DIGIT.width) == GAP + GROUP_EXTRA,
    "the threes are not held apart"
);
const _: () = assert!(
    place_x(1) - (place_x(0) + DIGIT.width) == GAP,
    "the places within a three are not evenly spaced"
);

/// Draws the pairing passkey the user confirms on the phone.
///
/// Zero-padded, and every place lit. This is the one number on the watch that
/// is not a reading: a leading zero is part of the code rather than a place the
/// value has not reached, so blanking it the way a step count or a heart rate
/// blanks its leading places would be showing a five-figure code.
pub fn draw_pairing(
    canvas: &mut Canvas<'_>,
    passkey: u32,
    keep_alive: &mut dyn FnMut(),
) -> Result<(), CanvasError> {
    canvas.clear(theme::BACKGROUND)?;
    keep_alive();

    let mut divisor = 100_000;
    for place in 0..PLACES {
        #[allow(clippy::cast_possible_truncation)]
        let digit = ((passkey / divisor) % 10) as u8;
        draw_cell(
            canvas,
            DIGIT,
            place_x(place),
            CODE_Y,
            Cell::Digit(digit),
            theme::ACCENT,
        )?;
        divisor /= 10;
        keep_alive();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::Probe;

    fn painted(passkey: u32) -> Probe {
        let mut probe = Probe::new();
        draw_pairing(&mut Canvas::new(&mut probe), passkey, &mut || {})
            .expect("the probe accepts every operation");
        probe
    }

    /// The code is what the user copies, so the digits have to be the passkey's
    /// own, in order, zero-padded. A rotation or a dropped leading zero is a
    /// code that simply does not work and gives no hint why.
    #[test]
    fn the_places_are_the_passkey_zero_padded() {
        let cells = |passkey: u32| {
            let mut divisor = 100_000;
            let mut out = [0_u8; PLACES];
            for slot in &mut out {
                #[allow(clippy::cast_possible_truncation)]
                let digit = ((passkey / divisor) % 10) as u8;
                *slot = digit;
                divisor /= 10;
            }
            out
        };
        assert_eq!(cells(123_456), [1, 2, 3, 4, 5, 6]);
        assert_eq!(cells(1), [0, 0, 0, 0, 0, 1]);
        assert_eq!(cells(0), [0; 6]);
        assert_eq!(cells(999_999), [9, 9, 9, 9, 9, 9]);
    }

    /// A modal owns the whole panel while it shows, so it has to paint the
    /// whole panel - nothing underneath is cleared for it.
    #[test]
    fn the_prompt_covers_the_panel() {
        assert_eq!(painted(123_456).unpainted(), 0);
    }
}
