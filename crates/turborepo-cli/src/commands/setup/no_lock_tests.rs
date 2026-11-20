//! Real parser/root/policy/HTTP acceptance; owned fixtures, no executed tools.
use turborepo_setup::lock::Lock;

use super::*;

const NO_LOCK: &[&str] = &["--no-lock", "--tools-only"];
fn current(f: &Fixture) -> Current {
    Installed::open(&f.owned).current().unwrap().unwrap()
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
fn no_lock_ci_repeat_preserves_24_0_pin_environment_and_zero_advance_metadata() {
    // Plain --no-lock cannot install locked dependencies (TURBO-6229).
    let f = fresh();
    fs::write(f.owned.root().join(".nvmrc"), "24.x").unwrap();
    let before = tracked(&f);
    let u = LoopbackServer::new(routes(&f, "24.0.0", "10.0.0"), |_| {}).unwrap();
    assert_eq!(f.run(&u, NO_LOCK).unwrap(), 0); // Also run with actual CI=1.
    assert_eq!(u.hits(), 7);
    let pinned = selection(&f);
    let environment = current(&f).bin;
    let manifest = f.manifest();
    let advanced = LoopbackServer::new(routes(&f, "24.1.0", "10.0.0"), |_| {}).unwrap();
    assert_eq!(f.run(&advanced, NO_LOCK).unwrap(), 0);
    assert_eq!(advanced.hits(), 0);
    assert_eq!(selection(&f), pinned);
    assert_eq!(selection(&f).tools()["node"].version, "24.0.0");
    assert_eq!(current(&f).bin, environment);
    assert_eq!(f.manifest(), manifest);
    assert_eq!(tracked(&f), before);
    let current = current(&f);
    assert!(
        current
            .tree("node")
            .join("lib/node_modules/npm/package.json")
            .is_file()
    );
    assert!(current.tree("pnpm").join("dist/resource").is_file());
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
    // pnpm's tree is reused beside any Node: no new package-manager
    // resolution, selection or download.
    assert_eq!(node.hits(), 3);
    assert!(
        !paths
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.starts_with("/pnpm/"))
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
    fs::write(f.owned.root().join(".nvmrc"), "24.x").unwrap();
    let u = LoopbackServer::new(routes(&f, "24.0.0", "10.0.0"), |_| {}).unwrap();
    f.run(&u, LOCAL).unwrap();
    assert!(current(&f).record.is_none());
    let before = tracked(&f);
    let pinned = selected(&f);
    let empty = LoopbackServer::new([], |_| {}).unwrap();
    f.run(&empty, NO_LOCK).unwrap();
    assert_eq!(empty.hits(), 0);
    // The disk lock stays the resolution: no clone-local selection is kept.
    assert_eq!(selected(&f), pinned);
    assert!(current(&f).record.is_none());
    assert_eq!(tracked(&f), before);
    let environment = current(&f).bin;
    f.run(&empty, NO_LOCK).unwrap();
    assert_eq!(current(&f).bin, environment);
    fs::write(f.owned.root().join(".nvmrc"), "26.x").unwrap();
    let before = publication_guards::state(f.owned.root());
    assert!(f.run(&empty, NO_LOCK).is_err());
    assert_eq!(empty.hits(), 0);
    assert_eq!(publication_guards::state(f.owned.root()), before);
    untouched(&f);
}
#[test]
fn no_lock_recordless_inventory_without_disk_lock_rejects_without_refresh() {
    let f = fresh();
    fs::write(f.owned.root().join(".nvmrc"), "24.x").unwrap();
    let seed = LoopbackServer::new(routes(&f, "24.0.0", "10.0.0"), |_| {}).unwrap();
    f.run(&seed, LOCAL).unwrap();
    let before = current(&f);
    assert!(before.record.is_none());
    assert_eq!(before.tools[0].version, "24.0.0");
    fs::remove_file(f.owned.root().join("turbo.lock")).unwrap();
    let manifest = f.manifest();
    let sources = tracked(&f);
    let advanced = LoopbackServer::new(routes(&f, "24.1.0", "10.0.0"), |_| {}).unwrap();
    let result = f.run(&advanced, NO_LOCK);
    assert!(
        result.is_err(),
        "recordless repeat: {result:?}, {} requests",
        advanced.hits()
    );
    assert_eq!(advanced.hits(), 0);
    let after = current(&f);
    assert_eq!(after.tools, before.tools);
    assert_eq!(after.bin, before.bin);
    assert!(after.record.is_none());
    assert_eq!(f.manifest(), manifest);
    assert_eq!(tracked(&f), sources);
    assert!(!f.owned.root().join("turbo.lock").exists());
    untouched(&f);
}
#[test]
fn no_lock_repairs_missing_or_damaged_installation_without_refreshing_pins() {
    for case in 0..4 {
        let f = fresh();
        fs::write(f.owned.root().join(".nvmrc"), "24.x").unwrap();
        let u = LoopbackServer::new(routes(&f, "24.0.0", "10.0.0"), |_| {}).unwrap();
        f.run(&u, NO_LOCK).unwrap();
        let pinned = selection(&f);
        let installed = current(&f);
        let env = installed.bin.parent().unwrap().to_owned();
        match case {
            // The whole Node tree, seal included: only Node is downloaded.
            0 => fs::remove_dir_all(installed.tree("node").parent().unwrap()).unwrap(),
            // Shims are rendered again from sealed trees, without traffic.
            1 => fs::remove_dir_all(&env).unwrap(),
            2 => {
                fs::remove_file(env.join("bin/pnpm")).unwrap();
                std::os::unix::fs::symlink(
                    installed.tree("pnpm").join("dist/resource"),
                    env.join("bin/pnpm"),
                )
                .unwrap();
            }
            _ => fs::remove_file(env.join("bin/pnpm")).unwrap(),
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
        assert_eq!(repair.hits(), if case == 0 { 1 } else { 0 }, "case {case}");
        assert_eq!(
            fs::read(current(&f).tree("pnpm").join("dist/resource")).unwrap(),
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
        json!({"packageManager":"yarn@4.5.0"}),
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
fn no_lock_names_a_missing_node_declaration_or_unsupported_tool_before_any_change() {
    for (package, nvmrc, unsupported) in [
        (json!({"packageManager":"pnpm@10.0.0"}), false, None),
        (json!({}), false, None),
        (json!({"packageManager":"yarn@4.5.0"}), true, Some("yarn")),
    ] {
        let f = fresh();
        if !nvmrc {
            fs::remove_file(f.owned.root().join(".nvmrc")).unwrap();
        }
        fs::write(f.owned.root().join("package.json"), package.to_string()).unwrap();
        let u = LoopbackServer::new([], |_| {}).unwrap();
        let before = publication_guards::state(f.owned.root());
        let error = f.run(&u, NO_LOCK).unwrap_err();
        match unsupported {
            Some(tool) => assert!(matches!(&error, Error::NoLockTool(id) if id == tool)),
            None => assert!(matches!(error, Error::NoLockNode), "{error}"),
        }
        assert_eq!(u.hits(), 0);
        assert_eq!(publication_guards::state(f.owned.root()), before);
        assert!(!f.owned.root().join(".turbo").exists());
        untouched(&f);
    }
}
#[test]
fn no_lock_original_snapshot_rejects_lock_source_and_policy_races_during_download() {
    for existing in [false, true] {
        for name in ["turbo.lock", ".nvmrc", "turbo.json", "pnpm-workspace.yaml"] {
            let f = fresh();
            if existing {
                let seed = LoopbackServer::new(routes(&f, "24.0.0", "10.0.0"), |_| {}).unwrap();
                f.run(&seed, NO_LOCK).unwrap();
                fs::write(
                    f.owned.root().join("turbo.lock"),
                    selection(&f).canonical_bytes().unwrap(),
                )
                .unwrap();
                // Only a missing tree makes setup download again.
                fs::remove_dir_all(current(&f).tree("node").parent().unwrap()).unwrap();
            }
            let before = f.manifest();
            let root = f.owned.root().to_owned();
            let u = LoopbackServer::new(routes(&f, "24.0.0", "10.0.0"), move |p| {
                if p.ends_with(".tar.gz") {
                    fs::write(
                        root.join(name),
                        if name == "turbo.lock" {
                            "{}"
                        } else if name == "turbo.json" {
                            UNSUPPORTED_SETUP
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
fn no_lock_sources_changed_before_recording_preserve_previous_selection() {
    for case in 0..3 {
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
        let envs = f.owned.home().join("envs");
        let fired = std::cell::Cell::new(false);
        // Once the new environment is published, only the record remains.
        let result = dispatch(&f, &u, NO_LOCK, || {
            if fs::read_dir(&envs).unwrap().count() < 2 || fired.replace(true) {
                return;
            }
            match case {
                0 => fs::write(f.owned.root().join("turbo.lock"), "{}").unwrap(),
                1 => fs::write(f.owned.root().join("turbo.json"), UNSUPPORTED_SETUP).unwrap(),
                _ => fs::write(f.owned.root().join("pnpm-workspace.yaml"), "packages: []").unwrap(),
            }
        });
        assert!(fired.get());
        assert!(result.is_err(), "case {case}");
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
fn no_lock_waits_revalidate_before_traffic_after_writer_acquisition() {
    let f = fresh();
    let root = f.owned.root();
    let snapshot = f.sources().unwrap();
    let guard = snapshot.guard().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let empty = LoopbackServer::new([], |_| {}).unwrap();
    std::thread::scope(|scope| {
        let pending = scope.spawn(|| {
            let calls = std::cell::Cell::new(0);
            dispatch(&f, &empty, NO_LOCK, || {
                calls.set(calls.get() + 1);
                if calls.get() == 1 {
                    tx.send(()).unwrap();
                }
            })
        });
        rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        fs::write(root.join("turbo.json"), UNSUPPORTED_SETUP).unwrap();
        drop(guard);
        assert!(pending.join().unwrap().is_err());
    });
    assert_eq!(empty.hits(), 0);
    assert!(f.manifest().is_none());
    untouched(&f);
}
fn trusted(f: &Fixture) -> (PathBuf, Value) {
    let path = fs::read_dir(f.owned.home().join("repos"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("record.json");
    let value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    (path, value)
}
fn trust_store(f: &Fixture) -> crate::commands::setup::trust::TrustStore {
    let store = turborepo_tool_install::UserStore::at(f.owned.home(), f.owned.root()).unwrap();
    crate::commands::setup::trust::TrustStore::new(&store, f.owned.root()).unwrap()
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn locked_plan(f: &Fixture) -> turborepo_setup::activation::ActivationPlan {
    let context = crate::commands::managed::context().unwrap();
    turborepo_setup::activation::ActivationPlan::captured(
        &turborepo_setup::lock::Snapshot::capture(f.owned.root()).unwrap(),
        context,
    )
    .unwrap()
}
#[test]
fn no_lock_records_clone_local_trust_that_a_committed_lock_never_inherits() {
    let f = fresh();
    fs::write(f.owned.root().join(".nvmrc"), "24.x").unwrap();
    let u = LoopbackServer::new(routes(&f, "24.0.0", "10.0.0"), |_| {}).unwrap();
    f.run(&u, NO_LOCK).unwrap();
    let (path, value) = trusted(&f);
    let installed = current(&f);
    assert!(value.get("lock_sha256").is_none());
    assert_eq!(
        value["selection"].as_str().unwrap().as_bytes(),
        installed.record.as_ref().unwrap().bytes()
    );
    let env = installed.bin.parent().unwrap().file_name().unwrap();
    assert_eq!(value["env"], env.to_str().unwrap());
    let trust = trust_store(&f);
    let recorded = trust.recorded().unwrap().unwrap();
    assert_eq!(recorded.activation.bin(), installed.bin);
    assert!(!recorded.stale);
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let empty = LoopbackServer::new([], |_| {}).unwrap();
    f.run(&empty, NO_LOCK).unwrap();
    assert_eq!(empty.hits(), 0);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    // The same selection committed as turbo.lock is another resolution: stale
    // for the hook and never authority for the locked plan.
    let lock = selection(&f).canonical_bytes().unwrap();
    fs::write(f.owned.root().join("turbo.lock"), &lock).unwrap();
    assert!(trust.recorded().unwrap().unwrap().stale);
    assert!(trust.authorize(&locked_plan(&f)).unwrap().is_none());
    // Setup from that lock records its digest instead, for the same environment.
    f.run(&empty, NO_LOCK).unwrap();
    assert_eq!(empty.hits(), 0);
    assert_eq!(current(&f).bin, installed.bin);
    assert_eq!(trusted(&f).1["lock_sha256"], digest(&lock));
    assert!(trust.authorize(&locked_plan(&f)).unwrap().is_some());
    untouched(&f);
}
#[test]
fn no_lock_without_its_record_resolves_again_but_reuses_verified_trees() {
    let f = fresh();
    fs::write(f.owned.root().join(".nvmrc"), "24.x").unwrap();
    let u = LoopbackServer::new(routes(&f, "24.0.0", "10.0.0"), |_| {}).unwrap();
    f.run(&u, NO_LOCK).unwrap();
    let pinned = selection(&f);
    let old = current(&f).bin;
    // The record holds the clone-local pins; without it they are gone.
    fs::remove_file(trusted(&f).0).unwrap();
    let trust = trust_store(&f);
    assert!(trust.recorded().unwrap().is_none());
    let paths = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = paths.clone();
    let resolve = LoopbackServer::new(routes(&f, "24.0.0", "10.0.0"), move |p| {
        log.lock().unwrap().push(p.to_owned());
    })
    .unwrap();
    f.run(&resolve, NO_LOCK).unwrap();
    // Resolution only: Node metadata, and pnpm metadata plus the tarball it
    // hashes for the selection. The Node archive is never fetched again.
    assert_eq!(resolve.hits(), 4);
    assert!(
        !paths.lock().unwrap().iter().any(|p| p.ends_with(".tar.gz")),
        "{:?}",
        paths.lock().unwrap()
    );
    assert_eq!(selection(&f), pinned);
    assert_eq!(current(&f).bin, old);
    assert_eq!(
        trust.recorded().unwrap().unwrap().activation.bin(),
        current(&f).bin
    );
    untouched(&f);
}
