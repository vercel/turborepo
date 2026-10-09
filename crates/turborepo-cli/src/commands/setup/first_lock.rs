//! Actual parser → discovery → local lock writer → frozen executor fixtures.
use sha2::Sha512;

use super::*;

const LOCAL: &[&str] = &["--tools-only", "--no-frozen"];
#[path = "no_lock_tests.rs"]
mod no_lock;
#[path = "refresh.rs"]
mod refresh;
#[test]
fn actual_local_missing_host_plan_leaves_zero_repository_state() {
    let f = fresh();
    fs::write(f.owned.root().join("package.json"), "{}").unwrap();
    let mut r = routes(&f, "24.0.0", "10.0.0");
    let file = if matches!(
        provision::platform().unwrap(),
        turborepo_setup::lock::Platform::LinuxX64Gnu
    ) {
        "osx-arm64-tar"
    } else {
        "linux-x64"
    };
    r.iter_mut()
        .find(|(p, _)| p == "/dist/index.json")
        .unwrap()
        .1 = json!([{"version":"v24.0.0","npm":"11.6.1","lts":false,"files":[file]}])
        .to_string()
        .into_bytes();
    let u = LoopbackServer::new(r, |_| {}).unwrap();
    let before = super::publication_guards::state(f.owned.root());
    assert!(f.run(&u, LOCAL).is_err());
    assert_eq!(u.hits(), 2);
    assert_eq!(super::publication_guards::state(f.owned.root()), before);
    assert!(!f.owned.root().join(".turbo").exists());
    untouched(&f);
}
#[test]
fn actual_local_workspace_and_git_indirection_repros_cannot_publish() {
    for git in [false, true] {
        let f = fresh();
        let root = f.owned.root();
        if git {
            fs::rename(root.join(".git"), root.join("gitdir")).unwrap();
            fs::write(root.join(".git"), "gitdir: gitdir\n").unwrap();
            fs::create_dir(root.join("second")).unwrap();
            assert!(
                std::process::Command::new("git")
                    .current_dir(root.join("second"))
                    .args(["init", "-q"])
                    .status()
                    .unwrap()
                    .success()
            );
            fs::rename(root.join("second/.git"), root.join("replacement-gitdir")).unwrap();
        } else {
            fs::write(root.join("workspace.yaml"), "packages: ['apps/*']\n").unwrap();
            std::os::unix::fs::symlink("workspace.yaml", root.join("pnpm-workspace.yaml")).unwrap();
        }
        let u = LoopbackServer::new(routes(&f, "24.0.0", "10.0.0"), |_| {}).unwrap();
        let fired = std::cell::Cell::new(false);
        let result = dispatch(&f, &u, LOCAL, || {
            if root.join(".turbo/setup-lock/staged").exists() && !fired.replace(true) {
                if git {
                    fs::rename(root.join("gitdir"), root.join("old-gitdir")).unwrap();
                    fs::rename(root.join("replacement-gitdir"), root.join("gitdir")).unwrap();
                } else {
                    fs::write(
                        root.join("workspace.yaml"),
                        "packages: ['apps/*', 'packages/*']\n",
                    )
                    .unwrap();
                }
            }
        });
        assert!(result.is_err());
        assert_eq!(fired.get(), git); // Unsupported workspace link rejects BEFORE staging.
        assert_eq!(u.hits(), if git { 4 } else { 0 });
        assert!(!root.join("turbo.lock").exists());
        assert!(f.manifest().is_none());
        if !git {
            assert!(!root.join(".turbo").exists());
        }
        untouched(&f);
    }
}
fn bundled(prefix: &str) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for (path, body) in [
        ("bin/node", "#!/bin/sh\ntouch probe-ran\nexit 91\n"),
        ("resource", "complete Node resource"),
        (
            "lib/node_modules/npm/package.json",
            r#"{"name":"npm","version":"11.6.1","bin":{"npm":"bin/npm-cli.js","npx":"bin/npx-cli.js"}}"#,
        ),
        (
            "lib/node_modules/npm/bin/npm-cli.js",
            "throw new Error('never execute');",
        ),
        (
            "lib/node_modules/npm/bin/npx-cli.js",
            "throw new Error('never execute');",
        ),
    ] {
        let mut h = tar::Header::new_gnu();
        h.set_size(body.len() as u64);
        h.set_mode(0o755);
        h.set_cksum();
        tar.append_data(&mut h, format!("{prefix}/{path}"), body.as_bytes())
            .unwrap();
    }
    for (name, cli) in [("npm", "npm-cli.js"), ("npx", "npx-cli.js")] {
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(tar::EntryType::Symlink);
        h.set_size(0);
        h.set_mode(0o777);
        h.set_link_name(format!("../lib/node_modules/npm/bin/{cli}"))
            .unwrap();
        h.set_cksum();
        tar.append_data(&mut h, format!("{prefix}/bin/{name}"), &b""[..])
            .unwrap();
    }
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&tar.into_inner().unwrap()).unwrap();
    gzip.finish().unwrap()
}
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::new();
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            encoded.push(if i > chunk.len() {
                '='
            } else {
                ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char
            });
        }
    }
    encoded
}
fn routes(f: &Fixture, node_version: &str, manager_version: &str) -> Vec<(String, Vec<u8>)> {
    let index = json!([{"version":format!("v{node_version}"),"npm":"11.6.1","lts":"Krypton",
        "files":["osx-x64-tar","osx-arm64-tar","linux-x64","linux-arm64","win-x64-zip","win-arm64-zip"]}]);
    let mut result = vec![(
        "/dist/index.json".into(),
        serde_json::to_vec(&index).unwrap(),
    )];
    let host = f
        .node_path
        .rsplit('/')
        .next()
        .unwrap()
        .trim_start_matches("node-v24.0.0-")
        .trim_end_matches(".tar.gz");
    let mut sums = String::new();
    for target in [
        "darwin-x64",
        "darwin-arm64",
        "linux-x64",
        "linux-arm64",
        "win-x64",
        "win-arm64",
    ] {
        let prefix = format!("node-v{node_version}-{target}");
        let windows = target.starts_with("win-");
        let bytes = if windows {
            b"PK\x05\x06\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0".to_vec()
        } else {
            bundled(&prefix)
        };
        let filename = format!("{prefix}.{}", if windows { "zip" } else { "tar.gz" });
        sums.push_str(&format!("{:x}  {filename}\n", Sha256::digest(&bytes)));
        if target == host {
            result.push((format!("/dist/v{node_version}/{filename}"), bytes));
        }
    }
    result.push((
        format!("/dist/v{node_version}/SHASUMS256.txt"),
        sums.into_bytes(),
    ));
    let pnpm = archive(
        "package",
        &[
            (
                "package.json",
                &format!(
                    r#"{{"name":"pnpm","version":"{manager_version}","bin":{{"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"}},"scripts":{{"install":"touch hook-ran"}}}}"#
                ),
            ),
            ("bin/pnpm.cjs", "throw new Error('never execute');"),
            ("bin/pnpx.cjs", "throw new Error('never execute');"),
            ("dist/resource", "complete pnpm resource"),
        ],
    );
    result.push((
        format!("/pnpm/{manager_version}"),
        json!({"name":"pnpm","version":manager_version,"dist":{
        "tarball":format!("https://registry.npmjs.org/pnpm/-/pnpm-{manager_version}.tgz"),
        "integrity":format!("sha512-{}",base64(&Sha512::digest(&pnpm)))}})
        .to_string()
        .into_bytes(),
    ));
    result.push((format!("/pnpm/-/pnpm-{manager_version}.tgz"), pnpm));
    result
}
fn fresh() -> Fixture {
    let f = Fixture::new();
    fs::remove_file(f.owned.root().join("turbo.lock")).unwrap();
    for name in [
        "pnpm-lock.yaml",
        "package-lock.json",
        "Cargo.lock",
        "uv.lock",
        "go.sum",
    ] {
        fs::write(f.owned.root().join(name), "ecosystem sentinel").unwrap();
    }
    f
}
fn untouched(f: &Fixture) {
    f.no_execution();
    for name in [
        "pnpm-lock.yaml",
        "package-lock.json",
        "Cargo.lock",
        "uv.lock",
        "go.sum",
    ] {
        assert_eq!(
            fs::read(f.owned.root().join(name)).unwrap(),
            b"ecosystem sentinel"
        );
    }
    assert!(!f.owned.root().join(".turbo/setup-lock/staged").exists());
}
fn selected(f: &Fixture) -> turborepo_setup::lock::Lock {
    turborepo_setup::lock::Lock::read(f.owned.root())
        .unwrap()
        .unwrap()
}
fn dispatch(
    f: &Fixture,
    u: &LoopbackServer,
    flags: &[&str],
    hook: impl Fn(),
) -> Result<i32, Error> {
    let args = f.args(flags);
    crate::cli::dispatch_setup(&args, |args, setup_args| run_with_policy(args, setup_args, Some(provision::Transports {
        node: turborepo_setup::node_provision::NodeTransport::loopback_http_for_tests(u.origin()).unwrap(),
        pnpm: turborepo_setup::pnpm_provision::PnpmTransport::loopback_http_for_tests(u.origin()).unwrap(),
        registry: turborepo_setup::registry_resolution::RegistryTransport::loopback_http_for_tests(u.origin()).unwrap(),
    }), || {
        hook();
        Ok(f.owned.policy_at(&f.owned.root().join("apps/web/src"))?)
    })).unwrap()
}
#[test]
fn local_create_repeat_targeted_update_removal_and_frozen_drift() {
    let f = fresh();
    fs::write(f.owned.root().join(".nvmrc"), "24.x").unwrap();
    let u = LoopbackServer::new(routes(&f, "24.0.0", "10.0.0"), |_| {}).unwrap();
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    assert_eq!(u.hits(), 7); // Four resolution requests, then exact frozen provisioning.
    let initial = selected(&f);
    let manifest = f.manifest().unwrap();
    let lock = fs::read(f.owned.root().join("turbo.lock")).unwrap();
    let empty = LoopbackServer::new([], |_| {}).unwrap();
    assert_eq!(f.run(&empty, LOCAL).unwrap(), 0);
    assert_eq!(empty.hits(), 0); // Unchanged floating Node never refreshes.
    assert_eq!(f.manifest().unwrap(), manifest);
    assert_eq!(fs::read(f.owned.root().join("turbo.lock")).unwrap(), lock);
    fs::write(
        f.owned.root().join("package.json"),
        r#"{"packageManager":"pnpm@10.1.0"}"#,
    )
    .unwrap();
    assert!(f.run(&empty, FROZEN).is_err());
    assert_eq!(fs::read(f.owned.root().join("turbo.lock")).unwrap(), lock);
    let update = LoopbackServer::new(routes(&f, "25.0.0", "10.1.0"), |p| {
        assert!(!p.starts_with("/dist/"))
    })
    .unwrap();
    assert_eq!(f.run(&update, LOCAL).unwrap(), 0);
    assert_eq!(update.hits(), 4);
    assert_eq!(selected(&f).tools()["node"], initial.tools()["node"]);
    fs::write(f.owned.root().join(".nvmrc"), "24.1.x").unwrap();
    let node = LoopbackServer::new(routes(&f, "24.1.0", "10.1.0"), |_| {}).unwrap();
    let prior_manager = selected(&f).tools()["pnpm"].clone();
    assert_eq!(f.run(&node, LOCAL).unwrap(), 0);
    assert_eq!(node.hits(), 5); // Two Node metadata reads; frozen generation also re-stages pnpm.
    assert_eq!(selected(&f).tools()["pnpm"], prior_manager);
    fs::write(f.owned.root().join(".node-version"), "24.1.x").unwrap();
    f.run(&node, LOCAL).unwrap();
    fs::remove_file(f.owned.root().join(".nvmrc")).unwrap();
    f.run(&node, LOCAL).unwrap(); // Removal of a Node constraint retains the other source.
    fs::write(f.owned.root().join("package.json"), "{}").unwrap();
    f.run(&empty, LOCAL).unwrap();
    assert_eq!(selected(&f).tools().len(), 1);
    assert_eq!(empty.hits(), 0);
    let store = turborepo_tool_install::Store::open(f.owned.root()).unwrap();
    let current = store.current().unwrap().unwrap();
    assert_eq!(current.tools.len(), 1);
    assert!(!current.bin.join("pnpm").exists());
    assert_eq!(
        fs::read(current.bin.join("../tools/node/resource")).unwrap(),
        b"complete Node resource"
    );
    untouched(&f);
}
#[test]
fn local_unsupported_and_missing_frozen_fail_before_any_writer() {
    for case in 0..7 {
        let f = fresh();
        let u = LoopbackServer::new([], |_| {}).unwrap();
        match case {
            0 => fs::remove_file(f.owned.root().join(".nvmrc")).unwrap(),
            1 => fs::write(f.owned.root().join("package.json"), r#"{"packageManager":"npm@11.6.1"}"#).unwrap(),
            2 => fs::write(f.owned.root().join("package.json"), r#"{"packageManager":"pnpm@10.x"}"#).unwrap(),
            3 => fs::write(f.owned.root().join(".npmrc"), "").unwrap(),
            4 => fs::write(f.owned.root().join(".gitignore"), "").unwrap(),
            5 => fs::write(f.owned.root().join("package.json"), r#"{"packageManager":"pnpm@10.0.0","devEngines":{"packageManager":{"name":"npm","version":"11.6.1","onFail":"error"}}}"#).unwrap(),
            _ => {},
        }
        assert!(
            f.run(&u, if case == 6 { FROZEN } else { LOCAL }).is_err(),
            "case {case}"
        );
        assert_eq!(u.hits(), 0);
        assert!(!f.owned.root().join(".turbo").exists());
        assert!(!f.owned.root().join("turbo.lock").exists());
        untouched(&f);
    }
}
#[test]
fn local_post_stage_preconditions_abort_without_lock_or_generation_publication() {
    for case in 0..11 {
        let f = fresh();
        let root = f.owned.root();
        let u = LoopbackServer::new(routes(&f, "24.0.0", "10.0.0"), |_| {}).unwrap();
        let fired = std::cell::Cell::new(false);
        let result = dispatch(&f, &u, LOCAL, || {
            if !root.join(".turbo/setup-lock/staged").exists() || fired.replace(true) {
                return;
            }
            match case {
                0 => fs::write(root.join(".nvmrc"), "26.x").unwrap(),
                1 => fs::write(root.join("apps/web/turbo.json"), ENABLED).unwrap(),
                2 => fs::write(root.join("apps/web/pnpm-workspace.yaml"), "packages: []").unwrap(),
                3 => fs::write(root.join("apps/web/.git"), "gitdir: /not-probed").unwrap(),
                4 => fs::write(f.owned.home().join(".npmrc"), "").unwrap(),
                5 => fs::write(root.join(".gitignore"), "").unwrap(),
                6 => fs::write(root.join("turbo.json"), "{}").unwrap(),
                7 => {
                    let mut foreign: Value = serde_json::from_slice(
                        &fs::read(root.join(".turbo/setup-lock/staged")).unwrap(),
                    )
                    .unwrap();
                    foreign["tools"]["node"]["version"] = json!("24.9.0");
                    fs::write(root.join("turbo.lock"), foreign.to_string()).unwrap();
                }
                10 => fs::write(root.join("pnpm-workspace.yaml"), "packages: []").unwrap(),
                8 => {
                    fs::rename(root, root.with_extension("old")).unwrap();
                    fs::create_dir(root).unwrap();
                }
                _ => {
                    fs::write(root.join(".turbo/sentinel"), "untracked must stay ignored").unwrap();
                    assert!(
                        std::process::Command::new("git")
                            .current_dir(root)
                            .args(["add", "-f", ".turbo/sentinel"])
                            .status()
                            .unwrap()
                            .success()
                    );
                }
            }
        });
        assert!(fired.get(), "case {case}");
        assert!(result.is_err(), "case {case}");
        assert_eq!(u.hits(), 4);
        assert!(f.manifest().is_none());
        if case != 7 {
            assert!(!root.join("turbo.lock").exists());
        }
        if case == 8 {
            fs::remove_dir(root).unwrap();
            fs::rename(root.with_extension("old"), root).unwrap();
        }
        untouched(&f);
    }
}
#[test]
fn local_writer_wait_rechecks_policy_git_and_workspace() {
    for name in ["apps/web/.npmrc", ".gitignore", "pnpm-workspace.yaml"] {
        let f = fresh();
        let (tx, rx) = std::sync::mpsc::channel();
        let u = LoopbackServer::new(routes(&f, "24.0.0", "10.0.0"), move |p| {
            if p == "/pnpm/-/pnpm-10.0.0.tgz" {
                tx.send(()).unwrap();
            }
        })
        .unwrap();
        let snapshot = turborepo_setup::lock::Snapshot::capture(f.owned.root()).unwrap();
        let guard = snapshot.guard().unwrap();
        std::thread::scope(|scope| {
            let pending = scope.spawn(|| f.run(&u, LOCAL));
            rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
            fs::write(f.owned.root().join(name), "").unwrap();
            drop(guard);
            assert!(pending.join().unwrap().is_err());
        });
        assert_eq!(u.hits(), 4);
        assert!(f.manifest().is_none());
        assert!(!f.owned.root().join("turbo.lock").exists());
        untouched(&f);
    }
}
#[test]
fn intentional_publication_retains_original_root_native_config_and_expected_lock() {
    for case in 0..4 {
        let f = fresh();
        let root = f.owned.root();
        let old = root.with_extension("old");
        let u = LoopbackServer::new(routes(&f, "24.0.0", "10.0.0"), |_| {}).unwrap();
        let fired = std::cell::Cell::new(false);
        assert!(
            dispatch(&f, &u, LOCAL, || {
                if !root.join("turbo.lock").exists()
                    || root.join(".turbo/setup-lock/staged").exists()
                    || fired.replace(true)
                {
                    return;
                }
                match case {
                    0 => {
                        let mut foreign: Value =
                            serde_json::from_slice(&fs::read(root.join("turbo.lock")).unwrap())
                                .unwrap();
                        foreign["tools"]["node"]["version"] = json!("24.9.0");
                        fs::write(root.join("turbo.lock"), foreign.to_string()).unwrap();
                    }
                    1 => fs::write(root.join(".nvmrc"), "26.x").unwrap(),
                    2 => fs::write(root.join("turbo.json"), "{}").unwrap(),
                    _ => {
                        fs::rename(root, &old).unwrap();
                        fs::create_dir(root).unwrap();
                        fs::create_dir_all(root.join("apps/web/src")).unwrap();
                        for name in [
                            "turbo.json",
                            ".nvmrc",
                            "package.json",
                            "turbo.lock",
                            ".gitignore",
                        ] {
                            fs::copy(old.join(name), root.join(name)).unwrap();
                        }
                        fs::rename(old.join(".git"), root.join(".git")).unwrap();
                    }
                }
            })
            .is_err(),
            "case {case}"
        );
        assert!(fired.get());
        assert_eq!(u.hits(), 4);
        assert!(f.manifest().is_none());
        if case == 0 {
            assert_eq!(selected(&f).tools()["node"].version, "24.9.0");
        }
        if case == 3 {
            assert!(!root.join(".turbo").exists());
            fs::rename(root.join(".git"), old.join(".git")).unwrap();
            fs::remove_dir_all(root).unwrap();
            fs::rename(old, root).unwrap();
        }
        untouched(&f);
    }
}
#[test]
fn published_lock_does_not_hide_late_source_or_foreign_candidate_and_tools_are_atomic() {
    for case in 0..3 {
        let f = fresh();
        let initial = LoopbackServer::new(routes(&f, "24.0.0", "10.0.0"), |_| {}).unwrap();
        f.run(&initial, LOCAL).unwrap();
        let manifest = f.manifest().unwrap();
        fs::write(f.owned.root().join(".nvmrc"), "24.1.x").unwrap();
        let root = f.owned.root().to_owned();
        let u = LoopbackServer::new(routes(&f, "24.1.0", "10.0.0"), move |p| {
            if p.ends_with(".tar.gz") && p.starts_with("/dist/") {
                match case {
                    0 => fs::write(root.join("package.json"), "{}").unwrap(),
                    1 => {
                        let mut foreign: Value =
                            serde_json::from_slice(&fs::read(root.join("turbo.lock")).unwrap())
                                .unwrap();
                        foreign["tools"]["node"]["version"] = json!("24.9.0");
                        fs::write(root.join("turbo.lock"), foreign.to_string()).unwrap();
                    }
                    _ => fs::write(root.join("apps/web/.npmrc"), "").unwrap(),
                }
            }
        })
        .unwrap();
        assert!(f.run(&u, LOCAL).is_err());
        assert_eq!(f.manifest().unwrap(), manifest);
        let store = turborepo_tool_install::Store::open(f.owned.root()).unwrap();
        assert_eq!(store.current().unwrap().unwrap().tools.len(), 2);
        untouched(&f);
    }
}
