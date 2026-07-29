//! The settings root: which setting to change, not what to change it to.
//!
//! Every row leads to a leaf that offers that setting's presets. Cycling a
//! value in place was cheaper to build and worse to use - it named one value
//! and hid the rest, so finding out what a setting could be meant pressing it
//! until it came round again.

use pineforge_state::{AppEvent, ScreenAction, ScreenId, Setting};

use crate::canvas::{Canvas, CanvasError};
use crate::{
    menu::{self, Menu, MenuColumn, MenuOutcome, MenuRow, MenuState},
    render::ROW_HEIGHT,
    screen::{Paint, Screen},
    status::STATUS_HEIGHT,
};

/// Rows a page shows. Four of them fill the space between the status corner and
/// the hint at the foot; a fifth would run into it.
const ROWS_PER_PAGE: usize = 4;

/// Rows start below the status corner, which every screen but a watchface
/// carries.
static MENU: Menu = Menu {
    title: None,
    first_row_y: STATUS_HEIGHT + 4,
    row_step: ROW_HEIGHT + 6,
    // Each row is named by the setting it opens, not by a second shorter name
    // kept here. The two used to disagree - a row reading DIM opened a screen
    // headed DIM AFTER - which is one thing wearing two names, and the kind of
    // drift that only shows up to someone reading the screen rather than the
    // code.
    rows: &[
        MenuRow::Navigate {
            label: Setting::Brightness.title(),
            target: ScreenId::Brightness,
        },
        MenuRow::Navigate {
            label: Setting::DimTimeout.title(),
            target: ScreenId::DimTimeout,
        },
        MenuRow::Navigate {
            label: Setting::OffTimeout.title(),
            target: ScreenId::OffTimeout,
        },
        MenuRow::Navigate {
            label: Setting::HeartRate.title(),
            target: ScreenId::HeartRate,
        },
        MenuRow::Navigate {
            label: Setting::WakeGesture.title(),
            target: ScreenId::WakeGesture,
        },
        MenuRow::Navigate {
            label: Setting::Watchface.title(),
            target: ScreenId::WatchfaceSelect,
        },
    ],
    hint: "> back",
};

/// Lists the settings and opens the one that is chosen.
///
/// It holds no settings of its own: the values live on the leaves that offer
/// them, so this screen has nothing to redraw when one changes.
pub struct DisplaySettingsScreen {
    menu: MenuState<ROWS_PER_PAGE>,
}

impl Default for DisplaySettingsScreen {
    fn default() -> Self {
        Self {
            menu: MenuState::new(&MENU),
        }
    }
}

impl Paint for DisplaySettingsScreen {
    fn draw_full(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        // Navigation rows carry their own marker, so nothing is supplied.
        menu::draw(
            &MENU,
            self.menu.page(),
            MenuColumn::Values(&[]),
            canvas,
            keep_alive,
        )
    }
}

impl Screen for DisplaySettingsScreen {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction {
        match self.menu.handle(&MENU, event) {
            MenuOutcome::Navigate(target) => ScreenAction::Push(target),
            MenuOutcome::Chose(_) | MenuOutcome::None => ScreenAction::None,
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
                MenuColumn::Values(&[]),
                canvas,
                keep_alive,
            )?;
        }
        Ok(())
    }
}
