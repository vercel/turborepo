use std::process::{Command, Stdio};

use anyhow::Result;

use crate::{child::spawn_child, cli::GenerateCommand};

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

pub fn run(command: &GenerateCommand, tag: &String) -> Result<()> {
    // ensure npx is available
    verify_requirements()?;

    match command {
        GenerateCommand::Add(args) => {
            let mut add_args = args.clone();
            // example implies copy
            if add_args.example.is_some() {
                add_args.copy = true;
                add_args.empty = false;
            }

            // convert args to json
            let raw_args = serde_json::to_string(&add_args)?;
            call_turbo_gen("add", tag, &raw_args)?;
        }
        GenerateCommand::Custom(args) => {
            let raw_args = serde_json::to_string(args)?;
            call_turbo_gen("generate", tag, &raw_args)?;
        }
    };

    Ok(())
}
