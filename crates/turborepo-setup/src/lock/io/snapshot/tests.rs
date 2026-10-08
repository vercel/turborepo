use std::{fs, sync::mpsc, thread, time::Duration};

use serde_json::json;
use turborepo_types::{CONFIG_FILE, CONFIG_FILE_JSONC, CONFIG_FILES};

use super::{
    super::{
        probe_native,
        tests::{selection, writer_root},
        write,
    },
    *,
};
use crate::{node_discovery::MAX_MANIFEST_BYTES, version_request::MAX_REQUEST_BYTES};

fn write_file(root: &Path, file: &str, bytes: impl AsRef<[u8]>) {
    fs::write(root.join(file), bytes).unwrap();
}
fn reject_oversized(root: &Path, file: &str, limit: usize) {
    write_file(root, file, vec![b'x'; limit + 1]);
    assert!(Snapshot::capture(root).is_err());
}

fn root() -> tempfile::TempDir {
    let root = writer_root();
    write_file(root.path(), ".nvmrc", ">=24 <30\n");
    write_file(root.path(), "package.json", "{}");
    write_file(
        root.path(),
        CONFIG_FILE,
        r#"{"futureFlags":{"experimentalSetup":true}}"#,
    );
    root
}
fn candidate(snapshot: &Snapshot, version: &str) -> Lock {
    selection(snapshot.declarations().clone(), version)
}
fn assert_conflict(result: Result<WriteOutcome, StorageError>) {
    assert!(matches!(result, Err(StorageError::Conflict)), "{result:?}");
}
fn locked_version(root: &Path) -> String {
    let lock = Lock::read(root).unwrap().unwrap();
    lock.tools()["node"].version.clone()
}
fn no_stage(root: &Path) {
    assert!(!root.join(".turbo/setup-lock/staged").exists());
    assert!(!root.join(".turbo-lock.writer").exists());
}

#[test]
fn capture_and_resolver_inputs_are_read_only_immutable_and_redacted() {
    let root = root();
    write_file(root.path(), "package.json", json!({"description":"credential-secret",
        "packageManager":"npm@11.0.0","devEngines":{"runtime":{"name":"node","version":"24.x","onFail":"error"}}}).to_string());
    let snapshot = Snapshot::capture(root.path()).unwrap();
    assert!(snapshot.previous_lock().is_none());
    assert_eq!(snapshot.declarations(), &probe_native(root.path()).unwrap());
    assert!(!format!("{snapshot:?}").contains("credential-secret"));
    let changed_manifest = json!({"packageManager":"pnpm@10.0.0",
        "devEngines":{"runtime":{"name":"node","version":"26.x"}}});
    write_file(root.path(), "package.json", changed_manifest.to_string());
    let releases = [
        crate::NodeRelease::new("24.0.0", None).unwrap(),
        crate::NodeRelease::new("26.0.0", None).unwrap(),
    ];
    let requirements = snapshot.node_requirements().unwrap();
    let resolved = requirements.resolve(&releases).unwrap();
    assert_eq!(resolved.version.to_string(), "24.0.0");
    let manager = snapshot.package_manager().unwrap().unwrap().manager;
    assert_eq!(manager, crate::package_manager::Manager::Npm);
    assert!(
        snapshot.declarations()["node"]
            .iter()
            .any(|d| d.field.as_deref() == Some("devEngines.runtime.onFail"))
    );
    assert!(!root.path().join(".turbo").exists());
}

#[test]
fn missing_to_present_and_unchanged_keep_exact_expectations() {
    let root = root();
    let absent = Snapshot::capture(root.path()).unwrap();
    let first = candidate(&absent, "24.0.0");
    assert_eq!(absent.commit(&first).unwrap(), WriteOutcome::Written);
    assert_conflict(absent.commit(&first)); // Not unchanged: expected absence is stale.
    let present = Snapshot::capture(root.path()).unwrap();
    assert_eq!(present.previous_lock().unwrap(), &first);
    let before = fs::metadata(root.path().join(LOCK_NAME)).unwrap();
    assert_eq!(present.commit(&first).unwrap(), WriteOutcome::Unchanged);
    let after = fs::metadata(root.path().join(LOCK_NAME)).unwrap();
    assert_eq!(before.modified().unwrap(), after.modified().unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(before.ino(), after.ino());
    }
    fs::remove_file(root.path().join(LOCK_NAME)).unwrap();
    assert_conflict(present.commit(&candidate(&present, "25.0.0")));
    assert!(!root.path().join(LOCK_NAME).exists());
    no_stage(root.path());
}

#[test]
fn changed_lock_bytes_and_sources_never_overwrite_new_state() {
    for change in [
        "lock-whitespace",
        "package.json",
        ".nvmrc",
        "delete-nvmrc",
        "new-node-version",
        "package-whitespace",
        "package-unrelated",
        "request-whitespace",
        "delete-config",
        "inactive-runtime",
        "runtime-policy",
    ]
    .into_iter()
    .chain(CONFIG_FILES)
    {
        let root = root();
        let initial = Snapshot::capture(root.path()).unwrap();
        write(root.path(), &candidate(&initial, "24.0.0")).unwrap();
        let snapshot = Snapshot::capture(root.path()).unwrap();
        match change {
            "lock-whitespace" => {
                let mut bytes = snapshot.bytes.clone().unwrap();
                bytes.push(b' '); // Same parsed lock, different exact expected bytes.
                fs::write(root.path().join(LOCK_NAME), bytes).unwrap();
            }
            file if CONFIG_FILES.contains(&file) => write_file(root.path(), file,
                r#"{"futureFlags":{"experimentalSetup":false},"setup":{"javascript":"skip"}}"#),
            "inactive-runtime" | "runtime-policy" => write_file(root.path(), "package.json",
                json!({"devEngines":{"runtime":{"name":if change == "inactive-runtime" {"other"} else {"node"},"onFail":"error"}}}).to_string()),
            "package-whitespace" => write_file(root.path(), "package.json", " { }\n"),
            "package-unrelated" => write_file(root.path(), "package.json", r#"{"description":"unrelated"}"#),
            "request-whitespace" => write_file(root.path(), ".nvmrc", "  >=24 <30\r\n"),
            "delete-config" => fs::remove_file(root.path().join(CONFIG_FILE)).unwrap(),
            "delete-nvmrc" => fs::remove_file(root.path().join(".nvmrc")).unwrap(),
            "new-node-version" => write_file(root.path(), ".node-version", "24.0.0"),
            "package.json" => write_file(root.path(), change,
                json!({"devEngines":{"runtime":{"name":"node","onFail":"error"}}}).to_string()),
            _ => write_file(root.path(), change, "26.x"),
        }
        if [
            "package-whitespace",
            "package-unrelated",
            "request-whitespace",
            "inactive-runtime",
        ]
        .contains(&change)
            || CONFIG_FILES.contains(&change)
        {
            assert_eq!(&probe_native(root.path()).unwrap(), snapshot.declarations());
        }
        let current = fs::read(root.path().join(LOCK_NAME)).unwrap();
        for version in ["24.0.0", "25.0.0"] {
            // Even the identical candidate must conflict.
            assert_conflict(snapshot.commit(&candidate(&snapshot, version)));
            assert_eq!(fs::read(root.path().join(LOCK_NAME)).unwrap(), current);
        }
        no_stage(root.path());
    }
}

#[test]
fn mismatched_candidate_and_extra_adapters_fail_before_storage_init() {
    let root = root();
    let snapshot = Snapshot::capture(root.path()).unwrap();
    let mut document = candidate(&snapshot, "24.0.0").document().clone();
    document.tools.get_mut("node").unwrap().declarations[0].request = Some("26.x".into());
    assert!(snapshot.commit(&Lock::new(document).unwrap()).is_err());
    let mut document = candidate(&snapshot, "24.0.0").document().clone();
    let mut extra = document.tools["node"].clone();
    extra.adapter = "generic-download".into();
    extra.installation = super::super::Installation::VerifySystem {
        executables: vec!["extra".into()],
    };
    document.tools.insert("extra".into(), extra);
    let extra = Lock::new(document).unwrap();
    assert!(snapshot.commit(&extra).is_err());
    assert!(!root.path().join(".turbo").exists());
    // An existing extra adapter must not be silently dropped by a native-only
    // update.
    write_file(root.path(), LOCK_NAME, extra.canonical_bytes().unwrap());
    let snapshot = Snapshot::capture(root.path()).unwrap();
    assert!(snapshot.commit(&candidate(&snapshot, "24.0.0")).is_err());
    assert!(!root.path().join(".turbo").exists());
}

#[test]
fn previous_aliases_and_uncovered_sources_are_preserved_without_writer_init() {
    for kind in ["alias", "mismatched-native-id", "uncovered-source"] {
        let root = root();
        let initial = Snapshot::capture(root.path()).unwrap();
        let mut document = candidate(&initial, "24.0.0").document().clone();
        let mut tool = document.tools.remove("node").unwrap();
        let id = match kind {
            "alias" => "custom-node",
            "mismatched-native-id" => "pnpm",
            _ => {
                tool.declarations[0].file = "custom-node.json".into();
                "node"
            }
        };
        document.tools.insert(id.into(), tool);
        let bytes = Lock::new(document).unwrap().canonical_bytes().unwrap();
        fs::write(root.path().join(LOCK_NAME), &bytes).unwrap();
        let snapshot = Snapshot::capture(root.path()).unwrap();
        assert!(snapshot.commit(&candidate(&snapshot, "25.0.0")).is_err());
        assert_eq!(fs::read(root.path().join(LOCK_NAME)).unwrap(), bytes);
        assert!(!root.path().join(".turbo").exists());
    }
}

#[test]
fn final_post_flush_check_aborts_source_and_lock_races_and_failures() {
    for change in [
        "source",
        "lock",
        "failure",
        "lock-created",
        "lock-deleted",
        "source-deleted",
        "source-created",
        "flags-policy",
        "config-deleted",
    ] {
        let root = root();
        let initial = Snapshot::capture(root.path()).unwrap();
        if change != "lock-created" {
            write(root.path(), &candidate(&initial, "24.0.0")).unwrap();
        }
        let snapshot = Snapshot::capture(root.path()).unwrap();
        let previous = snapshot.bytes.clone();
        let external = candidate(&snapshot, "27.0.0").canonical_bytes().unwrap();
        let result = snapshot.commit_with(&candidate(&snapshot, "25.0.0"), || {
            assert_eq!(
                fs::read(root.path().join(".turbo/setup-lock/staged"))?,
                candidate(&snapshot, "25.0.0").canonical_bytes().unwrap()
            );
            match change {
                "source" => fs::write(root.path().join(".nvmrc"), "26.x"),
                "lock" | "lock-created" => fs::write(root.path().join(LOCK_NAME), &external),
                "lock-deleted" => fs::remove_file(root.path().join(LOCK_NAME)),
                "config-deleted" => fs::remove_file(root.path().join(CONFIG_FILE)),
                "source-deleted" => fs::remove_file(root.path().join(".nvmrc")),
                "source-created" => fs::write(root.path().join(".node-version"), "24.0.0"),
                "flags-policy" => fs::write(
                    root.path().join(CONFIG_FILE_JSONC),
                    r#"{"futureFlags":{"experimentalSetup":false},"setup":{}}"#,
                ),
                _ => Err(io::Error::other("injected pre-promotion failure")),
            }
        });
        if change == "failure" {
            assert!(matches!(result, Err(StorageError::Io(_))));
        } else {
            assert_conflict(result);
        }
        assert_eq!(
            read_optional(root.path(), LOCK_NAME, MAX_LOCK_BYTES).unwrap(),
            if ["lock", "lock-created"].contains(&change) {
                Some(external)
            } else if change == "lock-deleted" {
                None
            } else {
                previous
            }
        );
        no_stage(root.path());
        if change == "failure" {
            assert_eq!(
                snapshot.commit(&candidate(&snapshot, "25.0.0")).unwrap(),
                WriteOutcome::Written
            );
        }
    }
}

#[test]
fn competing_resolver_candidates_cannot_publish_last_writer_wins() {
    let root = root();
    let absent = Snapshot::capture(root.path()).unwrap();
    write(root.path(), &candidate(&absent, "24.0.0")).unwrap();
    let snapshot = Snapshot::capture(root.path()).unwrap();
    let first_snapshot = snapshot.clone();
    let (staged_tx, staged_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let first = thread::spawn(move || {
        first_snapshot.commit_with(&candidate(&first_snapshot, "25.0.0"), || {
            staged_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(())
        })
    });
    staged_rx.recv().unwrap();
    assert_eq!(locked_version(root.path()), "24.0.0");
    #[cfg(unix)]
    {
        fs::rename(root.path().join(".turbo"), root.path().join("old-cache")).unwrap();
        fs::create_dir(root.path().join(".turbo")).unwrap();
    }
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let second = thread::spawn(move || {
        started_tx.send(()).unwrap();
        let result = snapshot.commit(&candidate(&snapshot, "26.0.0"));
        done_tx.send(()).unwrap();
        result
    });
    started_rx.recv().unwrap();
    assert!(done_rx.recv_timeout(Duration::from_millis(100)).is_err());
    release_tx.send(()).unwrap();
    assert_eq!(first.join().unwrap().unwrap(), WriteOutcome::Written);
    assert_conflict(second.join().unwrap());
    assert_eq!(locked_version(root.path()), "25.0.0");
    no_stage(root.path());
}

#[test]
fn capture_rejects_oversized_invalid_inputs_without_writer_state() {
    let root = root();
    for file in CONFIG_FILES.into_iter().chain(["package.json"]) {
        reject_oversized(root.path(), file, MAX_MANIFEST_BYTES);
        fs::remove_file(root.path().join(file)).unwrap();
    }
    write_file(root.path(), "package.json", "{}");
    reject_oversized(root.path(), ".nvmrc", MAX_REQUEST_BYTES);
    write_file(root.path(), ".nvmrc", "24.x");
    write_file(root.path(), LOCK_NAME, "{invalid");
    assert!(Snapshot::capture(root.path()).is_err());
    reject_oversized(root.path(), LOCK_NAME, MAX_LOCK_BYTES);
    assert!(!root.path().join(".turbo").exists());
}

#[cfg(unix)]
#[test]
fn readonly_capture_and_symlink_replacement_never_write_or_follow_targets() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join(".nvmrc"), "24.x").unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o555)).unwrap();
    assert!(Snapshot::capture(root.path()).is_ok());
    assert!(!root.path().join(".turbo").exists());
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o755)).unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("source"), ">=24 <30").unwrap();
    fs::remove_file(root.path().join(".nvmrc")).unwrap();
    symlink(outside.path().join("source"), root.path().join(".nvmrc")).unwrap();
    assert!(Snapshot::capture(root.path()).is_err());
    let writer = self::root();
    let snapshot = Snapshot::capture(writer.path()).unwrap();
    fs::remove_file(writer.path().join(".nvmrc")).unwrap();
    symlink(outside.path().join("source"), writer.path().join(".nvmrc")).unwrap();
    assert!(snapshot.commit(&candidate(&snapshot, "24.0.0")).is_err());
    assert!(!writer.path().join(LOCK_NAME).exists());
    assert_eq!(
        fs::read(outside.path().join("source")).unwrap(),
        b">=24 <30"
    );
}
