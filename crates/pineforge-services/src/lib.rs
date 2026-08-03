#![no_std]
// A runner's future holds capacity-erased channel handles, which borrow the
// channel behind a `&dyn` and are therefore not `Send`. Nothing here is ever
// moved between threads: the watch runs one Cortex-M thread executor, and a
// host test drives the future on the thread that built it.
#![allow(clippy::future_not_send)]

//! The sensors, and what drives them over time.
//!
//! Separate from the firmware for the reason [`pineforge_ui`] is: nothing here
//! needs a watch. A driver speaks `embedded-hal` to whatever bus it is handed,
//! and a runner owns a subsystem's lifecycle and cadence above it - so what a
//! sensor does over an hour can be exercised on a host instead of read off a
//! wrist.
//!
//! That property was claimed before this crate existed, and it was not true.
//! The runners lived in the firmware binary, which is `no_main` and pulls in
//! `embassy-nrf`, so `cargo test` could never reach them; the boundary was held
//! by a grep for the string `embassy_nrf` in `scripts/check-layers.sh`. A grep
//! is a spelling rule - a runner that reached the chip through
//! `crate::drivers::backlight` would have passed it. Cargo checks this one.
//!
//! # Ports rather than globals
//!
//! A runner used to reach directly for the firmware's `ipc` statics. That is
//! what tied it to the binary, and it is also what made a second instance
//! impossible: two runners would have shared one channel whether or not that
//! was wanted. Each now takes its endpoints as [`ports`], erased of their
//! capacities, so the firmware hands it the real bus and a test hands it one of
//! its own.
//!
//! # Logging
//!
//! `defmt` is optional, and off when the host tests run. Its macros need a
//! global logger at link time, which a test binary has no business providing;
//! the shims in [`log`] forward to `defmt` for a firmware build and evaluate to
//! nothing otherwise.

#[cfg(test)]
extern crate std;

mod log;

pub mod drivers;
pub mod ports;
pub mod runners;
