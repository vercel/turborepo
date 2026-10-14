#![allow(clippy::unwrap_used)]

use std::{fs, io::Write, path::Path, process::Command};

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha224, Sha256, Sha512};

use super::*;
use crate::{
    package_manager::discover_package_manager, registry_metadata::RegistryMetadataError,
    source_policy::test_support::LoopbackServer,
};

const TARGET: Platform = Platform::MacosArm64;

struct Line {
    version: &'static str,
    package: &'static str,
    tarball: &'static str,
    bin: &'static str,
}
const CLASSIC: Line = Line {
    version: "1.22.22",
    package: "yarn",
    tarball: "/yarn/-/yarn-1.22.22.tgz",
    bin: "./bin/yarn.js",
};

fn bundle(line: &Line) -> String {
    format!("// yarn {} bundle\n", line.version)
}
fn archive(line: &Line) -> Vec<u8> {
    let package = json!({"name":line.package,"version":line.version,
        "bin":{"yarn":line.bin,"yarnpkg":line.bin},"scripts":{"preinstall":"touch hook-ran"}});
    let mut tar = tar::Builder::new(Vec::new());
    for (path, data) in [
        ("package.json", package.to_string()),
        ("bin/yarn.js", bundle(line)),
        ("lib/cli.js", "adjacent resource".into()),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, format!("package/{path}"), data.as_bytes())
            .unwrap();
    }
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&tar.into_inner().unwrap()).unwrap();
    gzip.finish().unwrap()
}
fn serve(line: &Line, bytes: &[u8]) -> (LoopbackServer, YarnTransport) {
    let metadata = json!({"name":line.package,"version":line.version,"dist":{
        "tarball":format!("https://registry.npmjs.org{}", line.tarball),
        "integrity":format!("sha512-{}", STANDARD.encode(Sha512::digest(bytes)))
    }});
    let server = LoopbackServer::new(
        [
            (
                format!("/{}/{}", line.package, line.version),
                metadata.to_string().into_bytes(),
            ),
            (line.tarball.to_owned(), bytes.to_vec()),
        ],
        |_| {},
    )
    .unwrap();
    let transport = YarnTransport::loopback_http_for_tests(server.origin()).unwrap();
    (server, transport)
}
fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn sha512(bytes: &[u8]) -> CorepackIntegrity {
    CorepackIntegrity {
        algorithm: "sha512",
        digest: hex::encode(Sha512::digest(bytes)),
    }
}
fn artifact(line: &Line, bytes: &[u8]) -> Value {
    json!({"url":format!("https://registry.npmjs.org{}", line.tarball),
        "sha256":sha256(bytes),"format":"tar-gz","rootPrefix":"package",
        "executables":{"yarn":"bin/yarn.js","yarnpkg":"bin/yarn.js"}})
}
fn lock(artifact: Value, version: &str, request: &str) -> Value {
    json!({"schemaVersion":0,"tools":{
        "node":{"adapter":"node","version":"24.0.0","declarations":[{"file":".nvmrc","request":"24.0.0"}],
            "installation":{"kind":"managed","artifacts":{"macos-arm64":{"distribution":{
                "url":"https://nodejs.org/dist/v24.0.0/node-v24.0.0-darwin-arm64.tar.gz",
                "sha256":"11".repeat(32),"format":"tar-gz","rootPrefix":"node-v24.0.0-darwin-arm64",
                "executables":{"node":"bin/node"}}}}}},
        "yarn":{"adapter":"yarn","version":version,
            "declarations":[{"file":"package.json","field":"/packageManager","request":request}],
            "installation":{"kind":"managed","artifacts":{"any":{"package":artifact}}}}
    }})
}
fn parse(value: &Value) -> Lock {
    Lock::parse(&serde_json::to_vec(value).unwrap()).unwrap()
}
fn plan(lock: &Value, request: &str) -> Result<(NodePlan, YarnPlan), Error> {
    let lock = parse(lock);
    let node = NodePlan::from_lock(&lock, TARGET).unwrap();
    let declaration = discover_package_manager(&json!({ "packageManager": request }))
        .unwrap()
        .unwrap();
    let yarn = YarnPlan::from_declaration(&lock, TARGET, &node, &declaration)?;
    Ok((node, yarn))
}
fn desired(node: &NodePlan, yarn: &YarnPlan) -> [Tool; 2] {
    [node.inventory_tool().clone(), yarn.inventory_tool().clone()]
}
// Builtins only: prints the launched entrypoint's first line and arguments.
fn node_stage(_: &Tool, destination: &Path) -> Result<(), turborepo_tool_install::Error> {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir(destination.join("bin"))?;
    let node = destination.join("bin/node");
    fs::write(
        &node,
        "#!/bin/sh\nIFS= read -r line < \"$1\"\nprintf '%s\\n' \"$line\"\nshift\nprintf '<%s>' \
         \"$@\"\n",
    )?;
    fs::set_permissions(node, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

/// Resolve, lock, prepare and reconcile `line`, then invoke both shims.
async fn locks_and_invokes_the_exact_bundle(line: &Line) {
    let bytes = archive(line);
    let (server, transport) = serve(line, &bytes);
    let resolved = transport
        .resolve_exact(Manager::Yarn, line.version, None)
        .await
        .unwrap();
    let resolved = serde_json::to_value(resolved.into_artifact()).unwrap();
    assert_eq!(resolved, artifact(line, &bytes));
    let request = format!("yarn@{}", line.version);
    let (node, yarn) = plan(&lock(resolved, line.version, &request), &request).unwrap();
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let desired = desired(&node, &yarn);
    let prepared = yarn
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
    let bin = store.current().unwrap().unwrap().bin;
    for name in ["yarn", "yarnpkg"] {
        let output = Command::new(bin.join(name))
            .args(["install", "--frozen-lockfile"])
            .current_dir(repo.path())
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("{}<install><--frozen-lockfile>", bundle(line))
        );
    }
    assert!(!repo.path().join("hook-ran").exists());
    let hits = server.hits();
    let reused = yarn.prepare_if_needed(&store, &desired, &transport).await;
    assert!(reused.unwrap().is_none());
    assert_eq!(server.hits(), hits);
}

#[tokio::test]
async fn classic_locks_and_invokes_the_exact_bundle_through_managed_node() {
    locks_and_invokes_the_exact_bundle(&CLASSIC).await;
}

#[tokio::test]
async fn classic_pins_cover_the_registry_archive() {
    let bytes = archive(&CLASSIC);
    let (server, transport) = serve(&CLASSIC, &bytes);
    let other = bundle(&CLASSIC).into_bytes();
    let (good, bad) = (sha512(&bytes), sha512(&other));
    let resolve = |pin| transport.resolve_exact(Manager::Yarn, CLASSIC.version, Some(pin));
    assert!(resolve(&bad).await.is_err());
    let resolved = resolve(&good).await.unwrap();
    resolved.verify_authored(Some(&good)).unwrap();
    assert!(resolved.verify_authored(Some(&bad)).is_err());

    let pinned = |pin: String| {
        let request = format!("yarn@{}+{pin}", CLASSIC.version);
        (
            lock(artifact(&CLASSIC, &bytes), CLASSIC.version, &request),
            request,
        )
    };
    let repo = tempfile::tempdir().unwrap();
    let store = Store::open(repo.path()).unwrap();
    let (value, request) = pinned(format!("sha256.{}", sha256(&bytes)));
    let (node, yarn) = plan(&value, &request).unwrap();
    let prepared = yarn
        .prepare_if_needed(&store, &desired(&node, &yarn), &transport)
        .await;
    assert!(prepared.unwrap().is_some());
    // Legacy Corepack SHA-224 pins also cover the verified archive bytes.
    let (value, request) = pinned(format!("sha224.{}", hex::encode(Sha224::digest(&bytes))));
    let (node, yarn) = plan(&value, &request).unwrap();
    assert!(
        yarn.prepare_if_needed(&store, &desired(&node, &yarn), &transport)
            .await
            .unwrap()
            .is_some()
    );
    let hits = server.hits();
    for (pin, requests) in [
        (format!("sha256.{}", sha256(&other)), 0),
        (format!("sha512.{}", sha512(&other).digest), 1),
        (format!("sha224.{}", "a".repeat(56)), 2),
    ] {
        let before = server.hits();
        let (value, request) = pinned(pin);
        let result = match plan(&value, &request) {
            Ok((node, yarn)) => yarn
                .prepare_if_needed(&store, &desired(&node, &yarn), &transport)
                .await
                .map(drop),
            Err(error) => Err(error),
        };
        assert!(result.is_err());
        assert_eq!(server.hits(), before + requests);
    }
    // SHA-256 conflicts fail before traffic; SHA-512 conflicts need only
    // metadata. SHA-224 is verified against the downloaded archive bytes.
    assert_eq!(server.hits(), hits + 3);
    assert!(store.current().unwrap().is_none());
}

#[test]
fn url_managers_mismatched_locks_and_hosts_fail_clearly() {
    let url = "https://repo.yarnpkg.com/4.5.0/packages/yarnpkg-cli/bin/yarn.js";
    for root in [
        json!({ "packageManager": format!("yarn@{url}") }),
        json!({"devEngines":{"packageManager":{"name":"yarn","version":url}}}),
    ] {
        let error = discover_package_manager(&root).unwrap_err().to_string();
        assert!(error.contains("URLs, paths and aliases are unsupported"));
    }
    let valid = lock(artifact(&CLASSIC, b"bytes"), "1.22.22", "yarn@1.22.22");
    let node = NodePlan::from_lock(&parse(&valid), TARGET).unwrap();
    assert!(YarnPlan::from_lock(&parse(&valid), TARGET, &node, None).is_ok());
    assert!(matches!(
        YarnPlan::from_lock(&parse(&valid), Platform::WindowsX64, &node, None),
        Err(Error::UnsupportedTarget)
    ));
    let (location, executables) = (
        "/installation/artifacts/any/package/url",
        "/installation/artifacts/any/package/executables",
    );
    for (pointer, value) in [
        ("/version", json!("1.22.21")),
        (
            location,
            json!("https://registry.npmjs.org/@yarnpkg/cli-dist/-/cli-dist-1.22.22.tgz"),
        ),
        (location, json!(url)),
        (
            executables,
            json!({"yarn":"bin/yarn.js","corepack":"bin/yarn.js"}),
        ),
        (executables, json!({"yarnpkg":"bin/yarn.js"})),
    ] {
        let mut value_lock = valid.clone();
        *value_lock["tools"]["yarn"].pointer_mut(pointer).unwrap() = value;
        let lock = parse(&value_lock);
        assert!(matches!(
            YarnPlan::from_lock(&lock, TARGET, &node, None),
            Err(Error::InvalidLock)
        ));
    }
    // TODO(TURBO-6369): Yarn 2+ is the `@yarnpkg/cli-dist` bundle.
    let mut modern = valid.clone();
    modern["tools"]["yarn"]["version"] = json!("4.5.0");
    assert!(matches!(
        YarnPlan::from_lock(&parse(&modern), TARGET, &node, None),
        Err(Error::Metadata(RegistryMetadataError::UnsupportedManager))
    ));
}
