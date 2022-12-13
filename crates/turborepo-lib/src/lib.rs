mod cli;
mod commands;
mod package_manager;
mod shim;

use anyhow::Result;
use tiny_gradient::{GradientStr, RGB};
use turbo_updater::check_for_updates;

pub use crate::cli::Args;
use crate::package_manager::PackageManager;

/// The payload from running main, if the program can complete without using Go
/// the Rust variant will be returned. If Go is needed then the args that
/// should be passed to Go will be returned.
pub enum Payload {
    Rust(Result<i32>),
    Go(Box<Args>),
}

fn get_version() -> &'static str {
    include_str!("../../../version.txt")
        .split_once('\n')
        .expect("Failed to read version from version.txt")
        .0
}

pub fn main() -> Result<Payload> {
    // custom footer for update message
    let footer = format!(
        "Follow {username} for updates: {url}",
        username = "@turborepo".gradient([RGB::new(0, 153, 247), RGB::new(241, 23, 18)]),
        url = "https://twitter.com/turborepo"
    );

    // check for updates
    let _ = check_for_updates(
        "turbo",
        "https://github.com/vercel/turbo",
        Some(&footer),
        get_version(),
        // use defaults for timeout and refresh interval (800ms and 1 day respectively)
        None,
        None,
    );

    shim::run()
}
