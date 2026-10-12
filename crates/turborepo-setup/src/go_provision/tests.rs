#![allow(clippy::unwrap_used)]

use std::{collections::BTreeMap, io::Write, os::unix::fs::PermissionsExt};

use sha2::{Digest, Sha256};

use super::*;
use crate::{Preparation, lock::Document, source_policy::test_support::LoopbackServer};

const ROUTE: &str = "/go/go1.22.3.linux-amd64.tar.gz";

fn add(builder: &mut tar::Builder<Vec<u8>>, path: &str, mode: u32, data: &[u8]) {
    let mut header = tar::Header::new_ustar();
    header.set_mode(mode);
    header.set_size(data.len() as u64);
    header.set_path(path).unwrap();
    header.set_cksum();
    builder.append(&header, data).unwrap();
}

// Mirrors the official layout, including a PAX-named non-ASCII test fixture.
fn archive(gofmt: bool) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    add(
        &mut builder,
        "go/bin/go",
        0o755,
        b"#!/bin/sh\necho go1.22.3\n",
    );
    if gofmt {
        add(&mut builder, "go/bin/gofmt", 0o755, b"#!/bin/sh\n");
    }
    add(&mut builder, "go/src/fmt/print.go", 0o644, b"package fmt\n");
    let fixture = "go/test/fixedbugs/\u{de}foo.go";
    builder
        .append_pax_extensions([("path", fixture.as_bytes())])
        .unwrap();
    add(
        &mut builder,
        "go/test/fixedbugs/foo.go",
        0o644,
        b"package p\n",
    );
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&builder.into_inner().unwrap()).unwrap();
    encoder.finish().unwrap()
}

fn document(platform: Platform, bytes: &[u8]) -> Document {
    let version = GoVersion::parse("1.22.3").unwrap();
    let (name, format) = go_resolution::archive(version, platform).unwrap();
    let artifact = lock::Artifact {
        url: format!("https://dl.google.com/go/{name}"),
        sha256: format!("{:x}", Sha256::digest(bytes)),
        format,
        alternates: BTreeMap::new(),
        root_prefix: Some("go".into()),
        destination: None,
        executables: go_resolution::executables(format),
    };
    let tool = lock::Tool {
        adapter: "go".into(),
        version: version.to_string(),
        declarations: vec![lock::Declaration {
            file: "go.work".into(),
            field: Some("go".into()),
            request: Some("1.22".into()),
        }],
        options: BTreeMap::new(),
        installation: Installation::Managed {
            artifacts: BTreeMap::from([(
                platform,
                BTreeMap::from([("distribution".into(), artifact)]),
            )]),
        },
    };
    Document {
        schema_version: lock::SCHEMA_VERSION,
        tools: BTreeMap::from([("go".into(), tool)]),
    }
}
fn plan(document: Document, platform: Platform) -> Result<GoPlan, Error> {
    GoPlan::from_lock(&Lock::new(document).unwrap(), platform)
}

#[tokio::test]
async fn verified_archive_installs_go_and_gofmt_then_reuses_healthy_trees() {
    let bytes = archive(true);
    let platform = Platform::LinuxX64Gnu;
    let plan = plan(document(platform, &bytes), platform).unwrap();
    let server = LoopbackServer::new([(ROUTE.to_owned(), bytes)], |_| {}).unwrap();
    let transport = GoTransport::loopback_http_for_tests(server.origin()).unwrap();
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let desired = [plan.inventory_tool().clone()];
    let prepared = plan
        .prepare(&store, &desired, &transport, Preparation::IfNeeded)
        .await
        .unwrap()
        .unwrap();
    store
        .reconcile(&desired, |tool, path| prepared.stage(tool, path))
        .unwrap();
    let current = Store::inspect(repo.path()).unwrap().unwrap();
    let tree = current.tool_tree(plan.inventory_tool()).unwrap();
    for name in ["go", "gofmt"] {
        let mode = fs::metadata(tree.join("bin").join(name))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "{name}");
        assert!(current.bin.join(name).exists(), "{name}");
    }
    assert!(tree.join("src/fmt/print.go").is_file());
    assert!(!tree.join("test").exists());
    assert_eq!(server.requests(), [ROUTE]);
    // Repeated setup reuses the healthy tree; fresh preparation re-verifies.
    let again = plan.prepare(&store, &desired, &transport, Preparation::IfNeeded);
    assert!(again.await.unwrap().is_none());
    assert_eq!(server.hits(), 1);
    let fresh = plan.prepare(&store, &desired, &transport, Preparation::Fresh);
    assert!(fresh.await.unwrap().is_some());
    assert_eq!(server.hits(), 2);
    let partial = plan.prepare(&store, &[], &transport, Preparation::IfNeeded);
    assert!(matches!(partial.await, Err(Error::InventoryMismatch)));
}

/// One mirror base serves both official paths: go.dev's index and
/// dl.google.com's archives. The lock keeps the official URLs.
#[tokio::test]
async fn one_mirror_base_serves_the_official_index_and_archive_paths() {
    use crate::go_discovery::{GO_WORK, GoRequirements};
    let bytes = archive(true);
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    let file = serde_json::json!({"filename": "go1.22.3.linux-amd64.tar.gz", "os": "linux",
        "arch": "amd64", "version": "go1.22.3", "sha256": sha256, "kind": "archive"});
    let index = serde_json::json!([{"version": "go1.22.3", "files": [file]}]).to_string();
    let paths = [
        format!("/mirror{}", go_resolution::INDEX_PATH),
        format!("/mirror{ROUTE}"),
    ];
    let routes = paths.clone().into_iter().zip([index.into_bytes(), bytes]);
    let server = LoopbackServer::new(routes, |_| {}).unwrap();
    let fixture = crate::source_policy::test_support::OwnedSetupFixture::new().unwrap();
    let base = format!("{}/mirror/", server.origin());
    let mirror = crate::mirror::Mirror::loopback_for_tests(&base).unwrap();
    let transport = GoTransport::mirrored(&fixture.policy().unwrap(), &mirror).unwrap();
    let requirements =
        GoRequirements::read_with(|name, _| Ok((name == GO_WORK).then(|| b"go 1.22\n".to_vec())));
    let tool = transport.resolve(&requirements.unwrap().unwrap()).await;
    let lock = Lock::new(Document {
        schema_version: lock::SCHEMA_VERSION,
        tools: BTreeMap::from([("go".into(), tool.unwrap())]),
    });
    // Planning accepts only the official dl.google.com URL.
    let plan = GoPlan::from_lock(&lock.unwrap(), Platform::LinuxX64Gnu).unwrap();
    let repo = tempfile::tempdir().unwrap();
    let store = Store::open(repo.path()).unwrap();
    let desired = [plan.inventory_tool().clone()];
    let prepared = plan.prepare(&store, &desired, &transport, Preparation::IfNeeded);
    assert!(prepared.await.unwrap().is_some());
    assert_eq!(server.requests(), paths);
}

#[tokio::test]
async fn invalid_complete_cohorts_fail_before_traffic_including_fresh_preparation() {
    let bytes = archive(true);
    let platform = Platform::LinuxX64Gnu;
    let plan = plan(document(platform, &bytes), platform).unwrap();
    let server = LoopbackServer::new([(ROUTE.to_owned(), bytes)], |_| {}).unwrap();
    let transport = GoTransport::loopback_http_for_tests(server.origin()).unwrap();
    let repo = tempfile::tempdir().unwrap();
    let store = Store::open(repo.path()).unwrap();
    let tool = plan.inventory_tool().clone();
    let mut collision = tool.clone();
    collision.id = "other".into();
    let mut invalid = tool.clone();
    invalid.id = "../invalid".into();
    for preparation in [Preparation::IfNeeded, Preparation::Fresh] {
        for desired in [
            vec![tool.clone(), tool.clone()],
            vec![tool.clone(), collision.clone()],
            vec![tool.clone(), invalid.clone()],
        ] {
            assert!(matches!(
                plan.prepare(&store, &desired, &transport, preparation)
                    .await,
                Err(Error::Install(_))
            ));
            assert_eq!(server.hits(), 0);
            assert!(Store::inspect(repo.path()).unwrap().is_none());
        }
    }
}

#[tokio::test]
async fn corrupt_or_incomplete_archives_never_stage() {
    let platform = Platform::LinuxX64Gnu;
    for (locked, served) in [
        (archive(true), b"corrupt".to_vec()),
        (archive(false), archive(false)),
    ] {
        let plan = plan(document(platform, &locked), platform).unwrap();
        let server = LoopbackServer::new([(ROUTE.to_owned(), served)], |_| {}).unwrap();
        let transport = GoTransport::loopback_http_for_tests(server.origin()).unwrap();
        let repo = tempfile::tempdir().unwrap();
        let store = Store::open(repo.path()).unwrap();
        let desired = [plan.inventory_tool().clone()];
        let result = plan
            .prepare(&store, &desired, &transport, Preparation::IfNeeded)
            .await;
        assert!(matches!(
            result,
            Err(Error::Download(turborepo_download::Error::DigestMismatch)
                | Error::Archive(turborepo_archive::Error::LayoutMismatch))
        ));
        assert!(Store::inspect(repo.path()).unwrap().is_none());
    }
}

#[tokio::test]
async fn locks_must_match_the_official_layout_for_qualified_targets() {
    let bytes = archive(true);
    let linux = || document(Platform::LinuxX64Gnu, &bytes);
    assert!(matches!(
        plan(linux(), Platform::LinuxX64Musl),
        Err(Error::UnsupportedTarget)
    ));
    assert!(matches!(
        plan(linux(), Platform::MacosArm64),
        Err(Error::MissingTarget)
    ));
    let tampered: [fn(&mut lock::Tool); 5] = [
        |tool| tool.version = "1.22.4".into(),
        |tool| {
            tool.options.insert("channel".into(), vec!["stable".into()]);
        },
        |tool| artifact(tool).url = "https://dl.google.com/go/other.tar.gz".into(),
        |tool| artifact(tool).root_prefix = None,
        |tool| {
            artifact(tool).executables.remove("gofmt");
        },
    ];
    for tamper in tampered {
        let mut document = linux();
        tamper(document.tools.get_mut("go").unwrap());
        assert!(matches!(
            plan(document, Platform::LinuxX64Gnu),
            Err(Error::InvalidLock)
        ));
    }
    // Windows archives are lockable but promotion stays unqualified.
    let windows = plan(document(Platform::WindowsX64, &bytes), Platform::WindowsX64).unwrap();
    let transport = GoTransport::loopback_http_for_tests("http://127.0.0.1:9").unwrap();
    let repo = tempfile::tempdir().unwrap();
    let store = Store::open(repo.path()).unwrap();
    let desired = [windows.inventory_tool().clone()];
    let result = windows.prepare(&store, &desired, &transport, Preparation::IfNeeded);
    assert!(matches!(result.await, Err(Error::UnsupportedTarget)));
}

fn artifact(tool: &mut lock::Tool) -> &mut lock::Artifact {
    let Installation::Managed { artifacts } = &mut tool.installation else {
        panic!("expected managed Go");
    };
    artifacts
        .values_mut()
        .next()
        .unwrap()
        .get_mut("distribution")
        .unwrap()
}
