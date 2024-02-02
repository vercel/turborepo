#![feature(panic_info_message)]
#![deny(clippy::all)]

mod panic_handler;

use std::process;

use anyhow::Result;
use miette::Report;

use crate::panic_handler::panic_handler;

// This function should not expanded. Please add any logic to
// `turborepo_lib::main` instead
fn main() -> Result<()> {
    std::panic::set_hook(Box::new(panic_handler));

    let exit_code = turborepo_lib::main().unwrap_or_else(|err| {
        println!("{:?}", Report::new(err));
        1
    });

    process::exit(exit_code)
}
