use std::process::{Command, Stdio};

use anyhow::Result;
use tracing::debug;

use crate::{
    child::spawn_child,
    cli::{GenerateCommand, GeneratorCustomArgs},
};

fn verify_requirements() -> Result<()> {
    let output = Command::new("npx")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    match output {
        Ok(result) if result.success() => Ok(()),
        _ => Err(anyhow::anyhow!(
            "Unable to run generate - missing requirements (npx)"
        )),
    }
}

fn call_turbo_gen(command: &str, tag: &String, raw_args: &str) -> Result<i32> {
    debug!(
        "Running @turbo/gen@{} with command `{}` and args {:?}",
        tag, command, raw_args
    );
    let mut npx = Command::new("npx");
    npx.arg("--yes")
        .arg(format!("@turbo/gen@{}", tag))
        .arg("raw")
        .arg(command)
        .args(["--json", raw_args])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let child = spawn_child(npx)?;
    let exit_code = child.wait()?.code().unwrap_or(2);
    Ok(exit_code)
}

pub fn run(
    tag: &String,
    command: &Option<GenerateCommand>,
    args: &GeneratorCustomArgs,
) -> Result<()> {
    // ensure npx is available
    verify_requirements()?;

    match command {
        // check if a subcommand was passed
        Some(command) => {
            if let GenerateCommand::Workspace(workspace_args) = command {
                let raw_args = serde_json::to_string(&workspace_args)?;
                call_turbo_gen("workspace", tag, &raw_args)?;
            } else {
                let raw_args = serde_json::to_string(&args)?;
                call_turbo_gen("run", tag, &raw_args)?;
            }
        }
        // if no subcommand was passed, run the generate command as default
        None => {
            let raw_args = serde_json::to_string(&args)?;
            call_turbo_gen("run", tag, &raw_args)?;
        }
    };

    Ok(())
}
