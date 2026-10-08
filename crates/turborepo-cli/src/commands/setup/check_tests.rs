use std::{os::unix::fs::MetadataExt, path::Path, time::SystemTime};

use super::*;

#[derive(Debug, PartialEq, Eq)]
struct Entry(PathBuf, u32, SystemTime, Vec<u8>, Option<PathBuf>);

// All owned files, including the Git index, ignored generations, links,
// directories and synthetic user/system state. No host inputs or exclusions.
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

const CHECK: &[&str] = &["--check", "--tools-only"];
impl Fixture {
    fn check(&self, flags: &[&str]) -> Result<i32, Error> {
        let before = snapshot(self.owned.root().parent().unwrap());
        let args = self.args(flags);
        let result = crate::cli::dispatch_setup(&args, |args, setup| {
            run_with_policy(args, setup, None, || {
                Ok(self
                    .owned
                    .policy_at(&self.owned.root().join("apps/web/src"))?)
            })
        })
        .unwrap();
        assert_eq!(snapshot(self.owned.root().parent().unwrap()), before);
        self.no_execution();
        result
    }
    fn install(&self, upstream: &Upstream) {
        assert!(
            std::process::Command::new("git")
                .current_dir(self.owned.root())
                .args(["add", "."])
                .status()
                .unwrap()
                .success()
        );
        assert_eq!(self.run(upstream, FROZEN).unwrap(), 0);
    }
}

#[test]
fn install_repeat_and_check_all_readonly_modes_without_traffic_or_locks() {
    let f = Fixture::new();
    let u = Upstream::new(&f, false, None);
    assert!(matches!(
        f.check(CHECK),
        Err(Error::Activation(
            turborepo_setup::activation::Error::MissingInventory
        ))
    ));
    assert_eq!(u.hits(), 0);
    f.install(&u);
    let before = snapshot(f.owned.root().parent().unwrap());
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    assert_eq!(snapshot(f.owned.root().parent().unwrap()), before);
    for flags in [
        CHECK,
        &["--check", "--tools-only", "--frozen"],
        &["--check", "--tools-only", "--offline"],
        &["--check", "--tools-only", "--no-frozen", "--offline"],
    ] {
        assert_eq!(f.check(flags).unwrap(), 0);
    }
    assert_eq!(u.hits(), 3);
    let sources = turborepo_setup::lock::Snapshot::capture(f.owned.root()).unwrap();
    let writer = sources.guard().unwrap();
    let store = turborepo_tool_install::Store::open(f.owned.root()).unwrap();
    // Held by another transaction. Check must neither wait for nor take locks.
    std::thread::scope(|scope| {
        let (send, receive) = std::sync::mpsc::channel();
        let fixture = &f;
        let pending = scope.spawn(move || send.send(fixture.check(CHECK)).unwrap());
        let result = receive.recv_timeout(std::time::Duration::from_secs(5));
        drop(store);
        drop(writer);
        pending.join().unwrap();
        assert_eq!(result.unwrap().unwrap(), 0);
    });
    assert_eq!(u.hits(), 3);
}

#[test]
fn check_damage_declaration_inventory_and_lock_drift_are_actionable_readonly_failures() {
    for case in 0..10 {
        let mut f = Fixture::new();
        let u = Upstream::new(&f, false, None);
        f.install(&u);
        let current = turborepo_tool_install::Store::inspect(f.owned.root())
            .unwrap()
            .unwrap();
        match case {
            0 => fs::remove_file(f.owned.root().join(".turbo/tools/manifest.json")).unwrap(),
            1 => fs::write(current.bin.join("../tools/node/resource"), "damage").unwrap(),
            2 => fs::remove_file(current.bin.join("../tools/pnpm/dist/resource")).unwrap(),
            3 => fs::write(f.owned.root().join(".nvmrc"), "26.0.0").unwrap(),
            4 => {
                f.lock["tools"]["pnpm"]["version"] = json!("11.0.0");
                f.save();
            }
            5 => fs::remove_file(f.owned.root().join("turbo.lock")).unwrap(),
            6 => fs::write(f.owned.root().join("turbo.lock"), "invalid").unwrap(),
            7 => {
                let artifacts = f.lock["tools"]["node"]["installation"]["artifacts"]
                    .as_object_mut()
                    .unwrap();
                artifacts.values_mut().next().unwrap()["distribution"]["sha256"] =
                    json!("0".repeat(64));
                f.save();
            }
            8 => fs::write(
                f.owned.root().join("package.json"),
                r#"{"packageManager":"pnpm@11.0.0"}"#,
            )
            .unwrap(),
            _ => {
                fs::remove_file(current.bin.join("node")).unwrap();
                std::os::unix::fs::symlink("../tools/pnpm/bin/pnpm.cjs", current.bin.join("node"))
                    .unwrap();
            }
        }
        let error = f.check(CHECK).unwrap_err().to_string();
        assert!(
            error.contains("turbo setup") || error.contains("lock"),
            "case {case}: {error}"
        );
        assert_eq!(u.hits(), 3);
    }
}

#[test]
fn check_unsupported_policy_and_adapters_fail_without_mutation_or_traffic() {
    for case in 0..7 {
        let mut f = Fixture::new();
        let u = Upstream::new(&f, false, None);
        match case {
            0 => fs::write(f.owned.root().join("turbo.json"), "{}").unwrap(),
            1 => fs::write(
                f.owned.root().join("turbo.json"),
                r#"{"futureFlags":{"experimentalSetup":true},"setup":{}}"#,
            )
            .unwrap(),
            2 => fs::write(f.owned.root().join("apps/web/.npmrc"), "").unwrap(),
            3 => fs::write(f.owned.home().join(".npmrc"), "").unwrap(),
            4 => {
                f.lock["tools"]["node"]["adapter"] = json!("generic-download");
                f.save();
            }
            5 => {
                fs::write(f.owned.root().join("turbo.json"), r#"{"futureFlags":{"experimentalSetup":true,"experimentalCargoWorkspaces":true}}"#).unwrap();
                fs::write(f.owned.root().join("Cargo.toml"), "not probed").unwrap();
            }
            _ => {
                f.declare(json!({"packageManager":"npm@11.0.0"}));
                f.lock["tools"].as_object_mut().unwrap().remove("pnpm");
                f.save();
            }
        }
        assert!(f.check(CHECK).is_err(), "case {case}");
        assert!(!f.owned.root().join(".turbo").exists());
        assert_eq!(u.hits(), 0);
    }
}

#[test]
fn check_rejects_a_locked_node_version_that_does_not_satisfy_native_declarations() {
    let mut f = Fixture::new();
    let u = Upstream::new(&f, false, None);
    f.install(&u);
    fs::write(f.owned.root().join(".nvmrc"), "26.0.0").unwrap();
    f.lock["tools"]["node"]["declarations"][0]["request"] = json!("26.0.0");
    f.save();
    // Dependency regression: keep visible until the ActivationPlan follow-up
    // validates the selected Node version, rather than only provenance equality.
    assert!(f.check(CHECK).is_err());
    assert_eq!(u.hits(), 3);
}

#[test]
fn check_revalidates_captured_discovery_sources_and_policy_before_success() {
    for case in 0..9 {
        let f = Fixture::new();
        let u = Upstream::new(&f, false, None);
        f.install(&u);
        let args = f.args(CHECK);
        let calls = std::cell::Cell::new(0);
        let mut after_edit = None;
        let result = crate::cli::dispatch_setup(&args, |args, setup| {
            let after_edit = std::cell::RefCell::new(&mut after_edit);
            run_with_policy(args, setup, None, || {
                calls.set(calls.get() + 1);
                if calls.get() == 2 {
                    let (path, contents) = match case {
                        0 => (f.owned.root().join("turbo.json"), "{}".into()),
                        1 => (f.owned.root().join("turbo.lock"), format!("{} ", f.lock)),
                        2 => (f.owned.root().join(".nvmrc"), "26.0.0".into()),
                        3 => (f.owned.root().join("apps/web/turbo.json"), ENABLED.into()),
                        4 => (
                            f.owned.root().join("apps/web/pnpm-workspace.yaml"),
                            "packages: []".into(),
                        ),
                        5 => (
                            f.owned.root().join("apps/web/.git"),
                            "gitdir: /not-probed".into(),
                        ),
                        6 => (f.owned.home().join(".npmrc"), "".into()),
                        7 => (
                            f.owned.root().join("turbo.json"),
                            r#"{"futureFlags":{"experimentalSetup":true},"setup":{}}"#.into(),
                        ),
                        _ => (f.owned.root().join("package.json"), "{}".into()),
                    };
                    fs::write(path, contents).unwrap();
                    **after_edit.borrow_mut() = Some(snapshot(f.owned.root().parent().unwrap()));
                }
                Ok(f.owned.policy_at(&f.owned.root().join("apps/web/src"))?)
            })
        })
        .unwrap();
        assert!(result.is_err(), "case {case}");
        assert_eq!(calls.get(), 2);
        assert_eq!(
            snapshot(f.owned.root().parent().unwrap()),
            after_edit.unwrap()
        );
        assert_eq!(u.hits(), 3);
        f.no_execution();
    }
}
