//! Where a paint's time actually goes, measured rather than argued about.
//!
//! A transition was timed at 241 ms, of which the diagnostics build attributes
//! 176 ms to "transfer" and 64 ms to composing. But the transfer window wraps
//! three different things at once: pulling colours out of an iterator, packing
//! them into bytes, and moving those bytes over SPI. The 8 MHz bus can carry a
//! full frame in 115.2 ms, so 61 ms of that window belongs to something else -
//! and every remaining optimisation worth doing depends on which something.
//!
//! If the bus alone is near its floor, the 61 ms is CPU and the answer is a
//! faster pixel path. If the bus alone is nearer 176 ms, the CPU is not the
//! problem and the answer is a driver that chains `EasyDMA` chunks without the
//! core in between. The two conclusions point at completely different work, so
//! this measures them apart before either is started.
//!
//! Reported twice over: to RTT for whoever has a debugger, and back to the
//! caller, which puts it on the diagnostics screen for whoever does not.

use defmt::info;
use embassy_time::Instant;
use embedded_graphics::{
    pixelcolor::{Rgb565, raw::RawU16},
    prelude::RawData,
};
use embedded_hal::spi::SpiDevice as _;
use pineforge_ui::metrics::BusBench;
use static_cell::StaticCell;

use crate::board::buses::DisplaySpi;

/// One full 240x240 RGB565 frame, in bytes and in pixels.
const FRAME_BYTES: usize = 240 * 240 * 2;
const FRAME_PIXELS: usize = 240 * 240;

/// What the bus owes for a frame if nothing else cost anything: 115,200 bytes
/// of 8 bits at 8 Mbit/s.
const FLOOR_US: u64 = 115_200;

/// The write sizes the sweep walks, in bytes.
///
/// `EasyDMA` on the nRF52832 caps a transfer at 255 bytes, so embassy splits
/// every call into that many chunks and spins on `events_end` between them.
/// Each size below is a whole number of chunks - 1, 2, 4, 8, 16 - which is what
/// separates the two costs that could be there. A time that scales with the
/// *call* count means the per-call overhead dominates, and the production
/// buffer should grow again. A time that barely moves across the whole sweep
/// means the per-chunk restart dominates, and no buffer size fixes it.
///
/// 2,040 is what the firmware runs today; 4,080 is one step past it, and is the
/// only reason this module carries a buffer of its own.
const CALL_SIZES: [usize; 5] = [255, 510, 1020, 2040, 4080];

const BENCH_BUFFER_BYTES: usize = 4080;
static BENCH_BUFFER: StaticCell<[u8; BENCH_BUFFER_BYTES]> = StaticCell::new();

/// Runs both halves of the measurement and logs them.
///
/// Called with the panel held in reset, so the ST7789 sees none of this and no
/// picture depends on what is sent. That also keeps the numbers honest: what is
/// timed is the bus and the core, with no display-side behaviour mixed in.
pub fn run(spi: &mut DisplaySpi, keep_alive: &mut dyn FnMut()) -> BusBench {
    let buffer = BENCH_BUFFER.init([0; BENCH_BUFFER_BYTES]);
    // A real colour rather than zeroes. Nothing about SPI timing depends on the
    // bits, but a buffer full of one recognisable value is worth more than a
    // buffer full of nothing if this is ever pointed at a live panel.
    for pixel in buffer.chunks_exact_mut(2) {
        pixel.copy_from_slice(&RawU16::from(Rgb565::new(0, 63, 0)).into_inner().to_be_bytes());
    }

    info!(
        "bench: a frame is {=usize} bytes, and the 8 MHz floor for one is {=u64} us",
        FRAME_BYTES, FLOOR_US
    );
    let bus_us = bus_sweep(spi, buffer, keep_alive);
    let (pack_concrete_us, pack_dyn_us) = pack_sweep(buffer, keep_alive);
    // The glyph half is filled in by the caller: it composes into the
    // transition scratch, which belongs to the display task and not here.
    BusBench {
        bus_us,
        pack_concrete_us,
        pack_dyn_us,
        ..BusBench::default()
    }
}

/// Times a frame's worth of bytes onto the bus, at one write size per pass.
fn bus_sweep(spi: &mut DisplaySpi, buffer: &[u8], keep_alive: &mut dyn FnMut()) -> [u64; 5] {
    let mut timings = [0; CALL_SIZES.len()];
    for (size, timing) in CALL_SIZES.into_iter().zip(&mut timings) {
        let calls = FRAME_BYTES.div_ceil(size);
        let started = Instant::now();
        for _ in 0..calls {
            // Errors are not interesting here and nothing downstream could act
            // on one: the bus either carried the bytes or the watch has no
            // display, and the second case is visible without a benchmark.
            let _ = spi.write(&buffer[..size]);
        }
        *timing = started.elapsed().as_micros();
        info!(
            "bench bus: {=usize} bytes x {=usize} calls = {=usize} bytes in {=u64} us",
            size,
            calls,
            calls * size,
            *timing
        );
        keep_alive();
    }
    timings
}

/// Times packing a frame's worth of colours into bytes, with no bus involved.
///
/// Twice, over the same colours, and the difference between the two runs is the
/// whole point. `Canvas` hands its backend a `&mut dyn Iterator`, so every one
/// of a frame's 57,600 pixels crosses a vtable on its way into the buffer and
/// nothing about the source iterator can be inlined into the packing loop. The
/// transition path avoids that by keeping the display concrete. If the gap
/// below is large, a row-oriented surface method removes it from every screen;
/// if it is small, that work is not worth doing.
fn pack_sweep(buffer: &mut [u8], keep_alive: &mut dyn FnMut()) -> (u64, u64) {
    let started = Instant::now();
    let packed = pack(buffer, colors());
    let concrete = started.elapsed().as_micros();
    info!(
        "bench pack: {=u32} bytes through a concrete iterator in {=u64} us",
        packed, concrete
    );
    keep_alive();

    let mut source = colors();
    let started = Instant::now();
    let packed = pack(buffer, &mut source as &mut dyn Iterator<Item = Rgb565>);
    let dynamic = started.elapsed().as_micros();
    info!(
        "bench pack: {=u32} bytes through a dyn iterator in {=u64} us, {=u64} us of vtable",
        packed,
        dynamic,
        dynamic.saturating_sub(concrete)
    );
    keep_alive();
    (concrete, dynamic)
}

/// A frame's worth of colours that cannot be folded into a constant.
fn colors() -> impl Iterator<Item = Rgb565> {
    (0..FRAME_PIXELS).map(|index| {
        let index = u8::try_from(index % 32).unwrap_or(0);
        Rgb565::new(index, index * 2, index)
    })
}

/// Packs colours into the buffer the way the display interface does.
///
/// Generic on purpose: called once with a concrete iterator and once with a
/// `&mut dyn Iterator`, it is the same loop both times and the only difference
/// is how `next` is reached. `black_box` keeps the whole thing from being
/// optimised away, since nothing here looks at the bytes afterwards.
fn pack<I>(buffer: &mut [u8], mut colors: I) -> u32
where
    I: Iterator<Item = Rgb565>,
{
    let mut packed = 0_u32;
    loop {
        let mut filled = 0_usize;
        for pixel in buffer.chunks_exact_mut(2) {
            let Some(color) = colors.next() else { break };
            pixel.copy_from_slice(&RawU16::from(color).into_inner().to_be_bytes());
            filled += 2;
        }
        core::hint::black_box(&buffer);
        packed = packed.saturating_add(u32::try_from(filled).unwrap_or(0));
        if filled < buffer.len() {
            return packed;
        }
    }
}
