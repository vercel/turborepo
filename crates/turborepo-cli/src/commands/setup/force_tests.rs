use super::*;

const FORCE: &[&str] = &["--force", "--tools-only", "--no-frozen"];

fn assert_locked_traffic(f: &Fixture, u: &Upstream, rounds: usize) {
    let requests = u.requests();
    for path in [
        f.node_path.as_str(),
        "/pnpm/10.0.0",
        "/pnpm/-/pnpm-10.0.0.tgz",
    ] {
        assert_eq!(
            requests.iter().filter(|p| p.as_str() == path).count(),
            rounds
        );
    }
    // Exact registry metadata verifies locked bytes; no packument, Node index
    // or SHASUMS selection traffic may hide in the total.
    assert_eq!(requests.len(), 3 * rounds);
}

#[test]
fn force_reverifies_healthy_trees_without_traffic_and_restages_only_damage() {
    for flags in [
        FORCE,
        &["--force", "--tools-only", "--frozen"][..],
        &["--force", "--no-frozen"][..],
    ] {
        let mut f = Fixture::new();
        fs::write(f.owned.root().join(".nvmrc"), "24.x\n").unwrap();
        f.declare(json!({"packageManager":"pnpm@10.0.0"}));
        for name in [
            "pnpm-lock.yaml",
            "package-lock.json",
            "yarn.lock",
            "bun.lock",
        ] {
            fs::write(f.owned.root().join(name), format!("{name} sentinel\n")).unwrap();
        }
        if !flags.contains(&"--tools-only") {
            fs::write(f.owned.root().join("turbo.json"), SKIP_DEPENDENCIES).unwrap();
        }
        // Everything outside the ignored storage: what `git status` would see.
        let worktree = || {
            let storage = f.owned.root().join(".turbo");
            let mut state = super::publication_guards::state(f.owned.root());
            state.retain(|(path, _)| !path.starts_with(&storage));
            state
        };
        let tracked = worktree();
        let u = Upstream::new(&f, false, None);
        f.run(&u, FROZEN).unwrap();
        let before = f.manifest().unwrap();
        let current = Installed::open(&f.owned).current().unwrap().unwrap();
        // A healthy tree is byte-equal to a fresh install: nothing to fetch.
        f.run(&u, flags).unwrap();
        assert_locked_traffic(&f, &u, 1);
        assert_eq!(f.manifest().unwrap(), before);
        // A damaged tree is restaged from its locked artifact, alone.
        turborepo_setup::test_support::damage(&current.tree("node").join("resource"), "damaged");
        f.run(&u, flags).unwrap();
        assert_eq!(u.requests().len(), 4);
        assert_eq!(u.requests()[3], f.node_path);
        let repaired = Installed::open(&f.owned).current().unwrap().unwrap();
        assert_eq!(repaired.tools, current.tools); // No fabricated identity.
        assert_eq!(repaired.bin, current.bin);
        for (id, resource, bytes) in [
            ("node", "resource", b"node resource".as_slice()),
            ("pnpm", "dist/resource", b"pnpm resource".as_slice()),
        ] {
            assert_eq!(fs::read(repaired.tree(id).join(resource)).unwrap(), bytes);
        }
        f.run(&u, FROZEN).unwrap();
        assert_eq!(u.requests().len(), 4);
        assert_eq!(f.manifest().unwrap(), before);
        assert_eq!(worktree(), tracked);
        f.no_execution();
    }
}

#[test]
fn force_missing_stale_no_host_and_unsupported_locks_fail_before_mutation() {
    for case in 0..8 {
        let mut f = Fixture::new();
        let u = Upstream::new(&f, false, None);
        match case {
            0 => fs::remove_file(f.owned.root().join("turbo.lock")).unwrap(),
            1 => fs::write(f.owned.root().join(".nvmrc"), "26.x").unwrap(),
            2 => {
                let artifacts = f.lock["tools"]["node"]["installation"]["artifacts"]
                    .as_object_mut()
                    .unwrap();
                let artifact = artifacts.values().next().unwrap().clone();
                artifacts.clear();
                artifacts.insert("windows-x64".into(), artifact);
                f.save();
            }
            3 => {
                f.lock["tools"]["node"]["adapter"] = json!("generic-download");
                f.save();
            }
            4 => fs::write(
                f.owned.root().join("package.json"),
                r#"{"packageManager":"npm@11.0.0"}"#,
            )
            .unwrap(),
            5 => fs::write(f.owned.root().join(".node-version"), "26.0.0").unwrap(),
            6 => fs::write(f.owned.root().join("turbo.json"), "{}").unwrap(),
            _ => {
                let pin = format!("pnpm@10.0.0+sha256.{}", "0".repeat(64));
                f.declare(json!({"packageManager":pin}));
            }
        }
        let before = fs::read(f.owned.root().join("turbo.lock")).ok();
        assert!(f.run(&u, FORCE).is_err(), "case {case}");
        assert_eq!(u.hits(), 0);
        assert!(!f.owned.root().join(".turbo").exists());
        assert_eq!(fs::read(f.owned.root().join("turbo.lock")).ok(), before);
        f.no_execution();
    }
}

#[test]
fn force_late_failures_keep_the_previous_record() {
    for case in 0..6 {
        let f = Fixture::new();
        let first = Upstream::new(&f, false, None);
        f.run(&first, FROZEN).unwrap();
        let before = f.manifest().unwrap();
        let old = Installed::open(&f.owned).current().unwrap().unwrap();
        // Both trees must be restaged, so force downloads both again.
        turborepo_setup::test_support::damage(&old.tree("node").join("resource"), "damaged");
        turborepo_setup::test_support::damage(&old.tree("pnpm").join("dist/resource"), "damaged");
        let lock = fs::read(f.owned.root().join("turbo.lock")).unwrap();
        let drift = match case {
            2 => Some((f.owned.root().join(".nvmrc"), "26.0.0".into())),
            3 => Some((f.owned.root().join("turbo.lock"), format!("{} ", f.lock))),
            4 => Some((f.owned.root().join("apps/web/turbo.json"), ENABLED.into())),
            5 => Some((f.owned.root().join("turbo.json"), UNSUPPORTED_SETUP.into())),
            _ => None,
        };
        let mut corrupted = f;
        if case == 0 {
            corrupted.node = b"corrupt Node".to_vec();
        }
        let u = Upstream::new(&corrupted, case == 1, drift);
        assert!(corrupted.run(&u, FORCE).is_err(), "case {case}");
        assert_eq!(u.hits(), if case == 0 { 1 } else { 3 });
        assert_eq!(corrupted.manifest().unwrap(), before);
        let retained = Installed::open(&corrupted.owned)
            .current()
            .unwrap()
            .unwrap();
        assert_eq!(retained.bin, old.bin);
        assert_eq!(retained.tools, old.tools);
        if case != 3 {
            assert_eq!(
                fs::read(corrupted.owned.root().join("turbo.lock")).unwrap(),
                lock
            );
        }
        corrupted.no_execution();
    }
}

#[test]
fn force_modes_parse_but_unimplemented_combinations_never_mutate() {
    for flags in [
        vec!["--force", "--offline"],
        vec!["--force", "--tools-only", "--offline"],
        vec!["--force", "--tools-only", "--plan", "--no-lock"],
        vec!["--force", "--tools-only", "--no-lock"],
        vec!["--force", "--tools-only", "--no-frozen", "--update-lock"],
    ] {
        let f = Fixture::new();
        let u = Upstream::new(&f, false, None);
        let result = f.run(&u, &flags);
        if flags.contains(&"--offline") && !flags.contains(&"--tools-only") {
            // Offline dependency setup needs the fixture's missing
            // pnpm-lock.yaml before anything else.
            assert!(matches!(result, Err(Error::Dependencies(_))));
        } else if flags.contains(&"--offline") {
            // Nothing is installed, so offline force has no verified bytes.
            assert!(matches!(result, Err(Error::Offline(_))));
        } else {
            assert!(matches!(result, Err(Error::NotImplemented)));
        }
        assert_eq!(u.hits(), 0);
        assert!(!f.owned.root().join(".turbo").exists());
    }
    for flags in [
        vec!["--force", "--check"],
        vec!["--force", "--frozen", "--no-frozen"],
        vec!["--force", "--frozen", "--update-lock"],
        vec!["--force", "--frozen", "--no-lock"],
    ] {
        let words = ["turbo", "setup"].into_iter().chain(flags);
        assert!(Args::parse_args(words.map(std::ffi::OsString::from).collect()).is_err());
    }
}

#[test]
fn force_node_only_uses_locked_bytes_without_selection_traffic() {
    let mut f = Fixture::new();
    f.lock["tools"].as_object_mut().unwrap().remove("pnpm");
    f.declare(json!({}));
    let lock = fs::read(f.owned.root().join("turbo.lock")).unwrap();
    let u = Upstream::new(&f, false, None);
    f.run(&u, FORCE).unwrap();
    let before = f.manifest().unwrap();
    f.run(&u, FORCE).unwrap();
    assert_eq!(f.manifest().unwrap(), before);
    assert_eq!(u.requests(), [f.node_path.clone()]);
    assert_eq!(fs::read(f.owned.root().join("turbo.lock")).unwrap(), lock);
    assert_eq!(
        Installed::open(&f.owned)
            .current()
            .unwrap()
            .unwrap()
            .tools
            .len(),
        1
    );
    f.no_execution();
}

#[test]
fn force_keeps_authored_sha512_verification_and_binding() {
    let mut f = Fixture::new();
    let pin = format!("pnpm@10.0.0+sha512.{:x}", sha2::Sha512::digest(&f.pnpm));
    f.declare(json!({"packageManager":pin}));
    let u = Upstream::new(&f, false, None);
    f.run(&u, FROZEN).unwrap();
    f.run(&u, FORCE).unwrap();
    assert_locked_traffic(&f, &u, 1);
    let before = f.manifest().unwrap();
    let selected = Installed::open(&f.owned).current().unwrap().unwrap();
    let pin = format!("pnpm@10.0.0+sha512.{}", "0".repeat(128));
    f.declare(json!({"packageManager":pin}));
    assert!(f.run(&u, FORCE).is_err());
    // Exact registry verification only; reject before the tarball. The
    // healthy Node tree needs no download.
    assert_eq!(u.hits(), 4);
    assert_eq!(f.manifest().unwrap(), before);
    assert_eq!(
        Installed::open(&f.owned).current().unwrap().unwrap().tools,
        selected.tools
    );
    f.no_execution();
}

#[test]
fn local_write_normalization_never_authorizes_force_resolution_or_lock_writes() {
    for ci in [false, true] {
        for flags in [
            vec!["--force", "--tools-only"],
            vec!["--force", "--tools-only", "--no-frozen"],
        ] {
            let args = Args::parse_args(
                ["turbo", "setup"]
                    .into_iter()
                    .chain(flags)
                    .map(std::ffi::OsString::from)
                    .collect(),
            )
            .unwrap();
            let Some(crate::cli::Command::Setup { setup_args }) = args.command else {
                panic!()
            };
            let request = SetupRequest::new(&setup_args, ci).unwrap();
            assert!(validate_request(&request).is_ok());
            assert!(request.force && !request.update_lock);
            assert_eq!(
                request.lock,
                if ci && !setup_args.no_frozen {
                    LockMode::Frozen
                } else {
                    LockMode::Write
                }
            );
        }
    }
}
