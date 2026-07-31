use core::fmt::Write;

use embedded_graphics::{
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::{Alignment, Text},
};
use heapless::String;

use crate::canvas::{Canvas, CanvasError};
use crate::font::{AaTextStyle, hint_text, ui_text};
use crate::{
    metrics::RenderMetrics,
    render::{draw_mono_text_visible, draw_visible},
    screen::{Paint, Screen},
};
#[cfg(feature = "ui-animations")]
use pineforge_state::NavigationDirection;
use pineforge_state::{AppEvent, ScreenAction};

const TOUCH_AREA: Rectangle = Rectangle::new(Point::new(0, 64), Size::new(240, 76));
const METRICS_AREA: Rectangle = Rectangle::new(Point::new(0, 140), Size::new(240, 74));
const FOOTER_AREA: Rectangle = Rectangle::new(Point::new(0, 214), Size::new(240, 26));
const TOUCH_MARKER_SIZE: Size = Size::new(13, 13);

/// Where the stack reading sits: inside the blue header, under the title.
const STACK_AREA: Rectangle = Rectangle::new(Point::new(0, 46), Size::new(240, 16));
const STACK_BASELINE_Y: i32 = 58;

#[derive(Default)]
pub struct TestScreen {
    last_touch: Option<Point>,
    previous_touch: Option<Point>,
    touching: bool,
    previous_touching: bool,
    forward_metrics: Option<RenderMetrics>,
    backward_metrics: Option<RenderMetrics>,
    /// Deepest the stack has been, and how much there is, in bytes. The
    /// firmware measures it; this screen is only where it can be read on a watch
    /// with no debugger attached.
    stack: Option<(usize, usize)>,
    previous_stack: Option<(usize, usize)>,
    /// What the last paint straight onto the panel cost, in microseconds.
    ///
    /// The transition reports its own split above, and for a long time that was
    /// the only render this screen could show a number for - which meant the
    /// paths that actually cost the most, a wake and a face swap, were the ones
    /// nothing measured. These are those: a full repaint and a partial one, kept
    /// apart because the whole point is how far apart they are.
    full_paint_us: Option<u64>,
    dirty_paint_us: Option<u64>,
}

impl TestScreen {
    /// Records the stack's high-water mark for display.
    ///
    /// Fed on the tick rather than polled here, because reading it means walking
    /// the painted region and that belongs to the firmware that painted it.
    pub const fn set_stack(&mut self, used: usize, capacity: usize) {
        self.previous_stack = self.stack;
        self.stack = Some((used, capacity));
    }

    /// The mark, over its budget. Drawn on the header rather than with the
    /// render metrics below, because it is the reading this screen currently
    /// exists to deliver and it should not need looking for.
    fn draw_stack(&self, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        STACK_AREA
            .into_styled(PrimitiveStyle::with_fill(Rgb565::BLUE))
            .draw(canvas)?;
        let mut line = String::<24>::new();
        match self.stack {
            Some((used, capacity)) => {
                let _ = write!(line, "STACK {used} / {capacity}");
            }
            None => {
                let _ = line.push_str("STACK ---");
            }
        }
        draw_visible(
            &Text::with_alignment(
                &line,
                Point::new(120, STACK_BASELINE_Y),
                hint_text(Rgb565::WHITE, Rgb565::BLUE),
                Alignment::Center,
            ),
            canvas,
        )
    }

    /// Records what a paint straight onto the panel cost.
    ///
    /// Fed by the registry, which is the one place every paint passes through,
    /// rather than by each of the display task's call sites - a measurement that
    /// depends on remembering to take it measures the sites somebody remembered.
    pub const fn record_paint(&mut self, full: bool, micros: u64) {
        if full {
            self.full_paint_us = Some(micros);
        } else {
            self.dirty_paint_us = Some(micros);
        }
    }

    #[cfg(feature = "ui-animations")]
    pub const fn record_transition(
        &mut self,
        direction: NavigationDirection,
        metrics: RenderMetrics,
    ) {
        match direction {
            NavigationDirection::Forward => self.forward_metrics = Some(metrics),
            NavigationDirection::Backward => self.backward_metrics = Some(metrics),
        }
    }

    pub fn draw_metrics(&self, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        METRICS_AREA
            .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
            .draw(canvas)?;
        let style = hint_text(Rgb565::WHITE, Rgb565::BLACK);
        Self::draw_metric_line(canvas, "F", self.forward_metrics, 154, style)?;
        Self::draw_metric_line(canvas, "B", self.backward_metrics, 170, style)?;
        Self::draw_max_line(
            canvas,
            self.forward_metrics,
            self.backward_metrics,
            190,
            style,
        )?;
        // Where the stripe count used to be. That number is `STRIPE_THICKNESS`
        // divided into the panel and could never say anything a constant does
        // not; these two can, and there is no fifth line in this band.
        //
        // Microseconds for both, and said so. A full repaint runs to six digits
        // and a partial one to three or four, so the pair only reads as a
        // comparison if it is in one unit - which is the whole reason they sit
        // on the same line.
        let mut paint_line = String::<32>::new();
        let _ = paint_line.push_str("PAINT us");
        match self.full_paint_us {
            Some(micros) => {
                let _ = write!(paint_line, " F{micros}");
            }
            None => {
                let _ = paint_line.push_str(" F---");
            }
        }
        match self.dirty_paint_us {
            Some(micros) => {
                let _ = write!(paint_line, " D{micros}");
            }
            None => {
                let _ = paint_line.push_str(" D---");
            }
        }
        draw_mono_text_visible(&paint_line, Point::new(0, 206), style, canvas)?;
        Ok(())
    }

    /// Touch contact state, relocated here from the watchface status row.
    fn draw_touch_state(&self, canvas: &mut Canvas<'_>) -> Result<(), CanvasError> {
        Rectangle::new(Point::new(0, 214), Size::new(50, 26))
            .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
            .draw(canvas)?;
        let style = hint_text(
            if self.touching {
                Rgb565::YELLOW
            } else {
                Rgb565::WHITE
            },
            Rgb565::BLACK,
        );
        let state = if self.touching { "AKTIV" } else { "BEREIT" };
        draw_mono_text_visible(state, Point::new(2, 232), style, canvas)
    }

    fn draw_metric_line(
        canvas: &mut Canvas<'_>,
        label: &str,
        metrics: Option<RenderMetrics>,
        baseline: i32,
        style: AaTextStyle,
    ) -> Result<(), CanvasError> {
        let mut line = String::<24>::new();
        if let Some(metrics) = metrics {
            let _ = write!(
                line,
                "{label} {} C{} S{}",
                metrics.total_us / 1_000,
                metrics.compose_us / 1_000,
                metrics.transfer_us / 1_000,
            );
        } else {
            let _ = write!(line, "{label} ---");
        }
        draw_mono_text_visible(&line, Point::new(0, baseline), style, canvas)
    }

    fn draw_max_line(
        canvas: &mut Canvas<'_>,
        forward: Option<RenderMetrics>,
        backward: Option<RenderMetrics>,
        baseline: i32,
        style: AaTextStyle,
    ) -> Result<(), CanvasError> {
        let mut line = String::<24>::new();
        let forward_max = forward.map_or(0, |metrics| metrics.max_stripe_us / 1_000);
        let backward_max = backward.map_or(0, |metrics| metrics.max_stripe_us / 1_000);
        let _ = write!(line, "MAX F{forward_max} B{backward_max}");
        draw_mono_text_visible(&line, Point::new(0, baseline), style, canvas)
    }
}

impl Paint for TestScreen {
    fn draw_full(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        Rectangle::new(Point::new(0, 0), Size::new(240, 64))
            .into_styled(PrimitiveStyle::with_fill(Rgb565::BLUE))
            .draw(canvas)?;
        keep_alive();

        let heading = ui_text(Rgb565::WHITE, Rgb565::BLACK);
        draw_visible(
            &Text::with_alignment(
                "PineForge Touch-Test",
                Point::new(120, 38),
                heading,
                Alignment::Center,
            ),
            canvas,
        )?;
        self.draw_stack(canvas)?;
        keep_alive();

        TOUCH_AREA
            .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
            .draw(canvas)?;
        if let Some(point) = self.last_touch.filter(|point| TOUCH_AREA.contains(*point)) {
            Rectangle::new(Point::new(point.x - 6, point.y - 6), TOUCH_MARKER_SIZE)
                .into_styled(PrimitiveStyle::with_fill(Rgb565::YELLOW))
                .draw(canvas)?;
        }
        keep_alive();

        self.draw_metrics(canvas)?;
        keep_alive();

        FOOTER_AREA
            .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
            .draw(canvas)?;
        let hint = hint_text(Rgb565::WHITE, Rgb565::BLACK);
        draw_visible(
            &Text::with_alignment(
                "< Swipe right: Zurueck",
                Point::new(120, 232),
                hint,
                Alignment::Center,
            ),
            canvas,
        )?;
        self.draw_touch_state(canvas)?;
        keep_alive();

        Ok(())
    }
}

impl Screen for TestScreen {
    fn handle_event(&mut self, event: AppEvent) -> ScreenAction {
        let AppEvent::Touch { x, y, pressed } = event else {
            return ScreenAction::None;
        };
        let point = Point::new(x, y);
        self.previous_touch = self.last_touch;
        self.last_touch = Some(point);
        self.previous_touching = self.touching;
        self.touching = pressed;
        ScreenAction::None
    }

    fn draw_dirty(
        &self,
        canvas: &mut Canvas<'_>,
        keep_alive: &mut dyn FnMut(),
    ) -> Result<(), CanvasError> {
        if let Some(point) = self
            .previous_touch
            .filter(|point| TOUCH_AREA.contains(*point))
        {
            Rectangle::new(Point::new(point.x - 6, point.y - 6), TOUCH_MARKER_SIZE)
                .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
                .draw(canvas)?;
            keep_alive();
        }

        if let Some(point) = self.last_touch.filter(|point| TOUCH_AREA.contains(*point)) {
            Rectangle::new(Point::new(point.x - 6, point.y - 6), TOUCH_MARKER_SIZE)
                .into_styled(PrimitiveStyle::with_fill(Rgb565::YELLOW))
                .draw(canvas)?;
            keep_alive();
        }

        if self.touching != self.previous_touching {
            self.draw_touch_state(canvas)?;
            keep_alive();
        }

        // Only when the mark actually moved. It grows a handful of times in a
        // session and then stands still, so a redraw per tick would be one
        // SPI transaction a second for a line that has not changed.
        if self.stack != self.previous_stack {
            self.draw_stack(canvas)?;
            keep_alive();
        }

        Ok(())
    }
}
