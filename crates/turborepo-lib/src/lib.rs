mod cli;
mod client;
mod commands;
mod config;
mod daemon;
mod package_manager;
mod retry;
mod shim;
mod ui;

use anyhow::Result;
use log::error;

pub use crate::cli::Args;
use crate::package_manager::PackageManager;

/// The payload from running main, if the program can complete without using Go
/// the Rust variant will be returned. If Go is needed then the args that
/// should be passed to Go will be returned.
pub enum Payload {
    Rust(Result<i32>),
    Go(Box<Args>),
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
