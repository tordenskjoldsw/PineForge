#[cfg(feature = "diagnostics")]
use embassy_time::Instant;
use embedded_graphics::{
    draw_target::DrawTarget,
    geometry::{Point, Size},
    pixelcolor::Rgb565,
    primitives::Rectangle,
};
use pineforge_state::{Navigation, SwipeDirection};

use super::canvas::Canvas;
#[cfg(feature = "diagnostics")]
use super::metrics::RenderMetrics;
use super::scratch::{STRIPE_THICKNESS, UiScratch};
use super::screen::Paint;

/// A panel whose controller shows a movable window onto its frame memory.
///
/// The controller this firmware drives holds more pixels than the panel shows,
/// and a register decides which of them are on screen. That is worth a trait of
/// its own because it changes what an animation costs: content written into the
/// rows that are not showing costs what any other write costs, and moving the
/// window costs nothing. A screen can therefore be slid in for one frame's worth
/// of pixels rather than one frame per step, which is the difference between
/// motion and a slideshow on a bus that needs 115 ms for a full frame.
///
/// The window wraps at the end of memory, and it does not return to where it
/// started: after a slide, screen row 0 is some other memory row for good. An
/// implementation owes the translation to every later draw, so this trait
/// deliberately addresses memory rows and nothing above it may.
pub trait ScrollingPanel: DrawTarget<Color = Rgb565> {
    /// Rows of frame memory the window moves within.
    const MEMORY_ROWS: u16;

    /// The memory row currently shown at the top of the panel.
    fn origin(&self) -> u16;

    /// Paints full-width rows at an absolute memory row, wrapping at the end.
    fn fill_memory_rows(&mut self, row: u16, pixels: &[Rgb565]) -> Result<(), Self::Error>;

    /// Shows the window from this memory row on.
    fn show_from(&mut self, row: u16) -> Result<(), Self::Error>;
}

/// Brings a screen on the way the gesture that called for it travelled.
///
/// Vertically this is a real slide: the panel's own window does the moving, so
/// the outgoing screen is carried off without a single one of its pixels being
/// sent again. Horizontally there is no such register, and the best the hardware
/// allows is to reveal the incoming screen in strips from the edge it enters
/// through. Both compose the screen once per strip in the display task's shared
/// [`UiScratch`], so their cost is the frame, not the number of components on it.
///
/// The screen arrives as `&dyn Paint` so this exists once rather than once per
/// screen that can move. The panel stays a type parameter: the strip blit is the
/// hot path of every navigation and is kept on the concrete target, where it
/// neither pays for a vtable nor loses its iterator.
pub fn draw_slide_reveal<D>(
    screen: &dyn Paint,
    display: &mut D,
    scratch: &mut UiScratch,
    navigation: Navigation,
    keep_alive: &mut dyn FnMut(),
) -> Result<TransitionOutput, D::Error>
where
    D: ScrollingPanel,
{
    match navigation.motion {
        SwipeDirection::Up => draw_scroll_slide(screen, display, scratch, false, keep_alive),
        SwipeDirection::Down => draw_scroll_slide(screen, display, scratch, true, keep_alive),
        // Content moving left uncovers the far edge first, so the reveal starts
        // there and walks back towards the gesture's origin.
        SwipeDirection::Left => draw_strip_reveal(screen, display, scratch, true, keep_alive),
        SwipeDirection::Right => draw_strip_reveal(screen, display, scratch, false, keep_alive),
    }
}

/// Slides a screen in by moving the panel's window over it.
///
/// Each step writes one band of the incoming screen into memory the panel is not
/// showing, then moves the window by exactly that band. The band it has just
/// written is the strip the move uncovers, so the picture is complete at every
/// step and the screen being left slides away without being redrawn at all.
///
/// `downward` is the way the content travels: a screen opened by pulling down
/// comes in from above.
fn draw_scroll_slide<D>(
    screen: &dyn Paint,
    display: &mut D,
    scratch: &mut UiScratch,
    downward: bool,
    keep_alive: &mut dyn FnMut(),
) -> Result<TransitionOutput, D::Error>
where
    D: ScrollingPanel,
{
    #[cfg(feature = "diagnostics")]
    let transition_started = Instant::now();
    #[cfg(feature = "diagnostics")]
    let mut metrics = RenderMetrics::default();
    let screen_area = display.bounding_box();
    let memory = u32::from(D::MEMORY_ROWS);
    let visible = screen_area.size.height;
    let width = screen_area.size.width;
    let origin = u32::from(display.origin());
    let band_count = visible.div_ceil(STRIPE_THICKNESS);
    // Where the window comes to rest, and therefore where every band of the
    // incoming screen has to be written for it to be there when it arrives.
    let settled = if downward {
        (origin + memory - visible) % memory
    } else {
        (origin + visible) % memory
    };

    for index in 0..band_count {
        #[cfg(feature = "diagnostics")]
        let band_started = Instant::now();
        // Coming down, the screen is brought on from its foot: the band that
        // ends up at the bottom is the one the window uncovers first.
        let band = if downward {
            band_count - index - 1
        } else {
            index
        };
        let top = band * STRIPE_THICKNESS;
        let thickness = STRIPE_THICKNESS.min(visible - top);
        let area = Rectangle::new(
            Point::new(
                screen_area.top_left.x,
                screen_area.top_left.y + i32::try_from(top).unwrap_or(0),
            ),
            Size::new(width, thickness),
        );
        scratch.prepare_stripe(area);
        #[cfg(feature = "diagnostics")]
        let compose_started = Instant::now();
        // The scratch buffer cannot fail a write, but the canvas that carries it
        // erases that guarantee along with the backend's error type. There is
        // nothing to recover from either way: a band that failed to compose
        // still has to be sent, or the window would move over nothing.
        let _ = screen.draw_full(&mut Canvas::new(scratch), keep_alive);
        #[cfg(feature = "diagnostics")]
        {
            metrics.compose_us += compose_started.elapsed().as_micros();
        }

        let row = (settled + top) % memory;
        let count = (width * thickness) as usize;
        #[cfg(feature = "diagnostics")]
        let transfer_started = Instant::now();
        display.fill_memory_rows(row_of(row), &scratch.pixels[..count])?;
        // Going up the window ends this step one band further on; coming down it
        // ends on the band just written, which is now its top row.
        let shown = if downward {
            row
        } else {
            (origin + top + thickness) % memory
        };
        display.show_from(row_of(shown))?;
        #[cfg(feature = "diagnostics")]
        {
            metrics.transfer_us += transfer_started.elapsed().as_micros();
            metrics.max_stripe_us = metrics
                .max_stripe_us
                .max(band_started.elapsed().as_micros());
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

/// A memory row, which is always in range by construction above.
#[allow(clippy::cast_possible_truncation)]
const fn row_of(row: u32) -> u16 {
    row as u16
}

/// Reveals a screen in strips from the edge it enters through.
///
/// What the hardware allows across the panel: nothing moves, but the incoming
/// screen arrives from the side the gesture came from rather than all at once.
/// `from_far_edge` starts the reveal at the far side and walks it back.
fn draw_strip_reveal<D>(
    screen: &dyn Paint,
    display: &mut D,
    scratch: &mut UiScratch,
    from_far_edge: bool,
    keep_alive: &mut dyn FnMut(),
) -> Result<TransitionOutput, D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    #[cfg(feature = "diagnostics")]
    let transition_started = Instant::now();
    #[cfg(feature = "diagnostics")]
    let mut metrics = RenderMetrics::default();
    let screen_area = display.bounding_box();
    let span = screen_area.size.width;
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
        let area = Rectangle::new(
            Point::new(screen_area.top_left.x + offset, screen_area.top_left.y),
            Size::new(thickness, screen_area.size.height),
        );
        scratch.prepare_stripe(area);
        #[cfg(feature = "diagnostics")]
        let compose_started = Instant::now();
        let _ = screen.draw_full(&mut Canvas::new(scratch), keep_alive);
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
