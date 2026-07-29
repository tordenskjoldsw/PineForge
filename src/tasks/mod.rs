//! Every task the executor runs.
//!
//! A task binds concrete `PineTime` resources (a bus handle, a pin, a
//! peripheral) to whatever does the work, and is the only layer allowed to name
//! `embassy_nrf`. Where the work has a portable half it lives in
//! [`crate::services`] and the task here is the few lines that start it; where
//! it does not, the task is the whole of it.
//!
//! Each stateful peripheral has exactly one owning task, and they reach each
//! other only through [`crate::ipc`].

pub mod accelerometer;
pub mod battery;
#[cfg(feature = "ble")]
pub mod ble;
pub mod button;
pub mod display;
pub mod heart_rate;
pub mod input;
pub mod power;
pub mod storage;
pub mod vibration;
pub mod watchdog;
