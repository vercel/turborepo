use std::{
    cell::Cell,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    path::Path,
    time::SystemTime,
};

use turborepo_setup::{
    activation::ActivationPlan,
    execution_identity::{ExecutionContext, Libc},
};
use turborepo_tool_install::Store;

use super::*;
use crate::commands::setup::authorization::{Authorization, Error as AuthorizationError};

fn context() -> ExecutionContext {
    let libc = match provision::platform().unwrap() {
        turborepo_setup::lock::Platform::LinuxX64Gnu
        | turborepo_setup::lock::Platform::LinuxArm64Gnu => Libc::Gnu { abi: "gnu".into() },
        _ => Libc::None,
    };
    // Fixture context only, not runtime ABI/CPU qualification.
    ExecutionContext::new(
        turborepo_platform::Platform::current(),
        libc,
        "authorization-fixture".into(),
        ["baseline".into()].into(),
    )
    .unwrap()
}

fn inspect(f: &Fixture) -> Result<Authorization, AuthorizationError> {
    Authorization::inspect(
        f.owned.root(),
        f.owned.home(),
        context(),
        Default::default(),
    )
}

fn installed() -> (Fixture, Upstream) {
    let f = Fixture::new();
    let u = Upstream::new(&f, false, None);
    f.run(&u, FROZEN).unwrap();
    (f, u)
}

fn record(f: &Fixture) -> PathBuf {
    fs::read_dir(f.owned.home().join("turborepo/setup-roots"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
}

#[derive(Debug, PartialEq, Eq)]
struct Entry(PathBuf, u32, SystemTime, Vec<u8>, Option<PathBuf>);

// Include all owned state: repo, Git index, generations, locks and user state.
fn snapshot(root: &Path) -> Vec<Entry> {
    fn visit(path: &Path, entries: &mut Vec<Entry>) {
        let metadata = fs::symlink_metadata(path).unwrap();
        let link = metadata.file_type().is_symlink();
        entries.push(Entry(
            path.to_owned(),
            metadata.mode(),
            metadata.modified().unwrap(),
            if metadata.is_file() {
                fs::read(path).unwrap()
            } else {
                vec![]
            },
            if link {
                Some(fs::read_link(path).unwrap())
            } else {
                None
            },
        ));
        if metadata.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                visit(&entry.unwrap().path(), entries);
            }
        }
    }
    let mut entries = vec![];
    visit(root, &mut entries);
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

fn rejects_without_effects(f: &Fixture, u: &Upstream, selected: &Authorization) {
    let before = snapshot(f.owned.root().parent().unwrap());
    let hits = u.hits();
    let called = Cell::new(false);
    let error = selected.with_current(|_| called.set(true)).unwrap_err();
    assert!(error.to_string().contains("turbo setup"), "{error}");
    assert!(!called.get());
    assert!(inspect(f).is_err());
    assert_eq!(snapshot(f.owned.root().parent().unwrap()), before);
    assert_eq!(u.hits(), hits);
    f.no_execution();
}

#[test]
fn authorization_success_is_readonly_and_rechecks_at_every_use_without_locks() {
    let (f, u) = installed();
    fs::remove_file(f.owned.root().join(".turbo/tools/transaction.lock")).unwrap();
    let before = snapshot(f.owned.root().parent().unwrap());
    let environment: std::collections::BTreeMap<_, _> = std::env::vars_os().collect();
    let selected = inspect(&f).unwrap();
    for _ in 0..2 {
        selected
            .with_current(|plan| {
                assert_eq!(plan.tools().len(), 2);
                assert!(plan.runtime_env().is_empty());
                let path = plan.path(Some(std::ffi::OsStr::new("/host/bin"))).unwrap();
                let entries: Vec<_> = std::env::split_paths(&path).collect();
                assert_eq!(
                    entries,
                    vec![plan.path_prepend().to_owned(), "/host/bin".into()]
                );
            })
            .unwrap();
    }
    assert_eq!(snapshot(f.owned.root().parent().unwrap()), before);
    assert_eq!(
        std::env::vars_os().collect::<std::collections::BTreeMap<_, _>>(),
        environment
    );
    assert_eq!(u.hits(), 3);
    f.no_execution();

    // Neither inspection nor use may acquire producer locks.
    let sources = f.sources().unwrap();
    let writer = sources.guard().unwrap();
    let store = Store::open(f.owned.root()).unwrap();
    std::thread::scope(|scope| {
        let (send, receive) = std::sync::mpsc::channel();
        let fixture = &f;
        let pending = scope.spawn(move || {
            send.send(inspect(fixture).and_then(|a| a.with_current(|_| ())))
                .unwrap()
        });
        let result = receive.recv_timeout(std::time::Duration::from_secs(5));
        drop(store);
        drop(writer);
        pending.join().unwrap();
        result.unwrap().unwrap();
    });
}

#[test]
fn removed_corrupt_stale_or_unsafe_external_records_never_authorize() {
    for case in 0..14 {
        let (f, u) = installed();
        let selected = inspect(&f).unwrap();
        let path = record(&f);
        let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        match case {
            0 => fs::remove_file(&path).unwrap(),
            1 => fs::write(&path, "corrupt").unwrap(),
            2 => {
                value["schema"] = json!(2);
            }
            3 => {
                value["lock_sha256"] = json!("0".repeat(64));
            }
            4 => {
                value["installation"]["manifest_sha256"] = json!("0".repeat(64));
            }
            5 => {
                value["installation"]["bin"] = json!(f.owned.root().join(".turbo/tools/bin"));
            }
            6 => {
                value["installation"]["inode"] = json!(0);
            }
            7 => {
                value["installation"]["tools"][0]["executables"] = json!({"git":"bin/node"});
            }
            8 => {
                value["installation"]["root"] = json!(f.owned.home());
            }
            9 => fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap(),
            10 => fs::set_permissions(path.parent().unwrap(), fs::Permissions::from_mode(0o777))
                .unwrap(),
            11 => {
                fs::remove_file(&path).unwrap();
                symlink(f.owned.root().join("turbo.lock"), &path).unwrap();
            }
            12 => fs::hard_link(&path, f.owned.root().join("committed-record")).unwrap(),
            _ => {
                value["unknown"] = json!(true);
            }
        }
        if matches!(case, 2..=8 | 13) {
            fs::write(&path, value.to_string()).unwrap();
        }
        rejects_without_effects(&f, &u, &selected);
    }
}

#[test]
fn lock_declaration_and_captured_source_drift_fail_closed() {
    for case in 0..6 {
        let (f, u) = installed();
        let selected = inspect(&f).unwrap();
        match case {
            0 => fs::write(f.owned.root().join("turbo.lock"), format!("{}\n", f.lock)).unwrap(),
            1 => fs::remove_file(f.owned.root().join("turbo.lock")).unwrap(),
            2 => fs::write(f.owned.root().join(".nvmrc"), "26.0.0").unwrap(),
            3 => fs::write(
                f.owned.root().join("package.json"),
                r#"{"packageManager":"pnpm@11.0.0"}"#,
            )
            .unwrap(),
            4 => fs::write(f.owned.root().join(".node-version"), "26.0.0").unwrap(),
            _ => fs::write(f.owned.root().join("turbo.json"), "{}").unwrap(),
        }
        let before = snapshot(f.owned.root().parent().unwrap());
        let called = Cell::new(false);
        assert!(selected.with_current(|_| called.set(true)).is_err());
        assert!(!called.get());
        // Policy gating is future caller work; recapture can inspect a changed
        // non-declaration config, but the original selection must reject it.
        if case != 5 {
            assert!(inspect(&f).is_err());
        }
        assert_eq!(snapshot(f.owned.root().parent().unwrap()), before);
        assert_eq!(u.hits(), 3);
        f.no_execution();
    }
}

#[test]
fn every_export_and_generation_is_freshly_checked_before_use() {
    for case in 0..5 {
        let (f, u) = installed();
        let selected = inspect(&f).unwrap();
        let current = Store::inspect(f.owned.root()).unwrap().unwrap();
        match case {
            0 => fs::remove_file(current.bin.join("node")).unwrap(),
            1 => fs::remove_file(current.bin.join("pnpm")).unwrap(),
            2 => fs::remove_file(current.bin.join("pnpx")).unwrap(),
            3 => symlink("../tools/node/bin/node", current.bin.join("git")).unwrap(),
            _ => fs::remove_file(f.owned.root().join(".turbo/tools/manifest.json")).unwrap(),
        }
        rejects_without_effects(&f, &u, &selected);
    }
}

#[test]
fn damaged_contents_are_left_to_setup_verification() {
    for case in 0..3 {
        let (f, u) = installed();
        let selected = inspect(&f).unwrap();
        selected.with_current(|_| ()).unwrap();
        let manifest = f.manifest().unwrap();
        let evidence = fs::read(record(&f)).unwrap();
        let tools = Store::inspect(f.owned.root())
            .unwrap()
            .unwrap()
            .bin
            .join("../tools");
        match case {
            0 => turborepo_setup::test_support::damage(&tools.join("node/resource"), "damaged"),
            1 => fs::remove_file(tools.join("pnpm/dist/resource")).unwrap(),
            _ => turborepo_setup::test_support::damage(
                &tools.join("node/bin/node"),
                "#!/bin/sh\nexit 0\n",
            ),
        }
        let before = snapshot(f.owned.root().parent().unwrap());
        selected.with_current(|_| ()).unwrap();
        inspect(&f).unwrap().with_current(|_| ()).unwrap();
        assert_eq!(snapshot(f.owned.root().parent().unwrap()), before);
        assert_eq!(f.manifest().unwrap(), manifest);
        assert_eq!(fs::read(record(&f)).unwrap(), evidence);

        let error = f.check(super::checks::CHECK).unwrap_err().to_string();
        assert!(error.contains("turbo setup"), "case {case}: {error}");
        assert!(
            matches!(
                Store::inspect_with(f.owned.root(), turborepo_tool_install::Verification::Full),
                Err(turborepo_tool_install::Error::DamagedContents)
            ),
            "case {case}: a full check sees the damage"
        );
        assert_eq!(u.hits(), 3);
        f.no_execution();
    }
}

#[test]
fn a_manifest_that_no_longer_matches_the_setup_record_is_untrusted() {
    let (f, u) = installed();
    let selected = inspect(&f).unwrap();
    let path = f.owned.root().join(".turbo/tools/manifest.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["tools"][0]["tree_sha256"] = json!("0".repeat(64));
    fs::write(&path, manifest.to_string()).unwrap();
    assert!(
        turborepo_tool_install::Store::inspect_with(
            f.owned.root(),
            turborepo_tool_install::Verification::Shallow
        )
        .unwrap()
        .is_some()
    );

    let called = Cell::new(false);
    assert!(matches!(
        selected.with_current(|_| called.set(true)),
        Err(AuthorizationError::Untrusted)
    ));
    assert!(!called.get());
    assert!(matches!(inspect(&f), Err(AuthorizationError::Untrusted)));
    assert_eq!(u.hits(), 3);
    f.no_execution();
}

#[test]
fn source_errors_include_local_repair_guidance_and_preserve_the_cause() {
    use std::error::Error as _;

    let (f, u) = installed();
    let selected = inspect(&f).unwrap();
    fs::write(f.owned.root().join(".nvmrc"), "26.0.0").unwrap();
    let error = selected.with_current(|_| ()).unwrap_err();
    assert!(matches!(error, AuthorizationError::Sources(_)));
    let message = error.to_string();
    assert!(message.contains("run turbo setup locally"));
    assert!(message.contains("commit the changed turbo.lock"));
    assert!(error.source().is_some());
    assert_eq!(u.hits(), 3);
    f.no_execution();
}

#[test]
fn a_new_trusted_generation_cannot_replace_an_existing_selection() {
    let (f, u) = installed();
    let selected = inspect(&f).unwrap();
    f.run(&u, &["--force", "--tools-only", "--frozen"]).unwrap();
    let before = snapshot(f.owned.root().parent().unwrap());
    let called = Cell::new(false);
    assert!(matches!(
        selected.with_current(|_| called.set(true)),
        Err(AuthorizationError::Changed)
    ));
    assert!(!called.get());
    inspect(&f).unwrap().with_current(|_| ()).unwrap();
    assert_eq!(snapshot(f.owned.root().parent().unwrap()), before);
    assert_eq!(u.hits(), 6);
    f.no_execution();
}

#[test]
fn healthy_repository_forgery_is_readiness_not_authority() {
    let (f, u) = installed();
    let selected = inspect(&f).unwrap();
    let tools = Store::inspect(f.owned.root()).unwrap().unwrap().tools;
    Store::open(f.owned.root())
        .unwrap()
        .force_reconcile_checked(
            &tools,
            |tool, root| {
                for path in tool.executables.values() {
                    let path = root.join(path);
                    fs::create_dir_all(path.parent().unwrap())?;
                    fs::write(&path, "#!/bin/sh\ntouch probe-ran\nexit 91\n")?;
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
                }
                Ok(())
            },
            || Ok(()),
        )
        .unwrap();
    // A self-consistent manifest and valid trees cannot mint external evidence.
    assert!(
        ActivationPlan::captured(
            &turborepo_setup::lock::Snapshot::capture(f.owned.root()).unwrap(),
            context(),
            turborepo_tool_install::Verification::Shallow,
        )
        .is_ok()
    );
    rejects_without_effects(&f, &u, &selected);
    fs::remove_file(record(&f)).unwrap();
    rejects_without_effects(&f, &u, &selected);
}

#[test]
fn missing_user_state_is_not_created_or_replaced_by_ancestor_evidence() {
    let (f, u) = installed();
    let home = f.owned.home().join("never-created");
    let before = snapshot(f.owned.root().parent().unwrap());
    assert!(matches!(
        Authorization::inspect(f.owned.root(), &home, context(), Default::default()),
        Err(AuthorizationError::Untrusted)
    ));
    assert!(!home.exists());
    assert_eq!(snapshot(f.owned.root().parent().unwrap()), before);
    assert_eq!(u.hits(), 3);
    f.no_execution();
}

#[test]
fn moved_and_same_path_recloned_roots_require_new_setup() {
    for reclone in [false, true] {
        let (f, u) = installed();
        let selected = inspect(&f).unwrap();
        let moved = f.owned.home().join("moved-repo");
        fs::rename(f.owned.root(), &moved).unwrap();
        if reclone {
            fs::create_dir(f.owned.root()).unwrap();
            for name in ["turbo.lock", ".nvmrc", "package.json", "turbo.json"] {
                fs::copy(moved.join(name), f.owned.root().join(name)).unwrap();
            }
            fs::rename(moved.join(".turbo"), f.owned.root().join(".turbo")).unwrap();
        }
        let before = snapshot(f.owned.root().parent().unwrap());
        let called = Cell::new(false);
        assert!(selected.with_current(|_| called.set(true)).is_err());
        assert!(!called.get());
        let root = if reclone {
            f.owned.root()
        } else {
            moved.as_path()
        };
        assert!(
            ActivationPlan::captured(
                &turborepo_setup::lock::Snapshot::capture(root).unwrap(),
                context(),
                turborepo_tool_install::Verification::Shallow,
            )
            .is_ok()
        );
        assert!(
            Authorization::inspect(root, f.owned.home(), context(), Default::default()).is_err()
        );
        assert_eq!(snapshot(f.owned.root().parent().unwrap()), before);
        assert_eq!(u.hits(), 3);
    }
}
