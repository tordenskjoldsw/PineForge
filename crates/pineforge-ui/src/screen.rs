use pineforge_state::{AppEvent, ScreenAction, SwipeDirection};

use crate::canvas::{Canvas, CanvasError};

/// How much of a paged screen a repaint has to cover.
///
/// Nothing here buffers pixels, so a repaint is an SPI transfer of exactly the
/// area it names and costs time proportional to it. Tapping one tile of a
/// launcher page changes one tile; repainting the page is four times the
/// transfer for the same picture, and the finger feels the difference. This is
/// what lets a screen say which.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Dirty {
    /// Showing the truth already.
    #[default]
    Nothing,
    /// One slot, named by its position on the page. The page rail cannot have
    /// moved, because only paging moves it.
    Slot(usize),
    /// Every slot, and the rail with them.
    Everything,
}

impl Dirty {
    /// Whether `slot` has to be painted in this pass.
    #[must_use]
    pub const fn covers(self, slot: usize) -> bool {
        match self {
            Self::Nothing => false,
            Self::Slot(one) => one == slot,
            Self::Everything => true,
        }
    }

    /// Whether anything at all has to be painted.
    #[must_use]
    pub const fn is_clean(self) -> bool {
        matches!(self, Self::Nothing)
    }
}

/// Anything that can paint a complete surface.
///
/// Separate from [`Screen`] because a transition composes the incoming surface
/// once per stripe, and that surface is not always a screen by itself: a screen
/// shown under the status corner has to be painted together with it, or the
/// corner would be missing for the length of the animation and appear
/// afterwards.
///
/// The methods name [`Canvas`] rather than a type parameter, which is what
/// keeps one screen from being compiled once per backend. It also leaves the
/// trait object-safe, so a screen can be passed as `&dyn Paint` and the code
/// that draws it exists once for all screens.
pub trait Paint {
    /// Draws every pixel of the surface without relying on a preceding clear.
    ///
    /// This opaque-rendering contract avoids transmitting a redundant full
    /// frame before drawing the actual content. It takes `&self` because the
    /// same surface is drawn once per stripe and must not change while it is
    /// being drawn.
    fn draw_full(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError>;
}

/// Application-facing contract implemented by screens and watchfaces.
pub trait Screen: Paint {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction;

    /// Whether this screen will spend the swipe on itself.
    ///
    /// Navigation is offered a gesture first, and for good reason: a screen is
    /// left by the reverse of what opened it, and a screen that could swallow
    /// that gesture would be a screen with no way out. This is the one
    /// exception, and it is narrow by construction - a paged screen answers yes
    /// only while it has a page to turn to in that direction, so the way out is
    /// still there, one page further on.
    ///
    /// Answering yes commits the screen to consuming the event: it will be
    /// handed on unconditionally, and navigation never sees it.
    fn claims(&self, _direction: SwipeDirection) -> bool {
        false
    }

    /// Draws only regions changed by the most recently handled event.
    fn draw_dirty(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError>;
}
