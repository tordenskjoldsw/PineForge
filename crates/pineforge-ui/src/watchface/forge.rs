//! `PineForge`'s own face: the time in numerals built from rectangles.
//!
//! The terminal face is borrowed - it is `InfiniTime`'s look, kept because it is
//! a good one. This is what falls out of this firmware's own constraints
//! instead.
//!
//! Nothing here is stored. A digit is seven rectangles and a table saying which
//! of them are lit, so the face carries no glyph atlas at all: the numerals it
//! draws at 70 by 80 pixels would cost around 23 KB as a font, against the
//! 54 KB of flash the image has left. It is the same principle the status corner
//! already follows, which draws its symbols from geometry rather than from
//! bitmaps.
//!
//! Rectangles are also what the panel is fastest at - a filled rectangle is one
//! address window and one stream of pixels - and they make the partial redraw
//! trivial: a minute passing repaints one digit's cell, not a row of text and
//! not the screen.
//!
//! Unlit segments are drawn too, in the surface colour. They cost four more
//! rectangles per digit and they are what makes the face read as an instrument
//! rather than as a font that happens to be square.

use core::fmt::Write;

use embedded_graphics::{
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
};
use heapless::String;
use pineforge_state::{HeartRateState, WatchField, WatchFields, WatchState};

use crate::canvas::{Canvas, CanvasError};
use crate::font::{JETBRAINS_MONO_6X14, hint_text};
use crate::{
    render::draw_mono_text_visible,
    theme,
    watchface::{
        Watchface,
        row::{SCREEN_AREA, UNSYNCHRONIZED_DATE, fill_band, format_date},
    },
};

/// A digit's cell, and the thickness of the strokes inside it.
const DIGIT_WIDTH: i32 = 70;
const DIGIT_HEIGHT: i32 = 80;
const STROKE: i32 = 12;
/// Where the two strokes of a vertical pair meet.
const WAIST: i32 = DIGIT_HEIGHT / 2;

/// Left edges of the two digits in a pair, centred across the panel.
const LEFT_DIGIT_X: i32 = 40;
const RIGHT_DIGIT_X: i32 = 130;
/// Top edges of the hour and minute pairs.
const HOURS_Y: i32 = 18;
const MINUTES_Y: i32 = 108;
/// Advance of the smaller face, for placing text after text by hand.
const HINT_WIDTH: i32 = JETBRAINS_MONO_6X14.cell.width.cast_signed();
/// The bolt's slot, and where the charge reading starts when it is occupied.
///
/// The slot is not held open while discharging. Holding it open kept the line
/// from shifting when a charger went on or off, which sounded right and was the
/// wrong trade: the shift happens at the moment of plugging in, which nobody is
/// reading the watch during, while the gap it left sat there the rest of the
/// time. So the reading starts flush at [`BOLT_X`] with no charger, and moves
/// right to make room for the bolt when there is one.
const BOLT_X: i32 = 20;
const BOLT_WIDTH: i32 = 9;
const BOLT_HEIGHT: i32 = 13;
const CHARGE_X: i32 = BOLT_X + BOLT_WIDTH + 4;

/// Where the digits stop and the readings begin.
const FOOTER_TOP: i32 = MINUTES_Y + DIGIT_HEIGHT;
const DATE_BASELINE: i32 = FOOTER_TOP + 20;
const READINGS_BASELINE: i32 = FOOTER_TOP + 42;

/// Which strokes each numeral lights, in the order [`strokes`] returns them.
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

/// The seven strokes of a numeral whose cell begins at `x`, `y`.
///
/// Returned as a fixed array rather than drawn here so that the lit and unlit
/// passes cannot disagree about where a stroke is.
fn strokes(x: i32, y: i32) -> [Rectangle; 7] {
    let rect = |x: i32, y: i32, width: i32, height: i32| {
        Rectangle::new(
            Point::new(x, y),
            Size::new(
                u32::try_from(width).unwrap_or(0),
                u32::try_from(height).unwrap_or(0),
            ),
        )
    };
    let right = x + DIGIT_WIDTH - STROKE;
    let middle = y + WAIST - STROKE / 2;
    // From the middle stroke down to the bottom, so the lower pair meets both.
    let lower = DIGIT_HEIGHT - WAIST + STROKE / 2;
    [
        rect(x, y, DIGIT_WIDTH, STROKE),                         // top
        rect(right, y, STROKE, WAIST),                           // upper right
        rect(right, middle, STROKE, lower),                      // lower right
        rect(x, y + DIGIT_HEIGHT - STROKE, DIGIT_WIDTH, STROKE), // bottom
        rect(x, middle, STROKE, lower),                          // lower left
        rect(x, y, STROKE, WAIST),                               // upper left
        rect(x, middle, DIGIT_WIDTH, STROKE),                    // middle
    ]
}

/// The cell a numeral occupies, which is what a partial redraw repaints.
fn cell(x: i32, y: i32) -> Rectangle {
    Rectangle::new(
        Point::new(x, y),
        Size::new(
            u32::try_from(DIGIT_WIDTH).unwrap_or(0),
            u32::try_from(DIGIT_HEIGHT).unwrap_or(0),
        ),
    )
}

fn fill(canvas: &mut Canvas<'_>, area: Rectangle, color: Rgb565) -> Result<(), CanvasError> {
    if area.size.width == 0 || area.size.height == 0 {
        return Ok(());
    }
    area.into_styled(PrimitiveStyle::with_fill(color))
        .draw(canvas)
}

/// Paints one numeral over its whole cell.
///
/// Opaque by construction: the cell is filled before the strokes go on, so a
/// caller repainting a single digit never has to know what was there before.
fn draw_digit(canvas: &mut Canvas<'_>, x: i32, y: i32, value: u8) -> Result<(), CanvasError> {
    fill(canvas, cell(x, y), theme::BACKGROUND)?;
    let lit = LIT[usize::from(value.min(9))];
    for (index, stroke) in strokes(x, y).into_iter().enumerate() {
        let is_lit = lit & (1 << index) != 0;
        let color = if is_lit {
            theme::ACCENT
        } else {
            theme::SURFACE
        };
        fill(canvas, stroke, color)?;
    }
    Ok(())
}

/// A lightning bolt, stepped out of five rectangles.
///
/// The same trick as the numerals, for the same reason: a bitmap would be the
/// eighth icon in a set this face otherwise does not use, and the shape only
/// has to read at a glance rather than survive scrutiny. Stepping the rows is
/// what gives it the lean that says bolt rather than cross.
///
/// It is the charging indicator. The colour beside it cannot carry that on its
/// own - `theme::battery` already answers to the charge level, and a full
/// battery reads green whether or not a charger is attached - so the state gets
/// a shape of its own.
fn draw_bolt(canvas: &mut Canvas<'_>, x: i32, y: i32) -> Result<(), CanvasError> {
    // Rows from the top down: the upper stroke leans right, the waist crosses,
    // and the lower stroke leans left.
    let rows = [
        (x + 4, y, 4, 3),
        (x + 2, y + 3, 4, 3),
        (x, y + 6, BOLT_WIDTH, 2),
        (x + 3, y + 8, 4, 3),
        (x + 1, y + 11, 4, 2),
    ];
    for (x, y, width, height) in rows {
        fill(
            canvas,
            Rectangle::new(
                Point::new(x, y),
                Size::new(
                    u32::try_from(width).unwrap_or(0),
                    u32::try_from(height).unwrap_or(0),
                ),
            ),
            theme::OK,
        )?;
    }
    Ok(())
}

/// The hours and minutes of the clock, as four numerals.
const fn digits_of(seconds: u64) -> [u8; 4] {
    let of_day = seconds % 86_400;
    let hours = of_day / 3_600;
    let minutes = (of_day / 60) % 60;
    #[allow(clippy::cast_possible_truncation)]
    [
        (hours / 10) as u8,
        (hours % 10) as u8,
        (minutes / 10) as u8,
        (minutes % 10) as u8,
    ]
}

/// Where each of the four numerals sits.
const PLACES: [(i32, i32); 4] = [
    (LEFT_DIGIT_X, HOURS_Y),
    (RIGHT_DIGIT_X, HOURS_Y),
    (LEFT_DIGIT_X, MINUTES_Y),
    (RIGHT_DIGIT_X, MINUTES_Y),
];

/// Holds no readings of its own; it renders the shared [`WatchState`].
#[derive(Default)]
pub struct ForgeWatchface;

impl ForgeWatchface {
    /// Paints the four numerals and the background around them.
    ///
    /// The bands between and beside the cells are filled here rather than by a
    /// blanking pass over the whole panel, for the reason the terminal face
    /// stopped blanking: the cells are painted anyway, and filling them twice
    /// was most of the cost of drawing a face.
    fn draw_clock(
        state: &WatchState,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        let digits = digits_of(state.clock_seconds());
        for (index, (x, y)) in PLACES.into_iter().enumerate() {
            draw_digit(canvas, x, y, digits[index])?;
            keep_alive();
        }
        Ok(())
    }

    /// Everything in the clock area that is not a numeral's cell.
    fn draw_clock_surround(canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        let gap_x = LEFT_DIGIT_X + DIGIT_WIDTH;
        let gap_width = RIGHT_DIGIT_X - gap_x;
        let right_x = RIGHT_DIGIT_X + DIGIT_WIDTH;
        let right_width = SCREEN_AREA.size.width.cast_signed() - right_x;

        // Above the hours, between the two pairs, and below the minutes.
        fill_band(canvas, SCREEN_AREA.top_left.y, HOURS_Y)?;
        fill_band(canvas, HOURS_Y + DIGIT_HEIGHT, MINUTES_Y)?;

        // The columns beside the digits, over both rows at once.
        for (y, height) in [(HOURS_Y, DIGIT_HEIGHT), (MINUTES_Y, DIGIT_HEIGHT)] {
            for (x, width) in [
                (SCREEN_AREA.top_left.x, LEFT_DIGIT_X),
                (gap_x, gap_width),
                (right_x, right_width),
            ] {
                fill(
                    canvas,
                    Rectangle::new(
                        Point::new(x, y),
                        Size::new(
                            u32::try_from(width).unwrap_or(0),
                            u32::try_from(height).unwrap_or(0),
                        ),
                    ),
                    theme::BACKGROUND,
                )?;
            }
        }
        Ok(())
    }

    /// The date and the readings, under the clock.
    ///
    /// One line of text apiece, in the smaller face: the numerals are what this
    /// watchface is for, and a reading competing with them for attention would
    /// be the wrong trade.
    fn draw_footer(state: &WatchState, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        let panel_bottom = SCREEN_AREA.top_left.y + SCREEN_AREA.size.height.cast_signed();
        fill_band(canvas, FOOTER_TOP, panel_bottom)?;

        let style = hint_text(theme::TEXT, theme::BACKGROUND);
        let date = format_date(state.date().unwrap_or(UNSYNCHRONIZED_DATE));
        draw_mono_text_visible(&date, Point::new(20, DATE_BASELINE), style, canvas)?;

        // The charge sits apart from the rest, because it is the one reading
        // whose colour carries meaning: level while running on the battery, and
        // the bolt beside it while current is going in.
        let mut charge: String<16> = String::new();
        let mut charge_color = theme::TEXT;
        let mut charging = false;
        match state.battery() {
            Some(status) => {
                // `PowerSource::label` rather than a tag spelled out here: it is
                // the product's answer to what the three states are called, and
                // the terminal face reads the same one.
                let _ = write!(charge, "{}% {}", status.percent, status.source().label());
                charge_color = theme::battery(status.level());
                charging = status.charging;
            }
            None => {
                let _ = charge.push_str("--%");
            }
        }
        let charge_x = if charging {
            draw_bolt(canvas, BOLT_X, READINGS_BASELINE - BOLT_HEIGHT)?;
            CHARGE_X
        } else {
            BOLT_X
        };
        draw_mono_text_visible(
            &charge,
            Point::new(charge_x, READINGS_BASELINE),
            hint_text(charge_color, theme::BACKGROUND),
            canvas,
        )?;

        let mut rest: String<32> = String::new();
        if let HeartRateState::Result(bpm) = state.heart_rate() {
            let _ = write!(rest, " {bpm} BPM");
        }
        if let Some(steps) = state.steps() {
            let _ = write!(rest, " {steps} ST");
        }
        let rest_x = charge_x + i32::try_from(charge.len()).unwrap_or(0) * HINT_WIDTH;
        draw_mono_text_visible(
            &rest,
            Point::new(rest_x, READINGS_BASELINE),
            hint_text(theme::ACCENT, theme::BACKGROUND),
            canvas,
        )
    }
}

impl Watchface for ForgeWatchface {
    fn draw_full(
        &self,
        state: &WatchState,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        Self::draw_clock_surround(canvas)?;
        keep_alive();
        Self::draw_clock(state, canvas, keep_alive)?;
        Self::draw_footer(state, canvas)?;
        keep_alive();
        Ok(())
    }

    fn draw_changed(
        &self,
        state: &WatchState,
        changed: WatchFields,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        if changed.contains(WatchField::Clock) {
            // Only the numerals that actually differ. At a minute boundary that
            // is one cell, and for three ticks in four it is nothing at all -
            // this face shows no seconds, so most clock ticks cost no pixels.
            let before = digits_of(state.previous_clock_seconds());
            let now = digits_of(state.clock_seconds());
            for (index, (x, y)) in PLACES.into_iter().enumerate() {
                if before[index] != now[index] {
                    draw_digit(canvas, x, y, now[index])?;
                    keep_alive();
                }
            }
        }
        // The footer carries the date and every reading, so anything but the
        // clock moving is one line to repaint.
        if changed.contains(WatchField::Date)
            || changed.contains(WatchField::Battery)
            || changed.contains(WatchField::Steps)
            || changed.contains(WatchField::HeartRate)
        {
            Self::draw_footer(state, canvas)?;
            keep_alive();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use embedded_graphics::{
        geometry::{Point, Size},
        primitives::Rectangle,
    };
    use pineforge_state::{AppEvent, BatteryStatus, WatchState};

    use super::{BOLT_HEIGHT, BOLT_WIDTH, BOLT_X, ForgeWatchface, READINGS_BASELINE};
    use crate::{canvas::Canvas, probe::Probe, watchface::Watchface};

    /// The slot the bolt occupies, which is what a charging watch must fill and
    /// a discharging one must leave alone.
    fn bolt_slot() -> Rectangle {
        Rectangle::new(
            Point::new(BOLT_X, READINGS_BASELINE - BOLT_HEIGHT),
            Size::new(
                u32::try_from(BOLT_WIDTH).unwrap(),
                u32::try_from(BOLT_HEIGHT).unwrap(),
            ),
        )
    }

    fn painted_with(charging: bool) -> Probe {
        let mut state = WatchState::new();
        let _ = state.apply(AppEvent::BatteryUpdated(BatteryStatus {
            millivolts: 3_700,
            // Deliberately a Low level: `theme::battery` paints a Good battery
            // in the same green as the bolt, so at 80% the reading's own text
            // would answer the colour question the bolt is being asked.
            percent: 30,
            charging,
            power_present: charging,
        }));
        let mut probe = Probe::new();
        ForgeWatchface
            .draw_full(&state, &mut Canvas::new(&mut probe), &mut || {})
            .expect("the probe accepts every operation");
        probe
    }

    /// The charge colour cannot carry this on its own: `theme::battery` answers
    /// to the level, so a full battery is green whether or not a charger is
    /// attached. The bolt is the only thing that distinguishes the two.
    #[test]
    fn the_bolt_appears_only_while_charging() {
        assert!(
            painted_with(true).painted_within(bolt_slot()),
            "a charging watch drew no bolt"
        );
    }

    /// And no bolt without a charger - which is not the same as an empty slot,
    /// since the reading now starts there instead. Checked by the bolt's colour
    /// rather than by coverage, because the text covers those pixels too.
    #[test]
    fn a_discharging_watch_draws_no_bolt() {
        let probe = painted_with(false);
        assert!(
            !probe.painted_in(bolt_slot(), crate::theme::OK),
            "a discharging watch drew a bolt"
        );
    }

    /// The reading moves into the space the bolt vacates rather than leaving a
    /// hole in front of itself.
    #[test]
    fn the_reading_starts_flush_when_nothing_is_charging() {
        let probe = painted_with(false);
        assert!(
            probe.painted_other_than(bolt_slot(), crate::theme::BACKGROUND),
            "the charge reading did not move into the empty bolt slot"
        );
    }
}
