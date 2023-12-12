use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=../../cli");
    let profile = env::var("PROFILE").unwrap();
    let is_ci_release =
        &profile == "release" && matches!(env::var("RELEASE_TURBO_CLI"), Ok(v) if v == "true");

    let invocation = std::env::var("RUSTC_WRAPPER").unwrap_or_default();
    if !is_ci_release && !invocation.ends_with("rust-analyzer") {
        // build_local_go_binary(profile);
    }
}

#[cfg(any(not(feature = "go-binary"), doc))]
fn build_local_go_binary(_: String) {}

#[cfg(all(feature = "go-binary", not(doc)))]
fn build_local_go_binary(profile: String) {
    let cli_path = cli_path();
    let target = build_target::target().unwrap();

    let go_binary_name = if target.os == build_target::Os::Windows {
        "go-turbo.exe"
    } else {
        "go-turbo"
    };

    #[cfg(not(windows))]
    let mut cmd = {
        let mut cmd = Command::new("make");
        cmd.current_dir(&cli_path);
        cmd.arg(go_binary_name);
        cmd
    };
    #[cfg(windows)]
    let mut cmd = {
        let mut cmd = Command::new(cli_path.join("build_go.bat"));
        cmd.current_dir(&cli_path);
        cmd
    };

    assert!(
        cmd.stdout(std::process::Stdio::inherit())
            .status()
            .expect("failed to build go binary")
            .success(),
        "failed to build go binary"
    );

    let go_binary_path = env::var("CARGO_WORKSPACE_DIR")
        .map(PathBuf::from)
        .unwrap()
        .join("cli")
        .join(go_binary_name);

    let new_go_binary_path = env::var_os("CARGO_WORKSPACE_DIR")
        .map(PathBuf::from)
        .unwrap()
        .join("target")
        .join(profile)
        .join(go_binary_name);

    fs::rename(go_binary_path, new_go_binary_path).unwrap();
}

fn cli_path() -> PathBuf {
    env::var_os("CARGO_WORKSPACE_DIR")
        .map(PathBuf::from)
        .unwrap()
        .join("cli")
}
