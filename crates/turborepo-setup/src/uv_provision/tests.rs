#![allow(clippy::unwrap_used)]

use std::{collections::BTreeMap, io::Write, process::Command};

use semver::Version;
use sha2::{Digest, Sha256};
use turborepo_tool_install::Outcome;

use super::*;
use crate::{
    lock::{Declaration, Document},
    source_policy::test_support::LoopbackServer,
};

// Prints how it was reached; must never be a real uv.
const FIXTURE: &str = "#!/bin/sh\necho \"$0|$UV_PYTHON_INSTALL_DIR|$UV_PYTHON_DOWNLOADS|$*\"\n";
const QUALIFIED: [Platform; 4] = [
    Platform::MacosX64,
    Platform::MacosArm64,
    Platform::LinuxX64Gnu,
    Platform::LinuxArm64Gnu,
];

fn official(platform: Platform) -> UvArtifact {
    UvArtifact::for_platform(&Version::new(0, 13, 0), platform).unwrap()
}
fn archive(platform: Platform, names: &[&str]) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for name in names {
        let mut header = tar::Header::new_gnu();
        header.set_size(FIXTURE.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        let path = format!("uv-{}/{name}", official(platform).target());
        tar.append_data(&mut header, path, FIXTURE.as_bytes())
            .unwrap();
    }
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&tar.into_inner().unwrap()).unwrap();
    gzip.finish().unwrap()
}
fn document(platform: Platform, bytes: &[u8]) -> Document {
    let artifact = official(platform).lock_artifact(format!("{:x}", Sha256::digest(bytes)));
    Document {
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
    }
}
fn artifact(document: &mut Document) -> &mut BTreeMap<String, lock::Artifact> {
    let Installation::Managed { artifacts } =
        &mut document.tools.get_mut("uv").unwrap().installation
    else {
        unreachable!()
    };
    artifacts.get_mut(&Platform::LinuxX64Gnu).unwrap()
}
fn plan(document: Document, platform: Platform) -> Result<UvPlan, Error> {
    UvPlan::from_lock(&Lock::new(document).unwrap(), platform)
}
fn serve(platform: Platform, bytes: Vec<u8>) -> (LoopbackServer, UvTransport) {
    let path = official(platform).url()[uv::ORIGIN.len()..].to_owned();
    let upstream = LoopbackServer::new([(path, bytes)], |_| {}).unwrap();
    let transport = UvTransport::loopback_http_for_tests(upstream.origin()).unwrap();
    (upstream, transport)
}
async fn install(
    store: &mut Store,
    plan: &UvPlan,
    transport: &UvTransport,
    preparation: crate::Preparation,
) -> Result<Outcome, Error> {
    let desired = [plan.inventory_tool().clone()];
    let prepared = plan
        .prepare(store, &desired, transport, preparation)
        .await?;
    Ok(store.reconcile(&desired, |tool, path| {
        prepared
            .as_ref()
            .ok_or(turborepo_tool_install::Error::InvalidInventory)?
            .stage(tool, path)
    })?)
}
fn run(program: &Path) -> String {
    let output = Command::new(program)
        .args(["sync", "--locked"])
        .env_clear()
        .env("UV_PYTHON_INSTALL_DIR", "/global/uv/python")
        .env("UV_PYTHON_DOWNLOADS", "automatic")
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[tokio::test(flavor = "current_thread")]
async fn locked_uv_installs_verified_tree_with_repo_scoped_launchers() {
    for platform in QUALIFIED {
        let bytes = archive(platform, &["uv", "uvx"]);
        let (upstream, transport) = serve(platform, bytes.clone());
        let plan = plan(document(platform, &bytes), platform).unwrap();
        let repo = tempfile::tempdir().unwrap();
        let mut store = Store::open(repo.path()).unwrap();
        let fresh = crate::Preparation::IfNeeded;
        assert_eq!(
            install(&mut store, &plan, &transport, fresh).await.unwrap(),
            Outcome::Replaced
        );
        let current = store.current().unwrap().unwrap();
        assert_eq!(current.tools, [plan.inventory_tool().clone()]);
        let tools = fs::canonicalize(current.bin.join("../tools")).unwrap();
        let launch = &plan.inventory_tool().executables["uv"];
        assert!(launch.starts_with("launch-") && launch.ends_with("/uv"));
        // Through the generation bin link and directly through the launcher, the
        // caller's global interpreter settings are replaced by repo scope.
        for (program, binary) in [
            (current.bin.join("uv"), "uv"),
            (current.bin.join("uvx"), "uvx"),
            (tools.join("uv").join(launch), "uv"),
        ] {
            assert_eq!(
                run(&program),
                format!(
                    "{}|{}|never|sync --locked",
                    tools.join("uv").join(binary).display(),
                    tools.join("python").display()
                )
            );
        }
        assert!(!tools.join("python").exists());
        // Healthy repeat is network-free; fresh preparation re-verifies bytes.
        assert_eq!(
            install(&mut store, &plan, &transport, fresh).await.unwrap(),
            Outcome::Unchanged
        );
        assert_eq!(upstream.hits(), 1);
        let desired = [plan.inventory_tool().clone()];
        let prepared = plan
            .prepare(&store, &desired, &transport, crate::Preparation::Fresh)
            .await
            .unwrap();
        assert!(prepared.is_some());
        assert_eq!(upstream.hits(), 2);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn mirrors_serve_the_official_archive_path_beneath_their_base() {
    let platform = Platform::LinuxX64Gnu;
    let bytes = archive(platform, &["uv", "uvx"]);
    let path = format!("/mirror{}", &official(platform).url()[uv::ORIGIN.len()..]);
    let upstream = LoopbackServer::new([(path.clone(), bytes.clone())], |_| {}).unwrap();
    let fixture = crate::source_policy::test_support::OwnedSetupFixture::new().unwrap();
    let base = format!("{}/mirror", upstream.origin());
    let mirror = crate::mirror::Mirror::loopback_for_tests(&base).unwrap();
    let transport = UvTransport::mirrored(&fixture.policy().unwrap(), &mirror).unwrap();
    let plan = plan(document(platform, &bytes), platform).unwrap();
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let outcome = install(&mut store, &plan, &transport, crate::Preparation::IfNeeded).await;
    assert_eq!(outcome.unwrap(), Outcome::Replaced);
    assert_eq!(upstream.requests(), [path]);
}

#[tokio::test(flavor = "current_thread")]
async fn checksum_and_layout_failures_never_publish() {
    let platform = Platform::LinuxX64Gnu;
    let good = archive(platform, &["uv", "uvx"]);
    let missing_uvx = archive(platform, &["uv"]);
    for (locked, served, layout) in [
        (
            good.clone(),
            archive(platform, &["uv", "uvx", "extra"]),
            false,
        ),
        (missing_uvx.clone(), missing_uvx, true),
    ] {
        let (_upstream, transport) = serve(platform, served);
        let plan = plan(document(platform, &locked), platform).unwrap();
        let repo = tempfile::tempdir().unwrap();
        let mut store = Store::open(repo.path()).unwrap();
        let result = install(&mut store, &plan, &transport, crate::Preparation::IfNeeded).await;
        assert!(
            if layout {
                matches!(result, Err(Error::Archive(_)))
            } else {
                matches!(
                    result,
                    Err(Error::Download(turborepo_download::Error::DigestMismatch))
                )
            },
            "{result:?}"
        );
        assert!(store.current().unwrap().is_none());
    }
}

#[test]
fn plans_accept_only_official_layouts_on_qualified_targets() {
    let bytes = archive(Platform::LinuxX64Gnu, &["uv", "uvx"]);
    for platform in [
        Platform::LinuxX64Musl,
        Platform::LinuxArm64Musl,
        Platform::WindowsX64,
        Platform::WindowsArm64,
    ] {
        assert!(matches!(
            plan(document(platform, &bytes), platform),
            Err(Error::UnsupportedTarget)
        ));
    }
    assert!(matches!(
        plan(
            document(Platform::LinuxX64Gnu, &bytes),
            Platform::MacosArm64
        ),
        Err(Error::MissingTarget)
    ));
    let mutations: [fn(&mut Document); 5] = [
        |d| {
            artifact(d).get_mut("distribution").unwrap().url =
                "https://example.com/uv.tar.gz".into()
        },
        |d| artifact(d).get_mut("distribution").unwrap().root_prefix = Some("uv".into()),
        |d| {
            let part = artifact(d).get_mut("distribution").unwrap();
            part.executables.insert("uvw".into(), "uvw".into());
        },
        |d| {
            let part = artifact(d).remove("distribution").unwrap();
            artifact(d).insert("archive".into(), part);
        },
        |d| {
            d.tools.get_mut("uv").unwrap().options =
                BTreeMap::from([("python".into(), vec!["3.12".into()])]);
        },
    ];
    for mutate in mutations {
        let mut document = document(Platform::LinuxX64Gnu, &bytes);
        mutate(&mut document);
        assert!(matches!(
            plan(document, Platform::LinuxX64Gnu),
            Err(Error::InvalidLock)
        ));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn preparation_requires_the_complete_desired_inventory() {
    let platform = Platform::MacosArm64;
    let bytes = archive(platform, &["uv", "uvx"]);
    let (upstream, transport) = serve(platform, bytes.clone());
    let plan = plan(document(platform, &bytes), platform).unwrap();
    let repo = tempfile::tempdir().unwrap();
    let store = Store::open(repo.path()).unwrap();
    assert!(matches!(
        plan.prepare_if_needed(&store, &[], &transport).await,
        Err(Error::InventoryMismatch)
    ));
    assert_eq!(upstream.hits(), 0);
}
