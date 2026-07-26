use core::fmt::Write;

use embedded_graphics::prelude::*;
use heapless::String;
use pineforge_state::{AppEvent, Button, ButtonBounds, ButtonOutcome, ScreenAction};

use crate::ui::canvas::{Canvas, CanvasError};
use crate::ui::font::ui_text;
use crate::ui::{
    render::{ROW_HEIGHT, ROW_WIDTH, ROW_X, draw_mono_text_visible, draw_row},
    screen::{Paint, Screen},
    status::STATUS_HEIGHT,
    theme,
};

/// The title clears the status corner; the rows follow below it.
const TITLE_BASELINE_Y: i32 = STATUS_HEIGHT + 20;
const CONFIRM_ROW_Y: i32 = TITLE_BASELINE_Y + 16;
const RESTART_ROW_Y: i32 = CONFIRM_ROW_Y + ROW_HEIGHT + 16;
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

    fn draw_rows(&self, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        draw_row(canvas, CONFIRM_ROW_Y, "FW", &self.confirm_value())?;
        // An unconfirmed image does not survive a reset: MCUBoot restores the
        // image this one replaced.
        draw_row(
            canvas,
            RESTART_ROW_Y,
            "RESTART",
            if self.confirmed { "REBOOT" } else { "ROLLBACK" },
        )
    }
}

impl Paint for FirmwareScreen {
    fn draw_full(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        canvas.clear(theme::BACKGROUND)?;
        keep_alive();

        let mut title: String<24> = String::new();
        let _ = write!(title, "PINEFORGE {}", env!("CARGO_PKG_VERSION"));
        draw_mono_text_visible(
            &title,
            Point::new(ROW_X, TITLE_BASELINE_Y),
            ui_text(theme::TEXT, theme::BACKGROUND),
            canvas,
        )?;
        keep_alive();

        self.draw_rows(canvas)?;
        keep_alive();
        draw_mono_text_visible(
            "> back",
            Point::new(ROW_X, HINT_BASELINE_Y),
            ui_text(theme::TEXT, theme::BACKGROUND),
            canvas,
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

    fn draw_dirty(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        if self.dirty {
            self.draw_rows(canvas)?;
            keep_alive();
        }
        Ok(())
    }
}
