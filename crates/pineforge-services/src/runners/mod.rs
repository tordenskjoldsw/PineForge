//! Executor-independent runners.
//!
//! A runner owns a subsystem's lifecycle and cadence and exposes `run()`. It
//! names no executor and no chip: the concrete Embassy task in the firmware
//! binds `PineTime` peripherals and its [`crate::ports`] to it and spawns it.
//!
//! The rule runs one way. A subsystem with nothing portable to extract - a
//! motor that pulses, a watchdog that is petted, deadline arithmetic only
//! `embassy-time` can do - is a task alone and has no runner half, and no
//! wrapper is invented for symmetry.

pub mod heart_rate;
pub mod motion;
