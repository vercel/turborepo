#![allow(clippy::unwrap_used)]

use serde_json::{Value, json};

use super::*;
use crate::{go_discovery::GO_WORK, source_policy::test_support::LoopbackServer};

const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn release(version: &str) -> Value {
    let raw = format!("go{version}");
    let mut files: Vec<_> = [
        ("darwin", "amd64", "tar.gz"),
        ("darwin", "arm64", "tar.gz"),
        ("linux", "amd64", "tar.gz"),
        ("linux", "arm64", "tar.gz"),
        ("windows", "amd64", "zip"),
        ("windows", "arm64", "zip"),
    ]
    .into_iter()
    .map(|(os, arch, extension)| {
        json!({"filename": format!("{raw}.{os}-{arch}.{extension}"), "os": os, "arch": arch,
            "version": raw, "sha256": SHA, "size": 1, "kind": "archive"})
    })
    .collect();
    // Unrelated files (sources, installers, other targets) are never selected.
    files.push(
        json!({"filename": format!("{raw}.src.tar.gz"), "os": "", "arch": "",
        "version": raw, "sha256": "", "size": 1, "kind": "source"}),
    );
    json!({"version": raw, "stable": !version.contains("rc"), "files": files})
}
fn index(releases: &[Value]) -> Vec<u8> {
    serde_json::to_vec(releases).unwrap()
}
fn requirements(text: &str) -> GoRequirements {
    GoRequirements::read_with(|file, _| Ok((file == GO_WORK).then(|| text.as_bytes().to_vec())))
        .unwrap()
        .unwrap()
}
fn artifacts(tool: &Tool) -> &BTreeMap<Platform, BTreeMap<String, Artifact>> {
    let Installation::Managed { artifacts } = &tool.installation else {
        panic!("expected managed Go");
    };
    artifacts
}

#[test]
fn locks_one_release_with_official_per_platform_archives() {
    let historical = json!({"version": "go1", "stable": true, "files": []});
    let releases = [
        release("1.23.4"),
        release("1.22.12"),
        release("1.22.3"),
        historical,
    ];
    let tool = resolve(&requirements("go 1.22\n"), &index(&releases)).unwrap();
    assert_eq!(
        (tool.adapter.as_str(), tool.version.as_str()),
        ("go", "1.22.12")
    );
    assert_eq!(
        tool.declarations,
        [Declaration {
            file: "go.work".into(),
            field: Some("go".into()),
            request: Some("1.22".into()),
        }]
    );
    assert_eq!(
        artifacts(&tool).keys().copied().collect::<Vec<_>>(),
        PLATFORMS
    );
    assert_eq!(
        artifacts(&tool)[&Platform::LinuxArm64Gnu]["distribution"],
        Artifact {
            url: "https://dl.google.com/go/go1.22.12.linux-arm64.tar.gz".into(),
            sha256: SHA.into(),
            format: Format::TarGz,
            root_prefix: Some("go".into()),
            destination: None,
            executables: BTreeMap::from([
                ("go".into(), "bin/go".into()),
                ("gofmt".into(), "bin/gofmt".into()),
            ]),
        }
    );
    let windows = &artifacts(&tool)[&Platform::WindowsX64]["distribution"];
    assert_eq!(windows.format, Format::Zip);
    assert_eq!(windows.executables["gofmt"], "bin/gofmt.exe");
    let document = |tool: Tool| Document {
        schema_version: lock::SCHEMA_VERSION,
        tools: BTreeMap::from([("go".into(), tool)]),
    };
    let lock = Lock::new(document(tool.clone())).unwrap();
    assert_eq!(Lock::parse(&lock.canonical_bytes().unwrap()).unwrap(), lock);
    // The lock owns exact canonical Go identities, never language versions.
    for version in ["1.22", "go1.22.12", "1.22.012", "1.22.12-custom"] {
        let mut invalid = tool.clone();
        invalid.version = version.into();
        assert!(Lock::new(document(invalid)).is_err(), "{version}");
    }
}

#[test]
fn exact_toolchain_omits_unpublished_targets_and_requires_sha256() {
    let exact = requirements("go 1.22\ntoolchain go1.22.3\n");
    let mut partial = release("1.22.3");
    partial["files"]
        .as_array_mut()
        .unwrap()
        .retain(|file| file["os"] != "windows");
    let tool = resolve(&exact, &index(&[partial.clone(), release("1.23.4")])).unwrap();
    assert_eq!(tool.version, "1.22.3");
    assert!(!artifacts(&tool).contains_key(&Platform::WindowsX64));
    assert!(artifacts(&tool).contains_key(&Platform::MacosArm64));
    // Historical SHA-1 values are not official SHA-256 checksums.
    partial["files"][0]["sha256"] = json!("da39a3ee5e6b4b0d3255bfef95601890afd80709");
    assert!(matches!(
        resolve(&exact, &index(&[partial.clone()])),
        Err(Error::MissingChecksum(name)) if name == "go1.22.3.darwin-amd64.tar.gz"
    ));
    partial["files"] = json!([]);
    assert!(matches!(
        resolve(&exact, &index(&[partial])),
        Err(Error::NoArtifacts)
    ));
    assert!(matches!(
        resolve(&exact, &index(&[release("1.22.4")])),
        Err(Error::Discovery(GoDiscoveryError::NoMatchingRelease(_)))
    ));
}

#[test]
fn ambiguous_or_inconsistent_indexes_are_rejected() {
    let mut mislabeled = release("1.22.3");
    mislabeled["files"][2]["arch"] = json!("arm64");
    let mut repeated = release("1.22.3");
    let first = repeated["files"][0].clone();
    repeated["files"].as_array_mut().unwrap().push(first);
    for bytes in [
        index(&[release("1.22.3"), release("1.22.3")]),
        index(&[mislabeled]),
        index(&[repeated]),
        br#"[{"version":"go1.22.3","version":"go1.22.4","files":[]}]"#.to_vec(),
        br#"{"version":"go1.22.3","files":[]}"#.to_vec(),
        b"[]".to_vec(),
        vec![b' '; MAX_INDEX_BYTES + 1],
    ] {
        assert!(matches!(
            ReleaseIndex::parse(&bytes),
            Err(Error::Index | Error::Limit)
        ));
    }
}

#[tokio::test]
async fn transport_reads_only_the_bounded_official_index() {
    let routes = [(INDEX_PATH.to_owned(), index(&[release("1.22.3")]))];
    let server = LoopbackServer::new(routes, |_| {}).unwrap();
    let transport = GoTransport::loopback_http_for_tests(server.origin()).unwrap();
    let tool = transport.resolve(&requirements("go 1.22\n")).await.unwrap();
    assert_eq!(tool.version, "1.22.3");
    // Lock provenance stays official even through a fixture origin.
    assert!(artifacts(&tool).values().all(|parts| {
        parts["distribution"]
            .url
            .starts_with("https://dl.google.com/go/")
    }));
    assert_eq!(server.requests(), [INDEX_PATH]);
}
