#![allow(clippy::unwrap_used)]

use std::{ffi::OsStr, os::unix::fs::PermissionsExt, process::Command};

use sha2::{Digest, Sha256};

use super::*;
use crate::source_policy::test_support::{LoopbackServer, OwnedSetupFixture};

const TARGET: Platform = Platform::LinuxX64Gnu;
// Never the real installer: it reports the multicall name and its homes.
const RUSTUP: &str =
    "#!/bin/sh\nprintf '%s|%s|%s|%s\\n' \"${0##*/}\" \"$RUSTUP_HOME\" \"$CARGO_HOME\" \"$*\"\n";

fn routes(
    installer: &str,
    checksum: impl Fn(Platform) -> String,
    base: &str,
) -> Vec<(String, Vec<u8>)> {
    let mut routes = vec![(
        format!("{base}/rustup/release-stable.toml"),
        b"schema-version = '1'\nversion = '1.29.1'\n".to_vec(),
    )];
    for platform in rustup::PLATFORMS {
        let url = rustup::installer_url(&semver::Version::new(1, 29, 1), platform).unwrap();
        let path = format!("{base}{}", url.strip_prefix(ORIGIN).unwrap());
        routes.push((format!("{path}.sha256"), checksum(platform).into_bytes()));
        routes.push((path, installer.as_bytes().to_vec()));
    }
    routes
}

fn serve(
    installer: &str,
    checksum: impl Fn(Platform) -> String,
) -> (LoopbackServer, RustupTransport) {
    let server = LoopbackServer::new(routes(installer, checksum, ""), |_| {}).unwrap();
    let transport = RustupTransport::loopback_http_for_tests(server.origin()).unwrap();
    (server, transport)
}

// Published spellings: Unix names the file, Windows prints only the digest.
fn official(platform: Platform) -> String {
    let digest = format!("{:x}", Sha256::digest(RUSTUP));
    if rustup::windows(platform) {
        format!("{digest}\n")
    } else {
        format!("{digest} *./{}\n", rustup::installer_file(platform))
    }
}

fn declarations() -> Vec<Declaration> {
    vec![Declaration {
        file: "rust-toolchain.toml".into(),
        field: None,
        request: None,
    }]
}

async fn resolved() -> (LoopbackServer, RustupTransport, Lock) {
    let (server, transport) = serve(RUSTUP, official);
    let tool = transport.resolve(declarations()).await.unwrap();
    (server, transport, single(tool))
}

fn single(tool: lock::Tool) -> Lock {
    Lock::new(lock::Document {
        schema_version: lock::SCHEMA_VERSION,
        tools: BTreeMap::from([("rustup".into(), tool)]),
    })
    .unwrap()
}

#[tokio::test]
async fn resolution_records_every_native_platform_from_official_checksums() {
    let (server, _, lock) = resolved().await;
    let tool = &lock.tools()["rustup"];
    assert_eq!(tool.version, "1.29.1");
    let Installation::Managed { artifacts } = &tool.installation else {
        panic!("managed rustup");
    };
    assert_eq!(artifacts.len(), 8);
    let windows = &artifacts[&Platform::WindowsArm64][PART];
    assert!(
        windows
            .url
            .ends_with("/1.29.1/aarch64-pc-windows-msvc/rustup-init.exe")
    );
    assert_eq!(windows.executables["rustup"], "bin/rustup.exe");
    // Metadata only: resolution never downloads or runs an installer.
    assert!(
        server
            .requests()
            .iter()
            .all(|path| path.ends_with(".sha256") || path == "/rustup/release-stable.toml")
    );

    let (_bad, transport) = serve(RUSTUP, |platform| match platform {
        Platform::MacosX64 => "not a digest\n".into(),
        _ => official(platform),
    });
    assert!(matches!(
        transport.resolve(tool.declarations.clone()).await,
        Err(Error::Metadata(rustup::Error::Checksum))
    ));
}

#[tokio::test]
async fn locked_plans_accept_only_the_official_binary_shape() {
    let (_, _, lock) = resolved().await;
    let plan = RustupPlan::from_lock(&lock, TARGET).unwrap();
    assert_eq!(plan.inventory_tool().platform, "linux-x64-gnu");
    assert_eq!(
        plan.inventory_tool().executables["rustup"],
        "bin/rustup".into()
    );
    assert!(matches!(
        RustupPlan::from_lock(&lock, Platform::Any),
        Err(Error::UnsupportedTarget)
    ));
    fn part(tool: &mut lock::Tool) -> &mut BTreeMap<String, Artifact> {
        let Installation::Managed { artifacts } = &mut tool.installation else {
            unreachable!()
        };
        artifacts.get_mut(&TARGET).unwrap()
    }
    let mutations: [fn(&mut lock::Tool); 5] = [
        |tool| tool.version = "1.29".into(),
        |tool| tool.adapter = "generic".into(),
        |tool| {
            tool.options
                .insert("profile".into(), vec!["minimal".into()]);
        },
        |tool| {
            part(tool).get_mut(PART).unwrap().url =
                format!("{ORIGIN}/rustup/dist/x86_64-unknown-linux-gnu/rustup-init")
        },
        |tool| {
            let artifact = part(tool).get_mut(PART).unwrap();
            artifact
                .executables
                .insert("rustup".into(), "rustup-init".into());
        },
    ];
    for mutate in mutations {
        let mut tool = lock.tools()["rustup"].clone();
        mutate(&mut tool);
        let result = RustupPlan::from_lock(&single(tool), TARGET);
        assert!(matches!(result, Err(Error::InvalidLock)));
    }
    let mut tool = lock.tools()["rustup"].clone();
    let Installation::Managed { artifacts } = &mut tool.installation else {
        unreachable!()
    };
    artifacts.remove(&TARGET);
    assert!(matches!(
        RustupPlan::from_lock(&single(tool), TARGET),
        Err(Error::MissingTarget)
    ));
}

#[tokio::test]
async fn verified_bootstrap_publishes_one_shim_and_failures_keep_the_prior_generation() {
    let repo = tempfile::tempdir().unwrap();
    let manifest = repo.path().join(".turbo/tools/manifest.json");
    let (server, transport, lock) = resolved().await;
    let (_corrupt, corrupt) = serve("corrupt", official);
    let plan = RustupPlan::from_lock(&lock, TARGET).unwrap();
    let desired = vec![plan.inventory_tool().clone()];
    let mut store = Store::open(repo.path()).unwrap();
    use crate::Preparation::{Fresh, IfNeeded};
    assert!(matches!(
        plan.prepare(&store, &desired, &corrupt, IfNeeded).await,
        Err(Error::Download(turborepo_download::Error::DigestMismatch))
    ));
    assert!(!manifest.exists());

    let prepared = plan.prepare(&store, &desired, &transport, IfNeeded);
    let prepared = prepared.await.unwrap().unwrap();
    store
        .reconcile(&desired, |tool, path| prepared.stage(tool, path))
        .unwrap();
    let current = store.current().unwrap().unwrap();
    let shim = current.bin.join("rustup");
    assert_eq!(
        fs::read_link(&shim).unwrap(),
        Path::new("../tools/rustup/bin/rustup")
    );
    let tree = current.tool_tree(plan.inventory_tool()).unwrap();
    let mode = fs::metadata(tree.join("bin/rustup"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o555);
    // Only the binary: no homes, proxies or toolchains.
    assert_eq!(fs::read_dir(&tree).unwrap().count(), 1);
    assert_eq!(fs::read_dir(tree.join("bin")).unwrap().count(), 1);
    let output = Command::new(&shim).output().unwrap();
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .starts_with("rustup|")
    );

    let hits = server.hits();
    let reused = plan.prepare(&store, &desired, &transport, IfNeeded).await;
    assert!(reused.unwrap().is_none());
    assert_eq!(server.hits(), hits);
    let published = fs::read(&manifest).unwrap();
    let fresh = plan.prepare(&store, &desired, &corrupt, Fresh).await;
    assert!(fresh.is_err());
    assert_eq!(fs::read(&manifest).unwrap(), published);
    assert!(store.is_current(&desired).unwrap());
}

/// One mirror base serves rustup's release, checksum, and installer paths
/// and the channel manifests, and becomes the private rustup's distribution
/// server. The lock keeps the official URLs.
#[tokio::test]
async fn one_mirror_base_serves_every_rustup_and_dist_path() {
    let manifest = crate::rust_metadata::tests::fixture("1.99.0", "2026-10-01");
    let mut routes = routes(RUSTUP, official, "/mirror");
    routes.push((
        "/mirror/dist/channel-rust-1.99.toml".into(),
        manifest.into(),
    ));
    let server = LoopbackServer::new(routes, |_| {}).unwrap();
    let fixture = OwnedSetupFixture::new().unwrap();
    let base = format!("{}/mirror", server.origin());
    let mirror = crate::mirror::Mirror::loopback_for_tests(&format!("{base}/")).unwrap();
    let transport = RustupTransport::mirrored(&fixture.policy().unwrap(), &mirror).unwrap();
    let lock = single(transport.resolve(declarations()).await.unwrap());
    let request = "[toolchain]\nchannel = \"1.99\"\n";
    let toolchain = crate::rust_discovery::Toolchain::parse("rust-toolchain.toml", request);
    let rust = crate::rust_resolution::resolve_official(&transport, &toolchain.unwrap()).await;
    for tool in [&lock.tools()["rustup"], &rust.unwrap()] {
        let Installation::Managed { artifacts } = &tool.installation else {
            panic!("managed {}", tool.adapter);
        };
        let mut urls = artifacts.values().flat_map(|parts| parts.values());
        assert!(urls.all(|artifact| artifact.url.starts_with(&format!("{ORIGIN}/"))));
    }
    let plan = RustupPlan::from_lock(&lock, TARGET).unwrap();
    let repo = tempfile::tempdir().unwrap();
    let store = Store::open(repo.path()).unwrap();
    let desired = [plan.inventory_tool().clone()];
    let prepared = plan.prepare(&store, &desired, &transport, crate::Preparation::IfNeeded);
    assert!(prepared.await.unwrap().is_some());
    let requests = server.requests();
    assert!(requests.iter().all(|path| path.starts_with("/mirror/")));
    let installer = "/mirror/rustup/archive/1.29.1/x86_64-unknown-linux-gnu/rustup-init";
    assert!(requests.iter().any(|path| path == installer));
    assert!(requests.contains(&"/mirror/dist/channel-rust-1.99.toml".to_owned()));

    let parent = tempfile::tempdir().unwrap();
    let homes = PrivateHomes::create(parent.path()).unwrap();
    let command = homes.command(Path::new("rustup"), &transport);
    let envs: BTreeMap<_, _> = command.get_envs().collect();
    let update = format!("{base}/rustup");
    assert_eq!(
        envs[OsStr::new("RUSTUP_DIST_SERVER")],
        Some(OsStr::new(&base))
    );
    assert_eq!(
        envs[OsStr::new("RUSTUP_UPDATE_ROOT")],
        Some(OsStr::new(&update))
    );
}

#[tokio::test]
async fn windows_and_musl_installers_are_locked_but_not_promoted() {
    let repo = tempfile::tempdir().unwrap();
    let (server, transport, lock) = resolved().await;
    let store = Store::open(repo.path()).unwrap();
    let hits = server.hits();
    for platform in [
        Platform::WindowsX64,
        Platform::WindowsArm64,
        Platform::LinuxX64Musl,
        Platform::LinuxArm64Musl,
    ] {
        let plan = RustupPlan::from_lock(&lock, platform).unwrap();
        let desired = [plan.inventory_tool().clone()];
        let prepared = plan
            .prepare(&store, &desired, &transport, crate::Preparation::IfNeeded)
            .await;
        assert!(matches!(prepared, Err(Error::UnsupportedTarget)));
    }
    assert_eq!(server.hits(), hits);
}

#[test]
fn private_homes_confine_rustup_without_touching_home_or_path() {
    let fixture = OwnedSetupFixture::new().unwrap();
    let parent = tempfile::tempdir().unwrap();
    let homes = PrivateHomes::create(parent.path()).unwrap();
    for home in [homes.rustup_home(), homes.cargo_home()] {
        let mode = fs::metadata(home).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }
    assert!(PrivateHomes::create(parent.path()).is_err());

    let bin = tempfile::tempdir().unwrap();
    let binary = bin.path().join("rustup");
    fs::write(&binary, RUSTUP).unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    let transport = RustupTransport::official(&fixture.policy().unwrap()).unwrap();
    let mut command = homes.command(&binary, &transport);
    let envs: BTreeMap<_, _> = command.get_envs().collect();
    assert_eq!(envs[OsStr::new("RUSTUP_TOOLCHAIN")], None);
    assert_eq!(envs[OsStr::new("RUSTUP_DIST_ROOT")], None);
    assert_eq!(
        envs[OsStr::new("RUSTUP_DIST_SERVER")],
        Some(OsStr::new(ORIGIN))
    );
    assert_eq!(
        envs[OsStr::new("RUSTUP_UPDATE_ROOT")],
        Some(OsStr::new("https://static.rust-lang.org/rustup"))
    );
    assert!(!envs.contains_key(OsStr::new("PATH")));
    let output = command
        .env("HOME", fixture.home())
        .arg("show")
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim_end(),
        format!(
            "rustup|{}|{}|show",
            homes.rustup_home().display(),
            homes.cargo_home().display()
        )
    );
    assert_eq!(fs::read_dir(fixture.home()).unwrap().count(), 0);
}
