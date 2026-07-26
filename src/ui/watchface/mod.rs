//! The watchface screen and the faces it can show.
//!
//! One module per face, as `InfiniTime` does it. What differs is where the
//! readings live: a face here owns no state, it renders the shared
//! [`WatchState`] that this screen feeds from the event stream. A second face
//! therefore costs its drawing code and nothing else - no second copy of the
//! battery, the step count, or the logic deciding what changed.

#[cfg(feature = "diagnostics")]
mod diagnostics;
mod row;
mod terminal;

use pineforge_state::{AppEvent, ScreenAction, WatchFields, WatchState, WatchfaceId};

use crate::ui::canvas::{Canvas, CanvasError};
use crate::ui::screen::{Paint, Screen};
#[cfg(feature = "diagnostics")]
pub use diagnostics::DiagnosticsWatchface;
pub use terminal::TerminalWatchface;

/// Renders the watch state in one particular style.
///
/// Faces draw from a shared reference and cannot remember what they drew, so
/// anything a partial redraw needs to compare against belongs in
/// [`WatchState`], not here.
pub trait Watchface {
    /// Paints the whole face. Must be opaque: the slide transition composes it
    /// stripe by stripe and never clears behind it.
    fn draw_full(
        &self,
        state: &WatchState,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError>;

    /// Repaints only the parts of the layout that `changed` touches.
    fn draw_changed(
        &self,
        state: &WatchState,
        changed: WatchFields,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError>;
}

/// A face this build can show, with its state.
///
/// Static dispatch: there is no allocator to hold a boxed face, and an enum
/// keeps the compiler checking that every face is handled. It costs the size of
/// its largest variant, which is why faces hold no readings.
enum ActiveWatchface {
    Terminal(TerminalWatchface),
    #[cfg(feature = "diagnostics")]
    Diagnostics(DiagnosticsWatchface),
}

impl ActiveWatchface {
    /// Exhaustive in every build: an id for a face this build lacks cannot be
    /// constructed, because decoding a settings record substitutes the default
    /// for one it does not recognize.
    const fn new(id: WatchfaceId) -> Self {
        match id {
            WatchfaceId::Terminal => Self::Terminal(TerminalWatchface),
            #[cfg(feature = "diagnostics")]
            WatchfaceId::Diagnostics => Self::Diagnostics(DiagnosticsWatchface),
        }
    }

    const fn id(&self) -> WatchfaceId {
        match self {
            Self::Terminal(_) => WatchfaceId::Terminal,
            #[cfg(feature = "diagnostics")]
            Self::Diagnostics(_) => WatchfaceId::Diagnostics,
        }
    }
}

impl Watchface for ActiveWatchface {
    fn draw_full(
        &self,
        state: &WatchState,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        match self {
            Self::Terminal(face) => face.draw_full(state, canvas, keep_alive),
            #[cfg(feature = "diagnostics")]
            Self::Diagnostics(face) => face.draw_full(state, canvas, keep_alive),
        }
    }

    fn draw_changed(
        &self,
        state: &WatchState,
        changed: WatchFields,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        match self {
            Self::Terminal(face) => face.draw_changed(state, changed, canvas, keep_alive),
            #[cfg(feature = "diagnostics")]
            Self::Diagnostics(face) => face.draw_changed(state, changed, canvas, keep_alive),
        }
    }
}

/// The watchface screen: owns the readings and the face showing them.
///
/// The screen stack sees one watchface whichever face is selected, so
/// navigation is unaffected by the choice.
pub struct WatchfaceScreen {
    state: WatchState,
    face: ActiveWatchface,
}

impl Default for WatchfaceScreen {
    fn default() -> Self {
        Self {
            state: WatchState::new(),
            face: ActiveWatchface::new(WatchfaceId::default()),
        }
    }
}

impl WatchfaceScreen {
    /// Switches to a face, reporting whether the screen has to be repainted.
    ///
    /// Selecting the showing face is not a change, so a settings update that
    /// leaves the choice alone costs no redraw.
    pub fn select(&mut self, id: WatchfaceId) -> bool {
        if self.face.id() == id {
            return false;
        }
        self.face = ActiveWatchface::new(id);
        true
    }
}

impl Paint for WatchfaceScreen {
    fn draw_full(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        self.face.draw_full(&self.state, canvas, keep_alive)
    }
}

impl Screen for WatchfaceScreen {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction {
        // Every event is a reading or it is not; where a gesture leads is the
        // navigation contract's business, so a face never returns an action.
        self.state.apply(event);
        ScreenAction::None
    }

    fn draw_dirty(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        self.face
            .draw_changed(&self.state, self.state.changed(), canvas, keep_alive)
    }
}
