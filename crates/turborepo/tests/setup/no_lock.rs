//! Production binary qualification: no loopback/policy override, no Node
//! probes.
use turborepo_setup::{
    lock::{Lock, Platform, Snapshot},
    native_baseline::NativeRecord,
    node_provision::NodePlan,
    pnpm_provision::PnpmPlan,
    test_support::OwnedSetupFixture,
};
use turborepo_tool_install::Store;

use super::*;

#[test]
fn production_no_lock_ci_healthy_repeat_has_positive_pins_record_and_no_traffic() {
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
            fs::write(&path, "#!/bin/sh\ntouch probe-ran\nexit 99\n")?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
        }
        if tool.id == "pnpm" { fs::write(tree.join("package.json"), r#"{"name":"pnpm","version":"10.0.0","bin":{"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"}}"#)?; }
        fs::write(tree.join("resource"), "complete fixture resource")?;
        Ok(())
    }, || { sources.check_guard(&guard).map_err(io::Error::other).map_err(Into::into) }).unwrap();
    let before = store.current().unwrap().unwrap();
    drop(store);
    drop(guard);
    let monitor = TcpListener::bind("127.0.0.1:0").unwrap();
    monitor.set_nonblocking(true).unwrap();
    let proxy = format!("http://{}", monitor.local_addr().unwrap());
    let git = which::which("git").unwrap();
    let mut positive = 0;
    for existing in [false, true] {
        if existing {
            fs::write(root.join("turbo.lock"), lock.canonical_bytes().unwrap()).unwrap();
        }
        let original = snapshot(root);
        for _ in 0..2 {
            let output = Command::new(env!("CARGO_BIN_EXE_turbo"))
                .env_clear()
                .current_dir(root)
                .args(["setup", "--no-lock", "--tools-only", "--cwd=apps/web/src"])
                .env("CI", "1")
                .env("PATH", git.parent().unwrap())
                .env("HOME", root.join("home"))
                .env("TURBO_CONFIG_DIR_PATH", root.join("config"))
                .env("VERCEL_CONFIG_DIR_PATH", root.join("config"))
                .env("TURBO_TELEMETRY_DISABLED", "1")
                .env("DO_NOT_TRACK", "1")
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
                    && text.contains("activation is not enabled")
            );
            let after = Store::inspect(root).unwrap().unwrap();
            assert_eq!(after.bin, before.bin);
            assert_eq!(
                after.record.unwrap().bytes(),
                before.record.as_ref().unwrap().bytes()
            );
            assert_eq!(snapshot(root), original);
            assert_eq!(root.join("turbo.lock").exists(), existing);
            assert!(!root.join("probe-ran").exists());
            assert!(matches!(monitor.accept(), Err(e) if e.kind() == io::ErrorKind::WouldBlock));
            positive += 1;
        }
    }
    eprintln!("Production no-lock Linux qualification: {positive}/4 positive, 0 blocked");
}
