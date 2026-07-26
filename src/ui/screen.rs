use embedded_graphics::{draw_target::DrawTarget, pixelcolor::Rgb565};

use pineforge_state::{AppEvent, ScreenAction};

/// Anything that can paint a complete surface.
///
/// Separate from [`Screen`] because a transition composes the incoming surface
/// once per stripe, and that surface is not always a screen by itself: a screen
/// shown under the status corner has to be painted together with it, or the
/// corner would be missing for the length of the animation and appear
/// afterwards.
pub trait Paint {
    /// Draws every pixel of the surface without relying on a preceding clear.
    ///
    /// This opaque-rendering contract avoids transmitting a redundant full
    /// frame before drawing the actual content. It takes `&self` because the
    /// same surface is drawn once per stripe and must not change while it is
    /// being drawn.
    fn draw_full<D>(&self, display: &mut D, keep_alive: impl FnMut()) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>;
}

/// Application-facing contract implemented by screens and watchfaces.
pub trait Screen: Paint {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction;

    /// Draws only regions changed by the most recently handled event.
    fn draw_dirty<D>(&self, display: &mut D, keep_alive: impl FnMut()) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>;
}
