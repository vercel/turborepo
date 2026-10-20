//! Locked pnpm workspace installs with the managed pnpm on the managed Node.
use super::*;

const LOCK: &[u8] = b"lockfileVersion: '9.0'\n";

/// The pnpm shim runs the managed Node with pnpm's CLI script, so this Node
/// records pnpm's arguments, then runs `body` in the workspace root.
fn installing(body: &str) -> Fixture {
    let mut f = Fixture::new();
    f.record_node("pnpm-args", body);
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
    fs::remove_dir_all(root.join("node_modules")).unwrap();
    assert_eq!(f.run(&u, &["--frozen"]).unwrap(), 0);
    assert!(root.join("node_modules/.installed").exists());
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    assert_eq!(
        args(&f),
        ["install --frozen-lockfile", "install --frozen-lockfile"]
    );
    no_tasks(&f);
}

#[test]
fn pnpm_native_settings_reach_the_installer_unchanged() {
    let f = installing("cp pnpm-workspace.yaml observed-settings");
    let root = f.owned.root();
    let u = Upstream::new(&f, false, None);
    for settings in [
        "packages:\n  - apps/*\nonlyBuiltDependencies:\n  - esbuild\n",
        "packages:\n  - apps/*\nignoreScripts: true\n",
        "packages: [malformed native settings",
    ] {
        fs::write(root.join("pnpm-workspace.yaml"), settings).unwrap();
        assert_eq!(f.run(&u, &["--frozen"]).unwrap(), 0);
        assert_eq!(
            fs::read_to_string(root.join("observed-settings")).unwrap(),
            settings
        );
        assert_eq!(
            fs::read_to_string(root.join("pnpm-workspace.yaml")).unwrap(),
            settings
        );
    }
    assert_eq!(args(&f), ["install --frozen-lockfile"; 3]);
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
