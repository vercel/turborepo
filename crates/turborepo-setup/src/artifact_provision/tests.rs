// Store generations and shims are qualified on Unix only.
#![cfg(unix)]

use std::{
    collections::BTreeMap,
    io::Write,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use turborepo_tool_install::Outcome;

use super::*;

const PLATFORM: Platform = Platform::LinuxX64Gnu;
const SCRIPT: &str = "#!/bin/sh\necho fixture 1.2.3\n";

struct Upstream {
    origin: String,
    requests: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Upstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve(files: Vec<(&'static str, Vec<u8>)>) -> Upstream {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let count = requests.clone();
    let files: BTreeMap<_, _> = files.into_iter().collect();
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 8192];
            let n = socket.read(&mut request).await.unwrap();
            let line = String::from_utf8_lossy(&request[..n]).into_owned();
            let path = line.split(' ').nth(1).unwrap_or_default();
            count.fetch_add(1, Ordering::SeqCst);
            let (status, body) = match files.get(path) {
                Some(body) => ("200 OK", body.clone()),
                None => ("404 Not Found", Vec::new()),
            };
            let header = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(header.as_bytes()).await.unwrap();
            socket.write_all(&body).await.unwrap();
        }
    });
    Upstream {
        origin,
        requests,
        task,
    }
}

/// Entries are `(path, contents, mode)`. Paths are written verbatim.
fn tar_gz(entries: &[(&str, &str, u32)]) -> Vec<u8> {
    let mut archive = tar::Builder::new(Vec::new());
    for (path, contents, mode) in entries {
        let mut header = tar::Header::new_gnu();
        header.as_gnu_mut().unwrap().name[..path.len()].copy_from_slice(path.as_bytes());
        header.set_size(contents.len() as u64);
        header.set_mode(*mode);
        header.set_cksum();
        archive.append(&header, contents.as_bytes()).unwrap();
    }
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&archive.into_inner().unwrap()).unwrap();
    gzip.finish().unwrap()
}

fn archive() -> Vec<u8> {
    tar_gz(&[
        ("fixture-1.2.3/bin/fixture", SCRIPT, 0o755),
        ("fixture-1.2.3/share/fixture/data.txt", "resource", 0o644),
        ("fixture-1.2.3/lib/libfixture.so", "library", 0o644),
    ])
}

fn entry(version: &str, file: &str, bytes: &[u8], format: &str) -> Value {
    let mut artifact = json!({"url": format!("https://downloads.example.test/{file}"),
        "sha256": format!("{:x}", Sha256::digest(bytes)), "format": format,
        "executables": {"fixture": "bin/fixture"}});
    if format != "binary" {
        artifact["rootPrefix"] = json!("fixture-1.2.3");
    }
    json!({"adapter": ADAPTER, "version": version, "declarations": [{"file": "turbo.json",
        "field": "/setup/additionalTools/fixture", "request": version}],
        "installation": {"kind": "managed", "artifacts": {"linux-x64-gnu": {PART: artifact}}}})
}

fn plans(tools: Value) -> Result<Vec<ArtifactPlan>, Error> {
    let lock = json!({"schemaVersion": 0, "tools": tools});
    let lock = Lock::parse(&serde_json::to_vec(&lock).unwrap()).unwrap();
    ArtifactPlan::from_lock(&lock, PLATFORM)
}

fn plan(tool: Value) -> ArtifactPlan {
    plans(json!({"fixture": tool})).unwrap().remove(0)
}

async fn install(
    store: &mut Store,
    plans: &[ArtifactPlan],
    transport: &ArtifactTransport,
    preparation: Preparation,
) -> Result<Outcome, Error> {
    let desired: Vec<_> = plans.iter().map(|p| p.inventory_tool().clone()).collect();
    let mut prepared = Vec::new();
    for plan in plans {
        prepared.push(
            plan.prepare(store, &desired, transport, preparation)
                .await?,
        );
    }
    let stage = |tool: &Tool, path: &Path| {
        let index = desired.iter().position(|d| d == tool).unwrap();
        prepared[index].as_ref().unwrap().stage(tool, path)
    };
    Ok(match preparation {
        Preparation::IfNeeded => store.reconcile_checked(&desired, stage, || Ok(()))?,
        Preparation::Fresh => store.force_reconcile_checked(&desired, stage, || Ok(()))?,
    })
}

fn run(store: &Store, name: &str) -> String {
    let bin = store.current().unwrap().unwrap().bin;
    let output = std::process::Command::new(bin.join(name)).output().unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap()
}

#[tokio::test]
async fn archives_install_with_resources_and_repeat_without_network() {
    let bytes = archive();
    let upstream = serve(vec![("/fixture-1.2.3.tar.gz", bytes.clone())]).await;
    let transport = ArtifactTransport::loopback_http_for_tests(&upstream.origin).unwrap();
    let plans = [plan(entry(
        "1.2.3",
        "fixture-1.2.3.tar.gz",
        &bytes,
        "tar-gz",
    ))];
    assert_eq!(
        plans[0].source(),
        "https://downloads.example.test/fixture-1.2.3.tar.gz"
    );
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let ifneeded = Preparation::IfNeeded;
    let outcome = install(&mut store, &plans, &transport, ifneeded).await;
    assert_eq!(outcome.unwrap(), Outcome::Replaced);
    assert_eq!(run(&store, "fixture"), "fixture 1.2.3\n");
    let current = store.current().unwrap().unwrap();
    let tree = current.tool_tree(plans[0].inventory_tool()).unwrap();
    assert_eq!(
        fs::read_to_string(tree.join("share/fixture/data.txt")).unwrap(),
        "resource"
    );
    assert!(tree.join("lib/libfixture.so").is_file());

    let outcome = install(&mut store, &plans, &transport, ifneeded).await;
    assert_eq!(outcome.unwrap(), Outcome::Unchanged);
    assert_eq!(upstream.requests.load(Ordering::SeqCst), 1);
    // Force downloads and verifies fresh bytes even for a healthy tool.
    let outcome = install(&mut store, &plans, &transport, Preparation::Fresh).await;
    assert_eq!(outcome.unwrap(), Outcome::Replaced);
    assert_eq!(upstream.requests.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn binaries_install_at_their_declared_path() {
    let upstream = serve(vec![("/fixture", SCRIPT.into())]).await;
    let transport = ArtifactTransport::loopback_http_for_tests(&upstream.origin).unwrap();
    let plans = [plan(entry("1.2.3", "fixture", SCRIPT.as_bytes(), "binary"))];
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let outcome = install(&mut store, &plans, &transport, Preparation::IfNeeded).await;
    assert_eq!(outcome.unwrap(), Outcome::Replaced);
    assert_eq!(run(&store, "fixture"), "fixture 1.2.3\n");
}

#[tokio::test]
async fn failures_leave_the_previous_installation_usable() {
    let good = archive();
    let escape = "turborepo-additional-tool-escape";
    let traversal = tar_gz(&[
        ("fixture-1.2.3/bin/fixture", SCRIPT, 0o755),
        (&format!("fixture-1.2.3/../../{escape}"), "escaped", 0o644),
    ]);
    let missing = tar_gz(&[("fixture-1.2.3/share/fixture/data.txt", "resource", 0o644)]);
    let not_executable = tar_gz(&[("fixture-1.2.3/bin/fixture", SCRIPT, 0o644)]);
    let upstream = serve(vec![
        ("/good.tar.gz", good.clone()),
        ("/traversal.tar.gz", traversal.clone()),
        ("/missing.tar.gz", missing.clone()),
        ("/not-executable.tar.gz", not_executable.clone()),
    ])
    .await;
    let transport = ArtifactTransport::loopback_http_for_tests(&upstream.origin).unwrap();
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let ifneeded = Preparation::IfNeeded;
    let previous = [plan(entry("1.2.3", "good.tar.gz", &good, "tar-gz"))];
    install(&mut store, &previous, &transport, ifneeded)
        .await
        .unwrap();

    let mut checksum = entry("1.2.4", "good.tar.gz", &good, "tar-gz");
    let artifact = &mut checksum["installation"]["artifacts"]["linux-x64-gnu"][PART];
    artifact["sha256"] = json!("0".repeat(64));
    for (tool, expected) in [
        (checksum, "Download"),
        (
            entry("1.2.4", "traversal.tar.gz", &traversal, "tar-gz"),
            "Archive",
        ),
        (
            entry("1.2.4", "missing.tar.gz", &missing, "tar-gz"),
            "Archive",
        ),
        (
            entry("1.2.4", "not-executable.tar.gz", &not_executable, "tar-gz"),
            "Install",
        ),
    ] {
        let error = install(&mut store, &[plan(tool)], &transport, ifneeded)
            .await
            .unwrap_err();
        assert!(format!("{error:?}").starts_with(expected), "{error:?}");
        if expected != "Install" {
            assert!(error.to_string().contains("\"fixture\""), "{error}");
        }
        let current = store.current().unwrap().unwrap();
        assert_eq!(current.tools, [previous[0].inventory_tool().clone()]);
        assert_eq!(run(&store, "fixture"), "fixture 1.2.3\n");
    }
    for parent in [repo.path().to_path_buf(), std::env::temp_dir()] {
        assert!(!parent.join(escape).exists());
    }
}

#[tokio::test]
async fn removed_tools_leave_the_next_generation() {
    let bytes = archive();
    let upstream = serve(vec![("/fixture.tar.gz", bytes.clone())]).await;
    let transport = ArtifactTransport::loopback_http_for_tests(&upstream.origin).unwrap();
    let mut other = entry("2.0.0", "fixture.tar.gz", &bytes, "tar-gz");
    other["declarations"][0]["field"] = json!("/setup/additionalTools/other");
    let artifact = &mut other["installation"]["artifacts"]["linux-x64-gnu"][PART];
    artifact["executables"] = json!({"other": "bin/fixture"});
    let both = plans(
        json!({"fixture": entry("1.2.3", "fixture.tar.gz", &bytes, "tar-gz"),
        "other": other}),
    )
    .unwrap();
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let ifneeded = Preparation::IfNeeded;
    install(&mut store, &both, &transport, ifneeded)
        .await
        .unwrap();
    assert_eq!(run(&store, "other"), "fixture 1.2.3\n");

    let outcome = install(&mut store, &both[..1], &transport, ifneeded).await;
    assert_eq!(outcome.unwrap(), Outcome::Replaced);
    let current = store.current().unwrap().unwrap();
    assert_eq!(current.tools, [both[0].inventory_tool().clone()]);
    assert!(!current.bin.join("other").exists());
    // The remaining tool was copied from the healthy generation.
    assert_eq!(upstream.requests.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn plans_require_the_exact_locked_platform_and_shape() {
    let bytes = archive();
    let fixture = entry("1.2.3", "fixture.tar.gz", &bytes, "tar-gz");
    let lock = json!({"schemaVersion": 0, "tools": {"fixture": fixture}});
    let lock = Lock::parse(&serde_json::to_vec(&lock).unwrap()).unwrap();
    let error = ArtifactPlan::from_lock(&lock, Platform::LinuxArm64Gnu)
        .err()
        .unwrap();
    assert!(
        matches!(&error, Error::MissingTarget { tool, platform }
            if tool == "fixture" && platform == "linux-arm64-gnu"),
        "{error:?}"
    );
    assert!(error.to_string().contains("setup.additionalTools.fixture"));

    let mutations: [fn(&mut Value); 4] = [
        |tool| tool["options"] = json!({"flavor": ["x"]}),
        |tool| {
            tool["installation"]["artifacts"]["linux-x64-gnu"][PART]["destination"] = json!("opt")
        },
        |tool| {
            let part = &mut tool["installation"]["artifacts"]["linux-x64-gnu"][PART];
            part.as_object_mut().unwrap().remove("rootPrefix");
        },
        |tool| {
            let parts = &mut tool["installation"]["artifacts"]["linux-x64-gnu"];
            let mut second = parts[PART].clone();
            second["executables"] = json!({"helper": "bin/helper"});
            parts["second"] = second;
        },
    ];
    for mutate in mutations {
        let mut tool = entry("1.2.3", "fixture.tar.gz", &bytes, "tar-gz");
        mutate(&mut tool);
        let lock = json!({"schemaVersion": 0, "tools": {"fixture": tool}});
        let lock = Lock::parse(&serde_json::to_vec(&lock).unwrap()).unwrap();
        assert!(matches!(
            ArtifactPlan::from_lock(&lock, PLATFORM),
            Err(Error::InvalidLock(id)) if id == "fixture"
        ));
    }

    // Windows promotion is unqualified.
    let mut windows_lock = entry("1.2.3", "fixture.exe", SCRIPT.as_bytes(), "binary");
    let artifacts = windows_lock["installation"]["artifacts"]
        .as_object_mut()
        .unwrap();
    let artifact = artifacts.remove("linux-x64-gnu").unwrap();
    artifacts.insert("windows-x64".into(), artifact);
    let lock = json!({"schemaVersion": 0, "tools": {"fixture": windows_lock}});
    let lock = Lock::parse(&serde_json::to_vec(&lock).unwrap()).unwrap();
    let plans = ArtifactPlan::from_lock(&lock, Platform::WindowsX64).unwrap();
    let repo = tempfile::tempdir().unwrap();
    let store = Store::open(repo.path()).unwrap();
    let transport = ArtifactTransport::loopback_http_for_tests("http://127.0.0.1:9").unwrap();
    let desired = [plans[0].inventory_tool().clone()];
    let prepared = plans[0]
        .prepare(&store, &desired, &transport, Preparation::IfNeeded)
        .await;
    assert!(matches!(prepared, Err(Error::UnsupportedTarget)));
}
