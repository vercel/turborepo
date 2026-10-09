#![allow(clippy::unwrap_used)]
use std::{fs, path::Path};

use turborepo_tool_install::{Error as InstallError, Tool as InstalledTool};

use super::*;
use crate::{
    lock::{Artifact, Document, Format},
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
fn desired(lock: &Lock, snapshot: &Snapshot) -> Vec<InstalledTool> {
    let node = NodePlan::from_lock(lock, Platform::MacosArm64).unwrap();
    let mut tools = vec![node.inventory_tool().clone()];
    if let Some(manager) = snapshot.package_manager().unwrap() {
        let pnpm = PnpmPlan::from_declaration(lock, Platform::MacosArm64, &node, &manager).unwrap();
        tools.push(pnpm.inventory_tool().clone());
    }
    tools
}
fn fixture(manifest: Value) -> (tempfile::TempDir, Snapshot, Lock, Vec<InstalledTool>, Store) {
    let repo = root(manifest);
    let snapshot = Snapshot::capture(repo.path()).unwrap();
    let lock = selection(&snapshot);
    let tools = desired(&lock, &snapshot);
    let store = Store::open(repo.path()).unwrap();
    (repo, snapshot, lock, tools, store)
}
type Artifacts = BTreeMap<Platform, BTreeMap<String, Artifact>>;
fn artifacts<'a>(wire: &'a mut Document, id: &str) -> &'a mut Artifacts {
    match &mut wire.tools.get_mut(id).unwrap().installation {
        Installation::Managed { artifacts } => artifacts,
        _ => panic!("managed fixture required"),
    }
}
fn populate(tool: &InstalledTool, tree: &Path) -> Result<(), InstallError> {
    for path in tool.executables.values() {
        let path = tree.join(path);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(&path, b"fixture; never executed")?;
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
    }
    if tool.id == "pnpm" {
        fs::write(tree.join("package.json"), json!({"name":"pnpm","version":tool.version,"bin":{"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"}}).to_string())?;
    }
    Ok(())
}
fn publish(store: &mut Store, tools: &[InstalledTool], bytes: Vec<u8>) {
    publish_with(store, tools, bytes, populate);
}
fn publish_with(
    store: &mut Store,
    tools: &[InstalledTool],
    bytes: Vec<u8>,
    stage: impl FnMut(&InstalledTool, &Path) -> Result<(), InstallError>,
) {
    let record = Record::new(bytes).unwrap();
    let expected = store.generation().unwrap();
    store
        .reconcile_recorded_checked(tools, &record, &expected, false, stage, || Ok(()))
        .unwrap();
}
fn assert_invalid(result: Result<Option<Baseline>, Error>) {
    assert!(matches!(result, Err(Error::Invalid)));
}
fn reject(snapshot: &Snapshot, store: &mut Store, tools: &[InstalledTool], lock: Lock) {
    publish(store, tools, lock.canonical_bytes().unwrap());
    assert_invalid(Baseline::capture(snapshot, store));
}

#[test]
fn canonical_record_retains_typed_previous_provenance_not_a_fabricated_disk_lock() {
    let (repo, snapshot, lock, tools, mut store) = fixture(
        json!({"packageManager":"pnpm@10.1.0", "secret":"never-record-me",
        "devEngines":{"runtime":[{"name":"node"},{"name":"node","version":"24.x","onFail":"error"}],
        "packageManager":[{"name":"pnpm","version":"10.1.0","onFail":"error"}]}}),
    );
    let native = NativeRecord::from_snapshot(&snapshot, &lock).unwrap();
    let bytes = native.record().bytes();
    let repeated = NativeRecord::from_snapshot(&snapshot, &lock).unwrap();
    assert_eq!(bytes, lock.canonical_bytes().unwrap());
    assert_eq!(bytes, repeated.record().bytes());
    let text = String::from_utf8_lossy(bytes);
    assert!(!text.contains("never-record-me"));
    assert!(!text.contains(repo.path().to_str().unwrap()));
    publish(&mut store, &tools, bytes.to_vec());
    let before = Baseline::capture(&snapshot, &store).unwrap().unwrap();
    fs::write(repo.path().join(".nvmrc"), "22.x").unwrap();
    assert!(before.check(&snapshot, &store).is_err());
    let changed = Snapshot::capture(repo.path()).unwrap();
    let baseline = Baseline::capture(&changed, &store).unwrap().unwrap();
    let prior = baseline.native(&changed, &store).unwrap();
    assert_eq!(prior.selection(), &lock);
    let version = semver::Version::new(24, 1, 0);
    assert!(prior.node_requirements().matches_locked_version(&version));
    assert_eq!(prior.package_manager().unwrap().manager, Manager::Pnpm);
    assert!(changed.previous_lock().is_none());
    assert!(!repo.path().join("turbo.lock").exists());
    changed.ensure_current().unwrap();
    store.check_generation(baseline.generation()).unwrap();
    assert!(NativeRecord::from_snapshot(&changed, &lock).is_err());
}

#[test]
fn strict_decode_rejects_noncanonical_unknown_duplicate_schema_and_oversized_records() {
    let (_repo, snapshot, lock, tools, mut store) = fixture(json!({"engines":{"node":"24.x"}}));
    let bytes = lock.canonical_bytes().unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    let mut unknown = serde_json::to_value(lock.document()).unwrap();
    unknown["tools"]["node"]["extra"] = json!(true);
    let schema = "\"schemaVersion\":0";
    let cases = [
        text.replace(schema, "\"schemaVersion\":1").into_bytes(),
        text.replace(schema, &format!("{schema},{schema}"))
            .into_bytes(),
        serde_json::to_vec(&unknown).unwrap(),
        serde_json::to_vec_pretty(lock.document()).unwrap(),
        bytes[..bytes.len() - 1].to_vec(),
        [b" ".as_slice(), &bytes].concat(),
        [bytes.as_slice(), b"{}"].concat(),
        vec![b' '; crate::lock::MAX_LOCK_BYTES + 1],
    ];
    for bytes in cases {
        assert!(NativeRecord::decode(&bytes).is_err());
        if bytes.len() <= crate::lock::MAX_LOCK_BYTES {
            publish(&mut store, &tools, bytes);
            assert_invalid(Baseline::capture(&snapshot, &store));
        }
    }
    let mut reordered = lock.document().clone();
    let node = reordered.tools.get_mut("node").unwrap();
    node.declarations.reverse();
    let reordered = Lock::new(reordered).unwrap();
    let canonical = NativeRecord::from_snapshot(&snapshot, &reordered).unwrap();
    assert_eq!(canonical.record().bytes(), bytes);
}

#[test]
fn native_grammar_roundtrip_rejects_invented_and_malformed_sources() {
    let (_repo, snapshot, lock, tools, mut store) =
        fixture(json!({"packageManager":"pnpm@10.1.0"}));
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
        reject(&snapshot, &mut store, &tools, Lock::new(wire).unwrap());
    }
    for pointer in [
        "/secret",
        "/devEngines/packageManager/1/name",
        "/devEngines/packageManager/onFail",
    ] {
        let mut wire = lock.document().clone();
        wire.tools.get_mut("pnpm").unwrap().declarations[0].field = Some(pointer.into());
        reject(&snapshot, &mut store, &tools, Lock::new(wire).unwrap());
    }
    for request in ["22.x", "v24.1.0", "lts/SECRET"] {
        let mut wire = lock.document().clone();
        wire.tools.get_mut("node").unwrap().declarations[0].request = Some(request.into());
        reject(&snapshot, &mut store, &tools, Lock::new(wire).unwrap());
    }
}

#[test]
fn every_portable_variant_is_validated_not_just_the_selected_target() {
    let (_repo, snapshot, lock, tools, mut store) =
        fixture(json!({"packageManager":"pnpm@10.1.0"}));
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
                // must fail the native adapter checks through actual Store data.
                match Lock::new(wire) {
                    Ok(bad) => reject(&snapshot, &mut store, &tools, bad),
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
    let (_repo, snapshot, lock, tools, mut store) =
        fixture(json!({"packageManager":format!("pnpm@10.1.0+sha256.{}", "b".repeat(64))}));
    NativeRecord::from_snapshot(&snapshot, &lock).unwrap();
    let mut wire = lock.document().clone();
    let artifacts = artifacts(&mut wire, "pnpm");
    let mut foreign = artifacts.remove(&Platform::Any).unwrap();
    artifacts.insert(Platform::MacosArm64, foreign.clone());
    foreign.values_mut().next().unwrap().sha256 = "c".repeat(64);
    artifacts.insert(Platform::WindowsX64, foreign);
    reject(&snapshot, &mut store, &tools, Lock::new(wire).unwrap());
}

#[test]
fn root_live_guard_real_lock_sources_and_generation_remain_bound() {
    let (repo, snapshot, lock, tools, mut store) = fixture(json!({}));
    let other = root(json!({}));
    let bytes = lock.canonical_bytes().unwrap();
    assert!(Baseline::capture(&snapshot, &store).unwrap().is_none());
    store.reconcile(&tools, populate).unwrap();
    assert!(Baseline::capture(&snapshot, &store).unwrap().is_none());
    publish(&mut store, &tools, lock.canonical_bytes().unwrap());
    let baseline = Baseline::capture(&snapshot, &store).unwrap().unwrap();
    let other_snapshot = Snapshot::capture(other.path()).unwrap();
    let other_store = Store::open(other.path()).unwrap();
    assert!(Baseline::capture(&other_snapshot, &store).is_err());
    assert!(baseline.check(&other_snapshot, &other_store).is_err());
    drop(store);
    let mut reopened = Store::open(repo.path()).unwrap();
    assert!(baseline.check(&snapshot, &reopened).is_err());
    let current = Baseline::capture(&snapshot, &reopened).unwrap().unwrap();
    reopened
        .reconcile(&tools, |_, _| panic!("reuse required"))
        .unwrap();
    assert!(current.native(&snapshot, &reopened).is_err());
    publish(&mut reopened, &tools, bytes.clone());
    let current = Baseline::capture(&snapshot, &reopened).unwrap().unwrap();
    fs::write(repo.path().join("turbo.lock"), bytes).unwrap();
    assert!(current.check(&snapshot, &reopened).is_err());
    let actual = Snapshot::capture(repo.path()).unwrap();
    let present = Baseline::capture(&actual, &reopened).unwrap().unwrap();
    assert_eq!(actual.previous_lock(), Some(&lock));
    present.check(&actual, &reopened).unwrap();
    fs::write(repo.path().join(".nvmrc"), "24.x\n ").unwrap();
    assert!(present.check(&actual, &reopened).is_err());
}

#[test]
fn selected_payload_inventory_and_corruption_cannot_be_replaced_by_caller_data() {
    for case in 0..8 {
        let (_repo, snapshot, lock, mut tools, mut store) =
            fixture(json!({"packageManager":"pnpm@10.1.0"}));
        match case {
            0 => tools[0].version = "24.2.0".into(),
            1 => tools[1].artifact_sha256 = "c".repeat(64),
            2 => {
                tools.pop();
            }
            3 => tools[1]
                .executables
                .get_mut("pnpm")
                .unwrap()
                .push_str("-forged"),
            _ => {}
        }
        publish_with(
            &mut store,
            &tools,
            lock.canonical_bytes().unwrap(),
            |tool, tree| {
                populate(tool, tree)?;
                if tool.id == "pnpm" && case == 4 {
                    fs::write(
                        tree.join("package.json"),
                        json!({"name":"alien","version":"10.1.0"}).to_string(),
                    )?;
                }
                Ok(())
            },
        );
        if case >= 5 {
            let baseline = Baseline::capture(&snapshot, &store).unwrap().unwrap();
            let current = store.current().unwrap().unwrap();
            let generation = current.bin.parent().unwrap();
            match case {
                5 => fs::write(generation.join("record.json"), b"corrupt").unwrap(),
                6 => fs::write(current.bin.join("node"), b"corrupt").unwrap(),
                _ => fs::remove_dir_all(generation).unwrap(),
            }
            assert!(baseline.check(&snapshot, &store).is_err());
        }
        if case >= 6 {
            let repair = Baseline::capture(&snapshot, &store).unwrap().unwrap();
            assert!(!repair.generation().healthy());
            assert_eq!(repair.native(&snapshot, &store).unwrap().selection(), &lock);
        } else {
            assert!(Baseline::capture(&snapshot, &store).is_err(), "case {case}");
        }
    }
}

#[test]
fn selected_cohort_does_not_require_unselected_pnpm_coverage() {
    let (_repo, snapshot, lock, tools, mut store) =
        fixture(json!({"packageManager":"pnpm@10.1.0"}));
    let mut wire = lock.document().clone();
    let variants = artifacts(&mut wire, "pnpm");
    let payload = variants.remove(&Platform::Any).unwrap();
    variants.insert(Platform::MacosArm64, payload.clone());
    let partial = Lock::new(wire.clone()).unwrap();
    let native = NativeRecord::from_snapshot(&snapshot, &partial).unwrap();
    publish(&mut store, &tools, native.record().bytes().to_vec());
    assert!(store.is_current(&tools).unwrap());
    let baseline = Baseline::capture(&snapshot, &store).unwrap().unwrap();
    let prior = baseline.native(&snapshot, &store).unwrap();
    assert_eq!(prior.selection(), &partial);
    let variants = artifacts(&mut wire, "pnpm");
    variants.clear();
    variants.insert(Platform::MacosX64, payload);
    let missing = Lock::new(wire).unwrap();
    NativeRecord::from_snapshot(&snapshot, &missing).unwrap();
    reject(&snapshot, &mut store, &tools, missing);
}

#[test]
fn ignored_runtime_entries_and_forged_holes_have_an_explicit_format_diagnosis() {
    let (_repo, snapshot, lock, tools, mut store) = fixture(json!({"devEngines":{"runtime":[
        {"name":"deno","version":"2.x"},{"name":"node","version":"24.x"}]}}));
    assert!(lock.matches_native(snapshot.declarations()).unwrap());
    let requirements = snapshot.node_requirements().unwrap();
    assert!(requirements.matches_locked_version(&semver::Version::new(24, 1, 0)));
    let error = NativeRecord::from_snapshot(&snapshot, &lock).err().unwrap();
    assert!(matches!(error, Error::RuntimeArrayGap(0)));
    let message = error.to_string();
    assert!(message.contains("package.json#devEngines.runtime[0]"));
    assert!(message.contains("does not retain ignored non-Node runtime entries"));
    publish(&mut store, &tools, lock.canonical_bytes().unwrap());
    assert!(matches!(
        Baseline::capture(&snapshot, &store),
        Err(Error::RuntimeArrayGap(0))
    ));
    let mut forged = lock.document().clone();
    forged.tools.get_mut("node").unwrap().declarations[1].field =
        Some("devEngines.runtime[63].version".into());
    assert!(NativeRecord::decode(&Lock::new(forged).unwrap().canonical_bytes().unwrap()).is_err());
}
