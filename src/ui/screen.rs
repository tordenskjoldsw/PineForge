use embedded_graphics::{draw_target::DrawTarget, pixelcolor::Rgb565};

use pineforge_state::{AppEvent, ScreenAction};

/// Application-facing contract implemented by screens and, later, watchfaces.
pub trait Screen {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction;

    /// Draws every pixel of the screen without relying on a preceding clear.
    ///
    /// This opaque-rendering contract avoids transmitting a redundant full
    /// frame before drawing the actual screen.
    fn draw_full<D>(&self, display: &mut D, keep_alive: impl FnMut()) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>;

    /// Draws only regions changed by the most recently handled event.
    fn draw_dirty<D>(&self, display: &mut D, keep_alive: impl FnMut()) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>;
}
