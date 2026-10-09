use super::{checks::snapshot, *};

const PLAN: &[&str] = &["--plan", "--tools-only"];

impl Fixture {
    fn plan_report(&self, force: bool) -> Result<String, Error> {
        let discovery = root::Discovery::capture(&self.args(PLAN))?;
        let sources = turborepo_setup::lock::Snapshot::capture(self.owned.root())?;
        plan::inspect(&discovery, sources, force, || {
            Ok(self
                .owned
                .policy_at(&self.owned.root().join("apps/web/src"))?)
        })
    }

    fn expected_plan(&self, action: &str) -> String {
        format!(
            "Locked tools-only plan (turbo.lock unchanged):\n\
             node 24.0.0: {action}\n  Declaration: .nvmrc (24.0.0)\n  Source: https://nodejs.org{}\n  SHA-256: {:x}\n\
             pnpm 10.0.0: {action}\n  Declaration: package.json#/packageManager (pnpm@10.0.0)\n  Source: https://registry.npmjs.org/pnpm/-/pnpm-10.0.0.tgz\n  SHA-256: {:x}\n\
             Dependencies skipped (--tools-only); dependency readiness was not checked.\n\
             Tracked changes: none. No downloads, probes, or tasks run.",
            self.node_path,
            Sha256::digest(&self.node),
            Sha256::digest(&self.pnpm),
        )
    }

    fn assert_plan(&self, upstream: &Upstream, flags: &[&str], action: &str) {
        let before = snapshot(self.owned.root().parent().unwrap());
        let hits = upstream.hits();
        assert_eq!(self.run(upstream, flags).unwrap(), 0);
        assert_eq!(
            self.plan_report(flags.contains(&"--force")).unwrap(),
            self.expected_plan(action),
        );
        assert_eq!(snapshot(self.owned.root().parent().unwrap()), before);
        assert_eq!(upstream.hits(), hits);
        self.no_execution();
    }
}

#[test]
fn locked_plans_report_missing_reuse_and_force_in_every_readonly_mode() {
    let f = Fixture::new();
    let u = Upstream::new(&f, false, None);
    for installed in [false, true] {
        if installed {
            f.install(&u);
        }
        for extra in [
            None,
            Some("--frozen"),
            Some("--offline"),
            Some("--no-frozen"),
        ] {
            let mut flags = PLAN.to_vec();
            flags.extend(extra);
            f.assert_plan(
                &u,
                &flags,
                if installed {
                    "reuse (healthy installation)"
                } else {
                    "install (missing installation)"
                },
            );
            flags.push("--force");
            f.assert_plan(&u, &flags, "reinstall (--force)");
        }
        assert_eq!(f.owned.root().join(".turbo").exists(), installed);
    }
    assert_eq!(u.hits(), 3);
    let sources = turborepo_setup::lock::Snapshot::capture(f.owned.root()).unwrap();
    let writer = sources.guard().unwrap();
    let store = turborepo_tool_install::Store::open(f.owned.root()).unwrap();
    std::thread::scope(|scope| {
        let (send, receive) = std::sync::mpsc::channel();
        let fixture = &f;
        let pending = scope.spawn(move || send.send(fixture.plan_report(false)).unwrap());
        let result = receive.recv_timeout(std::time::Duration::from_secs(5));
        drop(store);
        drop(writer);
        pending.join().unwrap();
        assert_eq!(
            result.unwrap().unwrap(),
            f.expected_plan("reuse (healthy installation)")
        );
    });
    assert_eq!(u.hits(), 3);
}

#[test]
fn node_only_plan_ignores_disabled_ecosystems_without_probing_or_creating_storage() {
    let mut f = Fixture::new();
    f.lock["tools"].as_object_mut().unwrap().remove("pnpm");
    f.declare(json!({}));
    for name in ["Cargo.toml", "pyproject.toml", "go.mod"] {
        fs::write(f.owned.root().join(name), "not enabled; must not probe").unwrap();
    }
    let u = Upstream::new(&f, false, None);
    let before = snapshot(f.owned.root().parent().unwrap());
    assert_eq!(f.run(&u, PLAN).unwrap(), 0);
    assert_eq!(f.plan_report(false).unwrap(), format!(
        "Locked tools-only plan (turbo.lock unchanged):\n\
         node 24.0.0: install (missing installation)\n  Declaration: .nvmrc (24.0.0)\n  Source: https://nodejs.org{}\n  SHA-256: {:x}\n\
         Dependencies skipped (--tools-only); dependency readiness was not checked.\n\
         Tracked changes: none. No downloads, probes, or tasks run.",
        f.node_path, Sha256::digest(&f.node),
    ));
    assert_eq!(snapshot(f.owned.root().parent().unwrap()), before);
    assert_eq!(u.hits(), 0);
    assert!(!f.owned.root().join(".turbo").exists());
}

#[test]
fn damaged_plans_never_report_reuse_or_change_any_owned_bytes() {
    for case in 0..5 {
        let mut f = Fixture::new();
        let u = Upstream::new(&f, false, None);
        f.install(&u);
        let current = turborepo_tool_install::Store::inspect(f.owned.root())
            .unwrap()
            .unwrap();
        match case {
            0 => fs::write(current.bin.join("../tools/node/resource"), "damage").unwrap(),
            1 => fs::remove_file(current.bin.join("../tools/pnpm/dist/resource")).unwrap(),
            2 => fs::write(f.owned.root().join(".turbo/tools/manifest.json"), "invalid").unwrap(),
            3 => {
                fs::write(current.bin.join("../tools/node/resource"), "damage").unwrap();
                fs::remove_file(current.bin.join("node")).unwrap();
                std::os::unix::fs::symlink("../tools/pnpm/bin/pnpm.cjs", current.bin.join("node"))
                    .unwrap();
            }
            _ => {
                f.lock["tools"]["node"]["installation"]["artifacts"]
                    .as_object_mut()
                    .unwrap()
                    .values_mut()
                    .next()
                    .unwrap()["distribution"]["sha256"] = json!("0".repeat(64));
                f.save();
            }
        }
        let before = snapshot(f.owned.root().parent().unwrap());
        for force in [false, true] {
            let mut flags = PLAN.to_vec();
            if force {
                flags.push("--force");
            }
            if matches!(case, 0 | 1 | 4) {
                assert_eq!(f.run(&u, &flags).unwrap(), 0);
                let action = if force {
                    "reinstall (--force)"
                } else if case == 4 {
                    "repair (installation does not match turbo.lock)"
                } else {
                    "repair (damaged installation)"
                };
                let report = f.plan_report(force).unwrap();
                assert!(report.contains(action) && !report.contains("reuse"));
                if case != 4 {
                    assert_eq!(report, f.expected_plan(action));
                }
            } else {
                assert!(matches!(
                    f.run(&u, &flags),
                    Err(Error::Activation(
                        turborepo_setup::activation::Error::DamagedInventory(_)
                    ))
                ));
                assert!(matches!(
                    f.plan_report(force),
                    Err(Error::Activation(
                        turborepo_setup::activation::Error::DamagedInventory(_)
                    ))
                ));
            }
        }
        assert_eq!(snapshot(f.owned.root().parent().unwrap()), before);
        assert_eq!(u.hits(), 3);
        if matches!(case, 0 | 1) {
            assert_eq!(
                f.run(&u, &["--force", "--tools-only", "--frozen"]).unwrap(),
                0
            );
            assert_eq!(u.hits(), 6);
            assert_eq!(
                f.plan_report(false).unwrap(),
                f.expected_plan("reuse (healthy installation)")
            );
        }
        f.no_execution();
    }
}

#[test]
fn plan_storage_gates_match_provisioning_and_recheck_before_reporting() {
    for installed in [false, true] {
        for late in [false, true] {
            let f = Fixture::new();
            let u = Upstream::new(&f, false, None);
            if installed {
                f.install(&u);
            }
            let edit = || {
                if installed {
                    track(f.owned.root());
                } else {
                    fs::write(f.owned.root().join(".gitignore"), "").unwrap();
                }
            };
            if !late {
                edit();
            }
            let expected = std::cell::RefCell::new(snapshot(f.owned.root().parent().unwrap()));
            let discovery = root::Discovery::capture(&f.args(PLAN)).unwrap();
            let sources = turborepo_setup::lock::Snapshot::capture(f.owned.root()).unwrap();
            let calls = std::cell::Cell::new(0);
            let result = plan::inspect(&discovery, sources, installed, || {
                calls.set(calls.get() + 1);
                if late && calls.get() == 2 {
                    edit();
                    *expected.borrow_mut() = snapshot(f.owned.root().parent().unwrap());
                }
                Ok(f.owned.policy_at(&f.owned.root().join("apps/web/src"))?)
            });
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("storage must be untracked and Git-ignored")
            );
            assert!(
                f.run(&u, FROZEN)
                    .unwrap_err()
                    .to_string()
                    .contains("storage must be untracked and Git-ignored")
            );
            assert_eq!(
                snapshot(f.owned.root().parent().unwrap()),
                expected.into_inner()
            );
            assert_eq!(u.hits(), if installed { 3 } else { 0 });
            assert_eq!(f.owned.root().join(".turbo").exists(), installed);
            f.no_execution();
        }
    }
}

#[test]
fn missing_stale_unsupported_and_denied_plans_fail_without_writes_or_traffic() {
    for case in 0..13 {
        let mut f = Fixture::new();
        let u = Upstream::new(&f, false, None);
        let mut flags = PLAN.to_vec();
        match case {
            0 => fs::remove_file(f.owned.root().join("turbo.lock")).unwrap(),
            1 => fs::write(f.owned.root().join(".nvmrc"), "26.x").unwrap(),
            2 => fs::write(f.owned.root().join("turbo.lock"), "invalid").unwrap(),
            3 => fs::write(f.owned.root().join("apps/web/.npmrc"), "").unwrap(),
            4 => fs::write(f.owned.home().join(".npmrc"), "").unwrap(),
            5 => {
                f.lock["tools"]["node"]["adapter"] = json!("generic-download");
                f.save();
            }
            6 => {
                f.lock["tools"]["pnpm"]["version"] = json!("11.0.0");
                f.save();
            }
            7 => fs::write(f.owned.root().join("turbo.json"), "{}").unwrap(),
            8 => fs::write(
                f.owned.root().join("turbo.json"),
                r#"{"futureFlags":{"experimentalSetup":true},"setup":{}}"#,
            )
            .unwrap(),
            9 => flags.push("--no-lock"),
            10 => flags.extend(["--update-lock", "--no-frozen"]),
            11 => {
                let artifacts = f.lock["tools"]["node"]["installation"]["artifacts"]
                    .as_object_mut()
                    .unwrap();
                let value = artifacts.values().next().unwrap().clone();
                artifacts.clear();
                artifacts.insert("windows-x64".into(), value);
                f.save();
            }
            _ => {
                fs::write(f.owned.root().join("turbo.json"), r#"{"futureFlags":{"experimentalSetup":true,"experimentalCargoWorkspaces":true}}"#).unwrap();
                fs::write(f.owned.root().join("Cargo.toml"), "must not probe").unwrap();
            }
        }
        let before = snapshot(f.owned.root().parent().unwrap());
        let error = f.run(&u, &flags).unwrap_err();
        if case == 0 {
            assert!(
                error
                    .to_string()
                    .contains("proposed first-lock resolution plans are not implemented")
            );
        }
        if matches!(case, 9 | 10) {
            assert!(matches!(error, Error::NotImplemented));
        }
        assert_eq!(snapshot(f.owned.root().parent().unwrap()), before);
        assert_eq!(u.hits(), 0);
        assert!(!f.owned.root().join(".turbo").exists());
        f.no_execution();
    }
}

#[test]
fn plan_rejects_drift_of_original_discovery_sources_and_policy_before_reporting() {
    for case in 0..7 {
        let f = Fixture::new();
        let u = Upstream::new(&f, false, None);
        let discovery = root::Discovery::capture(&f.args(PLAN)).unwrap();
        let sources = turborepo_setup::lock::Snapshot::capture(f.owned.root()).unwrap();
        let calls = std::cell::Cell::new(0);
        let edited = std::cell::RefCell::new(None);
        let result = plan::inspect(&discovery, sources, false, || {
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
                    _ => (f.owned.home().join(".npmrc"), "".into()),
                };
                fs::write(path, contents).unwrap();
                *edited.borrow_mut() = Some(snapshot(f.owned.root().parent().unwrap()));
            }
            Ok(f.owned.policy_at(&f.owned.root().join("apps/web/src"))?)
        });
        assert!(result.is_err(), "case {case}");
        assert_eq!(calls.get(), 2);
        assert_eq!(
            snapshot(f.owned.root().parent().unwrap()),
            edited.into_inner().unwrap()
        );
        assert_eq!(u.hits(), 0);
        f.no_execution();
    }
}
