#![allow(clippy::unwrap_used)]
//! Native Windows toolchain installs. A batch stand-in for the private rustup
//! (the setup command runs the staged `rustup.exe`) lays out the toolchain the
//! way the real one does; every export is a hard link to the tree's proxy.

use sha2::{Digest, Sha256};

use super::*;
use crate::{
    Preparation::{Fresh, IfNeeded},
    rust_metadata::tests::fixture,
    rust_resolution,
};

const HOST: &str = "x86_64-pc-windows-msvc";
const REQUEST: &str = "[toolchain]\nchannel = \"1.99\"\nprofile = \"minimal\"\ncomponents = \
                       [\"clippy\"]\ntargets = [\"wasm32-unknown-unknown\"]\n";
const COMPONENTS: [&str; 5] = [
    "cargo-x86_64-pc-windows-msvc",
    "clippy-preview-x86_64-pc-windows-msvc",
    "rust-std-wasm32-unknown-unknown",
    "rust-std-x86_64-pc-windows-msvc",
    "rustc-x86_64-pc-windows-msvc",
];
const BINARIES: &str = "cargo cargo-clippy clippy-driver rustc rustdoc";

/// The official-shaped fixture with its Linux host published as `host`.
fn manifest(version: &str, host: &str) -> String {
    fixture(version, "2026-10-01").replace("x86_64-unknown-linux-gnu", host)
}

/// Locked Rust for `manifest`, plus a rustup whose Windows installers are
/// `rustup` (the bytes every proxy must copy).
fn lock(rustup: &[u8], manifest: &str, with_rustup: bool) -> Lock {
    let toolchain = Toolchain::parse("rust-toolchain.toml", REQUEST).unwrap();
    let rust = rust_resolution::resolve(&toolchain, manifest.as_bytes()).unwrap();
    let mut tools = BTreeMap::from([("rust".to_owned(), rust.clone())]);
    let version = semver::Version::new(1, 29, 1);
    let artifacts = [Platform::WindowsX64, Platform::WindowsArm64].map(|platform| {
        let part = Artifact {
            url: rustup::installer_url(&version, platform).unwrap(),
            sha256: format!("{:x}", Sha256::digest(rustup)),
            format: Format::Binary,
            root_prefix: None,
            destination: None,
            executables: BTreeMap::from([("rustup".into(), "bin/rustup.exe".into())]),
            alternates: BTreeMap::new(),
        };
        (platform, BTreeMap::from([("rustup-init".to_owned(), part)]))
    });
    let installer = lock::Tool {
        adapter: "rustup".into(),
        version: version.to_string(),
        declarations: rust.declarations,
        options: BTreeMap::new(),
        installation: Installation::Managed {
            artifacts: artifacts.into(),
        },
    };
    if with_rustup {
        tools.insert("rustup".into(), installer);
    }
    Lock::new(lock::Document {
        schema_version: lock::SCHEMA_VERSION,
        tools,
    })
    .unwrap()
}

/// A stand-in rustup: logs its invocation, then lays out `binaries`, the
/// installed component list and rustup's manifest copy for the host.
fn rustup(dir: &Path, binaries: &str, components: &[&str], manifest: &str, status: i32) -> PathBuf {
    let copy = dir.join("manifest.toml");
    fs::write(&copy, manifest).unwrap();
    let mut script = format!(
        "@echo off\r\n>>\"{log}\" echo \
         %*;%RUSTUP_HOME%;%CARGO_HOME%;%RUSTUP_DIST_SERVER%;%RUSTUP_UPDATE_ROOT%\r\nif not \
         \"{status}\"==\"0\" exit /b {status}\r\nset \
         \"tc=%RUSTUP_HOME%\\toolchains\\%3-{HOST}\"\r\nmkdir \"%tc%\\bin\" \
         \"%tc%\\lib\\rustlib\"\r\nfor %%e in ({binaries}) do >\"%tc%\\bin\\%%e.exe\" echo \
         %%e\r\ncopy /y \"{copy}\" \"%tc%\\lib\\rustlib\\multirust-channel-manifest.toml\" \
         >nul\r\n>\"%RUSTUP_HOME%\\settings.toml\" echo version = \"12\"\r\n(\r\n",
        log = dir.join("log").display(),
        copy = copy.display(),
    );
    for component in components {
        script.push_str(&format!("echo {component}\r\n"));
    }
    script.push_str(") >\"%tc%\\lib\\rustlib\\components\"\r\n");
    let path = dir.join("rustup.cmd");
    fs::write(&path, script).unwrap();
    path
}

fn official() -> RustupTransport {
    let fixture = crate::source_policy::test_support::OwnedSetupFixture::new().unwrap();
    RustupTransport::official(&fixture.policy().unwrap()).unwrap()
}

fn calls(dir: &Path) -> Vec<String> {
    fs::read_to_string(dir.join("log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn windows_toolchains_install_privately_and_export_rustup_proxies() {
    let (repo, fixtures) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let manifest = manifest("1.99.0", HOST);
    let binary = rustup(fixtures.path(), BINARIES, &COMPONENTS, &manifest, 0);
    let lock = lock(&fs::read(&binary).unwrap(), &manifest, true);
    let plan = RustPlan::from_lock(&lock, Platform::WindowsX64).unwrap();
    let tool = plan.inventory_tool();
    assert!(
        tool.executables
            .values()
            .all(|export| export.as_file() == Some(PROXY))
    );
    assert_eq!(plan.toolchain(), format!("1.99.0-{HOST}"));
    let desired = vec![tool.clone()];
    let mut store = Store::open(repo.path()).unwrap();
    let transport = official();
    let prepared = plan
        .prepare(&store, &desired, &binary, &transport, IfNeeded)
        .unwrap()
        .unwrap();
    let [call] = calls(fixtures.path()).try_into().unwrap();
    let [args, rustup_home, cargo_home, dist, update] =
        call.split(';').collect::<Vec<_>>().try_into().unwrap();
    assert_eq!(
        args,
        "toolchain install 1.99.0 --no-self-update --profile minimal --component clippy --target \
         wasm32-unknown-unknown"
    );
    assert_eq!(
        (dist, update),
        (rustup::ORIGIN, "https://static.rust-lang.org/rustup")
    );
    // Both homes are fresh beside the Store, in their ordinary spelling.
    let tools = repo
        .path()
        .canonicalize()
        .unwrap()
        .join(".turbo")
        .join("tools");
    for home in [rustup_home, cargo_home] {
        assert!(
            Path::new(home).starts_with(dunce::simplified(&tools)),
            "{home}"
        );
        assert!(!home.starts_with(r"\\?\"), "{home}");
    }
    store
        .reconcile(&desired, |tool, path| prepared.stage(tool, path))
        .unwrap();
    drop(prepared);
    let current = store.current().unwrap().unwrap();
    let mut shims: Vec<_> = fs::read_dir(&current.bin)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    shims.sort();
    assert_eq!(
        shims,
        [
            "cargo-clippy.exe",
            "cargo.exe",
            "clippy-driver.exe",
            "rustc.exe",
            "rustdoc.exe"
        ]
    );
    // Every export is the verified proxy, never a toolchain binary.
    let tree = current.tool_tree(tool).unwrap();
    let proxy = tree.join("proxy").join("rustup.exe");
    assert_eq!(fs::read(&proxy).unwrap(), fs::read(&binary).unwrap());
    for shim in &shims {
        assert!(same_file::is_same_file(current.bin.join(shim), &proxy).unwrap());
    }
    let toolchain = tree.join("toolchains").join(plan.toolchain());
    assert_eq!(
        fs::read(toolchain.join("bin").join("rustc.exe")).unwrap(),
        b"rustc\r\n"
    );
    assert!(tree.join("settings.toml").is_file());
    let leftovers = fs::read_dir(&tools).unwrap().filter(|entry| {
        let name = entry.as_ref().unwrap().file_name();
        name.to_string_lossy().starts_with(".rust-")
    });
    assert_eq!(leftovers.count(), 0);

    assert!(
        plan.prepare(&store, &desired, &binary, &transport, IfNeeded)
            .unwrap()
            .is_none()
    );
    assert_eq!(calls(fixtures.path()).len(), 1);
    assert!(
        plan.prepare(&store, &desired, &binary, &transport, Fresh)
            .unwrap()
            .is_some()
    );
    assert_eq!(calls(fixtures.path()).len(), 2);
}

#[test]
fn failed_mismatched_or_unverified_installs_publish_nothing() {
    let repo = tempfile::tempdir().unwrap();
    let store = Store::open(repo.path()).unwrap();
    let locked = manifest("1.99.0", HOST);
    let newer = manifest("1.99.1", HOST);
    let missing = "cargo cargo-clippy clippy-driver rustc";
    for (binaries, components, manifest, status, proxy) in [
        (BINARIES, &COMPONENTS[..], &locked, 1, true),
        (BINARIES, &COMPONENTS[..4], &locked, 0, true),
        (BINARIES, &COMPONENTS[..], &newer, 0, true),
        // A locked proxy target that rustup did not install.
        (missing, &COMPONENTS[..], &locked, 0, true),
        // The proxy must copy exactly the locked rustup.
        (BINARIES, &COMPONENTS[..], &locked, 0, false),
    ] {
        let fixtures = tempfile::tempdir().unwrap();
        let binary = rustup(fixtures.path(), binaries, components, manifest, status);
        let bytes = if proxy {
            fs::read(&binary).unwrap()
        } else {
            b"another rustup".to_vec()
        };
        let plan = RustPlan::from_lock(&lock(&bytes, &locked, true), Platform::WindowsX64);
        let plan = plan.unwrap();
        let desired = [plan.inventory_tool().clone()];
        let result = plan.prepare(&store, &desired, &binary, &official(), IfNeeded);
        assert!(
            matches!(result, Err(Error::Rustup(_) | Error::Mismatch)),
            "{binaries} {components:?} {proxy}"
        );
        assert_eq!(calls(fixtures.path()).len(), 1);
    }
    assert!(store.current().unwrap().is_none());
}

#[test]
fn windows_plans_bind_the_locked_rustup_and_never_install_arm64_or_unix() {
    let x64 = manifest("1.99.0", HOST);
    let one = RustPlan::from_lock(&lock(b"one", &x64, true), Platform::WindowsX64).unwrap();
    let two = RustPlan::from_lock(&lock(b"two", &x64, true), Platform::WindowsX64).unwrap();
    assert_ne!(
        one.inventory_tool().artifact_sha256,
        two.inventory_tool().artifact_sha256
    );
    assert!(matches!(
        RustPlan::from_lock(&lock(b"one", &x64, false), Platform::WindowsX64),
        Err(Error::InvalidLock)
    ));

    let fixtures = tempfile::tempdir().unwrap();
    let binary = rustup(fixtures.path(), BINARIES, &COMPONENTS, &x64, 0);
    let arm64 = manifest("1.99.0", "aarch64-pc-windows-msvc");
    let repo = tempfile::tempdir().unwrap();
    let store = Store::open(repo.path()).unwrap();
    for (manifest, platform) in [
        (&arm64, Platform::WindowsArm64),
        (&x64, Platform::MacosArm64),
    ] {
        let plan = RustPlan::from_lock(&lock(b"one", manifest, true), platform).unwrap();
        let desired = [plan.inventory_tool().clone()];
        let result = plan.prepare(&store, &desired, &binary, &official(), IfNeeded);
        assert!(
            matches!(result, Err(Error::UnsupportedTarget)),
            "{platform:?}"
        );
    }
    assert!(calls(fixtures.path()).is_empty());
}
