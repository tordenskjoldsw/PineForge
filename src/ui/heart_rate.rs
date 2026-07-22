use core::fmt::Write;

use embedded_graphics::{
    mono_font::{MonoTextStyle, ascii::FONT_10X20},
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
};
use heapless::String;
use pineforge_state::{AppEvent, HeartRateState, ScreenAction, SwipeDirection};

use crate::ui::{render::draw_mono_text_visible, screen::Screen};

const STATUS_AREA: Rectangle = Rectangle::new(Point::new(0, 70), Size::new(240, 100));

pub struct HeartRateScreen {
    state: HeartRateState,
}

impl Default for HeartRateScreen {
    fn default() -> Self {
        Self {
            state: HeartRateState::Starting,
        }
    }
}

impl HeartRateScreen {
    pub const fn begin_measurement(&mut self) {
        self.state = HeartRateState::Starting;
    }

    fn draw_status<D>(&self, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        STATUS_AREA
            .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
            .draw(display)?;
        let mut text: String<20> = String::new();
        match self.state {
            HeartRateState::Disabled => {
                let _ = text.push_str("STOPPED");
            }
            HeartRateState::Starting => {
                let _ = text.push_str("STARTING...");
            }
            HeartRateState::Collecting => {
                let _ = text.push_str("MEASURING...");
            }
            HeartRateState::Measuring => {
                let _ = text.push_str("VALIDATING...");
            }
            HeartRateState::Result(bpm) => {
                let _ = write!(text, "{bpm} BPM");
            }
            HeartRateState::NoSignal => {
                let _ = text.push_str("NO SIGNAL");
            }
            HeartRateState::AmbientLight => {
                let _ = text.push_str("AMBIENT LIGHT");
            }
            HeartRateState::Error => {
                let _ = text.push_str("SENSOR ERROR");
            }
        }
        draw_mono_text_visible(
            &text,
            Point::new(20, 125),
            MonoTextStyle::new(&FONT_10X20, Rgb565::RED),
            display,
        )
    }
}

impl Screen for HeartRateScreen {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction {
        if event == AppEvent::Swipe(SwipeDirection::Right) {
            return ScreenAction::Back;
        }
        if let AppEvent::HeartRateStateUpdated(state) = event {
            self.state = state;
        }
        ScreenAction::None
    }

    fn draw_full<D>(&self, display: &mut D, mut keep_alive: impl FnMut()) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        display.clear(Rgb565::BLACK)?;
        draw_mono_text_visible(
            "HEART RATE",
            Point::new(20, 40),
            MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE),
            display,
        )?;
        keep_alive();
        self.draw_status(display)?;
        draw_mono_text_visible(
            "< swipe right",
            Point::new(20, 225),
            MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE),
            display,
        )?;
        keep_alive();
        Ok(())
    }

    fn draw_dirty<D>(&self, display: &mut D, mut keep_alive: impl FnMut()) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        self.draw_status(display)?;
        keep_alive();
        Ok(())
    }
}
