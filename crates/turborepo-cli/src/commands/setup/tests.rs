#![allow(clippy::unwrap_used)]
use std::{fs, io::Write, path::PathBuf};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use turborepo_setup::test_support::{LoopbackServer, OwnedSetupFixture};

use super::*;

const ENABLED: &str = r#"{"futureFlags":{"experimentalSetup":true}}"#;
/// The overrides managed `turbo run` applies from the committed root config
/// and the fixture's trusted setup record, plus the managed bin first on PATH.
fn run_overrides(
    owned: &OwnedSetupFixture,
) -> Result<(turborepo_run::EnvironmentVariableMap, PathBuf), String> {
    use crate::commands::managed::{ManagedTools, context};
    let root = owned.root();
    let tools = ManagedTools::inspect(root, &root.join("turbo.json"), true, owned.home(), context)?
        .ok_or("unmanaged")?;
    let env = tools.revalidate()?;
    let bin = std::env::split_paths(&env["PATH"]).next().unwrap();
    Ok((env, bin))
}
fn archive(root: &str, files: &[(&str, &str)]) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for (path, body) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, format!("{root}/{path}"), body.as_bytes())
            .unwrap();
    }
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&tar.into_inner().unwrap()).unwrap();
    gzip.finish().unwrap()
}
struct Fixture {
    owned: OwnedSetupFixture,
    lock: Value,
    node: Vec<u8>,
    pnpm: Vec<u8>,
    node_path: String,
}
impl Fixture {
    fn new() -> Self {
        let owned = OwnedSetupFixture::new().unwrap();
        let root = owned.root();
        owned.init_git().unwrap();
        fs::write(root.join("turbo.json"), ENABLED).unwrap();
        fs::write(root.join(".nvmrc"), "24.0.0\n").unwrap();
        fs::write(
            root.join("package.json"),
            r#"{"packageManager":"pnpm@10.0.0","scripts":{"build":"touch task-ran"}}"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("apps/web/src")).unwrap();
        let (platform, spelling) = match provision::platform().unwrap() {
            turborepo_setup::lock::Platform::MacosArm64 => ("macos-arm64", "darwin-arm64"),
            turborepo_setup::lock::Platform::MacosX64 => ("macos-x64", "darwin-x64"),
            turborepo_setup::lock::Platform::LinuxX64Gnu => ("linux-x64-gnu", "linux-x64"),
            _ => ("linux-arm64-gnu", "linux-arm64"),
        };
        let prefix = format!("node-v24.0.0-{spelling}");
        let node_path = format!("/dist/v24.0.0/{prefix}.tar.gz");
        // Would visibly fail if any tool probe, task, or install hook executed.
        let node = archive(
            &prefix,
            &[
                ("bin/node", "#!/bin/sh\ntouch probe-ran\nexit 91\n"),
                ("resource", "node resource"),
            ],
        );
        let pnpm = archive(
            "package",
            &[
                (
                    "package.json",
                    r#"{"name":"pnpm","version":"10.0.0","bin":{"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"},"scripts":{"install":"touch hook-ran"}}"#,
                ),
                ("bin/pnpm.cjs", "throw new Error('must not run');"),
                ("bin/pnpx.cjs", "throw new Error('must not run');"),
                ("dist/resource", "pnpm resource"),
            ],
        );
        let lock = json!({"schemaVersion":0,"tools":{
            "node":{"adapter":"node","version":"24.0.0","declarations":[{"file":".nvmrc","request":"24.0.0"}],
                "installation":{"kind":"managed","artifacts":{platform:{"distribution":{
                    "url":format!("https://nodejs.org{node_path}"),"sha256":format!("{:x}",Sha256::digest(&node)),
                    "format":"tar-gz","rootPrefix":prefix,"executables":{"node":"bin/node"}
                }}}}},
            "pnpm":{"adapter":"pnpm","version":"10.0.0","declarations":[{"file":"package.json","field":"/packageManager","request":"pnpm@10.0.0"}],
                "installation":{"kind":"managed","artifacts":{"any":{"package":{
                    "url":"https://registry.npmjs.org/pnpm/-/pnpm-10.0.0.tgz","sha256":format!("{:x}",Sha256::digest(&pnpm)),
                    "format":"tar-gz","rootPrefix":"package","executables":{"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"}
                }}}}}
        }});
        let f = Self {
            owned,
            lock,
            node,
            pnpm,
            node_path,
        };
        f.save();
        f
    }
    fn save(&self) {
        fs::write(self.owned.root().join("turbo.lock"), self.lock.to_string()).unwrap();
    }
    fn declare(&mut self, package: Value) {
        fs::write(self.owned.root().join("package.json"), package.to_string()).unwrap();
        for (id, declarations) in turborepo_setup::lock::probe_native(self.owned.root()).unwrap() {
            self.lock["tools"][id]["declarations"] = serde_json::to_value(declarations).unwrap();
        }
        self.save();
    }
    fn args(&self, flags: &[&str]) -> Args {
        let mut words = vec![
            "turbo".into(),
            "setup".into(),
            "--cwd".into(),
            self.owned.root().join("apps/web/src").into_os_string(),
        ];
        words.extend(flags.iter().map(std::ffi::OsString::from));
        Args::parse_args(words).unwrap()
    }
    fn transports(
        &self,
        origin: &str,
        cache: Option<turborepo_setup::artifact_cache::ArtifactCache>,
    ) -> provision::Transports {
        provision::Transports {
            trust_base: Some(self.owned.home().to_owned()),
            registry:
                turborepo_setup::registry_resolution::RegistryTransport::loopback_http_for_tests(
                    origin,
                )
                .unwrap(),
            node: turborepo_setup::node_provision::NodeTransport::loopback_http_for_tests(origin)
                .unwrap()
                .with_cache(cache.clone()),
            pnpm: turborepo_setup::pnpm_provision::PnpmTransport::loopback_http_for_tests(origin)
                .unwrap()
                .with_cache(cache),
            uv: turborepo_setup::uv_resolution::UvTransport::loopback_http_for_tests(origin)
                .unwrap(),
            rustup: turborepo_setup::rustup_provision::RustupTransport::loopback_http_for_tests(
                origin,
            )
            .unwrap(),
            python: turborepo_setup::python_resolution::PythonTransport::loopback_http_for_tests(
                origin,
            )
            .unwrap(),
            artifact: Some(
                turborepo_setup::artifact_provision::ArtifactTransport::loopback_http_for_tests(
                    origin,
                )
                .unwrap(),
            ),
            go: turborepo_setup::go_resolution::GoTransport::loopback_http_for_tests(origin)
                .unwrap(),
            bun: turborepo_setup::bun_provision::BunTransport::loopback_http_for_tests(origin)
                .unwrap(),
        }
    }
    fn run(&self, upstream: &LoopbackServer, flags: &[&str]) -> Result<i32, Error> {
        self.run_with(upstream.origin(), None, flags)
    }
    fn run_with(
        &self,
        origin: &str,
        cache: Option<&std::path::Path>,
        flags: &[&str],
    ) -> Result<i32, Error> {
        let args = self.args(flags);
        let cache = cache.and_then(turborepo_setup::artifact_cache::ArtifactCache::new);
        crate::cli::dispatch_setup(&args, |args, setup_args| {
            run_with_policy(
                args,
                setup_args,
                Some(self.transports(origin, cache.clone())),
                || {
                    Ok(self
                        .owned
                        .policy_at(&self.owned.root().join("apps/web/src"))?)
                },
            )
        })
        .unwrap()
    }
    /// The effective dependency setup that `flags` select, without running it.
    fn dependencies(&self, flags: &[&str]) -> dependencies::Dependencies {
        let args = self.args(flags);
        let Some(crate::cli::Command::Setup { setup_args }) = &args.command else {
            unreachable!("setup words");
        };
        let request = SetupRequest::new(setup_args, false).unwrap();
        let discovery = root::Discovery::capture(&args).unwrap();
        let javascript = [turborepo_repository::toolchain::ToolchainId::JAVASCRIPT];
        let plan = effective::Plan::new(discovery.policy(), javascript, request.tools_only);
        let snapshot = turborepo_setup::lock::Snapshot::capture(self.owned.root()).unwrap();
        dependencies::Dependencies::select(&discovery, &plan, &snapshot, &request).unwrap()
    }
    fn manifest(&self) -> Option<Vec<u8>> {
        fs::read(self.owned.root().join(".turbo/tools/manifest.json")).ok()
    }
    fn no_execution(&self) {
        for path in ["node_modules", "task-ran", "hook-ran", "probe-ran"] {
            assert!(!self.owned.root().join(path).exists());
        }
    }
}
struct Upstream(LoopbackServer);
impl std::ops::Deref for Upstream {
    type Target = LoopbackServer;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl Upstream {
    fn new(f: &Fixture, corrupt: bool, drift: Option<(PathBuf, String)>) -> Self {
        Self(
            LoopbackServer::node_pnpm(
                f.node_path.clone(),
                f.node.clone(),
                f.pnpm.clone(),
                corrupt,
                move |request| {
                    if request == "/pnpm/-/pnpm-10.0.0.tgz"
                        && let Some((path, body)) = &drift
                    {
                        if body == "track" {
                            track(path);
                        } else {
                            fs::write(path, body).unwrap();
                        }
                    }
                },
            )
            .unwrap(),
        )
    }
}
fn track(root: &std::path::Path) {
    assert!(
        std::process::Command::new("git")
            .current_dir(root)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .args(["add", "-f", ".turbo/tools/manifest.json"])
            .status()
            .unwrap()
            .success()
    );
}
#[path = "cache_tests.rs"]
mod cache;
#[path = "check_tests.rs"]
mod checks;
#[path = "cleanup_tests.rs"]
mod cleanups;
#[path = "plan_tests.rs"]
mod plans;

const FROZEN: &[&str] = &["--frozen", "--tools-only"];
#[path = "additional_tools_tests.rs"]
mod additional_tools;
#[path = "authorization_tests.rs"]
mod authorization;
#[path = "bun_tests.rs"]
mod bun;
#[path = "first_lock.rs"]
mod first_lock;
#[path = "force_tests.rs"]
mod force;
#[path = "managed_exec_tests.rs"]
mod managed_exec;
#[path = "managed_run_tests.rs"]
mod managed_run;
#[path = "native_handoff_tests.rs"]
mod native_handoff;
#[path = "native_plain_run_tests.rs"]
mod native_plain_run;
#[path = "npm_tests.rs"]
mod npm;
#[path = "offline_tests.rs"]
mod offline_repeat;
#[path = "publication_guards.rs"]
mod publication_guards;
#[path = "rustup_tests.rs"]
mod rustup;
#[cfg(target_os = "macos")]
#[path = "trust_acl_tests.rs"]
mod trust_acl;
#[path = "trust_publication_test.rs"]
mod trust_publication;
#[path = "trust_tests.rs"]
mod trusted_roots;
#[path = "unlocked_trust_tests.rs"]
mod unlocked_trust;
#[path = "uv_tests.rs"]
mod uv;
#[path = "yarn_tests.rs"]
mod yarn;
#[test]
fn authored_pins_fail_before_mutation_and_dev_integrity_cannot_reuse_unverified_tools() {
    for algorithm in ["sha1", "sha256"] {
        for top in [true, false] {
            let mut f = Fixture::new();
            let digest = "0".repeat(if algorithm == "sha1" { 40 } else { 64 });
            let request = format!("10.0.0+{algorithm}.{digest}");
            f.declare(if top {
                json!({"packageManager":format!("pnpm@{request}")})
            } else {
                json!({"devEngines":{"packageManager":{"name":"pnpm","version":request}}})
            });
            let u = Upstream::new(&f, false, None);
            assert!(f.run(&u, FROZEN).is_err());
            assert_eq!(u.hits(), 0);
            assert!(!f.owned.root().join(".turbo").exists());
        }
    }
    let mut f = Fixture::new();
    let u = Upstream::new(&f, false, None);
    f.run(&u, FROZEN).unwrap();
    let before = f.manifest().unwrap();
    f.declare(
        json!({"packageManager":"pnpm@10.0.0","devEngines":{"packageManager":{
            "name":"pnpm","version":format!("10.0.0+sha512.{}", "0".repeat(128))
        }}}),
    );
    assert!(f.run(&u, FROZEN).is_err());
    assert_eq!(u.hits(), 4); // Reused Node, but the new dev pin requires registry verification.
    assert_eq!(f.manifest().unwrap(), before);
    f.no_execution();
}
#[test]
fn authoritative_pnpm_keeps_wrong_manager_advisories_in_frozen_local_and_check() {
    for on_fail in ["warn", "ignore"] {
        let mut f = Fixture::new();
        f.declare(
            json!({"packageManager":"pnpm@10.0.0","devEngines":{"packageManager":{
                "name":"npm","version":"11.6.1","onFail":on_fail
            }}}),
        );
        let snapshot = turborepo_setup::lock::Snapshot::capture(f.owned.root()).unwrap();
        let declaration = snapshot.package_manager().unwrap().unwrap();
        assert_eq!(
            declaration.manager,
            turborepo_setup::package_manager::Manager::Pnpm
        );
        assert_eq!(declaration.warnings.len(), usize::from(on_fail == "warn"));
        fs::write(
            f.owned.root().join("turbo.lock"),
            snapshot.previous_lock().unwrap().canonical_bytes().unwrap(),
        )
        .unwrap();
        let u = Upstream::new(&f, false, None);
        assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
        let before = publication_guards::state(f.owned.root());
        for flags in [
            FROZEN,
            &["--tools-only", "--no-frozen"],
            &["--check", "--tools-only", "--offline"],
            &["--check"],
        ] {
            assert_eq!(f.run(&u, flags).unwrap(), 0);
            assert_eq!(publication_guards::state(f.owned.root()), before);
        }
        assert_eq!(u.hits(), 3);
        f.no_execution();
    }
}
#[test]
fn frozen_node_pnpm_command_repeat_is_noop_and_preserves_complete_resources() {
    let f = Fixture::new();
    let u = Upstream::new(&f, false, None);
    let lock = fs::read(f.owned.root().join("turbo.lock")).unwrap();
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    assert_eq!(u.hits(), 3);
    let manifest = f.manifest().unwrap();
    let store = turborepo_tool_install::Store::open(f.owned.root()).unwrap();
    let current = store.current().unwrap().unwrap();
    assert_eq!(current.tools.len(), 2);
    assert_eq!(
        fs::read(current.bin.join("../tools/node/resource")).unwrap(),
        b"node resource"
    );
    assert_eq!(
        fs::read(current.bin.join("../tools/pnpm/dist/resource")).unwrap(),
        b"pnpm resource"
    );
    drop(store);
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    assert_eq!(u.hits(), 3);
    assert_eq!(f.manifest().unwrap(), manifest);
    assert_eq!(fs::read(f.owned.root().join("turbo.lock")).unwrap(), lock);
    f.no_execution();
}
#[test]
fn root_policy_and_cli_overrides_select_the_effective_setup() {
    let configure = |f: &Fixture, flags: &str, setup: &str| {
        let json =
            format!(r#"{{"futureFlags":{{"experimentalSetup":true{flags}}},"setup":{setup}}}"#);
        fs::write(f.owned.root().join("turbo.json"), json).unwrap();
    };
    // A repository-chosen tools-only setup needs no --tools-only.
    let f = Fixture::new();
    configure(&f, "", r#"{"dependencies":"skip"}"#);
    let u = Upstream::new(&f, false, None);
    assert_eq!(f.run(&u, &["--frozen"]).unwrap(), 0);
    assert_eq!(f.run(&u, &["--check"]).unwrap(), 0);
    assert_eq!(u.hits(), 3);
    f.no_execution();

    // --tools-only overrides locked dependencies for one invocation.
    let f = Fixture::new();
    configure(&f, "", r#"{"dependencies":"locked"}"#);
    let u = Upstream::new(&f, false, None);
    // Locked dependencies reach dependency setup instead of failing as
    // pending; this fixture's pnpm installer reports itself as skipped.
    assert_eq!(f.run(&u, &["--frozen"]).unwrap(), 0);
    assert_eq!(u.hits(), 3);
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);

    // Preserve the native managed adapters added after the original policy
    // stack. System Rust remains unsupported and must fail before traffic.
    let cargo = r#","experimentalCargoWorkspaces":true"#;
    let f = Fixture::new();
    configure(&f, cargo, r#"{"rust":{"tools":"verify-system"}}"#);
    let u = Upstream::new(&f, false, None);
    assert!(matches!(f.run(&u, FROZEN), Err(Error::Unsupported(_))));
    assert_eq!(u.hits(), 0);
    configure(&f, cargo, r#"{"tools":"managed"}"#);
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    assert_eq!(u.hits(), 3);
    f.no_execution();

    // Explicitly skipped Rust is not interpreted while JavaScript proceeds.
    let f = Fixture::new();
    configure(&f, cargo, r#"{"rust":{"tools":"skip"}}"#);
    fs::write(f.owned.root().join("rust-toolchain.toml"), "invalid = [").unwrap();
    let u = Upstream::new(&f, false, None);
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    assert_eq!(u.hits(), 3);
    f.no_execution();

    // Skipped JavaScript tools prepare nothing and never hide locked tools.
    let f = Fixture::new();
    configure(&f, "", r#"{"javascript":{"tools":"skip"}}"#);
    let u = Upstream::new(&f, false, None);
    assert!(matches!(f.run(&u, FROZEN), Err(Error::Unsupported(_))));
    fs::remove_file(f.owned.root().join("turbo.lock")).unwrap();
    assert_eq!(f.run(&u, &["--frozen"]).unwrap(), 0);
    assert_eq!(u.hits(), 0);
    assert!(!f.owned.root().join(".turbo").exists());
    f.no_execution();
}

#[test]
fn unsupported_modes_and_policy_fail_before_traffic_or_storage() {
    for flags in [
        vec!["--plan"],
        vec!["--check"],
        vec!["--no-frozen", "--tools-only"],
        vec!["--no-lock"],
        vec!["--no-lock", "--tools-only", "--force"],
        vec!["--no-lock", "--tools-only", "--plan"],
        vec!["--no-lock", "--tools-only", "--offline"],
        vec!["--frozen"],
        vec!["--plan", "--tools-only", "--no-lock"],
        vec!["--frozen", "--tools-only", "--offline"],
        vec!["--no-lock", "--offline"],
        vec!["--frozen", "--offline"],
    ] {
        let f = Fixture::new();
        let u = Upstream::new(&f, false, None);
        if flags == ["--no-frozen", "--tools-only"] || flags == ["--frozen"] {
            assert_eq!(f.run(&u, &flags).unwrap(), 0);
            assert_eq!(u.hits(), 3);
        } else if flags == ["--check"] {
            // Like --check --tools-only: nothing installed is an error, read-only.
            let result = f.run(&u, &flags);
            assert!(result.is_err() && !matches!(result, Err(Error::NotImplemented)));
            assert_eq!(u.hits(), 0);
            assert!(!f.owned.root().join(".turbo").exists());
        } else {
            let result = f.run(&u, &flags);
            match flags.as_slice() {
                // Plain plans include dependencies (none for pnpm yet).
                ["--plan"] => assert_eq!(result.unwrap(), 0),
                // Neither can install locked dependencies (#14486, #14518).
                ["--no-lock"] | ["--frozen", "--offline"] => {
                    assert!(
                        matches!(&result, Err(Error::Unsupported(_))),
                        "{flags:?}: {result:?}"
                    );
                }
                _ if flags.contains(&"--offline") && !flags.contains(&"--no-lock") => {
                    assert!(
                        matches!(&result, Err(Error::Offline(_))),
                        "{flags:?}: {result:?}"
                    );
                }
                _ => assert!(
                    matches!(&result, Err(Error::NotImplemented)),
                    "{flags:?}: {result:?}"
                ),
            }
            assert_eq!(u.hits(), 0);
            assert!(!f.owned.root().join(".turbo").exists());
        }
        f.no_execution();
    }
    for path in [".npmrc", "apps/web/.npmrc", "turbo.json"] {
        let f = Fixture::new();
        let u = Upstream::new(&f, false, None);
        fs::write(
            f.owned.root().join(path),
            if path == "turbo.json" {
                r#"{"futureFlags":{"experimentalSetup":true},"setup":{}}"#
            } else {
                "registry=https://private.invalid"
            },
        )
        .unwrap();
        assert!(f.run(&u, FROZEN).is_err());
        assert_eq!(u.hits(), 0);
        assert!(!f.owned.root().join(".turbo").exists());
    }
}
#[test]
fn missing_stale_non_native_or_wrong_exact_lock_never_mutates() {
    for case in 0..7 {
        let mut f = Fixture::new();
        let u = Upstream::new(&f, false, None);
        match case {
            0 => fs::remove_file(f.owned.root().join("turbo.lock")).unwrap(),
            1 => fs::write(f.owned.root().join(".nvmrc"), "26.0.0").unwrap(),
            2 => {
                f.lock["tools"]["node"]["adapter"] = json!("generic-download");
                f.save();
            }
            3 => {
                f.lock["tools"]["pnpm"]["version"] = json!("11.0.0");
                f.save();
            }
            4 => {
                fs::write(f.owned.root().join(".nvmrc"), "26.0.0").unwrap();
                f.lock["tools"]["node"]["declarations"][0]["request"] = json!("26.0.0");
                f.save();
            }
            5 => fs::write(f.owned.root().join("turbo.json"), "{}").unwrap(),
            _ => fs::write(f.owned.root().join(".gitignore"), "").unwrap(),
        }
        assert!(f.run(&u, FROZEN).is_err(), "case {case}");
        assert_eq!(u.hits(), 0);
        assert!(!f.owned.root().join(".turbo").exists());
        f.no_execution();
    }
}
#[test]
fn git_storage_drift_while_waiting_for_store_lock_never_reuses_or_downloads() {
    for indexed in [false, true] {
        let f = Fixture::new();
        let u = Upstream::new(&f, false, None);
        f.run(&u, FROZEN).unwrap();
        let before = f.manifest().unwrap();
        let store = turborepo_tool_install::Store::open(f.owned.root()).unwrap();
        std::thread::scope(|scope| {
            let pending = scope.spawn(|| f.run(&u, FROZEN));
            turborepo_setup::test_support::wait_for_writer(f.owned.root()).unwrap();
            if indexed {
                track(f.owned.root());
            } else {
                fs::write(f.owned.root().join(".gitignore"), "").unwrap();
            }
            drop(store);
            assert!(pending.join().unwrap().is_err());
        });
        assert_eq!(u.hits(), 3);
        assert_eq!(f.manifest().unwrap(), before);
    }
}
#[test]
fn late_failures_preserve_previous_inventory_and_never_publish_partial_node() {
    for case in 0..11 {
        let mut f = Fixture::new();
        let first = Upstream::new(&f, false, None);
        assert_eq!(f.run(&first, FROZEN).unwrap(), 0);
        let selected = f.manifest().unwrap();
        // Change the desired artifact, leaving the prior generation healthy.
        let prefix = f
            .node_path
            .rsplit('/')
            .next()
            .unwrap()
            .trim_end_matches(".tar.gz");
        f.node = archive(
            prefix,
            &[
                ("bin/node", "#!/bin/sh\ntouch probe-ran\nexit 91\n"),
                ("resource", "new resource"),
            ],
        );
        let artifacts = f.lock["tools"]["node"]["installation"]["artifacts"]
            .as_object_mut()
            .unwrap();
        artifacts.values_mut().next().unwrap()["distribution"]["sha256"] =
            json!(format!("{:x}", Sha256::digest(&f.node)));
        f.save();
        let drift = match case {
            1 => Some((f.owned.root().join(".nvmrc"), "26.0.0".into())),
            2 => Some((f.owned.root().join("turbo.lock"), format!("{} ", f.lock))),
            3 => Some((f.owned.root().join("apps/web/turbo.json"), ENABLED.into())),
            4 => Some((
                f.owned.root().join("apps/web/pnpm-workspace.yaml"),
                "packages: []".into(),
            )),
            5 => Some((
                f.owned.root().join("apps/web/.git"),
                "gitdir: /not-probed\n".into(),
            )),
            6 => Some((f.owned.home().join(".npmrc"), "".into())),
            7 => Some((f.owned.root().join("package.json"), "{}".into())),
            8 => Some((
                f.owned.root().join("apps/web/package.json"),
                r#"{"workspaces":[]}"#.into(),
            )),
            9 => Some((f.owned.root().join(".gitignore"), "".into())),
            10 => Some((f.owned.root().to_owned(), "track".into())),
            _ => None,
        };
        let u = Upstream::new(&f, case == 0, drift);
        assert!(f.run(&u, FROZEN).is_err(), "case {case}");
        assert_eq!(u.hits(), 3);
        assert_eq!(f.manifest().unwrap(), selected);
        let store = turborepo_tool_install::Store::open(f.owned.root()).unwrap();
        assert_eq!(store.current().unwrap().unwrap().tools.len(), 2);
        f.no_execution();
    }
    let f = Fixture::new();
    let u = Upstream::new(&f, true, None);
    assert!(f.run(&u, FROZEN).is_err());
    assert_eq!(u.hits(), 3);
    assert!(f.manifest().is_none());
    assert!(
        !fs::read_dir(f.owned.root().join(".turbo/tools"))
            .unwrap()
            .any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("generation-"))
    );
}

#[test]
fn telemetry_report_counts_artifact_bytes_and_reuse() {
    use turborepo_telemetry::events::setup::{
        SetupDependencies, SetupFailure, SetupLock, SetupMode,
    };
    let f = Fixture::new();
    let u = Upstream::new(&f, false, None);
    let run = |flags: &[&str]| {
        let mut report = telemetry::Report::default();
        let result = crate::cli::dispatch_setup(&f.args(flags), |args, setup_args| {
            let transports = f.transports(u.origin(), None);
            run_reporting(args, setup_args, Some(transports), &mut report, || {
                Ok(f.owned.policy_at(&f.owned.root().join("apps/web/src"))?)
            })
        })
        .unwrap();
        (result.map_err(|error| telemetry::failure(&error)), report)
    };
    let provision = Some((
        SetupMode::Provision,
        SetupLock::Frozen,
        SetupDependencies::Skip,
    ));
    let ready = |downloaded_bytes, tools_installed, tools_reused| telemetry::Report {
        gated: true,
        request: provision,
        downloaded_bytes,
        tools_installed,
        tools_reused,
    };
    // First ready state downloads both verified artifacts; a repeat reuses them.
    let artifacts = (f.node.len() + f.pnpm.len()) as u64;
    assert_eq!(run(FROZEN), (Ok(0), ready(artifacts, 2, 0)));
    assert_eq!(run(FROZEN), (Ok(0), ready(0, 0, 2)));
    let before = checks::snapshot(f.owned.root().parent().unwrap());
    assert_eq!(
        run(&["--plan", "--tools-only", "--frozen"]),
        (
            Ok(0),
            telemetry::Report {
                gated: true,
                request: Some((SetupMode::Plan, SetupLock::Frozen, SetupDependencies::Skip)),
                ..Default::default()
            }
        )
    );
    assert_eq!(checks::snapshot(f.owned.root().parent().unwrap()), before);
    assert_eq!(u.hits(), 3);
    assert_eq!(
        run(&["--check", "--tools-only", "--frozen"]),
        (
            Ok(0),
            telemetry::Report {
                gated: true,
                request: Some((SetupMode::Check, SetupLock::Frozen, SetupDependencies::Skip)),
                ..Default::default()
            }
        )
    );
    assert_eq!(
        run(&["--frozen"]).1.request,
        Some((
            SetupMode::Provision,
            SetupLock::Frozen,
            SetupDependencies::Locked
        ))
    );
    fs::remove_file(f.owned.root().join("turbo.lock")).unwrap();
    assert_eq!(
        run(FROZEN),
        (Err(SetupFailure::LockMissing), ready(0, 0, 0))
    );
    // The gate decides whether anything is recorded at all.
    fs::write(f.owned.root().join("turbo.json"), "{}").unwrap();
    assert_eq!(
        run(FROZEN),
        (Err(SetupFailure::Request), telemetry::Report::default())
    );
    f.no_execution();
}
