use core::fmt::Write;

use embedded_graphics::{
    mono_font::{MonoTextStyle, ascii::FONT_10X20},
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyleBuilder, Rectangle},
};
use heapless::String;
use pineforge_state::{
    AppEvent, Button, ButtonBounds, ButtonOutcome, DisplaySettings, ScreenAction, SwipeDirection,
};

use crate::ui::{render::draw_mono_text_visible, screen::Screen};

const ROW_HEIGHT: i32 = 50;
const ROW_WIDTH: i32 = 200;
const ROW_X: i32 = 20;
const BRIGHTNESS_ROW_Y: i32 = 60;
const DIM_ROW_Y: i32 = 120;
const OFF_ROW_Y: i32 = 180;

/// Adjusts display settings through preset-cycling rows.
pub struct DisplaySettingsScreen {
    settings: DisplaySettings,
    brightness_row: Button,
    dim_row: Button,
    off_row: Button,
    dirty: bool,
}

impl Default for DisplaySettingsScreen {
    fn default() -> Self {
        Self {
            settings: DisplaySettings::DEFAULT,
            brightness_row: Button::new(ButtonBounds::new(
                ROW_X,
                BRIGHTNESS_ROW_Y,
                ROW_WIDTH,
                ROW_HEIGHT,
            )),
            dim_row: Button::new(ButtonBounds::new(ROW_X, DIM_ROW_Y, ROW_WIDTH, ROW_HEIGHT)),
            off_row: Button::new(ButtonBounds::new(ROW_X, OFF_ROW_Y, ROW_WIDTH, ROW_HEIGHT)),
            dirty: false,
        }
    }
}

impl DisplaySettingsScreen {
    fn draw_row<D>(display: &mut D, y: i32, label: &str, value: &str) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        let bounds = Rectangle::new(
            Point::new(ROW_X, y),
            Size::new(ROW_WIDTH as u32, ROW_HEIGHT as u32),
        );
        bounds
            .into_styled(
                PrimitiveStyleBuilder::new()
                    .fill_color(Rgb565::BLACK)
                    .stroke_color(Rgb565::WHITE)
                    .stroke_width(1)
                    .build(),
            )
            .draw(display)?;
        draw_mono_text_visible(
            label,
            Point::new(ROW_X + 10, y + 20),
            MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE),
            display,
        )?;
        draw_mono_text_visible(
            value,
            Point::new(ROW_X + 10, y + 42),
            MonoTextStyle::new(&FONT_10X20, Rgb565::CSS_ORANGE),
            display,
        )
    }

    fn draw_rows<D>(&self, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        // The three backlight FETs only yield two distinguishable levels.
        let brightness = if self.settings.brightness() > 1 {
            "FULL"
        } else {
            "LOW"
        };
        Self::draw_row(display, BRIGHTNESS_ROW_Y, "BRIGHT", brightness)?;

        let mut value: String<16> = String::new();
        let _ = write!(value, "{} s", self.settings.dim_after_millis() / 1_000);
        Self::draw_row(display, DIM_ROW_Y, "DIM", &value)?;

        value.clear();
        let _ = write!(value, "{} s", self.settings.off_after_millis() / 1_000);
        Self::draw_row(display, OFF_ROW_Y, "OFF", &value)
    }
}

impl Screen for DisplaySettingsScreen {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction {
        self.dirty = false;
        if event == AppEvent::Swipe(SwipeDirection::Right) {
            return ScreenAction::Back;
        }
        if let AppEvent::DisplaySettingsUpdated(settings) = event {
            if settings != self.settings {
                self.settings = settings;
                self.dirty = true;
            }
            return ScreenAction::None;
        }

        let mut updated = None;
        if self.brightness_row.handle_event(event) == ButtonOutcome::Activated {
            updated = Some(self.settings.cycle_brightness());
        } else if self.dim_row.handle_event(event) == ButtonOutcome::Activated {
            updated = Some(self.settings.cycle_dim_timeout());
        } else if self.off_row.handle_event(event) == ButtonOutcome::Activated {
            updated = Some(self.settings.cycle_off_timeout());
        }

        if let Some(settings) = updated {
            self.settings = settings;
            self.dirty = true;
            return ScreenAction::ApplySettings(settings);
        }
        ScreenAction::None
    }

    fn draw_full<D>(&self, display: &mut D, mut keep_alive: impl FnMut()) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        display.clear(Rgb565::BLACK)?;
        draw_mono_text_visible(
            "DISPLAY",
            Point::new(20, 30),
            MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE),
            display,
        )?;
        keep_alive();
        self.draw_rows(display)?;
        keep_alive();
        draw_mono_text_visible(
            "< swipe right",
            Point::new(20, 232),
            MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE),
            display,
        )
    }

    fn draw_dirty<D>(&self, display: &mut D, mut keep_alive: impl FnMut()) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        if self.dirty {
            self.draw_rows(display)?;
            keep_alive();
        }
        Ok(())
    }
}
