use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    let is_release = matches!(env::var("PROFILE"), Ok(profile) if profile == "release");
    let lib_search_path = if is_release && env::var("RELEASE_TURBO_CLI") == Ok("true".to_string()) {
        expect_release_lib()
    } else {
        build_debug_libturbo()
    };
    println!("cargo:rerun-if-changed={}", lib_search_path);
    println!("cargo:rustc-link-search={}", lib_search_path);
    println!("cargo:rustc-link-lib=turbo");

    let target = build_target::target().unwrap();
    let bindings = bindgen::Builder::default()
        .header(header_path(&target.os))
        // Tell cargo to invalidate the built crate whenever any of the
        // included header files changed.
        .parse_callbacks(Box::new(bindgen::CargoCallbacks))
        .allowlist_function("nativeRunWithTurboState")
        .allowlist_type("GoString")
        .generate()
        .expect("Unable to generate bindings");

    let out_dir = env::var("OUT_DIR").unwrap();
    let out_path = Path::new(&out_dir).join("bindings.rs");

    bindings
        .write_to_file(out_path)
        .expect("Couldn't write bindings!");

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

        cmd.env("CGO_ENABLED", "1")
            .env("CC", "gcc")
            .env("CXX", "g++")
            .arg("turbo.lib");
    } else {
        cmd.arg("libturbo.a");
    }
    assert!(
        cmd.stdout(std::process::Stdio::inherit())
            .status()
            .expect("failed to build turbo.lib")
            .success(),
        "failed to build turbo static library"
    );
    cli_path.to_string_lossy().to_string()
}

fn header_path(target: &build_target::Os) -> &'static str {
    match target {
        build_target::Os::Windows => "../cli/turbo.h",
        _ => "../cli/libturbo.h",
    }
}
