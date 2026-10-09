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
        executable(&tree.join("bin/pnpm.cjs"));
        executable(&tree.join("bin/pnpx.cjs"));
    }
    fs::write(tree.join("resource"), b"adjacent resource")?;
    Ok(())
}
fn manifest(root: &Path) -> Vec<u8> {
    fs::read(root.join(".turbo/tools/manifest.json")).unwrap()
}
fn seed_ephemeral(root: &Path, store: &mut Store) -> Lock {
    let snapshot = Snapshot::capture(root).unwrap();
    let selection = staged(&snapshot, store, &fixture(&["24.0.0"])).unwrap();
    let lock = selection.selection().clone();
    assert_eq!(
        selection
            .publish(&snapshot, store, false, populate, || Ok(()))
            .unwrap(),
        Outcome::Replaced
    );
    assert!(snapshot.previous_lock().is_none());
    snapshot.ensure_current().unwrap();
    assert!(snapshot.after_publication(&lock).is_err());
    lock
}

#[test]
fn healthy_repeat_preserves_floating_pins_record_resources_and_zero_metadata_after_advance() {
    let repo = root(Some("24.x"), json!({"packageManager":"pnpm@10.0.0"}));
    let p = repo.path();
    let mut store = Store::open(p).unwrap();
    let lock = seed_ephemeral(p, &mut store);
    let before = manifest(p);
    for force in [false, true] {
        let snapshot = Snapshot::capture(p).unwrap();
        let world = fixture(&["24.1.0", "24.0.0"]);
        let selection = staged(&snapshot, &store, &world).unwrap();
        assert_eq!(selection.selection(), &lock);
        assert!(world.paths().is_empty());
        let mut calls = Vec::new();
        let result = selection
            .publish(
                &snapshot,
                &mut store,
                force,
                |tool, tree| {
                    calls.push(tool.id.clone());
                    populate(tool, tree)
                },
                || Ok(()),
            )
            .unwrap();
        assert_eq!(
            result,
            if force {
                Outcome::Replaced
            } else {
                Outcome::Unchanged
            }
        );
        assert_eq!(calls, if force { vec!["node", "pnpm"] } else { vec![] });
        let current = Store::inspect(p).unwrap().unwrap();
        assert_eq!(
            current.record.as_ref().unwrap().bytes(),
            lock.canonical_bytes().unwrap()
        );
        for tool in &current.tools {
            assert_eq!(
                fs::read(current.tool_tree(tool).unwrap().join("resource")).unwrap(),
                b"adjacent resource"
            );
            for name in tool.executables.keys() {
                assert!(current.bin.join(name).is_symlink());
            }
        }
        assert!(!p.join("turbo.lock").exists());
        assert_eq!(manifest(p) == before, !force);
    }
}

#[test]
fn targeted_manager_node_drift_and_removal_preserve_unaffected_selection_bytes() {
    let repo = root(Some("24.x"), json!({"packageManager":"pnpm@10.0.0"}));
    let p = repo.path();
    let mut store = Store::open(p).unwrap();
    let old = seed_ephemeral(p, &mut store);
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
    next.publish(&snapshot, &mut store, false, populate, || Ok(()))
        .unwrap();
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
    next.publish(&snapshot, &mut store, false, populate, || Ok(()))
        .unwrap();
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
    for damage in 0..5 {
        let repo = root(Some("lts/*"), json!({"packageManager":"pnpm@10.0.0"}));
        let p = repo.path();
        let mut store = Store::open(p).unwrap();
        let lock = seed_ephemeral(p, &mut store);
        let current = store.current().unwrap().unwrap();
        let tree = current.tool_tree(&current.tools[0]).unwrap();
        match damage {
            0 => fs::remove_dir_all(current.bin.parent().unwrap()).unwrap(),
            1 => fs::remove_dir_all(&tree).unwrap(),
            2 => fs::write(tree.join("resource"), b"damaged").unwrap(),
            3 => fs::remove_file(current.bin.join("node")).unwrap(),
            _ => fs::remove_file(current.bin.parent().unwrap().join("record.json")).unwrap(),
        }
        assert!(store.generation().is_err());
        assert!(!store.is_current(&current.tools).unwrap());
        let snapshot = Snapshot::capture(p).unwrap();
        let world = fixture(&["24.1.0", "24.0.0"]);
        let next = staged(&snapshot, &store, &world).unwrap();
        assert_eq!(next.selection(), &lock);
        next.check(&snapshot, &store).unwrap();
        assert_eq!(
            next.publish(&snapshot, &mut store, false, populate, || Ok(()))
                .unwrap(),
            Outcome::Replaced
        );
        assert!(store.is_current(&current.tools).unwrap());
        assert_eq!(
            store
                .current()
                .unwrap()
                .unwrap()
                .record
                .as_ref()
                .unwrap()
                .bytes(),
            lock.canonical_bytes().unwrap()
        );
        assert!(world.paths().is_empty());
        assert!(!p.join("turbo.lock").exists());
        if damage < 2 {
            let selected = store.current().unwrap().unwrap();
            fs::remove_dir_all(selected.bin.parent().unwrap()).unwrap();
            let hash = format!("{:x}", Sha256::digest(lock.canonical_bytes().unwrap()));
            let record = p.join(format!(".turbo/tools/record-{hash}.json"));
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
    for changed in [false, true] {
        for race in 0..6 {
            let repo = root(Some("24.x"), json!({"packageManager":"pnpm@10.0.0"}));
            let p = repo.path();
            let mut store = Store::open(p).unwrap();
            let lock = seed_ephemeral(p, &mut store);
            if changed {
                write_manifest(p, json!({}));
            }
            let snapshot = Snapshot::capture(p).unwrap();
            let selection = staged(&snapshot, &store, &World::new(vec![], |_| {})).unwrap();
            let before = manifest(p);
            let current = store.current().unwrap().unwrap();
            let tree = current.tool_tree(&current.tools[0]).unwrap();
            let mut calls = 0;
            let result = selection.publish(&snapshot, &mut store, false, populate, || {
                calls += 1;
                if calls == 3 {
                    match race {
                        0 => fs::write(p.join("package.json"), b"{} ").unwrap(),
                        1 => fs::write(p.join("turbo.lock"), lock.canonical_bytes().unwrap())
                            .unwrap(),
                        2 => fs::write(p.join("turbo.json"), b"{}").unwrap(),
                        3 => fs::write(tree.join("resource"), b"wait drift").unwrap(),
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
            assert_eq!(
                manifest(p),
                if race == 4 {
                    [before.as_slice(), b" "].concat()
                } else {
                    before
                }
            );
        }
    }
}

#[test]
fn foreign_live_store_and_real_lock_and_unsupported_integrity_fail_closed() {
    let repo = root(Some("24.x"), json!({}));
    let other = root(Some("24.x"), json!({}));
    let mut store = Store::open(repo.path()).unwrap();
    let lock = seed_ephemeral(repo.path(), &mut store);
    let snapshot = Snapshot::capture(repo.path()).unwrap();
    let selection = staged(&snapshot, &store, &World::new(vec![], |_| {})).unwrap();
    let foreign = Store::open(other.path()).unwrap();
    assert!(selection.check(&snapshot, &foreign).is_err());
    drop(store);
    let mut reopened = Store::open(repo.path()).unwrap();
    assert!(
        selection
            .publish(&snapshot, &mut reopened, false, |_, _| panic!(), || Ok(()))
            .is_err()
    );
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
    assert!(
        ephemeral_reconcile::stage(
            &snapshot,
            &reopened,
            Platform::MacosArm64,
            false,
            |request| {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                let world = fixture(&["24.0.0"]);
                runtime
                    .block_on(resolve(request, &world.node, &world.registry))
                    .map_err(|e| reconcile::Error::Resolution(e.to_string()))
            },
            || Ok(())
        )
        .is_err()
    );
    assert_eq!(manifest(repo.path()), before);
}
