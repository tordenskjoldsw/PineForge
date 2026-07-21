use embedded_graphics::{draw_target::DrawTarget, pixelcolor::Rgb565};

use pineforge_state::{AppEvent, ScreenAction};

/// Application-facing contract implemented by screens and, later, watchfaces.
pub trait Screen {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction;

    /// Draw the complete screen, for example after display initialization.
    fn draw<D>(&self, display: &mut D, keep_alive: impl FnMut()) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>;

    /// Draw only the parts changed by the most recently handled event.
    fn draw_update<D>(&self, display: &mut D, keep_alive: impl FnMut()) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>;
}
