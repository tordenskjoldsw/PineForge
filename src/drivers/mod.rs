//! Device-local state machines, each speaking to one chip.
//!
//! A driver owns the register sequences and the ordering rules of a single
//! device, and nothing above it. What it must not own is the cadence a sensor
//! is read at or what a reading means - those belong to [`crate::services`] and
//! [`pineforge_state`], which is what keeps a driver from having a policy that
//! can only be checked by wearing the watch.
//!
//! Most of them are generic over the `embedded-hal` trait they need, which is
//! what lets a service stay executor- and chip-independent above them.
//! [`backlight`] and [`vibration`] are the exceptions: both are a handful of
//! GPIO writes with no bus to abstract, and both name `embassy_nrf` directly.
//!
//! That exception is the one thing to know before reaching for a driver from a
//! service. `scripts/check-layers.sh` greps `src/services/` for `embassy_nrf`,
//! so importing either of those two from there would carry the chip across the
//! layer boundary without the check seeing a thing.

pub mod backlight;
pub mod bma42x;
pub mod hrs3300;
pub mod touch;
pub mod vibration;
pub mod xt25f32;
