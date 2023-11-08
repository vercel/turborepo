use std::{
    io,
    process::{Command, Stdio},
};

use thiserror::Error;
use tracing::debug;
use which::which;

use crate::{
    child::spawn_child,
    cli::{GenerateCommand, GeneratorCustomArgs},
};

#[derive(Debug, Error)]
pub enum Error {
    #[error("Unable to run generate - missing requirements (npx): {0}")]
    NpxNotFound(#[source] which::Error),
    #[error("Failed to run npx: {0}")]
    NpxFailed(#[source] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

fn call_turbo_gen(command: &str, tag: &String, raw_args: &str) -> Result<i32, Error> {
    debug!(
        "Running @turbo/gen@{} with command `{}` and args {:?}",
        tag, command, raw_args
    );
    let npx_path = which("npx").map_err(Error::NpxNotFound)?;
    let mut npx = Command::new(npx_path);
    npx.arg("--yes")
        .arg(format!("@turbo/gen@{}", tag))
        .arg("raw")
        .arg(command)
        .args(["--json", raw_args])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let child = spawn_child(npx).map_err(Error::NpxFailed)?;
    let exit_code = child.wait().map_err(Error::NpxFailed)?.code().unwrap_or(2);
    Ok(exit_code)
}

pub fn run(
    tag: &String,
    command: &Option<Box<GenerateCommand>>,
    args: &GeneratorCustomArgs,
) -> Result<(), Error> {
    // check if a subcommand was passed
    if let Some(box GenerateCommand::Workspace(workspace_args)) = command {
        let raw_args = serde_json::to_string(&workspace_args)?;
        call_turbo_gen("workspace", tag, &raw_args)?;
    } else {
        // if no subcommand was passed, run the generate command as default
        let raw_args = serde_json::to_string(&args)?;
        call_turbo_gen("run", tag, &raw_args)?;
    }

    Ok(())
}
