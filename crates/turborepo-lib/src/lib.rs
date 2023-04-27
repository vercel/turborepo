#![feature(assert_matches)]

mod child;
mod cli;
mod commands;
mod config;
mod daemon;
mod execution_state;
mod formatter;
mod package_json;
mod package_manager;
mod shim;
mod ui;

use anyhow::Result;
pub use child::spawn_child;
use tracing::error;

pub use crate::{cli::Args, execution_state::ExecutionState};
use crate::{commands::CommandBase, package_manager::PackageManager};

/// The payload from running main, if the program can complete without using Go
/// the Rust variant will be returned. If Go is needed then the execution state
/// that should be passed to Go will be returned.
pub enum Payload {
    Rust(Result<i32>),
    Go(Box<CommandBase>),
}

pub fn get_version() -> &'static str {
    include_str!("../../../version.txt")
        .split_once('\n')
        .expect("Failed to read version from version.txt")
        .0
        // On windows we still have a trailing \r
        .trim_end()
}

pub fn main() -> Payload {
    match shim::run() {
        Ok(payload) => payload,
        Err(err) => {
            error!("{}", err.to_string());
            Payload::Rust(Err(err))
        }
    }
}
