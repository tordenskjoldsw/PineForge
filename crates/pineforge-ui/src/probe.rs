//! A [`Surface`] that records what was drawn instead of showing it.
//!
//! This is the whole point of the crate boundary. A screen paints into a
//! `Canvas`, and a `Canvas` borrows any `Surface` - so on the host a screen can
//! be drawn into a recording buffer and asked what it did, without a panel, an
//! SPI bus, or a watch.
//!
//! It records two things, because the questions worth asking are of two kinds.
//! Coverage answers "did every pixel get painted", which is the opaque-drawing
//! contract every screen owes the transition. The list of areas answers "where
//! did it paint", which is what a layout assertion needs.

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
    covered: [bool; (PANEL.width * PANEL.height) as usize],
    /// Every area handed to the surface, in order and unclamped, so a test can
    /// catch one that reached past the panel rather than silently clipping it.
    areas: std::vec::Vec<Rectangle>,
}

impl Default for Probe {
    fn default() -> Self {
        Self::new()
    }
}

impl Probe {
    #[must_use]
    pub fn new() -> Self {
        Self {
            covered: [false; (PANEL.width * PANEL.height) as usize],
            areas: std::vec::Vec::new(),
        }
    }

    /// Pixels of the panel nothing painted.
    ///
    /// A screen's `draw_full` owes the transition an opaque surface: the stripe
    /// renderer never clears behind it, so anything left unpainted shows the
    /// previous screen through it.
    #[must_use]
    pub fn unpainted(&self) -> usize {
        self.covered.iter().filter(|painted| !**painted).count()
    }

    /// The areas that reached outside the panel, if any.
    #[must_use]
    pub fn out_of_bounds(&self) -> std::vec::Vec<Rectangle> {
        let panel = Rectangle::new(Point::zero(), PANEL);
        self.areas
            .iter()
            .filter(|area| area.size != Size::zero() && area.intersection(&panel).size != area.size)
            .copied()
            .collect()
    }

    /// Whether anything at all was painted inside `area`.
    #[must_use]
    pub fn painted_within(&self, area: Rectangle) -> bool {
        self.rows(area).any(|(x, y)| self.covered[index(x, y)])
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

    fn mark(&mut self, area: &Rectangle) {
        self.areas.push(*area);
        for (x, y) in self.rows(*area).collect::<std::vec::Vec<_>>() {
            self.covered[index(x, y)] = true;
        }
    }
}

fn index(x: i32, y: i32) -> usize {
    (y as usize) * PANEL.width as usize + (x as usize)
}

impl Surface for Probe {
    fn area(&self) -> Rectangle {
        Rectangle::new(Point::zero(), PANEL)
    }

    fn fill_solid(&mut self, area: &Rectangle, _color: Rgb565) -> Result<(), CanvasError> {
        self.mark(area);
        Ok(())
    }

    fn fill_contiguous(
        &mut self,
        area: &Rectangle,
        colors: &mut dyn Iterator<Item = Rgb565>,
    ) -> Result<(), CanvasError> {
        // Drained rather than ignored: the iterator is the caller's rasteriser,
        // and a test that never runs it would not be exercising the drawing
        // code it is meant to check.
        let _ = colors.count();
        self.mark(area);
        Ok(())
    }

    fn draw_pixels(
        &mut self,
        pixels: &mut dyn Iterator<Item = Pixel<Rgb565>>,
    ) -> Result<(), CanvasError> {
        for Pixel(point, _) in pixels {
            self.mark(&Rectangle::new(point, Size::new(1, 1)));
        }
        Ok(())
    }
}
