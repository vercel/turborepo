use serde_json::json;

use super::*;
use crate::NodeRequirements;

fn entry(version: &str) -> Value {
    json!({"version":version,"files":["linux-x64","linux-arm64","osx-x64-tar",
        "osx-arm64-tar","win-x64-zip","win-arm64-zip"],"npm":"11.0.0","lts":"Krypton",
        "date":"2026-10-01","security":false,"v8":"uninterpreted upstream field"})
}
fn index(entries: Vec<Value>) -> Result<ReleaseIndex, Error> {
    ReleaseIndex::parse(&serde_json::to_vec(&entries).unwrap())
}
fn checksums(name: &str) -> Checksums {
    Checksums::parse(format!("{}  {name}\n", "a".repeat(64)).as_bytes()).unwrap()
}
fn platform(os: OperatingSystem, arch: Architecture) -> Platform {
    Platform::new(os, arch)
}

#[test]
fn six_advertised_platforms_bind_exact_official_names_and_digests() {
    let index = index(vec![entry("v24.0.0")]).unwrap();
    let version = Version::new(24, 0, 0);
    for (os, vendor_os, extension) in [
        (OperatingSystem::Linux, "linux", "tar.gz"),
        (OperatingSystem::Macos, "darwin", "tar.gz"),
        (OperatingSystem::Windows, "win", "zip"),
    ] {
        for (arch, vendor_arch) in [(Architecture::X64, "x64"), (Architecture::Arm64, "arm64")] {
            let name = format!("node-v24.0.0-{vendor_os}-{vendor_arch}.{extension}");
            let metadata = index
                .artifact(&version, platform(os, arch), &checksums(&name))
                .unwrap()
                .unwrap();
            assert_eq!(metadata.artifact().filename(), name);
            assert_eq!(
                metadata.artifact().url(),
                format!("https://nodejs.org/dist/v24.0.0/{name}")
            );
            assert_eq!(metadata.sha256(), "a".repeat(64));
        }
    }
    assert_eq!(
        index.release(&version).unwrap().bundled_npm(),
        Some(&Version::new(11, 0, 0))
    );
    assert_eq!(INDEX_URL, "https://nodejs.org/dist/index.json");
}

#[test]
fn unavailable_and_inconsistent_targets_never_change_selected_version() {
    let target = platform(OperatingSystem::Linux, Architecture::Arm64);
    let mut older = entry("v22.0.0");
    older["files"] = json!(["linux-x64"]);
    let index = index(vec![entry("v24.0.0"), older]).unwrap();
    let wrong = checksums("node-v22.0.0-linux-x64.tar.gz");
    assert!(
        index
            .artifact(&Version::new(22, 0, 0), target, &wrong)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        index
            .artifact(&Version::new(24, 0, 0), target, &wrong)
            .unwrap_err(),
        Error::MissingChecksum
    );
    assert_eq!(
        index
            .artifact(&Version::new(23, 0, 0), target, &wrong)
            .unwrap_err(),
        Error::MissingRelease
    );
    assert_eq!(
        index
            .artifact(
                &Version::new(24, 0, 0),
                platform(OperatingSystem::Unknown, Architecture::X64),
                &wrong
            )
            .unwrap_err(),
        Error::UnsupportedTarget
    );
}

#[test]
fn resolver_candidates_preserve_normalized_lts_and_are_order_independent() {
    let index = index(vec![entry("v24.0.1"), entry("v22.0.0"), entry("v24.0.0")]).unwrap();
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join(".nvmrc"), "lts/krypton").unwrap();
    let requirements = NodeRequirements::read(root.path()).unwrap();
    assert_eq!(
        requirements.resolve(&index.releases()).unwrap().version,
        Version::new(24, 0, 1)
    );
    for npm in [json!(null), json!("")] {
        let mut value = entry("v24.0.0");
        value["npm"] = npm;
        assert!(
            super::tests::index(vec![value])
                .unwrap()
                .release(&Version::new(24, 0, 0))
                .unwrap()
                .bundled_npm()
                .is_none()
        );
    }
    let mut value = entry("v0.1.0");
    value.as_object_mut().unwrap().remove("npm");
    value["lts"] = json!(false);
    value["files"] = json!([]);
    assert!(super::tests::index(vec![value]).is_ok());
}

#[test]
fn index_shapes_exact_versions_and_all_duplicate_entries_are_strict() {
    for field in ["version", "files", "lts", "npm"] {
        let mut value = entry("v24.0.0");
        value[field] = json!(true);
        assert!(index(vec![value]).is_err());
    }
    for version in [
        "24.0.0",
        "v24",
        "v24.x",
        "v^24.0.0",
        "v24.0.0 ",
        "v24.0.0/escape",
    ] {
        assert!(index(vec![entry(version)]).is_err());
    }
    for npm in ["11", "v11.0.0", "11.x", "^11.0.0"] {
        let mut value = entry("v24.0.0");
        value["npm"] = json!(npm);
        assert!(index(vec![value]).is_err());
    }
    assert_eq!(
        index(vec![entry("v24.0.0"), entry("v24.0.0")]).unwrap_err(),
        Error::Index
    );
    let mut value = entry("v24.0.0");
    value["files"] = json!(["linux-x64", "linux-x64"]);
    assert!(index(vec![value]).is_err());
    for file in ["", "../escape", "linux-x64\n", "x".repeat(65).as_str()] {
        let mut value = entry("v24.0.0");
        value["files"] = json!([file]);
        assert!(index(vec![value]).is_err());
    }
    for input in [
        "{}",
        "[null]",
        "[]",
        "[{\"version\":\"v24.0.0\",\"version\":\"v22.0.0\"}]",
        "[{\"version\":\"v24.0.0\",\"files\":[],\"unknown\":{\"secret\":1,\"secret\":2}}]",
    ] {
        assert!(ReleaseIndex::parse(input.as_bytes()).is_err());
    }
}

#[test]
fn checksum_format_all_paths_duplicates_and_secret_diagnostics_are_bounded() {
    let digest = "A".repeat(64);
    let valid =
        format!("{digest}  node-v24.0.0-linux-x64.tar.gz\r\n{digest} *win-x64/node.exe\r\n");
    let sums = Checksums::parse(valid.as_bytes()).unwrap();
    assert_eq!(sums.0["win-x64/node.exe"], digest.to_ascii_lowercase());
    for invalid in [
        "",
        " ",
        "1234  filename",
        "g".repeat(64).as_str(),
        "secret-token",
    ] {
        let error = Checksums::parse(invalid.as_bytes()).unwrap_err();
        assert!(!format!("{error:?} {error}").contains("secret-token"));
    }
    for path in [
        "/absolute",
        "a/../b",
        "a/./b",
        "a//b",
        "a\\b",
        "a:stream",
        "a?token=secret",
        "a/b/c/d/e",
        "x".repeat(256).as_str(),
    ] {
        assert!(Checksums::parse(format!("{digest}  {path}\n").as_bytes()).is_err());
    }
    for second in [
        "node-v24.0.0-linux-x64.tar.gz",
        "*node-v24.0.0-linux-x64.tar.gz",
    ] {
        assert_eq!(
            Checksums::parse(
                format!("{digest}  node-v24.0.0-linux-x64.tar.gz\n{digest}  {second}\n").as_bytes()
            )
            .unwrap_err(),
            Error::Checksums
        );
    }
    assert!(Checksums::parse(format!("{digest}  file extra\n").as_bytes()).is_err());
    assert!(Checksums::parse(&[0xff]).is_err());
}

#[test]
fn byte_entry_and_recursion_boundaries_are_inclusive() {
    let base = serde_json::to_vec(&vec![entry("v24.0.0")]).unwrap();
    let mut padded = base.clone();
    padded.resize(MAX_INDEX_BYTES, b' ');
    assert!(ReleaseIndex::parse(&padded).is_ok());
    padded.push(b' ');
    assert_eq!(ReleaseIndex::parse(&padded).unwrap_err(), Error::Limit);
    let line = format!("{}  file\n", "a".repeat(64));
    let mut padded = line.into_bytes();
    padded.resize(MAX_CHECKSUM_BYTES, b'\n');
    assert!(Checksums::parse(&padded).is_ok());
    padded.push(b'\n');
    assert_eq!(Checksums::parse(&padded).unwrap_err(), Error::Limit);
    let mut lines = String::new();
    for i in 0..MAX_CHECKSUM_ENTRIES {
        lines.push_str(&format!("{}  file-{i}\n", "a".repeat(64)));
    }
    assert!(Checksums::parse(lines.as_bytes()).is_ok());
    lines.push_str(&format!("{}  extra\n", "a".repeat(64)));
    assert_eq!(
        Checksums::parse(lines.as_bytes()).unwrap_err(),
        Error::Limit
    );
    let entries: Vec<_> = (0..MAX_RELEASES)
        .map(|i| json!({"version":format!("v1.{i}.0"),"files":[]}))
        .collect();
    assert!(index(entries.clone()).is_ok());
    let mut entries = entries;
    entries.push(entry("v2.0.0"));
    assert_eq!(index(entries).unwrap_err(), Error::Limit);
    let recursive = format!("{}0{}", "[".repeat(150), "]".repeat(150));
    assert!(
        serde_json::from_str::<UniqueJson>(&recursive)
            .err()
            .unwrap()
            .to_string()
            .contains("recursion limit exceeded")
    );
    assert!(
        serde_json::from_str::<UniqueJson>(&format!("{}0{}", "[".repeat(10), "]".repeat(10)))
            .is_ok()
    );
}
