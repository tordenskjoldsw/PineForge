#[cfg(feature = "diagnostics")]
use embassy_time::Instant;
use embedded_graphics::{
    draw_target::DrawTarget,
    geometry::{Point, Size},
    pixelcolor::Rgb565,
    primitives::Rectangle,
};
use pineforge_state::{Navigation, SwipeDirection};

#[cfg(feature = "diagnostics")]
use super::metrics::RenderMetrics;
use super::scratch::{STRIPE_THICKNESS, UiScratch};
use super::screen::Paint;

/// Reveals a screen in strips along the axis the navigation travelled.
///
/// `Screen::draw_full` guarantees opaque output. Each stripe is composed in the
/// display task's shared [`UiScratch`] and then sent with one contiguous
/// transaction, making transition speed independent of component count.
///
/// The incoming screen is revealed from the edge it enters through, which is
/// the edge the gesture came from, so the motion follows the finger.
pub fn draw_slide_reveal<S, D>(
    screen: &S,
    display: &mut D,
    scratch: &mut UiScratch,
    navigation: Navigation,
    mut keep_alive: impl FnMut(),
) -> Result<TransitionOutput, D::Error>
where
    S: Paint,
    D: DrawTarget<Color = Rgb565>,
{
    #[cfg(feature = "diagnostics")]
    let transition_started = Instant::now();
    #[cfg(feature = "diagnostics")]
    let mut metrics = RenderMetrics::default();
    let screen_area = display.bounding_box();
    let vertical = matches!(navigation.motion, SwipeDirection::Up | SwipeDirection::Down);
    // Content moving left or up uncovers the far edge first, so the reveal
    // starts there and walks back towards the gesture's origin.
    let from_far_edge = matches!(navigation.motion, SwipeDirection::Left | SwipeDirection::Up);
    let span = if vertical {
        screen_area.size.height
    } else {
        screen_area.size.width
    };
    let stripe_count = span.div_ceil(STRIPE_THICKNESS);

    for index in 0..stripe_count {
        #[cfg(feature = "diagnostics")]
        let stripe_started = Instant::now();
        let stripe_index = if from_far_edge {
            stripe_count - index - 1
        } else {
            index
        };
        let offset = stripe_index * STRIPE_THICKNESS;
        let thickness = STRIPE_THICKNESS.min(span - offset);
        let offset = i32::try_from(offset).unwrap_or(0);
        let area = if vertical {
            Rectangle::new(
                Point::new(screen_area.top_left.x, screen_area.top_left.y + offset),
                Size::new(screen_area.size.width, thickness),
            )
        } else {
            Rectangle::new(
                Point::new(screen_area.top_left.x + offset, screen_area.top_left.y),
                Size::new(thickness, screen_area.size.height),
            )
        };
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

        let width = usize::try_from(area.size.width).unwrap_or(0);
        let height = usize::try_from(area.size.height).unwrap_or(0);
        let stride = scratch.stride();
        let pixels = &scratch.pixels;
        let colors = (0..height).flat_map(|y| (0..width).map(move |x| pixels[y * stride + x]));
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
