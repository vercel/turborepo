#![feature(box_patterns)]
#![feature(try_blocks)]
// miette's derive macro causes false positives for this lint
#![allow(unused_assignments)]
#![deny(clippy::all)]
// Clippy's needless mut lint is buggy: https://github.com/rust-lang/rust-clippy/issues/11299
#![allow(clippy::needless_pass_by_ref_mut)]
#![allow(clippy::result_large_err)]
#![allow(dead_code)]

mod child;
mod cli;
mod commands;
mod config;
pub mod devtools;
mod engine;
#[cfg(feature = "heap-dhat")]
mod heap_profile;

mod boundaries;
mod microfrontends;
mod opts;
mod package_changes_watcher;
mod panic_handler;
mod rayon_compat;
mod run;
mod shim;
mod task_change_detector;
mod task_graph;
mod task_hash;
mod tracing;
mod turbo_json;

pub use run::package_discovery::DaemonPackageDiscovery;
// Re-export daemon types from the new crate location
pub use turborepo_daemon::{
    DaemonClient, DaemonConnector, DaemonConnectorError, DaemonError, Paths as DaemonPaths,
};
pub use turborepo_query_api::QueryServer;

pub use crate::{child::spawn_child, cli::Args, panic_handler::panic_handler};

#[cfg(feature = "heap-dhat")]
pub fn finish_heap_profile() {
    heap_profile::finish_global();
}

#[cfg(not(feature = "heap-dhat"))]
pub fn finish_heap_profile() {}

pub fn get_version() -> &'static str {
    include_str!("../../../version.txt")
        .split_once('\n')
        .map_or(include_str!("../../../version.txt"), |(version, _)| version)
        // On windows we still have a trailing \r
        .trim_end()
}

/// Main entry point for the turborepo CLI.
///
/// `query_server` provides the GraphQL query execution layer. When `None`,
/// the `turbo query` command returns an error. Pass `Some(...)` with a
/// [`QueryServer`] implementation to enable the full query subsystem.
pub fn main(
    query_server: Option<std::sync::Arc<dyn turborepo_query_api::QueryServer>>,
) -> Result<i32, shim::Error> {
    raise_open_file_limit();
    shim::run(query_server)
}

/// Raise the process's soft `RLIMIT_NOFILE` to its hard limit.
///
/// Task execution costs several descriptors per live child (pipes or pty),
/// and default concurrency is ten times the core count, so a large run
/// needs hundreds of descriptors at once. macOS defaults the soft limit to
/// 256, which `turbo run` on a many-core machine exhausts at the first
/// spawn burst (`Too many open files`). Raising the soft limit at startup
/// is the same remedy Node.js and the Go runtime apply.
#[cfg(unix)]
fn raise_open_file_limit() {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit/setrlimit are passed a valid, initialized rlimit.
    unsafe {
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) != 0 || limit.rlim_cur >= limit.rlim_max
        {
            return;
        }
        let mut raised = limit;
        // Clamp below the hard limit: tools spawned by tasks may iterate
        // file descriptors up to the soft limit (the classic `closefrom`
        // loop), which a million-entry table makes pathological. 65536
        // covers any realistic spawn burst.
        raised.rlim_cur = limit.rlim_max.min(65536);
        if libc::setrlimit(libc::RLIMIT_NOFILE, &raised) != 0 {
            // macOS reports RLIM_INFINITY as the hard limit but rejects
            // raising the soft limit past `kern.maxfilesperproc`; its
            // documented ceiling for unprivileged processes is OPEN_MAX
            // (10240).
            raised.rlim_cur = limit.rlim_max.min(10240);
            let _ = libc::setrlimit(libc::RLIMIT_NOFILE, &raised);
        }
    }
}

#[cfg(not(unix))]
fn raise_open_file_limit() {
    // Windows has no RLIMIT_NOFILE equivalent; handle limits are already
    // in the tens of thousands.
}

#[cfg(all(feature = "native-tls", feature = "rustls-tls"))]
compile_error!("You can't enable both the `native-tls` and `rustls-tls` feature.");

#[cfg(all(not(feature = "native-tls"), not(feature = "rustls-tls")))]
compile_error!("You have to enable one of the TLS features: `native-tls` or `rustls-tls`");
