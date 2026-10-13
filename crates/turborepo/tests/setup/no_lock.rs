//! Production binary qualification: no loopback/policy override, no Node
//! probes.
#![allow(clippy::unwrap_used)] // Fixture failures fail the test.
use turborepo_setup::{
    lock::{Lock, Platform, Snapshot},
    native_baseline::NativeRecord,
    node_provision::NodePlan,
    pnpm_provision::PnpmPlan,
    test_support::OwnedSetupFixture,
};
use turborepo_tool_install::Store;

use super::*;

/// A Node/pnpm generation sealed with its clone-local selection, as `turbo
/// setup --no-lock` leaves it, whose executables run `script`.
fn seeded(script: &str) -> (OwnedSetupFixture, Lock, turborepo_tool_install::Current) {
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
    let mut store = Store::open(root).unwrap();
    let expected = store.generation().unwrap();
    store.reconcile_recorded_checked(&[node.inventory_tool().clone(), pnpm.inventory_tool().clone()], native.record(), &expected, false, |tool, tree| {
        for relative in tool.executables.values() {
            let path = tree.join(relative);
            fs::create_dir_all(path.parent().unwrap())?;
            fs::write(&path, script)?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
        }
        if tool.id == "pnpm" { fs::write(tree.join("package.json"), r#"{"name":"pnpm","version":"10.0.0","bin":{"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"}}"#)?; }
        fs::write(tree.join("resource"), "complete fixture resource")?;
        Ok(())
    }, || { sources.check_guard(&guard).map_err(io::Error::other).map_err(Into::into) }).unwrap();
    let before = store.current().unwrap().unwrap();
    drop(store);
    drop(guard);
    // Seeded bytes are reusable only with the user-state record setup
    // publishes for its own verified installs.
    trust(root, &f.home().join("data"), &before);
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
    let data = f.home().join("data");
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
                text.contains("node 24.0.0: reused") && text.contains("pnpm 10.0.0: reused"),
                "{text}"
            );
            assert!(
                text.contains("No turbo.lock written")
                    && text.contains("cross-machine")
                    && text.contains("Dependencies skipped")
                    && text.contains("turbo run uses these tools in this clone only")
            );
            let after = Store::inspect(root).unwrap().unwrap();
            assert_eq!(after.bin, before.bin);
            assert_eq!(
                after.record.unwrap().bytes(),
                before.record.as_ref().unwrap().bytes()
            );
            assert_eq!(snapshot(root), original);
            assert_eq!(root.join("turbo.lock").exists(), existing);
            // Without turbo.lock the record names the clone-local selection.
            let record: serde_json::Value = serde_json::from_slice(
                &fs::read(
                    fs::read_dir(data.join("turborepo/setup-roots"))
                        .unwrap()
                        .next()
                        .unwrap()
                        .unwrap()
                        .path(),
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(record.get("lock_sha256").is_some(), existing);
            assert!(record["installation"]["selection_sha256"].is_string());
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
    // The shared `invoke` helper adds a git-only `host-bin` PATH to the root on
    // first use (#14639 composition), so snapshot after it exists.
    failure(root, &words, true, "--no-lock needs a Node declaration");
    let before = snapshot(root);
    let text = failure(root, &words, true, "--no-lock needs a Node declaration");
    assert!(
        diagnostic_contains(&text, "Declare Node in .nvmrc"),
        "{text}"
    );
    assert!(
        !diagnostic_contains(&text, "native resolution record"),
        "{text}"
    );
    assert_eq!(snapshot(root), before);
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
    // Strict like a lock: drift names the clone-local fix, never a commit.
    write(f.root(), ".nvmrc", "26.x");
    let output = turbo(&f, "apps/web/src", &exec).output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(stderr.contains("run turbo setup --no-lock"), "{stderr}");
    assert!(!stderr.contains("commit"), "{stderr}");
    assert!(output.stdout.is_empty());
}

/// The record `turbo setup --no-lock` publishes for `current` without
/// turbo.lock, in the private user-state layout.
fn trust(root: &Path, data: &Path, current: &turborepo_tool_install::Current) {
    use std::os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, PermissionsExt},
    };

    use sha2::Digest;
    let sha = |bytes: &[u8]| format!("{:x}", sha2::Sha256::digest(bytes));
    let root = root.canonicalize().unwrap();
    let identity = fs::metadata(&root).unwrap();
    let manifest = fs::read(root.join(".turbo/tools/manifest.json")).unwrap();
    let record = serde_json::json!({"schema": 1, "installation": {
        "root": root, "device": identity.dev(), "inode": identity.ino(),
        "manifest_sha256": sha(&manifest), "bin": current.bin, "tools": current.tools,
        "selection_sha256": sha(current.record.as_ref().unwrap().bytes()),
    }});
    let records = data.join("turborepo/setup-roots");
    fs::create_dir_all(&records).unwrap();
    for dir in [data.to_owned(), data.join("turborepo"), records.clone()] {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let path = records.join(format!("{}.json", sha(root.as_os_str().as_bytes())));
    fs::write(&path, record.to_string()).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
}
