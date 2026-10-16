use std::{collections::BTreeSet, fs, path::Path};

use serde_json::{Value, json};

use super::*;
use crate::lock::{
    Document, Lock, SCHEMA_VERSION, Snapshot, StorageError, WriteOutcome,
    reconcile::{Error, Mode, Resolution, reconcile},
};

const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn tool() -> Value {
    let archive = json!({"url": "https://downloads.example.test/fixture-1.2.3.tar.gz",
        "sha256": SHA, "format": "tar-gz", "rootPrefix": "fixture-1.2.3",
        "executables": {"fixture": "bin/fixture"}});
    json!({"version": "1.2.3", "artifacts": {"linux-x64-gnu": archive, "macos-arm64": archive,
        "windows-x64": {"url": "https://downloads.example.test/fixture.exe", "sha256": SHA,
            "format": "binary", "executables": {"fixture": "fixture.exe"}}}})
}

fn config(root: &Path, file: &str, tools: &Value) {
    let config = json!({"futureFlags": {"experimentalSetup": true},
        "setup": {"additionalTools": tools}});
    fs::write(root.join(file), serde_json::to_vec_pretty(&config).unwrap()).unwrap();
}

fn repo(tools: &Value) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let git = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .arg(root.path())
        .status()
        .unwrap();
    assert!(git.success());
    fs::write(root.path().join(".gitignore"), "/.turbo/\n").unwrap();
    config(root.path(), "turbo.json", tools);
    root
}

fn never(_: Resolution<'_>) -> Result<Lock, Error> {
    panic!("exact additional tools never need resolution")
}

fn lock_bytes(root: &Path) -> Vec<u8> {
    fs::read(root.join("turbo.lock")).unwrap()
}

fn local(root: &Path) -> Option<WriteOutcome> {
    let snapshot = Snapshot::capture(root).unwrap();
    reconcile(&snapshot, Mode::Local, false, never)
        .unwrap()
        .publication
}

#[test]
fn declarations_lock_as_portable_exact_entries() {
    let root = repo(&json!({"fixture": tool()}));
    assert_eq!(local(root.path()), Some(WriteOutcome::Written));
    let lock: Value = serde_json::from_slice(&lock_bytes(root.path())).unwrap();
    let fixture = &lock["tools"]["fixture"];
    assert_eq!(fixture["adapter"], ADAPTER);
    assert_eq!(fixture["version"], "1.2.3");
    assert_eq!(
        fixture["declarations"],
        json!([{"file": "turbo.json", "field": "/setup/additionalTools/fixture",
            "request": "1.2.3"}])
    );
    let artifacts = &fixture["installation"]["artifacts"];
    assert_eq!(
        artifacts["linux-x64-gnu"][PART],
        json!({"url": "https://downloads.example.test/fixture-1.2.3.tar.gz", "sha256": SHA,
            "format": "tar-gz", "rootPrefix": "fixture-1.2.3",
            "executables": {"fixture": "bin/fixture"}})
    );
    // Exactly the declared platforms; nothing is added for the current host.
    let platforms: BTreeSet<_> = artifacts
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        platforms,
        BTreeSet::from(["linux-x64-gnu", "macos-arm64", "windows-x64"])
    );

    // Repeats are no-ops, and a clone elsewhere locks byte-identical entries.
    assert_eq!(local(root.path()), Some(WriteOutcome::Unchanged));
    let elsewhere = repo(&json!({"fixture": tool()}));
    local(elsewhere.path());
    assert_eq!(lock_bytes(elsewhere.path()), lock_bytes(root.path()));
}

#[test]
fn frozen_and_no_lock_report_drift_without_writing() {
    let root = repo(&json!({"fixture": tool()}));
    local(root.path());
    let locked = lock_bytes(root.path());
    let mut digest = json!({"fixture": tool()});
    digest["fixture"]["artifacts"]["macos-arm64"]["sha256"] = json!("f".repeat(64));
    let mut platform = json!({"fixture": tool()});
    platform["fixture"]["artifacts"]["linux-arm64-gnu"] =
        tool()["artifacts"]["linux-x64-gnu"].clone();
    let mut layout = json!({"fixture": tool()});
    layout["fixture"]["artifacts"]["linux-x64-gnu"]["executables"]["fixture"] = json!("fixture");
    // The version text is unchanged in every case.
    for changed in [digest, platform, layout, json!({})] {
        config(root.path(), "turbo.json", &changed);
        let snapshot = Snapshot::capture(root.path()).unwrap();
        for mode in [Mode::Frozen, Mode::NoLock] {
            let error = reconcile(&snapshot, mode, false, never).unwrap_err();
            assert!(
                matches!(&error, Error::Drift(ids) if ids == &BTreeSet::from(["fixture".into()])),
                "{error:?}"
            );
            assert!(error.to_string().contains("run `turbo setup` locally"));
        }
        assert_eq!(lock_bytes(root.path()), locked);
    }
    // Unchanged declarations freeze without touching the lock.
    config(root.path(), "turbo.json", &json!({"fixture": tool()}));
    let snapshot = Snapshot::capture(root.path()).unwrap();
    for mode in [Mode::Frozen, Mode::NoLock] {
        let outcome = reconcile(&snapshot, mode, false, never).unwrap();
        assert_eq!(outcome.publication, None);
        assert_eq!(outcome.lock.canonical_bytes().unwrap(), locked);
    }
}

#[test]
fn local_and_refresh_rewrite_only_changed_declarations() {
    let mut tools = json!({"fixture": tool(), "other": tool()});
    tools["other"]["artifacts"]["linux-x64-gnu"]["executables"] = json!({"other": "bin/other"});
    tools["other"]["artifacts"]
        .as_object_mut()
        .unwrap()
        .retain(|platform, _| platform == "linux-x64-gnu");
    let root = repo(&tools);
    local(root.path());
    let before = Lock::parse(&lock_bytes(root.path())).unwrap();

    tools["fixture"]["artifacts"]["macos-arm64"]["url"] =
        json!("https://mirror.example.test/fixture-1.2.3.tar.gz");
    config(root.path(), "turbo.json", &tools);
    assert_eq!(local(root.path()), Some(WriteOutcome::Written));
    let after = Lock::parse(&lock_bytes(root.path())).unwrap();
    assert_eq!(after.tools()["other"], before.tools()["other"]);
    assert_ne!(after.tools()["fixture"], before.tools()["fixture"]);

    // An explicit refresh has nothing to resolve for exact declarations.
    let snapshot = Snapshot::capture(root.path()).unwrap();
    let refreshed = reconcile(&snapshot, Mode::Refresh, false, never).unwrap();
    assert_eq!(refreshed.publication, Some(WriteOutcome::Unchanged));

    // Removing a declaration removes its entry.
    tools.as_object_mut().unwrap().remove("other");
    config(root.path(), "turbo.json", &tools);
    assert_eq!(local(root.path()), Some(WriteOutcome::Written));
    let removed = Lock::parse(&lock_bytes(root.path())).unwrap();
    assert_eq!(removed.tools().keys().collect::<Vec<_>>(), ["fixture"]);
}

#[test]
fn native_selections_are_preserved_when_additional_tools_change() {
    let root = repo(&json!({"fixture": tool()}));
    fs::write(root.path().join(".nvmrc"), "24.x").unwrap();
    let snapshot = Snapshot::capture(root.path()).unwrap();
    let mut node = snapshot.additional_tools()["fixture"].clone();
    node.adapter = "node".into();
    node.version = "24.0.0".into();
    node.declarations = snapshot.declarations()["node"].clone();
    let Installation::Managed { artifacts } = &mut node.installation else {
        unreachable!()
    };
    artifacts.retain(|platform, _| *platform == Platform::LinuxX64Gnu);
    let parts = artifacts.get_mut(&Platform::LinuxX64Gnu).unwrap();
    parts.get_mut(PART).unwrap().executables = BTreeMap::from([("node".into(), "bin/node".into())]);
    let resolved = Lock::new(Document {
        schema_version: SCHEMA_VERSION,
        tools: BTreeMap::from([("node".into(), node.clone())]),
    })
    .unwrap();
    let first = reconcile(&snapshot, Mode::Local, false, |request| {
        // Only the native declaration is resolved.
        assert_eq!(request.version_ids(), &BTreeSet::from(["node".into()]));
        Ok(resolved)
    })
    .unwrap();
    assert_eq!(first.lock.tools().len(), 2);

    let mut changed = json!({"fixture": tool()});
    changed["fixture"]["version"] = json!("1.2.4");
    config(root.path(), "turbo.json", &changed);
    assert_eq!(local(root.path()), Some(WriteOutcome::Written));
    let lock = Lock::parse(&lock_bytes(root.path())).unwrap();
    assert_eq!(lock.tools()["node"], node);
    assert_eq!(lock.tools()["fixture"].version, "1.2.4");
}

#[test]
fn foreign_or_ambiguous_entries_fail_closed() {
    let root = repo(&json!({"fixture": tool()}));
    local(root.path());
    // A hand-written entry that claims another source is not covered.
    let mut document: Document = serde_json::from_slice(&lock_bytes(root.path())).unwrap();
    let fixture = document.tools.get_mut("fixture").unwrap();
    fixture.declarations[0].field = Some("/setup/additionalTools/other".into());
    fs::write(
        root.path().join("turbo.lock"),
        Lock::new(document).unwrap().canonical_bytes().unwrap(),
    )
    .unwrap();
    let snapshot = Snapshot::capture(root.path()).unwrap();
    assert!(matches!(
        reconcile(&snapshot, Mode::Local, false, never),
        Err(Error::Storage(StorageError::Lock(_)))
    ));

    // Both root configurations, or an unparsable one, are never guessed at.
    config(root.path(), "turbo.jsonc", &json!({}));
    assert!(matches!(
        Snapshot::capture(root.path()),
        Err(StorageError::Declarations)
    ));
    fs::remove_file(root.path().join("turbo.jsonc")).unwrap();
    fs::write(root.path().join("turbo.json"), "{\"setup\": {}}").unwrap();
    assert!(matches!(
        Snapshot::capture(root.path()),
        Err(StorageError::Declarations)
    ));
}

#[test]
fn turbo_jsonc_is_the_recorded_declaration_file() {
    let root = repo(&json!({}));
    fs::remove_file(root.path().join("turbo.json")).unwrap();
    config(root.path(), "turbo.jsonc", &json!({"fixture": tool()}));
    let snapshot = Snapshot::capture(root.path()).unwrap();
    let fixture = &snapshot.additional_tools()["fixture"];
    assert_eq!(fixture.declarations[0].file, "turbo.jsonc");
    assert!(covered("fixture", fixture));
    assert!(!covered("other", fixture));
}
