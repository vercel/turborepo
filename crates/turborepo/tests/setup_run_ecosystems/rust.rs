//! TURBO-6231: a Cargo workspace without host Rust.
use sha2::{Digest, Sha256};

use super::*;

const TURBO_JSON: &str = r#"{"futureFlags":{"experimentalCargoWorkspaces":true},
    "agentGuidance":false,"noUpdateNotifier":true,"tasks":{"build":{"cache":false}}}"#;
const CARGO_TOML: &str = "[workspace]\nmembers = [\"crates/app\"]\nresolver = \
                          \"2\"\n\n[workspace.metadata]\nname = \"demo\"\n";
const TOOLCHAIN: &str = "[toolchain]\nchannel = \"stable\"\nprofile = \"minimal\"\n";

/// Answers `cargo metadata` for the one-crate workspace and reports builds.
const CARGO: &str = r#"#!/bin/sh
echo "cargo $*" >> "$HOME/managed-calls"
root=$(pwd -P)
id="path+file://$root/crates/app#0.1.0"
case "$1" in
metadata) printf '{"packages":[{"id":"%s","name":"app","source":null,"manifest_path":"%s/crates/app/Cargo.toml","dependencies":[],"targets":[{"name":"app","kind":["lib"]}]}],"workspace_members":["%s"],"target_directory":"%s/target","metadata":{"name":"demo"}}\n' "$id" "$root" "$id" "$root" ;;
build) echo "managed-cargo $* toolchain=$RUSTUP_TOOLCHAIN cargo_home=${CARGO_HOME##*/.turbo/} secret=${JOURNEY_SECRET:-filtered}" ;;
*) exit 1 ;;
esac
"#;
const RUSTC: &str = r#"#!/bin/sh
echo "rustc $*" >> "$HOME/managed-calls"
case "$1" in
-vV) printf 'rustc 1.99.0 (b9 2026-10-01)\nbinary: rustc\nhost: HOST\nrelease: 1.99.0\n' ;;
--print) echo HOST ;;
*) exit 1 ;;
esac
"#;

fn triple() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", _) => "x86_64-apple-darwin",
        (_, "aarch64") => "aarch64-unknown-linux-gnu",
        _ => "x86_64-unknown-linux-gnu",
    }
}

/// An official-shaped channel manifest for this host's minimal profile.
fn manifest(version: &str) -> String {
    let mut text = format!(
        "manifest-version = \"2\"\ndate = \"2026-10-01\"\n[profiles]\nminimal = [\"rustc\", \
         \"cargo\", \"rust-std\"]\n[pkg.rust]\nversion = \"{version} (b9 2026-10-01)\"\n"
    );
    for package in ["rustc", "cargo", "rust-std"] {
        let url = format!(
            "https://static.rust-lang.org/dist/2026-10-01/{package}-{version}-{}.tar.gz",
            triple()
        );
        let xz_url = url.replace(".tar.gz", ".tar.xz");
        text.push_str(&format!(
            "[pkg.{package}]\nversion = \"{version}\"\n[pkg.{package}.target.{}]\navailable = \
             true\nurl = \"{url}\"\nhash = \"{:x}\"\nxz_url = \"{xz_url}\"\nxz_hash = \"{:x}\"\n",
            triple(),
            Sha256::digest(&url),
            Sha256::digest(&xz_url)
        ));
    }
    text
}

/// The stand-in rustup lays out the requested toolchain from mock tools kept
/// outside the repository, plus rustup's release metadata and checksums.
fn routes(outside: &Path) -> Vec<(String, Vec<u8>)> {
    for (name, contents) in [("mock-cargo", CARGO), ("mock-rustc", RUSTC)] {
        executable(&outside.join(name), &contents.replace("HOST", triple()));
    }
    fs::write(outside.join("manifest-1.99.0.toml"), manifest("1.99.0")).unwrap();
    let installer = r#"#!/bin/sh
tc="$RUSTUP_HOME/toolchains/$3-HOST"
mkdir -p "$tc/bin" "$tc/lib/rustlib"
cp 'OUTSIDE/mock-cargo' "$tc/bin/cargo"
cp 'OUTSIDE/mock-rustc' "$tc/bin/rustc"
cp 'OUTSIDE/mock-rustc' "$tc/bin/rustdoc"
printf 'cargo-HOST\nrust-std-HOST\nrustc-HOST\n' > "$tc/lib/rustlib/components"
cp "OUTSIDE/manifest-$3.toml" "$tc/lib/rustlib/multirust-channel-manifest.toml"
"#
    .replace("OUTSIDE", &outside.display().to_string())
    .replace("HOST", triple());
    let digest = format!("{:x}\n", Sha256::digest(&installer)).into_bytes();
    let mut routes = vec![
        (
            format!("/rustup/archive/1.29.1/{}/rustup-init", triple()),
            installer.into_bytes(),
        ),
        (
            "/rustup/release-stable.toml".into(),
            b"schema-version = '1'\nversion = '1.29.1'\n".to_vec(),
        ),
        (
            "/dist/channel-rust-stable.toml".into(),
            manifest("1.99.0").into_bytes(),
        ),
    ];
    for arch in ["x86_64", "aarch64"] {
        for os in [
            "apple-darwin",
            "unknown-linux-gnu",
            "unknown-linux-musl",
            "pc-windows-msvc",
        ] {
            let exe = if os.contains("windows") { ".exe" } else { "" };
            let path = format!("/rustup/archive/1.29.1/{arch}-{os}/rustup-init{exe}.sha256");
            routes.push((path, digest.clone()));
        }
    }
    routes
}

#[test]
fn rust_setup_then_plain_run_uses_the_locked_toolchain_without_host_rust() {
    let outside = tempfile::tempdir().unwrap();
    let journey = Journey::new(
        &[
            ("turbo.json", TURBO_JSON),
            (".gitignore", "/.turbo/\n/target/\n"),
            ("Cargo.toml", CARGO_TOML),
            ("rust-toolchain.toml", TOOLCHAIN),
            (
                "Cargo.lock",
                "version = 4\n\n[[package]]\nname = \"app\"\nversion = \"0.1.0\"\n",
            ),
            (
                "crates/app/Cargo.toml",
                "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            ),
            ("crates/app/src/lib.rs", ""),
        ],
        routes(outside.path()),
    );
    // Setup resolves the stable channel from a nested directory, then repeats
    // frozen without traffic.
    journey.setup("crates/app", LOCAL);
    journey.setup("", &["--frozen", "--tools-only"]);
    let bin = journey.bin();
    assert!(bin.join("cargo").exists() && bin.join("rustc").exists());
    journey.calls();

    let toolchain = format!("toolchain=1.99.0-{}", triple());
    journey.runs_managed(
        "crates/app",
        &["build"],
        &["managed-cargo build", &toolchain, "cargo_home=cargo-home"],
    );
    // turbo's own Cargo discovery and compiler identity used the managed tools.
    let calls = journey.calls();
    for probe in ["cargo metadata --format-version 1", "rustc -vV"] {
        assert!(calls.contains(probe), "{probe}: {calls}");
    }

    fs::remove_file(bin.join("cargo")).unwrap();
    journey.fails("build", "run turbo setup to repair the installation");
    journey.setup("", &["--frozen", "--tools-only"]);
    let file = journey.root().join("rust-toolchain.toml");
    fs::write(&file, TOOLCHAIN.replace("stable", "1.98")).unwrap();
    journey.fails("build", "run turbo setup to update turbo.lock");
    fs::write(&file, TOOLCHAIN).unwrap();
    let (code, text) = journey.turbo("", &["run", "build", "--ui=stream"]);
    assert_eq!(code, 0, "{text}");
}
