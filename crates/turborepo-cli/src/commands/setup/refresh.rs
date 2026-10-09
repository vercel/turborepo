//! Explicit parser/discovery refresh through the existing checked coordinator.
use super::*;

const REFRESH: &[&str] = &["--no-frozen", "--update-lock", "--tools-only"];
fn initialized() -> Fixture {
    let f = fresh();
    fs::write(f.owned.root().join(".nvmrc"), "24.x").unwrap();
    let u = LoopbackServer::new(routes(&f, "24.0.0", "10.0.0"), |_| {}).unwrap();
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    f
}
fn healthy_previous(f: &Fixture, manifest: &[u8]) {
    assert_eq!(f.manifest().unwrap(), manifest);
    let store = turborepo_tool_install::Store::open(f.owned.root()).unwrap();
    let current = store.current().unwrap().unwrap();
    assert_eq!(current.tools.len(), 2);
    assert_eq!(current.tools[0].version, "24.0.0");
    untouched(f);
}

#[test]
fn ordinary_float_preserves_but_refresh_selects_newer_and_repeat_is_noop() {
    let f = fresh();
    let initial_routes = routes(&f, "24.0.0", "10.0.0");
    let pnpm = &initial_routes
        .iter()
        .find(|(p, _)| p.ends_with(".tgz"))
        .unwrap()
        .1;
    let expected = format!("{:x}", Sha256::digest(pnpm));
    fs::write(f.owned.root().join(".nvmrc"), "24.x").unwrap();
    fs::write(
        f.owned.root().join("package.json"),
        json!({
            "packageManager":format!("pnpm@10.0.0+sha512.{:x}", Sha512::digest(pnpm))
        })
        .to_string(),
    )
    .unwrap();
    let initial = LoopbackServer::new(initial_routes, |_| {}).unwrap();
    f.run(&initial, LOCAL).unwrap();
    assert_eq!(initial.hits(), 7);
    let old = selected(&f);
    let before = publication_guards::state(f.owned.root());
    let empty = LoopbackServer::new([], |_| {}).unwrap();
    assert_eq!(f.run(&empty, LOCAL).unwrap(), 0);
    assert_eq!(empty.hits(), 0);
    assert_eq!(publication_guards::state(f.owned.root()), before);

    let mut r = routes(&f, "24.1.0", "10.0.0");
    let index = r.iter_mut().find(|(p, _)| p == "/dist/index.json").unwrap();
    let mut releases: Value = serde_json::from_slice(&index.1).unwrap();
    let mut older = releases[0].clone();
    older["version"] = json!("v24.0.0");
    releases.as_array_mut().unwrap().push(older);
    index.1 = serde_json::to_vec(&releases).unwrap();
    let u = LoopbackServer::new(r, |p| {
        assert!(!p.starts_with("/dist/v24.0.0/"));
        if p.starts_with("/pnpm/") {
            assert!(p == "/pnpm/10.0.0" || p == "/pnpm/-/pnpm-10.0.0.tgz");
        }
    })
    .unwrap();
    // Explicitly select non-frozen refresh so the local fixture also runs in CI.
    assert_eq!(f.run(&u, REFRESH).unwrap(), 0);
    assert_eq!(u.hits(), 7);
    let new = selected(&f);
    assert_eq!(new.tools()["node"].version, "24.1.0");
    assert_eq!(new.tools()["pnpm"], old.tools()["pnpm"]);
    let turborepo_setup::lock::Installation::Managed { artifacts } =
        &new.tools()["pnpm"].installation
    else {
        panic!()
    };
    assert_eq!(
        artifacts[&turborepo_setup::lock::Platform::Any]["package"].sha256,
        expected
    );
    assert_eq!(
        provision::lock_report(Some(&old), &new),
        "turbo.lock updated. Commit turbo.lock to share the exact managed tool selections.\nnode: \
         24.0.0 -> 24.1.0"
    );
    let lock_path = f.owned.root().join("turbo.lock");
    let inode = same_file::Handle::from_path(&lock_path).unwrap();
    let modified = fs::metadata(&lock_path).unwrap().modified().unwrap();
    let before = publication_guards::state(f.owned.root());
    assert_eq!(f.run(&u, REFRESH).unwrap(), 0);
    assert_eq!(u.hits(), 11); // Metadata + exact pnpm bytes, no provisioning downloads.
    assert_eq!(same_file::Handle::from_path(&lock_path).unwrap(), inode);
    assert_eq!(
        fs::metadata(&lock_path).unwrap().modified().unwrap(),
        modified
    );
    assert_eq!(publication_guards::state(f.owned.root()), before);
    untouched(&f);
}

#[test]
fn refresh_repairs_invalid_previous_native_selection_but_local_rejects_it() {
    for case in 0..3 {
        let f = initialized();
        let manifest = f.manifest().unwrap();
        let mut old: Value =
            serde_json::from_slice(&fs::read(f.owned.root().join("turbo.lock")).unwrap()).unwrap();
        match case {
            0 => {
                old["tools"]["node"]["version"] = json!("26.0.0");
                old["tools"]["node"]["installation"] = serde_json::from_str(
                    &old["tools"]["node"]["installation"]
                        .to_string()
                        .replace("24.0.0", "26.0.0"),
                )
                .unwrap();
            }
            1 => old["tools"]["node"]["options"] = json!({"unsupported":["true"]}),
            _ => old["tools"]["pnpm"]["version"] = json!("11.0.0"),
        }
        fs::write(f.owned.root().join("turbo.lock"), old.to_string()).unwrap();
        let before = publication_guards::state(f.owned.root());
        let empty = LoopbackServer::new([], |_| {}).unwrap();
        let error = f.run(&empty, LOCAL).unwrap_err().to_string();
        if case == 0 {
            assert!(
                error.contains("locked Node does not satisfy native declarations"),
                "{error}"
            );
        }
        assert_eq!(empty.hits(), 0);
        assert_eq!(publication_guards::state(f.owned.root()), before);
        let u = LoopbackServer::new(routes(&f, "24.1.0", "10.0.0"), |_| {}).unwrap();
        assert_eq!(f.run(&u, REFRESH).unwrap(), 0);
        assert_eq!(selected(&f).tools()["node"].version, "24.1.0");
        assert_eq!(selected(&f).tools()["pnpm"].version, "10.0.0");
        assert_ne!(f.manifest().unwrap(), manifest);
        untouched(&f);
    }
}

#[test]
fn refresh_requires_exact_pnpm_and_a_supported_previous_cohort() {
    for case in 0..4 {
        let mut f = Fixture::new();
        if case == 0 {
            f.declare(json!({"devEngines":{"packageManager":{"name":"pnpm","version":"10.x"}}}));
        } else if case == 1 {
            f.declare(json!({"packageManager":"pnpm@10.x"}));
        } else if case == 2 {
            let mut extra = f.lock["tools"]["pnpm"].clone();
            extra["adapter"] = json!("npm");
            f.lock["tools"]["npm"] = extra;
            f.save();
        } else {
            f.lock["tools"]["node"]["adapter"] = json!("generic-download");
            f.save();
        }
        let before = publication_guards::state(f.owned.root());
        let u = LoopbackServer::new([], |_| {}).unwrap();
        let error = f.run(&u, REFRESH).unwrap_err().to_string();
        if case <= 1 {
            assert!(error.contains("floating pnpm request"), "{error}");
        }
        assert_eq!(u.hits(), 0);
        assert_eq!(publication_guards::state(f.owned.root()), before);
        assert!(!f.owned.root().join(".turbo").exists());
        f.no_execution();
    }
}

#[test]
fn refresh_candidate_host_validation_precedes_any_writer() {
    let f = Fixture::new();
    fs::write(f.owned.root().join("package.json"), "{}").unwrap();
    let mut r = routes(&f, "24.0.0", "10.0.0");
    let file = if matches!(
        provision::platform().unwrap(),
        turborepo_setup::lock::Platform::LinuxX64Gnu
    ) {
        "osx-arm64-tar"
    } else {
        "linux-x64"
    };
    r.iter_mut()
        .find(|(p, _)| p == "/dist/index.json")
        .unwrap()
        .1 = json!([{"version":"v24.0.0","npm":"11.6.1","lts":false,"files":[file]}])
        .to_string()
        .into_bytes();
    let u = LoopbackServer::new(r, |_| {}).unwrap();
    let before = publication_guards::state(f.owned.root());
    assert!(f.run(&u, REFRESH).is_err());
    assert_eq!(u.hits(), 2);
    assert_eq!(publication_guards::state(f.owned.root()), before);
    assert!(!f.owned.root().join(".turbo").exists());
    f.no_execution();
}

#[test]
fn refresh_resolution_metadata_and_integrity_failures_preserve_prior_lock_and_inventory() {
    for case in 0..5 {
        let f = initialized();
        let manifest = f.manifest().unwrap();
        let mut r = routes(&f, "24.1.0", "10.0.0");
        let target = match case {
            0 => "/dist/index.json",
            1 => "/dist/v24.1.0/SHASUMS256.txt",
            2 => "/pnpm/10.0.0",
            _ => "/pnpm/-/pnpm-10.0.0.tgz",
        };
        if case == 4 {
            fs::write(
                f.owned.root().join("package.json"),
                json!({
                    "packageManager":format!("pnpm@10.0.0+sha512.{}", "0".repeat(128))
                })
                .to_string(),
            )
            .unwrap();
        } else {
            r.iter_mut().find(|(p, _)| p == target).unwrap().1 = b"corrupt".to_vec();
        }
        let before = publication_guards::state(f.owned.root());
        let u = LoopbackServer::new(r, |_| {}).unwrap();
        assert!(f.run(&u, REFRESH).is_err(), "case {case}");
        assert_eq!(u.hits(), [1, 2, 3, 4, 4][case]);
        assert_eq!(publication_guards::state(f.owned.root()), before);
        healthy_previous(&f, &manifest);
    }
}

#[test]
fn refresh_late_guards_keep_original_discovery_and_snapshot_across_publication() {
    for after in [false, true] {
        for case in 0..7 {
            let f = initialized();
            let root = f.owned.root();
            let old_root = root.with_extension("old");
            let manifest = f.manifest().unwrap();
            let lock = fs::read(root.join("turbo.lock")).unwrap();
            let u = LoopbackServer::new(routes(&f, "24.1.0", "10.0.0"), |_| {}).unwrap();
            let fired = std::cell::Cell::new(false);
            let result = dispatch(&f, &u, REFRESH, || {
                let staged = root.join(".turbo/setup-lock/staged").exists();
                let published = fs::read(root.join("turbo.lock")).unwrap() != lock;
                if (if after { !published || staged } else { !staged }) || fired.replace(true) {
                    return;
                }
                match case {
                    0 => fs::write(root.join(".nvmrc"), "26.x").unwrap(),
                    1 => fs::write(root.join("turbo.json"), "{}").unwrap(),
                    2 => fs::write(root.join("apps/web/turbo.json"), ENABLED).unwrap(),
                    3 => {
                        fs::rename(root.join(".git"), root.join(".git-old")).unwrap();
                        fs::create_dir(root.join(".git")).unwrap();
                        for name in ["config", "HEAD"] {
                            fs::copy(
                                root.join(".git-old").join(name),
                                root.join(".git").join(name),
                            )
                            .unwrap();
                        }
                    }
                    4 => fs::write(f.owned.home().join(".npmrc"), "").unwrap(),
                    5 => {
                        let mut foreign: Value =
                            serde_json::from_slice(&fs::read(root.join("turbo.lock")).unwrap())
                                .unwrap();
                        foreign["tools"]["node"]["version"] = json!("24.9.0");
                        fs::write(root.join("turbo.lock"), foreign.to_string()).unwrap();
                    }
                    _ => {
                        fs::rename(root, &old_root).unwrap();
                        fs::create_dir_all(root.join("apps/web/src")).unwrap();
                        for name in [
                            "turbo.json",
                            ".nvmrc",
                            "package.json",
                            "turbo.lock",
                            ".gitignore",
                        ] {
                            fs::copy(old_root.join(name), root.join(name)).unwrap();
                        }
                        fs::rename(old_root.join(".git"), root.join(".git")).unwrap();
                    }
                }
            });
            assert!(fired.get(), "after={after} case={case}");
            assert!(result.is_err(), "after={after} case={case}");
            assert_eq!(u.hits(), 4); // Never reach provisioning or recapture edited state.
            if case == 6 {
                assert!(!root.join(".turbo").exists());
                fs::rename(root.join(".git"), old_root.join(".git")).unwrap();
                fs::remove_dir_all(root).unwrap();
                fs::rename(&old_root, root).unwrap();
            }
            if case == 3 {
                fs::remove_dir_all(root.join(".git")).unwrap();
                fs::rename(root.join(".git-old"), root.join(".git")).unwrap();
            }
            if case == 5 {
                assert_eq!(selected(&f).tools()["node"].version, "24.9.0");
            } else if !after {
                assert_eq!(fs::read(root.join("turbo.lock")).unwrap(), lock);
            } else {
                assert_eq!(selected(&f).tools()["node"].version, "24.1.0");
            }
            healthy_previous(&f, &manifest);
        }
    }
}

#[test]
fn refresh_download_failure_after_lock_publication_keeps_prior_generation() {
    let f = initialized();
    let manifest = f.manifest().unwrap();
    let mut r = routes(&f, "24.1.0", "10.0.0");
    r.iter_mut()
        .find(|(p, _)| p.ends_with(".tar.gz"))
        .unwrap()
        .1 = b"corrupt".to_vec();
    let u = LoopbackServer::new(r, |_| {}).unwrap();
    assert!(f.run(&u, REFRESH).is_err());
    assert_eq!(u.hits(), 5);
    assert_eq!(selected(&f).tools()["node"].version, "24.1.0");
    healthy_previous(&f, &manifest);
}

#[test]
fn refresh_reports_exact_manager_changes_and_same_version_selection_changes() {
    let f = initialized();
    let old = selected(&f);
    fs::write(
        f.owned.root().join("package.json"),
        r#"{"packageManager":"pnpm@10.1.0"}"#,
    )
    .unwrap();
    let u = LoopbackServer::new(routes(&f, "24.0.0", "10.1.0"), |_| {}).unwrap();
    f.run(&u, REFRESH).unwrap();
    let new = selected(&f);
    let report = provision::lock_report(Some(&old), &new);
    assert!(report.contains("pnpm: 10.0.0 -> 10.1.0"));
    assert!(report.contains("Commit turbo.lock"));
    assert!(!report.contains("node:"));
    let mut changed: Value = serde_json::from_slice(&new.canonical_bytes().unwrap()).unwrap();
    let artifacts = changed["tools"]["node"]["installation"]["artifacts"]
        .as_object_mut()
        .unwrap();
    artifacts.values_mut().next().unwrap()["distribution"]["sha256"] = json!("a".repeat(64));
    let changed = turborepo_setup::lock::Lock::parse(changed.to_string().as_bytes()).unwrap();
    assert!(
        provision::lock_report(Some(&new), &changed).contains("node: 24.0.0 (selection changed)")
    );
    untouched(&f);
}
