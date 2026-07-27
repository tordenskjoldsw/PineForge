//! The one screen every settings leaf is.
//!
//! A leaf is always the same screen: the presets a setting can take, the
//! current one marked, and choosing applies at once. What differs between
//! brightness and a screen timeout is which table the rows come from and which
//! setter runs, and that lives in [`Setting`] in the state crate where it can
//! be tested. So there is one screen here, configured when it is opened.
//!
//! One screen rather than one per setting is not only smaller. Each picker used
//! to hold its own copy of the settings, refreshed only while it was the screen
//! on top - so a picker opened after another had changed something applied its
//! preset to a record from before that change and silently reverted it. Worse,
//! a picker never opened since boot still held the defaults, so the first
//! choice made after a restart discarded everything that had been persisted.
//!
//! The fix is ownership, not refresh: the display task holds the one record
//! that is current, and [`SettingPickerScreen::open`] hands it over each time a
//! leaf is entered. A screen that cannot keep a stale copy cannot apply one.

use pineforge_state::{AppEvent, DisplaySettings, ScreenAction, ScreenId, Setting};

use crate::canvas::{Canvas, CanvasError};
use crate::{
    menu::{self, Menu, MenuColumn, MenuOutcome, MenuRow, MenuState, MenuTitle},
    render::ROW_HEIGHT,
    screen::{Paint, Screen},
    status::STATUS_HEIGHT,
};

/// Rows a titled screen fits; see the note on the menu geometry.
const ROWS_PER_PAGE: usize = 3;
const TITLE_BASELINE_Y: i32 = STATUS_HEIGHT + 20;

/// The leaf a screen id opens, if it opens one.
#[must_use]
pub const fn setting_of(screen: ScreenId) -> Option<Setting> {
    match screen {
        ScreenId::Brightness => Some(Setting::Brightness),
        ScreenId::DimTimeout => Some(Setting::DimTimeout),
        ScreenId::OffTimeout => Some(Setting::OffTimeout),
        ScreenId::HeartRate => Some(Setting::HeartRate),
        ScreenId::HeartRateInterval => Some(Setting::HeartRateInterval),
        ScreenId::WatchfaceSelect => Some(Setting::Watchface),
        _ => None,
    }
}

/// One row per preset, built from the names rather than written out again.
const fn option_rows<const N: usize>(names: &'static [&'static str]) -> [MenuRow; N] {
    let mut rows = [MenuRow::Choice { label: "" }; N];
    let mut index = 0;
    while index < N {
        rows[index] = MenuRow::Choice {
            label: names[index],
        };
        index += 1;
    }
    rows
}

pub struct SettingPickerScreen {
    setting: Setting,
    /// The record this screen was opened with, kept only for the length of the
    /// visit. See the module note on why it must not outlive one.
    settings: DisplaySettings,
    menu: MenuState<ROWS_PER_PAGE>,
    description: &'static Menu,
}

impl Default for SettingPickerScreen {
    fn default() -> Self {
        let setting = Setting::Brightness;
        let description = describe(setting);
        Self {
            setting,
            settings: DisplaySettings::DEFAULT,
            menu: MenuState::new(description),
            description,
        }
    }
}

impl SettingPickerScreen {
    /// Points the screen at a setting and hands it the record that is current.
    ///
    /// Called every time a leaf is entered, which is what keeps the record it
    /// applies a preset to from being one it captured earlier.
    pub fn open(&mut self, setting: Setting, settings: DisplaySettings) {
        self.setting = setting;
        self.settings = settings;
        self.description = describe(setting);
        self.menu = MenuState::new(self.description);
    }

    fn column(&self) -> MenuColumn<'static> {
        // A value outside the presets marks nothing rather than the first row.
        MenuColumn::Selected(self.setting.selected(self.settings).unwrap_or(usize::MAX))
    }
}

impl Paint for SettingPickerScreen {
    fn draw_full(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        menu::draw(
            self.description,
            self.menu.page(),
            self.column(),
            canvas,
            keep_alive,
        )
    }
}

impl Screen for SettingPickerScreen {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction {
        let chosen = self.menu.handle(self.description, event);

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
        // A preset the setters refuse - an off timeout before dimming - leaves
        // the marker where it was, which is the screen saying no.
        let Some(updated) = self.setting.apply(self.settings, entry) else {
            return ScreenAction::None;
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
            menu::draw_rows(
                self.description,
                self.menu.page(),
                self.column(),
                canvas,
                keep_alive,
            )?;
        }
        Ok(())
    }
}

/// The `'static` description belonging to a setting.
const fn describe(setting: Setting) -> &'static Menu {
    match setting {
        Setting::Brightness => &BRIGHTNESS_MENU,
        Setting::DimTimeout => &DIM_MENU,
        Setting::OffTimeout => &OFF_MENU,
        Setting::HeartRate => &HEART_RATE_MENU,
        Setting::HeartRateInterval => &HEART_RATE_INTERVAL_MENU,
        Setting::Watchface => &WATCHFACE_MENU,
    }
}

macro_rules! picker_menu {
    ($menu:ident, $rows:ident, $setting:expr) => {
        static $rows: [MenuRow; $setting.names().len()] = option_rows($setting.names());
        static $menu: Menu = Menu {
            title: Some(MenuTitle {
                text: $setting.title(),
                baseline_y: TITLE_BASELINE_Y,
            }),
            first_row_y: TITLE_BASELINE_Y + 16,
            row_step: ROW_HEIGHT + 6,
            rows: &$rows,
            hint: "> back",
        };
    };
}

picker_menu!(BRIGHTNESS_MENU, BRIGHTNESS_ROWS, Setting::Brightness);
picker_menu!(DIM_MENU, DIM_ROWS, Setting::DimTimeout);
picker_menu!(OFF_MENU, OFF_ROWS, Setting::OffTimeout);
picker_menu!(HEART_RATE_MENU, HEART_RATE_ROWS, Setting::HeartRate);
picker_menu!(
    HEART_RATE_INTERVAL_MENU,
    HEART_RATE_INTERVAL_ROWS,
    Setting::HeartRateInterval
);
picker_menu!(WATCHFACE_MENU, WATCHFACE_ROWS, Setting::Watchface);
