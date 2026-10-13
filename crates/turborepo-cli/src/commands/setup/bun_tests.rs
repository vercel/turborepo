//! Locked Bun provisioning through `turbo setup`, without a global Bun.
use turborepo_setup::{lock::Platform, test_support::stored_zip};

use super::*;

const RELEASE: &str = "/oven-sh/bun/releases/download/bun-v1.2.23";

/// A canonical Bun lock for this host and its loopback release.
fn bun() -> (Fixture, LoopbackServer) {
    bun_with("#!/bin/sh\nexit 93\n")
}

/// `bun()` whose managed Bun binary is `script`.
fn bun_with(script: &str) -> (Fixture, LoopbackServer) {
    let mut f = Fixture::new();
    let platform = provision::platform().unwrap();
    let target = match platform {
        Platform::MacosArm64 => "darwin-aarch64",
        Platform::MacosX64 => "darwin-x64-baseline",
        Platform::LinuxX64Gnu => "linux-x64-baseline",
        _ => "linux-aarch64",
    };
    // `bun()`'s exits 93 when run: proves the shims execute this binary.
    let archive = stored_zip(&format!("bun-{target}/bun"), script.as_bytes());
    let path = format!("{RELEASE}/bun-{target}.zip");
    let key = serde_json::to_value(platform).unwrap();
    let key = key.as_str().unwrap();
    let tools = f.lock["tools"].as_object_mut().unwrap();
    tools.remove("pnpm");
    tools.insert(
        "bun".into(),
        json!({"adapter":"bun","version":"1.2.23","declarations":[],
            "installation":{"kind":"managed","artifacts":{key:{"distribution":{
                "url":format!("https://github.com{path}"),"sha256":format!("{:x}", Sha256::digest(&archive)),
                "format":"zip","rootPrefix":format!("bun-{target}"),"executables":{"bun":"bun","bunx":"bun"}
            }}}}}),
    );
    f.declare(json!({"packageManager":"bun@1.2.23"}));
    let lock_path = f.owned.root().join("turbo.lock");
    let canonical = turborepo_setup::lock::Lock::parse(&fs::read(&lock_path).unwrap()).unwrap();
    fs::write(&lock_path, canonical.canonical_bytes().unwrap()).unwrap();
    let u = LoopbackServer::new(
        [(f.node_path.clone(), f.node.clone()), (path, archive)],
        |_| {},
    )
    .unwrap();
    (f, u)
}

#[test]
fn locked_bun_provisions_checks_repeats_and_rejects_frozen_drift() {
    let (f, u) = bun();
    let lock_path = f.owned.root().join("turbo.lock");
    let lock = fs::read(&lock_path).unwrap();
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    assert_eq!(u.hits(), 2);
    let store = turborepo_tool_install::Store::open(f.owned.root()).unwrap();
    let current = store.current().unwrap().unwrap();
    let selected: Vec<_> = current.tools.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(selected, ["bun", "node"]);
    let scratch = tempfile::tempdir().unwrap();
    for name in ["bun", "bunx"] {
        let output = std::process::Command::new(current.bin.join(name))
            .env("PATH", scratch.path())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(93));
    }
    drop(store);
    let manifest = f.manifest().unwrap();
    for flags in [
        FROZEN,
        &["--tools-only", "--no-frozen"],
        &["--check", "--tools-only"],
    ] {
        assert_eq!(f.run(&u, flags).unwrap(), 0);
    }
    fs::write(
        f.owned.root().join("package.json"),
        json!({"packageManager":"bun@1.2.24"}).to_string(),
    )
    .unwrap();
    assert!(f.run(&u, FROZEN).is_err());
    assert_eq!(u.hits(), 2);
    assert_eq!(f.manifest().unwrap(), manifest);
    assert_eq!(fs::read(&lock_path).unwrap(), lock);
    f.no_execution();
}

#[test]
fn bun_check_failures_are_readonly_and_actionable() {
    let installed = || {
        let (f, u) = bun();
        assert!(matches!(
            f.check(super::checks::CHECK),
            Err(Error::Activation(
                turborepo_setup::activation::Error::MissingInventory
            ))
        ));
        (f, u)
    };
    super::checks::assert_manager_check_failures(installed, "bun", "yarn@4.5.0");
}

/// A managed Bun that records its arguments, then runs `body`.
fn installing(body: &str) -> (Fixture, LoopbackServer) {
    bun_with(&format!("#!/bin/sh\necho \"$*\" >> bun-args\n{body}\n"))
}

fn bun_args(f: &Fixture) -> Vec<String> {
    let args = fs::read_to_string(f.owned.root().join("bun-args")).unwrap_or_default();
    args.lines().map(str::to_owned).collect()
}

#[test]
fn locked_bun_dependencies_install_frozen_and_repeat_safely() {
    let (f, u) = installing("mkdir -p node_modules");
    let root = f.owned.root();
    fs::write(root.join("bun.lock"), "{}\n").unwrap();
    assert_eq!(f.run(&u, &["--frozen"]).unwrap(), 0);
    // A repeat reinstalls removed dependencies, and Bun's own flag skips
    // scripts; tools-only setup never runs Bun.
    fs::remove_dir_all(root.join("node_modules")).unwrap();
    assert_eq!(f.run(&u, &["--frozen", "--ignore-scripts"]).unwrap(), 0);
    assert!(root.join("node_modules").exists());
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    assert_eq!(
        bun_args(&f),
        [
            "install --frozen-lockfile",
            "install --frozen-lockfile --ignore-scripts"
        ]
    );
    assert_eq!(fs::read(root.join("bun.lock")).unwrap(), b"{}\n");
    let lock = fs::read(root.join("turbo.lock")).unwrap();
    let lock = turborepo_setup::lock::Lock::parse(&lock).unwrap();
    let plan = f.dependencies(&["--plan"]).plan(root, &lock).unwrap();
    assert_eq!(
        plan[1],
        "  bun 1.2.23: native bun policy; project scripts run; dependency scripts run only for \
         trusted packages"
    );
    assert!(!root.join("task-ran").exists() && !root.join("hook-ran").exists());
}

#[test]
fn bun_without_an_offline_mode_is_refused_before_offline_setup_changes_anything() {
    let (f, u) = installing("mkdir -p node_modules");
    let root = f.owned.root();
    fs::write(root.join("bun.lock"), "{}\n").unwrap();
    assert_eq!(f.run(&u, &["--frozen"]).unwrap(), 0);
    let (hits, manifest) = (u.hits(), f.manifest());
    let error = f.run(&u, &["--frozen", "--offline"]).unwrap_err();
    let error = error.to_string();
    assert!(
        error.contains("javascript: Bun 1.2.23 has no offline install mode")
            && error.contains("needs Bun 1.4.1 or newer")
            && error.contains("nothing was installed"),
        "{error}"
    );
    assert_eq!(bun_args(&f), ["install --frozen-lockfile"]);
    assert_eq!((u.hits(), f.manifest()), (hits, manifest));
    // Plans report it; tools-only offline setup is unaffected.
    let lock = turborepo_setup::lock::Lock::parse(&fs::read(root.join("turbo.lock")).unwrap());
    let plan = f.dependencies(&["--plan", "--offline"]);
    assert_eq!(
        plan.plan(root, &lock.unwrap()).unwrap(),
        [
            "javascript dependencies: cannot install offline: Bun 1.2.23 has no offline install \
             mode (`bun install --offline` needs Bun 1.4.1 or newer)"
        ]
    );
    assert_eq!(f.run(&u, &["--offline", "--tools-only"]).unwrap(), 0);
}

#[test]
fn bun_failures_missing_locks_and_lock_migrations_are_actionable() {
    // Bun's refusal of a stale lock passes its exit status through.
    let (f, u) = installing("[ -f stale ] && exit 1\ntrue");
    fs::write(f.owned.root().join("bun.lockb"), "binary").unwrap();
    fs::write(f.owned.root().join("stale"), "").unwrap();
    assert_eq!(f.run(&u, &["--frozen"]).unwrap(), 1);
    fs::remove_file(f.owned.root().join("stale")).unwrap();
    assert_eq!(f.run(&u, &["--frozen"]).unwrap(), 0);

    // A text lock written beside the committed binary one is removed.
    let (f, u) = installing("echo '{}' > bun.lock");
    fs::write(f.owned.root().join("bun.lockb"), "binary").unwrap();
    let error = f.run(&u, &["--frozen"]).unwrap_err().to_string();
    assert!(error.contains("changed the committed bun.lock"), "{error}");
    assert!(!f.owned.root().join("bun.lock").exists());

    let (f, u) = bun();
    let error = f.run(&u, &["--frozen"]).unwrap_err().to_string();
    assert!(
        error.contains("bun dependency setup requires a committed bun.lock or bun.lockb"),
        "{error}"
    );
    assert_eq!(u.hits(), 0);
}
