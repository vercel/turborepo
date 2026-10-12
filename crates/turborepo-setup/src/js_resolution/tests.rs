#![allow(clippy::unwrap_used)]
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::Path,
    sync::{Arc, Mutex},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256, Sha512};

use super::*;
use crate::{
    NodeArtifact, bundled_npm,
    lock::{
        Snapshot,
        reconcile::{self, Mode},
    },
    node::lock_target,
    node_resolution::PLATFORMS,
    source_policy::test_support::LoopbackServer,
};

mod npm;

const PNPM_PATHS: [&str; 2] = ["/pnpm/10.0.0", "/pnpm/-/pnpm-10.0.0.tgz"];

fn tar(files: &[(&str, String)]) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for (path, data) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, path, data.as_bytes()).unwrap();
    }
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&tar.into_inner().unwrap()).unwrap();
    gzip.finish().unwrap()
}
fn pnpm_bytes() -> Vec<u8> {
    let package = json!({"name":"pnpm","version":"10.0.0",
        "bin":{"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"},"scripts":{"install":"exit 99"}});
    let script = "throw new Error('must not execute');";
    tar(&[
        ("package/package.json", package.to_string()),
        ("package/bin/pnpm.cjs", script.into()),
        ("package/bin/pnpx.cjs", script.into()),
    ])
}
fn registry_routes(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let metadata = json!({"name":"pnpm","version":"10.0.0","dist":{
        "tarball":"https://registry.npmjs.org/pnpm/-/pnpm-10.0.0.tgz",
        "integrity":format!("sha512-{}", STANDARD.encode(Sha512::digest(bytes)))}});
    vec![
        (PNPM_PATHS[0].into(), metadata.to_string().into_bytes()),
        (PNPM_PATHS[1].into(), bytes.to_vec()),
    ]
}
fn node_bytes(platform: Platform, version: &str) -> Vec<u8> {
    if matches!(platform, Platform::WindowsX64 | Platform::WindowsArm64) {
        // Real empty ZIP, with a platform/version comment; metadata tests do
        // not download/extract Node or claim Windows installation support.
        let comment = format!("{platform:?}-{version}");
        let mut zip = b"PK\x05\x06\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0".to_vec();
        zip.extend_from_slice(&(comment.len() as u16).to_le_bytes());
        zip.extend_from_slice(comment.as_bytes());
        zip
    } else {
        tar(&[(
            "bin/node",
            format!("{platform:?}-{version}: must not execute"),
        )])
    }
}
fn node_routes(versions: &[&str]) -> Vec<(String, Vec<u8>)> {
    let index: Vec<_> = versions
        .iter()
        .map(|v| {
            json!({"version":format!("v{v}"),
        "lts":"Krypton","npm":"11.6.1","files":["osx-x64-tar","osx-arm64-tar",
        "linux-x64","linux-arm64","win-x64-zip","win-arm64-zip"]})
        })
        .collect();
    let mut routes = vec![(
        "/dist/index.json".into(),
        serde_json::to_vec(&index).unwrap(),
    )];
    for version in versions {
        let mut sums = String::new();
        for platform in PLATFORMS {
            let (target, _) = lock_target(platform).unwrap();
            let artifact =
                NodeArtifact::for_platform(&semver::Version::parse(version).unwrap(), target)
                    .unwrap();
            sums.push_str(&format!(
                "{:x}  {}\n",
                Sha256::digest(node_bytes(platform, version)),
                artifact.filename()
            ));
        }
        routes.push((
            format!("/dist/v{version}/SHASUMS256.txt"),
            sums.into_bytes(),
        ));
    }
    routes
}
struct World {
    _server: LoopbackServer,
    node: NodeTransport,
    registry: RegistryTransport,
    requests: Arc<Mutex<Vec<String>>>,
}
impl World {
    fn new(routes: Vec<(String, Vec<u8>)>, hook: impl Fn(&str) + Send + 'static) -> Self {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = requests.clone();
        let server = LoopbackServer::new(routes, move |path| {
            log.lock().unwrap().push(path.into());
            hook(path);
        })
        .unwrap();
        Self {
            node: NodeTransport::loopback_http_for_tests(server.origin()).unwrap(),
            registry: RegistryTransport::loopback_http_for_tests(server.origin()).unwrap(),
            _server: server,
            requests,
        }
    }
    fn paths(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}
fn root(node: Option<&str>, manifest: Value) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .arg(root.path())
            .status()
            .unwrap()
            .success()
    );
    fs::write(root.path().join(".gitignore"), "/.turbo/\n").unwrap();
    if let Some(node) = node {
        fs::write(root.path().join(".nvmrc"), node).unwrap();
    }
    write_manifest(root.path(), manifest);
    root
}
fn write_manifest(root: &Path, value: Value) {
    fs::write(root.join("package.json"), value.to_string()).unwrap();
}
fn lock_bytes(root: &Path) -> Option<Vec<u8>> {
    fs::read(root.join("turbo.lock")).ok()
}
fn apply(
    root: &Path,
    mode: Mode,
    offline: bool,
    world: &World,
) -> Result<reconcile::Outcome, reconcile::Error> {
    let snapshot = Snapshot::capture(root).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    reconcile::reconcile(&snapshot, mode, offline, |request| {
        let before = lock_bytes(root);
        let manifest = fs::read(root.join("package.json")).unwrap();
        let result = runtime.block_on(resolve(request, &world.node, &world.registry));
        assert_eq!(lock_bytes(root), before); // Resolver never publishes.
        // Only explicit response-hook drift may change this sentinel.
        if result.is_ok() {
            assert_eq!(fs::read(root.join("package.json")).unwrap(), manifest);
        }
        result.map_err(|e| reconcile::Error::Resolution(e.to_string()))
    })
}
fn fixture(versions: &[&str]) -> World {
    let mut routes = node_routes(versions);
    routes.extend(registry_routes(&pnpm_bytes()));
    World::new(routes, |_| {})
}
fn save(root: &Path, lock: &Lock) {
    fs::write(root.join("turbo.lock"), lock.canonical_bytes().unwrap()).unwrap();
}
fn seed(root: &Path) -> Lock {
    let lock = apply(root, Mode::NoLock, false, &fixture(&["24.0.0"]))
        .unwrap()
        .lock;
    save(root, &lock);
    lock
}
fn rejected(root: &Path, world: &World, offline: bool, paths: &[&str]) {
    let before = lock_bytes(root);
    assert!(apply(root, Mode::Local, offline, world).is_err());
    assert_eq!(world.paths(), paths);
    assert_eq!(lock_bytes(root), before);
    assert!(!root.join(".turbo").exists());
}

#[test]
fn exact_range_alias_portable_cohort_and_all_native_integrities() {
    let bytes = pnpm_bytes();
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    let sha512 = format!("{:x}", Sha512::digest(&bytes));
    for (request, expected) in [
        ("v24.0.0", "24.0.0"),
        ("24.x", "24.1.0"),
        ("lts/*", "24.1.0"),
    ] {
        let repo = root(
            Some(request),
            json!({"packageManager":format!("pnpm@10.0.0+sha256.{sha256}"),
            "engines":{"node":">=24 <25"}, "devEngines":{"packageManager":[
                {"name":"pnpm","version":format!("10.0.0+sha512.{sha512}")},
                {"name":"pnpm","version":format!("10.0.0+sha512.{sha512}")} ]}}),
        );
        let world = fixture(&["25.0.0", "24.1.0", "24.0.0"]);
        let p = repo.path();
        let result = apply(p, Mode::NoLock, false, &world).unwrap();
        assert!(result.publication.is_none());
        assert!(lock_bytes(p).is_none());
        assert!(!p.join(".turbo").exists());
        let node = &result.lock.tools()["node"];
        assert_eq!(node.version, expected);
        assert_eq!(node.options["bundled-npm"], ["11.6.1"]);
        let Installation::Managed { artifacts } = &node.installation else {
            panic!()
        };
        assert_eq!(artifacts.len(), 6);
        for platform in PLATFORMS {
            let artifact = &artifacts[&platform]["distribution"];
            assert_eq!(
                artifact.sha256,
                format!("{:x}", Sha256::digest(node_bytes(platform, expected)))
            );
            assert!(artifact.url.starts_with("https://nodejs.org/dist/"));
            assert_eq!(artifact.executables.len(), 3);
            NodePlan::from_lock(&result.lock, platform).unwrap();
        }
        let Installation::Managed { artifacts } = &result.lock.tools()["pnpm"].installation else {
            panic!()
        };
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[&Platform::Any]["package"].sha256, sha256);
        assert_eq!(artifacts[&Platform::Any]["package"].executables.len(), 2);
        let snapshot = Snapshot::capture(p).unwrap();
        assert!(result.lock.matches_native(snapshot.declarations()).unwrap());
        for (id, tool) in result.lock.tools() {
            assert_eq!(&tool.declarations, &snapshot.declarations()[id]);
        }
        assert_eq!(
            world.paths(),
            [
                "/dist/index.json".into(),
                format!("/dist/v{expected}/SHASUMS256.txt"),
                "/pnpm/10.0.0".into(),
                "/pnpm/-/pnpm-10.0.0.tgz".into()
            ]
        );
        let canonical = result.lock.canonical_bytes().unwrap();
        assert_eq!(Lock::parse(&canonical).unwrap(), result.lock);
    }
    // Identical exact dev-only alternatives are not flattened to a first entry.
    let repo = root(
        Some("24.0.0"),
        json!({"devEngines":{"packageManager":[
        {"name":"pnpm","version":"10.0.0"},{"name":"pnpm","version":"10.0.0"}]}}),
    );
    apply(repo.path(), Mode::NoLock, false, &fixture(&["24.0.0"])).unwrap();
}

#[test]
fn manager_only_add_remove_preserves_node_and_never_reads_index() {
    for node_only in [false, true] {
        let repo = root(Some("24.x"), json!({}));
        let p = repo.path();
        let mut old = seed(p).document().clone();
        if node_only {
            let node = old.tools.get_mut("node").unwrap();
            node.options.clear();
            let Installation::Managed { artifacts } = &mut node.installation else {
                panic!()
            };
            for artifact in artifacts.values_mut().flat_map(BTreeMap::values_mut) {
                artifact.executables.retain(|name, _| name == "node");
            }
        }
        let old = Lock::new(old).unwrap();
        save(p, &old);
        write_manifest(p, json!({"packageManager":"pnpm@10.0.0"}));
        // No Node routes: any accidental metadata refresh fails this test.
        let world = World::new(registry_routes(&pnpm_bytes()), |_| {});
        let next = apply(p, Mode::Local, false, &world).unwrap().lock;
        assert_eq!(next.tools()["node"], old.tools()["node"]);
        assert_eq!(world.paths(), ["/pnpm/10.0.0", "/pnpm/-/pnpm-10.0.0.tgz"]);
        assert_eq!(lock_bytes(p).unwrap(), next.canonical_bytes().unwrap());
        write_manifest(p, json!({}));
        let world = World::new(vec![], |_| {});
        let removed = apply(p, Mode::Local, true, &world).unwrap().lock;
        assert_eq!(removed.tools(), old.tools());
        assert!(world.paths().is_empty());
        fs::remove_file(p.join(".nvmrc")).unwrap();
        assert!(
            apply(p, Mode::Local, true, &world)
                .unwrap()
                .lock
                .tools()
                .is_empty()
        );
    }
}

#[test]
fn node_drift_preserves_exact_manager_and_explicit_refresh_is_distinct() {
    for platforms in [vec![Platform::Any], PLATFORMS.to_vec()] {
        let repo = root(Some("24.x"), json!({"packageManager":"pnpm@10.0.0"}));
        let p = repo.path();
        let old = apply(p, Mode::Local, false, &fixture(&["24.0.0"]))
            .unwrap()
            .lock;
        let mut document = old.document().clone();
        let Installation::Managed { artifacts } =
            &mut document.tools.get_mut("pnpm").unwrap().installation
        else {
            panic!()
        };
        *artifacts = platforms
            .into_iter()
            .map(|p| (p, artifacts[&Platform::Any].clone()))
            .collect();
        let old = Lock::new(document).unwrap();
        save(p, &old);
        let no_traffic = World::new(vec![], |_| {});
        assert_eq!(apply(p, Mode::Local, true, &no_traffic).unwrap().lock, old);
        assert!(no_traffic.paths().is_empty());
        fs::write(p.join(".nvmrc"), "^24.0.0").unwrap();
        let world = World::new(node_routes(&["24.0.0", "24.1.0"]), |_| {});
        let next = apply(p, Mode::Local, false, &world).unwrap().lock;
        assert_eq!(next.tools()["node"].version, "24.1.0");
        assert_eq!(next.tools()["pnpm"], old.tools()["pnpm"]);
        assert_eq!(
            world.paths(),
            ["/dist/index.json", "/dist/v24.1.0/SHASUMS256.txt"]
        );
        let refreshed = apply(p, Mode::Refresh, false, &fixture(&["24.2.0"]))
            .unwrap()
            .lock;
        assert_eq!(refreshed.tools()["node"].version, "24.2.0");
    }
}

#[test]
fn removed_bundled_npm_keeps_node_identity_and_export_ownership() {
    let repo = root(Some("24.x"), json!({}));
    let node = seed(repo.path()).tools()["node"].clone();
    write_manifest(repo.path(), json!({"packageManager":"npm@11.6.1"}));
    let snapshot = Snapshot::capture(repo.path()).unwrap();
    let npm = bundled_npm::resolve(&snapshot, &node).unwrap();
    let old = Lock::new(Document {
        schema_version: lock::SCHEMA_VERSION,
        tools: BTreeMap::from([("node".into(), node.clone()), ("npm".into(), npm)]),
    })
    .unwrap();
    save(repo.path(), &old);
    write_manifest(repo.path(), json!({}));
    let world = World::new(vec![], |_| {});
    let result = apply(repo.path(), Mode::Local, true, &world).unwrap().lock;
    assert_eq!(result.tools(), &BTreeMap::from([("node".into(), node)]));
    assert!(world.paths().is_empty());
}

#[test]
fn unsupported_scope_and_offline_misses_fail_before_traffic_or_publication() {
    let sha512 = format!("{:x}", Sha512::digest(pnpm_bytes()));
    for manifest in [
        json!({"packageManager":"pnpm@10.x"}),
        json!({"devEngines":{"packageManager":{"name":"pnpm"}}}),
        json!({"devEngines":{"packageManager":[
            {"name":"pnpm","version":"10.0.0"},{"name":"pnpm","version":"10.1.0"}]}}),
        json!({"packageManager":format!("pnpm@10.0.0+sha1.{}", "a".repeat(40))}),
        json!({"packageManager":"pnpm@10.0.0", "devEngines":{"packageManager":[
            {"name":"pnpm","version":format!("10.0.0+sha512.{sha512}")},
            {"name":"pnpm","version":"10.0.0"}]}}),
    ] {
        let repo = root(Some("24.x"), manifest);
        rejected(repo.path(), &World::new(vec![], |_| {}), false, &[]);
    }
    for (node, offline) in [(None, false), (Some("24.x"), true)] {
        let repo = root(node, json!({"packageManager":"pnpm@10.0.0"}));
        rejected(repo.path(), &World::new(vec![], |_| {}), offline, &[]);
    }
}

#[test]
fn integrity_failures_and_ambiguous_applicable_alternatives_never_publish() {
    let bytes = pnpm_bytes();
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    for (manifest, corrupt) in [
        (
            json!({"packageManager":format!("pnpm@10.0.0+sha256.{}", "a".repeat(64))}),
            false,
        ),
        (
            json!({"packageManager":format!("pnpm@10.0.0+sha512.{}", "a".repeat(128))}),
            false,
        ),
        (
            json!({"packageManager":format!("pnpm@10.0.0+sha256.{sha256}"), "devEngines":{
            "packageManager":{"name":"pnpm","version":format!("10.0.0+sha512.{}", "a".repeat(128))}}}),
            false,
        ),
        (
            json!({"packageManager":format!("pnpm@10.0.0+sha256.{sha256}")}),
            true,
        ),
    ] {
        let repo = root(Some("24.x"), json!({}));
        let p = repo.path();
        seed(p);
        write_manifest(p, manifest);
        let mut routes = registry_routes(&bytes);
        if corrupt {
            routes[1].1 = b"corrupted archive".to_vec();
        }
        let world = World::new(routes, |_| {});
        rejected(p, &world, false, &PNPM_PATHS);
    }
}

#[test]
fn fresh_source_drift_revalidation_fails_before_publication_and_next_traffic() {
    let repo = root(Some("24.x"), json!({}));
    let p = repo.path();
    let old = seed(p).canonical_bytes().unwrap();
    write_manifest(p, json!({"packageManager":"pnpm@10.0.0"}));
    let path = p.join("package.json");
    let world = World::new(registry_routes(&pnpm_bytes()), move |request| {
        if request.ends_with(".tgz") {
            fs::write(&path, "{}").unwrap();
        }
    });
    rejected(p, &world, false, &PNPM_PATHS);
    assert_eq!(lock_bytes(p).unwrap(), old);
    // Already stale captured inputs are refused by resolve itself, not merely
    // by reconcile's outer entry check.
    fs::write(p.join(".nvmrc"), "24.0.0").unwrap();
    let snapshot = Snapshot::capture(p).unwrap();
    let world = World::new(vec![], |_| {});
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    assert!(
        reconcile::reconcile(&snapshot, Mode::Local, false, |request| {
            fs::write(p.join(".nvmrc"), "24.1.0").unwrap();
            rt.block_on(resolve(request, &world.node, &world.registry))
                .map_err(|e| reconcile::Error::Resolution(e.to_string()))
        })
        .is_err()
    );
    assert!(world.paths().is_empty());
    assert_eq!(lock_bytes(p).unwrap(), old);
}

#[test]
fn preserved_payload_shapes_fail_before_either_transport() {
    for id in ["node", "pnpm"] {
        for (field, bad) in [
            ("url", json!("https://example.invalid/pnpm.tgz")),
            ("format", json!("tar")),
            ("rootPrefix", json!("wrong")),
            ("destination", json!("nested")),
            ("executables", json!({"other":"bin/other"})),
            ("system", json!({"kind":"verify-system","executables":[id]})),
        ] {
            let repo = root(Some("24.x"), json!({"packageManager":"pnpm@10.0.0"}));
            let p = repo.path();
            let mut document = serde_json::to_value(seed(p).document()).unwrap();
            let tool = &mut document["tools"][id];
            if field == "system" {
                tool["installation"] = bad;
            } else {
                let (platform, part) = if id == "node" {
                    ("windows-arm64", "distribution")
                } else {
                    ("any", "package")
                };
                tool["installation"]["artifacts"][platform][part][field] = bad;
            }
            let previous = Lock::parse(&serde_json::to_vec(&document).unwrap()).unwrap();
            save(p, &previous);
            if id == "node" {
                write_manifest(
                    p,
                    json!({"packageManager":"pnpm@10.0.0","devEngines":{
                    "packageManager":{"name":"pnpm","version":"10.x"}}}),
                );
            } else {
                fs::write(p.join(".nvmrc"), "^24.0.0").unwrap();
            }
            rejected(p, &fixture(&["24.1.0"]), false, &[]);
        }
    }
}

#[test]
fn incomplete_node_metadata_fails_without_publishing_a_partial_cohort() {
    for missing_npm in [false, true] {
        let repo = root(Some("24.x"), json!({"packageManager":"pnpm@10.0.0"}));
        let mut routes = node_routes(&["24.0.0"]);
        if missing_npm {
            let mut index: Value = serde_json::from_slice(&routes[0].1).unwrap();
            index[0]["npm"] = Value::Null;
            routes[0].1 = serde_json::to_vec(&index).unwrap();
        } else {
            routes[1].1 = b"aa  incomplete\n".to_vec();
        }
        let world = World::new(routes, |_| {});
        rejected(
            repo.path(),
            &world,
            false,
            &["/dist/index.json", "/dist/v24.0.0/SHASUMS256.txt"],
        );
    }
}
