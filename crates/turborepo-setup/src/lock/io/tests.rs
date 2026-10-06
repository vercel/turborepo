use std::{fs, sync::mpsc, thread, time::Duration};

use serde_json::json;

use super::*;
use crate::lock::{Document, SCHEMA_VERSION, Tool};

#[path = "edge_tests.rs"]
mod edge_tests;

pub(super) fn selection(declarations: DeclarationMap, version: &str) -> Lock {
    Lock::new(Document {
        schema_version: SCHEMA_VERSION,
        tools: declarations
            .into_iter()
            .map(|(name, declarations)| {
                (
                    name.clone(),
                    Tool {
                        adapter: name.clone(),
                        version: version.into(),
                        declarations,
                        options: BTreeMap::new(),
                        installation: Installation::VerifySystem {
                            executables: vec![name],
                        },
                    },
                )
            })
            .collect(),
    })
    .unwrap()
}

fn fixture(version: &str) -> Lock {
    selection(
        BTreeMap::from([(
            "node".into(),
            vec![Declaration {
                file: ".nvmrc".into(),
                field: None,
                request: Some("lts/*".into()),
            }],
        )]),
        version,
    )
}

pub(super) fn writer_root() -> tempfile::TempDir {
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
    root
}

fn assert_writer_files(root: &Path) {
    assert_eq!(files(root), vec![".git", ".gitignore", ".turbo", LOCK_NAME]);
    assert!(root.join(".turbo/setup-lock/writer").is_file());
    assert!(!root.join(".turbo/setup-lock/staged").exists());
    assert!(!root.join(".turbo-lock.writer").exists());
}

fn files(root: &Path) -> Vec<String> {
    let mut files: Vec<_> = fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into())
        .collect();
    files.sort();
    files
}

#[test]
fn stable_bytes_include_canonical_provenance_platforms_and_no_credentials() {
    let mut doc: Document = serde_json::from_value(json!({"schemaVersion":0,"tools":{
        "node":{"adapter":"node","version":"24.0.0", "declarations":[
            {"file":"package.json","field":"engines.node","request":"^24"},
            {"file":".nvmrc","request":"lts/*"}], "installation":{"kind":"managed","artifacts":{
                "macos-arm64":{"distribution":{"url":"https://example.test/node.tar.gz", "sha256":"a".repeat(64),
                    "format":"tar-gz","executables":{"node":"bin/node"}}},
                "windows-x64":{"distribution":{"url":"https://example.test/node.zip", "sha256":"b".repeat(64),
                    "format":"zip","executables":{"node":"node.exe"}}}}}},
        "rust":{"adapter":"rust","version":"1.90.0","declarations":[{"file":"rust-toolchain.toml"}],
            "installation":{"kind":"verify-system","executables":["rustc","cargo"]}}
    }})).unwrap();
    let bytes = Lock::new(doc.clone()).unwrap().canonical_bytes().unwrap();
    doc.tools.get_mut("node").unwrap().declarations.reverse();
    if let Installation::VerifySystem { executables } =
        &mut doc.tools.get_mut("rust").unwrap().installation
    {
        executables.reverse();
    }
    assert_eq!(
        bytes,
        Lock::new(doc.clone()).unwrap().canonical_bytes().unwrap()
    );
    assert_eq!(bytes.last(), Some(&b'\n'));
    assert_eq!(
        Lock::parse(&bytes).unwrap().canonical_bytes().unwrap(),
        bytes
    );
    for url in [
        "https://secret@example.test/x",
        "https://example.test/x?token=secret",
    ] {
        if let Installation::Managed { artifacts } =
            &mut doc.tools.get_mut("node").unwrap().installation
        {
            artifacts
                .values_mut()
                .next()
                .unwrap()
                .values_mut()
                .next()
                .unwrap()
                .url = url.into();
        }
        let error = Lock::new(doc.clone()).unwrap_err();
        assert!(!error.to_string().contains("secret"));
    }
}

#[test]
fn identical_write_does_not_touch_existing_file_and_changed_write_is_readable() {
    let root = writer_root();
    let first = fixture("24.0.0");
    assert!(Lock::read(root.path()).unwrap().is_none());
    assert_eq!(write(root.path(), &first).unwrap(), WriteOutcome::Written);
    let metadata = fs::metadata(root.path().join(LOCK_NAME)).unwrap();
    assert_eq!(write(root.path(), &first).unwrap(), WriteOutcome::Unchanged);
    let after = fs::metadata(root.path().join(LOCK_NAME)).unwrap();
    assert_eq!(metadata.modified().unwrap(), after.modified().unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(metadata.ino(), after.ino());
    }
    assert_eq!(
        write(root.path(), &fixture("25.0.0")).unwrap(),
        WriteOutcome::Written
    );
    assert_eq!(Lock::read(root.path()).unwrap().unwrap(), fixture("25.0.0"));
    assert_writer_files(root.path());
}

#[test]
fn publication_failure_preserves_previous_bytes_and_ignored_state() {
    let root = writer_root();
    write(root.path(), &fixture("24.0.0")).unwrap();
    let previous = fs::read(root.path().join(LOCK_NAME)).unwrap();
    assert!(
        write_with(root.path(), &fixture("25.0.0"), || {
            Err(io::Error::other("injected failure"))
        })
        .is_err()
    );
    assert_eq!(fs::read(root.path().join(LOCK_NAME)).unwrap(), previous);
    assert_writer_files(root.path());
    fs::write(
        root.path().join(".turbo/setup-lock/staged"),
        b"crashed partial",
    )
    .unwrap();
    assert_eq!(
        write(root.path(), &fixture("24.0.0")).unwrap(),
        WriteOutcome::Unchanged
    );
    assert_writer_files(root.path());
}

#[test]
fn writers_share_persistent_lock_and_readers_see_complete_previous_file() {
    let root = writer_root();
    write(root.path(), &fixture("24.0.0")).unwrap();
    let (staged_tx, staged_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let path = root.path().to_owned();
    let first = thread::spawn(move || {
        write_with(&path, &fixture("25.0.0"), || {
            staged_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(())
        })
        .unwrap()
    });
    staged_rx.recv().unwrap();
    assert_eq!(Lock::read(root.path()).unwrap().unwrap(), fixture("24.0.0"));
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let path = root.path().to_owned();
    let second = thread::spawn(move || {
        started_tx.send(()).unwrap();
        let result = write(&path, &fixture("25.0.0")).unwrap();
        done_tx.send(()).unwrap();
        result
    });
    started_rx.recv().unwrap();
    assert!(done_rx.recv_timeout(Duration::from_millis(100)).is_err());
    release_tx.send(()).unwrap();
    assert_eq!(first.join().unwrap(), WriteOutcome::Written);
    assert_eq!(second.join().unwrap(), WriteOutcome::Unchanged);
    assert_eq!(Lock::read(root.path()).unwrap().unwrap(), fixture("25.0.0"));
    assert_writer_files(root.path());
}

#[test]
fn probe_keeps_all_native_locations_and_never_resolves_latest() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join(".nvmrc"), " lts/*\n").unwrap();
    fs::write(root.path().join(".node-version"), "v24.0.0\n").unwrap();
    let manifest = json!({"engines":{"node":"^24"}, "packageManager":"pnpm@10.0.0",
        "devEngines":{"runtime":[{"name":"node"},{"name":"node","version":"24.x"}],
            "packageManager":[{"name":"npm","version":"^11","onFail":"warn"},
                {"name":"pnpm","onFail":"warn"}]}});
    fs::write(root.path().join("package.json"), manifest.to_string()).unwrap();
    let current = probe_native(root.path()).unwrap();
    assert_eq!(current["node"].len(), 5); // Includes both runtime alternatives.
    assert_eq!(current["pnpm"].len(), 6); // Includes advisory npm and name-only pnpm.
    assert!(
        current["node"]
            .iter()
            .any(|d| d.field.as_deref() == Some("devEngines.runtime[0]") && d.request.is_none())
    );
    let lock = selection(current.clone(), "24.0.0");
    assert!(lock.matches_native(&current).unwrap()); // No release index exists.
    for id in ["node", "pnpm"] {
        for index in 0..current[id].len() {
            for change in ["file", "field", "request", "remove"] {
                let mut changed = current.clone();
                let sources = changed.get_mut(id).unwrap();
                match change {
                    "file" => sources[index].file = "other.json".into(),
                    "field" => sources[index].field = Some("/different".into()),
                    "request" => sources[index].request = Some("changed".into()),
                    _ => {
                        sources.remove(index);
                    }
                }
                assert!(
                    !lock.matches_native(&changed).unwrap(),
                    "{id} {index} {change}"
                );
            }
        }
    }
    let mut reordered = current.clone();
    reordered.values_mut().for_each(|v| v.reverse());
    assert!(lock.matches_native(&reordered).unwrap());
    let mut renamed = current.clone();
    let manager = renamed.remove("pnpm").unwrap();
    renamed.insert("npm".into(), manager);
    assert!(!lock.matches_native(&renamed).unwrap());
    fs::remove_file(root.path().join(".nvmrc")).unwrap();
    assert!(
        !lock
            .matches_native(&probe_native(root.path()).unwrap())
            .unwrap()
    );
    assert!(!root.path().join(".turbo").exists());
}

#[test]
fn merged_runtime_policy_api_changes_native_comparison_without_writes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("package.json");
    fs::write(
        &path,
        json!({"devEngines":{"runtime":{"name":"node"}}}).to_string(),
    )
    .unwrap();
    let before = probe_native(root.path()).unwrap();
    let original = selection(before.clone(), "24.0.0");
    fs::write(
        &path,
        json!({"devEngines":{"runtime":{"name":"node","onFail":"error"}}}).to_string(),
    )
    .unwrap();
    let with_policy = probe_native(root.path()).unwrap();
    assert!(!original.matches_native(&with_policy).unwrap());
    assert!(
        !selection(with_policy, "24.0.0")
            .matches_native(&before)
            .unwrap()
    );
    for policy in ["warn", "ignore"] {
        fs::write(
            &path,
            json!({"devEngines":{"runtime":{"name":"node","onFail":policy}}}).to_string(),
        )
        .unwrap();
        assert!(probe_native(root.path()).is_err());
    }
    assert_eq!(files(root.path()), vec!["package.json"]);
}

#[test]
fn presence_integrity_policy_and_invalid_sources_do_not_fall_back() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("package.json");
    let original = json!({"packageManager":format!("pnpm@10.0.0+sha256.{}", "a".repeat(64)),
        "devEngines":{"packageManager":{"name":"pnpm","onFail":"warn"}}});
    fs::write(&path, original.to_string()).unwrap();
    let lock = selection(probe_native(root.path()).unwrap(), "10.0.0");
    for new in [
        json!({}),
        json!({"packageManager":"pnpm@10.0.0"}),
        json!({"packageManager":"pnpm@10.0.0","devEngines":{"packageManager":{"name":"pnpm","version":"*","onFail":"ignore"}}}),
    ] {
        fs::write(&path, new.to_string()).unwrap();
        assert!(
            !lock
                .matches_native(&probe_native(root.path()).unwrap())
                .unwrap()
        );
    }
    for invalid in [
        "{",
        r#"{"packageManager":"pnpm@10.0.0","packageManager":"npm@11.0.0"}"#,
        r#"{"devEngines":{"runtime":{"name":"node","version":false}}}"#,
    ] {
        fs::write(&path, invalid).unwrap();
        assert!(probe_native(root.path()).is_err());
    }
    fs::write(&path, "{}").unwrap();
    fs::write(root.path().join(".nvmrc"), "not-a-request").unwrap();
    assert!(probe_native(root.path()).is_err());
    assert!(!root.path().join(".turbo").exists());
}

#[test]
fn own_inputs_are_bounded_and_no_partial_probe_or_write_succeeds() {
    let root = writer_root();
    fs::write(root.path().join(".nvmrc"), "lts/*").unwrap();
    fs::write(
        root.path().join("package.json"),
        vec![b' '; crate::node_discovery::MAX_MANIFEST_BYTES + 1],
    )
    .unwrap();
    assert!(probe_native(root.path()).is_err());
    fs::remove_file(root.path().join("package.json")).unwrap();
    fs::write(
        root.path().join(".node-version"),
        vec![b' '; crate::version_request::MAX_REQUEST_BYTES + 1],
    )
    .unwrap();
    assert!(probe_native(root.path()).is_err());
    let oversized = vec![b'x'; MAX_LOCK_BYTES + 1];
    fs::write(root.path().join(LOCK_NAME), &oversized).unwrap();
    assert!(Lock::read(root.path()).is_err());
    assert!(write(root.path(), &fixture("24.0.0")).is_err());
    assert_eq!(fs::read(root.path().join(LOCK_NAME)).unwrap(), oversized);
    // 32 alternatives produce 96 locations, beyond the schema provenance bound.
    fs::remove_file(root.path().join(".node-version")).unwrap();
    let entries: Vec<_> = (0..32)
        .map(|_| json!({"name":"pnpm","version":"*","onFail":"warn"}))
        .collect();
    fs::write(
        root.path().join("package.json"),
        json!({"devEngines":{"packageManager":entries}}).to_string(),
    )
    .unwrap();
    assert!(probe_native(root.path()).is_err());
}

#[cfg(unix)]
#[test]
fn read_only_probe_and_symlink_directory_fifo_safety() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(root.path().join(".nvmrc"), "lts/*").unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o555)).unwrap();
    assert_eq!(
        probe_native(root.path()).unwrap()["node"][0]
            .request
            .as_deref(),
        Some("lts/*")
    );
    assert!(Lock::read(root.path()).unwrap().is_none());
    assert_eq!(files(root.path()), vec![".nvmrc"]);
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(outside.path().join("target"), "untouched").unwrap();
    for name in ["package.json", ".node-version", LOCK_NAME] {
        let root = if name == LOCK_NAME {
            writer_root()
        } else {
            tempfile::tempdir().unwrap()
        };
        for target in ["target", "missing"] {
            symlink(outside.path().join(target), root.path().join(name)).unwrap();
            if name == LOCK_NAME {
                assert!(write(root.path(), &fixture("24.0.0")).is_err());
                assert!(Lock::read(root.path()).is_err());
            } else {
                assert!(probe_native(root.path()).is_err());
            }
            assert_eq!(
                fs::read_to_string(outside.path().join("target")).unwrap(),
                "untouched"
            );
            assert!(!outside.path().join("missing").exists());
            fs::remove_file(root.path().join(name)).unwrap();
        }
    }
    for name in ["package.json", LOCK_NAME] {
        let root = if name == LOCK_NAME {
            writer_root()
        } else {
            tempfile::tempdir().unwrap()
        };
        fs::create_dir(root.path().join(name)).unwrap();
        assert!(if name == "package.json" {
            probe_native(root.path()).is_err()
        } else {
            write(root.path(), &fixture("24.0.0")).is_err()
        });
        fs::remove_dir(root.path().join(name)).unwrap();
    }
    let fifo = root.path().join("package.json");
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(probe_native(root.path()).is_err());
}
