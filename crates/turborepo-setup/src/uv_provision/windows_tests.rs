#![allow(clippy::unwrap_used)]
//! Native Windows x64 uv: the verified flat zip, `.exe` hard-link exports
//! found through PATH/PATHEXT, reuse and tamper refusal.

use std::{collections::BTreeMap, path::PathBuf, process::Command};

use semver::Version;
use sha2::{Digest, Sha256};

use super::*;
use crate::{
    lock::{Declaration, Document},
    source_policy::test_support::{LoopbackServer, stored_zip_entries},
    test_store::{Outcome, TestStore},
};

fn system32() -> PathBuf {
    PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32")
}
fn official(platform: Platform) -> UvArtifact {
    UvArtifact::for_platform(&Version::new(0, 13, 0), platform).unwrap()
}
// A real PE image stands in for uv.exe and uvx.exe so exports are runnable.
fn zip() -> Vec<u8> {
    let cmd = fs::read(system32().join("cmd.exe")).unwrap();
    stored_zip_entries(&[
        ("uv.exe", &cmd[..]),
        ("uvw.exe", b"windowed uv resource"),
        ("uvx.exe", &cmd[..]),
    ])
}
fn lock(platform: Platform, bytes: &[u8]) -> Lock {
    let artifact = official(platform).lock_artifact(format!("{:x}", Sha256::digest(bytes)));
    Lock::new(Document {
        schema_version: lock::SCHEMA_VERSION,
        tools: BTreeMap::from([(
            "uv".into(),
            lock::Tool {
                adapter: "uv".into(),
                version: "0.13.0".into(),
                declarations: vec![Declaration {
                    file: "uv.toml".into(),
                    field: Some("required-version".into()),
                    request: Some(">=0.13".into()),
                }],
                options: BTreeMap::new(),
                installation: Installation::Managed {
                    artifacts: BTreeMap::from([(
                        platform,
                        BTreeMap::from([("distribution".into(), artifact)]),
                    )]),
                },
            },
        )]),
    })
    .unwrap()
}
fn serve(bytes: Vec<u8>) -> (LoopbackServer, UvTransport) {
    let path = official(Platform::WindowsX64).url()[uv::ORIGIN.len()..].to_owned();
    let upstream = LoopbackServer::new([(path, bytes)], |_| {}).unwrap();
    let transport = UvTransport::loopback_http_for_tests(upstream.origin()).unwrap();
    (upstream, transport)
}
async fn install(
    store: &TestStore,
    plan: &UvPlan,
    transport: &UvTransport,
) -> Result<Outcome, Error> {
    let desired = [plan.inventory_tool().clone()];
    let prepared = plan
        .prepare(store, &desired, transport, crate::Preparation::IfNeeded)
        .await?;
    Ok(store.reconcile(&desired, |tool, path| {
        prepared
            .as_ref()
            .ok_or(turborepo_tool_install::Error::InvalidInventory)?
            .stage(tool, path)
    })?)
}
fn exit_code(bin: &Path, line: &str) -> Option<i32> {
    Command::new(system32().join("cmd.exe"))
        .args(["/d", "/c", line])
        .env("PATH", dunce::simplified(bin))
        .env("PATHEXT", ".COM;.EXE;.BAT;.CMD")
        .status()
        .unwrap()
        .code()
}

#[tokio::test(flavor = "current_thread")]
async fn verified_zip_exports_uv_and_uvx_as_hard_links_and_repeats_reuse() {
    let bytes = zip();
    let (upstream, transport) = serve(bytes.clone());
    let plan =
        UvPlan::from_lock(&lock(Platform::WindowsX64, &bytes), Platform::WindowsX64).unwrap();
    let repo = tempfile::tempdir().unwrap();
    let store = TestStore::open(repo.path());
    assert_eq!(
        install(&store, &plan, &transport).await.unwrap(),
        Outcome::Replaced
    );
    let current = store.current().unwrap().unwrap();
    let tree = store.tree_of(plan.inventory_tool());
    let mut names: Vec<_> = fs::read_dir(&current.bin)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["uv.exe", "uvx.exe"]);
    for name in ["uv.exe", "uvx.exe"] {
        assert!(same_file::is_same_file(current.bin.join(name), tree.join(name)).unwrap());
    }
    // The full tree is kept, with no launcher directory beside it.
    let mut files: Vec<_> = fs::read_dir(&tree)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    files.sort();
    assert_eq!(files, ["uv.exe", "uvw.exe", "uvx.exe"]);
    // Direct process creation and PATHEXT lookups both reach the exports.
    assert_eq!(exit_code(&current.bin, "uv /d /c exit 5"), Some(5));
    assert_eq!(exit_code(&current.bin, "uvx /d /c exit 6"), Some(6));
    let status = Command::new(current.bin.join("uv.exe"))
        .args(["/d", "/c", "exit 7"])
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(7));
    assert_eq!(
        install(&store, &plan, &transport).await.unwrap(),
        Outcome::Unchanged
    );
    assert_eq!(upstream.hits(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn tampered_zips_and_unix_plans_never_publish_on_windows() {
    let (upstream, transport) = serve(stored_zip_entries(&[
        ("uv.exe", &b"x"[..]),
        ("uvx.exe", b"x"),
    ]));
    let plan =
        UvPlan::from_lock(&lock(Platform::WindowsX64, &zip()), Platform::WindowsX64).unwrap();
    let repo = tempfile::tempdir().unwrap();
    let store = TestStore::open(repo.path());
    assert!(matches!(
        install(&store, &plan, &transport).await,
        Err(Error::Download(turborepo_download::Error::DigestMismatch))
    ));
    assert_eq!(upstream.hits(), 1);
    // A Unix launcher plan is never staged on Windows, before any download.
    let unix =
        UvPlan::from_lock(&lock(Platform::LinuxX64Gnu, b"tar"), Platform::LinuxX64Gnu).unwrap();
    assert!(matches!(
        install(&store, &unix, &transport).await,
        Err(Error::UnsupportedTarget)
    ));
    assert_eq!(upstream.hits(), 1);
    assert!(store.current().unwrap().is_none());
    assert!(matches!(
        UvPlan::from_lock(
            &lock(Platform::WindowsArm64, b"zip"),
            Platform::WindowsArm64
        ),
        Err(Error::UnsupportedTarget)
    ));
}
