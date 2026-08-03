//! Device-local state machines, each speaking to one chip over a bus.
//!
//! A driver owns the register sequences and ordering rules of a single device
//! and nothing above it. What it must not own is the cadence a sensor is read
//! at or what a reading means - those belong to [`crate::runners`] and
//! [`pineforge_state`], which is what keeps a driver from holding a policy that
//! can only be checked by wearing the watch.
//!
//! Both are generic over the `embedded-hal` trait they need, which is why they
//! are here rather than in the firmware. The `PineTime`'s two GPIO-only drivers,
//! the backlight and the motor, have no bus to abstract and stay with the board
//! that owns their pins.

pub mod bma42x;
pub mod hrs3300;
