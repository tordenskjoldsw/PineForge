use core::fmt::Write;

use embedded_graphics::{
    mono_font::{MonoTextStyle, ascii::FONT_6X10, ascii::FONT_10X20},
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyleBuilder, Rectangle},
    text::{Alignment, Text},
};
use heapless::String;

use crate::ui::{render::draw_visible, theme};

const CODE_BOX: Rectangle = Rectangle::new(Point::new(20, 96), Size::new(200, 48));

/// Draws a full-screen pairing prompt with the passkey the user enters on the
/// phone, mirroring `InfiniTime`'s pairing screen.
pub fn draw_pairing<D>(
    display: &mut D,
    passkey: u32,
    mut keep_alive: impl FnMut(),
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    display.clear(theme::BACKGROUND)?;
    draw_visible(
        &Text::with_alignment(
            "BLUETOOTH PAIRING",
            Point::new(120, 50),
            MonoTextStyle::new(&FONT_10X20, theme::TEXT),
            Alignment::Center,
        ),
        display,
    )?;
    keep_alive();

    CODE_BOX
        .into_styled(
            PrimitiveStyleBuilder::new()
                .fill_color(theme::BACKGROUND)
                .stroke_color(theme::ACCENT)
                .stroke_width(2)
                .build(),
        )
        .draw(display)?;
    let mut code: String<8> = String::new();
    let _ = write!(code, "{passkey:06}");
    draw_visible(
        &Text::with_alignment(
            &code,
            Point::new(120, 128),
            MonoTextStyle::new(&FONT_10X20, theme::ACCENT),
            Alignment::Center,
        ),
        display,
    )?;
    keep_alive();

    draw_visible(
        &Text::with_alignment(
            "Enter this code on your phone",
            Point::new(120, 180),
            MonoTextStyle::new(&FONT_6X10, theme::TEXT),
            Alignment::Center,
        ),
        display,
    )?;
    keep_alive();
    Ok(())
}
