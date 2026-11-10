//! Whether the repository's `turbotrace.toml` is read: its hints (`loads`,
//! `[[env]]`), `[lint]` and `[nextjs]` tables. On by default (turbotrace's own
//! command line and its tests); a host that does not support the file yet
//! turns it off before anything reads it, and every reader then behaves as if
//! there were none (`turbo trace` v1: options are flags only).

use std::sync::atomic::{AtomicBool, Ordering};

static READ: AtomicBool = AtomicBool::new(true);

/// Whether `turbotrace.toml` is read.
pub fn reads_turbotrace_toml() -> bool {
    READ.load(Ordering::Relaxed)
}

/// Turns reading `turbotrace.toml` on or off for this process.
pub fn set_reads_turbotrace_toml(on: bool) {
    READ.store(on, Ordering::Relaxed);
}
