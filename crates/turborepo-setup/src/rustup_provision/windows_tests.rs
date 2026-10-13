#![allow(clippy::unwrap_used)]
//! Native Windows rustup bootstrap: verified `rustup-init.exe` bytes become one
//! hard-linked `rustup.exe` export. Nothing here runs an installer.

use std::ffi::OsStr;

use sha2::{Digest, Sha256};

use super::*;
use crate::{
    Preparation::{Fresh, IfNeeded},
    source_policy::test_support::{LoopbackServer, OwnedSetupFixture},
};

const INSTALLER: &[u8] = b"MZ stand-in rustup-init.exe, never executed";

/// Official release metadata and published checksum spellings. Every route
/// serves `installer`, while the checksums always name INSTALLER.
fn serve(installer: &[u8]) -> (LoopbackServer, RustupTransport) {
    let digest = format!("{:x}", Sha256::digest(INSTALLER));
    let release = b"schema-version = '1'\nversion = '1.29.1'\n".to_vec();
    let mut routes = vec![("/rustup/release-stable.toml".to_owned(), release)];
    for platform in rustup::PLATFORMS {
        let url = rustup::installer_url(&semver::Version::new(1, 29, 1), platform).unwrap();
        let path = url.strip_prefix(ORIGIN).unwrap().to_owned();
        let checksum = if rustup::windows(platform) {
            format!("{digest}\r\n")
        } else {
            format!("{digest} *./rustup-init\n")
        };
        routes.push((format!("{path}.sha256"), checksum.into_bytes()));
        routes.push((path, installer.to_vec()));
    }
    let server = LoopbackServer::new(routes, |_| {}).unwrap();
    let transport = RustupTransport::loopback_http_for_tests(server.origin()).unwrap();
    (server, transport)
}

async fn locked(transport: &RustupTransport) -> Lock {
    let declarations = vec![Declaration {
        file: "rust-toolchain.toml".into(),
        field: None,
        request: None,
    }];
    let tool = transport.resolve(declarations).await.unwrap();
    Lock::new(lock::Document {
        schema_version: lock::SCHEMA_VERSION,
        tools: BTreeMap::from([("rustup".into(), tool)]),
    })
    .unwrap()
}

#[tokio::test]
async fn windows_x64_bootstrap_exports_one_hard_linked_rustup_exe() {
    let (server, transport) = serve(INSTALLER);
    let plan = RustupPlan::from_lock(&locked(&transport).await, Platform::WindowsX64).unwrap();
    assert_eq!(plan.inventory_tool().platform, "windows-x64");
    assert_eq!(
        plan.inventory_tool().executables["rustup"],
        "bin/rustup.exe"
    );
    let desired = vec![plan.inventory_tool().clone()];
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let prepared = plan.prepare(&store, &desired, &transport, IfNeeded);
    let prepared = prepared.await.unwrap().unwrap();
    store
        .reconcile(&desired, |tool, path| prepared.stage(tool, path))
        .unwrap();
    let current = store.current().unwrap().unwrap();
    let names: Vec<_> = fs::read_dir(&current.bin)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, ["rustup.exe"]);
    let tree = current.tool_tree(plan.inventory_tool()).unwrap();
    let binary = tree.join("bin").join("rustup.exe");
    assert!(same_file::is_same_file(current.bin.join("rustup.exe"), &binary).unwrap());
    assert_eq!(fs::read(&binary).unwrap(), INSTALLER);
    // Only the binary: no homes, proxies or toolchains.
    assert_eq!(fs::read_dir(&tree).unwrap().count(), 1);

    // A healthy repeat downloads nothing; a corrupt fresh one keeps the
    // published generation selected.
    let hits = server.hits();
    let reused = plan.prepare(&store, &desired, &transport, IfNeeded).await;
    assert!(reused.unwrap().is_none());
    assert_eq!(server.hits(), hits);
    let (_corrupt, corrupt) = serve(b"corrupt");
    assert!(matches!(
        plan.prepare(&store, &desired, &corrupt, Fresh).await,
        Err(Error::Download(turborepo_download::Error::DigestMismatch))
    ));
    assert!(store.is_current(&desired).unwrap());
}

#[tokio::test]
async fn windows_arm64_and_unix_installers_are_locked_but_never_promoted() {
    let (server, transport) = serve(INSTALLER);
    let lock = locked(&transport).await;
    let repo = tempfile::tempdir().unwrap();
    let store = Store::open(repo.path()).unwrap();
    let hits = server.hits();
    for platform in [
        Platform::WindowsArm64,
        Platform::LinuxX64Gnu,
        Platform::MacosArm64,
    ] {
        let plan = RustupPlan::from_lock(&lock, platform).unwrap();
        let desired = [plan.inventory_tool().clone()];
        let prepared = plan.prepare(&store, &desired, &transport, IfNeeded).await;
        assert!(
            matches!(prepared, Err(Error::UnsupportedTarget)),
            "{platform:?}"
        );
    }
    assert_eq!(server.hits(), hits);
    assert!(store.current().unwrap().is_none());
}

/// A canonical store is `\\?\`-prefixed; rustup gets the ordinary spelling.
#[test]
fn private_homes_reach_rustup_with_ordinary_paths() {
    let fixture = OwnedSetupFixture::new().unwrap();
    let parent = tempfile::tempdir().unwrap();
    let verbatim = fs::canonicalize(parent.path()).unwrap();
    assert!(verbatim.to_string_lossy().starts_with(r"\\?\"));
    let homes = PrivateHomes::create(&verbatim).unwrap();
    let transport = RustupTransport::official(&fixture.policy().unwrap()).unwrap();
    let command = homes.command(Path::new("rustup.exe"), &transport);
    let envs: BTreeMap<_, _> = command.get_envs().collect();
    for (name, home) in [
        ("RUSTUP_HOME", homes.rustup_home()),
        ("CARGO_HOME", homes.cargo_home()),
    ] {
        let value = Path::new(envs[OsStr::new(name)].unwrap());
        assert!(!value.to_string_lossy().starts_with(r"\\?\"), "{name}");
        assert!(same_file::is_same_file(value, home).unwrap(), "{name}");
    }
    assert_eq!(envs[OsStr::new("RUSTUP_TOOLCHAIN")], None);
    assert_eq!(
        envs[OsStr::new("RUSTUP_AUTO_INSTALL")],
        Some(OsStr::new("0"))
    );
    assert!(!envs.contains_key(OsStr::new("PATH")));
}
