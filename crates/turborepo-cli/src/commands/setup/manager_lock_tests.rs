//! Local Yarn/Bun lock creation, targeted update, removal, refresh and frozen
//! drift, through the same writer and executor as Node/npm/pnpm.
use turborepo_setup::test_support::stored_zip;

use super::*;

const UPDATE: &[&str] = &["--no-frozen", "--update-lock", "--tools-only"];
type Routes = Vec<(String, Vec<u8>)>;

fn yarn_routes(version: &str) -> Routes {
    let package = if version.starts_with("1.") {
        "yarn"
    } else {
        "@yarnpkg/cli-dist"
    };
    let tarball = format!(
        "/{package}/-/{}-{version}.tgz",
        package.rsplit('/').next().unwrap()
    );
    let manifest = json!({"name":package,"version":version,
        "bin":{"yarn":"./bin/yarn.js","yarnpkg":"./bin/yarn.js"}});
    let bytes = archive(
        "package",
        &[
            ("package.json", &manifest.to_string()),
            ("bin/yarn.js", "throw new Error('never execute');"),
        ],
    );
    let metadata = json!({"name":package,"version":version,"dist":{
        "tarball":format!("https://registry.npmjs.org{tarball}"),
        "integrity":format!("sha512-{}", base64(&Sha512::digest(&bytes)))}});
    vec![
        (
            format!("/{package}/{version}"),
            metadata.to_string().into_bytes(),
        ),
        (tarball, bytes),
    ]
}

fn bun_routes(version: &str) -> Routes {
    let release = format!("/oven-sh/bun/releases/download/bun-v{version}");
    let mut sums = String::new();
    let mut routes = Vec::new();
    for target in [
        "darwin-x64-baseline",
        "darwin-aarch64",
        "linux-x64-baseline",
        "linux-aarch64",
        "windows-x64-baseline",
    ] {
        let binary = if target.starts_with("windows") {
            "bun.exe"
        } else {
            "bun"
        };
        let zip = stored_zip(&format!("bun-{target}/{binary}"), b"#!/bin/sh\nexit 93\n");
        sums.push_str(&format!("{:x}  bun-{target}.zip\n", Sha256::digest(&zip)));
        routes.push((format!("{release}/bun-{target}.zip"), zip));
    }
    routes.push((format!("{release}/SHASUMS256.txt"), sums.into_bytes()));
    routes
}

/// Create, repeat, frozen drift, targeted update, refresh, then removal.
fn journey(id: &str, first: &str, second: &str, routes: fn(&str) -> Routes) {
    let f = fresh();
    let declare = |version: &str| {
        let manifest = json!({ "packageManager": format!("{id}@{version}") });
        fs::write(f.owned.root().join("package.json"), manifest.to_string()).unwrap();
    };
    declare(first);
    let mut all = super::routes(&f, "24.0.0", "10.0.0");
    all.extend(routes(first));
    let u = LoopbackServer::new(all, |_| {}).unwrap();
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    let created = selected(&f);
    assert_eq!(created.tools()[id].version, first);
    assert_eq!(created.tools().len(), 2);
    let lock = fs::read(f.owned.root().join("turbo.lock")).unwrap();
    let empty = LoopbackServer::new([], |_| {}).unwrap();
    assert_eq!(f.run(&empty, LOCAL).unwrap(), 0);
    declare(second);
    assert!(f.run(&empty, FROZEN).is_err());
    assert_eq!(empty.hits(), 0);
    assert_eq!(fs::read(f.owned.root().join("turbo.lock")).unwrap(), lock);
    // Only the changed manager resolves; Node keeps its exact selection.
    let update =
        LoopbackServer::new(routes(second), |p| assert!(!p.starts_with("/dist/"))).unwrap();
    assert_eq!(f.run(&update, LOCAL).unwrap(), 0);
    let updated = selected(&f);
    assert_eq!(updated.tools()[id].version, second);
    assert_eq!(updated.tools()["node"], created.tools()["node"]);
    // Explicit refresh re-resolves through the same adapter path.
    let mut all = super::routes(&f, "24.0.0", "10.0.0");
    all.extend(routes(second));
    let refresh = LoopbackServer::new(all, |_| {}).unwrap();
    assert_eq!(f.run(&refresh, UPDATE).unwrap(), 0);
    assert_eq!(selected(&f), updated);
    fs::write(f.owned.root().join("package.json"), "{}").unwrap();
    assert_eq!(f.run(&empty, LOCAL).unwrap(), 0);
    assert_eq!(selected(&f).tools().len(), 1);
    assert_eq!(empty.hits(), 0);
    untouched(&f);
}

#[test]
fn yarn_classic_lock_lifecycle() {
    journey("yarn", "1.22.21", "1.22.22", yarn_routes);
}

#[test]
fn modern_yarn_lock_lifecycle() {
    journey("yarn", "4.5.0", "4.6.0", yarn_routes);
}

#[test]
fn bun_lock_lifecycle_and_preserved_variants_are_validated() {
    journey("bun", "1.2.22", "1.2.23", bun_routes);
    let f = fresh();
    fs::write(
        f.owned.root().join("package.json"),
        json!({"packageManager":"bun@1.2.23"}).to_string(),
    )
    .unwrap();
    let mut all = super::routes(&f, "24.0.0", "10.0.0");
    all.extend(bun_routes("1.2.23"));
    let u = LoopbackServer::new(all, |_| {}).unwrap();
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    // A tampered non-host variant fails before any resolver traffic.
    let path = f.owned.root().join("turbo.lock");
    let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["tools"]["bun"]["installation"]["artifacts"]["windows-x64"]["distribution"]["url"] =
        json!("https://github.com/oven-sh/bun/releases/download/bun-v1.2.23/bun-windows-x64.zip");
    fs::write(&path, value.to_string()).unwrap();
    fs::write(f.owned.root().join(".nvmrc"), "24.1.0").unwrap();
    let empty = LoopbackServer::new([], |_| {}).unwrap();
    assert!(f.run(&empty, LOCAL).is_err());
    assert_eq!(empty.hits(), 0);
    assert_eq!(fs::read(&path).unwrap(), value.to_string().into_bytes());
}

/// Bun runs scripts itself, so a Bun repository needs no Node declaration.
#[test]
fn bun_only_repositories_set_up_without_node() {
    let f = fresh();
    fs::remove_file(f.owned.root().join(".nvmrc")).unwrap();
    fs::write(
        f.owned.root().join("package.json"),
        json!({"packageManager":"bun@1.2.23"}).to_string(),
    )
    .unwrap();
    let u = LoopbackServer::new(bun_routes("1.2.23"), |_| {}).unwrap();
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    let created = selected(&f);
    assert_eq!(created.tools().keys().collect::<Vec<_>>(), ["bun"]);
    let store = turborepo_tool_install::Store::open(f.owned.root()).unwrap();
    let current = store.current().unwrap().unwrap();
    assert_eq!(current.tools.len(), 1);
    assert!(current.bin.join("bunx").exists());
    assert!(!current.bin.join("node").exists());
    drop(store);
    let empty = LoopbackServer::new([], |_| {}).unwrap();
    for flags in [LOCAL, FROZEN, &["--check", "--tools-only"]] {
        assert_eq!(f.run(&empty, flags).unwrap(), 0);
    }
    assert_eq!(empty.hits(), 0);
    // Declaring Node adds only its entry, and removing it drops only that.
    fs::write(f.owned.root().join(".nvmrc"), "24.x").unwrap();
    assert!(f.run(&empty, FROZEN).is_err());
    let node = LoopbackServer::new(super::routes(&f, "24.0.0", "10.0.0"), |p| {
        assert!(!p.starts_with("/oven-sh/"))
    })
    .unwrap();
    assert_eq!(f.run(&node, LOCAL).unwrap(), 0);
    assert_eq!(selected(&f).tools().len(), 2);
    assert_eq!(selected(&f).tools()["bun"], created.tools()["bun"]);
    fs::remove_file(f.owned.root().join(".nvmrc")).unwrap();
    assert_eq!(f.run(&empty, LOCAL).unwrap(), 0);
    assert_eq!(selected(&f), created);
    assert_eq!(empty.hits(), 0);
    untouched(&f);
}

#[test]
fn node_based_managers_still_require_a_node_declaration() {
    for manager in ["pnpm@10.0.0", "yarn@1.22.22", "yarn@4.5.0"] {
        let f = fresh();
        fs::remove_file(f.owned.root().join(".nvmrc")).unwrap();
        fs::write(
            f.owned.root().join("package.json"),
            json!({ "packageManager": manager }).to_string(),
        )
        .unwrap();
        let empty = LoopbackServer::new([], |_| {}).unwrap();
        let error = f.run(&empty, LOCAL).unwrap_err().to_string();
        assert!(
            error.contains("Node declaration is required"),
            "{manager}: {error}"
        );
        assert_eq!(empty.hits(), 0);
        assert!(!f.owned.root().join("turbo.lock").exists());
        untouched(&f);
    }
}
