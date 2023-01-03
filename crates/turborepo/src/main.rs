use std::{env::current_exe, io::Write, process, process::Stdio};

use anyhow::Result;
use turborepo_lib::{Args, Payload};

fn native_run(args: Args) -> Result<i32> {
    let mut go_binary_path = current_exe()?;
    go_binary_path.pop();
    go_binary_path.pop();
    go_binary_path.pop();
    go_binary_path.push("cli");
    go_binary_path.push("turbo");

    let mut command = process::Command::new(go_binary_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("Failed to execute turbo.");

    let serialized_args = serde_json::to_string(&args)?;

    command
        .stdin
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("Failed to get stdin"))?
        .write_all(serialized_args.as_bytes())?;
    let exit_code = command.wait()?.code().unwrap_or(2);

    Ok(exit_code.try_into()?)
}

// This function should not expanded. Please add any logic to
// `turborepo_lib::main` instead
fn main() -> Result<()> {
    let exit_code = match turborepo_lib::main()? {
        Payload::Rust(res) => res.unwrap_or(1),
        Payload::Go(state) => native_run(*state)?,
    };

    process::exit(exit_code)
}
