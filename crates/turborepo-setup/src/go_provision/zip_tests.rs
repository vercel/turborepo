#![allow(clippy::unwrap_used)]
//! The official Windows zips on every host: the `go/test` exclusion covers
//! its UTF-8 named fixtures, `.exe` executables are required, and only a
//! native Windows x64 host promotes the tree. Local fixtures only.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use super::*;
use crate::{
    Preparation,
    lock::Document,
    source_policy::test_support::{LoopbackServer, stored_zip_entries},
};

fn archive(gofmt: bool) -> Vec<u8> {
    let mut entries = vec![
        ("go/bin/go.exe", &b"go image"[..]),
        (
            "go/pkg/tool/windows_amd64/compile.exe",
            &b"compile image"[..],
        ),
        ("go/src/fmt/print.go", &b"package fmt\n"[..]),
        // As in the official zips; the name is not portable, so a write fails.
        (
            "go/test/fixedbugs/issue27836.dir/\u{de}foo.go",
            &b"package p\n"[..],
        ),
        ("go/test/fixedbugs/issue27836.go", &b"package p\n"[..]),
    ];
    if gofmt {
        entries.push(("go/bin/gofmt.exe", &b"gofmt image"[..]));
    }
    stored_zip_entries(&entries)
}

fn route(platform: Platform) -> String {
    let version = GoVersion::parse("1.22.3").unwrap();
    format!(
        "/go/{}",
        go_resolution::archive(version, platform).unwrap().0
    )
}

fn plan(platform: Platform, bytes: &[u8]) -> GoPlan {
    let version = GoVersion::parse("1.22.3").unwrap();
    let (name, format) = go_resolution::archive(version, platform).unwrap();
    let artifact = lock::Artifact {
        url: format!("https://dl.google.com/go/{name}"),
        sha256: format!("{:x}", Sha256::digest(bytes)),
        format,
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
    let lock = Lock::new(Document {
        schema_version: lock::SCHEMA_VERSION,
        tools: BTreeMap::from([("go".into(), tool)]),
    });
    GoPlan::from_lock(&lock.unwrap(), platform).unwrap()
}

#[tokio::test]
async fn windows_zips_skip_go_test_and_promote_only_on_native_x64() {
    for platform in [Platform::WindowsX64, Platform::WindowsArm64] {
        let bytes = archive(true);
        let plan = plan(platform, &bytes);
        let executables = &plan.inventory_tool().executables;
        assert_eq!(executables["go"], "bin/go.exe");
        assert_eq!(executables["gofmt"], "bin/gofmt.exe");
        let server = LoopbackServer::new([(route(platform), bytes)], |_| {}).unwrap();
        let transport = GoTransport::loopback_http_for_tests(server.origin()).unwrap();
        // Every host verifies and extracts the zip without writing `go/test`.
        let prepared = plan.download(&transport).await.unwrap();
        let tree = prepared.tree.root_path();
        assert!(tree.join("bin/go.exe").is_file() && tree.join("bin/gofmt.exe").is_file());
        assert!(tree.join("src/fmt/print.go").is_file());
        assert!(!tree.join("test").exists());

        let repo = tempfile::tempdir().unwrap();
        let mut store = Store::open(repo.path()).unwrap();
        let desired = [plan.inventory_tool().clone()];
        let result = plan.prepare(&store, &desired, &transport, Preparation::IfNeeded);
        if !(cfg!(windows) && platform == Platform::WindowsX64) {
            assert!(matches!(result.await, Err(Error::UnsupportedTarget)));
            assert_eq!(server.hits(), 1);
            continue;
        }
        let prepared = result.await.unwrap().unwrap();
        store
            .reconcile(&desired, |tool, path| prepared.stage(tool, path))
            .unwrap();
        let current = Store::inspect(repo.path()).unwrap().unwrap();
        let mut exports: Vec<_> = fs::read_dir(&current.bin)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        exports.sort();
        // PATHEXT finds `go` and `gofmt` through these `.exe` hard links.
        assert_eq!(exports, ["go.exe", "gofmt.exe"]);
        assert_eq!(fs::read(current.bin.join("go.exe")).unwrap(), b"go image");
        let installed = current.tool_tree(plan.inventory_tool()).unwrap();
        assert!(
            installed
                .join("pkg/tool/windows_amd64/compile.exe")
                .is_file()
        );
        assert!(!installed.join("test").exists());
        // A healthy tree is reused without another download.
        let again = plan.prepare(&store, &desired, &transport, Preparation::IfNeeded);
        assert!(again.await.unwrap().is_none());
        assert_eq!(server.hits(), 2);
    }
}

#[tokio::test]
async fn windows_zips_without_both_executables_never_stage() {
    let platform = Platform::WindowsX64;
    let bytes = archive(false);
    let plan = plan(platform, &bytes);
    let server = LoopbackServer::new([(route(platform), bytes)], |_| {}).unwrap();
    let transport = GoTransport::loopback_http_for_tests(server.origin()).unwrap();
    assert!(matches!(
        plan.download(&transport).await,
        Err(Error::Archive(turborepo_archive::Error::LayoutMismatch))
    ));
}
