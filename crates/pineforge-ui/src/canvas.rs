//! The one drawing surface the whole UI is compiled against.
//!
//! Screens used to be generic over `DrawTarget`. That meant every screen was
//! compiled twice - once for the display, once for the transition scratch
//! buffer - and every rectangle, glyph run, and icon blit existed in two
//! copies. The duplication grew with each screen added, which is the wrong
//! shape for a UI that is meant to gain screens.
//!
//! [`Canvas`] breaks it. It is a single concrete `DrawTarget` that forwards to
//! a backend behind a vtable, so the rasterising code embedded-graphics
//! generates for a screen exists exactly once however many backends there are.
//! A screen names `Canvas` where it used to name a type parameter; nothing
//! else about how it draws changes.
//!
//! The cost is indirection: one virtual call per filled area, and one per pixel
//! on the iterator-based paths. Both are far below the SPI transfer that
//! follows them, and the transition's own stripe blit deliberately stays on the
//! concrete display type so the hot path never pays it at all.

use embedded_graphics::{
    Pixel, draw_target::DrawTarget, geometry::Dimensions, pixelcolor::Rgb565, primitives::Rectangle,
};

/// What a drawing operation reports when the backend could not accept it.
///
/// The concrete cause - an SPI transfer that failed, say - is deliberately not
/// carried: nothing above this layer can act on it, and every call site the
/// firmware has already discards it. Erasing it here is what lets one canvas
/// stand in for backends whose error types differ, including the infallible
/// scratch buffer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CanvasError;

/// A backend a [`Canvas`] paints into.
///
/// Kept object-safe on purpose: `DrawTarget` is not, because of its associated
/// error type and its generic methods, so it cannot be used behind a reference
/// directly. This trait is the narrow, erased subset that can.
///
/// It is implemented for every `DrawTarget` over [`Rgb565`], so a backend only
/// has to be a normal embedded-graphics target to be usable as one.
pub trait Surface {
    /// The region this surface accepts drawing in. Anything outside is clipped.
    fn area(&self) -> Rectangle;

    fn fill_solid(&mut self, area: &Rectangle, color: Rgb565) -> Result<(), CanvasError>;

    fn fill_contiguous(
        &mut self,
        area: &Rectangle,
        colors: &mut dyn Iterator<Item = Rgb565>,
    ) -> Result<(), CanvasError>;

    fn draw_pixels(
        &mut self,
        pixels: &mut dyn Iterator<Item = Pixel<Rgb565>>,
    ) -> Result<(), CanvasError>;
}

impl<D> Surface for D
where
    D: DrawTarget<Color = Rgb565>,
{
    fn area(&self) -> Rectangle {
        self.bounding_box()
    }

    fn fill_solid(&mut self, area: &Rectangle, color: Rgb565) -> Result<(), CanvasError> {
        DrawTarget::fill_solid(self, area, color).map_err(|_| CanvasError)
    }

    fn fill_contiguous(
        &mut self,
        area: &Rectangle,
        colors: &mut dyn Iterator<Item = Rgb565>,
    ) -> Result<(), CanvasError> {
        DrawTarget::fill_contiguous(self, area, colors).map_err(|_| CanvasError)
    }

    fn draw_pixels(
        &mut self,
        pixels: &mut dyn Iterator<Item = Pixel<Rgb565>>,
    ) -> Result<(), CanvasError> {
        self.draw_iter(pixels).map_err(|_| CanvasError)
    }
}

/// The drawing surface every screen renders through.
///
/// Borrowing rather than owning its backend keeps the large buffers where they
/// belong - the display task owns both the panel and the scratch - and lets the
/// same screen be composed into either without knowing which it is.
pub struct Canvas<'a> {
    surface: &'a mut dyn Surface,
}

impl<'a> Canvas<'a> {
    /// Wraps a backend for the duration of one drawing pass.
    #[must_use]
    pub fn new(surface: &'a mut dyn Surface) -> Self {
        Self { surface }
    }
}

impl Dimensions for Canvas<'_> {
    fn bounding_box(&self) -> Rectangle {
        self.surface.area()
    }
}

impl DrawTarget for Canvas<'_> {
    type Color = Rgb565;
    type Error = CanvasError;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        self.surface.draw_pixels(&mut pixels.into_iter())
    }

    fn fill_contiguous<I>(&mut self, area: &Rectangle, colors: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Self::Color>,
    {
        self.surface.fill_contiguous(area, &mut colors.into_iter())
    }

    fn fill_solid(&mut self, area: &Rectangle, color: Self::Color) -> Result<(), Self::Error> {
        self.surface.fill_solid(area, color)
    }
}
