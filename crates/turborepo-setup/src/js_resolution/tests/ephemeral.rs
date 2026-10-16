use turborepo_tool_install::{Outcome, Store, Tool as InstalledTool};
#[path = "ephemeral_races.rs"]
mod races;

use super::*;
use crate::ephemeral_reconcile::{self, Staged};

fn staged(
    snapshot: &Snapshot,
    store: &Store,
    world: &World,
) -> Result<Staged, ephemeral_reconcile::Error> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    ephemeral_reconcile::stage(
        snapshot,
        store,
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
    for path in tool.executables.values() {
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
fn manifest(root: &Path) -> Vec<u8> {
    fs::read(root.join(".turbo/tools/manifest.json")).unwrap()
}
// Legacy fixtures remove only this copy, keeping the bound generation record.
fn recovery(root: &Path, lock: &Lock) -> std::path::PathBuf {
    let hash = format!("{:x}", Sha256::digest(lock.canonical_bytes().unwrap()));
    root.join(format!(".turbo/tools/record-{hash}.json"))
}
fn assert_installed(root: &Path, lock: &Lock) -> turborepo_tool_install::Current {
    let current = Store::inspect(root).unwrap().unwrap();
    let record = current.record.as_ref().unwrap();
    assert_eq!(record.bytes(), lock.canonical_bytes().unwrap());
    for tool in &current.tools {
        assert_eq!(
            fs::read(current.tool_tree(tool).unwrap().join("resource")).unwrap(),
            b"adjacent resource"
        );
        for name in tool.executables.keys() {
            assert!(current.bin.join(name).is_symlink());
        }
    }
    assert!(!root.join("turbo.lock").exists());
    current
}
fn publish(next: Staged, snapshot: &Snapshot, store: &mut Store) -> Outcome {
    next.publish(snapshot, store, false, populate, || Ok(()))
        .unwrap()
}
fn seed_ephemeral(root: &Path, store: &mut Store) -> Lock {
    let snapshot = Snapshot::capture(root).unwrap();
    let selection = staged(&snapshot, store, &fixture(&["24.0.0"])).unwrap();
    let lock = selection.selection().clone();
    assert_eq!(publish(selection, &snapshot, store), Outcome::Replaced);
    assert!(snapshot.previous_lock().is_none());
    snapshot.ensure_current().unwrap();
    assert!(snapshot.after_publication(&lock).is_err());
    lock
}

fn installed(node: &str, manifest: Value) -> (tempfile::TempDir, Store, Lock) {
    let repo = root(Some(node), manifest);
    let mut store = Store::open(repo.path()).unwrap();
    let lock = seed_ephemeral(repo.path(), &mut store);
    (repo, store, lock)
}

#[test]
fn healthy_repeat_preserves_floating_pins_record_resources_and_zero_metadata_after_advance() {
    for legacy in [false, true] {
        let (repo, mut store, lock) = installed("24.x", json!({"packageManager":"pnpm@10.0.0"}));
        let p = repo.path();
        if legacy {
            fs::remove_file(recovery(p, &lock)).unwrap();
        }
        let before = manifest(p);
        for (force, expected, expected_calls) in [
            (false, Outcome::Unchanged, vec![]),
            (true, Outcome::Replaced, vec!["node", "pnpm"]),
        ] {
            let snapshot = Snapshot::capture(p).unwrap();
            let world = fixture(&["24.1.0", "24.0.0"]);
            let selection = staged(&snapshot, &store, &world).unwrap();
            assert_eq!(selection.selection(), &lock);
            assert!(world.paths().is_empty());
            let mut calls = Vec::new();
            let stage_tool = |tool: &InstalledTool, tree: &Path| {
                calls.push(tool.id.clone());
                populate(tool, tree)
            };
            let result = selection
                .publish(&snapshot, &mut store, force, stage_tool, || Ok(()))
                .unwrap();
            assert_eq!(result, expected);
            assert_eq!(calls, expected_calls);
            assert_installed(p, &lock);
            assert_eq!(
                fs::read(recovery(p, &lock)).unwrap(),
                lock.canonical_bytes().unwrap()
            );
            assert_eq!(manifest(p) == before, !force);
        }
    }
}

#[test]
fn targeted_manager_node_drift_and_removal_preserve_unaffected_selection_bytes() {
    let (repo, mut store, old) = installed("24.x", json!({"packageManager":"pnpm@10.0.0"}));
    let p = repo.path();
    write_manifest(
        p,
        json!({"packageManager":"pnpm@10.0.0","devEngines":{"packageManager":{"name":"pnpm","version":"10.0.0"}}}),
    );
    let snapshot = Snapshot::capture(p).unwrap();
    let world = World::new(registry_routes(&pnpm_bytes()), |_| {});
    let next = staged(&snapshot, &store, &world).unwrap();
    assert_eq!(next.selection().tools()["node"], old.tools()["node"]);
    assert_eq!(world.paths(), PNPM_PATHS);
    let manager = next.selection().tools()["pnpm"].clone();
    assert_eq!(publish(next, &snapshot, &mut store), Outcome::Replaced);
    fs::write(p.join(".nvmrc"), "^24.0.0").unwrap();
    let snapshot = Snapshot::capture(p).unwrap();
    let world = World::new(node_routes(&["24.1.0", "24.0.0"]), |_| {});
    let next = staged(&snapshot, &store, &world).unwrap();
    assert_eq!(next.selection().tools()["pnpm"], manager);
    assert_eq!(next.selection().tools()["node"].version, "24.1.0");
    assert_eq!(
        world.paths(),
        ["/dist/index.json", "/dist/v24.1.0/SHASUMS256.txt"]
    );
    let node = next.selection().tools()["node"].clone();
    assert_eq!(publish(next, &snapshot, &mut store), Outcome::Replaced);
    write_manifest(p, json!({}));
    let snapshot = Snapshot::capture(p).unwrap();
    let world = World::new(vec![], |_| {});
    let next = staged(&snapshot, &store, &world).unwrap();
    assert_eq!(
        next.selection().tools(),
        &BTreeMap::from([("node".into(), node)])
    );
    next.publish(
        &snapshot,
        &mut store,
        false,
        |_, _| panic!("reuse Node resources"),
        || Ok(()),
    )
    .unwrap();
    let current = store.current().unwrap().unwrap();
    assert!(current.bin.join("npm").exists());
    assert!(!current.bin.join("pnpm").exists());
    assert!(world.paths().is_empty());
}

#[test]
fn missing_and_damaged_generations_repair_exact_record_not_refresh_floating_requests() {
    for damage in 0..6 {
        let (repo, mut store, lock) = installed("lts/*", json!({"packageManager":"pnpm@10.0.0"}));
        let p = repo.path();
        let current = store.current().unwrap().unwrap();
        let tree = current.tool_tree(&current.tools[0]).unwrap();
        match damage {
            0 => fs::remove_dir_all(current.bin.parent().unwrap()).unwrap(),
            1 => fs::remove_dir_all(&tree).unwrap(),
            2 => fs::write(tree.join("resource"), b"damaged").unwrap(),
            3 => fs::remove_file(current.bin.join("node")).unwrap(),
            5 => {
                fs::remove_file(recovery(p, &lock)).unwrap();
                fs::remove_file(tree.join("resource")).unwrap();
            }
            _ => fs::remove_file(current.bin.parent().unwrap().join("record.json")).unwrap(),
        }
        assert!(store.generation().is_err());
        assert!(!store.is_current(&current.tools).unwrap());
        let snapshot = Snapshot::capture(p).unwrap();
        let world = fixture(&["24.1.0", "24.0.0"]);
        let next = staged(&snapshot, &store, &world).unwrap();
        assert_eq!(next.selection(), &lock);
        next.check(&snapshot, &store).unwrap();
        assert_eq!(publish(next, &snapshot, &mut store), Outcome::Replaced);
        assert!(store.is_current(&current.tools).unwrap());
        let selected = assert_installed(p, &lock);
        assert!(world.paths().is_empty());
        if damage < 2 {
            fs::remove_dir_all(selected.bin.parent().unwrap()).unwrap();
            let record = recovery(p, &lock);
            if damage == 0 {
                fs::remove_file(record).unwrap();
            } else {
                fs::write(record, b"corrupt").unwrap();
            }
            assert!(staged(&snapshot, &store, &world).is_err());
            assert!(world.paths().is_empty());
        }
    }
}

#[test]
fn final_noop_and_changed_publication_recheck_sources_lock_generation_and_root() {
    for (changed, force) in [(false, false), (true, false), (false, true)] {
        for race in 0..if changed { 6 } else { 7 } {
            let (repo, mut store, lock) =
                installed("24.x", json!({"packageManager":"pnpm@10.0.0"}));
            let p = repo.path();
            if changed {
                write_manifest(p, json!({}));
            }
            let snapshot = Snapshot::capture(p).unwrap();
            let selection = staged(&snapshot, &store, &World::new(vec![], |_| {})).unwrap();
            let before = manifest(p);
            let current = store.current().unwrap().unwrap();
            let tree = current.tool_tree(&current.tools[0]).unwrap();
            let mut calls = 0;
            let result = selection.publish(&snapshot, &mut store, force, populate, || {
                calls += 1;
                if calls == 3 {
                    match race {
                        0 => fs::write(p.join("package.json"), b"{} ").unwrap(),
                        1 => fs::write(p.join("turbo.lock"), lock.canonical_bytes().unwrap())
                            .unwrap(),
                        2 => fs::write(p.join("turbo.json"), b"{}").unwrap(),
                        3 => fs::write(tree.join("resource"), b"wait drift").unwrap(),
                        6 => fs::write(recovery(p, &lock), b"corrupt").unwrap(),
                        5 => {
                            fs::rename(p.join(".turbo/tools"), p.join(".turbo/old-tools")).unwrap();
                            fs::create_dir(p.join(".turbo/tools")).unwrap();
                            fs::write(p.join(".turbo/tools/manifest.json"), &before).unwrap();
                        }
                        _ => fs::write(
                            p.join(".turbo/tools/manifest.json"),
                            [before.as_slice(), b" "].concat(),
                        )
                        .unwrap(),
                    }
                }
                Ok(())
            });
            assert!(result.is_err(), "changed={changed}, race={race}");
            let expected_manifest = if race == 4 {
                [before.as_slice(), b" "].concat()
            } else {
                before
            };
            assert_eq!(manifest(p), expected_manifest);
            if race == 6 {
                assert_installed(p, &lock);
                assert_eq!(fs::read(recovery(p, &lock)).unwrap(), b"corrupt");
            }
        }
    }
}

#[test]
fn foreign_live_store_and_real_lock_and_unsupported_integrity_fail_closed() {
    let (repo, store, lock) = installed("24.x", json!({}));
    let other = root(Some("24.x"), json!({}));
    let snapshot = Snapshot::capture(repo.path()).unwrap();
    let selection = staged(&snapshot, &store, &World::new(vec![], |_| {})).unwrap();
    let foreign = Store::open(other.path()).unwrap();
    assert!(selection.check(&snapshot, &foreign).is_err());
    drop(store);
    let mut reopened = Store::open(repo.path()).unwrap();
    let result = selection.publish(&snapshot, &mut reopened, false, |_, _| panic!(), || Ok(()));
    assert!(result.is_err());
    save(repo.path(), &lock);
    let actual = Snapshot::capture(repo.path()).unwrap();
    assert!(staged(&actual, &reopened, &World::new(vec![], |_| {})).is_err());
    fs::remove_file(repo.path().join("turbo.lock")).unwrap();
    write_manifest(
        repo.path(),
        json!({"packageManager":format!("pnpm@10.0.0+sha512.{:x}", Sha512::digest(pnpm_bytes()))}),
    );
    let snapshot = Snapshot::capture(repo.path()).unwrap();
    let before = manifest(repo.path());
    let world = fixture(&["24.0.0"]);
    assert!(staged(&snapshot, &reopened, &world).is_err());
    assert!(world.paths().is_empty()); // Unsupported record constraints preflight before traffic.
    assert_eq!(manifest(repo.path()), before);
}
