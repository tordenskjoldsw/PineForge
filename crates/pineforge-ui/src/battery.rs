//! The charge, which way it is going, and the voltage behind both.
//!
//! The status corner already carries a rune and the About screen already lists
//! the millivolts. This screen is what you open when the charge is the thing you
//! came for: the percentage is set in the same numerals the FORGE face builds
//! its clock from, and under it the two facts the rune cannot fit - whether the
//! number is climbing or falling, and the cell voltage the estimate was made
//! from.
//!
//! Nothing here measures anything. The battery task owns the ADC and the two
//! charger pins and publishes what it read; this screen keeps the last record it
//! was handed, whether or not it is showing, so a reading that arrived while the
//! watchface was up does not leave a stale number here.
//!
//! # Why three states and not two
//!
//! A watch sitting on the charger with a full cell is neither charging nor
//! discharging, and the hardware says so directly: `power_present` without
//! `charging` is the charger holding the cell at the top. Calling that
//! DISCHARGING would be false and calling it CHARGING would be a number that
//! never moves, so it has its own word.

use embedded_graphics::{geometry::Point, pixelcolor::Rgb565};
use heapless::String;
use pineforge_state::{AppEvent, BatteryStatus, ScreenAction};

use crate::canvas::{Canvas, CanvasError};
use crate::font::{LIBERATION_MONO_10X22, LIBERATION_MONO_FORGE_12X27, ui_text};
use crate::{
    render::{PANEL, draw_instrument_centred, draw_mono_text_visible, fill},
    screen::{Paint, Screen},
    segment::{Cell, SegmentSize, draw_cell, right_aligned},
    theme,
};

/// Three places, because a full battery is 100 and the leading place blanks
/// itself at anything less.
const PLACES: i32 = 3;
const PLACE_COUNT: usize = 3;
/// The same numerals the steps screen uses, which is a third of the watchface's.
const DIGIT: SegmentSize = SegmentSize::new(40, 54, 7);
const DIGIT_GAP: i32 = 5;
const DIGITS_WIDTH: i32 = PLACES * DIGIT.width + (PLACES - 1) * DIGIT_GAP;

/// The percent sign, drawn to the digits' proportions rather than set in a face.
///
/// A glyph from the text face beside numerals this size reads as a footnote, and
/// the numerals are the point of the screen. Built from the same rectangles the
/// digits are, for the same reason the corner masks are: a diagonal from the
/// primitive library would pull in a rasteriser this firmware needs nowhere
/// else.
const PERCENT_WIDTH: i32 = 34;
const PERCENT_GAP: i32 = 10;
/// Side of the two rings, and the thickness of everything in the sign.
const PERCENT_RING: i32 = 15;
const PERCENT_STROKE: i32 = 5;
/// How tall a step of the slash is. Two keeps the edge from reading as stairs
/// without paying for a step per row.
const SLASH_STEP: i32 = 2;

/// Digits and sign together, centred across the panel.
const VALUE_WIDTH: i32 = DIGITS_WIDTH + PERCENT_GAP + PERCENT_WIDTH;
const DIGITS_X: i32 = (PANEL.size.width.cast_signed() - VALUE_WIDTH) / 2;
const PERCENT_X: i32 = DIGITS_X + DIGITS_WIDTH + PERCENT_GAP;
const DIGITS_Y: i32 = 66;

/// Baseline of the title over the number.
const TITLE_BASELINE_Y: i32 = 46;
/// Baseline of the direction under it.
const STATE_BASELINE_Y: i32 = 162;
/// Baseline of the voltage under that.
///
/// Set in the UI face rather than the instrument face the lines above use. The
/// FORGE face carries A to Z and nothing else - it was cut for labels - so a
/// voltage set in it would be a lone V with the number missing.
const VOLTS_BASELINE_Y: i32 = 206;
/// Width of one character of the UI face, for centring the voltage by hand.
const VOLTS_CHARACTER_WIDTH: i32 = LIBERATION_MONO_10X22.cell.width.cast_signed();

/// The band the two lines under the number occupy, cleared as one.
const LINES_TOP: i32 = STATE_BASELINE_Y - LIBERATION_MONO_FORGE_12X27.baseline.cast_signed();
const LINES_BOTTOM: i32 = VOLTS_BASELINE_Y - LIBERATION_MONO_10X22.baseline.cast_signed()
    + LIBERATION_MONO_10X22.cell.height.cast_signed();

/// The last battery record the task published.
#[derive(Default)]
pub struct BatteryScreen {
    status: Option<BatteryStatus>,
    dirty: bool,
}

impl BatteryScreen {
    /// Whether the last event left anything to repaint.
    #[must_use]
    pub const fn moved(&self) -> bool {
        self.dirty
    }

    /// Says the panel now shows the record this screen holds.
    pub const fn mark_painted(&mut self) {
        self.dirty = false;
    }

    /// The charge, as cells. Blank all the way across before the first reading,
    /// because a watch that has not reported and a flat watch are not the same
    /// thing and must not look alike.
    fn cells(&self) -> [Cell; PLACE_COUNT] {
        self.status.map_or([Cell::Blank; PLACE_COUNT], |status| {
            right_aligned::<PLACE_COUNT>(u32::from(status.percent))
        })
    }

    /// What the number is coloured by: the same urgency the corner rune uses, so
    /// one charge never reads as two different things in two places.
    fn ink(&self) -> Rgb565 {
        self.status
            .map_or(theme::MUTED, |status| theme::battery(status.level()))
    }

    /// Which way the charge is going.
    const fn state(&self) -> &'static str {
        match self.status {
            Some(status) if status.charging => "CHARGING",
            // On the pad, drawing nothing: the cell is full.
            Some(status) if status.power_present => "CHARGED",
            Some(_) => "DISCHARGING",
            None => "NO READING",
        }
    }

    const fn state_ink(&self) -> Rgb565 {
        match self.status {
            Some(status) if status.power_present => theme::OK,
            Some(_) => theme::ACCENT,
            None => theme::MUTED,
        }
    }

    /// The cell voltage, to two places.
    ///
    /// Millivolts are what the task measured; volts are what a battery is
    /// spoken about in, and the third digit of a millivolt reading is noise from
    /// a 12-bit conversion across a 1 `MOhm` divider.
    fn volts(&self) -> String<12> {
        let mut line = String::new();
        let Some(status) = self.status else {
            return line;
        };
        let _ = core::fmt::Write::write_fmt(
            &mut line,
            format_args!(
                "{}.{:02} V",
                status.millivolts / 1000,
                status.millivolts % 1000 / 10
            ),
        );
        line
    }

    /// The percent sign: two rings and the bar between them.
    fn draw_percent(&self, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        let ink = self.ink();
        fill(
            canvas,
            PERCENT_X,
            DIGITS_Y,
            PERCENT_WIDTH,
            DIGIT.height,
            theme::BACKGROUND,
        )?;
        for (x, y) in [
            (PERCENT_X, DIGITS_Y),
            (
                PERCENT_X + PERCENT_WIDTH - PERCENT_RING,
                DIGITS_Y + DIGIT.height - PERCENT_RING,
            ),
        ] {
            fill(canvas, x, y, PERCENT_RING, PERCENT_RING, ink)?;
            fill(
                canvas,
                x + PERCENT_STROKE,
                y + PERCENT_STROKE,
                PERCENT_RING - 2 * PERCENT_STROKE,
                PERCENT_RING - 2 * PERCENT_STROKE,
                theme::BACKGROUND,
            )?;
        }

        // The bar, as a stack of steps from the foot of the sign to its head.
        // The travel is divided by the height rather than accumulated, so the
        // last step lands on the corner however the two divide.
        let travel = PERCENT_WIDTH - PERCENT_STROKE;
        let mut top = 0;
        while top < DIGIT.height {
            let height = SLASH_STEP.min(DIGIT.height - top);
            let x = PERCENT_X + travel * (DIGIT.height - top - height) / DIGIT.height;
            fill(canvas, x, DIGITS_Y + top, PERCENT_STROKE, height, ink)?;
            top += SLASH_STEP;
        }
        Ok(())
    }

    /// The charge, in the watchface's numerals.
    fn draw_charge(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        let ink = self.ink();
        for (place, cell) in self.cells().into_iter().enumerate() {
            let x = DIGITS_X + i32::try_from(place).unwrap_or(0) * (DIGIT.width + DIGIT_GAP);
            draw_cell(canvas, DIGIT, x, DIGITS_Y, cell, ink)?;
            keep_alive();
        }
        self.draw_percent(canvas)
    }

    /// The two lines under the number, over one cleared band.
    ///
    /// One band rather than one each: they are adjacent, both are repainted
    /// together, and a single fill is a single transfer.
    fn draw_readings(&self, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        let width = PANEL.size.width.cast_signed();
        fill(
            canvas,
            0,
            LINES_TOP,
            width,
            LINES_BOTTOM - LINES_TOP,
            theme::BACKGROUND,
        )?;
        draw_instrument_centred(self.state(), STATE_BASELINE_Y, self.state_ink(), canvas)?;

        let volts = self.volts();
        let text_width = i32::try_from(volts.len()).unwrap_or(0) * VOLTS_CHARACTER_WIDTH;
        draw_mono_text_visible(
            &volts,
            Point::new((width - text_width) / 2, VOLTS_BASELINE_Y),
            ui_text(theme::TEXT, theme::BACKGROUND),
            canvas,
        )
    }

    fn paint(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        let panel_right = PANEL.size.width.cast_signed();
        let panel_bottom = PANEL.size.height.cast_signed();

        // Opaque without a blanking pass, the way the steps screen is: every
        // part covers its own area and the bands between them are filled, so
        // nothing is painted twice and no black frame is sent ahead of the
        // content.
        fill(canvas, 0, 0, panel_right, DIGITS_Y, theme::BACKGROUND)?;
        keep_alive();
        draw_instrument_centred("BATTERY", TITLE_BASELINE_Y, theme::ACCENT, canvas)?;

        // Beside the value, over its rows.
        fill(
            canvas,
            0,
            DIGITS_Y,
            DIGITS_X,
            DIGIT.height,
            theme::BACKGROUND,
        )?;
        let right_x = DIGITS_X + VALUE_WIDTH;
        fill(
            canvas,
            right_x,
            DIGITS_Y,
            panel_right - right_x,
            DIGIT.height,
            theme::BACKGROUND,
        )?;
        // The gaps between the cells, and the one before the sign, which no cell
        // covers.
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
        fill(
            canvas,
            PERCENT_X - PERCENT_GAP,
            DIGITS_Y,
            PERCENT_GAP,
            DIGIT.height,
            theme::BACKGROUND,
        )?;
        self.draw_charge(canvas, keep_alive)?;

        // Everything under the value, which the two lines are then set into.
        let digits_bottom = DIGITS_Y + DIGIT.height;
        fill(
            canvas,
            0,
            digits_bottom,
            panel_right,
            panel_bottom - digits_bottom,
            theme::BACKGROUND,
        )?;
        keep_alive();
        self.draw_readings(canvas)
    }
}

impl Paint for BatteryScreen {
    fn draw_full(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        self.paint(canvas, keep_alive)
    }
}

impl Screen for BatteryScreen {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction {
        // A reading is not an interaction: it arrives on its own and only ever
        // changes what is shown. Everything else - a tick, a touch - leaves the
        // mark alone, so an event this screen does not care about cannot retire
        // a repaint that is still owed.
        if let AppEvent::BatteryUpdated(status) = event
            && self.status != Some(status)
        {
            self.status = Some(status);
            self.dirty = true;
        }
        ScreenAction::None
    }

    /// Repaints the number and the two lines, not the panel.
    ///
    /// The title and the bands around them do not change with the reading, and
    /// a sample arriving is not a reason to send the whole screen again.
    fn draw_dirty(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        if !self.dirty {
            return Ok(());
        }
        self.draw_charge(canvas, keep_alive)?;
        keep_alive();
        self.draw_readings(canvas)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::Probe;

    fn reading(percent: u8, millivolts: u16, charging: bool, power_present: bool) -> BatteryScreen {
        let mut screen = BatteryScreen::default();
        let _ = screen.handle_event(AppEvent::BatteryUpdated(BatteryStatus {
            millivolts,
            percent,
            charging,
            power_present,
        }));
        screen
    }

    /// The three states the hardware can actually report, and the one that is
    /// easy to get wrong: on the pad with a full cell is not discharging.
    #[test]
    fn the_charger_pins_decide_the_word() {
        assert_eq!(reading(50, 3800, true, true).state(), "CHARGING");
        assert_eq!(reading(100, 4180, false, true).state(), "CHARGED");
        assert_eq!(reading(50, 3800, false, false).state(), "DISCHARGING");
        assert_eq!(BatteryScreen::default().state(), "NO READING");
    }

    /// Millivolts are the measurement; volts are what the screen says. The
    /// third digit is conversion noise and is deliberately not shown.
    #[test]
    fn the_voltage_reads_as_volts_to_two_places() {
        assert_eq!(reading(80, 4056, false, false).volts().as_str(), "4.05 V");
        assert_eq!(reading(10, 3600, false, false).volts().as_str(), "3.60 V");
        // A watch that has not reported has no voltage to show, rather than a
        // zero that would read as a measurement.
        assert!(BatteryScreen::default().volts().is_empty());
    }

    /// A blank row and a row of zeroes say different things, and a watch that
    /// has not reported must not claim to be flat.
    #[test]
    fn no_reading_shows_no_number() {
        assert_eq!(BatteryScreen::default().cells(), [Cell::Blank; PLACE_COUNT]);
        assert_eq!(
            reading(0, 3300, false, false).cells(),
            [Cell::Blank, Cell::Blank, Cell::Digit(0)]
        );
    }

    /// The battery task publishes on a timer whether or not the reading moved.
    #[test]
    fn an_unchanged_record_costs_no_repaint() {
        let mut screen = reading(64, 3900, false, false);
        screen.mark_painted();
        let _ = screen.handle_event(AppEvent::BatteryUpdated(BatteryStatus {
            millivolts: 3900,
            percent: 64,
            charging: false,
            power_present: false,
        }));
        assert!(!screen.moved(), "a repeated reading asked for a repaint");

        // The voltage moves long before the percentage does, and it is on the
        // screen, so it has to count as movement.
        let _ = screen.handle_event(AppEvent::BatteryUpdated(BatteryStatus {
            millivolts: 3890,
            percent: 64,
            charging: false,
            power_present: false,
        }));
        assert!(screen.moved(), "a moved voltage asked for no repaint");
    }

    /// A tick arrives every second while this screen is up. It must not be able
    /// to retire a repaint the reading before it earned.
    #[test]
    fn a_tick_leaves_a_pending_repaint_alone() {
        let mut screen = reading(64, 3900, false, false);
        let _ = screen.handle_event(AppEvent::Tick {
            uptime_seconds: 1,
            wall_time: None,
            date: None,
        });
        assert!(
            screen.moved(),
            "a tick retired the repaint the reading earned"
        );
    }

    /// The partial path may leave the rest of the panel alone, but what it does
    /// paint has to be the number and the lines - not nothing.
    #[test]
    fn a_new_reading_repaints_something_without_repainting_the_panel() {
        let screen = reading(64, 3900, false, false);
        let mut probe = Probe::new();
        screen
            .draw_dirty(&mut Canvas::new(&mut probe), &mut || {})
            .expect("the probe accepts every operation");
        let painted = 240 * 240 - probe.unpainted();
        assert!(painted > 0, "a new reading drew nothing");
        assert!(
            painted < 240 * 240,
            "a partial repaint covered the whole panel"
        );
    }
}
