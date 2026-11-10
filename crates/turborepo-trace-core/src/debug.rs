//! Debug switches: timings, dumps, and the A/B switches that turn one rule of
//! the selection off to measure what it does. They are read from
//! `TURBO_TRACE_DEBUG_<NAME>` environment variables, here and nowhere else. Not
//! a user interface: undocumented, not configuration, and free to change
//! between versions. User options of `turbo trace` are flags.
//!
//! The switches exist only with the cargo feature `debug-knobs` (off by
//! default, so not in release builds). Without it every switch reads as unset,
//! which is each one's default. The crate README lists them.

use std::ffi::OsString;

/// The prefix of every debug variable.
pub const PREFIX: &str = "TURBO_TRACE_DEBUG_";

/// `TURBO_TRACE_DEBUG_<name>`, if set (always unset without `debug-knobs`).
#[cfg(feature = "debug-knobs")]
pub fn var_os(name: &str) -> Option<OsString> {
    std::env::var_os(format!("{PREFIX}{name}"))
}

/// `TURBO_TRACE_DEBUG_<name>`, if set (always unset without `debug-knobs`).
#[cfg(not(feature = "debug-knobs"))]
pub fn var_os(_name: &str) -> Option<OsString> {
    None
}

/// `TURBO_TRACE_DEBUG_<name>` as UTF-8 ([`std::env::var`]; always unset
/// without `debug-knobs`).
#[cfg(feature = "debug-knobs")]
pub fn var(name: &str) -> Result<String, std::env::VarError> {
    std::env::var(format!("{PREFIX}{name}"))
}

/// `TURBO_TRACE_DEBUG_<name>` as UTF-8 ([`std::env::var`]; always unset
/// without `debug-knobs`).
#[cfg(not(feature = "debug-knobs"))]
pub fn var(_name: &str) -> Result<String, std::env::VarError> {
    Err(std::env::VarError::NotPresent)
}
