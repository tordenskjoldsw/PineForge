use pineforge_state::{AppEvent, ScreenAction};

use crate::about::BuildInfo;
use crate::canvas::{Canvas, CanvasError};
use crate::{
    menu::{self, Menu, MenuColumn, MenuOutcome, MenuRow, MenuState, MenuTitle},
    render::ROW_HEIGHT,
    screen::{Paint, Screen},
    status::STATUS_HEIGHT,
};

/// The title clears the status corner; the rows follow below it.
const TITLE_BASELINE_Y: i32 = STATUS_HEIGHT + 20;

static MENU: Menu = Menu {
    title: Some(MenuTitle {
        text: "FIRMWARE",
        baseline_y: TITLE_BASELINE_Y,
    }),
    first_row_y: TITLE_BASELINE_Y + 16,
    // Tighter than the two rows this screen used to carry had room for: three
    // on the old rhythm ran the last one into the hint at the foot.
    row_step: ROW_HEIGHT + 8,
    rows: &[
        // The build this screen is deciding about. It reads as a row rather
        // than as the heading because the heading cannot carry it: a `Menu` is
        // a `'static` description, and which build this is only becomes known
        // when the firmware hands it over.
        MenuRow::Value { label: "BUILD" },
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
    /// The packaged release, handed over by the firmware at start-up. See
    /// [`crate::about`] for why it cannot be compiled in here.
    version: &'static str,
    menu: MenuState<3>,
}

impl Default for FirmwareScreen {
    fn default() -> Self {
        Self {
            confirmed: false,
            version: BuildInfo::UNKNOWN.version,
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

    pub const fn set_version(&mut self, version: &'static str) {
        self.version = version;
        self.menu.mark_dirty();
    }

    /// The right-hand column, in row order.
    ///
    /// An unconfirmed image does not survive a reset: `MCUBoot` restores the
    /// image this one replaced.
    const fn values(&self) -> [&'static str; 3] {
        if self.confirmed {
            [self.version, "OK", "REBOOT"]
        } else {
            [self.version, "CONFIRM", "ROLLBACK"]
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
            MenuOutcome::Chose(1) if !self.confirmed => ScreenAction::ConfirmFirmware,
            MenuOutcome::Chose(2) => ScreenAction::Reboot,
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
