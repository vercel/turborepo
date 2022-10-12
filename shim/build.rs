use std::{env, ffi::OsStr, process::Command, path::PathBuf};


fn main() {
    let is_release = match env::var("PROFILE") {
        Ok(profile) if profile == "release" => true,
        _ => false
    };
    let lib_search_path = if is_release {
        expect_release_lib()
    } else {
        build_debug_libturbo()
    };
    println!("cargo:rustc-link-search={}", lib_search_path);
    println!("cargo:rustc-link-lib=turbo");
    if cfg!(target_os = "macos") {
      println!("cargo:rustc-link-lib=framework=cocoa");
      println!("cargo:rustc-link-lib=framework=security");
    }
}
// setup rust in docker: curl https://sh.rustup.rs -sSf | bash -s -- -y
// M1 -> x86_64 linux: RUSTFLAGS='-C linker=x86_64-linux-gnu-gcc' cargo build --release --target x86_64-unknown-linux-gnu


fn expect_release_lib() -> String {
    let target = build_target::target().unwrap();
    let (platform, dist) = match target.os {
        build_target::Os::MacOs => ("darwin", "darwin"),
        build_target::Os::Windows => ("windows", "cross"),
        build_target::Os::Linux => ("linux", "cross"),
        _ => panic!("unsupported target {}", target.triple)
    };
    let arch = match target.arch {
        build_target::Arch::AARCH64 => "arm64",
        build_target::Arch::X86_64 => "amd64_v1",
        _ => panic!("unsupported target {}", target.triple)
    };
    let mut dir = PathBuf::from("libturbo");
    // format is ${BUILD_ID}_${OS}_${ARCH}. Build id is, for goreleaser reasons, turbo-${OS}
    dir.push(format!("turbo-{}_{}_{}", platform, platform, arch));
    dir.push("lib");
    dir.to_string_lossy().to_string()
}

fn build_debug_libturbo() -> String {
    let cli_path = "../cli";
    let mut cmd = new_command("make");
    cmd.current_dir(&cli_path);
    cmd.arg("libturbo.a");
    cli_path.to_string()
}

fn new_command<S: AsRef<OsStr>>(program: S) -> Command {
  let mut cmd = Command::new("sh");
  cmd.args(["-c", "exec \"$0\" \"$@\""]).arg(program);
  cmd
}
