#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderMetrics {
    pub total_us: u64,
    pub compose_us: u64,
    pub transfer_us: u64,
    pub max_stripe_us: u64,
    pub stripe_count: u8,
}

/// How much of a paint is the bus and how much is the core, taken at boot.
///
/// A transition reports one "transfer" figure that covers pulling colours from
/// an iterator, packing them into bytes and moving those bytes over SPI. The
/// 8 MHz bus owes 115.2 ms for a frame, so the excess over that belongs to
/// something the figure cannot name. These are the parts, measured apart.
///
/// Taken once, before the panel is up, and never touched again - which is why
/// this screen can show them safely. Nothing that happens while they are on the
/// panel can make them stale.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BusBench {
    /// A frame's worth of bytes onto the bus, one entry per write size, in
    /// ascending order from 255 bytes doubling to 4,080. Microseconds.
    pub bus_us: [u64; 5],
    /// A frame's worth of colours packed into bytes with no bus involved,
    /// through a concrete iterator and through a `dyn` one. The difference is
    /// what a vtable per pixel costs. Microseconds.
    pub pack_concrete_us: u64,
    pub pack_dyn_us: u64,
    /// The same question asked of the path that actually pays it: a screen's
    /// worth of glyph runs into the scratch, direct and through [`Canvas`].
    /// Microseconds.
    ///
    /// [`Canvas`]: crate::canvas::Canvas
    pub glyph_concrete_us: u64,
    pub glyph_dyn_us: u64,
}

/// What a screen's worth of glyph work costs, with the vtable and without it.
///
/// The pack figures above give the vtable's price per pixel, but a screen does
/// not send a frame of pixels through `fill_contiguous` - most of it is
/// rectangles, and a filled rectangle crosses the vtable once however large it
/// is. Only text goes through per pixel. So the pack numbers are a ceiling
/// nothing reaches, and this is the number that decides whether a row-oriented
/// surface method is worth building: it draws real runs in the real face into
/// the real stripe, exactly as composing a transition does.
///
/// Twenty passes of a twenty-three character line, which is about what a
/// text-heavy screen rasterises while a transition composes it. `black_box`
/// keeps a pass from being folded into its neighbour now that this crate builds
/// at `opt-level = 3`; without it the concrete loop is provably idempotent and
/// the compiler is free to run it once.
#[cfg(feature = "ui-animations")]
#[must_use]
pub fn glyph_bench(scratch: &mut crate::scratch::UiScratch) -> (u64, u64) {
    use embassy_time::Instant;
    use embedded_graphics::{
        geometry::{Point, Size},
        primitives::Rectangle,
        text::{Baseline, renderer::TextRenderer},
    };

    use crate::{canvas::Canvas, font::ui_text, scratch::STRIPE_THICKNESS, theme};

    const RUN: &str = "EINSTELLUNGEN 12:34 87%";
    const PASSES: u32 = 20;

    let style = ui_text(theme::TEXT, theme::SURFACE);
    scratch.prepare_stripe(Rectangle::new(
        Point::zero(),
        Size::new(240, STRIPE_THICKNESS),
    ));

    let started = Instant::now();
    for _ in 0..PASSES {
        let _ = style.draw_string(RUN, Point::zero(), Baseline::Top, &mut *scratch);
        core::hint::black_box(&mut *scratch);
    }
    let concrete = started.elapsed().as_micros();

    let started = Instant::now();
    for _ in 0..PASSES {
        let _ = style.draw_string(
            RUN,
            Point::zero(),
            Baseline::Top,
            &mut Canvas::new(&mut *scratch),
        );
        core::hint::black_box(&mut *scratch);
    }
    let dynamic = started.elapsed().as_micros();

    (concrete, dynamic)
}
