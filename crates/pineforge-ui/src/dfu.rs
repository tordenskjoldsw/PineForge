use core::fmt::Write;

use embedded_graphics::{
    prelude::*,
    primitives::Rectangle,
    text::{Alignment, Text},
};
use heapless::String;
use pineforge_state::DfuFailReason;

use crate::canvas::{Canvas, CanvasError};
use crate::font::{hint_text, ui_text};
use crate::{
    render::{draw_progress_bar, draw_visible},
    theme,
};

const TITLE_BASELINE: Point = Point::new(120, 90);
const PERCENT_BASELINE: Point = Point::new(120, 138);
/// The band the percentage can occupy at any width, cleared before it is
/// redrawn. The text is centred, so it grows in both directions and a shorter
/// number would otherwise leave the tail of a longer one standing beside it.
const PERCENT_AREA: Rectangle = Rectangle::new(Point::new(94, 119), Size::new(52, 26));
/// The progress bar, sized and placed so the three elements above it centre on
/// the panel. Sixteen pixels rather than the twenty-four it was: the corner
/// mask is five rows deep, and on a shallower bar that same curve reads as a
/// rounded bar instead of a box with the corners knocked off.
const BAR: Rectangle = Rectangle::new(Point::new(20, 152), Size::new(200, 16));

/// Repaints only what a percentage step changes: the number and the bar.
///
/// A transfer reports about a hundred steps, and repainting the whole screen
/// for each is what makes the progress screen flicker - the clear leaves the
/// panel blank for the length of an SPI frame, once per percent. The title and
/// the hint do not move, so they are drawn once and left alone.
pub fn refresh_progress(
    canvas: &mut Canvas<'_>,
    percent: u8,
    keep_alive: &mut dyn FnMut(),
) -> Result<(), CanvasError> {
    let percent = percent.min(100);
    canvas.fill_solid(&PERCENT_AREA, theme::BACKGROUND)?;
    draw_percent(canvas, percent)?;
    keep_alive();
    fill_bar(canvas, percent)
}

fn draw_percent(canvas: &mut Canvas<'_>, percent: u8) -> Result<(), CanvasError> {
    let mut label: String<8> = String::new();
    let _ = write!(label, "{percent}%");
    draw_visible(
        &Text::with_alignment(
            &label,
            PERCENT_BASELINE,
            ui_text(theme::ACCENT, theme::BACKGROUND),
            Alignment::Center,
        ),
        canvas,
    )
}

/// The transfer's progress, in the bar every screen reporting progress uses.
fn fill_bar(canvas: &mut Canvas<'_>, percent: u8) -> Result<(), CanvasError> {
    let filled = u32::from(percent) * BAR.size.width / 100;
    draw_progress_bar(canvas, &BAR, filled, theme::ACCENT)
}

/// Draws a full-screen firmware-update progress screen, mirroring
/// `InfiniTime`'s DFU screen: a title, a percentage, and a fill bar.
pub fn draw_dfu_progress(
    canvas: &mut Canvas<'_>,
    percent: u8,
    keep_alive: &mut dyn FnMut(),
) -> Result<(), CanvasError> {
    let percent = percent.min(100);
    canvas.clear(theme::BACKGROUND)?;
    draw_visible(
        &Text::with_alignment(
            "FIRMWARE UPDATE",
            TITLE_BASELINE,
            ui_text(theme::TEXT, theme::BACKGROUND),
            Alignment::Center,
        ),
        canvas,
    )?;
    keep_alive();

    draw_percent(canvas, percent)?;
    keep_alive();

    fill_bar(canvas, percent)?;
    keep_alive();
    Ok(())
}

/// Draws the power-loss-safe, one-time storage formatting progress.
pub fn draw_storage_progress(
    canvas: &mut Canvas<'_>,
    percent: u8,
    keep_alive: &mut dyn FnMut(),
) -> Result<(), CanvasError> {
    let percent = percent.min(100);
    canvas.clear(theme::BACKGROUND)?;
    draw_visible(
        &Text::with_alignment(
            "PREPARING STORAGE",
            TITLE_BASELINE,
            ui_text(theme::TEXT, theme::BACKGROUND),
            Alignment::Center,
        ),
        canvas,
    )?;
    keep_alive();

    draw_percent(canvas, percent)?;
    fill_bar(canvas, percent)?;
    keep_alive();
    // This one keeps its line. "Safe to restart" is not reassurance, it is the
    // answer to the question a first boot that sits on a progress bar actually
    // raises - the format is power-loss safe and losing patience costs nothing.
    draw_visible(
        &Text::with_alignment(
            "Safe to restart",
            Point::new(120, 196),
            hint_text(theme::TEXT, theme::BACKGROUND),
            Alignment::Center,
        ),
        canvas,
    )?;
    keep_alive();
    Ok(())
}

/// Draws a full-screen notice that an update did not happen.
///
/// The transfer was abandoned, the host stopped sending, or the watch declined
/// it outright. There is no wire protocol error for most of that, so the
/// watch's own screen is the only place it can be surfaced. The specific
/// reason, including the flash's JEDEC id when the chip was not recognized, is
/// shown to make a sealed watch diagnosable without a debug port.
///
/// A refusal is titled and coloured differently from a fault. Telling somebody
/// their watch failed when it deliberately protected its rollback image sends
/// them looking for a defect that is not there.
pub fn draw_dfu_failed(
    canvas: &mut Canvas<'_>,
    reason: DfuFailReason,
    keep_alive: &mut dyn FnMut(),
) -> Result<(), CanvasError> {
    let (headline, ink) = match reason {
        DfuFailReason::NotConfirmed => ("UPDATE REFUSED", theme::WARN),
        _ => ("UPDATE FAILED", theme::DANGER),
    };
    canvas.clear(theme::BACKGROUND)?;
    draw_visible(
        &Text::with_alignment(
            headline,
            Point::new(120, 108),
            ui_text(ink, theme::BACKGROUND),
            Alignment::Center,
        ),
        canvas,
    )?;
    keep_alive();

    let mut detail: String<64> = String::new();
    match reason {
        DfuFailReason::FlashUnrecognized([a, b, c]) => {
            let _ = write!(detail, "Flash not recognized\nid {a:02x} {b:02x} {c:02x}");
        }
        DfuFailReason::FlashInitFailed => {
            let _ = detail.push_str("Flash did not respond");
        }
        DfuFailReason::EraseFailed => {
            let _ = detail.push_str("Sector erase failed");
        }
        DfuFailReason::ProgramFailed => {
            let _ = detail.push_str("Page program failed");
        }
        DfuFailReason::VerifyFailed => {
            let _ = detail.push_str("Write verify failed");
        }
        DfuFailReason::TimedOut => {
            let _ = detail.push_str("Transfer stalled\nstart it again");
        }
        DfuFailReason::NotConfirmed => {
            let _ = detail.push_str("This build is unconfirmed\nOpen FIRMWARE to confirm");
        }
    }
    draw_visible(
        &Text::with_alignment(
            &detail,
            Point::new(120, 145),
            hint_text(theme::TEXT, theme::BACKGROUND),
            Alignment::Center,
        ),
        canvas,
    )?;
    keep_alive();

    draw_visible(
        &Text::with_alignment(
            "Swipe to dismiss",
            Point::new(120, 190),
            hint_text(theme::ACCENT, theme::BACKGROUND),
            Alignment::Center,
        ),
        canvas,
    )?;
    keep_alive();
    Ok(())
}
