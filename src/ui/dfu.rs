use core::fmt::Write;

use embedded_graphics::{
    mono_font::{MonoTextStyle, ascii::FONT_6X10, ascii::FONT_10X20},
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyleBuilder, Rectangle},
    text::{Alignment, Text},
};
use heapless::String;
use pineforge_state::DfuFailReason;

use crate::ui::render::draw_visible;

const BAR: Rectangle = Rectangle::new(Point::new(20, 128), Size::new(200, 24));
const ACCENT: Rgb565 = Rgb565::new(31, 20, 0);
const FAILED_ACCENT: Rgb565 = Rgb565::new(31, 0, 0);

/// Draws a full-screen firmware-update progress screen, mirroring
/// `InfiniTime`'s DFU screen: a title, a percentage, and a fill bar.
pub fn draw_dfu_progress<D>(
    display: &mut D,
    percent: u8,
    mut keep_alive: impl FnMut(),
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    let percent = percent.min(100);
    display.clear(Rgb565::BLACK)?;
    draw_visible(
        &Text::with_alignment(
            "FIRMWARE UPDATE",
            Point::new(120, 60),
            MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE),
            Alignment::Center,
        ),
        display,
    )?;
    keep_alive();

    let mut label: String<8> = String::new();
    let _ = write!(label, "{percent}%");
    draw_visible(
        &Text::with_alignment(
            &label,
            Point::new(120, 108),
            MonoTextStyle::new(&FONT_10X20, ACCENT),
            Alignment::Center,
        ),
        display,
    )?;
    keep_alive();

    // Bar outline, then a fill proportional to the received bytes.
    BAR.into_styled(
        PrimitiveStyleBuilder::new()
            .fill_color(Rgb565::BLACK)
            .stroke_color(ACCENT)
            .stroke_width(2)
            .build(),
    )
    .draw(display)?;
    let fill_width = u32::from(percent) * BAR.size.width / 100;
    if fill_width > 0 {
        Rectangle::new(BAR.top_left, Size::new(fill_width, BAR.size.height))
            .into_styled(PrimitiveStyleBuilder::new().fill_color(ACCENT).build())
            .draw(display)?;
    }
    keep_alive();

    draw_visible(
        &Text::with_alignment(
            "Keep the watch nearby",
            Point::new(120, 184),
            MonoTextStyle::new(&FONT_6X10, Rgb565::WHITE),
            Alignment::Center,
        ),
        display,
    )?;
    keep_alive();
    Ok(())
}

/// Draws the power-loss-safe, one-time storage formatting progress.
pub fn draw_storage_progress<D>(
    display: &mut D,
    percent: u8,
    mut keep_alive: impl FnMut(),
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    let percent = percent.min(100);
    display.clear(Rgb565::BLACK)?;
    draw_visible(
        &Text::with_alignment(
            "PREPARING STORAGE",
            Point::new(120, 60),
            MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE),
            Alignment::Center,
        ),
        display,
    )?;
    keep_alive();

    let mut label: String<8> = String::new();
    let _ = write!(label, "{percent}%");
    draw_visible(
        &Text::with_alignment(
            &label,
            Point::new(120, 108),
            MonoTextStyle::new(&FONT_10X20, ACCENT),
            Alignment::Center,
        ),
        display,
    )?;
    BAR.into_styled(
        PrimitiveStyleBuilder::new()
            .fill_color(Rgb565::BLACK)
            .stroke_color(ACCENT)
            .stroke_width(2)
            .build(),
    )
    .draw(display)?;
    let fill_width = u32::from(percent) * BAR.size.width / 100;
    if fill_width > 0 {
        Rectangle::new(BAR.top_left, Size::new(fill_width, BAR.size.height))
            .into_styled(PrimitiveStyleBuilder::new().fill_color(ACCENT).build())
            .draw(display)?;
    }
    keep_alive();
    draw_visible(
        &Text::with_alignment(
            "Safe to restart",
            Point::new(120, 184),
            MonoTextStyle::new(&FONT_6X10, Rgb565::WHITE),
            Alignment::Center,
        ),
        display,
    )?;
    keep_alive();
    Ok(())
}

/// Draws a full-screen firmware-update failure notice; a flash operation
/// during the update failed and the transfer was abandoned. There is no wire
/// protocol error for this case, so the watch's own screen is the only place
/// this can be surfaced. The specific reason (including the flash's JEDEC id
/// when the chip was not recognized) is shown to make a sealed watch
/// diagnosable without a debug port.
pub fn draw_dfu_failed<D>(
    display: &mut D,
    reason: DfuFailReason,
    mut keep_alive: impl FnMut(),
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    display.clear(Rgb565::BLACK)?;
    draw_visible(
        &Text::with_alignment(
            "UPDATE FAILED",
            Point::new(120, 108),
            MonoTextStyle::new(&FONT_10X20, FAILED_ACCENT),
            Alignment::Center,
        ),
        display,
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
    }
    draw_visible(
        &Text::with_alignment(
            &detail,
            Point::new(120, 145),
            MonoTextStyle::new(&FONT_6X10, Rgb565::WHITE),
            Alignment::Center,
        ),
        display,
    )?;
    keep_alive();

    draw_visible(
        &Text::with_alignment(
            "Swipe to dismiss",
            Point::new(120, 190),
            MonoTextStyle::new(&FONT_6X10, ACCENT),
            Alignment::Center,
        ),
        display,
    )?;
    keep_alive();
    Ok(())
}
