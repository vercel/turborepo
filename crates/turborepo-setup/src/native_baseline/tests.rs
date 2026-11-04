#![allow(clippy::unwrap_used)]
use std::fs;

use super::*;
use crate::{
    lock::{Artifact, Document, Format, Platform},
    node_resolution::PLATFORMS as TARGETS,
};

fn root(manifest: Value) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("package.json"), manifest.to_string()).unwrap();
    fs::write(root.path().join(".nvmrc"), "24.x\n").unwrap();
    root
}
fn selection(snapshot: &Snapshot) -> Lock {
    let mut artifacts = serde_json::Map::new();
    for platform in TARGETS {
        let (target, spelling) = crate::node::lock_target(platform).unwrap();
        let official =
            crate::NodeArtifact::for_platform(&semver::Version::new(24, 1, 0), target).unwrap();
        let windows = matches!(platform, Platform::WindowsX64 | Platform::WindowsArm64);
        let extension = if windows { ".zip" } else { ".tar.gz" };
        artifacts.insert(
            spelling.into(),
            json!({"runtime":{
                "url":official.url(), "sha256":"a".repeat(64),
                "format":if windows {"zip"} else {"tar-gz"},
                "rootPrefix":official.filename().strip_suffix(extension).unwrap(),
                "executables":{"node":if windows {"node.exe"} else {"bin/node"}}
            }}),
        );
    }
    let mut wire = json!({"schemaVersion":0,"tools":{"node":{
        "adapter":"node","version":"24.1.0","declarations":snapshot.declarations()["node"],
        "installation":{"kind":"managed","artifacts":artifacts}
    }}});
    if let Some(sources) = snapshot.declarations().get("pnpm") {
        wire["tools"]["pnpm"] = json!({"adapter":"pnpm","version":"10.1.0","declarations":sources,
        "installation":{"kind":"managed","artifacts":{"any":{"package":{
            "url":"https://registry.npmjs.org/pnpm/-/pnpm-10.1.0.tgz","sha256":"b".repeat(64),
            "format":"tar-gz","rootPrefix":"package",
            "executables":{"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"}
        }}}}});
    }
    Lock::parse(&serde_json::to_vec(&wire).unwrap()).unwrap()
}
fn fixture(manifest: Value) -> (tempfile::TempDir, Snapshot, Lock) {
    let repo = root(manifest);
    let snapshot = Snapshot::capture(repo.path()).unwrap();
    let lock = selection(&snapshot);
    (repo, snapshot, lock)
}
type Artifacts = BTreeMap<Platform, BTreeMap<String, Artifact>>;
fn artifacts<'a>(wire: &'a mut Document, id: &str) -> &'a mut Artifacts {
    match &mut wire.tools.get_mut(id).unwrap().installation {
        Installation::Managed { artifacts } => artifacts,
        _ => panic!("managed fixture required"),
    }
}
/// A user record holding `lock` never decodes into a previous selection.
fn reject(lock: Lock) {
    assert!(matches!(
        NativeRecord::decode(&lock.canonical_bytes().unwrap()),
        Err(Error::Invalid)
    ));
}

#[test]
fn canonical_record_retains_typed_previous_provenance_not_a_fabricated_disk_lock() {
    let (repo, snapshot, lock) = fixture(
        json!({"packageManager":"pnpm@10.1.0", "secret":"never-record-me",
        "devEngines":{"runtime":[{"name":"node"},{"name":"node","version":"24.x","onFail":"error"}],
        "packageManager":[{"name":"pnpm","version":"10.1.0","onFail":"error"}]}}),
    );
    let native = NativeRecord::from_snapshot(&snapshot, &lock).unwrap();
    let bytes = native.bytes();
    let repeated = NativeRecord::from_snapshot(&snapshot, &lock).unwrap();
    assert_eq!(bytes, lock.canonical_bytes().unwrap());
    assert_eq!(bytes, repeated.bytes());
    let text = String::from_utf8_lossy(bytes);
    assert!(!text.contains("never-record-me"));
    assert!(!text.contains(repo.path().to_str().unwrap()));
    // The recorded selection is the previous provenance after declarations
    // change, never a disk lock.
    fs::write(repo.path().join(".nvmrc"), "22.x").unwrap();
    let changed = Snapshot::capture(repo.path()).unwrap();
    let prior = NativeRecord::decode(bytes).unwrap();
    assert_eq!(prior.selection(), &lock);
    let version = semver::Version::new(24, 1, 0);
    assert!(prior.node_requirements().matches_locked_version(&version));
    assert_eq!(prior.package_manager().unwrap().manager, Manager::Pnpm);
    assert!(changed.previous_lock().is_none());
    assert!(!repo.path().join("turbo.lock").exists());
    assert!(NativeRecord::from_snapshot(&changed, &lock).is_err());
}

#[test]
fn strict_decode_rejects_noncanonical_unknown_duplicate_schema_and_oversized_records() {
    let (_repo, snapshot, lock) = fixture(json!({"engines":{"node":"24.x"}}));
    let bytes = lock.canonical_bytes().unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    let mut unknown = serde_json::to_value(lock.document()).unwrap();
    unknown["tools"]["node"]["extra"] = json!(true);
    let schema = "\"schemaVersion\": 0";
    let cases = [
        text.replace(schema, "\"schemaVersion\": 1").into_bytes(),
        text.replace(schema, &format!("{schema},{schema}"))
            .into_bytes(),
        serde_json::to_vec(&unknown).unwrap(),
        serde_json::to_vec(lock.document()).unwrap(),
        bytes[..bytes.len() - 1].to_vec(),
        [b" ".as_slice(), &bytes].concat(),
        [bytes.as_slice(), b"{}"].concat(),
        vec![b' '; crate::lock::MAX_LOCK_BYTES + 1],
    ];
    for (index, invalid_bytes) in cases.into_iter().enumerate() {
        assert_ne!(
            invalid_bytes, bytes,
            "case {index} must alter canonical bytes"
        );
        assert!(
            NativeRecord::decode(&invalid_bytes).is_err(),
            "case {index} must be rejected"
        );
    }
    let mut reordered = lock.document().clone();
    let node = reordered.tools.get_mut("node").unwrap();
    node.declarations.reverse();
    let reordered = Lock::new(reordered).unwrap();
    let canonical = NativeRecord::from_snapshot(&snapshot, &reordered).unwrap();
    assert_eq!(canonical.bytes(), bytes);
}

#[test]
fn native_grammar_roundtrip_rejects_invented_and_malformed_sources() {
    let (_repo, _snapshot, lock) = fixture(json!({"packageManager":"pnpm@10.1.0"}));
    for (field, request) in [
        (Some("engines.rust"), Some("24.x")),
        (Some("devEngines.runtime[01].version"), Some("24.x")),
        (Some("devEngines.runtime[64].version"), Some("24.x")),
        (Some("devEngines.runtime.onFail"), Some("warn")),
        (Some("devEngines.runtime.version"), Some("banana")),
        (None, None),
    ] {
        let mut wire = lock.document().clone();
        wire.tools.get_mut("node").unwrap().declarations = vec![crate::lock::Declaration {
            file: "package.json".into(),
            field: field.map(str::to_owned),
            request: request.map(str::to_owned),
        }];
        reject(Lock::new(wire).unwrap());
    }
    for pointer in [
        "/secret",
        "/devEngines/packageManager/1/name",
        "/devEngines/packageManager/onFail",
    ] {
        let mut wire = lock.document().clone();
        wire.tools.get_mut("pnpm").unwrap().declarations[0].field = Some(pointer.into());
        reject(Lock::new(wire).unwrap());
    }
    for request in ["22.x", "v24.1.0", "lts/SECRET"] {
        let mut wire = lock.document().clone();
        wire.tools.get_mut("node").unwrap().declarations[0].request = Some(request.into());
        reject(Lock::new(wire).unwrap());
    }
}

#[test]
fn every_portable_variant_is_validated_not_just_the_selected_target() {
    let (_repo, _snapshot, lock) = fixture(json!({"packageManager":"pnpm@10.1.0"}));
    for id in ["node", "pnpm"] {
        for platform in TARGETS {
            for mutation in 0..7 {
                let mut wire = lock.document().clone();
                let artifacts = artifacts(&mut wire, id);
                if id == "pnpm" {
                    let parts = artifacts.remove(&Platform::Any).unwrap();
                    artifacts.insert(Platform::MacosArm64, parts.clone());
                    artifacts.insert(platform, parts);
                }
                let parts = artifacts.get_mut(&platform).unwrap();
                let artifact = parts.values_mut().next().unwrap();
                match mutation {
                    0 => artifact.url = "https://example.com/wrong.tgz".into(),
                    1 => artifact.format = Format::Tar,
                    2 => artifact.root_prefix = Some("wrong-root".into()),
                    3 => artifact.destination = Some("nested".into()),
                    4 => {
                        artifact.executables.insert(id.into(), "wrong/path".into());
                    }
                    5 => {
                        artifact
                            .executables
                            .insert("alien".into(), "bin/alien".into());
                    }
                    _ => {
                        let extra = artifact.clone();
                        parts.insert("extra".into(), extra);
                    }
                }
                // Collision-invalid documents fail at schema validation; others
                // must fail the native adapter checks a record decodes with.
                match Lock::new(wire) {
                    Ok(bad) => reject(bad),
                    Err(error) => assert_eq!(
                        error,
                        crate::lock::Error::Invalid("co-active executable names collide")
                    ),
                }
            }
        }
    }
}

#[test]
fn exact_versions_and_every_authored_integrity_constraint_are_enforced() {
    for manager in [
        "pnpm@11.1.0".to_owned(),
        "pnpm@^10".into(),
        format!("pnpm@10.1.0+sha256.{}", "c".repeat(64)),
        format!("pnpm@10.1.0+sha512.{}", "c".repeat(128)),
        format!("pnpm@10.1.0+sha1.{}", "c".repeat(40)),
    ] {
        let repo = root(json!({"packageManager":manager}));
        let snapshot = Snapshot::capture(repo.path()).unwrap();
        assert!(NativeRecord::from_snapshot(&snapshot, &selection(&snapshot)).is_err());
    }
    let (_repo, snapshot, lock) =
        fixture(json!({"packageManager":format!("pnpm@10.1.0+sha256.{}", "b".repeat(64))}));
    NativeRecord::from_snapshot(&snapshot, &lock).unwrap();
    let mut wire = lock.document().clone();
    let artifacts = artifacts(&mut wire, "pnpm");
    let mut foreign = artifacts.remove(&Platform::Any).unwrap();
    artifacts.insert(Platform::MacosArm64, foreign.clone());
    foreign.values_mut().next().unwrap().sha256 = "c".repeat(64);
    artifacts.insert(Platform::WindowsX64, foreign);
    reject(Lock::new(wire).unwrap());
}

#[test]
fn selected_cohort_does_not_require_unselected_pnpm_coverage() {
    let (_repo, snapshot, lock) = fixture(json!({"packageManager":"pnpm@10.1.0"}));
    let mut wire = lock.document().clone();
    let variants = artifacts(&mut wire, "pnpm");
    let payload = variants.remove(&Platform::Any).unwrap();
    variants.insert(Platform::MacosArm64, payload.clone());
    let partial = Lock::new(wire.clone()).unwrap();
    let native = NativeRecord::from_snapshot(&snapshot, &partial).unwrap();
    let prior = NativeRecord::decode(native.bytes()).unwrap();
    assert_eq!(prior.selection(), &partial);
    let variants = artifacts(&mut wire, "pnpm");
    variants.clear();
    variants.insert(Platform::MacosX64, payload);
    let missing = Lock::new(wire).unwrap();
    NativeRecord::from_snapshot(&snapshot, &missing).unwrap();
}

#[test]
fn ignored_runtime_entries_and_forged_holes_have_an_explicit_format_diagnosis() {
    let (_repo, snapshot, lock) = fixture(json!({"devEngines":{"runtime":[
        {"name":"deno","version":"2.x"},{"name":"node","version":"24.x"}]}}));
    assert!(lock.matches_native(snapshot.declarations()).unwrap());
    let requirements = snapshot.node_requirements().unwrap();
    assert!(requirements.matches_locked_version(&semver::Version::new(24, 1, 0)));
    let error = NativeRecord::from_snapshot(&snapshot, &lock).err().unwrap();
    assert!(matches!(error, Error::RuntimeArrayGap(0)));
    let message = error.to_string();
    assert!(message.contains("package.json#devEngines.runtime[0]"));
    assert!(message.contains("does not retain ignored non-Node runtime entries"));
    assert!(matches!(
        NativeRecord::decode(&lock.canonical_bytes().unwrap()),
        Err(Error::RuntimeArrayGap(0))
    ));
    let mut forged = lock.document().clone();
    forged.tools.get_mut("node").unwrap().declarations[1].field =
        Some("devEngines.runtime[63].version".into());
    assert!(NativeRecord::decode(&Lock::new(forged).unwrap().canonical_bytes().unwrap()).is_err());
}
