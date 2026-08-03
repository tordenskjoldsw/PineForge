//! The drivers that stayed with the firmware.
//!
//! Two of them have to be here. [`backlight`] and [`vibration`] are a handful
//! of GPIO writes with no bus to abstract, so they name `embassy_nrf` directly
//! and moving them would mean inventing a pin abstraction for the sake of
//! symmetry.
//!
//! [`touch`] and [`xt25f32`] are generic over their bus and could move, but
//! there is nothing yet to move them for: both are owned by a task with no
//! runner half - the input task and the storage task - so they would arrive in
//! [`pineforge_services`] with no caller a host could exercise them through.
//! They follow if and when their owners grow one.
//!
//! What that leaves is the one thing to know before reaching for a driver from
//! a runner: anything in this module carries the chip with it. The sensor
//! drivers a runner does use live in [`pineforge_services::drivers`], which is
//! a crate that cannot name `embassy_nrf` at all.

pub mod backlight;
pub mod touch;
pub mod vibration;
pub mod xt25f32;
