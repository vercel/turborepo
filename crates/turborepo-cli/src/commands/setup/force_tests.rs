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
fn healthy_force_replaces_one_complete_generation_then_normal_reuses() {
    for flags in [FORCE, &["--force", "--tools-only", "--frozen"][..]] {
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
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .current_dir(f.owned.root())
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_INDEX_FILE")
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            output.stdout
        };
        git(&["add", "."]);
        git(&[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "Fixture",
        ]);
        let tracked = git(&["status", "--porcelain"]);
        assert!(tracked.is_empty());
        let u = Upstream::new(&f, false, None);
        f.run(&u, FROZEN).unwrap();
        let before = f.manifest().unwrap();
        let current = turborepo_tool_install::Store::inspect(f.owned.root())
            .unwrap()
            .unwrap();
        let generations = || {
            fs::read_dir(f.owned.root().join(".turbo/tools"))
                .unwrap()
                .filter(|e| {
                    e.as_ref()
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .starts_with("generation-")
                })
                .count()
        };
        assert_eq!(generations(), 1);
        f.run(&u, flags).unwrap();
        assert_locked_traffic(&f, &u, 2);
        let after = f.manifest().unwrap();
        assert_ne!(after, before);
        assert_eq!(generations(), 2);
        let replacement = turborepo_tool_install::Store::inspect(f.owned.root())
            .unwrap()
            .unwrap();
        assert_eq!(replacement.tools, current.tools); // No fabricated identity.
        assert_ne!(replacement.bin, current.bin);
        for (id, resource, bytes) in [
            ("node", "resource", b"node resource".as_slice()),
            ("pnpm", "dist/resource", b"pnpm resource".as_slice()),
        ] {
            assert_eq!(
                fs::read(replacement.bin.join(format!("../tools/{id}/{resource}"))).unwrap(),
                bytes
            );
        }
        f.run(&u, FROZEN).unwrap();
        assert_locked_traffic(&f, &u, 2);
        assert_eq!(f.manifest().unwrap(), after);
        assert_eq!(git(&["status", "--porcelain"]), tracked);
        f.no_execution();
    }
}

#[test]
fn force_missing_stale_no_host_and_unsupported_locks_fail_before_mutation() {
    for case in 0..9 {
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
            7 => fs::write(
                f.owned.root().join(".npmrc"),
                "registry=https://private.invalid",
            )
            .unwrap(),
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
fn force_late_failures_keep_the_healthy_complete_inventory() {
    for case in 0..8 {
        let f = Fixture::new();
        let first = Upstream::new(&f, false, None);
        f.run(&first, FROZEN).unwrap();
        let before = f.manifest().unwrap();
        let old = turborepo_tool_install::Store::inspect(f.owned.root())
            .unwrap()
            .unwrap();
        let lock = fs::read(f.owned.root().join("turbo.lock")).unwrap();
        let drift = match case {
            2 => Some((f.owned.root().join(".nvmrc"), "26.0.0".into())),
            3 => Some((f.owned.root().join("turbo.lock"), format!("{} ", f.lock))),
            4 => Some((f.owned.root().join("apps/web/turbo.json"), ENABLED.into())),
            5 => Some((f.owned.home().join(".npmrc"), "".into())),
            6 => Some((f.owned.root().join(".gitignore"), "".into())),
            7 => Some((f.owned.root().to_owned(), "track".into())),
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
        let retained = turborepo_tool_install::Store::inspect(corrupted.owned.root())
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
        vec!["--force"],
        vec!["--force", "--tools-only", "--offline"],
        vec!["--force", "--tools-only", "--plan"],
        vec!["--force", "--tools-only", "--no-lock"],
        vec!["--force", "--tools-only", "--no-frozen", "--update-lock"],
    ] {
        let f = Fixture::new();
        let u = Upstream::new(&f, false, None);
        assert!(matches!(f.run(&u, &flags), Err(Error::NotImplemented)));
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
    assert_ne!(f.manifest().unwrap(), before);
    assert_eq!(u.requests(), [f.node_path.clone(), f.node_path.clone()]);
    assert_eq!(fs::read(f.owned.root().join("turbo.lock")).unwrap(), lock);
    assert_eq!(
        turborepo_tool_install::Store::inspect(f.owned.root())
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
    assert_locked_traffic(&f, &u, 2);
    let before = f.manifest().unwrap();
    let selected = turborepo_tool_install::Store::inspect(f.owned.root())
        .unwrap()
        .unwrap();
    let pin = format!("pnpm@10.0.0+sha512.{}", "0".repeat(128));
    f.declare(json!({"packageManager":pin}));
    assert!(f.run(&u, FORCE).is_err());
    // Node artifact and exact registry verification only; reject before tarball.
    assert_eq!(u.hits(), 8);
    assert_eq!(f.manifest().unwrap(), before);
    assert_eq!(
        turborepo_tool_install::Store::inspect(f.owned.root())
            .unwrap()
            .unwrap()
            .tools,
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
