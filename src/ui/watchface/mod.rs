//! The watchface screen and the faces it can show.
//!
//! One module per face, as `InfiniTime` does it. What differs is where the
//! readings live: a face here owns no state, it renders the shared
//! [`WatchState`] that this screen feeds from the event stream. A second face
//! therefore costs its drawing code and nothing else - no second copy of the
//! battery, the step count, or the logic deciding what changed.

mod terminal;

use embedded_graphics::{draw_target::DrawTarget, pixelcolor::Rgb565};
use pineforge_state::{AppEvent, ScreenAction, WatchFields, WatchState};

use crate::ui::screen::Screen;
pub use terminal::TerminalWatchface;

/// Renders the watch state in one particular style.
///
/// Faces draw from a shared reference and cannot remember what they drew, so
/// anything a partial redraw needs to compare against belongs in
/// [`WatchState`], not here.
pub trait Watchface {
    /// Paints the whole face. Must be opaque: the slide transition composes it
    /// stripe by stripe and never clears behind it.
    fn draw_full<D>(
        &self,
        state: &WatchState,
        display: &mut D,
        keep_alive: impl FnMut(),
    ) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>;

    /// Repaints only the parts of the layout that `changed` touches.
    fn draw_changed<D>(
        &self,
        state: &WatchState,
        changed: WatchFields,
        display: &mut D,
        keep_alive: impl FnMut(),
    ) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>;
}

/// The watchface screen: owns the readings and the face showing them.
///
/// The screen stack sees one watchface, whichever face is selected, so
/// navigation is unaffected by the choice. Once a second face exists, this
/// field becomes an enum over the available faces - static dispatch, since
/// there is no allocator to hold a boxed one.
#[derive(Default)]
pub struct WatchfaceScreen {
    state: WatchState,
    face: TerminalWatchface,
}

impl Screen for WatchfaceScreen {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction {
        // Every event is a reading or it is not; where a gesture leads is the
        // navigation contract's business, so a face never returns an action.
        self.state.apply(event);
        ScreenAction::None
    }

    fn draw_full<D>(&self, display: &mut D, keep_alive: impl FnMut()) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        self.face.draw_full(&self.state, display, keep_alive)
    }

    fn draw_dirty<D>(&self, display: &mut D, keep_alive: impl FnMut()) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        self.face
            .draw_changed(&self.state, self.state.changed(), display, keep_alive)
    }
}
