//! Picks the watchface, from the faces this build carries.
//!
//! The first screen whose rows are choices rather than values: the menu lists
//! what the setting can be and marks what it currently is, instead of naming a
//! value the user cycles blind. That is the shape every settings leaf should
//! take once the settings screen splits into submenus.
//!
//! Choosing applies immediately and stays put, so the moved marker is the
//! confirmation and the way back is the way back from anywhere.

use pineforge_state::{
    AppEvent, DisplaySettings, ScreenAction, WATCHFACES, WatchfaceDescriptor, WatchfaceId,
};

use crate::ui::canvas::{Canvas, CanvasError};
use crate::ui::{
    menu::{self, Menu, MenuColumn, MenuOutcome, MenuRow, MenuState, MenuTitle},
    render::ROW_HEIGHT,
    screen::{Paint, Screen},
    status::STATUS_HEIGHT,
};

/// Rows the picker shows on one page. `WATCHFACES` is shorter than this in
/// every build today; a build that carries more faces pages rather than
/// hiding them.
const ROWS_PER_PAGE: usize = 4;

const TITLE_BASELINE_Y: i32 = STATUS_HEIGHT + 20;

/// One row per face, taken from the table rather than written out again.
///
/// Repeating the names here would let the picker and the faces it picks drift
/// apart, which is exactly the failure a build with a diagnostics face and one
/// without would produce first.
const fn face_rows<const N: usize>() -> [MenuRow; N] {
    let mut rows = [MenuRow::Choice { label: "" }; N];
    let mut index = 0;
    while index < N {
        rows[index] = MenuRow::Choice {
            label: WATCHFACES[index].name,
        };
        index += 1;
    }
    rows
}

static ROWS: [MenuRow; WATCHFACES.len()] = face_rows();

static MENU: Menu = Menu {
    title: Some(MenuTitle {
        text: "WATCHFACE",
        baseline_y: TITLE_BASELINE_Y,
    }),
    first_row_y: TITLE_BASELINE_Y + 16,
    row_step: ROW_HEIGHT + 2,
    rows: &ROWS,
    hint: "> back",
};

pub struct WatchfaceSelectScreen {
    settings: DisplaySettings,
    menu: MenuState<ROWS_PER_PAGE>,
}

impl Default for WatchfaceSelectScreen {
    fn default() -> Self {
        Self {
            settings: DisplaySettings::DEFAULT,
            menu: MenuState::new(&MENU),
        }
    }
}

impl WatchfaceSelectScreen {
    /// Position of the showing face in `WATCHFACES`.
    ///
    /// A face the table does not list leaves nothing marked rather than marking
    /// the first row, so a settings record from another build cannot make the
    /// picker claim a face it is not showing.
    fn selected(&self) -> usize {
        let showing = self.settings.watchface();
        WATCHFACES
            .iter()
            .position(|face: &WatchfaceDescriptor| face.id == showing)
            .unwrap_or(usize::MAX)
    }

    fn face_at(entry: usize) -> Option<WatchfaceId> {
        WATCHFACES.get(entry).map(|face| face.id)
    }
}

impl Paint for WatchfaceSelectScreen {
    fn draw_full(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        menu::draw(
            &MENU,
            self.menu.list(),
            MenuColumn::Selected(self.selected()),
            canvas,
            keep_alive,
        )
    }
}

impl Screen for WatchfaceSelectScreen {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction {
        // Cleared by the same call that reads the touch, so an update carrying
        // an unchanged face leaves the screen clean.
        let chosen = self.menu.handle(&MENU, event);

        if let AppEvent::DisplaySettingsUpdated(settings) = event {
            if settings != self.settings {
                self.settings = settings;
                self.menu.mark_dirty();
            }
            return ScreenAction::None;
        }

        let MenuOutcome::Chose(entry) = chosen else {
            return ScreenAction::None;
        };
        let Some(face) = Self::face_at(entry) else {
            return ScreenAction::None;
        };
        if face == self.settings.watchface() {
            return ScreenAction::None;
        }

        let updated = self.settings.with_watchface(face);
        self.settings = updated;
        self.menu.mark_dirty();
        ScreenAction::ApplySettings(updated)
    }

    fn draw_dirty(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        if self.menu.is_dirty() {
            menu::draw_rows(
                &MENU,
                self.menu.list(),
                MenuColumn::Selected(self.selected()),
                canvas,
                keep_alive,
            )?;
        }
        Ok(())
    }
}
