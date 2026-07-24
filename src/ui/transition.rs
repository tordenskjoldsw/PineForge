#[cfg(feature = "diagnostics")]
use embassy_time::Instant;
use embedded_graphics::{
    draw_target::DrawTarget,
    geometry::{Point, Size},
    pixelcolor::Rgb565,
    primitives::Rectangle,
};
use pineforge_state::NavigationDirection;

#[cfg(feature = "diagnostics")]
use super::metrics::RenderMetrics;
use super::scratch::{STRIPE_WIDTH, STRIPE_WIDTH_USIZE, UiScratch};
use super::screen::Screen;

/// Reveals a screen in directional strips without a full-screen framebuffer.
///
/// `Screen::draw_full` guarantees opaque output. Each stripe is composed in the
/// display task's shared [`UiScratch`] and then sent with one contiguous
/// transaction, making transition speed independent of component count.
pub fn draw_slide_reveal<S, D>(
    screen: &S,
    display: &mut D,
    scratch: &mut UiScratch,
    direction: NavigationDirection,
    mut keep_alive: impl FnMut(),
) -> Result<TransitionOutput, D::Error>
where
    S: Screen,
    D: DrawTarget<Color = Rgb565>,
{
    #[cfg(feature = "diagnostics")]
    let transition_started = Instant::now();
    #[cfg(feature = "diagnostics")]
    let mut metrics = RenderMetrics::default();
    let screen_area = display.bounding_box();
    let stripe_count = screen_area.size.width.div_ceil(STRIPE_WIDTH);

    for index in 0..stripe_count {
        #[cfg(feature = "diagnostics")]
        let stripe_started = Instant::now();
        let stripe_index = match direction {
            NavigationDirection::Forward => stripe_count - index - 1,
            NavigationDirection::Backward => index,
        };
        let x_offset = stripe_index * STRIPE_WIDTH;
        let width = STRIPE_WIDTH.min(screen_area.size.width - x_offset);
        let area = Rectangle::new(
            Point::new(
                screen_area.top_left.x + i32::try_from(x_offset).unwrap_or(0),
                screen_area.top_left.y,
            ),
            Size::new(width, screen_area.size.height),
        );
        scratch.prepare_stripe(area);
        #[cfg(feature = "diagnostics")]
        let compose_started = Instant::now();
        match screen.draw_full(scratch, &mut keep_alive) {
            Ok(()) => {}
            Err(error) => match error {},
        }
        #[cfg(feature = "diagnostics")]
        {
            metrics.compose_us += compose_started.elapsed().as_micros();
        }

        let width = usize::try_from(width).unwrap_or(0);
        let height = usize::try_from(area.size.height).unwrap_or(0);
        let pixels = &scratch.pixels;
        let colors =
            (0..height).flat_map(|y| (0..width).map(move |x| pixels[y * STRIPE_WIDTH_USIZE + x]));
        #[cfg(feature = "diagnostics")]
        let transfer_started = Instant::now();
        display.fill_contiguous(&area, colors)?;
        #[cfg(feature = "diagnostics")]
        {
            metrics.transfer_us += transfer_started.elapsed().as_micros();
            metrics.max_stripe_us = metrics
                .max_stripe_us
                .max(stripe_started.elapsed().as_micros());
            metrics.stripe_count = metrics.stripe_count.saturating_add(1);
        }
        keep_alive();
    }

    #[cfg(feature = "diagnostics")]
    {
        metrics.total_us = transition_started.elapsed().as_micros();
        Ok(metrics)
    }
    #[cfg(not(feature = "diagnostics"))]
    Ok(())
}

#[cfg(feature = "diagnostics")]
pub type TransitionOutput = RenderMetrics;

#[cfg(not(feature = "diagnostics"))]
pub type TransitionOutput = ();
