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

use embedded_graphics::{
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
};
use pineforge_state::{WatchField, WatchFields, WatchState};

use crate::canvas::{Canvas, CanvasError};
use crate::segment::{Cell, SegmentSize, draw_cell, right_aligned};
use crate::{
    theme,
    watchface::{
        Watchface,
        row::{SCREEN_AREA, UNSYNCHRONIZED_DATE, fill_band},
    },
};

/// A digit's cell, and the thickness of the strokes inside it.
///
/// The shape itself lives in [`crate::segment`], which the steps app draws the
/// same numerals from at a third of this size. What is left here is how large
/// this face wants them.
const DIGIT: SegmentSize = SegmentSize::new(70, 80, 12);
const DIGIT_WIDTH: i32 = DIGIT.width;
const DIGIT_HEIGHT: i32 = DIGIT.height;

/// Left edges of the two digits in a pair, centred across the panel.
const LEFT_DIGIT_X: i32 = 40;
const RIGHT_DIGIT_X: i32 = 130;
/// Top edges of the hour and minute pairs.
const HOURS_Y: i32 = 18;
const MINUTES_Y: i32 = 108;
/// Where the digits stop and the readings begin.
const FOOTER_TOP: i32 = MINUTES_Y + DIGIT_HEIGHT;

/// The footer sits between the same edges the digits do.
///
/// It used to start at 20 while the digits ran from 40 to 200, so it hung off
/// to the left against nothing. Sharing their edges is what makes the face read
/// as one object rather than as a clock with a caption under it.
const FOOTER_X: i32 = LEFT_DIGIT_X;
const FOOTER_WIDTH: i32 = RIGHT_DIGIT_X + DIGIT_WIDTH - LEFT_DIGIT_X;
const FOOTER_RIGHT: i32 = FOOTER_X + FOOTER_WIDTH;

/// The footer's numerals: the clock's own shape, at a quarter of its size.
///
/// Segments, not type. This is the thing three earlier attempts at this footer
/// got wrong by rearranging: a line of text under the clock is a different
/// rendering technique from everything above it - an antialiased atlas against
/// rectangles whose unlit strokes are drawn - and no amount of moving it about
/// makes the two belong together. The face has no font on it at all now.
const FOOTER_DIGIT: SegmentSize = SegmentSize::new(16, 22, 3);
const FOOTER_GAP: i32 = 3;
const FOOTER_DIGITS_Y: i32 = FOOTER_TOP + 12;

/// Advance from one footer cell to the next.
const CELL_STEP: i32 = FOOTER_DIGIT.width + FOOTER_GAP;

/// The separator between month and day: one stroke, the same weight the
/// numerals are drawn at, so it reads as part of them rather than as punctuation
/// borrowed from somewhere else.
const SEPARATOR_WIDTH: i32 = 8;
const SEPARATOR_X: i32 = FOOTER_X + 2 * CELL_STEP;
const DAY_X: i32 = SEPARATOR_X + SEPARATOR_WIDTH + FOOTER_GAP;

/// The charge, right-aligned against the edge the digits end on.
///
/// Three places, because it reaches a hundred. The two it does not need most of
/// the time stand as unlit cells rather than closing up - which is what the
/// clock does with a leading zero and what makes a number here read as a
/// reading rather than as a word.
const CHARGE_PLACES: usize = 3;
const CHARGE_CELLS: i32 = 3;
const CHARGE_X: i32 = FOOTER_RIGHT - CHARGE_CELLS * CELL_STEP + FOOTER_GAP;

/// The charging bolt, left of the charge and centred on its cells.
const BOLT_WIDTH: i32 = 9;
const BOLT_HEIGHT: i32 = 13;
const BOLT_X: i32 = CHARGE_X - BOLT_WIDTH - 6;
const BOLT_Y: i32 = FOOTER_DIGITS_Y + (FOOTER_DIGIT.height - BOLT_HEIGHT) / 2;

fn fill(canvas: &mut Canvas<'_>, area: Rectangle, color: Rgb565) -> Result<(), CanvasError> {
    if area.size.width == 0 || area.size.height == 0 {
        return Ok(());
    }
    area.into_styled(PrimitiveStyle::with_fill(color))
        .draw(canvas)
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
            draw_cell(
                canvas,
                DIGIT,
                x,
                y,
                Cell::Digit(digits[index]),
                theme::ACCENT,
            )?;
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

    /// Two cells of a number at `x`, most significant first.
    fn draw_pair(
        canvas: &mut Canvas<'_>,
        x: i32,
        value: u8,
        ink: Rgb565,
    ) -> Result<(), CanvasError> {
        draw_cell(
            canvas,
            FOOTER_DIGIT,
            x,
            FOOTER_DIGITS_Y,
            Cell::Digit(value / 10),
            ink,
        )?;
        draw_cell(
            canvas,
            FOOTER_DIGIT,
            x + CELL_STEP,
            FOOTER_DIGITS_Y,
            Cell::Digit(value % 10),
            ink,
        )
    }

    /// The date and the charge, in the clock's own numerals.
    ///
    /// The date is set in [`theme::TEXT`] rather than the accent: the clock is
    /// what the accent is for here, and a second set of accent numerals would
    /// be a second clock. The charge keeps the colour its level earns, because
    /// that is the one reading whose colour is the message.
    fn draw_footer(state: &WatchState, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        let panel_bottom = SCREEN_AREA.top_left.y + SCREEN_AREA.size.height.cast_signed();
        fill_band(canvas, FOOTER_TOP, panel_bottom)?;

        // Day and month. The year is a third of the line for a number nobody
        // checks on a watch.
        let date = state.date().unwrap_or(UNSYNCHRONIZED_DATE);
        Self::draw_pair(canvas, FOOTER_X, date.month, theme::TEXT)?;
        fill(
            canvas,
            Rectangle::new(
                Point::new(
                    SEPARATOR_X,
                    FOOTER_DIGITS_Y + FOOTER_DIGIT.height / 2 - FOOTER_DIGIT.stroke / 2,
                ),
                Size::new(
                    SEPARATOR_WIDTH.unsigned_abs(),
                    FOOTER_DIGIT.stroke.unsigned_abs(),
                ),
            ),
            theme::SURFACE,
        )?;
        Self::draw_pair(canvas, DAY_X, date.day, theme::TEXT)?;

        // Nothing reported yet leaves every place unlit, which says "no
        // reading" the way an instrument does rather than by showing a zero.
        let (percent, ink, charging) =
            state
                .battery()
                .map_or((0, theme::SURFACE, false), |status| {
                    (
                        u32::from(status.percent),
                        theme::battery(status.level()),
                        status.charging,
                    )
                });
        let reading = state.battery().is_some();
        let places = right_aligned::<CHARGE_PLACES>(percent);
        for (place, cell) in places.into_iter().enumerate() {
            let x = CHARGE_X + i32::try_from(place).unwrap_or(0) * CELL_STEP;
            let cell = if reading { cell } else { Cell::Blank };
            draw_cell(canvas, FOOTER_DIGIT, x, FOOTER_DIGITS_Y, cell, ink)?;
        }

        // Current going in is a state the level cannot carry: a full battery is
        // green whether or not a charger is attached.
        if charging {
            draw_bolt(canvas, BOLT_X, BOLT_Y)?;
        }
        Ok(())
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
                    draw_cell(canvas, DIGIT, x, y, Cell::Digit(now[index]), theme::ACCENT)?;
                    keep_alive();
                }
            }
        }
        // The footer carries the date and every reading, so anything but the
        // clock moving is one line to repaint.
        // Steps and pulse are not shown here any more, so neither is a reason
        // to send the footer again.
        if changed.contains(WatchField::Date) || changed.contains(WatchField::Battery) {
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

    use super::{
        BOLT_HEIGHT, BOLT_WIDTH, BOLT_X, BOLT_Y, CHARGE_X, FOOTER_DIGIT, FOOTER_DIGITS_Y,
        ForgeWatchface,
    };
    use crate::{canvas::Canvas, probe::Probe, watchface::Watchface};

    /// The slot the bolt occupies, which is what a charging watch must fill and
    /// a discharging one must leave alone.
    fn bolt_slot() -> Rectangle {
        Rectangle::new(
            Point::new(BOLT_X, BOLT_Y),
            Size::new(BOLT_WIDTH.unsigned_abs(), BOLT_HEIGHT.unsigned_abs()),
        )
    }

    fn painted_with(charging: bool) -> Probe {
        let mut state = WatchState::new();
        let _ = state.apply(AppEvent::BatteryUpdated(BatteryStatus {
            millivolts: 3_700,
            // Deliberately a Low level: `theme::battery` paints a Good battery
            // in the same green as the bolt, so at 80% the reading's own cells
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

    /// And no bolt without a charger. Checked by the bolt's own colour rather
    /// than by coverage, because the footer paints that band either way.
    #[test]
    fn a_discharging_watch_draws_no_bolt() {
        assert!(
            !painted_with(false).painted_in(bolt_slot(), crate::theme::OK),
            "a discharging watch drew a bolt"
        );
    }

    /// The bolt has its own room. It used to sit where the charge reading began
    /// and pushed it sideways; a reading that moves when a charger is plugged in
    /// is a reading that cannot be compared with itself.
    #[test]
    fn the_bolt_does_not_stand_in_the_charges_cells() {
        assert!(
            BOLT_X + BOLT_WIDTH < CHARGE_X,
            "the bolt overlaps the first charge cell"
        );
        let cells = FOOTER_DIGIT.cell(CHARGE_X, FOOTER_DIGITS_Y);
        assert_eq!(
            bolt_slot().intersection(&cells).size,
            Size::zero(),
            "the bolt and the charge share pixels"
        );
    }
}
