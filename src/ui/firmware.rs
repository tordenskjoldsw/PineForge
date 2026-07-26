use pineforge_state::{AppEvent, ScreenAction};

use crate::ui::canvas::{Canvas, CanvasError};
use crate::ui::{
    menu::{self, Menu, MenuColumn, MenuOutcome, MenuRow, MenuState, MenuTitle},
    render::ROW_HEIGHT,
    screen::{Paint, Screen},
    status::STATUS_HEIGHT,
};

/// The title clears the status corner; the rows follow below it.
const TITLE_BASELINE_Y: i32 = STATUS_HEIGHT + 20;

static MENU: Menu = Menu {
    title: Some(MenuTitle {
        // Built at compile time rather than formatted per paint: the version is
        // known then, and a runtime `write!` would cost a buffer and the
        // formatting machinery for a string that never changes.
        text: concat!("PINEFORGE ", env!("CARGO_PKG_VERSION")),
        baseline_y: TITLE_BASELINE_Y,
    }),
    first_row_y: TITLE_BASELINE_Y + 16,
    row_step: ROW_HEIGHT + 16,
    rows: &[
        MenuRow::Value { label: "FW" },
        MenuRow::Value { label: "RESTART" },
    ],
    hint: "> back",
};

/// Firmware confirmation and a software restart.
///
/// Both belong on one screen because they are the same decision seen twice: an
/// unconfirmed image is restarted *into the previous firmware*, which is exactly
/// the recovery the side button performs today. Making that reachable from
/// software is what frees the side button to become the back button, so the
/// value column names the consequence rather than the action.
pub struct FirmwareScreen {
    confirmed: bool,
    menu: MenuState<2>,
}

impl Default for FirmwareScreen {
    fn default() -> Self {
        Self {
            confirmed: false,
            menu: MenuState::new(&MENU),
        }
    }
}

impl FirmwareScreen {
    /// Reflects the confirmed/unconfirmed state read at boot or after a
    /// successful confirmation.
    pub const fn set_confirmed(&mut self, confirmed: bool) {
        self.confirmed = confirmed;
        self.menu.mark_dirty();
    }

    /// The right-hand column, in row order.
    ///
    /// An unconfirmed image does not survive a reset: `MCUBoot` restores the
    /// image this one replaced.
    const fn values(&self) -> [&'static str; 2] {
        if self.confirmed {
            ["OK", "REBOOT"]
        } else {
            ["CONFIRM", "ROLLBACK"]
        }
    }
}

impl Paint for FirmwareScreen {
    fn draw_full(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        menu::draw(
            &MENU,
            self.menu.page(),
            MenuColumn::Values(&self.values()),
            canvas,
            keep_alive,
        )
    }
}

impl Screen for FirmwareScreen {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction {
        match self.menu.handle(&MENU, event) {
            MenuOutcome::Chose(0) if !self.confirmed => ScreenAction::ConfirmFirmware,
            MenuOutcome::Chose(1) => ScreenAction::Reboot,
            _ => ScreenAction::None,
        }
    }

    fn draw_dirty(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        if self.menu.is_dirty() {
            menu::draw_rows(
                &MENU,
                self.menu.page(),
                MenuColumn::Values(&self.values()),
                canvas,
                keep_alive,
            )?;
        }
        Ok(())
    }
}
