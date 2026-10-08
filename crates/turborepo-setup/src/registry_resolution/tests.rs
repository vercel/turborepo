#![allow(clippy::unwrap_used)]
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    sync::{Arc, Mutex},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256, Sha512};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

use super::*;
use crate::{lock, registry_metadata::MAX_METADATA_BYTES};

struct Registry {
    transport: RegistryTransport,
    requests: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Registry {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn serve(metadata: Vec<u8>, bytes: Vec<u8>, artifact_length: Option<usize>) -> Registry {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let transport = RegistryTransport::loopback_http_for_tests(&format!(
        "http://{}",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let log = requests.clone();
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
            let artifact = path.ends_with(".tgz");
            let body = if artifact { &bytes } else { &metadata };
            let length = if artifact {
                artifact_length.unwrap_or(body.len())
            } else {
                body.len()
            };
            let header =
                format!("HTTP/1.1 200 OK\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n");
            if socket.write_all(header.as_bytes()).await.is_ok() {
                let _ = socket.write_all(body).await;
            }
        }
    });
    Registry {
        transport,
        requests,
        task,
    }
}
fn package(name: &str) -> Value {
    let bins = if name == "npm" {
        json!({"npm":"bin/npm-cli.js","npx":"bin/npx-cli.js"})
    } else {
        json!({"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"})
    };
    json!({"name":name,"version":"10.0.0","bin":bins,"scripts":{"install":"exit 99"}})
}
fn archive(package: &Value, paths: &[String]) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for (path, data) in std::iter::once(("package.json".to_owned(), package.to_string())).chain(
        paths
            .iter()
            .cloned()
            .map(|path| (path, "throw new Error('must not execute');".into())),
    ) {
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
fn paths(package: &Value) -> Vec<String> {
    package["bin"]
        .as_object()
        .unwrap()
        .values()
        .map(|v| v.as_str().unwrap().into())
        .collect()
}
fn metadata(name: &str, bytes: &[u8]) -> Value {
    json!({"name":name,"version":"10.0.0","dist":{
        "tarball":format!("https://registry.npmjs.org/{name}/-/{name}-10.0.0.tgz"),
        "integrity":format!("sha512-{}", STANDARD.encode(Sha512::digest(bytes))),
        "shasum":"not a fallback","sha256":"00".repeat(32)}})
}
async fn fixture(name: &str, document: Value, bytes: Vec<u8>) -> Registry {
    assert_eq!(name, document["name"].as_str().unwrap());
    serve(serde_json::to_vec(&document).unwrap(), bytes, None).await
}

#[tokio::test]
async fn exact_npm_pnpm_portable_identity_authored_pins_and_no_root_mutation() {
    let repo = tempfile::tempdir().unwrap();
    fs::write(
        repo.path().join("package.json"),
        "unchanged native declaration",
    )
    .unwrap();
    fs::write(repo.path().join("turbo.lock"), "unchanged committed lock").unwrap();
    for (manager, name) in [(Manager::Npm, "npm"), (Manager::Pnpm, "pnpm")] {
        let mut pkg = package(name);
        pkg["scripts"]["install"] =
            json!(format!("touch {}/should-never-run", repo.path().display()));
        let bytes = archive(&pkg, &paths(&pkg));
        let registry = fixture(name, metadata(name, &bytes), bytes.clone()).await;
        for algorithm in ["sha256", "sha512"] {
            let digest = if algorithm == "sha256" {
                format!("{:x}", Sha256::digest(&bytes))
            } else {
                format!("{:x}", Sha512::digest(&bytes))
            };
            let pin = CorepackIntegrity {
                algorithm,
                digest: digest.to_uppercase(),
            };
            let selected = registry
                .transport
                .resolve_exact(manager, "10.0.0", Some(&pin))
                .await
                .unwrap();
            assert_eq!(selected.manager(), manager);
            assert_eq!(selected.version().to_string(), "10.0.0");
            assert_eq!(
                selected.artifact().sha256,
                format!("{:x}", Sha256::digest(&bytes))
            );
            assert_eq!(
                selected.integrity().digest,
                format!("{:x}", Sha512::digest(&bytes))
            );
            assert_eq!(selected.integrity().algorithm, "sha512");
            selected.verify_authored(Some(&pin)).unwrap();
            let artifact = selected.into_artifact();
            assert_eq!(
                artifact.url,
                metadata(name, &bytes)["dist"]["tarball"].as_str().unwrap()
            );
            assert_eq!(artifact.format, lock::Format::TarGz);
            assert_eq!(artifact.root_prefix.as_deref(), Some("package"));
            assert!(artifact.destination.is_none());
            assert_eq!(
                artifact.executables,
                pkg["bin"]
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap().into()))
                    .collect()
            );
            lock::Lock::new(lock::Document {
                schema_version: lock::SCHEMA_VERSION,
                tools: BTreeMap::from([(
                    name.into(),
                    lock::Tool {
                        adapter: name.into(),
                        version: "10.0.0".into(),
                        options: BTreeMap::new(),
                        declarations: vec![lock::Declaration {
                            file: "package.json".into(),
                            field: Some("/packageManager".into()),
                            request: Some(format!("{name}@10.0.0")),
                        }],
                        installation: lock::Installation::Managed {
                            artifacts: BTreeMap::from([(
                                lock::Platform::Any,
                                BTreeMap::from([("package".into(), artifact)]),
                            )]),
                        },
                    },
                )]),
            })
            .unwrap();
        }
        assert_eq!(
            *registry.requests.lock().unwrap(),
            vec![
                format!("/{name}/10.0.0"),
                format!("/{name}/-/{name}-10.0.0.tgz"),
                format!("/{name}/10.0.0"),
                format!("/{name}/-/{name}-10.0.0.tgz")
            ]
        );
    }
    assert_eq!(
        fs::read_to_string(repo.path().join("package.json")).unwrap(),
        "unchanged native declaration"
    );
    assert_eq!(
        fs::read_to_string(repo.path().join("turbo.lock")).unwrap(),
        "unchanged committed lock"
    );
    assert_eq!(fs::read_dir(repo.path()).unwrap().count(), 2);
}

#[tokio::test]
async fn metadata_identity_sri_and_canonical_provenance_fail_before_download() {
    let pkg = package("pnpm");
    let bytes = archive(&pkg, &paths(&pkg));
    let good = metadata("pnpm", &bytes);
    for (field, value) in [
        ("name", json!("npm")),
        ("version", json!("10.0.1")),
        ("dist/tarball", json!("http://127.0.0.1/pnpm.tgz")),
        (
            "dist/tarball",
            json!("https://registry.npmjs.org:443/pnpm/-/pnpm-10.0.0.tgz"),
        ),
        ("dist/integrity", json!("sha256-not-sha512")),
        ("dist/integrity", Value::Null),
    ] {
        let mut document = good.clone();
        *document.pointer_mut(&format!("/{field}")).unwrap() = value;
        let registry = serve(serde_json::to_vec(&document).unwrap(), bytes.clone(), None).await;
        assert!(matches!(
            registry
                .transport
                .resolve_exact(Manager::Pnpm, "10.0.0", None)
                .await,
            Err(Error::Metadata(_))
        ));
        assert_eq!(registry.requests.lock().unwrap().len(), 1);
    }
    let registry = serve(br#"{"name":"pnpm","name":"pnpm"}"#.to_vec(), bytes, None).await;
    assert!(matches!(
        registry
            .transport
            .resolve_exact(Manager::Pnpm, "10.0.0", None)
            .await,
        Err(Error::Metadata(_))
    ));
}

#[tokio::test]
async fn mandatory_real_bytes_and_authored_mismatch_including_late_native_alternatives() {
    let pkg = package("pnpm");
    let bytes = archive(&pkg, &paths(&pkg));
    let wrong_metadata = metadata("pnpm", b"different bytes");
    let registry = fixture("pnpm", wrong_metadata, bytes.clone()).await;
    let sha256 = CorepackIntegrity {
        algorithm: "sha256",
        digest: format!("{:x}", Sha256::digest(&bytes)),
    };
    assert!(matches!(
        registry
            .transport
            .resolve_exact(Manager::Pnpm, "10.0.0", Some(&sha256))
            .await,
        Err(Error::Download(
            turborepo_download::Error::Sha512DigestMismatch
        ))
    ));
    let registry = fixture("pnpm", metadata("pnpm", &bytes), bytes).await;
    let selected = registry
        .transport
        .resolve_exact(Manager::Pnpm, "10.0.0", None)
        .await
        .unwrap();
    let declaration = crate::package_manager::discover_package_manager(
        &json!({"packageManager":"pnpm@10.0.0","devEngines":{"packageManager":[
            {"name":"pnpm","version":"9.0.0+sha1.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            {"name":"pnpm","version":format!("10.0.0+sha512.{}",selected.integrity().digest)}
        ]}}),
    )
    .unwrap()
    .unwrap();
    let late = declaration
        .locked_integrity(selected.version(), &selected.artifact().sha256)
        .unwrap();
    selected.verify_authored(late.as_ref()).unwrap();
    for (algorithm, length) in [("sha256", 64), ("sha512", 128), ("sha1", 40)] {
        let pin = CorepackIntegrity {
            algorithm,
            digest: "0".repeat(length),
        };
        assert!(selected.verify_authored(Some(&pin)).is_err());
        assert!(
            registry
                .transport
                .resolve_exact(Manager::Pnpm, "10.0.0", Some(&pin))
                .await
                .is_err()
        );
    }
    for digest in ["".into(), "g".repeat(128)] {
        assert!(
            selected
                .verify_authored(Some(&CorepackIntegrity {
                    algorithm: "sha512",
                    digest
                }))
                .is_err()
        );
    }
}

#[tokio::test]
async fn package_identity_complete_bins_and_required_regular_files() {
    for (manager, name) in [(Manager::Npm, "npm"), (Manager::Pnpm, "pnpm")] {
        let good = package(name);
        let files = paths(&good);
        for (field, value) in [
            ("name", json!("other")),
            ("version", json!("10.0.1")),
            ("bin", json!({})),
            ("bin", json!({name:"../escape"})),
        ] {
            let mut pkg = good.clone();
            pkg[field] = value;
            let bytes = archive(&pkg, &files);
            let registry = fixture(name, metadata(name, &bytes), bytes).await;
            assert!(matches!(
                registry
                    .transport
                    .resolve_exact(manager, "10.0.0", None)
                    .await,
                Err(Error::InvalidPackage)
            ));
        }
        let mut extra = good.clone();
        extra["bin"]["unexpected"] = json!("bin/extra.js");
        for bytes in [archive(&good, &files[..1]), archive(&extra, &files)] {
            let registry = fixture(name, metadata(name, &bytes), bytes).await;
            assert!(
                registry
                    .transport
                    .resolve_exact(manager, "10.0.0", None)
                    .await
                    .is_err()
            );
        }
    }
}

#[tokio::test]
async fn bounded_inputs_download_package_and_archive_with_no_request_escape() {
    let pkg = package("pnpm");
    let bytes = archive(&pkg, &paths(&pkg));
    let registry = fixture("pnpm", metadata("pnpm", &bytes), bytes.clone()).await;
    for version in [
        "latest".into(),
        "10.x".into(),
        "v10.0.0".into(),
        "10.0.0/../secret".into(),
        format!("10.0.0+{}", "a".repeat(128)),
    ] {
        assert!(
            registry
                .transport
                .resolve_exact(Manager::Pnpm, &version, None)
                .await
                .is_err()
        );
    }
    assert!(
        registry
            .transport
            .resolve_exact(Manager::Yarn, "10.0.0", None)
            .await
            .is_err()
    );
    assert!(registry.requests.lock().unwrap().is_empty());
    let registry = serve(vec![b' '; MAX_METADATA_BYTES + 1], bytes.clone(), None).await;
    assert!(matches!(
        registry
            .transport
            .resolve_exact(Manager::Pnpm, "10.0.0", None)
            .await,
        Err(Error::Download(turborepo_download::Error::TooLarge))
    ));
    let registry = serve(
        serde_json::to_vec(&metadata("pnpm", &bytes)).unwrap(),
        Vec::new(),
        Some(64 * 1024 * 1024 + 1),
    )
    .await;
    assert!(matches!(
        registry
            .transport
            .resolve_exact(Manager::Pnpm, "10.0.0", None)
            .await,
        Err(Error::Download(turborepo_download::Error::TooLarge))
    ));
    let mut big = pkg.clone();
    big["padding"] = json!("a".repeat(MAX_METADATA_BYTES));
    let bytes = archive(&big, &paths(&big));
    let registry = fixture("pnpm", metadata("pnpm", &bytes), bytes).await;
    assert!(matches!(
        registry
            .transport
            .resolve_exact(Manager::Pnpm, "10.0.0", None)
            .await,
        Err(Error::InvalidPackage)
    ));
    let mut deep = paths(&pkg);
    deep.push(vec!["d"; 65].join("/"));
    let bytes = archive(&pkg, &deep);
    let registry = fixture("pnpm", metadata("pnpm", &bytes), bytes).await;
    assert!(matches!(
        registry
            .transport
            .resolve_exact(Manager::Pnpm, "10.0.0", None)
            .await,
        Err(Error::Archive(turborepo_archive::Error::LimitExceeded))
    ));
}

#[test]
fn loopback_seam_is_literal_only_and_existing_aliases_are_the_same_type() {
    for origin in [
        "http://localhost:1234",
        "http://example.test",
        "http://192.0.2.1",
        "http://127.0.0.1:1234/path",
        "http://user:secret@127.0.0.1:1234",
    ] {
        assert!(RegistryTransport::loopback_http_for_tests(origin).is_err());
    }
    let pnpm: crate::pnpm_provision::PnpmTransport =
        RegistryTransport::loopback_http_for_tests("http://127.0.0.1:1234").unwrap();
    let _: crate::npm_provision::NpmTransport = pnpm;
    assert!(RegistryTransport::loopback_http_for_tests("http://[::1]:1234").is_ok());
}
