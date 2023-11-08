use std::{env::current_exe, io};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("could not get path to turbo binary: {0}")]
    NoCurrentExe(#[from] io::Error),
}

pub fn run() -> Result<(), Error> {
    let path = current_exe()?;
    // NOTE: The Go version uses `base.UI.Output`, we should use the Rust equivalent
    // eventually.
    println!("{}", path.to_string_lossy());

    Ok(())
}
