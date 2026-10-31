#![allow(clippy::unwrap_used)]

use std::process::Command;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::*;
use crate::{
    package_manager::discover_package_manager,
    source_policy::test_support::{LoopbackServer, stored_zip as zip},
    test_store::TestStore,
};

const VERSION: &str = "1.2.23";
const RELEASE: &str = "/oven-sh/bun/releases/download/bun-v1.2.23";
// Prints the invoked name and arguments: `bunx` is the same binary.
const SCRIPT: &str = "#!/bin/sh\nprintf '%s:' \"${0##*/}\"\nprintf '<%s>' \"$@\"\n";

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn routes(files: &[(&str, Vec<u8>)], base: &str) -> Vec<(String, Vec<u8>)> {
    let manifest: String = files
        .iter()
        .map(|(name, bytes)| format!("{}  {name}\n", sha256(bytes)))
        .collect();
    let mut routes = vec![(
        format!("{base}{RELEASE}/SHASUMS256.txt"),
        manifest.into_bytes(),
    )];
    routes.extend(
        files
            .iter()
            .map(|(name, bytes)| (format!("{base}{RELEASE}/{name}"), bytes.clone())),
    );
    routes
}
fn serve(files: &[(&str, Vec<u8>)]) -> (LoopbackServer, BunTransport) {
    let server = LoopbackServer::new(routes(files, ""), |_| {}).unwrap();
    let transport = BunTransport::loopback_http_for_tests(server.origin()).unwrap();
    (server, transport)
}
fn lock(installation: &Installation, request: &str) -> Lock {
    let value = json!({"schemaVersion":0,"tools":{"bun":{"adapter":"bun","version":VERSION,
        "declarations":[{"file":"package.json","field":"/packageManager","request":request}],
        "installation":installation}}});
    Lock::parse(&serde_json::to_vec(&value).unwrap()).unwrap()
}
fn declaration(request: &str) -> Declaration {
    discover_package_manager(&json!({ "packageManager": request }))
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn releases_lock_every_published_supported_target_and_nothing_else() {
    // Every standard target plus variants setup must never select.
    let published: Vec<_> = "darwin-aarch64 darwin-x64 darwin-x64-baseline linux-aarch64 \
                             linux-aarch64-musl linux-x64 linux-x64-baseline \
                             linux-x64-musl-baseline linux-x64-profile freebsd-x64 windows-x64 \
                             windows-x64-baseline windows-aarch64"
        .split_whitespace()
        .map(|target| (format!("bun-{target}.zip"), target.as_bytes().to_vec()))
        .collect();
    let files: Vec<_> = published
        .iter()
        .map(|(n, b)| (n.as_str(), b.clone()))
        .collect();
    let (server, transport) = serve(&files);
    let installation = transport.resolve_exact(VERSION).await.unwrap();
    assert_eq!(server.requests(), [format!("{RELEASE}/SHASUMS256.txt")]);
    let Installation::Managed { artifacts } = &installation else {
        panic!("managed");
    };
    let selected: Vec<_> = artifacts
        .iter()
        .map(|(platform, parts)| {
            let artifact = &parts["distribution"];
            assert_eq!(
                artifact,
                &official(*platform, VERSION, &artifact.sha256).unwrap()
            );
            (
                serde_json::to_value(platform).unwrap(),
                artifact.root_prefix.clone().unwrap(),
            )
        })
        .collect();
    let expected: Vec<(Value, String)> = [
        ("macos-x64", "darwin-x64-baseline"),
        ("macos-arm64", "darwin-aarch64"),
        ("linux-x64-gnu", "linux-x64-baseline"),
        ("linux-arm64-gnu", "linux-aarch64"),
        ("windows-x64", "windows-x64-baseline"),
        ("windows-arm64", "windows-aarch64"),
    ]
    .map(|(platform, target)| (json!(platform), format!("bun-{target}")))
    .into();
    assert_eq!(selected, expected);
    let windows = &artifacts[&Platform::WindowsX64]["distribution"];
    assert_eq!(windows.sha256, sha256(b"windows-x64-baseline"));
    assert_eq!(windows.executables["bunx"], "bun.exe");
    // The locked shape is a valid portable lock entry, executables included.
    lock(&installation, "bun@1.2.23");

    // Older releases omit unpublished targets; none at all is an error.
    let (_older, transport) = serve(&files[..9]);
    let Installation::Managed { artifacts } = transport.resolve_exact(VERSION).await.unwrap()
    else {
        panic!("managed");
    };
    assert!(!artifacts.contains_key(&Platform::WindowsX64));
    assert_eq!(artifacts.len(), 4);
    let (_musl, transport) = serve(&[("bun-linux-x64-musl.zip", vec![1])]);
    assert!(matches!(
        transport.resolve_exact(VERSION).await,
        Err(Error::Checksums)
    ));
    for version in ["v1.2.23", "1.2", "latest", "1.2.23+build", "01.2.3"] {
        assert!(matches!(
            transport.resolve_exact(version).await,
            Err(Error::InvalidVersion)
        ));
    }
}

#[tokio::test]
async fn locked_bun_and_bunx_shims_run_the_verified_release_binary() {
    for platform in [Platform::MacosArm64, Platform::LinuxX64Gnu] {
        let target = target(platform).unwrap();
        let archive = zip(&format!("bun-{target}/bun"), SCRIPT.as_bytes());
        let name = format!("bun-{target}.zip");
        let (server, transport) = serve(&[(&name, archive)]);
        let installation = transport.resolve_exact(VERSION).await.unwrap();
        let lock = lock(&installation, "bun@1.2.23");
        let bun = BunPlan::from_declaration(&lock, platform, &declaration("bun@1.2.23")).unwrap();
        let repo = tempfile::tempdir().unwrap();
        let store = TestStore::open(repo.path());
        let desired = [bun.inventory_tool().clone()];
        let prepared = bun
            .prepare_if_needed(&store, &desired, &transport)
            .await
            .unwrap()
            .unwrap();
        store
            .reconcile(&desired, |tool, path| prepared.stage(tool, path))
            .unwrap();
        let bin = store.bin();
        for (name, args, expected) in [
            (
                "bun",
                &["install", "--frozen-lockfile"][..],
                "bun:<install><--frozen-lockfile>",
            ),
            ("bunx", &["cowsay"][..], "bunx:<cowsay>"),
        ] {
            let output = Command::new(bin.join(name)).args(args).output().unwrap();
            assert!(output.status.success());
            assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
        }
        let hits = server.hits();
        let reused = bun.prepare_if_needed(&store, &desired, &transport).await;
        assert!(reused.unwrap().is_none());
        assert_eq!(server.hits(), hits);
    }
}

/// A mirror serves the official release paths beneath its base. The lock keeps
/// the official URLs, and the downloaded archive is still verified.
#[tokio::test]
async fn mirrors_serve_official_release_paths_beneath_their_base() {
    let target = target(Platform::LinuxX64Gnu).unwrap();
    let name = format!("bun-{target}.zip");
    let archive = zip(&format!("bun-{target}/bun"), SCRIPT.as_bytes());
    let routes = routes(&[(&name, archive)], "/mirror");
    let server = LoopbackServer::new(routes, |_| {}).unwrap();
    let fixture = crate::source_policy::test_support::OwnedSetupFixture::new().unwrap();
    let base = format!("{}/mirror/", server.origin());
    let mirror = crate::mirror::Mirror::loopback_for_tests(&base).unwrap();
    let transport = BunTransport::mirrored(&fixture.policy().unwrap(), &mirror).unwrap();
    let installation = transport.resolve_exact(VERSION).await.unwrap();
    let lock = lock(&installation, "bun@1.2.23");
    let bun = BunPlan::from_lock(&lock, Platform::LinuxX64Gnu).unwrap();
    assert!(
        bun.artifact
            .url
            .starts_with("https://github.com/oven-sh/bun/")
    );
    let repo = tempfile::tempdir().unwrap();
    let store = TestStore::open(repo.path());
    let desired = [bun.inventory_tool().clone()];
    let prepared = bun.prepare_if_needed(&store, &desired, &transport).await;
    assert!(prepared.unwrap().is_some());
    let expected = ["SHASUMS256.txt", &name].map(|file| format!("/mirror{RELEASE}/{file}"));
    assert_eq!(server.requests(), expected);
}

#[tokio::test]
async fn tampered_locks_hosts_declarations_and_bytes_fail_before_publication() {
    let archive = zip("bun-darwin-aarch64/bun", SCRIPT.as_bytes());
    let (server, transport) = serve(&[("bun-darwin-aarch64.zip", archive)]);
    let installation = transport.resolve_exact(VERSION).await.unwrap();
    let valid = lock(&installation, "bun@1.2.23");
    let target = Platform::MacosArm64;
    for platform in [Platform::WindowsX64, Platform::LinuxX64Musl] {
        let result = BunPlan::from_lock(&valid, platform);
        assert!(matches!(result, Err(Error::UnsupportedTarget)));
    }
    assert!(matches!(
        BunPlan::from_lock(&valid, Platform::LinuxArm64Gnu),
        Err(Error::MissingTarget)
    ));
    let artifact = "/installation/artifacts/macos-arm64/distribution";
    for (pointer, value) in [
        (
            "/url",
            json!(format!("https://github.com{RELEASE}/bun-darwin-x64.zip")),
        ),
        ("/rootPrefix", json!("bun-darwin-x64")),
        ("/executables", json!({"bun":"bun"})),
        (
            "/executables",
            json!({"bun":"bun","bunx":"bun","node":"bun"}),
        ),
        ("/format", json!("tar-gz")),
    ] {
        let mut doc = serde_json::to_value(valid.document()).unwrap();
        let field = format!("{artifact}{pointer}");
        *doc["tools"]["bun"].pointer_mut(&field).unwrap() = value;
        let lock = Lock::parse(&serde_json::to_vec(&doc).unwrap()).unwrap();
        assert!(matches!(
            BunPlan::from_lock(&lock, target),
            Err(Error::InvalidLock)
        ));
    }
    for request in ["bun@1.2.22", "pnpm@10.0.0"] {
        assert!(BunPlan::from_declaration(&valid, target, &declaration(request)).is_err());
    }
    let pinned = format!("bun@1.2.23+sha512.{}", "a".repeat(128));
    let error = discover_package_manager(&json!({ "packageManager": pinned }));
    assert!(
        error
            .unwrap_err()
            .to_string()
            .contains("Bun has no Corepack integrity")
    );

    // A wrong locked digest, then verified bytes with a different layout.
    let repo = tempfile::tempdir().unwrap();
    let store = TestStore::open(repo.path());
    let relocated = zip("bun-linux-x64/bun", SCRIPT.as_bytes());
    let (_other, relocated_transport) = serve(&[("bun-darwin-aarch64.zip", relocated.clone())]);
    for (digest, transport, layout) in [
        ("aa".repeat(32), &transport, false),
        (sha256(&relocated), &relocated_transport, true),
    ] {
        let mut doc = serde_json::to_value(valid.document()).unwrap();
        doc["tools"]["bun"].pointer_mut(artifact).unwrap()["sha256"] = json!(digest);
        let lock = Lock::parse(&serde_json::to_vec(&doc).unwrap()).unwrap();
        let bun = BunPlan::from_lock(&lock, target).unwrap();
        let desired = [bun.inventory_tool().clone()];
        let result = bun.prepare_if_needed(&store, &desired, transport).await;
        assert!(match result {
            Err(Error::Archive(_)) => layout,
            Err(Error::Download(turborepo_download::Error::DigestMismatch)) => !layout,
            _ => false,
        });
    }
    assert_eq!(server.hits(), 2);
    assert!(store.current().unwrap().is_none());
}
