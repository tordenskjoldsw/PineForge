//! Executor-independent product behavior.
//!
//! A service is a runner: it owns a subsystem's lifecycle and cadence, is
//! generic over the `embedded-hal` traits it needs, and exposes `run()`. What
//! it must not contain is the executor or the board - no `#[embassy_executor::task]`,
//! no `embassy_nrf`. The concrete task in [`crate::tasks`] binds `PineTime`
//! peripherals to it and spawns it.
//!
//! That boundary is what makes the cadence and lifecycle of a sensor readable
//! without a watch attached, and it is checked in CI rather than left to
//! discipline. A subsystem with nothing portable to extract - a motor that
//! pulses, a watchdog that is petted - is a task alone and has no service half;
//! the rule runs one way only.

pub mod heart_rate;
pub mod motion;
