use core::fmt::Write;

use heapless::String;
use pineforge_state::{AppEvent, DisplaySettings, ScreenAction, ScreenId};

use crate::ui::canvas::{Canvas, CanvasError};
use crate::ui::{
    menu::{self, Menu, MenuColumn, MenuOutcome, MenuRow, MenuState},
    render::ROW_HEIGHT,
    screen::{Paint, Screen},
    status::STATUS_HEIGHT,
};

/// Rows start below the status corner, which every screen but a watchface
/// carries.
static MENU: Menu = Menu {
    title: None,
    first_row_y: STATUS_HEIGHT + 2,
    row_step: ROW_HEIGHT + 2,
    rows: &[
        MenuRow::Value { label: "BRIGHT" },
        MenuRow::Value { label: "DIM" },
        MenuRow::Value { label: "OFF" },
        MenuRow::Value { label: "HEART" },
        MenuRow::Value { label: "HR INT" },
        MenuRow::Navigate {
            label: "FACE",
            target: ScreenId::WatchfaceSelect,
        },
    ],
    hint: "> back",
};

/// Rows whose value is formatted rather than picked from a fixed set.
///
/// Held together so the borrowed slice handed to the renderer outlives the
/// strings it points at.
#[derive(Default)]
struct Formatted {
    dim: String<16>,
    off: String<16>,
    interval: String<16>,
}

/// Adjusts display settings.
pub struct DisplaySettingsScreen {
    settings: DisplaySettings,
    menu: MenuState<5>,
}

impl Default for DisplaySettingsScreen {
    fn default() -> Self {
        Self {
            settings: DisplaySettings::DEFAULT,
            menu: MenuState::new(&MENU),
        }
    }
}

impl DisplaySettingsScreen {
    fn formatted(&self) -> Formatted {
        let mut values = Formatted::default();
        let _ = write!(values.dim, "{} s", self.settings.dim_after_millis() / 1_000);
        let _ = write!(values.off, "{} s", self.settings.off_after_millis() / 1_000);
        let _ = write!(
            values.interval,
            "{} min",
            self.settings.heart_rate_interval_seconds() / 60
        );
        values
    }

    /// The right-hand column, in row order.
    fn values<'a>(&self, formatted: &'a Formatted) -> [&'a str; 5] {
        [
            // The three cumulative backlight levels (see BRIGHTNESS_LEVELS).
            match self.settings.brightness() {
                1 => "LOW",
                3 => "MED",
                _ => "FULL",
            },
            &formatted.dim,
            &formatted.off,
            if self.settings.heart_rate_enabled() {
                "ON"
            } else {
                "OFF"
            },
            &formatted.interval,
        ]
    }
}

impl Paint for DisplaySettingsScreen {
    fn draw_full(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        let formatted = self.formatted();
        menu::draw(
            &MENU,
            self.menu.list(),
            MenuColumn::Values(&self.values(&formatted)),
            canvas,
            keep_alive,
        )
    }
}

impl Screen for DisplaySettingsScreen {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction {
        // Handled before the settings branch because this is what clears the
        // dirty flag: an update carrying unchanged values has to leave the
        // screen clean, not inherit the previous event's repaint. Rows ignore
        // anything that is not a touch, so feeding them costs nothing.
        let chosen = self.menu.handle(&MENU, event);

        if let AppEvent::DisplaySettingsUpdated(settings) = event {
            if settings != self.settings {
                self.settings = settings;
                self.menu.mark_dirty();
            }
            return ScreenAction::None;
        }

        // Row order is the menu's. A row added to the description without a
        // case here simply does nothing, rather than inheriting its neighbour's
        // effect the way a positional list of buttons would.
        if let MenuOutcome::Navigate(target) = chosen {
            return ScreenAction::Push(target);
        }

        let updated = match chosen {
            MenuOutcome::Chose(0) => self.settings.cycle_brightness(),
            MenuOutcome::Chose(1) => self.settings.cycle_dim_timeout(),
            MenuOutcome::Chose(2) => self.settings.cycle_off_timeout(),
            MenuOutcome::Chose(3) => self.settings.toggle_heart_rate(),
            MenuOutcome::Chose(4) => self.settings.cycle_heart_rate_interval(),
            _ => return ScreenAction::None,
        };

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
            let formatted = self.formatted();
            menu::draw_rows(
                &MENU,
                self.menu.list(),
                MenuColumn::Values(&self.values(&formatted)),
                canvas,
                keep_alive,
            )?;
        }
        Ok(())
    }
}
