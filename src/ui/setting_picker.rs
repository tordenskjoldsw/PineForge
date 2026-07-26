//! One screen type for every settings leaf.
//!
//! A leaf is always the same screen: the presets a setting can take, the
//! current one marked, and choosing applies at once. What differs between
//! brightness and a screen timeout is which table the rows come from and which
//! setter runs - a difference of data, not of drawing. So there is one screen
//! here and a [`Setting`] naming which one it is, rather than four screens that
//! would drift apart the first time a row style changed.

use pineforge_state::{
    AppEvent, BRIGHTNESS_LEVELS, BRIGHTNESS_NAMES, DIM_TIMEOUT_NAMES, DIM_TIMEOUTS_MILLIS,
    DisplaySettings, HEART_RATE_ENABLED_NAMES, HEART_RATE_INTERVAL_NAMES,
    HEART_RATE_INTERVALS_SECONDS, OFF_TIMEOUT_NAMES, OFF_TIMEOUTS_MILLIS, ScreenAction,
};

use crate::ui::canvas::{Canvas, CanvasError};
use crate::ui::{
    menu::{self, Menu, MenuColumn, MenuOutcome, MenuRow, MenuState, MenuTitle},
    render::ROW_HEIGHT,
    screen::{Paint, Screen},
    status::STATUS_HEIGHT,
};

/// Rows a titled screen fits; see the note on the menu geometry.
const ROWS_PER_PAGE: usize = 3;
const TITLE_BASELINE_Y: i32 = STATUS_HEIGHT + 20;

/// Which setting a picker edits.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    Brightness,
    DimTimeout,
    OffTimeout,
    HeartRate,
    HeartRateInterval,
}

/// Every setting a picker can edit, in the order the screens hold them.
pub const SETTINGS: [Setting; 5] = [
    Setting::Brightness,
    Setting::DimTimeout,
    Setting::OffTimeout,
    Setting::HeartRate,
    Setting::HeartRateInterval,
];

impl Setting {
    /// Where this setting's picker sits in the array of them.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Brightness => 0,
            Self::DimTimeout => 1,
            Self::OffTimeout => 2,
            Self::HeartRate => 3,
            Self::HeartRateInterval => 4,
        }
    }

    /// The names of this setting's presets, in the order they are offered.
    const fn names(self) -> &'static [&'static str] {
        match self {
            Self::Brightness => &BRIGHTNESS_NAMES,
            Self::DimTimeout => &DIM_TIMEOUT_NAMES,
            Self::OffTimeout => &OFF_TIMEOUT_NAMES,
            Self::HeartRate => &HEART_RATE_ENABLED_NAMES,
            Self::HeartRateInterval => &HEART_RATE_INTERVAL_NAMES,
        }
    }

    const fn title(self) -> &'static str {
        match self {
            Self::Brightness => "BRIGHTNESS",
            Self::DimTimeout => "DIM AFTER",
            Self::OffTimeout => "SCREEN OFF",
            Self::HeartRate => "HEART RATE",
            Self::HeartRateInterval => "HR INTERVAL",
        }
    }

    /// Which preset the settings currently hold, if it is one of them.
    ///
    /// A value the table does not list leaves nothing marked rather than
    /// marking the first row, so a record written by another build cannot make
    /// the picker claim a preset it is not using.
    fn selected(self, settings: DisplaySettings) -> usize {
        let found = match self {
            Self::Brightness => BRIGHTNESS_LEVELS
                .iter()
                .position(|level| *level == settings.brightness()),
            Self::DimTimeout => DIM_TIMEOUTS_MILLIS
                .iter()
                .position(|millis| *millis == settings.dim_after_millis()),
            Self::OffTimeout => OFF_TIMEOUTS_MILLIS
                .iter()
                .position(|millis| *millis == settings.off_after_millis()),
            Self::HeartRate => Some(usize::from(settings.heart_rate_enabled())),
            Self::HeartRateInterval => HEART_RATE_INTERVALS_SECONDS
                .iter()
                .position(|seconds| *seconds == settings.heart_rate_interval_seconds()),
        };
        found.unwrap_or(usize::MAX)
    }

    /// The settings with this preset chosen.
    ///
    /// Returning the whole record rather than mutating in place is what lets
    /// the setters keep their own invariants - choosing a dim timeout may move
    /// the off timeout with it, and only the state crate knows that.
    fn apply(self, settings: DisplaySettings, entry: usize) -> Option<DisplaySettings> {
        let updated = match self {
            Self::Brightness => settings.with_brightness(*BRIGHTNESS_LEVELS.get(entry)?),
            Self::DimTimeout => settings.with_dim_timeout(*DIM_TIMEOUTS_MILLIS.get(entry)?),
            Self::OffTimeout => settings.with_off_timeout(*OFF_TIMEOUTS_MILLIS.get(entry)?),
            Self::HeartRate => settings.with_heart_rate_enabled(entry == 1),
            Self::HeartRateInterval => {
                settings.with_heart_rate_interval(*HEART_RATE_INTERVALS_SECONDS.get(entry)?)
            }
        };
        (updated != settings).then_some(updated)
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
    settings: DisplaySettings,
    menu: MenuState<ROWS_PER_PAGE>,
    description: &'static Menu,
}

impl SettingPickerScreen {
    #[must_use]
    pub fn new(setting: Setting) -> Self {
        let description = describe(setting);
        Self {
            setting,
            settings: DisplaySettings::DEFAULT,
            menu: MenuState::new(description),
            description,
        }
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
            MenuColumn::Selected(self.setting.selected(self.settings)),
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
                MenuColumn::Selected(self.setting.selected(self.settings)),
                canvas,
                keep_alive,
            )?;
        }
        Ok(())
    }
}

/// The `'static` description belonging to a setting.
///
/// One constant per setting rather than one built at run time: the rows are
/// known at compile time, and a screen holding its own copy would put four
/// menus in RAM for no reason.
const fn describe(setting: Setting) -> &'static Menu {
    match setting {
        Setting::Brightness => &BRIGHTNESS_MENU,
        Setting::DimTimeout => &DIM_MENU,
        Setting::OffTimeout => &OFF_MENU,
        Setting::HeartRate => &HEART_RATE_MENU,
        Setting::HeartRateInterval => &HEART_RATE_INTERVAL_MENU,
    }
}

macro_rules! picker_menu {
    ($menu:ident, $rows:ident, $setting:expr, $len:expr) => {
        static $rows: [MenuRow; $len] = option_rows($setting.names());
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

picker_menu!(
    BRIGHTNESS_MENU,
    BRIGHTNESS_ROWS,
    Setting::Brightness,
    BRIGHTNESS_NAMES.len()
);
picker_menu!(
    DIM_MENU,
    DIM_ROWS,
    Setting::DimTimeout,
    DIM_TIMEOUT_NAMES.len()
);
picker_menu!(
    OFF_MENU,
    OFF_ROWS,
    Setting::OffTimeout,
    OFF_TIMEOUT_NAMES.len()
);
picker_menu!(
    HEART_RATE_MENU,
    HEART_RATE_ROWS,
    Setting::HeartRate,
    HEART_RATE_ENABLED_NAMES.len()
);
picker_menu!(
    HEART_RATE_INTERVAL_MENU,
    HEART_RATE_INTERVAL_ROWS,
    Setting::HeartRateInterval,
    HEART_RATE_INTERVAL_NAMES.len()
);
