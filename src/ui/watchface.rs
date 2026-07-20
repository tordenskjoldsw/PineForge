#![allow(dead_code)]

use core::fmt::Write;

use embedded_graphics::{
    mono_font::{MonoTextStyle, ascii::FONT_10X20},
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::{Alignment, Text},
};
use heapless::String;

use crate::services::clock::ClockSnapshot;

pub fn draw_watchface<D>(
    display: &mut D,
    now: ClockSnapshot,
    brightness: u8,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565> + OriginDimensions,
{
    Rectangle::new(Point::zero(), display.size())
        .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
        .draw(display)?;

    let style = MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE);
    let mut time: String<16> = String::new();
    let _ = write!(time, "{:02}:{:02}:{:02}", now.hour, now.minute, now.second);
    Text::with_alignment(&time, Point::new(120, 105), style, Alignment::Center).draw(display)?;

    let mut status: String<24> = String::new();
    let _ = write!(status, "Rust / light {brightness}");
    Text::with_alignment(&status, Point::new(120, 145), style, Alignment::Center).draw(display)?;
    Ok(())
}
