#![feature(assert_matches)]
#![feature(box_patterns)]
#![feature(error_generic_member_access)]
#![feature(once_cell_try)]
#![feature(try_blocks)]
#![feature(impl_trait_in_assoc_type)]
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
mod diagnostics;
mod engine;

mod boundaries;
mod microfrontends;
mod opts;
mod package_changes_watcher;
mod panic_handler;
mod query;
mod run;
mod shim;
mod task_graph;
mod task_hash;
mod tracing;
mod turbo_json;

pub use run::package_discovery::DaemonPackageDiscovery;
// Re-export daemon types from the new crate location
pub use turborepo_daemon::{
    DaemonClient, DaemonConnector, DaemonConnectorError, DaemonError, Paths as DaemonPaths,
};

pub use crate::{child::spawn_child, cli::Args, panic_handler::panic_handler};

pub fn get_version() -> &'static str {
    include_str!("../../../version.txt")
        .split_once('\n')
        .expect("Failed to read version from version.txt")
        .0
        // On windows we still have a trailing \r
        .trim_end()
}

pub fn main() -> Result<i32, shim::Error> {
    shim::run()
}

#[cfg(all(feature = "native-tls", feature = "rustls-tls"))]
compile_error!("You can't enable both the `native-tls` and `rustls-tls` feature.");

#[cfg(all(not(feature = "native-tls"), not(feature = "rustls-tls")))]
compile_error!("You have to enable one of the TLS features: `native-tls` or `rustls-tls`");
