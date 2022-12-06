use std::env::current_exe;

use anyhow::{anyhow, Result};

pub fn run() -> Result<()> {
    let path = current_exe().map_err(|e| anyhow!("could not get path to turbo binary: {}", e))?;
    // NOTE: The Go version uses `base.UI.Output`, we should use the Rust equivalent
    // eventually.
    println!("{}", path.to_string_lossy());

    Ok(())
}
