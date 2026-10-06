use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
#[cfg(unix)]
use std::{collections::BTreeMap, io::Write};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
#[cfg(unix)]
use turborepo_tool_install::Outcome;

use super::*;

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
async fn serve(body: Vec<u8>) -> Upstream {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let count = requests.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 8192];
            let n = socket.read(&mut request).await.unwrap();
            assert!(
                std::str::from_utf8(&request[..n])
                    .unwrap()
                    .starts_with("GET /dist/v24.0.0/node-v24.0.0-")
            );
            count.fetch_add(1, Ordering::SeqCst);
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
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

fn fixture(platform: Platform, bytes: &[u8]) -> Value {
    let spelling = serde_json::to_value(platform)
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned();
    let windows = matches!(platform, Platform::WindowsX64 | Platform::WindowsArm64);
    let (os, arch) = match platform {
        Platform::MacosX64 => ("darwin", "x64"),
        Platform::MacosArm64 => ("darwin", "arm64"),
        Platform::LinuxX64Gnu => ("linux", "x64"),
        Platform::LinuxArm64Gnu => ("linux", "arm64"),
        Platform::WindowsX64 => ("win", "x64"),
        Platform::WindowsArm64 => ("win", "arm64"),
        _ => unreachable!(),
    };
    let root = format!("node-v24.0.0-{os}-{arch}");
    json!({"schemaVersion":0,"tools":{"node":{
    "adapter":"node","version":"24.0.0","declarations":[{"file":".nvmrc","request":"24.0.0"}],
    "installation":{"kind":"managed","artifacts":{spelling:{"distribution":{
        "url":format!("https://nodejs.org/dist/v24.0.0/{root}.{}", if windows {"zip"} else {"tar.gz"}),
        "sha256":format!("{:x}", Sha256::digest(bytes)),
        "format":if windows {"zip"} else {"tar-gz"}, "rootPrefix":root,
        "executables":if windows {json!({"node":"node.exe","npm":"npm.cmd","npx":"npx.cmd"})}
            else {json!({"node":"bin/node","npm":"bin/npm","npx":"bin/npx"})}
    }}}}}}})
}
fn plan(value: &Value, platform: Platform) -> Result<NodePlan, Error> {
    let lock = Lock::parse(&serde_json::to_vec(value).unwrap()).unwrap();
    NodePlan::from_lock(&lock, platform)
}
#[cfg(unix)]
fn unix_archive(os: &str, arch: &str, bad_link: bool, missing: bool) -> Vec<u8> {
    let root = format!("node-v24.0.0-{os}-{arch}");
    let mut archive = tar::Builder::new(Vec::new());
    let cli = "#!/usr/bin/env node\n";
    for (path, contents) in [
        (
            "bin/node",
            "#!/bin/sh\ncase \"$1\" in --version) echo v24.0.0;; *npm|*npm-cli.js) echo \
             bundled-npm;; *npx|*npx-cli.js) echo bundled-npx;; *) exit 7;; esac\n",
        ),
        ("lib/node_modules/npm/bin/npm-cli.js", cli),
        ("lib/node_modules/npm/bin/npx-cli.js", cli),
        ("lib/node_modules/npm/package.json", "{\"name\":\"npm\"}"),
        ("share/man/man1/node.1", "fixture resource"),
    ] {
        if missing && path == "lib/node_modules/npm/bin/npx-cli.js" {
            continue;
        }
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        archive
            .append_data(&mut header, format!("{root}/{path}"), contents.as_bytes())
            .unwrap();
    }
    for (name, target) in [("npm", "npm-cli.js"), ("npx", "npx-cli.js")] {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header
            .set_link_name(if bad_link {
                "../../outside".into()
            } else {
                format!("../lib/node_modules/npm/bin/{target}")
            })
            .unwrap();
        header.set_cksum();
        archive
            .append_data(&mut header, format!("{root}/bin/{name}"), &[][..])
            .unwrap();
    }
    let tar = archive.into_inner().unwrap();
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&tar).unwrap();
    gzip.finish().unwrap()
}

// Stored ZIP32 fixture, including CRCs and both metadata copies; no ZIP tool.
fn zip_archive(arch: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut central = Vec::new();
    let files = [
        "node.exe",
        "npm.cmd",
        "npx.cmd",
        "node_modules/npm/bin/npm-cli.js",
        "node_modules/npm/bin/npx-cli.js",
        "node_modules/npm/package.json",
    ];
    for path in files {
        let name = format!("node-v24.0.0-win-{arch}/{path}");
        let data = b"fixture";
        let mut crc = !0u32;
        for byte in data {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(crc & 1));
            }
        }
        let crc = !crc;
        let offset = bytes.len() as u32;
        bytes.extend_from_slice(b"PK\x03\x04");
        for n in [20u16, 0, 0, 0, 0] {
            bytes.extend(n.to_le_bytes());
        }
        for n in [crc, data.len() as u32, data.len() as u32] {
            bytes.extend(n.to_le_bytes());
        }
        bytes.extend((name.len() as u16).to_le_bytes());
        bytes.extend(0u16.to_le_bytes());
        bytes.extend(name.as_bytes());
        bytes.extend(data);
        central.extend_from_slice(b"PK\x01\x02");
        for n in [20u16, 20, 0, 0, 0, 0] {
            central.extend(n.to_le_bytes());
        }
        for n in [crc, data.len() as u32, data.len() as u32] {
            central.extend(n.to_le_bytes());
        }
        for n in [name.len() as u16, 0, 0, 0, 0] {
            central.extend(n.to_le_bytes());
        }
        central.extend(0u32.to_le_bytes());
        central.extend(offset.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let start = bytes.len() as u32;
    let size = central.len() as u32;
    bytes.extend(central);
    bytes.extend_from_slice(b"PK\x05\x06");
    for n in [0u16, 0, files.len() as u16, files.len() as u16] {
        bytes.extend(n.to_le_bytes());
    }
    bytes.extend(size.to_le_bytes());
    bytes.extend(start.to_le_bytes());
    bytes.extend(0u16.to_le_bytes());
    bytes
}

#[cfg(unix)]
async fn install(
    store: &mut Store,
    plan: &NodePlan,
    transport: &NodeTransport,
) -> Result<Outcome, Error> {
    let desired = [plan.inventory_tool().clone()];
    let prepared = plan.prepare_if_needed(store, &desired, transport).await?;
    Ok(store.reconcile(&desired, |tool, path| {
        prepared
            .as_ref()
            .expect("staging cannot occur for a healthy no-op")
            .stage(tool, path)
    })?)
}

#[cfg(unix)]
#[tokio::test]
async fn locked_unix_matrix_full_resources_shims_repair_and_network_free_repeat() {
    for (platform, os, arch) in [
        (Platform::MacosX64, "darwin", "x64"),
        (Platform::MacosArm64, "darwin", "arm64"),
        (Platform::LinuxX64Gnu, "linux", "x64"),
        (Platform::LinuxArm64Gnu, "linux", "arm64"),
    ] {
        let bytes = unix_archive(os, arch, false, false);
        let upstream = serve(bytes.clone()).await;
        let transport = NodeTransport::loopback_http_for_tests(&upstream.origin).unwrap();
        let value = fixture(platform, &bytes);
        let before = serde_json::to_vec(&value).unwrap();
        let plan = plan(&value, platform).unwrap();
        let repo = tempfile::tempdir().unwrap();
        let mut store = Store::open(repo.path()).unwrap();
        assert_eq!(
            install(&mut store, &plan, &transport).await.unwrap(),
            Outcome::Replaced
        );
        let current = store.current().unwrap().unwrap();
        assert_eq!(current.tools, vec![plan.inventory_tool().clone()]);
        for (name, expected) in [
            ("node", "v24.0.0"),
            ("npm", "bundled-npm"),
            ("npx", "bundled-npx"),
        ] {
            let output = std::process::Command::new(current.bin.join(name))
                .arg("--version")
                .env_clear()
                .env("PATH", &current.bin)
                .output()
                .unwrap();
            assert!(output.status.success());
            assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), expected);
        }
        let root = current.bin.parent().unwrap().join("tools/node");
        assert_eq!(
            fs::read_to_string(root.join("share/man/man1/node.1")).unwrap(),
            "fixture resource"
        );
        assert!(root.join("lib/node_modules/npm/package.json").is_file());
        let manifest = repo.path().join(".turbo/tools/manifest.json");
        let old = fs::read(&manifest).unwrap();
        assert_eq!(
            install(&mut store, &plan, &transport).await.unwrap(),
            Outcome::Unchanged
        );
        assert_eq!(fs::read(&manifest).unwrap(), old);
        assert_eq!(upstream.requests.load(Ordering::SeqCst), 1);
        fs::write(root.join("bin/node"), "damaged").unwrap();
        assert!(!store.is_current(&[plan.inventory_tool().clone()]).unwrap());
        assert_eq!(
            install(&mut store, &plan, &transport).await.unwrap(),
            Outcome::Replaced
        );
        assert!(store.current().unwrap().is_some());
        assert_eq!(upstream.requests.load(Ordering::SeqCst), 2);
        assert_eq!(serde_json::to_vec(&value).unwrap(), before);
    }
}

#[cfg(unix)]
#[tokio::test]
async fn checksum_and_extraction_failures_never_publish_or_damage_prior_install() {
    let platform = Platform::MacosArm64;
    let good = unix_archive("darwin", "arm64", false, false);
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let good_plan = plan(&fixture(platform, &good), platform).unwrap();
    let upstream = serve(good.clone()).await;
    let transport = NodeTransport::loopback_http_for_tests(&upstream.origin).unwrap();
    install(&mut store, &good_plan, &transport).await.unwrap();
    let manifest = repo.path().join(".turbo/tools/manifest.json");
    let old = fs::read(&manifest).unwrap();
    for bytes in [
        b"corrupt bytes".to_vec(),
        unix_archive("darwin", "arm64", true, false),
        unix_archive("darwin", "arm64", false, true),
    ] {
        let upstream = serve(bytes.clone()).await;
        let transport = NodeTransport::loopback_http_for_tests(&upstream.origin).unwrap();
        let mut value = fixture(platform, &bytes);
        if bytes == b"corrupt bytes" {
            value["tools"]["node"]["installation"]["artifacts"]["macos-arm64"]["distribution"]
                ["sha256"] = json!("a".repeat(64));
        }
        let broken = plan(&value, platform).unwrap();
        let error = install(&mut store, &broken, &transport).await.unwrap_err();
        assert!(matches!(
            error,
            Error::Download(turborepo_download::Error::DigestMismatch) | Error::Archive(_)
        ));
        assert_eq!(fs::read(&manifest).unwrap(), old);
        assert!(
            store
                .is_current(&[good_plan.inventory_tool().clone()])
                .unwrap()
        );
        let empty = tempfile::tempdir().unwrap();
        let mut empty_store = Store::open(empty.path()).unwrap();
        assert!(
            install(&mut empty_store, &broken, &transport)
                .await
                .is_err()
        );
        assert!(!empty.path().join(".turbo/tools/manifest.json").exists());
        assert!(
            fs::read_dir(empty.path().join(".turbo/tools"))
                .unwrap()
                .all(|entry| !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("generation-"))
        );
    }
}

#[tokio::test]
async fn zip_matrix_prepares_full_windows_tree_without_claiming_promotion() {
    for (platform, arch) in [
        (Platform::WindowsX64, "x64"),
        (Platform::WindowsArm64, "arm64"),
    ] {
        let bytes = zip_archive(arch);
        let upstream = serve(bytes.clone()).await;
        let transport = NodeTransport::loopback_http_for_tests(&upstream.origin).unwrap();
        let plan = plan(&fixture(platform, &bytes), platform).unwrap();
        let prepared = plan.download(&transport).await.unwrap();
        let destination = tempfile::tempdir().unwrap();
        assert!(matches!(
            prepared.stage(plan.inventory_tool(), destination.path()),
            Err(turborepo_tool_install::Error::UnsupportedPlatform)
        ));
        assert!(
            prepared
                .tree
                .root_path()
                .join("node_modules/npm/package.json")
                .is_file()
        );
        #[cfg(unix)]
        {
            let repo = tempfile::tempdir().unwrap();
            let store = Store::open(repo.path()).unwrap();
            assert!(matches!(
                plan.prepare_if_needed(&store, &[plan.inventory_tool().clone()], &transport)
                    .await,
                Err(Error::UnsupportedTarget)
            ));
            assert_eq!(upstream.requests.load(Ordering::SeqCst), 1);
        }
    }
}

#[test]
fn adapter_rejects_nonofficial_metadata_and_unavailable_targets() {
    let platform = Platform::MacosArm64;
    let value = fixture(platform, &[]);
    for (key, bad) in [
        ("url", json!("https://evil.test/node.tar.gz")),
        ("rootPrefix", json!("node-v23.0.0-darwin-arm64")),
        ("format", json!("zip")),
        ("destination", json!("other")),
        (
            "executables",
            json!({"node":"bin/node","corepack":"bin/corepack"}),
        ),
        ("executables", json!({"npm":"bin/npm"})),
        ("executables", json!({"node":"bin/npm"})),
    ] {
        let mut invalid = value.clone();
        invalid["tools"]["node"]["installation"]["artifacts"]["macos-arm64"]["distribution"][key] =
            bad;
        assert!(matches!(plan(&invalid, platform), Err(Error::InvalidLock)));
    }
    assert!(matches!(
        plan(&value, Platform::MacosX64),
        Err(Error::MissingTarget)
    ));
    assert!(matches!(
        plan(&value, Platform::LinuxX64Musl),
        Err(Error::UnsupportedTarget)
    ));
    assert!(matches!(
        plan(&value, Platform::Any),
        Err(Error::UnsupportedTarget)
    ));
    for origin in [
        "http://localhost:8080",
        "http://192.0.2.1",
        "https://nodejs.org",
        "http://127.0.0.1/a",
    ] {
        assert!(NodeTransport::loopback_http_for_tests(origin).is_err());
    }
    let mut range = value.clone();
    range["tools"]["node"]["version"] = json!("24.x");
    assert!(Lock::parse(&serde_json::to_vec(&range).unwrap()).is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn selected_node_only_mapping_composes_with_separate_npm_inventory() {
    let platform = Platform::MacosArm64;
    let bytes = unix_archive("darwin", "arm64", false, false);
    let mut value = fixture(platform, &bytes);
    value["tools"]["node"]["installation"]["artifacts"]["macos-arm64"]["distribution"]
        ["executables"] = json!({"node":"bin/node"});
    value["tools"]["npm"] = json!({"adapter":"npm","version":"11.0.0","declarations":[{"file":"package.json","field":"packageManager","request":"npm@11.0.0"}],"installation":{"kind":"verify-system","executables":["npm","npx"]}});
    let plan = plan(&value, platform).unwrap();
    assert_eq!(plan.inventory_tool().executables.len(), 1);
    let npm = Tool {
        id: "npm".into(),
        version: "11.0.0".into(),
        platform: "macos-arm64".into(),
        artifact_sha256: "a".repeat(64),
        executables: BTreeMap::from([
            ("npm".into(), "bin/npm".into()),
            ("npx".into(), "bin/npx".into()),
        ]),
    };
    let desired = [npm.clone(), plan.inventory_tool().clone()];
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let upstream = serve(bytes).await;
    let transport = NodeTransport::loopback_http_for_tests(&upstream.origin).unwrap();
    assert!(matches!(
        plan.prepare_if_needed(&store, &[npm], &transport).await,
        Err(Error::InventoryMismatch)
    ));
    assert_eq!(upstream.requests.load(Ordering::SeqCst), 0);
    let prepared = plan
        .prepare_if_needed(&store, &desired, &transport)
        .await
        .unwrap()
        .unwrap();
    store
        .reconcile(&desired, |tool, path| {
            if tool.id == "node" {
                prepared.stage(tool, path)
            } else {
                fs::create_dir(path.join("bin"))?;
                for name in ["npm", "npx"] {
                    let executable = path.join("bin").join(name);
                    fs::write(&executable, "#!/bin/sh\nexit 0\n")?;
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(executable, fs::Permissions::from_mode(0o755))?;
                }
                Ok(())
            }
        })
        .unwrap();
    assert!(store.is_current(&desired).unwrap());
    assert!(
        plan.prepare_if_needed(&store, &desired, &transport)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(store.current().unwrap().unwrap().tools.len(), 2);
    assert_eq!(upstream.requests.load(Ordering::SeqCst), 1);
}
