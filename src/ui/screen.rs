use pineforge_state::{AppEvent, ScreenAction};

use crate::ui::canvas::{Canvas, CanvasError};

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

    /// Draws only regions changed by the most recently handled event.
    fn draw_dirty(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError>;
}
