//! The day's step count, and how far it is through the daily goal.
//!
//! The watchface already shows the number, in one line among several. This
//! screen is what you open when the number is the thing you came for: the count
//! is set in the same numerals the FORGE face builds its clock from, at a third
//! of the size, and under it a gauge says how far through the day's goal it is.
//!
//! Nothing here counts anything. The accelerometer service owns the step
//! counter and publishes it; this screen keeps the last value it was handed,
//! whether or not it is showing - a count that arrived while the watchface was
//! up must not leave a stale number here.
//!
//! # The gauge
//!
//! Divided into five cells rather than drawn as one continuous bar, because a
//! continuous bar answers "roughly how far" and five cells answer "how many
//! thousands", which is the question somebody checking their steps is actually
//! asking. Each cell is two thousand steps.
//!
//! It turns green at the goal rather than simply filling up. A full bar and an
//! almost-full bar are a few pixels apart at this width, and the difference
//! between them is the whole point of having a goal.

use embedded_graphics::{
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
};
use heapless::String;
use pineforge_state::{AppEvent, ScreenAction};

use crate::canvas::{Canvas, CanvasError};
use crate::font::{JETBRAINS_MONO_6X14, hint_text};
use crate::{
    render::{PANEL, draw_mono_text_visible},
    screen::{Paint, Screen},
    segment::{SegmentSize, draw_cell, right_aligned},
    theme,
};

/// The day's goal, from the one place that decides product policy.
use pineforge_state::DAILY_STEP_GOAL as GOAL;
/// Cells in the gauge. Five at a two-thousand step each, so a glance reads
/// thousands rather than a proportion.
const GAUGE_CELLS: u32 = 5;
const _: () = assert!(
    GOAL.is_multiple_of(GAUGE_CELLS),
    "the gauge cannot divide the goal evenly"
);

/// Five places, because the goal has five digits and the count is allowed to
/// pass it. Kept as an `i32` for the layout and narrowed where the array needs
/// a length, rather than cast at every use.
const PLACES: i32 = 5;
const PLACE_COUNT: usize = 5;
/// A third of the watchface's numerals, which is what five of them across a
/// 240-pixel panel comes to.
const DIGIT: SegmentSize = SegmentSize::new(40, 54, 7);
const DIGIT_GAP: i32 = 5;
const DIGITS_WIDTH: i32 = PLACES * DIGIT.width + (PLACES - 1) * DIGIT_GAP;
const DIGITS_X: i32 = (PANEL.size.width.cast_signed() - DIGITS_WIDTH) / 2;
const DIGITS_Y: i32 = 62;

/// Baseline of the label over the count.
const LABEL_BASELINE_Y: i32 = 44;

/// The gauge, in the column a menu row and a notification card occupy.
const GAUGE_X: i32 = 20;
const GAUGE_WIDTH: i32 = 200;
const GAUGE_Y: i32 = 154;
const GAUGE_HEIGHT: i32 = 26;
/// Gap cut between one cell of the gauge and the next.
const GAUGE_DIVIDE: i32 = 3;

/// Baseline of the line under the gauge.
const FOOTER_BASELINE_Y: i32 = 210;
const HINT_WIDTH: i32 = JETBRAINS_MONO_6X14.cell.width.cast_signed();

/// The day's step count, as last published by the motion service.
#[derive(Default)]
pub struct StepsScreen {
    steps: Option<u32>,
    dirty: bool,
}

impl StepsScreen {
    /// Whether the last event left anything to repaint.
    #[must_use]
    pub const fn moved(&self) -> bool {
        self.dirty
    }

    /// Says the panel now shows the count this screen holds.
    pub const fn mark_painted(&mut self) {
        self.dirty = false;
    }

    /// The count, or zero before the first reading arrives.
    ///
    /// Zero rather than blank places: a watch that has not been walked and a
    /// watch that has not reported are the same thing to somebody looking at
    /// it, and a row of unlit cells would read as a fault.
    const fn count(&self) -> u32 {
        match self.steps {
            Some(steps) => steps,
            None => 0,
        }
    }

    /// Whether the day's goal is met, which is what the gauge changes colour
    /// for.
    const fn reached(&self) -> bool {
        self.count() >= GOAL
    }

    const fn gauge_ink(&self) -> Rgb565 {
        if self.reached() {
            theme::OK
        } else {
            theme::ACCENT
        }
    }

    /// How much of the gauge is filled, in pixels.
    ///
    /// Multiplied before dividing so a count under a hundredth of the goal
    /// still moves it, and clamped at the full width so passing the goal fills
    /// the gauge rather than running past its end.
    fn filled_width(&self) -> i32 {
        let filled = u64::from(self.count().min(GOAL)) * u64::from(GAUGE_WIDTH.unsigned_abs())
            / u64::from(GOAL);
        i32::try_from(filled).unwrap_or(GAUGE_WIDTH)
    }

    fn draw_gauge(&self, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        let filled = self.filled_width();
        // The track first, then what is filled over it - two rectangles rather
        // than three, and no arithmetic about where the unfilled part starts.
        fill(
            canvas,
            GAUGE_X,
            GAUGE_Y,
            GAUGE_WIDTH,
            GAUGE_HEIGHT,
            theme::SURFACE,
        )?;
        fill(
            canvas,
            GAUGE_X,
            GAUGE_Y,
            filled,
            GAUGE_HEIGHT,
            self.gauge_ink(),
        )?;

        // Cut the cells out of both at once, so a divide lands in the same
        // place whether the fill has reached it or not.
        for cell in 1..GAUGE_CELLS {
            let at = GAUGE_X + GAUGE_WIDTH * cell.cast_signed() / GAUGE_CELLS.cast_signed();
            fill(
                canvas,
                at - GAUGE_DIVIDE / 2,
                GAUGE_Y,
                GAUGE_DIVIDE,
                GAUGE_HEIGHT,
                theme::BACKGROUND,
            )?;
        }
        Ok(())
    }

    /// The count, in the watchface's numerals.
    fn draw_count(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        let cells = right_aligned::<PLACE_COUNT>(self.count());
        let ink = if self.reached() {
            theme::OK
        } else {
            theme::ACCENT
        };
        for (place, cell) in cells.into_iter().enumerate() {
            let x = DIGITS_X + i32::try_from(place).unwrap_or(0) * (DIGIT.width + DIGIT_GAP);
            draw_cell(canvas, DIGIT, x, DIGITS_Y, cell, ink)?;
            keep_alive();
        }
        Ok(())
    }

    /// What is left to the goal, or that it is done.
    fn footer(&self) -> String<24> {
        let mut line = String::new();
        if self.reached() {
            let _ = line.push_str("GOAL REACHED");
        } else {
            let _ = core::fmt::Write::write_fmt(
                &mut line,
                format_args!("{} TO GO", GOAL - self.count()),
            );
        }
        line
    }

    fn paint(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        let panel_right = PANEL.size.width.cast_signed();
        let panel_bottom = PANEL.size.height.cast_signed();

        // Opaque without a blanking pass, the way the watchfaces are: the bands
        // between the parts are filled, and every part covers its own area.
        fill(canvas, 0, 0, panel_right, DIGITS_Y, theme::BACKGROUND)?;
        keep_alive();
        draw_centred("STEPS", LABEL_BASELINE_Y, theme::TEXT, canvas)?;

        // Beside the digits, over their rows.
        fill(
            canvas,
            0,
            DIGITS_Y,
            DIGITS_X,
            DIGIT.height,
            theme::BACKGROUND,
        )?;
        let right_x = DIGITS_X + DIGITS_WIDTH;
        fill(
            canvas,
            right_x,
            DIGITS_Y,
            panel_right - right_x,
            DIGIT.height,
            theme::BACKGROUND,
        )?;
        // The gaps between the cells, which no cell covers.
        for place in 1..PLACE_COUNT {
            let x = DIGITS_X + i32::try_from(place).unwrap_or(0) * (DIGIT.width + DIGIT_GAP);
            fill(
                canvas,
                x - DIGIT_GAP,
                DIGITS_Y,
                DIGIT_GAP,
                DIGIT.height,
                theme::BACKGROUND,
            )?;
        }
        self.draw_count(canvas, keep_alive)?;

        // Between the digits and the gauge, the margins beside it, and the rest
        // of the panel under it.
        let digits_bottom = DIGITS_Y + DIGIT.height;
        fill(
            canvas,
            0,
            digits_bottom,
            panel_right,
            GAUGE_Y - digits_bottom,
            theme::BACKGROUND,
        )?;
        fill(canvas, 0, GAUGE_Y, GAUGE_X, GAUGE_HEIGHT, theme::BACKGROUND)?;
        let gauge_right = GAUGE_X + GAUGE_WIDTH;
        fill(
            canvas,
            gauge_right,
            GAUGE_Y,
            panel_right - gauge_right,
            GAUGE_HEIGHT,
            theme::BACKGROUND,
        )?;
        self.draw_gauge(canvas)?;
        keep_alive();

        let gauge_bottom = GAUGE_Y + GAUGE_HEIGHT;
        fill(
            canvas,
            0,
            gauge_bottom,
            panel_right,
            panel_bottom - gauge_bottom,
            theme::BACKGROUND,
        )?;
        let footer = self.footer();
        let ink = if self.reached() {
            theme::OK
        } else {
            theme::TEXT
        };
        draw_centred(&footer, FOOTER_BASELINE_Y, ink, canvas)
    }
}

impl Paint for StepsScreen {
    fn draw_full(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        self.paint(canvas, keep_alive)
    }
}

impl Screen for StepsScreen {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction {
        // A reading is not an interaction: it arrives on its own and only ever
        // changes what is shown. Everything else - a tick, a touch - leaves the
        // mark alone, so an event this screen does not care about cannot retire
        // a repaint that is still owed.
        if let AppEvent::StepsUpdated(steps) = event {
            if self.steps != Some(steps) {
                self.steps = Some(steps);
                self.dirty = true;
            }
        }
        ScreenAction::None
    }

    /// Repaints the count and the gauge, not the panel.
    ///
    /// The label and the bands around them do not change with the reading, and
    /// a step arriving is not a reason to send the whole screen again.
    fn draw_dirty(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        if !self.dirty {
            return Ok(());
        }
        self.draw_count(canvas, keep_alive)?;
        self.draw_gauge(canvas)?;
        keep_alive();
        // The footer counts down as the gauge fills, so it moves with them.
        let footer = self.footer();
        let ink = if self.reached() {
            theme::OK
        } else {
            theme::TEXT
        };
        fill(
            canvas,
            0,
            FOOTER_BASELINE_Y - JETBRAINS_MONO_6X14.baseline.cast_signed(),
            PANEL.size.width.cast_signed(),
            JETBRAINS_MONO_6X14.cell.height.cast_signed(),
            theme::BACKGROUND,
        )?;
        draw_centred(&footer, FOOTER_BASELINE_Y, ink, canvas)
    }
}

fn fill(
    canvas: &mut Canvas<'_>,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    color: Rgb565,
) -> Result<(), CanvasError> {
    if width <= 0 || height <= 0 {
        return Ok(());
    }
    Rectangle::new(
        Point::new(x, y),
        Size::new(width.unsigned_abs(), height.unsigned_abs()),
    )
    .into_styled(PrimitiveStyle::with_fill(color))
    .draw(canvas)
}

/// One line of the hint face, centred across the panel.
fn draw_centred(
    text: &str,
    baseline: i32,
    ink: Rgb565,
    canvas: &mut Canvas<'_>,
) -> Result<(), CanvasError> {
    let width = i32::try_from(text.len()).unwrap_or(0) * HINT_WIDTH;
    draw_mono_text_visible(
        text,
        Point::new((PANEL.size.width.cast_signed() - width) / 2, baseline),
        hint_text(ink, theme::BACKGROUND),
        canvas,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::Probe;

    fn walked(steps: u32) -> StepsScreen {
        let mut screen = StepsScreen::default();
        let _ = screen.handle_event(AppEvent::StepsUpdated(steps));
        screen
    }

    /// The gauge is the whole reason this screen exists rather than a line on
    /// the watchface, so its arithmetic is worth pinning at both ends.
    #[test]
    fn the_gauge_runs_from_empty_to_full_and_stops_there() {
        assert_eq!(walked(0).filled_width(), 0, "a fresh day showed progress");
        assert_eq!(
            walked(GOAL / 2).filled_width(),
            GAUGE_WIDTH / 2,
            "half the goal was not half the gauge"
        );
        assert_eq!(walked(GOAL).filled_width(), GAUGE_WIDTH);
        assert_eq!(
            walked(GOAL * 3).filled_width(),
            GAUGE_WIDTH,
            "passing the goal ran the gauge past its end"
        );
    }

    /// A full gauge and an almost-full one are a few pixels apart, so the
    /// colour is what actually carries whether the goal is met.
    #[test]
    fn the_goal_changes_the_colour_rather_than_only_the_length() {
        assert_eq!(walked(GOAL - 1).gauge_ink(), theme::ACCENT);
        assert_eq!(walked(GOAL).gauge_ink(), theme::OK);
    }

    /// A step count that did not move is not a repaint. The service publishes
    /// on a timer whether or not anyone walked.
    #[test]
    fn an_unchanged_count_costs_no_repaint() {
        let mut screen = walked(500);
        screen.mark_painted();
        let _ = screen.handle_event(AppEvent::StepsUpdated(500));
        assert!(!screen.moved(), "a repeated reading asked for a repaint");
        let _ = screen.handle_event(AppEvent::StepsUpdated(501));
        assert!(screen.moved(), "a moved reading asked for no repaint");
    }

    /// A tick arrives every second while this screen is up. It must not be able
    /// to retire a repaint the reading before it earned.
    #[test]
    fn a_tick_leaves_a_pending_repaint_alone() {
        let mut screen = walked(500);
        let _ = screen.handle_event(AppEvent::Tick {
            uptime_seconds: 1,
            wall_time: None,
            date: None,
        });
        assert!(
            screen.moved(),
            "a tick retired the repaint the count earned"
        );
    }

    /// The partial path may leave the rest of the panel alone, but what it does
    /// paint has to be the count and the gauge - not nothing.
    #[test]
    fn a_new_count_repaints_something_without_repainting_the_panel() {
        let screen = walked(1234);
        let mut probe = Probe::new();
        screen
            .draw_dirty(&mut Canvas::new(&mut probe), &mut || {})
            .expect("the probe accepts every operation");
        let painted = 240 * 240 - probe.unpainted();
        assert!(painted > 0, "a new count drew nothing");
        assert!(
            painted < 240 * 240,
            "a partial repaint covered the whole panel"
        );
    }
}
