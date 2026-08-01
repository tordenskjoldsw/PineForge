//! A [`Surface`] that records what was drawn instead of showing it.
//!
//! This is the whole point of the crate boundary. A screen paints into a
//! `Canvas`, and a `Canvas` borrows any `Surface` - so on the host a screen can
//! be drawn into a recording buffer and asked what it did, without a panel, an
//! SPI bus, or a watch.
//!
//! It records three things, because the questions worth asking are of three
//! kinds. Coverage answers "did every pixel get painted", which is the
//! opaque-drawing contract every screen owes the transition. The list of areas
//! answers "where did it paint", which is what a layout assertion needs. And the
//! colour left on each pixel answers "did it paint *this*", without which a
//! symbol drawn over a background it matches in extent cannot be told from the
//! background alone.

use embedded_graphics::{
    Pixel,
    geometry::{Point, Size},
    pixelcolor::Rgb565,
    primitives::Rectangle,
};

use crate::canvas::{CanvasError, Surface};

/// The panel this firmware draws on.
pub const PANEL: Size = Size::new(240, 240);

pub struct Probe {
    /// The colour last written to each pixel, or `None` where nothing wrote.
    covered: [Option<Rgb565>; (PANEL.width * PANEL.height) as usize],
    /// Every area handed to the surface, in order and unclamped, so a test can
    /// catch one that reached past the panel rather than silently clipping it.
    areas: std::vec::Vec<Rectangle>,
    /// What this surface says it accepts. The panel unless a test narrows it to
    /// stand in for a transition stripe.
    area: Rectangle,
}

impl Default for Probe {
    fn default() -> Self {
        Self::new()
    }
}

impl Probe {
    #[must_use]
    pub fn new() -> Self {
        Self::clipped_to(Rectangle::new(Point::zero(), PANEL))
    }

    /// A probe that reports `area` as all it accepts, the way a stripe of the
    /// transition scratch does.
    ///
    /// It still records whatever it is handed, including outside that area -
    /// which is the point. A renderer that respects the clip can be told from
    /// one that leaves the clipping to the target, and only the first saves the
    /// work of rasterising what nobody keeps.
    #[must_use]
    pub fn clipped_to(area: Rectangle) -> Self {
        Self {
            covered: [None; (PANEL.width * PANEL.height) as usize],
            areas: std::vec::Vec::new(),
            area,
        }
    }

    /// Pixels of the panel nothing painted.
    ///
    /// A screen's `draw_full` owes the transition an opaque surface: the stripe
    /// renderer never clears behind it, so anything left unpainted shows the
    /// previous screen through it.
    #[must_use]
    pub fn unpainted(&self) -> usize {
        self.covered.iter().filter(|pixel| pixel.is_none()).count()
    }

    /// The areas that reached outside what this surface accepts, if any.
    #[must_use]
    pub fn out_of_bounds(&self) -> std::vec::Vec<Rectangle> {
        self.areas
            .iter()
            .filter(|area| {
                area.size != Size::zero() && area.intersection(&self.area).size != area.size
            })
            .copied()
            .collect()
    }

    /// The colour left on one pixel, or `None` if nothing wrote it.
    #[must_use]
    pub fn pixel(&self, point: Point) -> Option<Rgb565> {
        let panel = Rectangle::new(Point::zero(), PANEL);
        if panel.contains(point) {
            self.covered[index(point.x, point.y)]
        } else {
            None
        }
    }

    /// Whether anything at all was painted inside `area`.
    #[must_use]
    pub fn painted_within(&self, area: Rectangle) -> bool {
        self.rows(area)
            .any(|(x, y)| self.covered[index(x, y)].is_some())
    }

    /// Whether anything inside `area` was left in a colour other than `color`.
    ///
    /// The question `painted_in` cannot answer for antialiased text: a glyph's
    /// pixels are mostly blends between ink and background, and the ink colour
    /// itself may appear at no pixel at all in a small face. Asking what is
    /// *not* background is what tells you something was drawn there.
    #[must_use]
    pub fn painted_other_than(&self, area: Rectangle, color: Rgb565) -> bool {
        self.rows(area)
            .any(|(x, y)| self.covered[index(x, y)].is_some_and(|painted| painted != color))
    }

    /// Whether `color` was left anywhere inside `area`.
    ///
    /// What coverage cannot answer: a symbol drawn over a background that
    /// already filled the same pixels is invisible to `painted_within`, because
    /// both painted. Asking for the colour distinguishes the two.
    #[must_use]
    pub fn painted_in(&self, area: Rectangle, color: Rgb565) -> bool {
        self.rows(area)
            .any(|(x, y)| self.covered[index(x, y)] == Some(color))
    }

    /// Every panel pixel of `area`, clamped to the panel.
    fn rows(&self, area: Rectangle) -> impl Iterator<Item = (i32, i32)> + '_ {
        let panel = Rectangle::new(Point::zero(), PANEL);
        let clipped = area.intersection(&panel);
        let left = clipped.top_left.x;
        let top = clipped.top_left.y;
        let width = clipped.size.width as i32;
        let height = clipped.size.height as i32;
        (top..top + height).flat_map(move |y| (left..left + width).map(move |x| (x, y)))
    }

    fn mark(&mut self, area: &Rectangle, color: Rgb565) {
        self.areas.push(*area);
        for (x, y) in self.rows(*area).collect::<std::vec::Vec<_>>() {
            self.covered[index(x, y)] = Some(color);
        }
    }
}

fn index(x: i32, y: i32) -> usize {
    (y as usize) * PANEL.width as usize + (x as usize)
}

impl Surface for Probe {
    fn area(&self) -> Rectangle {
        self.area
    }

    fn fill_solid(&mut self, area: &Rectangle, color: Rgb565) -> Result<(), CanvasError> {
        self.mark(area, color);
        Ok(())
    }

    fn fill_contiguous(
        &mut self,
        area: &Rectangle,
        colors: &mut dyn Iterator<Item = Rgb565>,
    ) -> Result<(), CanvasError> {
        // Consumed pixel by pixel rather than counted: the iterator is the
        // caller's rasteriser, so a test that never runs it would not exercise
        // the drawing code it is meant to check - and the colour each glyph
        // pixel resolves to is exactly what makes it distinguishable from the
        // background it sits on.
        self.areas.push(*area);
        let points = self.rows(*area).collect::<std::vec::Vec<_>>();
        let mut points = points.into_iter();
        for color in colors {
            let Some((x, y)) = points.next() else {
                break;
            };
            self.covered[index(x, y)] = Some(color);
        }
        Ok(())
    }

    fn draw_pixels(
        &mut self,
        pixels: &mut dyn Iterator<Item = Pixel<Rgb565>>,
    ) -> Result<(), CanvasError> {
        for Pixel(point, color) in pixels {
            self.mark(&Rectangle::new(point, Size::new(1, 1)), color);
        }
        Ok(())
    }
}
