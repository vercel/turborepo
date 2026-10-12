//! Locked pnpm workspace installs with the managed pnpm on the managed Node.
use super::*;

const LOCK: &[u8] = b"lockfileVersion: '9.0'\n";

/// The pnpm shim runs the managed Node with pnpm's CLI script, so this Node
/// records pnpm's arguments, then runs `body` in the workspace root.
fn installing(body: &str) -> Fixture {
    let mut f = Fixture::new();
    let prefix = f.node_path.rsplit('/').next().unwrap();
    let prefix = prefix.trim_end_matches(".tar.gz").to_owned();
    let node = format!("#!/bin/sh\nshift\necho \"$*\" >> pnpm-args\n{body}\n");
    f.node = archive(&prefix, &[("bin/node", &node), ("resource", "node")]);
    let artifacts = &mut f.lock["tools"]["node"]["installation"]["artifacts"];
    let parts = artifacts
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap();
    parts["distribution"]["sha256"] = json!(format!("{:x}", Sha256::digest(&f.node)));
    f.save();
    let root = f.owned.root();
    fs::write(root.join("pnpm-workspace.yaml"), "packages:\n  - apps/*\n").unwrap();
    fs::write(root.join("pnpm-lock.yaml"), LOCK).unwrap();
    f
}

fn args(f: &Fixture) -> Vec<String> {
    let args = fs::read_to_string(f.owned.root().join("pnpm-args")).unwrap_or_default();
    args.lines().map(str::to_owned).collect()
}

fn no_tasks(f: &Fixture) {
    for path in ["task-ran", "hook-ran", "probe-ran"] {
        assert!(!f.owned.root().join(path).exists(), "{path}");
    }
    assert_eq!(
        fs::read(f.owned.root().join("pnpm-lock.yaml")).unwrap(),
        LOCK
    );
}

#[test]
fn locked_pnpm_workspaces_install_frozen_and_repeat_safely() {
    let f = installing("mkdir -p node_modules && touch node_modules/.installed");
    let root = f.owned.root();
    let u = Upstream::new(&f, false, None);
    assert_eq!(f.run(&u, &["--frozen"]).unwrap(), 0);
    assert!(root.join("node_modules/.installed").exists());
    // Every run reinstalls, so removed dependencies are repaired; skipped
    // scripts use pnpm's own flag, and tools-only setup never runs pnpm.
    fs::remove_dir_all(root.join("node_modules")).unwrap();
    assert_eq!(f.run(&u, &["--frozen", "--ignore-scripts"]).unwrap(), 0);
    assert!(root.join("node_modules/.installed").exists());
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    assert_eq!(
        args(&f),
        [
            "install --frozen-lockfile",
            "install --frozen-lockfile --ignore-scripts"
        ]
    );
    no_tasks(&f);
}

#[test]
fn pnpm_native_build_policy_is_reported_and_never_broadened() {
    let f = installing("");
    let lock = turborepo_setup::lock::Lock::parse(&f.lock.to_string().into_bytes()).unwrap();
    let plan = |f: &Fixture| {
        f.dependencies(&["--plan"])
            .plan(f.owned.root(), &lock)
            .unwrap()
    };
    assert_eq!(
        plan(&f)[..2],
        [
            "javascript dependencies: pnpm install --frozen-lockfile from pnpm-lock.yaml with \
             managed pnpm 10.0.0 (pnpm-lock.yaml stays unchanged)",
            "  pnpm 10.0.0: native pnpm policy; project scripts run; dependency scripts run only \
             for allowed packages",
        ]
    );
    // Committed allowlists and blocks stay pnpm's to enforce: setup reports
    // them and passes no flag of its own.
    let workspace = "packages:\n  - apps/*\nonlyBuiltDependencies:\n  - esbuild\n";
    fs::write(f.owned.root().join("pnpm-workspace.yaml"), workspace).unwrap();
    assert!(plan(&f)[1].ends_with("(committed: onlyBuiltDependencies in pnpm-workspace.yaml)"));
    let blocked = "packages:\n  - apps/*\nignoreScripts: true\n";
    fs::write(f.owned.root().join("pnpm-workspace.yaml"), blocked).unwrap();
    assert!(plan(&f)[1].contains("project scripts do not run; dependency scripts do not run"));
    let u = Upstream::new(&f, false, None);
    assert_eq!(f.run(&u, &["--frozen"]).unwrap(), 0);
    assert_eq!(args(&f), ["install --frozen-lockfile"]);
    no_tasks(&f);
}

#[test]
fn pnpm_lock_mismatch_and_missing_lock_fail_readably_and_recover() {
    // pnpm rejects a lock that no longer matches; its status passes through.
    let f = installing("[ -f stale ] && exit 1\nmkdir -p node_modules");
    let u = Upstream::new(&f, false, None);
    fs::write(f.owned.root().join("stale"), "").unwrap();
    assert_eq!(f.run(&u, &["--frozen"]).unwrap(), 1);
    fs::remove_file(f.owned.root().join("stale")).unwrap();
    assert_eq!(f.run(&u, &["--frozen"]).unwrap(), 0);
    no_tasks(&f);

    let f = installing("");
    fs::remove_file(f.owned.root().join("pnpm-lock.yaml")).unwrap();
    let u = Upstream::new(&f, false, None);
    for flags in [&["--frozen"][..], &["--plan"], &["--check"]] {
        let error = f.run(&u, flags).unwrap_err().to_string();
        assert!(
            error.contains("pnpm dependency setup requires a committed pnpm-lock.yaml"),
            "{error}"
        );
    }
    assert_eq!(u.hits(), 0);
    assert!(!f.owned.root().join(".turbo").exists());
    assert!(!f.owned.root().join("pnpm-lock.yaml").exists());
}
