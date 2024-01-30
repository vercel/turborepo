#![feature(assert_matches)]
#![feature(box_patterns)]
#![feature(byte_slice_trim_ascii)]
#![feature(error_generic_member_access)]
#![feature(hash_extract_if)]
#![feature(option_get_or_insert_default)]
#![feature(once_cell_try)]
#![deny(clippy::all)]
// Clippy's needless mut lint is buggy: https://github.com/rust-lang/rust-clippy/issues/11299
#![allow(clippy::needless_pass_by_ref_mut)]
#![allow(dead_code)]

mod child;
mod cli;
mod commands;
mod config;
mod daemon;
mod engine;

mod execution_state;
mod framework;
pub(crate) mod globwatcher;
mod hash;
mod opts;
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

use miette::Report;

pub use crate::{
    child::spawn_child,
    cli::Args,
    commands::DaemonRootHasher,
    daemon::{DaemonClient, DaemonConnector},
    execution_state::ExecutionState,
    run::package_discovery::DaemonPackageDiscovery,
};
use crate::{engine::BuilderError, shim::Error};

pub fn get_version() -> &'static str {
    include_str!("../../../version.txt")
        .split_once('\n')
        .expect("Failed to read version from version.txt")
        .0
        // On windows we still have a trailing \r
        .trim_end()
}

pub fn main() -> Result<i32, shim::Error> {
    match shim::run() {
        Ok(code) => Ok(code),
        // We only print using miette for some errors because we want to keep
        // compatibility with Go. When we've deleted the Go code we can
        // move all errors to miette since it provides slightly nicer
        // printing out of the box.
        Err(
            err @ (Error::MultipleCwd(..)
            | Error::EmptyCwd { .. }
            | Error::Cli(cli::Error::Run(run::Error::Builder(engine::BuilderError::Config(
                config::Error::InvalidEnvPrefix { .. },
            ))))
            | Error::Cli(cli::Error::Run(run::Error::Config(
                config::Error::InvalidEnvPrefix { .. },
            )))
            | Error::Cli(cli::Error::Run(run::Error::Config(
                config::Error::TurboJsonParseError(_),
            )))
            | Error::Cli(cli::Error::Run(run::Error::Builder(BuilderError::Config(
                config::Error::TurboJsonParseError(_),
            ))))
            | Error::Cli(cli::Error::Run(run::Error::Config(
                config::Error::PackageTaskInSinglePackageMode { .. },
            )))
            | Error::Cli(cli::Error::Run(run::Error::Builder(
                engine::BuilderError::Validation { .. },
            )))
            | Error::Cli(cli::Error::Run(run::Error::Builder(engine::BuilderError::Config(
                ..,
            ))))),
        ) => {
            println!("{:?}", Report::new(err));

            Ok(1)
        }
        // We don't need to print "Turbo error" for Run errors
        Err(err @ shim::Error::Cli(cli::Error::Run(_))) => Err(err),
        Err(err) => {
            // This raw print matches the Go behavior, once we no longer care
            // about matching formatting we should remove this.
            println!("Turbo error: {err}");

            Err(err)
        }
    }
}
