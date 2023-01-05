use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    let is_ci_release = matches!(env::var("PROFILE"), Ok(profile) if profile == "release");
    if !is_ci_release {
        build_debug_go_binary();
    }
}

fn build_debug_go_binary() -> PathBuf {
    let cli_path = cli_path();
    let target = build_target::target().unwrap();
    let mut cmd = Command::new("make");
    cmd.current_dir(&cli_path);
    if target.os == build_target::Os::Windows {
        let output_dir = env::var_os("OUT_DIR").map(PathBuf::from).unwrap();
        let output_deps = output_dir
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("deps");
        // workaround to make increment build works
        for ext in ["pdb", "exe", "d", "lib"].iter() {
            std::fs::remove_file(output_deps.join(format!("turbo.{ext}"))).unwrap_or(());
        }
        cmd.arg("go-turbo.exe");
    } else {
        cmd.arg("go-turbo");
    }
    assert!(
        cmd.stdout(std::process::Stdio::inherit())
            .status()
            .expect("failed to build go binary")
            .success(),
        "failed to build go binary"
    );

    let go_binary_name = if target.os == build_target::Os::Windows {
        "go-turbo.exe"
    } else {
        "go-turbo"
    };

    let go_binary_path = env::var("CARGO_WORKSPACE_DIR")
        .map(PathBuf::from)
        .unwrap()
        .join("cli")
        .join(go_binary_name);

    let new_go_binary_path = env::var_os("CARGO_WORKSPACE_DIR")
        .map(PathBuf::from)
        .unwrap()
        .join("target")
        .join("debug")
        .join(go_binary_name);

    fs::rename(go_binary_path, new_go_binary_path).unwrap();
    cli_path
}

fn cli_path() -> PathBuf {
    env::var_os("CARGO_WORKSPACE_DIR")
        .map(PathBuf::from)
        .unwrap()
        .join("cli")
}
