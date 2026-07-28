use core::fmt::Write;

use embedded_graphics::{
    prelude::*,
    primitives::{PrimitiveStyleBuilder, Rectangle},
    text::{Alignment, Text},
};
use heapless::String;
use pineforge_state::DfuFailReason;

use crate::canvas::{Canvas, CanvasError};
use crate::font::{hint_text, ui_text};
use crate::{render::draw_visible, theme};

const BAR: Rectangle = Rectangle::new(Point::new(20, 128), Size::new(200, 24));
const PERCENT_BASELINE: Point = Point::new(120, 108);
/// The band the percentage can occupy at any width, cleared before it is
/// redrawn. The text is centred, so it grows in both directions and a shorter
/// number would otherwise leave the tail of a longer one standing beside it.
const PERCENT_AREA: Rectangle = Rectangle::new(Point::new(94, 89), Size::new(52, 24));

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

/// The bar's interior: filled up to `percent`, background beyond it.
///
/// Both halves are painted rather than only the filled one, so a bar that ever
/// moves backwards - a transfer restarted - does not keep the fill it had.
fn fill_bar(canvas: &mut Canvas<'_>, percent: u8) -> Result<(), CanvasError> {
    let filled = u32::from(percent) * BAR.size.width / 100;
    if filled > 0 {
        canvas.fill_solid(
            &Rectangle::new(BAR.top_left, Size::new(filled, BAR.size.height)),
            theme::ACCENT,
        )?;
    }
    if filled < BAR.size.width {
        canvas.fill_solid(
            &Rectangle::new(
                Point::new(BAR.top_left.x + filled.cast_signed(), BAR.top_left.y),
                Size::new(BAR.size.width - filled, BAR.size.height),
            ),
            theme::BACKGROUND,
        )?;
    }
    Ok(())
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
            Point::new(120, 60),
            ui_text(theme::TEXT, theme::BACKGROUND),
            Alignment::Center,
        ),
        canvas,
    )?;
    keep_alive();

    draw_percent(canvas, percent)?;
    keep_alive();

    // Bar outline, then a fill proportional to the received bytes.
    BAR.into_styled(
        PrimitiveStyleBuilder::new()
            .fill_color(theme::BACKGROUND)
            .stroke_color(theme::ACCENT)
            .stroke_width(2)
            .build(),
    )
    .draw(canvas)?;
    fill_bar(canvas, percent)?;
    keep_alive();

    draw_visible(
        &Text::with_alignment(
            "Keep the watch nearby",
            Point::new(120, 184),
            hint_text(theme::TEXT, theme::BACKGROUND),
            Alignment::Center,
        ),
        canvas,
    )?;
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
            Point::new(120, 60),
            ui_text(theme::TEXT, theme::BACKGROUND),
            Alignment::Center,
        ),
        canvas,
    )?;
    keep_alive();

    draw_percent(canvas, percent)?;
    BAR.into_styled(
        PrimitiveStyleBuilder::new()
            .fill_color(theme::BACKGROUND)
            .stroke_color(theme::ACCENT)
            .stroke_width(2)
            .build(),
    )
    .draw(canvas)?;
    fill_bar(canvas, percent)?;
    keep_alive();
    draw_visible(
        &Text::with_alignment(
            "Safe to restart",
            Point::new(120, 184),
            hint_text(theme::TEXT, theme::BACKGROUND),
            Alignment::Center,
        ),
        canvas,
    )?;
    keep_alive();
    Ok(())
}

/// Draws a full-screen firmware-update failure notice.
///
/// The transfer was abandoned, either because a flash operation failed or
/// because the host stopped sending. There is no wire protocol error for
/// either case, so the watch's own screen is the only place it can be
/// surfaced. The specific reason - including the flash's JEDEC id when the
/// chip was not recognized - is shown to make a sealed watch diagnosable
/// without a debug port.
pub fn draw_dfu_failed(
    canvas: &mut Canvas<'_>,
    reason: DfuFailReason,
    keep_alive: &mut dyn FnMut(),
) -> Result<(), CanvasError> {
    canvas.clear(theme::BACKGROUND)?;
    draw_visible(
        &Text::with_alignment(
            "UPDATE FAILED",
            Point::new(120, 108),
            ui_text(theme::DANGER, theme::BACKGROUND),
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
