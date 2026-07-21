use embedded_graphics::{
    mono_font::{MonoTextStyle, ascii::FONT_6X10, ascii::FONT_10X20},
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::{Alignment, Text},
};

use crate::ui::{render::draw_visible, screen::Screen};
use pineforge_state::{AppEvent, ScreenAction, SwipeDirection};

const TOUCH_AREA: Rectangle = Rectangle::new(Point::new(0, 64), Size::new(240, 150));
const FOOTER_AREA: Rectangle = Rectangle::new(Point::new(0, 214), Size::new(240, 26));
const TOUCH_MARKER_SIZE: Size = Size::new(13, 13);

#[derive(Default)]
pub struct TestScreen {
    last_touch: Option<Point>,
    previous_touch: Option<Point>,
}

impl Screen for TestScreen {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction {
        if event == AppEvent::Swipe(SwipeDirection::Right) {
            return ScreenAction::Back;
        }
        let AppEvent::Touch { x, y, pressed } = event else {
            return ScreenAction::None;
        };
        let point = Point::new(x, y);
        self.previous_touch = self.last_touch;
        self.last_touch = Some(point);

        let _ = pressed;
        ScreenAction::None
    }

    fn draw_full<D>(&self, display: &mut D, mut keep_alive: impl FnMut()) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        Rectangle::new(Point::new(0, 0), Size::new(240, 64))
            .into_styled(PrimitiveStyle::with_fill(Rgb565::BLUE))
            .draw(display)?;
        keep_alive();

        let heading = MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE);
        draw_visible(
            &Text::with_alignment(
                "PineForge Touch-Test",
                Point::new(120, 38),
                heading,
                Alignment::Center,
            ),
            display,
        )?;
        keep_alive();

        TOUCH_AREA
            .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
            .draw(display)?;
        if let Some(point) = self.last_touch.filter(|point| TOUCH_AREA.contains(*point)) {
            Rectangle::new(Point::new(point.x - 6, point.y - 6), TOUCH_MARKER_SIZE)
                .into_styled(PrimitiveStyle::with_fill(Rgb565::YELLOW))
                .draw(display)?;
        }
        keep_alive();

        FOOTER_AREA
            .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
            .draw(display)?;
        let hint = MonoTextStyle::new(&FONT_6X10, Rgb565::WHITE);
        draw_visible(
            &Text::with_alignment(
                "< Swipe right: Zurueck",
                Point::new(120, 232),
                hint,
                Alignment::Center,
            ),
            display,
        )?;
        keep_alive();

        Ok(())
    }

    fn draw_dirty<D>(&self, display: &mut D, mut keep_alive: impl FnMut()) -> Result<(), D::Error>
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
