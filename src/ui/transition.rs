use core::convert::Infallible;

#[cfg(feature = "diagnostics")]
use embassy_time::Instant;
use embedded_graphics::{
    Pixel,
    draw_target::DrawTarget,
    geometry::{Dimensions, Point, Size},
    pixelcolor::{Rgb565, RgbColor},
    primitives::Rectangle,
};
use pineforge_state::NavigationDirection;

#[cfg(feature = "diagnostics")]
use super::metrics::RenderMetrics;
use super::screen::Screen;

const STRIPE_WIDTH: u32 = 24;
const STRIPE_WIDTH_USIZE: usize = 24;
const MAX_SCREEN_HEIGHT: u32 = 240;
const STRIPE_PIXELS: usize = STRIPE_WIDTH_USIZE * 240;

/// Fixed-capacity render buffer for one vertical transition stripe.
///
/// At 24 x 240 RGB565 pixels this uses 11.25 KiB, remains statically allocated,
/// and lets each stripe reach the display in one contiguous transfer.
pub struct SlideBuffer {
    pixels: [Rgb565; STRIPE_PIXELS],
    area: Rectangle,
}

impl SlideBuffer {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pixels: [Rgb565::BLACK; STRIPE_PIXELS],
            area: Rectangle::new(Point::zero(), Size::zero()),
        }
    }

    fn prepare(&mut self, area: Rectangle) {
        debug_assert!(area.size.width <= STRIPE_WIDTH);
        debug_assert!(area.size.height <= MAX_SCREEN_HEIGHT);
        self.area = area;
    }

    fn pixel_index(&self, point: Point) -> Option<usize> {
        if !self.area.contains(point) {
            return None;
        }

        let x = usize::try_from(point.x - self.area.top_left.x).ok()?;
        let y = usize::try_from(point.y - self.area.top_left.y).ok()?;
        Some(y * STRIPE_WIDTH_USIZE + x)
    }
}

impl Default for SlideBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl Dimensions for SlideBuffer {
    fn bounding_box(&self) -> Rectangle {
        self.area
    }
}

impl DrawTarget for SlideBuffer {
    type Color = Rgb565;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(point, color) in pixels {
            if let Some(index) = self.pixel_index(point) {
                self.pixels[index] = color;
            }
        }
        Ok(())
    }

    fn fill_solid(&mut self, area: &Rectangle, color: Self::Color) -> Result<(), Self::Error> {
        let intersection = area.intersection(&self.area);
        let x_start = usize::try_from(intersection.top_left.x - self.area.top_left.x).unwrap_or(0);
        let y_start = usize::try_from(intersection.top_left.y - self.area.top_left.y).unwrap_or(0);
        let width = usize::try_from(intersection.size.width).unwrap_or(0);
        let height = usize::try_from(intersection.size.height).unwrap_or(0);

        for y in y_start..y_start + height {
            let start = y * STRIPE_WIDTH_USIZE + x_start;
            self.pixels[start..start + width].fill(color);
        }
        Ok(())
    }
}

/// Reveals a screen in directional strips without a full-screen framebuffer.
///
/// `Screen::draw_full` guarantees opaque output. Each stripe is composed in
/// `SlideBuffer` and then sent with one contiguous display transaction, making
/// transition speed independent of the number of components in a screen.
pub fn draw_slide_reveal<S, D>(
    screen: &S,
    display: &mut D,
    buffer: &mut SlideBuffer,
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
        buffer.prepare(area);
        #[cfg(feature = "diagnostics")]
        let compose_started = Instant::now();
        match screen.draw_full(buffer, &mut keep_alive) {
            Ok(()) => {}
            Err(error) => match error {},
        }
        #[cfg(feature = "diagnostics")]
        {
            metrics.compose_us += compose_started.elapsed().as_micros();
        }

        let width = usize::try_from(width).unwrap_or(0);
        let height = usize::try_from(area.size.height).unwrap_or(0);
        let pixels = &buffer.pixels;
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
