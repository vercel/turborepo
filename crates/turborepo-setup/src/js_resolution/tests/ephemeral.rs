use turborepo_tool_install::Tool as InstalledTool;
#[path = "ephemeral_races.rs"]
mod races;

use super::*;
use crate::{
    ephemeral_reconcile::{self, Staged},
    test_store::{Outcome, TestStore},
};

/// Stages from `previous`, the selection the repository's user record holds.
fn staged(
    snapshot: &Snapshot,
    previous: Option<&[u8]>,
    world: &World,
) -> Result<Staged, ephemeral_reconcile::Error> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    ephemeral_reconcile::stage(
        snapshot,
        previous,
        Platform::MacosArm64,
        false,
        |request| {
            assert!(request.snapshot().previous_lock().is_none());
            runtime
                .block_on(resolve(request, &world.node, &world.registry))
                .map_err(|e| reconcile::Error::Resolution(e.to_string()))
        },
        || Ok(()),
    )
}
fn executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, b"fixture; must never execute").unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}
fn populate(tool: &InstalledTool, tree: &Path) -> Result<(), turborepo_tool_install::Error> {
    for path in tool.tree_files() {
        if tool.id != "node" || !path.ends_with("npm") && !path.ends_with("npx") {
            executable(&tree.join(path));
        }
    }
    if tool.id == "node" && tool.executables.contains_key("npm") {
        let package = tree.join("lib/node_modules/npm");
        for name in ["npm", "npx"] {
            executable(&package.join(format!("bin/{name}-cli.js")));
            std::os::unix::fs::symlink(
                format!("../lib/node_modules/npm/bin/{name}-cli.js"),
                tree.join(format!("bin/{name}")),
            )?;
        }
        fs::write(package.join("package.json"), json!({"name":"npm","version":"11.6.1","bin":{"npm":"bin/npm-cli.js","npx":"bin/npx-cli.js"}}).to_string())?;
    } else if tool.id == "pnpm" {
        fs::write(tree.join("package.json"), json!({"name":"pnpm","version":tool.version,"bin":{"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"}}).to_string())?;
    }
    fs::write(tree.join("resource"), b"adjacent resource")?;
    Ok(())
}
fn assert_installed(store: &TestStore, root: &Path) {
    let current = store.current().unwrap().unwrap();
    for tool in &current.tools {
        assert_eq!(
            fs::read(store.tree_of(tool).join("resource")).unwrap(),
            b"adjacent resource"
        );
        for name in tool.executables.keys() {
            assert!(current.bin.join(name).is_file());
        }
    }
    assert!(!root.join("turbo.lock").exists());
}
/// Publishes like `turbo setup --no-lock`: every freshly staged tree is
/// verified, and the selection becomes the record's.
fn publish(
    next: Staged,
    snapshot: &Snapshot,
    store: &TestStore,
    record: &mut Option<Vec<u8>>,
) -> Outcome {
    next.check(snapshot).unwrap();
    let outcome = store
        .reconcile(next.desired(), |tool, tree| {
            populate(tool, tree)?;
            Ok(next.verify(tool, tree)?)
        })
        .unwrap();
    *record = Some(next.selection_bytes().to_vec());
    outcome
}

fn installed(node: &str, manifest: Value) -> (tempfile::TempDir, TestStore, Lock, Vec<u8>) {
    let repo = root(Some(node), manifest);
    let store = TestStore::open(repo.path());
    let snapshot = Snapshot::capture_with(repo.path(), Ecosystems::default()).unwrap();
    let selection = staged(&snapshot, None, &fixture(&["24.0.0"])).unwrap();
    let lock = selection.selection().clone();
    let mut record = None;
    assert_eq!(
        publish(selection, &snapshot, &store, &mut record),
        Outcome::Replaced
    );
    assert!(snapshot.previous_lock().is_none());
    snapshot.ensure_current().unwrap();
    assert!(snapshot.after_publication(&lock).is_err());
    let record = record.unwrap();
    assert_eq!(record, lock.canonical_bytes().unwrap());
    (repo, store, lock, record)
}

#[test]
fn healthy_repeat_preserves_floating_pins_and_resources_with_zero_metadata_after_advance() {
    let (repo, store, lock, record) = installed("24.x", json!({"packageManager":"pnpm@10.0.0"}));
    let p = repo.path();
    let snapshot = Snapshot::capture_with(p, Ecosystems::default()).unwrap();
    let world = fixture(&["24.1.0", "24.0.0"]);
    let selection = staged(&snapshot, Some(&record), &world).unwrap();
    assert_eq!(selection.selection(), &lock);
    assert!(world.paths().is_empty());
    assert_eq!(
        store
            .reconcile(selection.desired(), |_, _| panic!(
                "healthy trees are reused"
            ))
            .unwrap(),
        Outcome::Unchanged
    );
    // A forced setup re-verifies the healthy trees without restaging them.
    assert_eq!(
        store
            .force_reconcile(selection.desired(), |_, _| panic!("healthy trees verify"))
            .unwrap(),
        Outcome::Unchanged
    );
    assert_installed(&store, p);
}

#[test]
fn targeted_manager_node_drift_and_removal_preserve_unaffected_selection_bytes() {
    let (repo, store, old, record) = installed("24.x", json!({"packageManager":"pnpm@10.0.0"}));
    let mut record = Some(record);
    let p = repo.path();
    write_manifest(
        p,
        json!({"packageManager":"pnpm@10.0.0","devEngines":{"packageManager":{"name":"pnpm","version":"10.0.0"}}}),
    );
    let snapshot = Snapshot::capture_with(p, Ecosystems::default()).unwrap();
    let world = World::new(registry_routes(&pnpm_bytes()), |_| {});
    let next = staged(&snapshot, record.as_deref(), &world).unwrap();
    assert_eq!(next.selection().tools()["node"], old.tools()["node"]);
    assert_eq!(world.paths(), PNPM_PATHS);
    let manager = next.selection().tools()["pnpm"].clone();
    publish(next, &snapshot, &store, &mut record);
    fs::write(p.join(".nvmrc"), "^24.0.0").unwrap();
    let snapshot = Snapshot::capture_with(p, Ecosystems::default()).unwrap();
    let world = World::new(node_routes(&["24.1.0", "24.0.0"]), |_| {});
    let next = staged(&snapshot, record.as_deref(), &world).unwrap();
    assert_eq!(next.selection().tools()["pnpm"], manager);
    assert_eq!(next.selection().tools()["node"].version, "24.1.0");
    assert_eq!(
        world.paths(),
        ["/dist/index.json", "/dist/v24.1.0/SHASUMS256.txt"]
    );
    let node = next.selection().tools()["node"].clone();
    assert_eq!(
        publish(next, &snapshot, &store, &mut record),
        Outcome::Replaced
    );
    write_manifest(p, json!({}));
    let snapshot = Snapshot::capture_with(p, Ecosystems::default()).unwrap();
    let world = World::new(vec![], |_| {});
    let next = staged(&snapshot, record.as_deref(), &world).unwrap();
    assert_eq!(
        next.selection().tools(),
        &BTreeMap::from([("node".into(), node)])
    );
    store
        .reconcile(next.desired(), |_, _| panic!("reuse Node resources"))
        .unwrap();
    let current = store.current().unwrap().unwrap();
    assert!(current.bin.join("npm").exists());
    assert!(!current.bin.join("pnpm").exists());
    assert!(world.paths().is_empty());
}

#[test]
fn missing_trees_repair_the_exact_recorded_selection_not_refreshed_floating_requests() {
    for damage in 0..3 {
        let (repo, store, lock, record) =
            installed("lts/*", json!({"packageManager":"pnpm@10.0.0"}));
        let p = repo.path();
        let current = store.current().unwrap().unwrap();
        let tree = store.tree_of(&current.tools[0]);
        match damage {
            0 => fs::remove_dir_all(current.bin.parent().unwrap()).unwrap(),
            1 => fs::remove_dir_all(tree.parent().unwrap()).unwrap(),
            _ => {
                for tree in fs::read_dir(store.store().root().join("trees")).unwrap() {
                    fs::remove_dir_all(tree.unwrap().path()).unwrap();
                }
            }
        }
        let snapshot = Snapshot::capture_with(p, Ecosystems::default()).unwrap();
        let world = fixture(&["24.1.0", "24.0.0"]);
        let next = staged(&snapshot, Some(&record), &world).unwrap();
        assert_eq!(next.selection(), &lock);
        assert!(world.paths().is_empty());
        let mut record = Some(record);
        publish(next, &snapshot, &store, &mut record);
        assert!(store.is_current(&current.tools).unwrap());
        assert_installed(&store, p);
        // A corrupt record never silently becomes a fresh resolution.
        assert!(staged(&snapshot, Some(b"corrupt"), &world).is_err());
        assert!(world.paths().is_empty());
    }
}

#[test]
fn staged_selections_recheck_their_sources_before_publication() {
    for race in 0..3 {
        let (repo, _store, lock, record) =
            installed("24.x", json!({"packageManager":"pnpm@10.0.0"}));
        let p = repo.path();
        let snapshot = Snapshot::capture_with(p, Ecosystems::default()).unwrap();
        let selection = staged(&snapshot, Some(&record), &World::new(vec![], |_| {})).unwrap();
        selection.check(&snapshot).unwrap();
        match race {
            0 => fs::write(p.join("package.json"), b"{} ").unwrap(),
            1 => fs::write(p.join("turbo.lock"), lock.canonical_bytes().unwrap()).unwrap(),
            _ => fs::write(p.join("turbo.json"), b"{}").unwrap(),
        }
        assert!(selection.check(&snapshot).is_err(), "race={race}");
    }
}

#[test]
fn foreign_roots_real_locks_and_unsupported_integrity_fail_closed() {
    let (repo, _store, lock, record) = installed("24.x", json!({}));
    let other = root(Some("24.x"), json!({}));
    let snapshot = Snapshot::capture_with(repo.path(), Ecosystems::default()).unwrap();
    let selection = staged(&snapshot, Some(&record), &World::new(vec![], |_| {})).unwrap();
    let foreign = Snapshot::capture_with(other.path(), Ecosystems::default()).unwrap();
    assert!(selection.check(&foreign).is_err());
    save(repo.path(), &lock);
    let actual = Snapshot::capture_with(repo.path(), Ecosystems::default()).unwrap();
    assert!(staged(&actual, Some(&record), &World::new(vec![], |_| {})).is_err());
    fs::remove_file(repo.path().join("turbo.lock")).unwrap();
    write_manifest(
        repo.path(),
        json!({"packageManager":format!("pnpm@10.0.0+sha512.{:x}", Sha512::digest(pnpm_bytes()))}),
    );
    let snapshot = Snapshot::capture_with(repo.path(), Ecosystems::default()).unwrap();
    let world = fixture(&["24.0.0"]);
    assert!(staged(&snapshot, Some(&record), &world).is_err());
    assert!(world.paths().is_empty()); // Unsupported record constraints preflight before traffic.
}
