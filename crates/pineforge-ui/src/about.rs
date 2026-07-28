//! What this build is: the release, the commit it came from, and its date.
//!
//! None of it is known to this crate. A screen is layout, and the identity of
//! the firmware belongs to the firmware - which reads it from its own build
//! script and hands it over at start-up, the same way it hands over the
//! confirmation state and the settings record. `CARGO_PKG_VERSION` compiled in
//! here would name *this crate*, which is how the firmware screen came to
//! announce version 0.1.0 on a watch running 0.2.1.

use embedded_graphics::{draw_target::DrawTarget, geometry::Point};
use pineforge_state::{AppEvent, ScreenAction};

use crate::canvas::{Canvas, CanvasError};
use crate::font::ui_text;
use crate::{
    render::{ROW_X, draw_mono_text_visible},
    screen::{Paint, Screen},
    status::STATUS_HEIGHT,
    theme,
};

const TITLE_BASELINE_Y: i32 = STATUS_HEIGHT + 20;
const FIRST_LINE_Y: i32 = TITLE_BASELINE_Y + 40;
const LINE_STEP: i32 = 30;
/// Where values start. Clears the longest label with a space to spare, and
/// leaves room for twelve characters before the right edge.
const VALUE_X: i32 = ROW_X + 90;
const HINT_BASELINE_Y: i32 = 232;

/// What the firmware knows about itself, in the words it will be shown in.
///
/// Every field is a `&'static str` because every one of them is a compile-time
/// constant of the binary - formatting none of it at runtime keeps this screen
/// free of buffers and of the formatting machinery that fills them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildInfo {
    /// The packaged release, build number and all: `0.2.1+20`.
    pub version: &'static str,
    /// Abbreviated commit, suffixed `-dirty` when the tree was not clean.
    pub commit: &'static str,
    /// The commit's date, not the build's. See the build script.
    pub date: &'static str,
}

impl BuildInfo {
    /// What a build that could not identify itself reports.
    ///
    /// Shown rather than hidden: a screen whose whole job is to say which build
    /// this is should say when it cannot, instead of leaving a blank that reads
    /// as a drawing fault.
    pub const UNKNOWN: Self = Self {
        version: "unknown",
        commit: "unknown",
        date: "unknown",
    };
}

impl Default for BuildInfo {
    fn default() -> Self {
        Self::UNKNOWN
    }
}

/// Names this build, for telling two of them apart on the wrist.
#[derive(Default)]
pub struct AboutScreen {
    build: BuildInfo,
}

impl AboutScreen {
    /// Handed over by the firmware at start-up; see the module note.
    pub const fn set_build(&mut self, build: BuildInfo) {
        self.build = build;
    }
}

impl Paint for AboutScreen {
    fn draw_full(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        canvas.clear(theme::BACKGROUND)?;
        keep_alive();

        let label = ui_text(theme::TEXT, theme::BACKGROUND);
        let value = ui_text(theme::ACCENT, theme::BACKGROUND);
        draw_mono_text_visible("ABOUT", Point::new(ROW_X, TITLE_BASELINE_Y), label, canvas)?;
        keep_alive();

        for (index, (name, text)) in [
            ("BUILD", self.build.version),
            ("COMMIT", self.build.commit),
            ("DATE", self.build.date),
        ]
        .into_iter()
        .enumerate()
        {
            let y = FIRST_LINE_Y + LINE_STEP * i32::try_from(index).unwrap_or(0);
            draw_mono_text_visible(name, Point::new(ROW_X, y), label, canvas)?;
            draw_mono_text_visible(text, Point::new(VALUE_X, y), value, canvas)?;
            keep_alive();
        }

        draw_mono_text_visible("> back", Point::new(ROW_X, HINT_BASELINE_Y), label, canvas)
    }
}

impl Screen for AboutScreen {
    /// Nothing here is a control. Leaving is the back gesture or the side
    /// button, both of which are resolved above a screen.
    fn handle_event(&mut self, _event: AppEvent) -> ScreenAction {
        ScreenAction::None
    }

    /// Nothing on this screen moves, so a redraw is only ever the full one.
    fn draw_dirty(
        &self,
        _canvas: &mut Canvas<'_>,
        _keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        Ok(())
    }
}
