//! Real parser/root/policy/HTTP acceptance; owned fixtures, no executed tools.
use turborepo_setup::lock::Lock;
use turborepo_tool_install::Store;

use super::*;

const NO_LOCK: &[&str] = &["--no-lock", "--tools-only"];
fn current(f: &Fixture) -> turborepo_tool_install::Current {
    Store::inspect(f.owned.root()).unwrap().unwrap()
}
fn selection(f: &Fixture) -> Lock {
    Lock::parse(current(f).record.as_ref().unwrap().bytes()).unwrap()
}
fn tracked(f: &Fixture) -> Vec<(PathBuf, Vec<u8>)> {
    publication_guards::state(f.owned.root())
        .into_iter()
        .filter(|(p, _)| !p.starts_with(f.owned.root().join(".turbo")))
        .collect()
}
#[test]
fn no_lock_ci_repeat_preserves_24_0_pin_generation_and_zero_advance_metadata() {
    let f = fresh();
    fs::write(f.owned.root().join(".nvmrc"), "24.x").unwrap();
    let before = tracked(&f);
    let u = LoopbackServer::new(routes(&f, "24.0.0", "10.0.0"), |_| {}).unwrap();
    assert_eq!(f.run(&u, NO_LOCK).unwrap(), 0); // Also run with actual CI=1.
    assert_eq!(u.hits(), 7);
    let pinned = selection(&f);
    let generation = current(&f).bin;
    let manifest = f.manifest();
    let advanced = LoopbackServer::new(routes(&f, "24.1.0", "10.0.0"), |_| {}).unwrap();
    assert_eq!(f.run(&advanced, NO_LOCK).unwrap(), 0);
    assert_eq!(advanced.hits(), 0);
    assert_eq!(selection(&f), pinned);
    assert_eq!(selection(&f).tools()["node"].version, "24.0.0");
    assert_eq!(current(&f).bin, generation);
    assert_eq!(f.manifest(), manifest);
    assert_eq!(tracked(&f), before);
    assert!(
        current(&f)
            .bin
            .join("../tools/node/lib/node_modules/npm/package.json")
            .is_file()
    );
    assert!(
        current(&f)
            .bin
            .join("../tools/pnpm/dist/resource")
            .is_file()
    );
    untouched(&f);
}
#[test]
fn no_lock_targeted_drift_retains_unaffected_pins_and_removes_manager() {
    let f = fresh();
    fs::write(f.owned.root().join(".nvmrc"), "24.x").unwrap();
    let u = LoopbackServer::new(routes(&f, "24.0.0", "10.0.0"), |_| {}).unwrap();
    f.run(&u, NO_LOCK).unwrap();
    let initial = selection(&f);
    fs::write(
        f.owned.root().join("package.json"),
        r#"{"packageManager":"pnpm@10.1.0"}"#,
    )
    .unwrap();
    let manager = LoopbackServer::new(routes(&f, "24.1.0", "10.1.0"), |p| {
        assert!(!p.starts_with("/dist/"))
    })
    .unwrap();
    f.run(&manager, NO_LOCK).unwrap();
    assert_eq!(manager.hits(), 4);
    assert_eq!(selection(&f).tools()["node"], initial.tools()["node"]);
    let pnpm = selection(&f).tools()["pnpm"].clone();
    fs::write(f.owned.root().join(".nvmrc"), "24.1.x").unwrap();
    let paths = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = paths.clone();
    let node = LoopbackServer::new(routes(&f, "24.1.0", "10.1.0"), move |p| {
        log.lock().unwrap().push(p.to_owned());
    })
    .unwrap();
    f.run(&node, NO_LOCK).unwrap();
    // Node binding changes pnpm's launcher identity: exact locked verification
    // is required, not a new package-manager resolution or selection.
    assert_eq!(node.hits(), 5);
    assert_eq!(
        paths
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.starts_with("/pnpm/"))
            .cloned()
            .collect::<Vec<_>>(),
        ["/pnpm/10.1.0", "/pnpm/-/pnpm-10.1.0.tgz"]
    );
    assert_eq!(selection(&f).tools()["pnpm"], pnpm);
    let prior = selection(&f).tools()["node"].clone();
    fs::write(f.owned.root().join("package.json"), "{}").unwrap();
    let empty = LoopbackServer::new([], |_| {}).unwrap();
    f.run(&empty, NO_LOCK).unwrap();
    assert_eq!(empty.hits(), 0);
    assert_eq!(selection(&f).tools()["node"], prior);
    assert_eq!(current(&f).tools.len(), 1);
    assert!(!current(&f).bin.join("pnpm").exists());
    assert!(!f.owned.root().join("turbo.lock").exists());
    untouched(&f);
}
#[test]
fn no_lock_existing_disk_lock_is_unchanged_and_stale_rejects_before_traffic_or_writer() {
    let f = fresh();
    let u = LoopbackServer::new(routes(&f, "24.0.0", "10.0.0"), |_| {}).unwrap();
    f.run(&u, LOCAL).unwrap();
    let before = tracked(&f);
    let pinned = selected(&f);
    let empty = LoopbackServer::new([], |_| {}).unwrap();
    f.run(&empty, NO_LOCK).unwrap();
    assert_eq!(empty.hits(), 0);
    assert_eq!(selection(&f), pinned);
    assert_eq!(tracked(&f), before);
    let generation = current(&f).bin;
    f.run(&empty, NO_LOCK).unwrap();
    assert_eq!(current(&f).bin, generation);
    fs::write(f.owned.root().join(".nvmrc"), "26.x").unwrap();
    let before = publication_guards::state(f.owned.root());
    assert!(f.run(&empty, NO_LOCK).is_err());
    assert_eq!(empty.hits(), 0);
    assert_eq!(publication_guards::state(f.owned.root()), before);
    untouched(&f);
}
#[test]
fn no_lock_repairs_missing_or_damaged_full_tree_without_refreshing_pins() {
    for case in 0..4 {
        let f = fresh();
        fs::write(f.owned.root().join(".nvmrc"), "24.x").unwrap();
        let u = LoopbackServer::new(routes(&f, "24.0.0", "10.0.0"), |_| {}).unwrap();
        f.run(&u, NO_LOCK).unwrap();
        let pinned = selection(&f);
        let tree = current(&f).bin.parent().unwrap().to_owned();
        match case {
            0 => fs::remove_dir_all(&tree).unwrap(),
            1 => fs::remove_dir_all(tree.join("tools/node")).unwrap(),
            2 => fs::write(tree.join("tools/pnpm/dist/resource"), "damaged").unwrap(),
            _ => fs::remove_file(tree.join("bin/pnpm")).unwrap(),
        }
        let mut repair_routes = routes(&f, "24.0.0", "10.0.0");
        repair_routes.retain(|(p, _)| p != "/dist/index.json");
        repair_routes.extend(
            routes(&f, "24.1.0", "10.0.0")
                .into_iter()
                .filter(|(p, _)| p == "/dist/index.json"),
        );
        let repair = LoopbackServer::new(repair_routes, |p| {
            assert!(!p.ends_with("index.json") && !p.ends_with("SHASUMS256.txt"))
        })
        .unwrap();
        f.run(&repair, NO_LOCK).unwrap();
        assert_eq!(selection(&f), pinned);
        assert_eq!(repair.hits(), 3);
        assert_eq!(
            fs::read(current(&f).bin.join("../tools/pnpm/dist/resource")).unwrap(),
            b"complete pnpm resource"
        );
        assert!(current(&f).bin.join("pnpm").is_file());
        untouched(&f);
    }
}
#[test]
fn no_lock_unsupported_native_shape_fails_before_writer_or_nonlock_changes() {
    for package in [
        json!({"packageManager":format!("pnpm@10.0.0+sha512.{}", "0".repeat(128))}),
        json!({"packageManager":"pnpm@10.0.0","devEngines":{"runtime":[{"name":"deno","version":"2.0.0"},{"name":"node","version":"24.x"}]}}),
        json!({"packageManager":"npm@11.6.1"}),
    ] {
        let f = fresh();
        fs::write(f.owned.root().join("package.json"), package.to_string()).unwrap();
        let u = LoopbackServer::new([], |_| {}).unwrap();
        let before = publication_guards::state(f.owned.root());
        assert!(f.run(&u, NO_LOCK).is_err());
        assert_eq!(u.hits(), 0);
        assert_eq!(publication_guards::state(f.owned.root()), before);
        assert!(!f.owned.root().join(".turbo").exists());
        untouched(&f);
    }
}
#[test]
fn no_lock_original_snapshot_rejects_lock_source_and_policy_races_during_download() {
    for existing in [false, true] {
        for name in [
            "turbo.lock",
            ".nvmrc",
            "apps/web/.npmrc",
            "pnpm-workspace.yaml",
            ".gitignore",
        ] {
            let f = fresh();
            if existing {
                let seed = LoopbackServer::new(routes(&f, "24.0.0", "10.0.0"), |_| {}).unwrap();
                f.run(&seed, NO_LOCK).unwrap();
                fs::write(
                    f.owned.root().join("turbo.lock"),
                    selection(&f).canonical_bytes().unwrap(),
                )
                .unwrap();
                fs::remove_dir_all(current(&f).bin.parent().unwrap()).unwrap();
            }
            let before = f.manifest();
            let root = f.owned.root().to_owned();
            let u = LoopbackServer::new(routes(&f, "24.0.0", "10.0.0"), move |p| {
                if p.ends_with(".tar.gz") {
                    fs::write(
                        root.join(name),
                        if name == "turbo.lock" {
                            "{}"
                        } else {
                            "changed"
                        },
                    )
                    .unwrap();
                }
            })
            .unwrap();
            assert!(f.run(&u, NO_LOCK).is_err(), "{existing} {name}");
            assert_eq!(f.manifest(), before);
            assert_eq!(u.hits(), if existing { 1 } else { 5 });
            if name != "turbo.lock" && !existing {
                assert!(!f.owned.root().join("turbo.lock").exists());
            }
            untouched(&f);
        }
    }
}
#[test]
fn no_lock_atomic_final_record_inventory_failures_preserve_previous_selection() {
    for case in 0..5 {
        let f = fresh();
        let seed = LoopbackServer::new(routes(&f, "24.0.0", "10.0.0"), |_| {}).unwrap();
        f.run(&seed, NO_LOCK).unwrap();
        let old = current(&f);
        let before = f.manifest();
        fs::write(
            f.owned.root().join("package.json"),
            r#"{"packageManager":"pnpm@10.1.0"}"#,
        )
        .unwrap();
        let u = LoopbackServer::new(routes(&f, "24.1.0", "10.1.0"), |_| {}).unwrap();
        let fired = std::cell::Cell::new(false);
        let result = dispatch(&f, &u, NO_LOCK, || {
            let Some(staged) = fs::read_dir(f.owned.root().join(".turbo/tools"))
                .ok()
                .and_then(|entries| {
                    entries
                        .filter_map(Result::ok)
                        .map(|e| e.path())
                        .find(|p| p.join("record.json").is_file() && p.join("bin") != old.bin)
                })
            else {
                return;
            };
            if fired.replace(true) {
                return;
            }
            match case {
                0 => fs::write(staged.join("record.json"), "corrupt").unwrap(),
                1 => fs::write(staged.join("tools/pnpm/dist/resource"), "corrupt").unwrap(),
                2 => fs::write(f.owned.root().join("turbo.lock"), "{}").unwrap(),
                3 => fs::write(f.owned.root().join("apps/web/.npmrc"), "").unwrap(),
                _ => fs::write(f.owned.root().join("pnpm-workspace.yaml"), "packages: []").unwrap(),
            }
        });
        assert!(fired.get());
        assert!(result.is_err());
        assert_eq!(f.manifest(), before);
        assert_eq!(current(&f).bin, old.bin);
        assert_eq!(
            current(&f).record.unwrap().bytes(),
            old.record.unwrap().bytes()
        );
        untouched(&f);
    }
}
#[test]
fn no_lock_waits_revalidate_before_traffic_after_writer_and_store_acquisition() {
    for writer in [true, false] {
        let f = fresh();
        let root = f.owned.root();
        let snapshot = turborepo_setup::lock::Snapshot::capture(root).unwrap();
        let guard = writer.then(|| snapshot.guard().unwrap());
        let store = (!writer).then(|| Store::open(root).unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        let empty = LoopbackServer::new([], |_| {}).unwrap();
        std::thread::scope(|scope| {
            let pending = scope.spawn(|| {
                let calls = std::cell::Cell::new(0);
                dispatch(&f, &empty, NO_LOCK, || {
                    let n = calls.get() + 1;
                    calls.set(n);
                    if n == if writer { 1 } else { 3 } {
                        tx.send(()).unwrap();
                    }
                })
            });
            rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
            fs::write(root.join("apps/web/.npmrc"), "").unwrap();
            drop(guard);
            drop(store);
            assert!(pending.join().unwrap().is_err());
        });
        assert_eq!(empty.hits(), 0);
        assert!(f.manifest().is_none());
        untouched(&f);
    }
}
