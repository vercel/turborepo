//! Offline repeat is strict external provenance, not a repo-readiness shortcut.
use std::os::unix::fs::{PermissionsExt, symlink};

use turborepo_setup::activation::ActivationPlan;
use turborepo_tool_install::Store;

use super::{checks::snapshot, *};

const OFFLINE: &[&str] = &["--tools-only", "--offline"];
const OFFLINE_FORCE: &[&str] = &["--tools-only", "--offline", "--force"];

fn installed() -> Fixture {
    let f = Fixture::new();
    f.run(&Upstream::new(&f, false, None), FROZEN).unwrap();
    f
}
fn record(f: &Fixture) -> PathBuf {
    fs::read_dir(f.owned.home().join("turborepo/setup-roots"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
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
fn unavailable(f: &Fixture, u: &LoopbackServer) -> Error {
    // Offline force has no other reinstall source than verified local bytes.
    let forced = attempt(f, u, OFFLINE_FORCE).unwrap_err();
    let error = attempt(f, u, OFFLINE).unwrap_err();
    assert_eq!(forced.to_string(), error.to_string());
    assert!(matches!(error, Error::Offline(_)), "{error}");
    let message = error.to_string();
    for fragment in [
        "node 24.0.0",
        "pnpm 10.0.0",
        &f.node_path,
        "pnpm-10.0.0.tgz",
        "SHA-256",
        "without --offline",
        "download-cache repair is not implemented",
    ] {
        assert!(message.contains(fragment), "{fragment}: {message}");
    }
    error
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
        fs::remove_file(f.owned.root().join(".turbo/tools/transaction.lock")).unwrap();
        let empty = LoopbackServer::new([], |_| {}).unwrap();
        for flags in [
            OFFLINE,
            OFFLINE_FORCE,
            &["--offline"][..],
            &["--offline", "--force"][..],
            &["--tools-only", "--offline", "--frozen"],
            &["--tools-only", "--offline", "--no-frozen"],
        ] {
            assert_eq!(attempt(&f, &empty, flags).unwrap(), 0);
        }
        assert!(
            !f.owned
                .root()
                .join(".turbo/tools/transaction.lock")
                .exists()
        );
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
fn unsafe_storage_reports_complete_urls_hashes_and_repair_limits_without_effects() {
    let empty = LoopbackServer::new([], |_| {}).unwrap();
    for path in [".turbo", ".turbo/tools"] {
        for link in [false, true] {
            let f = Fixture::new();
            let destination = f.owned.root().join(path);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            if link {
                symlink(f.owned.home(), &destination).unwrap();
            } else {
                fs::write(&destination, "not a directory").unwrap();
            }
            let error = unavailable(&f, &empty);
            let message = error.to_string();
            assert!(std::error::Error::source(&error).is_some());
            assert!(message.contains("unsafe .turbo storage path"), "{message}");
            assert!(message.contains(&format!("https://nodejs.org{}", f.node_path)));
            assert!(message.contains("https://registry.npmjs.org/pnpm/-/pnpm-10.0.0.tgz"));
            for digest in [Sha256::digest(&f.node), Sha256::digest(&f.pnpm)] {
                assert!(
                    message.contains(&format!("SHA-256 {digest:x}")),
                    "{message}"
                );
            }
        }
    }
    assert_eq!(empty.hits(), 0);
}

#[test]
fn missing_and_damaged_installations_report_the_complete_unverified_artifact_set() {
    let empty = LoopbackServer::new([], |_| {}).unwrap();
    let fresh = Fixture::new();
    unavailable(&fresh, &empty);
    assert!(!fresh.owned.root().join(".turbo").exists());
    assert!(!fresh.owned.home().join("turborepo").exists());
    for case in 0..5 {
        let f = installed();
        let current = Store::inspect(f.owned.root()).unwrap().unwrap();
        match case {
            0 => fs::remove_file(f.owned.root().join(".turbo/tools/manifest.json")).unwrap(),
            1 => fs::remove_file(current.bin.join("../tools/node/bin/node")).unwrap(),
            2 => fs::remove_file(current.bin.join("../tools/pnpm/dist/resource")).unwrap(),
            3 => fs::write(current.bin.join("../tools/node/resource"), "damaged").unwrap(),
            _ => fs::remove_file(current.bin.join("pnpx")).unwrap(),
        }
        unavailable(&f, &empty);
    }
}

#[test]
fn missing_corrupt_unsafe_and_stale_external_evidence_never_becomes_offline_authority() {
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
                value["installation"]["inode"] = json!(0);
                fs::write(&path, value.to_string()).unwrap();
            }
        }
        // Readiness still succeeds: the rejection must be external provenance.
        assert!(ActivationPlan::inspect(f.owned.root(), check::context().unwrap()).is_ok());
        unavailable(&f, &empty);
    }
}

#[test]
fn coherent_repository_only_forgery_is_healthy_but_never_authorized_offline() {
    let f = installed();
    let tools = Store::inspect(f.owned.root()).unwrap().unwrap().tools;
    Store::open(f.owned.root())
        .unwrap()
        .force_reconcile_checked(
            &tools,
            |tool, root| {
                for path in tool.executables.values() {
                    let file = root.join(path);
                    fs::create_dir_all(file.parent().unwrap())?;
                    fs::write(&file, "#!/bin/sh\ntouch probe-ran\nexit 91\n")?;
                    fs::set_permissions(&file, fs::Permissions::from_mode(0o755))?;
                }
                Ok(())
            },
            || Ok(()),
        )
        .unwrap();
    assert!(ActivationPlan::inspect(f.owned.root(), check::context().unwrap()).is_ok());
    let empty = LoopbackServer::new([], |_| {}).unwrap();
    unavailable(&f, &empty);
}

#[test]
fn declaration_drift_missing_lock_and_unsupported_offline_controls_never_resolve() {
    let empty = LoopbackServer::new([], |_| {}).unwrap();
    for case in 0..4 {
        let f = installed();
        match case {
            0 => fs::remove_file(f.owned.root().join("turbo.lock")).unwrap(),
            1 => fs::write(f.owned.root().join(".nvmrc"), "26.0.0").unwrap(),
            2 => fs::write(
                f.owned.root().join("package.json"),
                r#"{"packageManager":"pnpm@11.0.0"}"#,
            )
            .unwrap(),
            _ => fs::write(
                f.owned.root().join("apps/web/.npmrc"),
                "registry=https://private.invalid",
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
                trust_base: Some(f.owned.home().to_owned()),
                node: turborepo_setup::node_provision::NodeTransport::loopback_http_for_tests("http://127.0.0.1:1").unwrap(),
                pnpm: turborepo_setup::pnpm_provision::PnpmTransport::loopback_http_for_tests("http://127.0.0.1:1").unwrap(),
                registry: turborepo_setup::registry_resolution::RegistryTransport::loopback_http_for_tests("http://127.0.0.1:1").unwrap(),
                uv: turborepo_setup::uv_resolution::UvTransport::official(&policy).unwrap(),
                rustup: turborepo_setup::rustup_provision::RustupTransport::official(&policy).unwrap(),
                python: turborepo_setup::python_resolution::PythonTransport::official(&policy).unwrap(),
                go: turborepo_setup::go_resolution::GoTransport::official(&policy).unwrap(),
            }), || {
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
        assert!(result.is_err(), "case {case}");
        assert_eq!(called.get(), 2);
        assert_eq!(
            snapshot(f.owned.root().parent().unwrap()),
            expected.into_inner().unwrap()
        );
        f.no_execution();
    }
}

#[test]
fn offline_repeat_does_not_wait_for_writer_or_store_guards() {
    let f = installed();
    let empty = LoopbackServer::new([], |_| {}).unwrap();
    let sources = turborepo_setup::lock::Snapshot::capture(f.owned.root()).unwrap();
    let writer = sources.guard().unwrap();
    let store = Store::open(f.owned.root()).unwrap();
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
