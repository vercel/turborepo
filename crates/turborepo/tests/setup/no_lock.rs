//! Production binary qualification: no loopback/policy override, no Node
//! probes.
#![allow(clippy::unwrap_used)] // Fixture failures fail the test.
use turborepo_cli::test_support::RecordedInstallation;
use turborepo_setup::{
    lock::{Lock, Platform, Snapshot},
    native_baseline::NativeRecord,
    node_provision::NodePlan,
    pnpm_provision::PnpmPlan,
    test_support::OwnedSetupFixture,
};

use super::*;

/// The fixture's tool store, where `turbo` below locates it.
fn store(f: &OwnedSetupFixture) -> PathBuf {
    crate::common::setup::user_tool_store(f.home(), Some(&f.home().join("data")))
}

/// Node/pnpm installed and recorded with their clone-local selection, as
/// `turbo setup --no-lock` leaves them, whose executables run `script`.
fn seeded(script: &str) -> (OwnedSetupFixture, Lock, RecordedInstallation) {
    use std::os::unix::fs::PermissionsExt;
    let f = OwnedSetupFixture::new().unwrap();
    f.init_git().unwrap();
    let root = f.root();
    write(root, "turbo.json", ENABLED);
    write(root, ".nvmrc", "24.x");
    write(root, "package.json", r#"{"packageManager":"pnpm@10.0.0"}"#);
    fs::create_dir_all(root.join("apps/web/src")).unwrap();
    let (platform, selector, spelling) = if cfg!(target_arch = "aarch64") {
        (Platform::LinuxArm64Gnu, "linux-arm64-gnu", "linux-arm64")
    } else {
        (Platform::LinuxX64Gnu, "linux-x64-gnu", "linux-x64")
    };
    let prefix = format!("node-v24.0.0-{spelling}");
    let lock = Lock::parse(serde_json::json!({"schemaVersion":0,"tools":{
        "node":{"adapter":"node","version":"24.0.0","declarations":[{"file":".nvmrc","request":"24.x"}],
            "installation":{"kind":"managed","artifacts":{selector:{"distribution":{
                "url":format!("https://nodejs.org/dist/v24.0.0/{prefix}.tar.gz"),"sha256":"0".repeat(64),
                "format":"tar-gz","rootPrefix":prefix,"executables":{"node":"bin/node"}}}}}},
        "pnpm":{"adapter":"pnpm","version":"10.0.0","declarations":[{"file":"package.json","field":"/packageManager","request":"pnpm@10.0.0"}],
            "installation":{"kind":"managed","artifacts":{"any":{"package":{
                "url":"https://registry.npmjs.org/pnpm/-/pnpm-10.0.0.tgz","sha256":"1".repeat(64),
                "format":"tar-gz","rootPrefix":"package","executables":{"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"}}}}}}
    }}).to_string().as_bytes()).unwrap();
    let sources = Snapshot::capture(root).unwrap();
    let native = NativeRecord::from_snapshot(&sources, &lock).unwrap();
    let node = NodePlan::from_lock(&lock, platform).unwrap();
    let pnpm = PnpmPlan::from_declaration(
        &lock,
        platform,
        &node,
        &sources.package_manager().unwrap().unwrap(),
    )
    .unwrap();
    let guard = sources.guard().unwrap();
    let before = crate::common::setup::install_tools(
        root,
        &store(&f),
        vec![node.inventory_tool().clone(), pnpm.inventory_tool().clone()],
        Some(native.bytes()),
        |tool, tree| {
            for relative in tool.tree_files() {
                let path = tree.join(relative);
                fs::create_dir_all(path.parent().unwrap())?;
                fs::write(&path, script)?;
                fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
            }
            if tool.id == "pnpm" {
                fs::write(
                    tree.join("package.json"),
                    r#"{"name":"pnpm","version":"10.0.0","bin":{"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"}}"#,
                )?;
            }
            fs::write(tree.join("resource"), "complete fixture resource")?;
            Ok(())
        },
    );
    drop(guard);
    (f, lock, before)
}

fn turbo(f: &OwnedSetupFixture, cwd: &str, words: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_turbo"));
    command
        .env_clear()
        .current_dir(f.root().join(cwd))
        .args(words)
        .env("PATH", which::which("git").unwrap().parent().unwrap())
        .env("HOME", f.home())
        .env("XDG_DATA_HOME", f.home().join("data"))
        .env("TURBO_CONFIG_DIR_PATH", f.root().join("config"))
        .env("VERCEL_CONFIG_DIR_PATH", f.root().join("config"))
        .env("TURBO_TELEMETRY_DISABLED", "1")
        .env("DO_NOT_TRACK", "1");
    command
}

#[test]
fn production_no_lock_ci_healthy_repeat_has_positive_pins_record_and_no_traffic() {
    let (f, lock, before) = seeded("#!/bin/sh\ntouch probe-ran\nexit 99\n");
    let root = f.root();
    let monitor = TcpListener::bind("127.0.0.1:0").unwrap();
    monitor.set_nonblocking(true).unwrap();
    let proxy = format!("http://{}", monitor.local_addr().unwrap());
    let mut positive = 0;
    for existing in [false, true] {
        if existing {
            fs::write(root.join("turbo.lock"), lock.canonical_bytes().unwrap()).unwrap();
        }
        let original = snapshot(root);
        for _ in 0..2 {
            let words = ["setup", "--no-lock", "--tools-only", "--cwd=apps/web/src"];
            let output = turbo(&f, ".", &words)
                .env("CI", "1")
                .env("HTTP_PROXY", &proxy)
                .env("HTTPS_PROXY", &proxy)
                .env("ALL_PROXY", &proxy)
                .env("NO_PROXY", "")
                .output()
                .unwrap();
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                output.status.success(),
                "production positive required, not a policy bypass: {text}"
            );
            assert!(
                super::diagnostic_contains(&text, "node 24.0.0 reused")
                    && super::diagnostic_contains(&text, "pnpm 10.0.0 reused"),
                "{text}"
            );
            assert!(
                text.contains("No turbo.lock written")
                    && text.contains("cross-machine")
                    && text.contains("Dependencies skipped")
                    && !text.contains("No tasks run")
                    && text.contains("turbo run uses these tools in this clone only")
            );
            let after = crate::common::setup::installed(root, &store(&f)).unwrap();
            assert_eq!(after.bin, before.bin);
            assert_eq!(after.selection, before.selection);
            assert_eq!(snapshot(root), original);
            assert_eq!(root.join("turbo.lock").exists(), existing);
            // Without turbo.lock the record holds the clone-local selection;
            // with one, the lock's digest instead.
            let record: serde_json::Value = serde_json::from_slice(&after.record).unwrap();
            assert_eq!(record.get("lock_sha256").is_some(), existing);
            assert_eq!(record.get("selection").is_some(), !existing);
            assert!(!root.join("probe-ran").exists());
            assert!(matches!(monitor.accept(), Err(e) if e.kind() == io::ErrorKind::WouldBlock));
            positive += 1;
        }
    }
    eprintln!("Production no-lock Linux qualification: {positive}/4 positive, 0 blocked");
}

#[test]
fn production_no_lock_without_node_names_the_missing_declaration() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "turbo.json", ENABLED);
    write(root, "package.json", r#"{"packageManager":"pnpm@10.0.0"}"#);
    let words = ["setup", "--no-lock", "--tools-only"];
    // The helper prepares its git-only PATH before snapshotting the fixture,
    // then asserts the command leaves every file and directory unchanged.
    let text = failure(root, &words, true, "--no-lock needs a Node declaration");
    assert!(
        diagnostic_contains(&text, "Declare Node in .nvmrc"),
        "{text}"
    );
    assert!(
        !diagnostic_contains(&text, "native resolution record"),
        "{text}"
    );
    assert!(!root.join(".turbo").exists());
}

#[test]
fn production_exec_uses_the_clone_local_selection_and_drift_runs_nothing() {
    let (f, _, _) = seeded("#!/bin/sh\necho \"clone-local $*\"\n");
    let exec = ["exec", "--", "node", "--version"];
    let output = turbo(&f, "apps/web/src", &exec).output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "clone-local --version\n"
    );
    // This build as a repository-local turbo activates clone-local
    // selections, so exec hands off to it and keeps the selection.
    let package = f
        .root()
        .join("node_modules/@turbo")
        .join(turborepo_shim::TurboState::scoped_platform_package_dir());
    fs::create_dir_all(package.join("bin")).unwrap();
    let version = include_str!("../../../../version.txt")
        .lines()
        .next()
        .unwrap();
    fs::write(
        package.join("package.json"),
        format!(r#"{{"version":"{version}"}}"#),
    )
    .unwrap();
    let (turbo_bin, pin) = (env!("CARGO_BIN_EXE_turbo"), package.join("bin/turbo"));
    fs::hard_link(turbo_bin, &pin)
        .or_else(|_| fs::copy(turbo_bin, &pin).map(drop))
        .unwrap();
    let verbose = ["--verbosity=2", "exec", "--", "node", "--version"];
    let output = turbo(&f, "apps/web/src", &verbose).output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stderr.contains("Running local turbo binary in"),
        "{stderr}"
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "clone-local --version\n"
    );
    // Strict like a lock: drift names the clone-local fix, never a commit.
    write(f.root(), ".nvmrc", "26.x");
    let output = turbo(&f, "apps/web/src", &exec).output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(stderr.contains("run turbo setup --no-lock"), "{stderr}");
    assert!(!stderr.contains("commit"), "{stderr}");
    assert!(output.stdout.is_empty());
}
