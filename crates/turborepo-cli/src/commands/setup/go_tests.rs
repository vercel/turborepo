//! Go declarations through the actual parser → discovery → lock → provisioning
//! path, gated on `experimentalGoWorkspaces`.
use super::*;

const GO: &str = r#"{"futureFlags":{"experimentalSetup":true,"experimentalGoWorkspaces":true}}"#;
const REFRESH: &[&str] = &["--no-frozen", "--update-lock", "--tools-only"];

fn go_routes(versions: &[&str]) -> Vec<(String, Vec<u8>)> {
    // Would visibly fail if any probe ran the managed toolchain.
    let archive = archive(
        "go",
        &[
            ("bin/go", "#!/bin/sh\ntouch probe-ran\nexit 91\n"),
            ("bin/gofmt", "#!/bin/sh\nexit 91\n"),
            ("src/go.mod", "module std\n"),
        ],
    );
    let host = match provision::platform().unwrap() {
        turborepo_setup::lock::Platform::MacosArm64 => "darwin-arm64",
        turborepo_setup::lock::Platform::MacosX64 => "darwin-amd64",
        turborepo_setup::lock::Platform::LinuxX64Gnu => "linux-amd64",
        _ => "linux-arm64",
    };
    let mut routes = Vec::new();
    let mut releases = Vec::new();
    for version in versions {
        let mut files = Vec::new();
        for target in ["darwin-amd64", "darwin-arm64", "linux-amd64", "linux-arm64"] {
            let name = format!("go{version}.{target}.tar.gz");
            let sha256 = if target == host {
                routes.push((format!("/go/{name}"), archive.clone()));
                format!("{:x}", Sha256::digest(&archive))
            } else {
                "0".repeat(64)
            };
            let (os, arch) = target.split_once('-').unwrap();
            files.push(
                json!({"filename": name, "os": os, "arch": arch, "sha256": sha256,
                "version": format!("go{version}"), "size": 1, "kind": "archive"}),
            );
        }
        releases.push(json!({"version": format!("go{version}"), "stable": true, "files": files}));
    }
    let index = serde_json::to_vec(&releases).unwrap();
    routes.push((turborepo_setup::go_resolution::INDEX_PATH.into(), index));
    routes
}
fn go_only() -> Fixture {
    let f = fresh();
    let root = f.owned.root();
    fs::remove_file(root.join("package.json")).unwrap();
    fs::remove_file(root.join(".nvmrc")).unwrap();
    fs::write(root.join("turbo.json"), GO).unwrap();
    fs::write(root.join("go.work"), "go 1.22\n").unwrap();
    f
}
fn go_version(f: &Fixture) -> String {
    selected(f).tools()["go"].version.clone()
}
fn current(f: &Fixture) -> turborepo_tool_install::Current {
    let store = turborepo_tool_install::Store::open(f.owned.root()).unwrap();
    store.current().unwrap().unwrap()
}

#[test]
fn go_lock_create_repeat_refresh_update_and_frozen_drift() {
    let f = go_only();
    let root = f.owned.root();
    let u = LoopbackServer::new(go_routes(&["1.22.12", "1.23.4"]), |_| {}).unwrap();
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    assert_eq!(u.hits(), 2); // The release index, then the locked host archive.
    assert_eq!(selected(&f).tools().keys().collect::<Vec<_>>(), ["go"]);
    // The oldest supported release that satisfies the `go 1.22` minimum.
    assert_eq!(go_version(&f), "1.22.12");
    assert!(current(&f).bin.join("go").exists() && current(&f).bin.join("gofmt").exists());
    let lock = fs::read(root.join("turbo.lock")).unwrap();
    let manifest = f.manifest().unwrap();
    // Repeat and frozen setup are no-ops; an unchanged minimum never refreshes.
    let newer = LoopbackServer::new(go_routes(&["1.22.12", "1.23.4", "1.24.1"]), |_| {}).unwrap();
    for flags in [LOCAL, FROZEN] {
        assert_eq!(f.run(&newer, flags).unwrap(), 0);
    }
    assert_eq!(newer.hits(), 0);
    assert_eq!(fs::read(root.join("turbo.lock")).unwrap(), lock);
    assert_eq!(f.manifest().unwrap(), manifest);
    assert_eq!(f.run(&newer, REFRESH).unwrap(), 0);
    assert_eq!(go_version(&f), "1.23.4");
    // A changed declaration is stale in frozen mode and re-resolved locally.
    fs::write(root.join("go.work"), "go 1.22\ntoolchain go1.24.1\n").unwrap();
    let refreshed = fs::read(root.join("turbo.lock")).unwrap();
    let error = f.run(&newer, FROZEN).unwrap_err().to_string();
    assert!(error.contains("declarations changed"), "{error}");
    assert_eq!(fs::read(root.join("turbo.lock")).unwrap(), refreshed);
    assert_eq!(f.run(&newer, LOCAL).unwrap(), 0);
    assert_eq!(go_version(&f), "1.24.1");
    assert_eq!(selected(&f).tools()["go"].declarations.len(), 2);
    untouched(&f);
}

#[test]
fn go_needs_its_workspace_flag_and_preserves_unaffected_node_selections() {
    let f = fresh();
    let root = f.owned.root();
    fs::write(root.join("go.work"), "go 1.22\n").unwrap();
    let mut routes = routes(&f, "24.0.0", "10.0.0");
    routes.extend(go_routes(&["1.22.12"]));
    let u = LoopbackServer::new(routes, |_| {}).unwrap();
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    let go_requests = || {
        u.requests()
            .iter()
            .filter(|p| p.starts_with("/dl/"))
            .count()
    };
    assert_eq!(go_requests(), 0);
    let js = selected(&f);
    assert!(!js.tools().contains_key("go"));
    fs::write(root.join("turbo.json"), GO).unwrap();
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    assert_eq!(go_requests(), 1);
    let mixed = selected(&f);
    assert_eq!(mixed.tools()["go"].version, "1.22.12");
    for id in ["node", "pnpm"] {
        assert_eq!(mixed.tools()[id], js.tools()[id]);
    }
    assert_eq!(current(&f).tools.len(), 3);
    // Removing the declaration removes only the Go selection and installation.
    fs::remove_file(root.join("go.work")).unwrap();
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    assert_eq!(selected(&f).tools(), js.tools());
    assert!(!current(&f).bin.join("go").exists());
    untouched(&f);
}

#[test]
fn check_reports_go_readiness_and_fails_closed_without_the_flag_or_on_damage() {
    use turborepo_setup::activation::Error as Activation;
    const CHECK: &[&str] = &["--check", "--tools-only"];
    let f = go_only();
    let root = f.owned.root();
    let u = LoopbackServer::new(go_routes(&["1.22.12"]), |_| {}).unwrap();
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    let hits = u.hits();
    for flags in [CHECK, &["--check", "--tools-only", "--offline"]] {
        assert_eq!(f.check(flags).unwrap(), 0);
    }
    // With the workspace flag off, go.work is unread and the selection drifts.
    fs::write(root.join("turbo.json"), ENABLED).unwrap();
    let result = f.check(CHECK);
    assert!(matches!(
        result,
        Err(Error::Activation(Activation::DeclarationDrift))
    ));
    fs::write(root.join("turbo.json"), GO).unwrap();
    fs::write(current(&f).bin.join("../tools/go/src/go.mod"), "damage").unwrap();
    let result = f.check(CHECK);
    assert!(matches!(
        result,
        Err(Error::Activation(Activation::DamagedInventory(_)))
    ));
    assert_eq!(u.hits(), hits);
    untouched(&f);
}

#[test]
fn managed_run_pins_the_locked_go_toolchain_and_follows_the_workspace_flag() {
    let f = go_only();
    let u = LoopbackServer::new(go_routes(&["1.22.12"]), |_| {}).unwrap();
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    let (env, bin) = run_overrides(&f.owned).unwrap();
    assert!(bin.join("go").exists() && bin.join("gofmt").exists());
    assert_eq!(env["GOTOOLCHAIN"], "local");
    assert!(
        std::path::Path::new(&env["GOROOT"]).join("bin/go").exists(),
        "{env:?}"
    );
    // Setup ignores go.work without the flag, so run reads the selection as drift.
    fs::write(f.owned.root().join("turbo.json"), ENABLED).unwrap();
    let error = run_overrides(&f.owned).unwrap_err();
    assert!(error.contains("no longer match turbo.lock"), "{error}");
    assert!(!f.owned.root().join("probe-ran").exists());
}

#[test]
fn unsupported_go_directives_fail_before_any_writer() {
    let f = go_only();
    let root = f.owned.root();
    fs::write(root.join("go.work"), "go 1.22\ntoolchain go1.22.3-custom\n").unwrap();
    let u = LoopbackServer::new([], |_| {}).unwrap();
    let error = f.run(&u, LOCAL).unwrap_err().to_string();
    assert!(error.contains("custom goV-suffix builds"), "{error}");
    assert_eq!(u.hits(), 0);
    assert!(!root.join(".turbo").exists() && !root.join("turbo.lock").exists());
}
