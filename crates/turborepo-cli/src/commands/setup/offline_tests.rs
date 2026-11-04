//! Offline repeat is strict external provenance, not a repo-readiness shortcut.
//! Anything it cannot repeat installs from intact trees in the tool store.
use std::os::unix::fs::{PermissionsExt, symlink};

use turborepo_setup::activation::ActivationPlan;

use super::{checks::snapshot, *};

const OFFLINE: &[&str] = &["--tools-only", "--offline"];
const OFFLINE_FORCE: &[&str] = &["--tools-only", "--offline", "--force"];

fn installed() -> Fixture {
    let f = Fixture::new();
    f.run(&Upstream::new(&f, false, None), FROZEN).unwrap();
    f
}
fn record(f: &Fixture) -> PathBuf {
    fs::read_dir(f.owned.home().join("repos"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("record.json")
}
fn attempt(f: &Fixture, u: &LoopbackServer, flags: &[&str]) -> Result<i32, Error> {
    let before = snapshot(f.owned.root().parent().unwrap());
    let environment: std::collections::BTreeMap<_, _> = std::env::vars_os().collect();
    let hits = u.hits();
    let result = f.run(u, flags);
    assert_eq!(snapshot(f.owned.root().parent().unwrap()), before);
    assert_eq!(u.hits(), hits);
    assert_eq!(
        std::env::vars_os().collect::<std::collections::BTreeMap<_, _>>(),
        environment
    );
    f.no_execution();
    result
}
/// Offline setup, forced or not, fails without effects naming exactly the
/// `missing` tools' locked artifacts.
fn missing(f: &Fixture, u: &LoopbackServer, missing: &[&str]) {
    let forced = attempt(f, u, OFFLINE_FORCE).unwrap_err();
    let error = attempt(f, u, OFFLINE).unwrap_err();
    assert_eq!(forced.to_string(), error.to_string());
    assert!(matches!(error, Error::OfflineMissing(_)), "{error}");
    let message = error.to_string();
    let lines = [
        (
            "node",
            format!(
                "node 24.0.0 [distribution]: https://nodejs.org{} (SHA-256 {:x})",
                f.node_path,
                Sha256::digest(&f.node)
            ),
        ),
        (
            "pnpm",
            format!(
                "pnpm 10.0.0 [package]: https://registry.npmjs.org/pnpm/-/pnpm-10.0.0.tgz \
                 (SHA-256 {:x})",
                Sha256::digest(&f.pnpm)
            ),
        ),
    ];
    for (id, line) in lines {
        assert_eq!(
            message.contains(&line),
            missing.contains(&id),
            "{id}: {message}"
        );
    }
    assert!(message.contains("without --offline"), "{message}");
}

/// Offline setup succeeds once by installing from the store, then repeats
/// without effects.
fn installs_offline(f: &Fixture, u: &LoopbackServer) {
    assert_eq!(f.run(u, OFFLINE).unwrap(), 0);
    assert_eq!(
        Installed::open(&f.owned)
            .current()
            .unwrap()
            .unwrap()
            .tools
            .len(),
        2
    );
    assert_eq!(attempt(f, u, OFFLINE).unwrap(), 0);
}

#[test]
fn healthy_offline_repeat_uses_trusted_bytes_without_writes_locks_traffic_or_probes() {
    for node_only in [false, true] {
        let mut f = Fixture::new();
        if node_only {
            f.lock["tools"].as_object_mut().unwrap().remove("pnpm");
            f.declare(json!({}));
        }
        f.run(&Upstream::new(&f, false, None), FROZEN).unwrap();
        fs::write(
            f.owned.root().join("package-lock.json"),
            "dependency sentinel",
        )
        .unwrap();
        fs::write(f.owned.root().join("pnpm-lock.yaml"), "dependency sentinel").unwrap();
        // Prove repeat doesn't recreate even an absent producer lock file.
        fs::remove_file(f.owned.home().join("gc.lock")).unwrap();
        let empty = LoopbackServer::new([], |_| {}).unwrap();
        // Plain --offline also installs locked dependencies, which this
        // fixture's pnpm cannot run; dependency offline installs have their
        // own tests.
        for flags in [
            OFFLINE,
            &["--tools-only", "--offline", "--frozen"],
            &["--tools-only", "--offline", "--no-frozen"],
        ] {
            assert_eq!(attempt(&f, &empty, flags).unwrap(), 0);
        }
        assert!(!f.owned.home().join("gc.lock").exists());
        for file in ["package-lock.json", "pnpm-lock.yaml"] {
            assert_eq!(
                fs::read(f.owned.root().join(file)).unwrap(),
                b"dependency sentinel"
            );
        }
        assert_eq!(empty.hits(), 0);
    }
}

#[test]
fn offline_force_reverifies_trusted_cohorts_and_preserves_sources_dependencies_and_environment() {
    for node_only in [false, true] {
        let mut f = Fixture::new();
        if node_only {
            f.lock["tools"].as_object_mut().unwrap().remove("pnpm");
            f.declare(json!({}));
        }
        f.run(&Upstream::new(&f, false, None), FROZEN).unwrap();
        for name in [
            "package-lock.json",
            "pnpm-lock.yaml",
            "yarn.lock",
            "bun.lock",
        ] {
            fs::write(f.owned.root().join(name), format!("{name} sentinel")).unwrap();
        }
        let lock = fs::read(f.owned.root().join("turbo.lock")).unwrap();
        let declaration = fs::read(f.owned.root().join("package.json")).unwrap();
        let environment: std::collections::BTreeMap<_, _> = std::env::vars_os().collect();
        let empty = LoopbackServer::new([], |_| {}).unwrap();
        // Repeated forced re-verification; plain --offline --force would also
        // install locked dependencies, which this fixture's pnpm cannot run.
        for _ in 0..2 {
            let before = f.manifest().unwrap();
            assert_eq!(f.run(&empty, OFFLINE_FORCE).unwrap(), 0);
            assert_eq!(f.manifest().unwrap(), before);
            let authorization = super::super::authorization::Authorization::inspect(
                f.owned.root(),
                Some(f.owned.home()),
                check::context().unwrap(),
                Default::default(),
            )
            .unwrap();
            assert_eq!(
                authorization
                    .with_current(|plan, _| plan.tools().len())
                    .unwrap(),
                if node_only { 1 } else { 2 }
            );
            assert_eq!(fs::read(f.owned.root().join("turbo.lock")).unwrap(), lock);
            assert_eq!(
                fs::read(f.owned.root().join("package.json")).unwrap(),
                declaration
            );
            for name in [
                "package-lock.json",
                "pnpm-lock.yaml",
                "yarn.lock",
                "bun.lock",
            ] {
                assert_eq!(
                    fs::read_to_string(f.owned.root().join(name)).unwrap(),
                    format!("{name} sentinel")
                );
            }
            assert_eq!(
                std::env::vars_os().collect::<std::collections::BTreeMap<_, _>>(),
                environment
            );
            assert_eq!(attempt(&f, &empty, OFFLINE).unwrap(), 0);
            f.no_execution();
        }
        assert_eq!(empty.hits(), 0);
    }
}

#[test]
fn unsafe_storage_fails_before_any_effect() {
    let empty = LoopbackServer::new([], |_| {}).unwrap();
    for path in [".turbo", ".turbo/setup-lock"] {
        for link in [false, true] {
            let f = Fixture::new();
            let destination = f.owned.root().join(path);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            if link {
                symlink(f.owned.home(), &destination).unwrap();
            } else {
                fs::write(&destination, "not a directory").unwrap();
            }
            for flags in [OFFLINE, OFFLINE_FORCE] {
                let message = attempt(&f, &empty, flags).unwrap_err().to_string();
                assert!(message.contains("unsafe .turbo storage path"), "{message}");
            }
        }
    }
    assert_eq!(empty.hits(), 0);
}

#[test]
fn missing_and_damaged_trees_are_named_and_other_damage_rebuilds_from_the_store() {
    let empty = LoopbackServer::new([], |_| {}).unwrap();
    let fresh = Fixture::new();
    missing(&fresh, &empty, &["node", "pnpm"]);
    assert!(!fresh.owned.root().join(".turbo").exists());
    assert!(!fresh.owned.home().join("repos").exists());
    for case in 0..5 {
        let f = installed();
        let current = Installed::open(&f.owned).current().unwrap().unwrap();
        match case {
            0 => fs::remove_dir_all(current.bin.parent().unwrap()).unwrap(),
            1 => fs::remove_file(current.tree("node").join("bin/node")).unwrap(),
            2 => fs::remove_file(current.tree("pnpm").join("dist/resource")).unwrap(),
            3 => turborepo_setup::test_support::damage(
                &current.tree("node").join("resource"),
                "damaged",
            ),
            _ => fs::remove_file(current.bin.join("pnpx")).unwrap(),
        }
        match case {
            1 | 3 => missing(&f, &empty, &["node"]),
            2 => missing(&f, &empty, &["pnpm"]),
            // The environment is rebuilt from intact trees.
            _ => installs_offline(&f, &empty),
        }
        f.no_execution();
    }
    assert_eq!(empty.hits(), 0);
}

#[test]
fn unsafe_external_evidence_fails_closed_and_stale_evidence_is_rebound_from_the_store() {
    let empty = LoopbackServer::new([], |_| {}).unwrap();
    for case in 0..7 {
        let f = installed();
        let path = record(&f);
        match case {
            0 => fs::remove_file(&path).unwrap(),
            1 => fs::write(&path, "corrupt").unwrap(),
            2 => fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap(),
            3 => fs::set_permissions(path.parent().unwrap(), fs::Permissions::from_mode(0o777))
                .unwrap(),
            4 => {
                fs::rename(&path, f.owned.root().join("repo-record.json")).unwrap();
                symlink(f.owned.root().join("repo-record.json"), &path).unwrap();
            }
            5 => fs::write(f.owned.root().join("turbo.lock"), format!("{}\n", f.lock)).unwrap(),
            _ => {
                let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                value["inode"] = json!(0);
                fs::write(&path, value.to_string()).unwrap();
            }
        }
        // Readiness still succeeds: the rejection must be external provenance.
        assert!(
            ActivationPlan::captured(
                &turborepo_setup::lock::Snapshot::capture(f.owned.root()).unwrap(),
                check::context().unwrap(),
            )
            .is_ok()
        );
        match case {
            2..=4 => {
                for flags in [OFFLINE, OFFLINE_FORCE] {
                    let message = attempt(&f, &empty, flags).unwrap_err().to_string();
                    assert!(message.contains("unsafe setup user state"), "{message}");
                }
            }
            // Missing, corrupt, or stale records bind again to intact trees.
            _ => installs_offline(&f, &empty),
        }
        f.no_execution();
    }
    assert_eq!(empty.hits(), 0);
}

#[test]
fn declaration_drift_missing_lock_and_unsupported_offline_controls_never_resolve() {
    let empty = LoopbackServer::new([], |_| {}).unwrap();
    for case in 0..3 {
        let f = installed();
        match case {
            0 => fs::remove_file(f.owned.root().join("turbo.lock")).unwrap(),
            1 => fs::write(f.owned.root().join(".nvmrc"), "26.0.0").unwrap(),
            _ => fs::write(
                f.owned.root().join("package.json"),
                r#"{"packageManager":"pnpm@11.0.0"}"#,
            )
            .unwrap(),
        }
        assert!(attempt(&f, &empty, OFFLINE).is_err());
    }
    let f = installed();
    for flags in [
        vec!["--tools-only", "--offline", "--no-frozen", "--update-lock"],
        vec![
            "--tools-only",
            "--offline",
            "--force",
            "--no-frozen",
            "--update-lock",
        ],
        vec!["--tools-only", "--offline", "--no-lock"],
    ] {
        assert!(attempt(&f, &empty, &flags).is_err());
    }
    assert_eq!(
        attempt(&f, &empty, &["--check", "--tools-only", "--offline"]).unwrap(),
        0
    );
    // Main's locked plan is also read-only offline; it must retain the same
    // zero-write/traffic/probe guarantees verified by attempt().
    assert_eq!(
        attempt(&f, &empty, &["--plan", "--tools-only", "--offline"]).unwrap(),
        0
    );
    assert_eq!(empty.hits(), 0);
}

#[test]
fn offline_repeat_rechecks_original_sources_discovery_and_trust_before_success() {
    for case in 0..4 {
        let f = installed();
        let path = record(&f);
        let args = f.args(OFFLINE);
        let called = std::cell::Cell::new(0);
        let expected = std::cell::RefCell::new(None);
        let policy = f.owned.policy_at(f.owned.root()).unwrap();
        let result = crate::cli::dispatch_setup(&args, |args, setup_args| {
            run_with_policy(args, setup_args, Some(provision::Transports {
                bun: turborepo_setup::bun_provision::BunTransport::loopback_http_for_tests("http://127.0.0.1:1").unwrap(),
                artifact: None,
                tool_store: Some(f.owned.home().to_owned()),
                node: turborepo_setup::node_provision::NodeTransport::loopback_http_for_tests("http://127.0.0.1:1").unwrap(),
                pnpm: turborepo_setup::pnpm_provision::PnpmTransport::loopback_http_for_tests("http://127.0.0.1:1").unwrap(),
                registry: turborepo_setup::registry_resolution::RegistryTransport::loopback_http_for_tests("http://127.0.0.1:1").unwrap(),
                uv: turborepo_setup::uv_resolution::UvTransport::official(&policy).unwrap(),
                rustup: turborepo_setup::rustup_provision::RustupTransport::official(&policy).unwrap(),
                python: turborepo_setup::python_resolution::PythonTransport::official(&policy).unwrap(),
                go: turborepo_setup::go_resolution::GoTransport::official(&policy).unwrap(),
            }), f.git.clone(), || {
                called.set(called.get() + 1);
                if called.get() == 2 {
                    match case {
                        0 => fs::write(f.owned.root().join("package.json"), r#"{"packageManager":"pnpm@10.0.0","scripts":{"changed":"ignored by declaration matching"}}"#).unwrap(),
                        1 => fs::write(f.owned.root().join("apps/web/turbo.json"), ENABLED).unwrap(),
                        2 => fs::remove_file(&path).unwrap(),
                        _ => fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap(),
                    }
                    *expected.borrow_mut() = Some(snapshot(f.owned.root().parent().unwrap()));
                }
                Ok(f.owned.policy_at(&f.owned.root().join("apps/web/src"))?)
            })
        }).unwrap();
        if case == 2 {
            // A record removed during the repeat binds again to intact trees.
            assert_eq!(result.unwrap(), 0);
            assert!(record(&f).is_file());
            f.no_execution();
            continue;
        }
        let message = result.unwrap_err().to_string();
        if case == 3 {
            // An unsafe record also fails provisioning's trust check.
            assert!(message.contains("unsafe setup user state"), "{message}");
        } else {
            // Changed sources fail the repeat's own recheck.
            assert_eq!(called.get(), 2, "case {case}: {message}");
        }
        assert_eq!(
            snapshot(f.owned.root().parent().unwrap()),
            expected.into_inner().unwrap()
        );
        f.no_execution();
    }
}

#[test]
fn offline_repeat_does_not_wait_for_writer_or_store_leases() {
    let f = installed();
    let empty = LoopbackServer::new([], |_| {}).unwrap();
    let sources = f.sources().unwrap();
    let writer = sources.guard().unwrap();
    let installed = Installed::open(&f.owned);
    let store = installed
        .store
        .begin_setup(f.owned.root(), turborepo_tool_install::Reuse::Sealed)
        .unwrap();
    std::thread::scope(|scope| {
        let (tx, rx) = std::sync::mpsc::channel();
        let fixture = &f;
        let upstream = &empty;
        let pending = scope.spawn(move || tx.send(attempt(fixture, upstream, OFFLINE)).unwrap());
        let result = rx.recv_timeout(std::time::Duration::from_secs(5));
        drop(store);
        drop(writer);
        pending.join().unwrap();
        assert_eq!(result.unwrap().unwrap(), 0);
    });
    assert_eq!(empty.hits(), 0);
}
