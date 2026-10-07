use std::{collections::BTreeMap, fs, path::Path};

use super::*;
use crate::lock::{Artifact, Document, Format, Platform, SCHEMA_VERSION};

fn root(manager: Option<&str>) -> tempfile::TempDir {
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
    fs::write(root.path().join(".nvmrc"), "24.x").unwrap();
    manifest(root.path(), manager);
    root
}
fn manifest(root: &Path, manager: Option<&str>) {
    fs::write(
        root.join("package.json"),
        serde_json::to_vec(&manager.map_or_else(
            || serde_json::json!({}),
            |manager| serde_json::json!({"packageManager":manager}),
        ))
        .unwrap(),
    )
    .unwrap();
}
fn snapshot(root: &Path) -> Snapshot {
    Snapshot::capture(root).unwrap()
}
fn selection(snapshot: &Snapshot, version: &str) -> Lock {
    Lock::new(Document {
        schema_version: SCHEMA_VERSION,
        tools: snapshot
            .declarations()
            .iter()
            .map(|(id, declarations)| {
                (
                    id.clone(),
                    Tool {
                        adapter: id.clone(),
                        version: version.into(),
                        declarations: declarations.clone(),
                        options: BTreeMap::new(),
                        installation: Installation::Managed {
                            artifacts: BTreeMap::from([(
                                Platform::LinuxX64Gnu,
                                BTreeMap::from([(
                                    "distribution".into(),
                                    Artifact {
                                        url: format!("https://example.test/{id}-{version}.tgz"),
                                        sha256: "a".repeat(64),
                                        format: Format::TarGz,
                                        root_prefix: Some("package".into()),
                                        destination: None,
                                        executables: BTreeMap::from([(
                                            id.clone(),
                                            format!("bin/{id}"),
                                        )]),
                                    },
                                )]),
                            )]),
                        },
                    },
                )
            })
            .collect(),
    })
    .unwrap()
}
fn first(root: &Path) -> Lock {
    let source = snapshot(root);
    let lock = selection(&source, "24.0.0");
    let result = reconcile(&source, Mode::Local, false, |input| {
        assert_eq!(input.snapshot().declarations(), source.declarations());
        assert!(input.snapshot().previous_lock().is_none());
        assert_eq!(input.version_ids().len(), source.declarations().len());
        assert!(input.ownership_ids().is_empty());
        Ok(lock.clone())
    })
    .unwrap();
    assert_eq!(result.publication, Some(WriteOutcome::Written));
    lock
}
fn never(_: Resolution<'_>) -> Result<Lock, Error> {
    panic!("unexpected resolution")
}
fn bytes(root: &Path) -> Vec<u8> {
    fs::read(root.join("turbo.lock")).unwrap()
}
fn edit(lock: &Lock, change: impl FnOnce(&mut Document)) -> Lock {
    let mut document = lock.document().clone();
    change(&mut document);
    Lock::new(document).unwrap()
}
fn exports(tool: &mut Tool, bundled: bool) {
    tool.options.clear();
    if bundled {
        tool.options
            .insert("bundled-npm".into(), vec!["11.0.0".into()]);
    }
    let Installation::Managed { artifacts } = &mut tool.installation else {
        panic!()
    };
    for parts in artifacts.values_mut() {
        for artifact in parts.values_mut() {
            for name in ["npm", "npx"] {
                if bundled {
                    artifact
                        .executables
                        .insert(name.into(), format!("bin/{name}"));
                } else {
                    artifact.executables.remove(name);
                }
            }
        }
    }
}

#[test]
fn first_lock_repeat_force_is_not_a_resolution_control_and_offline_reuse() {
    let root = root(Some("pnpm@24.0.0"));
    let lock = first(root.path());
    let before = bytes(root.path());
    let source = snapshot(root.path());
    for mode in [Mode::Local, Mode::Frozen, Mode::NoLock] {
        // Reinstallation/force is deliberately outside this API: it uses Local.
        let result = reconcile(&source, mode, true, never).unwrap();
        assert_eq!(result.lock, lock);
        assert_eq!(
            result.publication,
            if mode == Mode::Local {
                Some(WriteOutcome::Unchanged)
            } else {
                None
            }
        );
        assert_eq!(bytes(root.path()), before);
    }
}

#[test]
fn frozen_missing_and_drift_and_no_lock_stale_fail_before_callback_or_writer() {
    let root = root(None);
    assert!(matches!(
        reconcile(&snapshot(root.path()), Mode::Frozen, false, never),
        Err(Error::Missing)
    ));
    assert!(!root.path().join(".turbo").exists());
    first(root.path());
    let before = bytes(root.path());
    for file in [".nvmrc", ".node-version"] {
        fs::write(root.path().join(file), "24.1.0").unwrap();
        for mode in [Mode::Frozen, Mode::NoLock] {
            assert!(matches!(
                reconcile(&snapshot(root.path()), mode, false, never),
                Err(Error::Drift(_))
            ));
        }
        assert_eq!(bytes(root.path()), before);
    }
}

#[test]
fn no_lock_first_selection_is_ephemeral_and_resolution_failure_preserves_state() {
    let root = root(None);
    let source = snapshot(root.path());
    let result = reconcile(&source, Mode::NoLock, true, |input| {
        assert!(input.offline());
        Ok(selection(input.snapshot(), "24.0.0"))
    })
    .unwrap();
    assert_eq!(result.publication, None);
    assert!(!root.path().join("turbo.lock").exists());
    assert!(!root.path().join(".turbo").exists());
    first(root.path());
    let before = bytes(root.path());
    fs::write(root.path().join(".nvmrc"), "25.x").unwrap();
    for offline in [false, true] {
        let error = reconcile(&snapshot(root.path()), Mode::Local, offline, |input| {
            assert_eq!(input.offline(), offline);
            Err(Error::Resolution(
                "metadata unavailable in injected cache".into(),
            ))
        })
        .unwrap_err();
        assert!(matches!(error, Error::Resolution(_)));
        assert_eq!(bytes(root.path()), before);
        assert!(!root.path().join(".turbo/setup-lock/staged").exists());
    }
}

#[test]
fn changed_node_preserves_unaffected_manager_selection_bytes_and_rejects_tampering() {
    let root = root(Some("pnpm@24.0.0"));
    let old = first(root.path());
    let manager_bytes = serde_json::to_vec(&old.tools()["pnpm"]).unwrap();
    fs::write(root.path().join(".nvmrc"), "25.x").unwrap();
    let source = snapshot(root.path());
    let mut candidate = selection(&source, "25.0.0").document().clone();
    candidate
        .tools
        .insert("pnpm".into(), old.tools()["pnpm"].clone());
    let candidate = Lock::new(candidate).unwrap();
    let tampered = edit(&candidate, |doc| {
        doc.tools.get_mut("pnpm").unwrap().version = "24.1.0".into()
    });
    assert!(matches!(
        reconcile(&source, Mode::Local, false, |_| Ok(tampered)),
        Err(Error::Unaffected(_))
    ));
    assert_eq!(bytes(root.path()), old.canonical_bytes().unwrap());
    let result = reconcile(&source, Mode::Local, false, |input| {
        assert_eq!(input.version_ids(), &BTreeSet::from(["node".into()]));
        assert!(input.ownership_ids().is_empty());
        assert_eq!(input.snapshot().previous_lock().unwrap(), &old);
        Ok(candidate)
    })
    .unwrap();
    assert_eq!(
        serde_json::to_vec(&result.lock.tools()["pnpm"]).unwrap(),
        manager_bytes
    );
}

#[test]
fn manager_identity_integrity_and_policy_changes_are_detected_without_node_refresh() {
    for new in [
        serde_json::json!({"packageManager":"npm@24.0.0"}),
        serde_json::json!({"packageManager":format!("pnpm@24.0.0+sha512.{}", "b".repeat(128))}),
        serde_json::json!({"packageManager":"pnpm@24.0.0", "devEngines":{
            "packageManager":{"name":"pnpm", "version":"24.x", "onFail":"warn"}}}),
        serde_json::json!({"devEngines":{"packageManager":{"name":"pnpm"}}}),
    ] {
        let root = root(Some("pnpm@24.0.0"));
        let old = first(root.path());
        fs::write(
            root.path().join("package.json"),
            serde_json::to_vec(&new).unwrap(),
        )
        .unwrap();
        let source = snapshot(root.path());
        let candidate = edit(&selection(&source, "24.0.0"), |doc| {
            doc.tools.insert("node".into(), old.tools()["node"].clone());
        });
        let result = reconcile(&source, Mode::Local, false, |input| {
            assert_eq!(input.version_ids().len(), 1);
            assert!(!input.version_ids().contains("node"));
            assert_eq!(input.ownership_ids(), &BTreeSet::from(["node".into()]));
            assert!(input.snapshot().package_manager().unwrap().is_some());
            Ok(candidate)
        })
        .unwrap();
        assert_eq!(result.lock.tools()["node"], old.tools()["node"]);
        assert!(result.lock.matches_native(source.declarations()).unwrap());
    }
}

#[test]
fn removals_need_no_version_resolution_but_manager_removal_can_restore_node_exports() {
    let root = root(Some("npm@24.0.0"));
    let old = first(root.path());
    manifest(root.path(), None);
    let source = snapshot(root.path());
    let candidate = edit(&old, |doc| {
        doc.tools.remove("npm");
        exports(doc.tools.get_mut("node").unwrap(), true);
    });
    let result = reconcile(&source, Mode::Local, false, |input| {
        assert!(input.version_ids().is_empty());
        assert_eq!(input.ownership_ids(), &BTreeSet::from(["node".into()]));
        Ok(candidate)
    })
    .unwrap();
    assert_eq!(result.lock.tools()["node"].version, "24.0.0");
    fs::remove_file(root.path().join(".nvmrc")).unwrap();
    let result = reconcile(&snapshot(root.path()), Mode::Local, false, never).unwrap();
    assert!(result.lock.tools().is_empty());
}

#[test]
fn bundled_node_to_pinned_npm_requires_complete_noncolliding_cohort_without_new_node_bytes() {
    let root = root(None);
    let source = snapshot(root.path());
    let old = edit(&selection(&source, "24.0.0"), |doc| {
        exports(doc.tools.get_mut("node").unwrap(), true)
    });
    source.commit(&old).unwrap();
    // Even a matching bundled version requires an npm Tool in the native map.
    manifest(root.path(), Some("npm@11.0.0"));
    let source = snapshot(root.path());
    let candidate = edit(&selection(&source, "11.0.0"), |doc| {
        let mut node = old.tools()["node"].clone();
        exports(&mut node, false);
        doc.tools.insert("node".into(), node);
    });
    for change in [
        "version",
        "digest",
        "url",
        "format",
        "layout",
        "option",
        "node-path",
    ] {
        let bad = edit(&candidate, |doc| {
            let node = doc.tools.get_mut("node").unwrap();
            if change == "version" {
                node.version = "24.1.0".into();
            } else if change == "option" {
                node.options.insert("extra".into(), vec!["value".into()]);
            } else {
                let Installation::Managed { artifacts } = &mut node.installation else {
                    panic!()
                };
                let artifact = artifacts
                    .values_mut()
                    .next()
                    .unwrap()
                    .values_mut()
                    .next()
                    .unwrap();
                match change {
                    "digest" => artifact.sha256 = "b".repeat(64),
                    "url" => artifact.url = "https://other.test/node.tgz".into(),
                    "format" => artifact.format = Format::Zip,
                    "layout" => artifact.root_prefix = Some("other".into()),
                    _ => {
                        artifact
                            .executables
                            .insert("node".into(), "other/node".into());
                    }
                }
            }
        });
        assert!(matches!(
            reconcile(&source, Mode::Local, false, |_| Ok(bad)),
            Err(Error::Unaffected(_))
        ));
        assert_eq!(bytes(root.path()), old.canonical_bytes().unwrap());
    }
    assert!(reconcile(&source, Mode::Local, false, |_| Ok(old.clone())).is_err());
    let mut collision = candidate.document().clone();
    collision
        .tools
        .insert("node".into(), old.tools()["node"].clone());
    assert!(Lock::new(collision).is_err());
    let result = reconcile(&source, Mode::Local, false, |input| {
        assert_eq!(input.version_ids(), &BTreeSet::from(["npm".into()]));
        assert_eq!(input.ownership_ids(), &BTreeSet::from(["node".into()]));
        Ok(candidate)
    })
    .unwrap();
    assert_eq!(
        without_npm_exports(&result.lock.tools()["node"]),
        without_npm_exports(&old.tools()["node"])
    );
}

#[test]
fn refresh_is_explicit_noop_is_stable_and_repeated_local_retains_floating_pin() {
    let root = root(Some("pnpm@24.0.0"));
    let old = first(root.path());
    let source = snapshot(root.path());
    let before = fs::metadata(root.path().join("turbo.lock"))
        .unwrap()
        .modified()
        .unwrap();
    let result = reconcile(&source, Mode::Refresh, false, |input| {
        assert_eq!(
            input.version_ids(),
            &BTreeSet::from(["node".into(), "pnpm".into()])
        );
        Ok(old.clone())
    })
    .unwrap();
    assert_eq!(result.publication, Some(WriteOutcome::Unchanged));
    assert_eq!(
        fs::metadata(root.path().join("turbo.lock"))
            .unwrap()
            .modified()
            .unwrap(),
        before
    );
    let newer = edit(&old, |doc| {
        doc.tools.get_mut("node").unwrap().version = "24.1.0".into()
    });
    reconcile(&source, Mode::Refresh, false, |_| Ok(newer.clone())).unwrap();
    assert_eq!(
        reconcile(&snapshot(root.path()), Mode::Local, true, never)
            .unwrap()
            .lock,
        newer
    );
}

#[test]
fn source_cas_including_config_presence_and_no_lock_after_resolution() {
    for mode in [Mode::Local, Mode::NoLock] {
        for file in [
            "package.json",
            ".nvmrc",
            ".node-version",
            "turbo.json",
            "turbo.jsonc",
        ] {
            let root = root(None);
            let source = snapshot(root.path());
            let candidate = selection(&source, "24.0.0");
            let result = reconcile(&source, mode, false, |_| {
                fs::write(root.path().join(file), " { }\n").unwrap();
                Ok(candidate)
            });
            assert!(matches!(
                result,
                Err(Error::Storage(StorageError::Conflict))
            ));
            assert!(!root.path().join("turbo.lock").exists());
        }
    }
    let root = root(None);
    let source = snapshot(root.path());
    fs::write(root.path().join("turbo.json"), "{}").unwrap();
    assert!(matches!(
        reconcile(&source, Mode::Local, false, never),
        Err(Error::Storage(StorageError::Conflict))
    ));
}

#[test]
fn reordered_provenance_preserves_pins_and_native_removal_preserves_other_tool() {
    let root = root(Some("pnpm@24.0.0"));
    fs::write(root.path().join(".node-version"), "24.x").unwrap();
    let old = first(root.path());
    let reordered = edit(&old, |doc| {
        doc.tools.get_mut("node").unwrap().declarations.reverse()
    });
    fs::write(
        root.path().join("turbo.lock"),
        serde_json::to_vec(reordered.document()).unwrap(),
    )
    .unwrap();
    for mode in [Mode::Frozen, Mode::NoLock, Mode::Local] {
        let result = reconcile(&snapshot(root.path()), mode, true, never).unwrap();
        assert_eq!(result.lock.tools()["node"].version, "24.0.0");
    }
    fs::remove_file(root.path().join(".nvmrc")).unwrap();
    fs::remove_file(root.path().join(".node-version")).unwrap();
    let result = reconcile(&snapshot(root.path()), Mode::Local, true, never).unwrap();
    assert_eq!(result.lock.tools().len(), 1);
    assert_eq!(result.lock.tools()["pnpm"], old.tools()["pnpm"]);
}

#[test]
fn failed_refresh_or_source_cas_keeps_existing_lock_and_captured_requirements() {
    let root = root(None);
    let old = first(root.path());
    let source = snapshot(root.path());
    assert!(
        reconcile(&source, Mode::Refresh, true, |_| Err(Error::Resolution(
            "offline miss".into()
        )))
        .is_err()
    );
    assert_eq!(bytes(root.path()), old.canonical_bytes().unwrap());
    let result = reconcile(&source, Mode::Refresh, false, |input| {
        fs::write(root.path().join(".nvmrc"), "25.x").unwrap();
        let resolved = input
            .snapshot()
            .node_requirements()
            .unwrap()
            .resolve(&[
                crate::NodeRelease::new("24.0.0", None).unwrap(),
                crate::NodeRelease::new("25.0.0", None).unwrap(),
            ])
            .unwrap();
        assert_eq!(resolved.version.to_string(), "24.0.0");
        Ok(old.clone())
    });
    assert!(matches!(
        result,
        Err(Error::Storage(StorageError::Conflict))
    ));
    assert_eq!(bytes(root.path()), old.canonical_bytes().unwrap());
    assert!(!root.path().join(".turbo/setup-lock/staged").exists());
}

#[test]
fn unknown_adapters_aliases_sources_and_unsupported_managers_fail_closed() {
    for kind in ["adapter", "alias", "source"] {
        let root = root(None);
        let lock = edit(&selection(&snapshot(root.path()), "24.0.0"), |doc| {
            let mut node = doc.tools.remove("node").unwrap();
            let id = if kind == "alias" {
                "custom-node"
            } else {
                "node"
            };
            if kind == "adapter" {
                node.adapter = "unknown".into();
            }
            if kind == "source" {
                node.declarations[0].file = "custom.json".into();
            }
            doc.tools.insert(id.into(), node);
        });
        fs::write(
            root.path().join("turbo.lock"),
            lock.canonical_bytes().unwrap(),
        )
        .unwrap();
        let before = bytes(root.path());
        for mode in [Mode::Local, Mode::Refresh, Mode::Frozen, Mode::NoLock] {
            assert!(reconcile(&snapshot(root.path()), mode, false, never).is_err());
            assert_eq!(bytes(root.path()), before);
            assert!(!root.path().join(".turbo").exists());
        }
    }
    let root = root(Some("yarn@4.0.0"));
    assert!(Snapshot::capture(root.path()).is_err());
    assert!(!root.path().join(".turbo").exists());
}
