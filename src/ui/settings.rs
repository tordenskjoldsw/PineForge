use core::fmt::Write;

use embedded_graphics::{
    mono_font::{MonoTextStyle, ascii::FONT_10X20},
    pixelcolor::Rgb565,
    prelude::*,
};
use heapless::String;
use pineforge_state::{
    AppEvent, Button, ButtonBounds, ButtonOutcome, DisplaySettings, ScreenAction,
};

use crate::ui::{
    render::{ROW_HEIGHT, ROW_WIDTH, ROW_X, draw_mono_text_visible, draw_row},
    screen::{Paint, Screen},
    status::STATUS_HEIGHT,
    theme,
};

/// Rows start below the status corner, which every screen but a watchface
/// carries.
const BRIGHTNESS_ROW_Y: i32 = STATUS_HEIGHT + 2;
const DIM_ROW_Y: i32 = BRIGHTNESS_ROW_Y + ROW_HEIGHT + 2;
const OFF_ROW_Y: i32 = DIM_ROW_Y + ROW_HEIGHT + 2;
const HEART_RATE_ROW_Y: i32 = OFF_ROW_Y + ROW_HEIGHT + 2;
const HEART_RATE_INTERVAL_ROW_Y: i32 = HEART_RATE_ROW_Y + ROW_HEIGHT + 2;

/// Adjusts display settings.
pub struct DisplaySettingsScreen {
    settings: DisplaySettings,
    brightness_row: Button,
    dim_row: Button,
    off_row: Button,
    heart_rate_row: Button,
    heart_rate_interval_row: Button,
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
            heart_rate_row: Button::new(ButtonBounds::new(
                ROW_X,
                HEART_RATE_ROW_Y,
                ROW_WIDTH,
                ROW_HEIGHT,
            )),
            heart_rate_interval_row: Button::new(ButtonBounds::new(
                ROW_X,
                HEART_RATE_INTERVAL_ROW_Y,
                ROW_WIDTH,
                ROW_HEIGHT,
            )),
            dirty: false,
        }
    }
}

impl DisplaySettingsScreen {
    fn draw_rows<D>(&self, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        // The three cumulative backlight levels (see BRIGHTNESS_LEVELS).
        let brightness = match self.settings.brightness() {
            1 => "LOW",
            3 => "MED",
            _ => "FULL",
        };
        draw_row(display, BRIGHTNESS_ROW_Y, "BRIGHT", brightness)?;

        let mut value: String<16> = String::new();
        let _ = write!(value, "{} s", self.settings.dim_after_millis() / 1_000);
        draw_row(display, DIM_ROW_Y, "DIM", &value)?;

        value.clear();
        let _ = write!(value, "{} s", self.settings.off_after_millis() / 1_000);
        draw_row(display, OFF_ROW_Y, "OFF", &value)?;

        draw_row(
            display,
            HEART_RATE_ROW_Y,
            "HEART",
            if self.settings.heart_rate_enabled() {
                "ON"
            } else {
                "OFF"
            },
        )?;
        value.clear();
        let _ = write!(
            value,
            "{} min",
            self.settings.heart_rate_interval_seconds() / 60
        );
        draw_row(display, HEART_RATE_INTERVAL_ROW_Y, "HR INT", &value)
    }
}

impl Paint for DisplaySettingsScreen {
    fn draw_full<D>(&self, display: &mut D, mut keep_alive: impl FnMut()) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        display.clear(theme::BACKGROUND)?;
        keep_alive();
        self.draw_rows(display)?;
        keep_alive();
        draw_mono_text_visible(
            "^ swipe up",
            Point::new(ROW_X, 232),
            MonoTextStyle::new(&FONT_10X20, theme::TEXT),
            display,
        )
    }
}

impl Screen for DisplaySettingsScreen {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction {
        self.dirty = false;
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
        } else if self.heart_rate_row.handle_event(event) == ButtonOutcome::Activated {
            updated = Some(self.settings.toggle_heart_rate());
        } else if self.heart_rate_interval_row.handle_event(event) == ButtonOutcome::Activated {
            updated = Some(self.settings.cycle_heart_rate_interval());
        }

        if let Some(settings) = updated {
            self.settings = settings;
            self.dirty = true;
            return ScreenAction::ApplySettings(settings);
        }
        ScreenAction::None
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
