//! `defmt` where there is a logger, and nothing where there is not.
//!
//! The firmware links `defmt-rtt` and wants every one of these lines. A host
//! test links no logger at all, and `defmt`'s macros reference symbols that
//! only a logger provides - so a crate that called them unconditionally would
//! compile for the watch and fail to link for `cargo test`, which is the one
//! thing this crate exists to make possible.
//!
//! Named `log_info`/`log_warn` rather than `info`/`warn`: a macro called `warn`
//! is ambiguous against the built-in lint attribute of the same name, and the
//! error that produces says nothing about which of the two was meant.
//!
//! The inactive arm still names its arguments. A message is often the only
//! place a value is read, and a shim that dropped it would turn every disabled
//! log into an unused-variable warning - which CI denies.

#[cfg(feature = "defmt")]
macro_rules! log_info {
    ($format:literal $(, $arg:expr)* $(,)?) => { ::defmt::info!($format $(, $arg)*) };
}

#[cfg(feature = "defmt")]
macro_rules! log_warn {
    ($format:literal $(, $arg:expr)* $(,)?) => { ::defmt::warn!($format $(, $arg)*) };
}

#[cfg(not(feature = "defmt"))]
macro_rules! log_info {
    ($format:literal $(, $arg:expr)* $(,)?) => {{ $( let _ = &$arg; )* }};
}

#[cfg(not(feature = "defmt"))]
macro_rules! log_warn {
    ($format:literal $(, $arg:expr)* $(,)?) => {{ $( let _ = &$arg; )* }};
}

pub(crate) use {log_info, log_warn};
