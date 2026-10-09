#![allow(clippy::unwrap_used)]

use std::{
    fs,
    io::Write,
    path::Path,
    process::Command,
    sync::{Arc, Mutex},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256, Sha512};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use turborepo_tool_install::Outcome;

use super::*;

const TARGET: Platform = Platform::MacosArm64;
const ARTIFACT: &str = "/tools/npm/installation/artifacts/any/package";
struct Registry {
    transport: NpmTransport,
    requests: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Registry {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn serve(metadata: Value, bytes: Vec<u8>) -> Registry {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let transport = NpmTransport::loopback_http_for_tests(&format!(
        "http://{}",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let log = requests.clone();
    let metadata = serde_json::to_vec(&metadata).unwrap();
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 8192];
            let n = socket.read(&mut request).await.unwrap();
            let path = std::str::from_utf8(&request[..n])
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap()
                .to_owned();
            log.lock().unwrap().push(path.clone());
            let body = match path.as_str() {
                "/npm/11.0.0" => &metadata,
                "/npm/-/npm-11.0.0.tgz" => &bytes,
                _ => panic!("unexpected registry request: {path}"),
            };
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            socket.write_all(body).await.unwrap();
        }
    });
    Registry {
        transport,
        requests,
        task,
    }
}
fn package() -> Value {
    json!({"name":"npm","version":"11.0.0","bin":{"npm":"bin/npm-cli.js","npx":"bin/npx-cli.js"},
        "scripts":{"install":"touch should-never-run"}})
}
fn archive(package: &str, npm: &str) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for (path, data, mode) in [
        ("package.json", package, 0o644),
        (
            npm,
            "#!/usr/bin/env node\nthrow new Error('not executed by provisioning');\n",
            0o644,
        ),
        ("bin/npx-cli.js", "#!/usr/bin/env node\n", 0o644),
        (
            "node_modules/resource/data.json",
            "adjacent resource",
            0o640,
        ),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(mode);
        header.set_cksum();
        tar.append_data(&mut header, format!("package/{path}"), data.as_bytes())
            .unwrap();
    }
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&tar.into_inner().unwrap()).unwrap();
    gzip.finish().unwrap()
}
fn metadata(bytes: &[u8]) -> Value {
    json!({"name":"npm","version":"11.0.0","dist":{
        "tarball":"https://registry.npmjs.org/npm/-/npm-11.0.0.tgz",
        "integrity":format!("sha512-{}", STANDARD.encode(Sha512::digest(bytes)))}})
}
fn fixture(bytes: &[u8]) -> Value {
    json!({"schemaVersion":0,"tools":{
        "node":{"adapter":"node","version":"24.0.0","declarations":[{"file":".nvmrc","request":"24.0.0"}],
            "installation":{"kind":"managed","artifacts":{"macos-arm64":{"distribution":{
                "url":"https://nodejs.org/dist/v24.0.0/node-v24.0.0-darwin-arm64.tar.gz",
                "sha256":"11".repeat(32),"format":"tar-gz","rootPrefix":"node-v24.0.0-darwin-arm64",
                "executables":{"node":"bin/node"}}}}}},
        "npm":{"adapter":"npm","version":"11.0.0","declarations":[{"file":"package.json","field":"/packageManager","request":"npm@11.0.0"}],
            "installation":{"kind":"managed","artifacts":{"any":{"package":{
                "url":"https://registry.npmjs.org/npm/-/npm-11.0.0.tgz",
                "sha256":format!("{:x}", Sha256::digest(bytes)),"format":"tar-gz","rootPrefix":"package",
                "executables":{"npm":"bin/npm-cli.js","npx":"bin/npx-cli.js"}}}}}}
    }})
}
fn lock(value: &Value) -> Lock {
    Lock::parse(&serde_json::to_vec(value).unwrap()).unwrap()
}
fn plans(value: &Value, pin: Option<&CorepackIntegrity>) -> (NodePlan, NpmPlan) {
    let lock = lock(value);
    let node = NodePlan::from_lock(&lock, TARGET).unwrap();
    let npm = NpmPlan::from_lock(&lock, TARGET, &node, pin).unwrap();
    (node, npm)
}
// Deliberately a fake interpreter: these tests qualify binding/argv/resources,
// not real Node/npm runtime compatibility. No ambient Node prerequisite.
fn node_stage(_: &Tool, path: &Path) -> Result<(), turborepo_tool_install::Error> {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir(path.join("bin"))?;
    fs::write(
        path.join("bin/node"),
        "#!/bin/sh\nprintf 'managed-node\\n<%s>\\n' \"$@\"\nexit 19\n",
    )?;
    fs::set_permissions(path.join("bin/node"), fs::Permissions::from_mode(0o755))?;
    Ok(())
}
fn manifest(repo: &Path) -> Vec<u8> {
    fs::read(repo.join(".turbo/tools/manifest.json")).unwrap()
}
async fn install(
    store: &mut Store,
    node: &NodePlan,
    npm: &NpmPlan,
    registry: &Registry,
) -> Result<Outcome, Error> {
    let desired = [node.inventory_tool().clone(), npm.inventory_tool().clone()];
    let prepared = npm
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

#[tokio::test]
async fn fresh_cohort_resources_bound_launchers_reuse_and_repair() {
    use std::os::unix::fs::PermissionsExt;
    let bytes = archive(&package().to_string(), "bin/npm-cli.js");
    let registry = serve(metadata(&bytes), bytes.clone()).await;
    let (node, npm) = plans(&fixture(&bytes), None);
    let repo = tempfile::Builder::new()
        .prefix("npm repo ' spaces ")
        .tempdir()
        .unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let desired = [node.inventory_tool().clone(), npm.inventory_tool().clone()];
    let prepared = npm
        .prepare_if_needed(&store, &desired, &registry.transport)
        .await
        .unwrap()
        .unwrap();
    assert!(store.current().unwrap().is_none());
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
    let root = current.bin.parent().unwrap().join("tools/npm");
    assert_eq!(
        fs::read_to_string(root.join("node_modules/resource/data.json")).unwrap(),
        "adjacent resource"
    );
    assert_eq!(
        fs::metadata(root.join("node_modules/resource/data.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o640
    );
    assert_eq!(
        fs::metadata(root.join("bin/npm-cli.js"))
            .unwrap()
            .permissions()
            .mode()
            & 0o100,
        0
    );
    assert!(matches!(
        Command::new(root.join("bin/npm-cli.js")).output(),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied
    ));
    let bait = tempfile::tempdir().unwrap();
    fs::write(bait.path().join("node"), "#!/bin/sh\nexit 99\n").unwrap();
    fs::set_permissions(bait.path().join("node"), fs::Permissions::from_mode(0o755)).unwrap();
    for name in ["npm", "npx"] {
        let launcher = root.join(&npm.inventory_tool().executables[name]);
        assert_ne!(
            fs::metadata(&launcher).unwrap().permissions().mode() & 0o100,
            0
        );
        for path in [current.bin.join(name), launcher] {
            let output = Command::new(path)
                .args(["space arg", "literal'quote"])
                .env("PATH", bait.path())
                .current_dir(bait.path())
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(19));
            let stdout = String::from_utf8(output.stdout).unwrap();
            assert!(stdout.starts_with("managed-node\n<"));
            assert!(stdout.contains(&format!("/tools/npm/bin/{name}-cli.js>")));
            assert!(stdout.contains("<space arg>\n") && stdout.contains("<literal'quote>\n"));
        }
    }
    assert!(!repo.path().join("should-never-run").exists());
    let before = manifest(repo.path());
    assert_eq!(
        install(&mut store, &node, &npm, &registry).await.unwrap(),
        Outcome::Unchanged
    );
    assert_eq!(manifest(repo.path()), before);
    assert_eq!(registry.requests.lock().unwrap().len(), 2);
    let extra = Tool {
        id: "extra".into(),
        version: "1".into(),
        platform: "any".into(),
        artifact_sha256: "ab".repeat(32),
        executables: std::collections::BTreeMap::from([("extra".into(), "bin/node".into())]),
    };
    let expanded = [desired.to_vec(), vec![extra]].concat();
    assert!(
        npm.prepare_if_needed(&store, &expanded, &registry.transport)
            .await
            .unwrap()
            .is_none()
    );
    store.reconcile(&expanded, node_stage).unwrap();
    assert_eq!(registry.requests.lock().unwrap().len(), 2);
    for damaged in [
        "bin/npm",
        "tools/npm/node_modules/resource/data.json",
        "tools/node/bin/node",
    ] {
        let bin = store.current().unwrap().unwrap().bin;
        let path = bin.parent().unwrap().join(damaged);
        if damaged == "bin/npm" {
            fs::remove_file(&path).unwrap();
            std::os::unix::fs::symlink("../tools/npm/bin/npm-cli.js", &path).unwrap();
        } else {
            fs::write(&path, "corrupt").unwrap();
        }
        let before = manifest(repo.path());
        assert!(
            npm.prepare_if_needed(&store, &desired, &registry.transport)
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(manifest(repo.path()), before);
        install(&mut store, &node, &npm, &registry).await.unwrap();
        assert!(store.is_current(&desired).unwrap());
    }
}

#[tokio::test]
async fn both_hashes_and_exact_package_layout_are_mandatory() {
    let good = archive(&package().to_string(), "bin/npm-cli.js");
    let mut cases = vec![(good.clone(), "sha256"), (good, "sha512")];
    for (pointer, wrong) in [
        ("/name", json!("pnpm")),
        ("/version", json!("11.0.1")),
        ("/bin/npm", json!("other.js")),
        ("/bin/npx", json!(false)),
    ] {
        let mut package = package();
        *package.pointer_mut(pointer).unwrap() = wrong;
        cases.push((archive(&package.to_string(), "bin/npm-cli.js"), "package"));
    }
    cases.push((
        archive("{\"name\":\"npm\",\"name\":\"npm\"}", "bin/npm-cli.js"),
        "package",
    ));
    cases.push((archive(&package().to_string(), "bin/wrong.js"), "layout"));
    let mut extra_bin = package();
    extra_bin["bin"]["unexpected"] = json!("bin/npm-cli.js");
    cases.push((archive(&extra_bin.to_string(), "bin/npm-cli.js"), "package"));
    for (bytes, expected) in cases {
        let mut value = fixture(&bytes);
        let mut meta = metadata(&bytes);
        if expected == "sha256" {
            value
                .pointer_mut(&format!("{ARTIFACT}/sha256"))
                .unwrap()
                .clone_from(&json!("00".repeat(32)));
        }
        if expected == "sha512" {
            meta["dist"]["integrity"] = metadata(b"wrong")["dist"]["integrity"].clone();
        }
        let (node, npm) = plans(&value, None);
        let registry = serve(meta, bytes).await;
        let repo = tempfile::tempdir().unwrap();
        let mut store = Store::open(repo.path()).unwrap();
        store
            .reconcile(&[node.inventory_tool().clone()], node_stage)
            .unwrap();
        let before = manifest(repo.path());
        let error = install(&mut store, &node, &npm, &registry)
            .await
            .unwrap_err();
        let kind = match error {
            Error::Download(turborepo_download::Error::DigestMismatch) => "sha256",
            Error::Download(turborepo_download::Error::Sha512DigestMismatch) => "sha512",
            Error::Archive(turborepo_archive::Error::LayoutMismatch) => "layout",
            Error::InvalidPackage => "package",
            _ => panic!("unexpected error: {error}"),
        };
        assert_eq!(kind, expected);
        assert_eq!(manifest(repo.path()), before);
    }
}

#[tokio::test]
async fn authored_constraints_and_node_identity_prevent_unverified_reuse() {
    let bytes = archive(&package().to_string(), "bin/npm-cli.js");
    let value = fixture(&bytes);
    let (node, npm) = plans(&value, None);
    let registry = serve(metadata(&bytes), bytes.clone()).await;
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let mut wrong_node = node.inventory_tool().clone();
    wrong_node.artifact_sha256 = "22".repeat(32);
    for desired in [
        vec![npm.inventory_tool().clone()],
        vec![npm.inventory_tool().clone(), wrong_node],
    ] {
        assert!(matches!(
            npm.prepare_if_needed(&store, &desired, &registry.transport)
                .await,
            Err(Error::NodeBinding)
        ));
    }
    assert!(registry.requests.lock().unwrap().is_empty());
    install(&mut store, &node, &npm, &registry).await.unwrap();
    let before = manifest(repo.path());
    for (algorithm, digest) in [
        ("sha512", "00".repeat(64)),
        ("sha256", "00".repeat(32)),
        ("sha512", "bad".into()),
    ] {
        let pin = CorepackIntegrity { algorithm, digest };
        let (_, pinned) = plans(&value, Some(&pin));
        assert_ne!(npm.inventory_tool(), pinned.inventory_tool());
        let registry = serve(metadata(&bytes), bytes.clone()).await;
        assert!(
            install(&mut store, &node, &pinned, &registry)
                .await
                .is_err()
        );
        assert_eq!(registry.requests.lock().unwrap().len(), 1);
        assert_eq!(manifest(repo.path()), before);
    }
    let pin = CorepackIntegrity {
        algorithm: "sha512",
        digest: format!("{:x}", Sha512::digest(&bytes)),
    };
    let (_, pinned) = plans(&value, Some(&pin));
    install(&mut store, &node, &pinned, &registry)
        .await
        .unwrap();
    assert_eq!(registry.requests.lock().unwrap().len(), 4);
    let mut changed = value.clone();
    changed["tools"]["node"]["installation"]["artifacts"]["macos-arm64"]["distribution"]
        ["sha256"] = json!("22".repeat(32));
    let (_, new_npm) = plans(&changed, None);
    assert_ne!(npm.inventory_tool(), new_npm.inventory_tool());
    assert!(matches!(
        NpmPlan::from_lock(&lock(&changed), TARGET, &node, None),
        Err(Error::NodeBinding)
    ));
    assert!(matches!(
        new_npm
            .prepare_if_needed(
                &store,
                &[
                    node.inventory_tool().clone(),
                    new_npm.inventory_tool().clone()
                ],
                &registry.transport
            )
            .await,
        Err(Error::NodeBinding)
    ));
    assert_eq!(registry.requests.lock().unwrap().len(), 4);
}

#[tokio::test]
async fn registry_metadata_identity_integrity_and_origin_fail_closed() {
    let bytes = archive(&package().to_string(), "bin/npm-cli.js");
    for (pointer, wrong) in [
        ("/name", json!("pnpm")),
        ("/version", json!("11.0.1")),
        ("/dist/tarball", json!("https://evil.test/npm.tgz")),
        (
            "/dist/tarball",
            json!("https://registry.npmjs.org/npm/-/../npm-11.0.0.tgz"),
        ),
        ("/dist/integrity", json!("sha1-only")),
    ] {
        let mut meta = metadata(&bytes);
        *meta.pointer_mut(pointer).unwrap() = wrong;
        let registry = serve(meta, bytes.clone()).await;
        let (node, npm) = plans(&fixture(&bytes), None);
        let repo = tempfile::tempdir().unwrap();
        let mut store = Store::open(repo.path()).unwrap();
        assert!(install(&mut store, &node, &npm, &registry).await.is_err());
        assert!(store.current().unwrap().is_none());
        assert_eq!(registry.requests.lock().unwrap().len(), 1);
    }
}

#[test]
fn lock_ownership_mappings_and_unqualified_targets_fail_closed() {
    let bytes = archive(&package().to_string(), "bin/npm-cli.js");
    let value = fixture(&bytes);
    let (node, _) = plans(&value, None);
    for platform in [
        Platform::WindowsX64,
        Platform::WindowsArm64,
        Platform::LinuxX64Musl,
        Platform::LinuxArm64Musl,
        Platform::Any,
    ] {
        assert!(matches!(
            NpmPlan::from_lock(&lock(&value), platform, &node, None),
            Err(Error::UnsupportedTarget)
        ));
    }
    for (pointer, wrong) in [
        (
            format!("{ARTIFACT}/url"),
            json!("https://evil.test/npm.tgz"),
        ),
        (format!("{ARTIFACT}/rootPrefix"), json!("other")),
        (format!("{ARTIFACT}/format"), json!("tar")),
        (
            format!("{ARTIFACT}/executables"),
            json!({"npm":"bin/npm-cli.js"}),
        ),
        (
            format!("{ARTIFACT}/executables/npx"),
            json!("bin/npm-cli.js"),
        ),
        ("/tools/npm/adapter".into(), json!("pnpm")),
        ("/tools/npm/options".into(), json!({"other":["value"]})),
    ] {
        let mut changed = value.clone();
        if pointer == "/tools/npm/options" {
            changed["tools"]["npm"]["options"] = wrong;
        } else {
            *changed.pointer_mut(&pointer).unwrap() = wrong;
        }
        assert!(matches!(
            NpmPlan::from_lock(&lock(&changed), TARGET, &node, None),
            Err(Error::InvalidLock)
        ));
    }
    let mut collision = value.clone();
    collision["tools"]["node"]["options"] = json!({"bundled-npm":["11.0.0"]});
    collision["tools"]["node"]["installation"]["artifacts"]["macos-arm64"]["distribution"]
        ["executables"] = json!({"node":"bin/node","npm":"bin/npm","npx":"bin/npx"});
    // Even an equal pin cannot invent duplicate ownership. TURBO-6326 owns
    // the bundled alternative; this independent registry plan never selects it.
    assert!(Lock::parse(&serde_json::to_vec(&collision).unwrap()).is_err());
    let mut unsafe_path = value.clone();
    *unsafe_path
        .pointer_mut(&format!("{ARTIFACT}/executables/npm"))
        .unwrap() = json!("../npm-cli.js");
    assert!(Lock::parse(&serde_json::to_vec(&unsafe_path).unwrap()).is_err());
    for origin in [
        "http://localhost:1234",
        "http://example.com",
        "http://127.0.0.1:1234/path",
        "http://user@127.0.0.1",
    ] {
        assert!(NpmTransport::loopback_http_for_tests(origin).is_err());
    }
}
