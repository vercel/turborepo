use std::{
    env::{consts, current_exe},
    process,
    process::Stdio,
};

use anyhow::Result;
use dunce::canonicalize as fs_canonicalize;
use log::{debug, error, trace};
use turborepo_lib::{Args, Payload};

fn run_go_binary(args: Args) -> Result<i32> {
    // canonicalize the binary path to ensure we can find go-turbo
    let turbo_path = fs_canonicalize(current_exe()?)?;
    let mut go_binary_path = turbo_path.clone();
    go_binary_path.pop();
    #[cfg(windows)]
    go_binary_path.push("go-turbo.exe");
    #[cfg(not(windows))]
    go_binary_path.push("go-turbo");

    if go_binary_path.exists() {
        debug!("Found go binary at {:?}", go_binary_path);
    } else {
        error!("Unable to find Go binary. Please report this issue at https://github.com/vercel/turbo/issues and include your package manager and version along with the following information:
        os={os}
        arch={arch}
        turbo-version={turbo_version}
        turbo-bin={turbo_bin}
        go-turbo-bin={go_turbo_bin}
        ",
            os = consts::OS,
            arch = consts::ARCH,
            turbo_version = turborepo_lib::get_version(),
            turbo_bin = turbo_path.display(),
            go_turbo_bin = go_binary_path.display()
        );
        // return an error
        return Err(anyhow::anyhow!(
            "Failed to execute turbo (Unable to locate Go binary)."
        ));
    }

    let serialized_args = serde_json::to_string(&args)?;
    trace!("Invoking go binary with {}", serialized_args);
    let mut command = process::Command::new(go_binary_path);
    command
        .arg(serialized_args)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let shared_child = shared_child::SharedChild::spawn(&mut command).unwrap();
    let child_arc = std::sync::Arc::new(shared_child);

    let child_arc_clone = child_arc.clone();
    ctrlc::set_handler(move || {
        // we are quiting anyways so just ignore
        child_arc_clone.kill().ok().unwrap();
    })
    .expect("handler set");

    let exit_code = child_arc.wait()?.code().unwrap_or(2);

    Ok(exit_code)
}

// This function should not expanded. Please add any logic to
// `turborepo_lib::main` instead
fn main() -> Result<()> {
    let exit_code = match turborepo_lib::main() {
        Payload::Rust(res) => res.unwrap_or(1),
        Payload::Go(state) => run_go_binary(*state)?,
    };

    process::exit(exit_code)
}
