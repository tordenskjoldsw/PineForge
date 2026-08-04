//! How deep the stack has actually been.
//!
//! The static side of RAM is exact: the linker vends every byte of `.data`,
//! `.bss` and `.uninit`, and CI holds it to a budget. The stack is the
//! opposite. It grows and shrinks with call depth, and the deepest it ever goes
//! depends on which paths run together - pairing doing elliptic-curve
//! arithmetic while a radio interrupt nests on top of whatever was drawing.
//!
//! So the space left for the stack was a guess, and the budget that reserves it
//! was a guess about a guess. This measures it instead: the region is filled
//! with a pattern before `main`, and whatever still holds that pattern was never
//! written to. The lowest surviving word is the high-water mark.
//!
//! It is a measurement, not a guard. `flip-link` is the guard - see
//! `.cargo/config.toml` for why the stack sits below the statics.
//!
//! # Reading it for the pairing peak
//!
//! The deepest path this firmware has is pairing, and it is easy to measure
//! everything except that. The bond lives in external flash and survives a DFU,
//! so flashing a diagnostics build and reconnecting runs no key exchange at
//! all. The mark that comes back is an ordinary connection's, several kilobytes
//! short, and trusting one such reading broke pairing on a shipped image.
//!
//! So: make the phone forget the device first. Only a connection with no bond to
//! resume performs the elliptic-curve arithmetic this number exists to bound.

use core::ptr;

/// Written into every unused stack word. Any value would do; this one is
/// recognisable in a memory dump, which is where to look next if the reported
/// number ever seems wrong.
const PAINT: u32 = 0xDEAD_BEEF;

/// Words left unpainted below the stack pointer, so painting cannot overwrite
/// the frame doing the painting. `pre_init` runs a handful of frames deep and
/// this is far more than it uses.
const HEADROOM_WORDS: usize = 64;

// Declared as `u32` rather than as a byte: these are addresses, and typing them
// as the word they are read in states the alignment the linker already gives
// them instead of casting it in later.
#[allow(unsafe_code)]
unsafe extern "C" {
    /// Lowest address the stack may reach. With `flip-link` this is the start
    /// of RAM, so overflowing past it leaves mapped memory entirely.
    static _stack_end: u32;
    /// The stack pointer's value at reset; the stack grows down from here.
    static _stack_start: u32;
}

#[allow(unsafe_code)]
fn bottom() -> *mut u32 {
    (&raw const _stack_end).cast_mut()
}

#[allow(unsafe_code)]
fn top() -> *mut u32 {
    (&raw const _stack_start).cast_mut()
}

/// Bytes between the two stack bounds - every byte the stack may use.
#[must_use]
pub fn capacity() -> usize {
    top() as usize - bottom() as usize
}

/// Fills the unused stack with [`PAINT`], before anything else runs.
///
/// `pre_init` is the only place this works: it runs before `.bss` is zeroed and
/// `.data` is copied, so nothing has state to lose yet, and the stack is at its
/// shallowest so almost all of it can be painted.
#[allow(unsafe_code)]
#[cortex_m_rt::pre_init]
unsafe fn paint_stack() {
    let bottom = bottom();
    // The address of a local is a fair reading of where the stack currently is,
    // and it needs no register access from inside a naked-ish context.
    let here = (&raw const bottom) as usize;
    let ceiling = here.saturating_sub(HEADROOM_WORDS * size_of::<u32>());

    let mut word = bottom;
    while (word as usize) < ceiling {
        // SAFETY: `word` walks from the linker-provided low bound of the stack
        // up to below the current frame, so it stays inside the stack region
        // and never reaches the frame this loop is running in.
        unsafe {
            ptr::write_volatile(word, PAINT);
            word = word.add(1);
        }
    }
}

/// The deepest the stack has been since reset, in bytes.
///
/// Counts the painted words still standing at the bottom and reports the rest.
/// Reads low to high and stops at the first disturbed word: everything above it
/// has been used at some point, whether or not it is in use now.
///
/// Pessimistic by construction, which is the right direction. Two reasons the
/// figure can exceed the truth and none that it can fall short of it: the
/// topmost [`HEADROOM_WORDS`] were never painted, so they always count as used,
/// and a genuine value that happens to equal `PAINT` reads as untouched only
/// below the mark, never above it.
#[allow(unsafe_code)]
#[must_use]
pub fn used() -> usize {
    let bottom = bottom();
    let top = top();
    let mut word = bottom;
    while (word as usize) < top as usize {
        // SAFETY: bounded by the linker-provided stack region, and the stack is
        // always mapped readable.
        if unsafe { ptr::read_volatile(word) } != PAINT {
            break;
        }
        word = unsafe { word.add(1) };
    }
    top as usize - word as usize
}
