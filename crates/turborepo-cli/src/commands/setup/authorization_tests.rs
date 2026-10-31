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
        Some(f.owned.home()),
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
    fs::read_dir(f.owned.home().join("repos"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("record.json")
}

#[derive(Debug, PartialEq, Eq)]
struct Entry(PathBuf, u32, SystemTime, Vec<u8>, Option<PathBuf>);

// Include all owned state: repo, Git index, locks and the tool store.
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
    let error = selected.with_current(|_, _| called.set(true)).unwrap_err();
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
    let before = snapshot(f.owned.root().parent().unwrap());
    let environment: std::collections::BTreeMap<_, _> = std::env::vars_os().collect();
    let selected = inspect(&f).unwrap();
    for _ in 0..2 {
        selected
            .with_current(|plan, activation| {
                assert_eq!(plan.tools().len(), 2);
                assert!(activation.vars().is_empty());
                let path = activation
                    .path(Some(std::ffi::OsStr::new("/host/bin")))
                    .unwrap();
                let entries: Vec<_> = std::env::split_paths(&path).collect();
                assert_eq!(
                    entries,
                    vec![activation.bin().to_owned(), "/host/bin".into()]
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
    let installed = Installed::open(&f.owned);
    let session = installed
        .store
        .begin_setup(f.owned.root(), turborepo_tool_install::Reuse::Sealed)
        .unwrap();
    std::thread::scope(|scope| {
        let (send, receive) = std::sync::mpsc::channel();
        let fixture = &f;
        let pending = scope.spawn(move || {
            send.send(inspect(fixture).and_then(|a| a.with_current(|_, _| ())))
                .unwrap()
        });
        let result = receive.recv_timeout(std::time::Duration::from_secs(5));
        drop(session);
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
                value["schema"] = json!(1);
            }
            3 => {
                value["lock_sha256"] = json!("0".repeat(64));
            }
            // Another environment, even one that exists, is not this one.
            4 => {
                value["env"] = json!("0".repeat(64));
            }
            5 => {
                value["env"] = json!("not-a-key");
            }
            6 => {
                value["inode"] = json!(0);
            }
            7 => {
                value["selection"] = json!(f.lock.to_string());
            }
            8 => {
                value["root"] = json!(f.owned.home());
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
        assert!(selected.with_current(|_, _| called.set(true)).is_err());
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
fn a_missing_or_unsealed_environment_is_checked_before_every_use() {
    for case in 0..3 {
        let (f, u) = installed();
        let selected = inspect(&f).unwrap();
        let env = Installed::open(&f.owned)
            .current()
            .unwrap()
            .unwrap()
            .bin
            .parent()
            .unwrap()
            .to_owned();
        match case {
            0 => fs::remove_dir_all(&env).unwrap(),
            1 => fs::remove_file(env.join("seal.json")).unwrap(),
            _ => fs::write(env.join("seal.json"), "{}").unwrap(),
        }
        rejects_without_effects(&f, &u, &selected);
    }
}

#[test]
fn damaged_contents_are_left_to_setup_verification() {
    for case in 0..3 {
        let (f, u) = installed();
        let selected = inspect(&f).unwrap();
        selected.with_current(|_, _| ()).unwrap();
        let evidence = fs::read(record(&f)).unwrap();
        let current = Installed::open(&f.owned).current().unwrap().unwrap();
        match case {
            0 => turborepo_setup::test_support::damage(
                &current.tree("node").join("resource"),
                "damaged",
            ),
            1 => fs::remove_file(current.tree("pnpm").join("dist/resource")).unwrap(),
            _ => turborepo_setup::test_support::damage(
                &current.tree("node").join("bin/node"),
                "#!/bin/sh\nexit 0\n",
            ),
        }
        let before = snapshot(f.owned.root().parent().unwrap());
        selected.with_current(|_, _| ()).unwrap();
        inspect(&f).unwrap().with_current(|_, _| ()).unwrap();
        assert_eq!(snapshot(f.owned.root().parent().unwrap()), before);
        assert_eq!(fs::read(record(&f)).unwrap(), evidence);

        // `turbo setup --check` re-hashes the trees and names the damage.
        let error = f.check(super::checks::CHECK).unwrap_err().to_string();
        assert!(
            error.contains("turbo setup --force"),
            "case {case}: {error}"
        );
        assert_eq!(u.hits(), 3);
        f.no_execution();
    }
}

#[test]
fn source_errors_include_local_repair_guidance_and_preserve_the_cause() {
    use std::error::Error as _;

    let (f, u) = installed();
    let selected = inspect(&f).unwrap();
    fs::write(f.owned.root().join(".nvmrc"), "26.0.0").unwrap();
    let error = selected.with_current(|_, _| ()).unwrap_err();
    assert!(matches!(error, AuthorizationError::Sources(_)));
    let message = error.to_string();
    assert!(message.contains("run turbo setup locally"));
    assert!(message.contains("commit the changed turbo.lock"));
    assert!(error.source().is_some());
    assert_eq!(u.hits(), 3);
    f.no_execution();
}

#[test]
fn forced_reverification_keeps_an_existing_selection_usable() {
    let (f, u) = installed();
    let selected = inspect(&f).unwrap();
    f.run(&u, &["--force", "--tools-only", "--frozen"]).unwrap();
    let called = Cell::new(false);
    selected.with_current(|_, _| called.set(true)).unwrap();
    assert!(called.get());
    // Healthy trees are re-hashed, never downloaded again.
    assert_eq!(u.hits(), 3);
    f.no_execution();
}

#[test]
fn the_repository_editor_link_is_never_read() {
    let (f, u) = installed();
    let selected = inspect(&f).unwrap();
    let decoy = tempfile::tempdir().unwrap();
    fs::write(decoy.path().join("node"), "#!/bin/sh\ntouch probe-ran\n").unwrap();
    let editor = f.owned.root().join(".turbo/bin");
    let current = Installed::open(&f.owned).current().unwrap().unwrap();
    assert_eq!(fs::read_link(&editor).unwrap(), current.stable_bin);
    fs::remove_file(&editor).unwrap();
    symlink(decoy.path(), &editor).unwrap();
    let bin = selected
        .with_current(|_, activation| activation.bin().to_owned())
        .unwrap();
    assert!(bin.starts_with(f.owned.home()));
    assert!(!bin.starts_with(f.owned.root()));
    assert_eq!(u.hits(), 3);
    f.no_execution();
}

#[test]
fn missing_user_state_is_not_created_or_replaced_by_ancestor_evidence() {
    let (f, u) = installed();
    let outside = tempfile::tempdir().unwrap();
    let home = outside.path().join("never-created");
    let before = snapshot(f.owned.root().parent().unwrap());
    assert!(matches!(
        Authorization::inspect(f.owned.root(), Some(&home), context(), Default::default()),
        Err(AuthorizationError::Untrusted)
    ));
    // Locating the store creates only its empty root.
    assert_eq!(fs::read_dir(&home).unwrap().count(), 0);
    assert_eq!(snapshot(f.owned.root().parent().unwrap()), before);
    assert_eq!(u.hits(), 3);
    f.no_execution();
}

#[test]
fn moved_and_same_path_recloned_roots_require_new_setup() {
    for reclone in [false, true] {
        let (f, u) = installed();
        let selected = inspect(&f).unwrap();
        let moved = f.owned.root().with_file_name("moved-repo");
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
        assert!(selected.with_current(|_, _| called.set(true)).is_err());
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
            )
            .is_ok()
        );
        assert!(
            Authorization::inspect(root, Some(f.owned.home()), context(), Default::default())
                .is_err()
        );
        assert_eq!(snapshot(f.owned.root().parent().unwrap()), before);
        assert_eq!(u.hits(), 3);
    }
}
