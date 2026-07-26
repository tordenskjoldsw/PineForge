use core::fmt::Write;

use embedded_graphics::{
    mono_font::{MonoTextStyle, ascii::FONT_10X20},
    pixelcolor::Rgb565,
    prelude::*,
};
use heapless::String;
use pineforge_state::{AppEvent, Button, ButtonBounds, ButtonOutcome, ScreenAction};

use crate::ui::{
    render::{ROW_HEIGHT, ROW_WIDTH, ROW_X, draw_mono_text_visible, draw_row},
    screen::Screen,
};

const TITLE_BASELINE_Y: i32 = 30;
const CONFIRM_ROW_Y: i32 = 60;
const RESTART_ROW_Y: i32 = 110;
const HINT_BASELINE_Y: i32 = 232;

/// Firmware confirmation and a software restart.
///
/// Both belong on one screen because they are the same decision seen twice: an
/// unconfirmed image is restarted *into the previous firmware*, which is exactly
/// the recovery the side button performs today. Making that reachable from
/// software is what frees the side button to become the back button, so the
/// value column names the consequence rather than the action.
pub struct FirmwareScreen {
    confirmed: bool,
    confirm_row: Button,
    restart_row: Button,
    dirty: bool,
}

impl Default for FirmwareScreen {
    fn default() -> Self {
        Self {
            confirmed: false,
            confirm_row: Button::new(ButtonBounds::new(
                ROW_X,
                CONFIRM_ROW_Y,
                ROW_WIDTH,
                ROW_HEIGHT,
            )),
            restart_row: Button::new(ButtonBounds::new(
                ROW_X,
                RESTART_ROW_Y,
                ROW_WIDTH,
                ROW_HEIGHT,
            )),
            dirty: false,
        }
    }
}

impl FirmwareScreen {
    /// Reflects the confirmed/unconfirmed state read at boot or after a
    /// successful confirmation.
    pub const fn set_confirmed(&mut self, confirmed: bool) {
        self.confirmed = confirmed;
        self.dirty = true;
    }

    fn confirm_value(&self) -> String<16> {
        let mut value = String::new();
        if self.confirmed {
            let _ = value.push_str("OK");
        } else {
            let _ = value.push_str("CONFIRM");
        }
        value
    }

    fn draw_rows<D>(&self, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        draw_row(display, CONFIRM_ROW_Y, "FW", &self.confirm_value())?;
        // An unconfirmed image does not survive a reset: MCUBoot restores the
        // image this one replaced.
        draw_row(
            display,
            RESTART_ROW_Y,
            "RESTART",
            if self.confirmed { "REBOOT" } else { "ROLLBACK" },
        )
    }
}

impl Screen for FirmwareScreen {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction {
        self.dirty = false;

        if self.confirm_row.handle_event(event) == ButtonOutcome::Activated && !self.confirmed {
            return ScreenAction::ConfirmFirmware;
        }
        if self.restart_row.handle_event(event) == ButtonOutcome::Activated {
            return ScreenAction::Reboot;
        }
        ScreenAction::None
    }

    fn draw_full<D>(&self, display: &mut D, mut keep_alive: impl FnMut()) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        display.clear(Rgb565::BLACK)?;
        keep_alive();

        let mut title: String<24> = String::new();
        let _ = write!(title, "PINEFORGE {}", env!("CARGO_PKG_VERSION"));
        draw_mono_text_visible(
            &title,
            Point::new(ROW_X, TITLE_BASELINE_Y),
            MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE),
            display,
        )?;
        keep_alive();

        self.draw_rows(display)?;
        keep_alive();
        draw_mono_text_visible(
            "< swipe left",
            Point::new(ROW_X, HINT_BASELINE_Y),
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
