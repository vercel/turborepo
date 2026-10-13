use super::*;

#[test]
fn missing_generation_is_absent_and_failed_repair_preserves_manifest() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let desired = [tool("node"), tool("pnpm")];
    store.reconcile(&desired, populate).unwrap();
    let before = manifest(&store);
    let bin = store.current().unwrap().unwrap().bin;
    fs::remove_dir_all(bin.parent().unwrap()).unwrap();

    assert!(Store::inventory(&store.root).unwrap().is_none());
    assert!(store.current().unwrap().is_none());
    assert!(!store.is_current(&desired).unwrap());
    for tool in &desired {
        assert!(!store.can_reuse(tool).unwrap());
    }
    assert_eq!(manifest(&store), before); // Readiness is read-only.
    let mut calls = Vec::new();
    assert!(
        store
            .reconcile(&desired, |tool, root| {
                calls.push(tool.id.clone());
                populate(tool, root)?;
                if tool.id == "pnpm" {
                    return Err(Error::Io(io::Error::other("interrupted repair")));
                }
                Ok(())
            })
            .is_err()
    );
    assert_eq!(calls, ["node", "pnpm"]);
    assert_eq!(manifest(&store), before);
    assert!(store.current().unwrap().is_none());
    assert_eq!(fs::read_dir(&store.root).unwrap().count(), 2); // lock and old manifest only

    calls.clear();
    assert_eq!(
        store
            .reconcile(&desired, |tool, root| {
                calls.push(tool.id.clone());
                populate(tool, root)
            })
            .unwrap(),
        Outcome::Replaced
    );
    assert_eq!(calls, ["node", "pnpm"]);
    assert_ne!(manifest(&store), before);
    assert!(store.is_current(&desired).unwrap());
    assert!(store.can_reuse(&desired[0]).unwrap());
    assert!(store.can_reuse(&desired[1]).unwrap());
    assert_ne!(store.current().unwrap().unwrap().bin, bin);
}

#[test]
fn whole_set_readiness_and_per_tool_reuse_have_distinct_contracts() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let node = tool("node");
    let pnpm = tool("pnpm");
    assert!(!store.is_current(&[]).unwrap());
    assert!(!store.can_reuse(&node).unwrap());
    store
        .reconcile(std::slice::from_ref(&node), populate)
        .unwrap();
    let before = manifest(&store);
    assert!(store.is_current(std::slice::from_ref(&node)).unwrap());
    assert!(!store.is_current(&[node.clone(), pnpm.clone()]).unwrap());
    assert!(!store.is_current(&[]).unwrap());
    assert!(store.can_reuse(&node).unwrap());
    assert!(!store.can_reuse(&pnpm).unwrap());
    assert_eq!(manifest(&store), before);

    let desired = [pnpm.clone(), node.clone()];
    let mut calls = Vec::new();
    store
        .reconcile(&desired, |tool, root| {
            calls.push(tool.id.clone());
            populate(tool, root)
        })
        .unwrap();
    assert_eq!(calls, ["pnpm"]); // Node needs no preparation when adding another tool.
    assert!(store.is_current(&desired).unwrap());
    assert!(store.is_current(&[node.clone(), pnpm.clone()]).unwrap());
    assert!(store.can_reuse(&node).unwrap());
    let before = manifest(&store);
    assert_eq!(
        store
            .reconcile(&desired, |_, _| panic!("healthy repeat must not stage"))
            .unwrap(),
        Outcome::Unchanged
    );
    assert_eq!(manifest(&store), before);

    assert!(!store.is_current(std::slice::from_ref(&node)).unwrap());
    assert!(store.can_reuse(&node).unwrap());
    assert_eq!(
        store
            .reconcile(std::slice::from_ref(&node), |_, _| panic!(
                "removing a sibling must reuse Node"
            ))
            .unwrap(),
        Outcome::Replaced
    );
    assert!(store.is_current(std::slice::from_ref(&node)).unwrap());
    assert!(store.can_reuse(&node).unwrap());
    assert!(!store.can_reuse(&pnpm).unwrap());
}

#[test]
fn every_tool_identity_field_must_match_before_reuse() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let node = tool("node");
    store
        .reconcile(std::slice::from_ref(&node), populate)
        .unwrap();
    for field in 0..5 {
        let mut changed = node.clone();
        match field {
            0 => changed.id = "other-node".into(),
            1 => changed.version = "2.0.0".into(),
            2 => changed.platform = "darwin-arm64".into(),
            3 => changed.artifact_sha256 = "b".repeat(64),
            _ => {
                changed
                    .executables
                    .insert("alias".into(), "bin/alias".into());
            }
        }
        assert!(!store.can_reuse(&changed).unwrap());
        assert!(!store.is_current(std::slice::from_ref(&changed)).unwrap());
        let mut calls = 0;
        store
            .reconcile(std::slice::from_ref(&changed), |tool, root| {
                calls += 1;
                populate(tool, root)
            })
            .unwrap();
        assert_eq!(calls, 1);
        store
            .reconcile(std::slice::from_ref(&node), populate)
            .unwrap();
    }
}

#[test]
fn damaged_sibling_invalidates_reuse_of_every_tool() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let desired = [tool("node"), tool("pnpm")];
    for damage in 0..6 {
        store.reconcile(&desired, populate).unwrap();
        let before = manifest(&store);
        let bin = store.current().unwrap().unwrap().bin;
        match damage {
            0 => fs::write(bin.join("pnpm"), b"corrupt sibling").unwrap(),
            1 => fs::remove_file(bin.join("pnpm")).unwrap(),
            2 => {
                fs::remove_file(bin.join("pnpm")).unwrap();
                symlink("node", bin.join("pnpm")).unwrap();
            }
            3 => fs::write(bin.join("foreign-shim"), b"unexpected").unwrap(),
            4 => {
                fs::remove_file(bin.join("pnpm")).unwrap();
                fs::write(bin.join("pnpm"), b"not a symlink").unwrap();
            }
            _ => {
                fs::remove_file(bin.join("pnpm")).unwrap();
                fs::create_dir(bin.join("pnpm")).unwrap();
            }
        }
        assert!(store.current().is_err());
        assert!(!store.is_current(&desired).unwrap());
        assert!(!store.can_reuse(&desired[0]).unwrap());
        assert!(!store.can_reuse(&desired[1]).unwrap());
        assert_eq!(manifest(&store), before);
        let mut calls = Vec::new();
        store
            .reconcile(&desired, |tool, root| {
                calls.push(tool.id.clone());
                populate(tool, root)
            })
            .unwrap();
        assert_eq!(calls, ["node", "pnpm"]);
        assert!(store.is_current(&desired).unwrap());
    }
}

fn assert_rejected(store: &mut Store, tool: &Tool) {
    assert!(store.current().is_err());
    assert!(store.is_current(std::slice::from_ref(tool)).is_err());
    assert!(store.can_reuse(tool).is_err());
    let before = manifest(store);
    assert!(
        store
            .reconcile(std::slice::from_ref(tool), |_, _| panic!(
                "invalid metadata must not stage"
            ))
            .is_err()
    );
    assert_eq!(manifest(store), before);
}

#[test]
fn unsafe_or_malformed_inventory_is_not_missing_installation_state() {
    let repo = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let node = tool("node");
    store
        .reconcile(std::slice::from_ref(&node), populate)
        .unwrap();
    let before = manifest(&store);
    for defect in 0..6 {
        let mut forged: Inventory = serde_json::from_slice(&before).unwrap();
        match defect {
            0 => forged.schema = 2,
            1 => forged.generation = "generation-../../escape".into(),
            2 => {
                forged.generation = "generation-missing".into();
                forged.tools[0].tool.id = "../escape".into();
            }
            3 => {
                forged.generation = "generation-missing".into();
                forged.tools[0]
                    .tool
                    .executables
                    .insert("node".into(), "../escape".into());
            }
            4 => {
                forged.generation = "generation-missing".into();
                forged.tools[0].tool.artifact_sha256 = "invalid".into();
            }
            _ => forged.tools.push(forged.tools[0].clone()),
        }
        fs::write(
            store.root.join("manifest.json"),
            serde_json::to_vec(&forged).unwrap(),
        )
        .unwrap();
        assert_rejected(&mut store, &node);
    }
    fs::write(store.root.join("manifest.json"), b"not json").unwrap();
    assert_rejected(&mut store, &node);
    fs::write(store.root.join("manifest.json"), &before).unwrap();
    let generation = store
        .current()
        .unwrap()
        .unwrap()
        .bin
        .parent()
        .unwrap()
        .to_path_buf();
    fs::remove_dir_all(&generation).unwrap();
    symlink(outside.path(), &generation).unwrap();
    assert_rejected(&mut store, &node);
    fs::remove_file(&generation).unwrap();
    symlink(outside.path().join("missing"), &generation).unwrap();
    assert_rejected(&mut store, &node); // Dangling symlink is unsafe, not absent.
    fs::remove_file(&generation).unwrap();
    fs::write(&generation, b"not a directory").unwrap();
    assert_rejected(&mut store, &node);
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);

    let mut invalid = node.clone();
    invalid.id = "../escape".into();
    assert!(store.can_reuse(&invalid).is_err());
    assert!(store.is_current(&[node.clone(), node]).is_err());
}

#[test]
fn malformed_tree_digest_is_rejected_before_inspecting_generation() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let node = tool("node");
    store
        .reconcile(std::slice::from_ref(&node), populate)
        .unwrap();
    let before = manifest(&store);
    let bin = store.current().unwrap().unwrap().bin;
    for missing in [false, true] {
        if missing {
            fs::remove_dir_all(bin.parent().unwrap()).unwrap();
        }
        for digest in [
            String::new(),
            "invalid".into(),
            "a".repeat(63),
            "A".repeat(64),
            "g".repeat(64),
        ] {
            let mut forged: Inventory = serde_json::from_slice(&before).unwrap();
            forged.tools[0].tree_sha256 = digest;
            fs::write(
                store.root.join("manifest.json"),
                serde_json::to_vec(&forged).unwrap(),
            )
            .unwrap();
            assert!(matches!(
                Store::inventory(&store.root),
                Err(Error::InvalidInventory)
            ));
            assert_rejected(&mut store, &node);
        }
    }
}

#[test]
fn unrelated_io_errors_are_not_swallowed_as_repairable_absence() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let node = tool("node");
    store
        .reconcile(std::slice::from_ref(&node), populate)
        .unwrap();
    let before = manifest(&store);
    let bin = store.current().unwrap().unwrap().bin;
    symlink("loop", bin.join("../tools/node/loop")).unwrap();
    // canonicalize encounters ELOOP, not a missing file or a hash mismatch.
    for result in [
        store.is_current(std::slice::from_ref(&node)),
        store.can_reuse(&node),
    ] {
        assert!(matches!(result, Err(Error::Io(e)) if e.kind() != io::ErrorKind::NotFound));
    }
    assert!(
        matches!(store.reconcile(std::slice::from_ref(&node), |_, _| panic!("I/O failure must propagate")), Err(Error::Io(e)) if e.kind() != io::ErrorKind::NotFound)
    );
    assert_eq!(manifest(&store), before);
}

#[test]
fn abrupt_exit_during_missing_generation_repair_keeps_manifest_and_releases_lock() {
    let repo = tempfile::tempdir().unwrap();
    let node = tool("node");
    let before = {
        let mut store = Store::open(repo.path()).unwrap();
        store
            .reconcile(std::slice::from_ref(&node), populate)
            .unwrap();
        let bin = store.current().unwrap().unwrap().bin;
        fs::remove_dir_all(bin.parent().unwrap()).unwrap();
        manifest(&store)
    };
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tests::crash_child", "--ignored"])
        .env("TURBO_INSTALL_CRASH_REPO", repo.path())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(79));
    let mut store = Store::open(repo.path()).unwrap();
    assert_eq!(manifest(&store), before);
    assert!(store.current().unwrap().is_none());
    assert!(!store.can_reuse(&node).unwrap());
    assert_eq!(
        store
            .reconcile(std::slice::from_ref(&node), populate)
            .unwrap(),
        Outcome::Replaced
    );
    assert!(store.is_current(std::slice::from_ref(&node)).unwrap());
}
