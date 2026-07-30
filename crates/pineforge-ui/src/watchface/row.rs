//! Row primitives shared by the terminal-styled faces.
//!
//! A face that does not lay its readings out as labelled rows - an analog one,
//! say - simply does not use this module.

use core::fmt::Write;

use embedded_graphics::{
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
};
use heapless::String;
use pineforge_state::{BleState, CalendarDate};

use crate::canvas::{Canvas, CanvasError};
use crate::font::{JETBRAINS_MONO_10X22, ui_text};
use crate::render::draw_mono_text_visible;

pub const ROW_HEIGHT: u32 = 25;
const VALUE_X: i32 = 70;
/// Advance of the UI face, used to place partial value redraws by character.
///
/// Read from the font rather than written out, so a later change of face moves
/// the redraw boundaries with it instead of silently shifting them off the
/// glyphs they are meant to land between.
const GLYPH_WIDTH: i32 = JETBRAINS_MONO_10X22.cell.width.cast_signed();
/// Baseline of a row's text, measured from its top edge.
const BASELINE_OFFSET: i32 = 20;

pub const SCREEN_AREA: Rectangle = Rectangle::new(Point::new(0, 0), Size::new(240, 240));
/// The rows every terminal-styled face opens with, in the same places. Below
/// them a face lays out whatever it is for, and where its last row falls is its
/// own business - which is why the status row is not here.
pub const DATE_ROW: Rectangle = row_at(25);
pub const TIME_ROW: Rectangle = row_at(50);
pub const BATTERY_ROW: Rectangle = row_at(75);

pub const LIGHT_GRAY: Rgb565 = Rgb565::new(20, 40, 20);
pub const TERMINAL_GREEN: Rgb565 = Rgb565::new(4, 51, 10);
pub const TERMINAL_BLUE: Rgb565 = Rgb565::new(0, 31, 31);
pub const TERMINAL_ORANGE: Rgb565 = Rgb565::new(31, 32, 0);
pub const TERMINAL_RED: Rgb565 = Rgb565::new(31, 12, 0);

/// Stand-in date shown until a phone synchronizes the clock over BLE.
pub const UNSYNCHRONIZED_DATE: CalendarDate = CalendarDate {
    year: 2026,
    month: 1,
    day: 1,
};

pub const fn row_at(y: i32) -> Rectangle {
    Rectangle::new(Point::new(0, y), Size::new(240, ROW_HEIGHT))
}

/// Formats a time of day; before synchronization the uptime stands in for it,
/// so the seconds wrap at a day rather than counting past 24 hours.
pub fn format_clock(seconds: u64) -> String<16> {
    let of_day = seconds % 86_400;
    let hours = of_day / 3_600;
    let minutes = (of_day / 60) % 60;
    let seconds = of_day % 60;
    let mut clock = String::new();
    let _ = write!(clock, "{hours:02}:{minutes:02}:{seconds:02}");
    clock
}

pub fn format_date(date: CalendarDate) -> String<16> {
    let mut value = String::new();
    let _ = write!(value, "{:04}-{:02}-{:02}", date.year, date.month, date.day);
    value
}

/// Padded to a fixed width so partial redraws blank a longer prior value.
pub fn format_ble(state: BleState) -> String<16> {
    let mut value = String::new();
    match state {
        BleState::Off => {
            let _ = value.push_str("off        ");
        }
        BleState::Advertising => {
            let _ = value.push_str("advertising");
        }
        BleState::Pairing(passkey) => {
            let _ = write!(value, "PAIR {passkey:06}");
        }
        BleState::Connected => {
            let _ = value.push_str("connected  ");
        }
        BleState::DfuProgress(percent) => {
            let _ = write!(value, "DFU {percent}%");
        }
        BleState::DfuFailed(_) => {
            let _ = value.push_str("DFU failed ");
        }
    }
    value
}

/// A passkey during pairing takes visual priority.
pub const fn ble_color(state: BleState) -> Rgb565 {
    if matches!(state, BleState::Pairing(_)) {
        TERMINAL_ORANGE
    } else {
        TERMINAL_BLUE
    }
}

/// Fills a rectangle with the face's background, skipping empty ones.
///
/// Sizes arrive as signed edges from layout arithmetic and a negative width is
/// simply nothing to paint, which is what makes the callers below readable.
fn fill(
    canvas: &mut Canvas<'_>,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Result<(), CanvasError> {
    if width <= 0 || height <= 0 {
        return Ok(());
    }
    Rectangle::new(
        Point::new(x, y),
        Size::new(
            u32::try_from(width).unwrap_or(0),
            u32::try_from(height).unwrap_or(0),
        ),
    )
    .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
    .draw(canvas)
}

/// Fills the full-width band between two `y` edges with the background.
///
/// The counterpart to [`draw`] for the strips a face leaves between or around
/// its rows: together they let a face cover the panel without a blanking pass.
/// Named by its edges rather than by a height because that is how a layout
/// reads - the band runs from where the last thing ended to where the next
/// begins, and getting that from a subtraction at the call site is how a seam
/// of a few pixels appears.
pub fn fill_band(canvas: &mut Canvas<'_>, from_y: i32, to_y: i32) -> Result<(), CanvasError> {
    fill(
        canvas,
        SCREEN_AREA.top_left.x,
        from_y,
        SCREEN_AREA.size.width.cast_signed(),
        to_y - from_y,
    )
}

/// Draws a labelled row, covering **every pixel** of `area`.
///
/// Opacity is the contract, not a detail. [`Watchface::draw_full`] must leave no
/// pixel untouched, because the slide transition composes a face stripe by
/// stripe and never clears behind it. A face built from these rows inherits that
/// guarantee here, which is what lets it skip blanking the panel first - the
/// terminal face used to, and then painted most of those pixels a second time
/// with the glyph backgrounds on top. Two thirds of a full repaint was the
/// blanking pass.
///
/// Every glyph still paints its own background, so redrawing a row with
/// unchanged text leaves the panel visually untouched; clearing the row first
/// would flash it black for the length of the SPI transfer. What is filled here
/// is only what no glyph covers:
///
/// - the slack above and below the glyph band, since the cell is shorter than
///   the row;
/// - the gap between the end of the label and the value column;
/// - the tail a longer previous value may have left standing.
///
/// The band is derived from the font's own metrics rather than written out, so
/// changing the face moves these fills with it instead of opening a seam that
/// only shows up against a bright screen.
///
/// [`Watchface::draw_full`]: super::Watchface::draw_full
pub fn draw(
    canvas: &mut Canvas<'_>,
    area: Rectangle,
    label: &str,
    value: &str,
    value_color: Rgb565,
) -> Result<(), CanvasError> {
    let baseline = area.top_left.y + BASELINE_OFFSET;
    let label_style = ui_text(Rgb565::WHITE, Rgb565::BLACK);
    let value_style = ui_text(value_color, Rgb565::BLACK);

    let left = area.top_left.x;
    let top = area.top_left.y;
    let width = i32::try_from(area.size.width).unwrap_or(0);
    let bottom = top + i32::try_from(area.size.height).unwrap_or(0);
    let right = left + width;

    // Where the glyph cells actually sit, taken from the face being drawn in.
    let band_top = baseline - JETBRAINS_MONO_10X22.baseline.cast_signed();
    let band_bottom = band_top + JETBRAINS_MONO_10X22.cell.height.cast_signed();

    // Above and below the text, full width.
    fill(canvas, left, top, width, band_top - top)?;
    fill(canvas, left, band_bottom, width, bottom - band_bottom)?;

    draw_mono_text_visible(label, Point::new(left, baseline), label_style, canvas)?;
    let label_end = left + i32::try_from(label.len()).unwrap_or(0) * GLYPH_WIDTH;
    // Between the label and the value column.
    fill(
        canvas,
        label_end,
        band_top,
        VALUE_X - label_end,
        band_bottom - band_top,
    )?;

    draw_mono_text_visible(value, Point::new(VALUE_X, baseline), value_style, canvas)?;
    let value_end = VALUE_X + i32::try_from(value.len()).unwrap_or(0) * GLYPH_WIDTH;
    // Whatever a longer previous value may have left standing.
    fill(
        canvas,
        value_end,
        band_top,
        right - value_end,
        band_bottom - band_top,
    )
}

/// Redraws a row's value from the first character that differs, which for a
/// ticking clock is usually a single digit.
pub fn draw_changed_value(
    canvas: &mut Canvas<'_>,
    area: Rectangle,
    old: &str,
    new: &str,
    color: Rgb565,
) -> Result<(), CanvasError> {
    let first_changed = old
        .bytes()
        .zip(new.bytes())
        .position(|(old, new)| old != new)
        .unwrap_or_else(|| old.len().min(new.len()));
    if first_changed == new.len() && old.len() == new.len() {
        return Ok(());
    }

    let style = ui_text(color, Rgb565::BLACK);
    let x = VALUE_X + i32::try_from(first_changed).unwrap_or(0) * GLYPH_WIDTH;
    draw_mono_text_visible(
        &new[first_changed..],
        Point::new(x, area.top_left.y + BASELINE_OFFSET),
        style,
        canvas,
    )
}
