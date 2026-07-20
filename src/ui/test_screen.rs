use embedded_graphics::{
    mono_font::{MonoTextStyle, ascii::FONT_6X10, ascii::FONT_10X20},
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::{Alignment, Text},
};

use crate::{
    services::events::UiEvent,
    ui::screen::{Screen, ScreenAction},
};

const RETURN_BUTTON: Rectangle = Rectangle::new(Point::new(20, 154), Size::new(200, 60));
const TOUCH_AREA: Rectangle = Rectangle::new(Point::new(0, 64), Size::new(240, 90));
const TOUCH_MARKER_SIZE: Size = Size::new(13, 13);

#[derive(Default)]
pub struct TestScreen {
    last_touch: Option<Point>,
    previous_touch: Option<Point>,
    return_button_pressed: bool,
}

impl Screen for TestScreen {
    fn handle_event(&mut self, event: UiEvent) -> ScreenAction {
        let UiEvent::Touch { x, y, pressed } = event else {
            return ScreenAction::None;
        };
        let point = Point::new(x, y);
        self.previous_touch = self.last_touch;
        self.last_touch = Some(point);

        if pressed {
            self.return_button_pressed = RETURN_BUTTON.contains(point);
            ScreenAction::None
        } else if core::mem::take(&mut self.return_button_pressed) && RETURN_BUTTON.contains(point)
        {
            ScreenAction::RequestRollback
        } else {
            ScreenAction::None
        }
    }

    fn draw<D>(&self, display: &mut D, mut keep_alive: impl FnMut()) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        display.clear(Rgb565::BLACK)?;
        keep_alive();

        Rectangle::new(Point::new(0, 0), Size::new(240, 64))
            .into_styled(PrimitiveStyle::with_fill(Rgb565::BLUE))
            .draw(display)?;
        keep_alive();

        let heading = MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE);
        Text::with_alignment(
            "PineForge Touch-Test",
            Point::new(120, 38),
            heading,
            Alignment::Center,
        )
        .draw(display)?;
        keep_alive();

        if let Some(point) = self.last_touch.filter(|point| TOUCH_AREA.contains(*point)) {
            Rectangle::new(Point::new(point.x - 6, point.y - 6), TOUCH_MARKER_SIZE)
                .into_styled(PrimitiveStyle::with_fill(Rgb565::YELLOW))
                .draw(display)?;
        }
        keep_alive();

        RETURN_BUTTON
            .into_styled(PrimitiveStyle::with_fill(Rgb565::GREEN))
            .draw(display)?;
        keep_alive();

        let button_text = MonoTextStyle::new(&FONT_6X10, Rgb565::BLACK);
        Text::with_alignment(
            "Touch: Neustart",
            Point::new(120, 188),
            button_text,
            Alignment::Center,
        )
        .draw(display)?;
        keep_alive();

        let hint = MonoTextStyle::new(&FONT_6X10, Rgb565::WHITE);
        Text::with_alignment(
            "Side button: InfiniTime",
            Point::new(120, 232),
            hint,
            Alignment::Center,
        )
        .draw(display)?;
        keep_alive();

        Ok(())
    }

    fn draw_update<D>(&self, display: &mut D, mut keep_alive: impl FnMut()) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        if let Some(point) = self
            .previous_touch
            .filter(|point| TOUCH_AREA.contains(*point))
        {
            Rectangle::new(Point::new(point.x - 6, point.y - 6), TOUCH_MARKER_SIZE)
                .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
                .draw(display)?;
            keep_alive();
        }

        if let Some(point) = self.last_touch.filter(|point| TOUCH_AREA.contains(*point)) {
            Rectangle::new(Point::new(point.x - 6, point.y - 6), TOUCH_MARKER_SIZE)
                .into_styled(PrimitiveStyle::with_fill(Rgb565::YELLOW))
                .draw(display)?;
            keep_alive();
        }

        Ok(())
    }
}
