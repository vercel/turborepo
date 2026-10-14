#![allow(clippy::unwrap_used)]

use std::{
    collections::BTreeMap,
    io::Write,
    process::Command,
    sync::{Arc, Mutex},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha224, Sha256, Sha512};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use turborepo_download::Error as DownloadError;
use turborepo_tool_install::Outcome;

use super::*;
use crate::{pnpm_provision::PnpmTransport, source_policy::test_support::LoopbackServer};

const TARGET: Platform = Platform::MacosArm64;

struct Registry {
    transport: PnpmTransport,
    requests: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Registry {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn serve(metadata: Value, bytes: Vec<u8>) -> Registry {
    serve_raw(serde_json::to_vec(&metadata).unwrap(), bytes).await
}
async fn serve_raw(metadata: Vec<u8>, bytes: Vec<u8>) -> Registry {
    serve_version("10.0.0", metadata, bytes).await
}
async fn serve_version(version: &str, metadata: Vec<u8>, bytes: Vec<u8>) -> Registry {
    let (metadata_path, tarball_path) = (
        format!("/pnpm/{version}"),
        format!("/pnpm/-/pnpm-{version}.tgz"),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let transport = PnpmTransport::loopback_http_for_tests(&format!(
        "http://{}",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let log = requests.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 8192];
            let n = socket.read(&mut request).await.unwrap();
            let path = std::str::from_utf8(&request[..n])
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap()
                .to_owned();
            log.lock().unwrap().push(path.clone());
            let body = match path.as_str() {
                p if p == metadata_path => &metadata,
                p if p == tarball_path => &bytes,
                _ => panic!("unexpected registry request: {path}"),
            };
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(header.as_bytes()).await.unwrap();
            socket.write_all(body).await.unwrap();
        }
    });
    Registry {
        transport,
        requests,
        task,
    }
}
fn metadata(bytes: &[u8]) -> Value {
    metadata_for("10.0.0", bytes)
}
fn metadata_for(version: &str, bytes: &[u8]) -> Value {
    json!({"name":"pnpm","version":version,"dist":{
        "tarball":format!("https://registry.npmjs.org/pnpm/-/pnpm-{version}.tgz"),
        "integrity":format!("sha512-{}", STANDARD.encode(Sha512::digest(bytes)))
    }})
}
fn package() -> Value {
    json!({"name":"pnpm","version":"10.0.0","bin":{"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"},
        "scripts":{"install":"touch should-never-run"}})
}
const HOOK: &str = "#!/usr/bin/env node\nthrow new Error('not a bootstrap hook');\n";
fn archive(package: &str, entry: &str) -> Vec<u8> {
    archive_files(&[
        ("package.json", package, 0o644),
        (entry, HOOK, 0o644),
        ("bin/pnpx.cjs", "#!/usr/bin/env node\n", 0o644),
        (
            "dist/templates/config.json",
            "adjacent runtime resource",
            0o640,
        ),
    ])
}
fn archive_files(files: &[(&str, &str, u32)]) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for &(path, data, mode) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(mode);
        header.set_cksum();
        tar.append_data(&mut header, format!("package/{path}"), data.as_bytes())
            .unwrap();
    }
    let tar = tar.into_inner().unwrap();
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&tar).unwrap();
    gzip.finish().unwrap()
}
fn fixture(bytes: &[u8]) -> Value {
    json!({"schemaVersion":0,"tools":{
        "node":{"adapter":"node","version":"24.0.0","declarations":[{"file":".nvmrc","request":"24.0.0"}],
            "installation":{"kind":"managed","artifacts":{"macos-arm64":{"distribution":{
                "url":"https://nodejs.org/dist/v24.0.0/node-v24.0.0-darwin-arm64.tar.gz",
                "sha256":"11".repeat(32),"format":"tar-gz","rootPrefix":"node-v24.0.0-darwin-arm64",
                "executables":{"node":"bin/node"}
            }}}}},
        "pnpm":{"adapter":"pnpm","version":"10.0.0","declarations":[{"file":"package.json","field":"/packageManager","request":"pnpm@10.0.0"}],
            "installation":{"kind":"managed","artifacts":{"any":{"package":{
                "url":"https://registry.npmjs.org/pnpm/-/pnpm-10.0.0.tgz",
                "sha256":format!("{:x}", Sha256::digest(bytes)),"format":"tar-gz","rootPrefix":"package",
                "executables":{"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"}
            }}}}}
    }})
}
fn plans(value: &Value) -> (NodePlan, PnpmPlan) {
    let lock = Lock::parse(&serde_json::to_vec(value).unwrap()).unwrap();
    let node = NodePlan::from_lock(&lock, TARGET).unwrap();
    let pnpm = PnpmPlan::from_lock(&lock, TARGET, &node, None).unwrap();
    (node, pnpm)
}
fn node_stage(_: &Tool, destination: &Path) -> Result<(), turborepo_tool_install::Error> {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir(destination.join("bin"))?;
    fs::write(
        destination.join("bin/node"),
        "#!/bin/sh\nprintf 'managed-node\\n'\nprintf '<%s>\\n' \"$@\"\nexit 19\n",
    )?;
    fs::set_permissions(
        destination.join("bin/node"),
        fs::Permissions::from_mode(0o755),
    )?;
    Ok(())
}
fn manifest(repo: &Path) -> Vec<u8> {
    fs::read(repo.join(".turbo/tools/manifest.json")).unwrap()
}
fn seeded(repo: &Path, node: &NodePlan) -> Store {
    let mut store = Store::open(repo).unwrap();
    store
        .reconcile(&[node.inventory_tool().clone()], node_stage)
        .unwrap();
    store
}
async fn install(
    store: &mut Store,
    node: &NodePlan,
    pnpm: &PnpmPlan,
    registry: &Registry,
) -> Result<Outcome, Error> {
    let desired = [node.inventory_tool().clone(), pnpm.inventory_tool().clone()];
    let prepared = pnpm
        .prepare_if_needed(store, &desired, &registry.transport)
        .await?;
    Ok(store.reconcile(&desired, |tool, path| {
        if tool == node.inventory_tool() {
            node_stage(tool, path)
        } else {
            prepared.as_ref().unwrap().stage(tool, path)
        }
    })?)
}

#[test]
fn declaration_preflight_binds_dev_integrity_and_rejects_known_invalid_top_pins() {
    use crate::package_manager::discover_package_manager;
    let bytes = archive(&package().to_string(), "bin/pnpm.cjs");
    let lock = Lock::parse(&serde_json::to_vec(&fixture(&bytes)).unwrap()).unwrap();
    let node = NodePlan::from_lock(&lock, TARGET).unwrap();
    let plain = PnpmPlan::from_lock(&lock, TARGET, &node, None).unwrap();
    for algorithm in ["sha1", "sha256"] {
        let manager = discover_package_manager(
            &json!({"packageManager":format!("pnpm@10.0.0+{algorithm}.{}",
            "0".repeat(if algorithm == "sha1" {40} else {64}))}),
        )
        .unwrap()
        .unwrap();
        assert!(PnpmPlan::from_declaration(&lock, TARGET, &node, &manager).is_err());
    }
    let mut tools = Vec::new();
    for digest in ["a", "b"] {
        let manager = discover_package_manager(&json!({"devEngines":{"packageManager":{
            "name":"pnpm","version":format!("10.0.0+sha512.{}",digest.repeat(128))
        }}}))
        .unwrap()
        .unwrap();
        let plan = PnpmPlan::from_declaration(&lock, TARGET, &node, &manager).unwrap();
        assert_ne!(plain.inventory_tool(), plan.inventory_tool());
        tools.push(plan.inventory_tool().clone());
    }
    assert_ne!(tools[0], tools[1]);
}

#[tokio::test]
async fn full_tree_bound_shims_repeat_reuse_and_stale_replacement() {
    use std::os::unix::fs::PermissionsExt;
    let bytes = archive(&package().to_string(), "bin/pnpm.cjs");
    let registry = serve(metadata(&bytes), bytes.clone()).await;
    let (node, pnpm) = plans(&fixture(&bytes));
    let repo = tempfile::Builder::new()
        .prefix("pnpm repo ' spaces ")
        .tempdir()
        .unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let desired = [node.inventory_tool().clone(), pnpm.inventory_tool().clone()];
    let prepared = pnpm
        .prepare_if_needed(&store, &desired, &registry.transport)
        .await
        .unwrap()
        .unwrap();
    assert!(store.current().unwrap().is_none()); // Preparation never publishes.
    store
        .reconcile(&desired, |tool, path| {
            if tool == node.inventory_tool() {
                node_stage(tool, path)
            } else {
                prepared.stage(tool, path)
            }
        })
        .unwrap();
    assert_eq!(
        fs::read_dir(repo.path().join(".turbo/tools"))
            .unwrap()
            .count(),
        3
    );
    let current = store.current().unwrap().unwrap();
    let root = current.bin.parent().unwrap().join("tools/pnpm");
    assert_eq!(
        fs::read_to_string(root.join("dist/templates/config.json")).unwrap(),
        "adjacent runtime resource"
    );
    let resource_mode = fs::metadata(root.join("dist/templates/config.json")).unwrap();
    assert_eq!(resource_mode.permissions().mode() & 0o777, 0o640);
    let cli_mode = fs::metadata(root.join("bin/pnpm.cjs")).unwrap();
    assert_eq!(cli_mode.permissions().mode() & 0o100, 0);
    let bait = tempfile::tempdir().unwrap();
    fs::write(
        bait.path().join("node"),
        "#!/bin/sh\nprintf ambient-node\nexit 99\n",
    )
    .unwrap();
    fs::set_permissions(bait.path().join("node"), fs::Permissions::from_mode(0o755)).unwrap();
    for path in [
        current.bin.join("pnpm"),
        root.join(&pnpm.inventory_tool().executables["pnpm"]),
        current.bin.join("pnpx"),
    ] {
        let output = Command::new(path)
            .args(["space argument", "literal'quote", "--version"])
            .env("PATH", bait.path())
            .current_dir(bait.path())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(19));
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.starts_with("managed-node\n<"));
        assert!(stdout.contains(".cjs>\n<space argument>\n<literal'quote>\n<--version>\n"));
        assert!(!stdout.contains("ambient-node"));
    }
    assert!(!repo.path().join("should-never-run").exists());
    let before = manifest(repo.path());
    assert_eq!(
        install(&mut store, &node, &pnpm, &registry).await.unwrap(),
        Outcome::Unchanged
    );
    assert_eq!(manifest(repo.path()), before);
    assert_eq!(registry.requests.lock().unwrap().len(), 2);
    let extra = Tool {
        id: "extra".into(),
        version: "1".into(),
        platform: "any".into(),
        artifact_sha256: "ab".repeat(32),
        executables: BTreeMap::from([("extra".into(), "bin/node".into())]),
    };
    let expanded = [
        node.inventory_tool().clone(),
        pnpm.inventory_tool().clone(),
        extra,
    ];
    assert!(
        pnpm.prepare_if_needed(&store, &expanded, &registry.transport)
            .await
            .unwrap()
            .is_none()
    );
    store.reconcile(&expanded, node_stage).unwrap();
    assert_eq!(registry.requests.lock().unwrap().len(), 2);
    let bin = store.current().unwrap().unwrap().bin;
    fs::remove_file(bin.join("pnpm")).unwrap();
    std::os::unix::fs::symlink("../tools/pnpm/bin/pnpm.cjs", bin.join("pnpm")).unwrap();
    install(&mut store, &node, &pnpm, &registry).await.unwrap();
    let bin = store.current().unwrap().unwrap().bin;
    assert_eq!(
        fs::read_link(bin.join("pnpm")).unwrap(),
        Path::new("../tools/pnpm").join(&pnpm.inventory_tool().executables["pnpm"])
    );
    assert!(!bin.join("extra").exists());
}

#[tokio::test]
async fn metadata_security_failures_do_not_download_or_mutate() {
    let bytes = archive(&package().to_string(), "bin/pnpm.cjs");
    for (pointer, value) in [
        ("/name", json!("npm")),
        ("/version", json!("10.0.1")),
        (
            "/dist/tarball",
            json!("https://registry.npmjs.org/pnpm/-/pnpm-10.0.1.tgz"),
        ),
        (
            "/dist/tarball",
            json!("https://evil.test/pnpm/-/pnpm-10.0.0.tgz"),
        ),
        (
            "/dist/tarball",
            json!("https://registry.npmjs.org/pnpm/-/../pnpm-10.0.0.tgz"),
        ),
        ("/dist/integrity", json!("sha1-not-sha512")),
    ] {
        let mut meta = metadata(&bytes);
        *meta.pointer_mut(pointer).unwrap() = value;
        let registry = serve(meta, bytes.clone()).await;
        let (node, pnpm) = plans(&fixture(&bytes));
        let repo = tempfile::tempdir().unwrap();
        let mut store = seeded(repo.path(), &node);
        let snapshot = manifest(repo.path());
        assert!(install(&mut store, &node, &pnpm, &registry).await.is_err());
        assert_eq!(registry.requests.lock().unwrap().len(), 1);
        assert_eq!(manifest(repo.path()), snapshot);
        assert_eq!(
            store.current().unwrap().unwrap().tools,
            [node.inventory_tool().clone()]
        );
    }
}

#[tokio::test]
async fn both_hashes_and_package_layout_are_mandatory() {
    let good = archive(&package().to_string(), "bin/pnpm.cjs");
    let mut wrong = package();
    wrong["name"] = json!("npm");
    let mut version = package();
    version["version"] = json!("10.0.1");
    let mut bin = package();
    bin["bin"]["pnpm"] = json!("dist/pnpm.cjs");
    let mut cases = vec![(good.clone(), "sha256"), (good, "sha512")];
    for invalid in [wrong, version, bin] {
        cases.push((archive(&invalid.to_string(), "bin/pnpm.cjs"), "package"));
    }
    cases.push((archive(&package().to_string(), "bin/wrong.cjs"), "layout"));
    let duplicate = archive("{\"name\":\"pnpm\",\"name\":\"pnpm\"}", "bin/pnpm.cjs");
    cases.push((duplicate, "package"));
    for (bytes, expected) in cases {
        let mut value = fixture(&bytes);
        let mut meta = metadata(&bytes);
        if expected == "sha256" {
            value["tools"]["pnpm"]["installation"]["artifacts"]["any"]["package"]["sha256"] =
                json!("00".repeat(32));
        }
        if expected == "sha512" {
            meta["dist"]["integrity"] = metadata(b"wrong sha512")["dist"]["integrity"].clone();
        }
        let (node, pnpm) = plans(&value);
        let registry = serve(meta, bytes).await;
        let repo = tempfile::tempdir().unwrap();
        let mut store = seeded(repo.path(), &node);
        let snapshot = manifest(repo.path());
        let error = install(&mut store, &node, &pnpm, &registry)
            .await
            .unwrap_err();
        let kind = match error {
            Error::Download(DownloadError::DigestMismatch) => "sha256",
            Error::Download(DownloadError::Sha512DigestMismatch) => "sha512",
            Error::Archive(turborepo_archive::Error::LayoutMismatch) => "layout",
            Error::InvalidPackage => "package",
            _ => panic!("unexpected error: {error}"),
        };
        assert_eq!(kind, expected);
        assert_eq!(manifest(repo.path()), snapshot);
    }
}

#[tokio::test]
async fn desired_node_binding_and_damaged_generation_repair() {
    let bytes = archive(&package().to_string(), "bin/pnpm.cjs");
    let registry = serve(metadata(&bytes), bytes.clone()).await;
    let (node, pnpm) = plans(&fixture(&bytes));
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let mut wrong = node.inventory_tool().clone();
    wrong.artifact_sha256 = "22".repeat(32);
    for nodes in [vec![], vec![wrong]] {
        let desired = [nodes, vec![pnpm.inventory_tool().clone()]].concat();
        assert!(matches!(
            pnpm.prepare_if_needed(&store, &desired, &registry.transport)
                .await,
            Err(Error::NodeBinding)
        ));
        assert!(registry.requests.lock().unwrap().is_empty());
        assert!(store.current().unwrap().is_none());
    }
    install(&mut store, &node, &pnpm, &registry).await.unwrap();
    let snapshot = manifest(repo.path());
    registry.requests.lock().unwrap().clear();
    let old = store.current().unwrap().unwrap().bin;
    fs::write(
        old.parent().unwrap().join("tools/node/bin/node"),
        "corrupted",
    )
    .unwrap();
    let desired = [node.inventory_tool().clone(), pnpm.inventory_tool().clone()];
    assert!(
        pnpm.prepare_if_needed(&store, &desired, &registry.transport)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(registry.requests.lock().unwrap().len(), 2);
    assert_eq!(manifest(repo.path()), snapshot);
    install(&mut store, &node, &pnpm, &registry).await.unwrap();
    assert!(store.is_current(&desired).unwrap());
    registry.requests.lock().unwrap().clear();
    let mut changed = fixture(&bytes);
    changed["tools"]["node"]["installation"]["artifacts"]["macos-arm64"]["distribution"]
        ["sha256"] = json!("22".repeat(32));
    let (_, new_pnpm) = plans(&changed);
    assert_ne!(pnpm.inventory_tool(), new_pnpm.inventory_tool());
    let desired = [
        node.inventory_tool().clone(),
        new_pnpm.inventory_tool().clone(),
    ];
    assert!(matches!(
        new_pnpm
            .prepare_if_needed(&store, &desired, &registry.transport)
            .await,
        Err(Error::NodeBinding)
    ));
    let lock = Lock::parse(&serde_json::to_vec(&changed).unwrap()).unwrap();
    let result = PnpmPlan::from_lock(&lock, TARGET, &node, None);
    assert!(matches!(result, Err(Error::NodeBinding)));
    assert!(registry.requests.lock().unwrap().is_empty());
}

#[test]
fn lock_transport_and_unqualified_targets_fail_closed() {
    let bytes = archive(&package().to_string(), "bin/pnpm.cjs");
    let value = fixture(&bytes);
    let lock = Lock::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    let (node, any) = plans(&value);
    let mut exact = value.clone();
    let sets = &mut exact["tools"]["pnpm"]["installation"]["artifacts"];
    sets["macos-arm64"] = sets["any"].clone();
    sets["macos-arm64"]["package"]["sha256"] = json!("33".repeat(32));
    assert!(Lock::parse(&serde_json::to_vec(&exact).unwrap()).is_err());
    exact["tools"]["pnpm"]["installation"]["artifacts"]
        .as_object_mut()
        .unwrap()
        .remove("any");
    let (_, native) = plans(&exact);
    assert_eq!(native.inventory_tool().artifact_sha256, "33".repeat(32));
    assert_eq!(
        any.inventory_tool().artifact_sha256,
        format!("{:x}", Sha256::digest(&bytes))
    );
    for platform in [
        Platform::WindowsX64,
        Platform::WindowsArm64,
        Platform::LinuxX64Musl,
        Platform::Any,
    ] {
        let result = PnpmPlan::from_lock(&lock, platform, &node, None);
        assert!(matches!(result, Err(Error::UnsupportedTarget)));
    }
    for (field, bad) in [
        ("url", json!("https://evil.test/pnpm.tgz")),
        ("rootPrefix", json!("other")),
        ("format", json!("tar")),
        ("executables", json!({"pnpm":"bin/wrong.cjs"})),
    ] {
        let mut changed = value.clone();
        changed["tools"]["pnpm"]["installation"]["artifacts"]["any"]["package"][field] = bad;
        let lock = Lock::parse(&serde_json::to_vec(&changed).unwrap()).unwrap();
        let result = PnpmPlan::from_lock(&lock, TARGET, &node, None);
        assert!(matches!(result, Err(Error::InvalidLock)));
    }
    for origin in [
        "http://localhost:1234",
        "http://example.com",
        "http://127.0.0.1:1234/arbitrary",
        "https://127.0.0.1",
        "http://user@127.0.0.1",
    ] {
        assert!(PnpmTransport::loopback_http_for_tests(origin).is_err());
    }
}

#[tokio::test]
async fn authored_pins_and_bounded_metadata_fail_before_artifacts() {
    let bytes = archive(&package().to_string(), "bin/pnpm.cjs");
    let value = fixture(&bytes);
    let lock = Lock::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    let (node, unpinned) = plans(&value);
    let repo = tempfile::tempdir().unwrap();
    let mut store = seeded(repo.path(), &node);
    let good = serve(metadata(&bytes), bytes.clone()).await;
    install(&mut store, &node, &unpinned, &good).await.unwrap();
    let snapshot = manifest(repo.path());
    for (algorithm, digest) in [
        ("sha512", "00".repeat(64)),
        ("sha256", "00".repeat(32)),
        ("sha512", "bad".into()),
        ("sha256", "bad".into()),
    ] {
        let pin = CorepackIntegrity { algorithm, digest };
        let pnpm = PnpmPlan::from_lock(&lock, TARGET, &node, Some(&pin)).unwrap();
        assert_ne!(pnpm.inventory_tool(), unpinned.inventory_tool());
        let registry = serve(metadata(&bytes), bytes.clone()).await;
        assert!(install(&mut store, &node, &pnpm, &registry).await.is_err());
        assert_eq!(registry.requests.lock().unwrap().len(), 1);
        assert_eq!(manifest(repo.path()), snapshot);
    }
    let registry = serve_raw(vec![b' '; MAX_METADATA_BYTES + 1], bytes).await;
    let (_, pnpm) = plans(&fixture(b"force metadata recheck"));
    let result = install(&mut store, &node, &pnpm, &registry).await;
    assert!(matches!(
        result,
        Err(Error::Download(DownloadError::TooLarge))
    ));
    assert_eq!(manifest(repo.path()), snapshot);
}

#[tokio::test]
async fn corepack_sha224_pins_check_downloaded_bytes_in_addition_to_sha512() {
    let bytes = archive(&package().to_string(), "bin/pnpm.cjs");
    let value = fixture(&bytes);
    let lock = Lock::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    let (node, unpinned) = plans(&value);
    let repo = tempfile::tempdir().unwrap();
    let mut store = seeded(repo.path(), &node);
    let snapshot = manifest(repo.path());
    let pin = |digest: String| CorepackIntegrity {
        algorithm: "sha224",
        digest,
    };
    let good = pin(format!("{:x}", Sha224::digest(&bytes)));
    for (authored, document, expected) in [
        // The pin is checked against the bytes the registry SHA-512 accepted.
        (
            pin("00".repeat(28)),
            metadata(&bytes),
            DownloadError::DigestMismatch,
        ),
        // A matching pin never replaces the mandatory registry SHA-512.
        (
            good.clone(),
            metadata(b"other bytes"),
            DownloadError::Sha512DigestMismatch,
        ),
    ] {
        let pnpm = PnpmPlan::from_lock(&lock, TARGET, &node, Some(&authored)).unwrap();
        let registry = serve(document, bytes.clone()).await;
        let result = install(&mut store, &node, &pnpm, &registry).await;
        assert!(matches!(&result, Err(Error::Download(error))
            if std::mem::discriminant(error) == std::mem::discriminant(&expected)));
        assert_eq!(registry.requests.lock().unwrap().len(), 2);
        assert_eq!(manifest(repo.path()), snapshot);
    }
    let pnpm = PnpmPlan::from_lock(&lock, TARGET, &node, Some(&good)).unwrap();
    assert_ne!(pnpm.inventory_tool(), unpinned.inventory_tool());
    let registry = serve(metadata(&bytes), bytes.clone()).await;
    install(&mut store, &node, &pnpm, &registry).await.unwrap();
    assert_ne!(manifest(repo.path()), snapshot);
}

#[tokio::test]
async fn pnpm_11_module_entry_points_and_aliases_run_on_managed_node() {
    let version = "11.28.2";
    let bins = json!({"pn":"bin/pnpm.mjs","pnpm":"bin/pnpm.mjs","pnpx":"bin/pnpx.mjs",
        "pnx":"bin/pnpx.mjs"});
    let package = json!({"name":"pnpm","version":version,"type":"module","bin":bins}).to_string();
    // Mirrors the published tarball: `.cjs` Corepack shims beside the entries.
    let shim = "#!/usr/bin/env node\nimport('./pnpm.mjs')\n";
    let bytes = archive_files(&[
        ("package.json", &package, 0o644),
        ("bin/pnpm.cjs", shim, 0o755),
        ("bin/pnpx.cjs", shim, 0o755),
        ("bin/pnpm.mjs", HOOK, 0o755),
        ("bin/pnpx.mjs", HOOK, 0o755),
        ("dist/pnpm.mjs", "bundle", 0o644),
    ]);
    let registry = serve_version(
        version,
        serde_json::to_vec(&metadata_for(version, &bytes)).unwrap(),
        bytes.clone(),
    )
    .await;
    let mut value = fixture(&bytes);
    let tool = &mut value["tools"]["pnpm"];
    tool["version"] = json!(version);
    tool["declarations"][0]["request"] = json!(format!("pnpm@{version}"));
    let artifact = &mut tool["installation"]["artifacts"]["any"]["package"];
    artifact["url"] = json!(format!(
        "https://registry.npmjs.org/pnpm/-/pnpm-{version}.tgz"
    ));
    artifact["executables"] = bins;
    let (node, pnpm) = plans(&value);
    let repo = tempfile::tempdir().unwrap();
    let mut store = seeded(repo.path(), &node);
    install(&mut store, &node, &pnpm, &registry).await.unwrap();
    let bin = store.current().unwrap().unwrap().bin;
    for (name, entry) in [
        ("pn", "pnpm"),
        ("pnpm", "pnpm"),
        ("pnpx", "pnpx"),
        ("pnx", "pnpx"),
    ] {
        let output = Command::new(bin.join(name))
            .arg("--version")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(19));
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.starts_with("managed-node\n<"), "{stdout}");
        assert!(
            stdout.ends_with(&format!("/pnpm/bin/{entry}.mjs>\n<--version>\n")),
            "{stdout}"
        );
    }
    // pnpm 10's entry points are not this release's layout, and pnpm 12 is
    // never provisioned from a JS package shape.
    for (version, legacy) in [("11.28.2", true), ("12.11.2", false)] {
        let mut changed = value.clone();
        let tool = &mut changed["tools"]["pnpm"];
        tool["version"] = json!(version);
        let artifact = &mut tool["installation"]["artifacts"]["any"]["package"];
        artifact["url"] = json!(format!(
            "https://registry.npmjs.org/pnpm/-/pnpm-{version}.tgz"
        ));
        if legacy {
            artifact["executables"] = json!({"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"});
        }
        let lock = Lock::parse(&serde_json::to_vec(&changed).unwrap()).unwrap();
        let result = PnpmPlan::from_lock(&lock, TARGET, &node, None);
        assert!(matches!(result, Err(Error::InvalidLock)), "{version}");
    }
}

#[tokio::test]
async fn native_pnpm_runs_the_verified_platform_binary_beside_its_wrapper() {
    use crate::{
        package_manager::discover_package_manager, registry_provision::native_fixture as fx,
    };
    let (wrapper, natives) = (fx::wrapper(&[]), fx::natives());
    let server = LoopbackServer::new(fx::routes(&wrapper, &natives), |_| {}).unwrap();
    let transport = PnpmTransport::loopback_http_for_tests(server.origin()).unwrap();
    let selected = transport
        .resolve_exact(Manager::Pnpm, fx::VERSION, None)
        .await
        .unwrap();
    let mut value = fixture(&wrapper);
    value["tools"]["pnpm"]["version"] = json!(fx::VERSION);
    value["tools"]["pnpm"]["installation"]["artifacts"] =
        serde_json::to_value(selected.into_artifacts()).unwrap();
    let (node, pnpm) = plans(&value);
    let lock = Lock::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    // Authored pins describe the wrapper, never the platform package.
    let parts = value["tools"]["pnpm"]["installation"]["artifacts"]["macos-arm64"].clone();
    let wrapper_sha256 = format!("{:x}", Sha256::digest(&wrapper));
    for (digest, valid) in [
        (wrapper_sha256.as_str(), true),
        (parts["native"]["sha256"].as_str().unwrap(), false),
    ] {
        let manager = discover_package_manager(
            &json!({"packageManager":format!("pnpm@{}+sha256.{digest}", fx::VERSION)}),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            PnpmPlan::from_declaration(&lock, TARGET, &node, &manager).is_ok(),
            valid
        );
    }
    let repo = tempfile::tempdir().unwrap();
    let mut store = seeded(repo.path(), &node);
    let desired = [node.inventory_tool().clone(), pnpm.inventory_tool().clone()];
    let prepared = pnpm
        .prepare_if_needed(&store, &desired, &transport)
        .await
        .unwrap()
        .unwrap();
    store
        .reconcile(&desired, |tool, path| {
            if tool == node.inventory_tool() {
                node_stage(tool, path)
            } else {
                prepared.stage(tool, path)
            }
        })
        .unwrap();
    let current = store.current().unwrap().unwrap();
    for (name, dlx) in [
        ("pn", ""),
        ("pnpm", ""),
        ("pnpx", "<dlx>\n"),
        ("pnx", "<dlx>\n"),
    ] {
        let output = Command::new(current.bin.join(name))
            .arg("--version")
            .env("PATH", "/nonexistent")
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("native:pnpm-native\n{dlx}<--version>\n")
        );
        assert_eq!(output.status.code(), Some(0));
    }
    // The wrapper's placeholders stay as published; its hooks never run.
    let root = current.bin.parent().unwrap().join("tools/pnpm");
    assert_eq!(
        fs::read_to_string(root.join("pnpm")).unwrap(),
        fx::PLACEHOLDER
    );
    let hits = server.hits();
    assert!(
        pnpm.prepare_if_needed(&store, &desired, &transport)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(server.hits(), hits);
    // Each part is verified against its own locked SHA-256 before staging, and
    // only qualified hosts provision.
    let mut tampered = value.clone();
    tampered["tools"]["pnpm"]["installation"]["artifacts"]["macos-arm64"]["native"]["sha256"] =
        json!("00".repeat(32));
    let (_, tampered) = plans(&tampered);
    let repo = tempfile::tempdir().unwrap();
    let store = seeded(repo.path(), &node);
    let desired = [
        node.inventory_tool().clone(),
        tampered.inventory_tool().clone(),
    ];
    let result = tampered
        .prepare_if_needed(&store, &desired, &transport)
        .await;
    assert!(matches!(
        result,
        Err(Error::Download(DownloadError::DigestMismatch))
    ));
    for platform in [Platform::WindowsX64, Platform::LinuxX64Musl] {
        assert!(matches!(
            PnpmPlan::from_lock(&lock, platform, &node, None),
            Err(Error::UnsupportedTarget)
        ));
    }
    for (part, field, bad) in [
        (
            "native",
            "url",
            json!("https://registry.npmjs.org/@pnpm/exe.linux-x64/-/exe.linux-x64-12.11.2.tgz"),
        ),
        (
            "native",
            "destination",
            json!("node_modules/@pnpm/exe.darwin-arm64"),
        ),
        (
            "package",
            "executables",
            parts["native"]["executables"].clone(),
        ),
    ] {
        let mut changed = value.clone();
        changed["tools"]["pnpm"]["installation"]["artifacts"]["macos-arm64"][part][field] = bad;
        let lock = Lock::parse(&serde_json::to_vec(&changed).unwrap());
        assert!(
            lock.is_err()
                || matches!(
                    PnpmPlan::from_lock(&lock.unwrap(), TARGET, &node, None),
                    Err(Error::InvalidLock)
                ),
            "{part}.{field}"
        );
    }
}

#[tokio::test]
async fn native_pnpm_cache_reuses_and_reverifies_both_locked_parts_across_repositories() {
    use crate::registry_provision::native_fixture as fx;
    let (wrapper, natives) = (fx::wrapper(&[]), fx::natives());
    let server = LoopbackServer::new(fx::routes(&wrapper, &natives), |_| {}).unwrap();
    let base = tempfile::tempdir().unwrap();
    let cache = ArtifactCache::new(base.path()).unwrap();
    let transport = PnpmTransport::loopback_http_for_tests(server.origin())
        .unwrap()
        .with_cache(Some(cache.clone()));
    let selected = transport
        .resolve_exact(Manager::Pnpm, fx::VERSION, None)
        .await
        .unwrap();
    let mut value = fixture(&wrapper);
    value["tools"]["pnpm"]["version"] = json!(fx::VERSION);
    value["tools"]["pnpm"]["installation"]["artifacts"] =
        serde_json::to_value(selected.into_artifacts()).unwrap();
    let (node, pnpm) = plans(&value);
    let desired = [node.inventory_tool().clone(), pnpm.inventory_tool().clone()];
    let native_sha = value["tools"]["pnpm"]["installation"]["artifacts"]["macos-arm64"]["native"]
        ["sha256"]
        .as_str()
        .unwrap();
    let wrapper_sha = format!("{:x}", Sha256::digest(&wrapper));
    let before = server.hits();
    for (index, expected_hits, cached) in [(0, 4, false), (1, 4, true), (2, 6, false)] {
        if index == 2 {
            let entry = base
                .path()
                .join("turborepo/setup-artifacts/v1/macos-arm64")
                .join(native_sha);
            fs::remove_file(&entry).unwrap();
            fs::write(&entry, b"torn native archive").unwrap();
        }
        let repo = tempfile::tempdir().unwrap();
        let mut store = seeded(repo.path(), &node);
        let prepared = pnpm
            .prepare(&store, &desired, &transport, crate::Preparation::Fresh)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(prepared.from_cache(), cached);
        assert_eq!(server.hits() - before, expected_hits);
        assert!(cache.get(TARGET, &wrapper_sha, PACKAGE_LIMIT).is_some());
        assert!(cache.get(TARGET, native_sha, PACKAGE_LIMIT).is_some());
        store
            .reconcile(&desired, |tool, path| {
                if tool == node.inventory_tool() {
                    node_stage(tool, path)
                } else {
                    prepared.stage(tool, path)
                }
            })
            .unwrap();
        let current = store.current().unwrap().unwrap();
        assert_eq!(
            fs::read_to_string(current.bin.parent().unwrap().join("tools/pnpm/pnpm-native"))
                .unwrap(),
            fx::BINARY
        );
        assert_eq!(
            fs::read_to_string(current.bin.parent().unwrap().join("tools/pnpm/pnpm")).unwrap(),
            fx::PLACEHOLDER
        );
    }
}
