use core::fmt::Write;

use embedded_graphics::{
    mono_font::{MonoTextStyle, ascii::FONT_6X10, ascii::FONT_10X20},
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::{Alignment, Text},
};
use heapless::String;

use crate::ui::{
    metrics::RenderMetrics,
    render::{draw_mono_text_visible, draw_visible},
    screen::Screen,
};
#[cfg(feature = "ui-animations")]
use pineforge_state::NavigationDirection;
use pineforge_state::{AppEvent, ScreenAction};

const TOUCH_AREA: Rectangle = Rectangle::new(Point::new(0, 64), Size::new(240, 76));
const METRICS_AREA: Rectangle = Rectangle::new(Point::new(0, 140), Size::new(240, 74));
const FOOTER_AREA: Rectangle = Rectangle::new(Point::new(0, 214), Size::new(240, 26));
const TOUCH_MARKER_SIZE: Size = Size::new(13, 13);

#[derive(Default)]
pub struct TestScreen {
    last_touch: Option<Point>,
    previous_touch: Option<Point>,
    touching: bool,
    previous_touching: bool,
    forward_metrics: Option<RenderMetrics>,
    backward_metrics: Option<RenderMetrics>,
}

impl TestScreen {
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

    pub fn draw_metrics<D>(&self, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        METRICS_AREA
            .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
            .draw(display)?;
        let style = MonoTextStyle::new(&FONT_6X10, Rgb565::WHITE);
        Self::draw_metric_line(display, "F", self.forward_metrics, 154, style)?;
        Self::draw_metric_line(display, "B", self.backward_metrics, 170, style)?;
        Self::draw_max_line(
            display,
            self.forward_metrics,
            self.backward_metrics,
            190,
            style,
        )?;
        let tiles = self
            .forward_metrics
            .or(self.backward_metrics)
            .map_or(0, |metrics| metrics.stripe_count);
        let mut tile_line = String::<24>::new();
        let _ = write!(tile_line, "TILES {tiles}");
        draw_mono_text_visible(&tile_line, Point::new(0, 206), style, display)?;
        Ok(())
    }

    /// Touch contact state, relocated here from the watchface status row.
    fn draw_touch_state<D>(&self, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        Rectangle::new(Point::new(0, 214), Size::new(50, 26))
            .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
            .draw(display)?;
        let style = MonoTextStyle::new(
            &FONT_6X10,
            if self.touching {
                Rgb565::YELLOW
            } else {
                Rgb565::WHITE
            },
        );
        let state = if self.touching { "AKTIV" } else { "BEREIT" };
        draw_mono_text_visible(state, Point::new(2, 232), style, display)
    }

    fn draw_metric_line<D>(
        display: &mut D,
        label: &str,
        metrics: Option<RenderMetrics>,
        baseline: i32,
        style: MonoTextStyle<'_, Rgb565>,
    ) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
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
        draw_mono_text_visible(&line, Point::new(0, baseline), style, display)
    }

    fn draw_max_line<D>(
        display: &mut D,
        forward: Option<RenderMetrics>,
        backward: Option<RenderMetrics>,
        baseline: i32,
        style: MonoTextStyle<'_, Rgb565>,
    ) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        let mut line = String::<24>::new();
        let forward_max = forward.map_or(0, |metrics| metrics.max_stripe_us / 1_000);
        let backward_max = backward.map_or(0, |metrics| metrics.max_stripe_us / 1_000);
        let _ = write!(line, "MAX F{forward_max} B{backward_max}");
        draw_mono_text_visible(&line, Point::new(0, baseline), style, display)
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

    fn draw_full<D>(&self, display: &mut D, mut keep_alive: impl FnMut()) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        Rectangle::new(Point::new(0, 0), Size::new(240, 64))
            .into_styled(PrimitiveStyle::with_fill(Rgb565::BLUE))
            .draw(display)?;
        keep_alive();

        let heading = MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE);
        draw_visible(
            &Text::with_alignment(
                "PineForge Touch-Test",
                Point::new(120, 38),
                heading,
                Alignment::Center,
            ),
            display,
        )?;
        keep_alive();

        TOUCH_AREA
            .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
            .draw(display)?;
        if let Some(point) = self.last_touch.filter(|point| TOUCH_AREA.contains(*point)) {
            Rectangle::new(Point::new(point.x - 6, point.y - 6), TOUCH_MARKER_SIZE)
                .into_styled(PrimitiveStyle::with_fill(Rgb565::YELLOW))
                .draw(display)?;
        }
        keep_alive();

        self.draw_metrics(display)?;
        keep_alive();

        FOOTER_AREA
            .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
            .draw(display)?;
        let hint = MonoTextStyle::new(&FONT_6X10, Rgb565::WHITE);
        draw_visible(
            &Text::with_alignment(
                "< Swipe right: Zurueck",
                Point::new(120, 232),
                hint,
                Alignment::Center,
            ),
            display,
        )?;
        self.draw_touch_state(display)?;
        keep_alive();

        Ok(())
    }

    fn draw_dirty<D>(&self, display: &mut D, mut keep_alive: impl FnMut()) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        if let Some(point) = self
            .previous_touch
            .filter(|point| TOUCH_AREA.contains(*point))
        {
            Rectangle::new(Point::new(point.x - 6, point.y - 6), TOUCH_MARKER_SIZE)
                .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
                .draw(display)?;
            keep_alive();
        }

        if let Some(point) = self.last_touch.filter(|point| TOUCH_AREA.contains(*point)) {
            Rectangle::new(Point::new(point.x - 6, point.y - 6), TOUCH_MARKER_SIZE)
                .into_styled(PrimitiveStyle::with_fill(Rgb565::YELLOW))
                .draw(display)?;
            keep_alive();
        }

        if self.touching != self.previous_touching {
            self.draw_touch_state(display)?;
            keep_alive();
        }

        Ok(())
    }
}
