#![feature(assert_matches)]
#![feature(box_patterns)]
#![feature(byte_slice_trim_ascii)]
#![feature(error_generic_member_access)]
#![feature(hash_extract_if)]
#![feature(option_get_or_insert_default)]
#![feature(once_cell_try)]
#![feature(try_blocks)]
#![feature(impl_trait_in_assoc_type)]
#![feature(lazy_cell)]
#![deny(clippy::all)]
// Clippy's needless mut lint is buggy: https://github.com/rust-lang/rust-clippy/issues/11299
#![allow(clippy::needless_pass_by_ref_mut)]
#![allow(dead_code)]

mod child;
mod cli;
mod commands;
mod config;
mod daemon;
mod diagnostics;
mod engine;

mod framework;
mod gitignore;
mod global_deps_package_change_mapper;
pub(crate) mod globwatcher;
mod hash;
mod opts;
mod package_changes_watcher;
mod process;
mod rewrite_json;
mod run;
mod shim;
mod signal;
mod task_graph;
mod task_hash;
mod tracing;
mod turbo_json;
mod unescape;

pub use crate::{
    child::spawn_child,
    cli::Args,
    daemon::{DaemonClient, DaemonConnector, Paths as DaemonPaths},
    run::package_discovery::DaemonPackageDiscovery,
};

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
