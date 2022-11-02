use std::{env, ffi::OsStr, path::PathBuf, process::Command};

fn main() {
    let is_release = matches!(env::var("PROFILE"), Ok(profile) if profile == "release");
    let lib_search_path = if is_release && env::var("RELEASE_TURBO_CLI") == Ok("true".to_string()) {
        expect_release_lib()
    } else {
        build_debug_libturbo()
    };
    println!("cargo:rustc-link-search={}", lib_search_path);
    println!("cargo:rustc-link-lib=turbo");

    let bindings = bindgen::Builder::default()
        .header("../cli/libturbo.h")
        // Tell cargo to invalidate the built crate whenever any of the
        // included header files changed.
        .parse_callbacks(Box::new(bindgen::CargoCallbacks))
        .allowlist_function("nativeRunWithArgs")
        .generate()
        .expect("Unable to generate bindings");

    bindings
        .write_to_file("src/ffi.rs")
        .expect("Couldn't write bindings!");
    let target = build_target::target().unwrap();
    if target.os == build_target::Os::MacOs {
        println!("cargo:rustc-link-lib=framework=cocoa");
        println!("cargo:rustc-link-lib=framework=security");
    }
}

fn expect_release_lib() -> String {
    let target = build_target::target().unwrap();
    let platform = match target.os {
        build_target::Os::MacOs => "darwin",
        build_target::Os::Windows => "windows",
        build_target::Os::Linux => "linux",
        _ => panic!("unsupported target {}", target.triple),
    };
    let arch = match target.arch {
        build_target::Arch::AARCH64 => "arm64",
        build_target::Arch::X86_64 => "amd64_v1",
        _ => panic!("unsupported target {}", target.triple),
    };
    let mut dir = PathBuf::from("libturbo");
    // format is ${BUILD_ID}_${OS}_${ARCH}. Build id is, for goreleaser reasons,
    // turbo-${OS}
    dir.push(format!("turbo-{platform}_{platform}_{arch}"));
    dir.push("lib");
    dir.to_string_lossy().to_string()
}

fn build_debug_libturbo() -> String {
    let cli_path = env::var_os("CARGO_WORKSPACE_DIR")
        .map(PathBuf::from)
        .unwrap()
        .join("cli");
    let target = build_target::target().unwrap();
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
            let _ = std::fs::remove_file(output_deps.join(&format!("turbo.{ext}"))).unwrap_or(());
        }
        let mut cmd = Command::new("go");
        assert!(
            cmd.current_dir(&cli_path)
                .env("CGO_ENABLED", "1")
                .env("CC", "clang")
                .env("CXX", "clang++")
                .arg("build")
                .arg("-buildmode=c-archive")
                .arg("-o")
                .arg("turbo.lib")
                .arg("./cmd/turbo/...")
                .stdout(std::process::Stdio::inherit())
                .status()
                .expect("failed to build turbo.lib")
                .success(),
            "failed to build turbo.lib"
        );
    } else {
        let mut cmd = new_command("make");
        cmd.current_dir(&cli_path);
        cmd.arg("libturbo.a");
        let mut child = cmd.spawn().expect("failed to spawn make libturbo.a");
        child.wait().expect("failed to build libturbo.a");
    }
    cli_path.to_string_lossy().to_string()
}

fn new_command(program: impl AsRef<OsStr>) -> Command {
    let mut cmd = Command::new("sh");
    cmd.args(["-c", "exec \"$0\" \"$@\""]).arg(program);
    cmd
}
