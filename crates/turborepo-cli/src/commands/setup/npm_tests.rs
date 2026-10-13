//! Parser → discovery → npm cohort publication/preparation → read-only check.
use sha2::Sha512;
use turborepo_setup::lock::{Installation, Lock};

use super::*;

const LOCAL: &[&str] = &["--tools-only", "--no-frozen"];
const FORCE: &[&str] = &["--tools-only", "--no-frozen", "--force"];
const CHECK: &[&str] = &["--tools-only", "--check", "--offline"];

struct NpmFixture {
    f: Fixture,
    npm: Vec<u8>,
    version: String,
}
impl NpmFixture {
    fn new(bundled: bool) -> Self {
        let mut f = Fixture::new();
        let version = if bundled { "11.6.1" } else { "11.0.0" };
        let npm = Self::package(version);
        let prefix = f
            .node_path
            .rsplit('/')
            .next()
            .unwrap()
            .trim_end_matches(".tar.gz");
        f.node = first_lock::bundled(prefix);
        let parts = f.lock["tools"]["node"]["installation"]["artifacts"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap();
        parts["distribution"]["sha256"] = json!(format!("{:x}", Sha256::digest(&f.node)));
        if bundled {
            parts["distribution"]["executables"] =
                json!({"node":"bin/node","npm":"bin/npm","npx":"bin/npx"});
            f.lock["tools"]["node"]["options"] = json!({"bundled-npm":[version]});
        }
        f.lock["tools"].as_object_mut().unwrap().remove("pnpm");
        f.lock["tools"]["npm"] = json!({"adapter":"npm","version":version,"declarations":[],
            "installation": if bundled { json!({"kind":"bundled","owner":"node"}) } else {
                json!({"kind":"managed","artifacts":{"any":{"package":{
                    "url":format!("https://registry.npmjs.org/npm/-/npm-{version}.tgz"),
                    "sha256":format!("{:x}",Sha256::digest(&npm)),"format":"tar-gz","rootPrefix":"package",
                    "executables":{"npm":"bin/npm-cli.js","npx":"bin/npx-cli.js"}
                }}}})
            }
        });
        f.declare(
            json!({"packageManager":format!("npm@{version}"),"scripts":{"build":"touch task-ran"}}),
        );
        let fixture = Self {
            f,
            npm,
            version: version.into(),
        };
        fs::write(
            fixture.f.owned.root().join("turbo.lock"),
            fixture.lock().canonical_bytes().unwrap(),
        )
        .unwrap();
        fs::write(
            fixture.f.owned.root().join("package-lock.json"),
            "dependency lock sentinel",
        )
        .unwrap();
        fixture
    }
    fn package(version: &str) -> Vec<u8> {
        archive(
            "package",
            &[
                (
                    "package.json",
                    &json!({"name":"npm","version":version,
                "bin":{"npm":"bin/npm-cli.js","npx":"bin/npx-cli.js"},
                "scripts":{"install":"touch hook-ran"}})
                    .to_string(),
                ),
                ("bin/npm-cli.js", "throw new Error('never execute npm');"),
                ("bin/npx-cli.js", "throw new Error('never execute npx');"),
                ("node_modules/resource/data.json", "complete npm resource"),
            ],
        )
    }
    fn lock(&self) -> Lock {
        Lock::parse(&fs::read(self.f.owned.root().join("turbo.lock")).unwrap()).unwrap()
    }
    fn routes(&self) -> Vec<(String, Vec<u8>)> {
        let filename = self.f.node_path.rsplit('/').next().unwrap();
        let file = match provision::platform().unwrap() {
            turborepo_setup::lock::Platform::MacosArm64 => "osx-arm64-tar",
            turborepo_setup::lock::Platform::MacosX64 => "osx-x64-tar",
            turborepo_setup::lock::Platform::LinuxX64Gnu => "linux-x64",
            _ => "linux-arm64",
        };
        vec![
            (
                "/dist/index.json".into(),
                json!([{"version":"v24.0.0","npm":"11.6.1","lts":false,"files":[file]}])
                    .to_string()
                    .into_bytes(),
            ),
            (
                "/dist/v24.0.0/SHASUMS256.txt".into(),
                format!("{:x}  {filename}\n", Sha256::digest(&self.f.node)).into_bytes(),
            ),
            (self.f.node_path.clone(), self.f.node.clone()),
            (
                format!("/npm/{}", self.version),
                json!({"name":"npm","version":self.version,"dist":{
                    "tarball":format!("https://registry.npmjs.org/npm/-/npm-{}.tgz",self.version),
                    "integrity":format!("sha512-{}", first_lock::base64(&Sha512::digest(&self.npm)))
                }})
                .to_string()
                .into_bytes(),
            ),
            (format!("/npm/-/npm-{}.tgz", self.version), self.npm.clone()),
        ]
    }
    fn upstream(&self) -> LoopbackServer {
        LoopbackServer::new(self.routes(), |_| {}).unwrap()
    }
    fn untouched(&self) {
        self.f.no_execution();
        assert_eq!(
            fs::read(self.f.owned.root().join("package-lock.json")).unwrap(),
            b"dependency lock sentinel"
        );
    }
}

#[test]
fn npm_non_host_variants_fail_all_consumers_before_traffic_or_writes() {
    for installed in [false, true] {
        for case in 0..5 {
            let mut n = NpmFixture::new(false);
            if case == 3 {
                n.f.declare(json!({"packageManager":format!(
                    "npm@11.0.0+sha256.{:x}", Sha256::digest(&n.npm)
                )}));
            }
            if installed {
                n.f.run(&n.upstream(), FROZEN).unwrap();
                assert_eq!(n.f.check(CHECK).unwrap(), 0);
            }
            let host = serde_json::to_value(provision::platform().unwrap())
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned();
            let artifacts = &mut n.f.lock["tools"]["npm"]["installation"]["artifacts"];
            let package = artifacts["any"].clone();
            *artifacts = json!({host:package,"windows-arm64":package});
            let package = &mut artifacts["windows-arm64"]["package"];
            match case {
                0 => package["url"] = json!("https://registry.npmjs.org/npm/-/npm-11.1.0.tgz"),
                1 => package["rootPrefix"] = json!("wrong"),
                2 => package["executables"]["npx"] = json!("bin/other-cli.js"),
                3 => package["sha256"] = json!("0".repeat(64)),
                _ => package["format"] = json!("zip"),
            }
            // Generic schema-valid, but not a valid portable native npm cohort.
            let lock = Lock::parse(&n.f.lock.to_string().into_bytes()).unwrap();
            fs::write(
                n.f.owned.root().join("turbo.lock"),
                lock.canonical_bytes().unwrap(),
            )
            .unwrap();
            let before = publication_guards::state(n.f.owned.root().parent().unwrap());
            let u = n.upstream();
            for flags in [LOCAL, FROZEN, FORCE, CHECK] {
                let result = if flags == CHECK {
                    n.f.check(flags)
                } else {
                    n.f.run(&u, flags)
                };
                match result {
                    Err(Error::Activation(turborepo_setup::activation::Error::InvalidLock))
                        if flags == CHECK => {}
                    Err(Error::Registry(turborepo_setup::npm_provision::Error::InvalidLock))
                        if flags != CHECK => {}
                    other => {
                        panic!("installed={installed}, case={case}, flags={flags:?}: {other:?}")
                    }
                }
                assert_eq!(u.hits(), 0);
                assert_eq!(
                    publication_guards::state(n.f.owned.root().parent().unwrap()),
                    before
                );
            }
            if !installed {
                assert!(!n.f.owned.root().join(".turbo").exists());
            }
            n.untouched();
        }
    }
}

#[test]
fn npm_local_first_lock_repeat_frozen_force_and_check_use_complete_host_cohort() {
    for bundled in [false, true] {
        let n = NpmFixture::new(bundled);
        let f = &n.f;
        fs::remove_file(f.owned.root().join("turbo.lock")).unwrap();
        assert!(f.check(CHECK).is_err());
        let u = n.upstream();
        assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
        assert_eq!(u.hits(), if bundled { 3 } else { 7 });
        if bundled {
            assert!(u.requests().iter().all(|p| !p.starts_with("/npm/")));
            assert!(matches!(
                n.lock().tools()["npm"].installation,
                Installation::Bundled { .. }
            ));
        } else {
            assert!(matches!(
                n.lock().tools()["npm"].installation,
                Installation::Managed { .. }
            ));
        }
        let before = publication_guards::state(f.owned.root());
        let empty = LoopbackServer::new([], |_| {}).unwrap();
        for flags in [LOCAL, FROZEN] {
            assert_eq!(f.run(&empty, flags).unwrap(), 0);
            assert_eq!(publication_guards::state(f.owned.root()), before);
        }
        assert_eq!(f.check(CHECK).unwrap(), 0);
        assert_eq!(empty.hits(), 0);
        let current = turborepo_tool_install::Store::inspect(f.owned.root())
            .unwrap()
            .unwrap();
        assert_eq!(current.tools.len(), if bundled { 1 } else { 2 });
        assert_eq!(current.tools[0].id, "node");
        for name in ["npm", "npx"] {
            assert!(current.bin.join(name).exists());
            if !bundled {
                let shim = fs::read_to_string(current.bin.join(name)).unwrap();
                assert!(shim.contains("exec \"$tools/node/bin/node\""));
                assert!(shim.contains(&format!("$tools/npm/bin/{name}-cli.js")));
            }
        }
        assert_eq!(
            fs::read(current.bin.join("../tools/node/resource")).unwrap(),
            b"complete Node resource"
        );
        if !bundled {
            assert_eq!(
                fs::read(
                    current
                        .bin
                        .join("../tools/npm/node_modules/resource/data.json")
                )
                .unwrap(),
                b"complete npm resource"
            );
        }
        let lock = fs::read(f.owned.root().join("turbo.lock")).unwrap();
        let force = n.upstream();
        assert_eq!(f.run(&force, FORCE).unwrap(), 0);
        assert_eq!(force.hits(), if bundled { 1 } else { 3 });
        assert!(
            force
                .requests()
                .iter()
                .all(|p| p != "/dist/index.json" && !p.ends_with("SHASUMS256.txt"))
        );
        assert_eq!(fs::read(f.owned.root().join("turbo.lock")).unwrap(), lock);
        assert_eq!(f.check(CHECK).unwrap(), 0);
        n.untouched();
    }
}

#[test]
fn npm_changed_pin_preserves_node_then_removal_reconciles_both_exports() {
    let mut n = NpmFixture::new(false);
    n.f.run(&n.upstream(), FROZEN).unwrap();
    let node = n.lock().tools()["node"].clone();
    let old_lock = fs::read(n.f.owned.root().join("turbo.lock")).unwrap();
    let old_manifest = n.f.manifest().unwrap();
    n.version = "11.1.0".into();
    n.npm = NpmFixture::package(&n.version);
    fs::write(
        n.f.owned.root().join("package.json"),
        r#"{"packageManager":"npm@11.1.0"}"#,
    )
    .unwrap();
    let empty = LoopbackServer::new([], |_| {}).unwrap();
    assert!(n.f.run(&empty, FROZEN).is_err());
    assert!(n.f.check(CHECK).is_err());
    assert_eq!(empty.hits(), 0);
    assert_eq!(n.f.manifest().unwrap(), old_manifest);
    assert_eq!(
        fs::read(n.f.owned.root().join("turbo.lock")).unwrap(),
        old_lock
    );
    let u = n.upstream();
    assert_eq!(n.f.run(&u, LOCAL).unwrap(), 0);
    assert_eq!(u.hits(), 4);
    assert!(u.requests().iter().all(|p| p.starts_with("/npm/")));
    assert_eq!(n.lock().tools()["node"], node);
    assert_eq!(n.lock().tools()["npm"].version, "11.1.0");
    assert_eq!(n.f.check(CHECK).unwrap(), 0);
    fs::write(n.f.owned.root().join("package.json"), "{}").unwrap();
    assert_eq!(n.f.run(&empty, LOCAL).unwrap(), 0);
    assert_eq!(empty.hits(), 0);
    let current = turborepo_tool_install::Store::inspect(n.f.owned.root())
        .unwrap()
        .unwrap();
    assert_eq!(current.tools.len(), 1);
    for name in ["npm", "npx"] {
        assert!(!current.bin.join(name).exists());
    }
    assert_eq!(n.f.check(CHECK).unwrap(), 0);
    n.untouched();
}

#[test]
fn npm_bundled_to_independent_then_pnpm_switch_reconciles_executable_owners() {
    let mut n = NpmFixture::new(true);
    n.f.run(&n.upstream(), FROZEN).unwrap();
    let before = n.lock().tools()["node"].clone();
    n.version = "11.0.0".into();
    n.npm = NpmFixture::package(&n.version);
    fs::write(
        n.f.owned.root().join("package.json"),
        r#"{"packageManager":"npm@11.0.0"}"#,
    )
    .unwrap();
    let u = n.upstream();
    n.f.run(&u, LOCAL).unwrap();
    assert_eq!(u.hits(), 5); // Ownership changes restage the exact same Node artifact.
    let after = n.lock().tools()["node"].clone();
    assert_eq!(after.version, before.version);
    let (Installation::Managed { artifacts: old }, Installation::Managed { artifacts: new }) =
        (&before.installation, &after.installation)
    else {
        panic!("expected managed Node")
    };
    for (platform, parts) in old {
        assert_eq!(
            parts["distribution"].sha256,
            new[platform]["distribution"].sha256
        );
        assert_eq!(parts["distribution"].url, new[platform]["distribution"].url);
        assert_eq!(new[platform]["distribution"].executables.len(), 1);
    }
    assert_eq!(n.f.check(CHECK).unwrap(), 0);
    fs::write(
        n.f.owned.root().join("package.json"),
        r#"{"packageManager":"pnpm@10.0.0"}"#,
    )
    .unwrap();
    let pnpm = Upstream::new(&n.f, false, None);
    assert_eq!(n.f.run(&pnpm, LOCAL).unwrap(), 0);
    assert_eq!(pnpm.hits(), 4);
    assert_eq!(n.lock().tools()["node"], after);
    assert!(!n.lock().tools().contains_key("npm"));
    let current = turborepo_tool_install::Store::inspect(n.f.owned.root())
        .unwrap()
        .unwrap();
    for name in ["npm", "npx"] {
        assert!(!current.bin.join(name).exists());
    }
    for name in ["pnpm", "pnpx"] {
        assert!(current.bin.join(name).exists());
    }
    assert_eq!(n.f.check(CHECK).unwrap(), 0);
    n.untouched();
}

#[test]
fn npm_failed_integrity_or_late_drift_never_publishes_a_partial_generation() {
    for case in 0..4 {
        let n = NpmFixture::new(false);
        let f = &n.f;
        f.run(&n.upstream(), FROZEN).unwrap();
        let before = f.manifest().unwrap();
        let lock = fs::read(f.owned.root().join("turbo.lock")).unwrap();
        let mut routes = n.routes();
        if case == 0 {
            routes.last_mut().unwrap().1 = b"corrupt".to_vec();
        }
        if case == 1 {
            // Internally valid registry SHA-512, but not the committed SHA-256.
            let changed = archive("package", &[("package.json", "changed bytes")]);
            let metadata = &mut routes[3].1;
            let mut value: Value = serde_json::from_slice(metadata).unwrap();
            value["dist"]["integrity"] = json!(format!(
                "sha512-{}",
                first_lock::base64(&Sha512::digest(&changed))
            ));
            *metadata = value.to_string().into_bytes();
            routes.last_mut().unwrap().1 = changed;
        }
        let root = f.owned.root().to_owned();
        let u = LoopbackServer::new(routes, move |p| {
            if p.ends_with(".tgz") {
                match case {
                    2 => fs::write(root.join("package.json"), "{}").unwrap(),
                    3 => fs::write(
                        root.join("apps/web/.npmrc"),
                        "registry=https://private.invalid",
                    )
                    .unwrap(),
                    _ => {}
                }
            }
        })
        .unwrap();
        assert!(f.run(&u, FORCE).is_err(), "case {case}");
        assert_eq!(u.hits(), 3);
        assert_eq!(f.manifest().unwrap(), before);
        assert_eq!(fs::read(f.owned.root().join("turbo.lock")).unwrap(), lock);
        assert_eq!(
            turborepo_tool_install::Store::inspect(f.owned.root())
                .unwrap()
                .unwrap()
                .tools
                .len(),
            2
        );
        n.untouched();
    }
}

#[test]
fn npm_authored_constraints_are_checked_before_traffic_and_bind_reuse() {
    for case in 0..4 {
        let mut n = NpmFixture::new(case == 3);
        let version = n.version.clone();
        n.f.declare(json!({"packageManager":format!("npm@{version}"),"devEngines":{"packageManager":{
            "name":"npm","version":format!("{version}+{}.{}", if case == 0 { "sha1" } else { "sha256" }, "0".repeat(if case == 0 { 40 } else { 64 }))
        }}}));
        if case == 2 {
            n.f.declare(
                json!({"packageManager":format!("npm@{version}"),"devEngines":{"packageManager":[
                    {"name":"npm","version":version},
                    {"name":"npm","version":format!("{version}+sha512.{}","0".repeat(128))}
                ]}}),
            );
        }
        let u = n.upstream();
        assert!(n.f.run(&u, FROZEN).is_err());
        assert_eq!(u.hits(), 0);
        assert!(!n.f.owned.root().join(".turbo").exists());
        n.untouched();
    }
    let mut n = NpmFixture::new(false);
    n.f.run(&n.upstream(), FROZEN).unwrap();
    let before = n.f.manifest().unwrap();
    n.f.declare(
        json!({"packageManager":"npm@11.0.0","devEngines":{"packageManager":{
            "name":"npm","version":format!("11.0.0+sha512.{}","0".repeat(128))
        }}}),
    );
    let u = n.upstream();
    assert!(n.f.run(&u, FROZEN).is_err());
    assert_eq!(u.hits(), 1); // Node reused; authored SHA-512 fails registry preflight.
    assert_eq!(n.f.manifest().unwrap(), before);
    assert!(n.f.check(CHECK).is_err());
    n.untouched();
}

#[test]
fn npm_valid_authored_pins_provision_registry_bytes_even_when_bundled_version_matches() {
    for matching in [false, true] {
        for dev in [false, true] {
            let mut n = NpmFixture::new(false);
            if matching {
                n.version = "11.6.1".into();
                n.npm = NpmFixture::package(&n.version);
            }
            let request = if dev {
                format!("{}+sha512.{:x}", n.version, Sha512::digest(&n.npm))
            } else {
                format!("{}+sha256.{:x}", n.version, Sha256::digest(&n.npm))
            };
            fs::write(
                n.f.owned.root().join("package.json"),
                if dev {
                    json!({"devEngines":{"packageManager":{"name":"npm","version":request}}})
                } else {
                    json!({"packageManager":format!("npm@{request}")})
                }
                .to_string(),
            )
            .unwrap();
            fs::remove_file(n.f.owned.root().join("turbo.lock")).unwrap();
            let u = n.upstream();
            assert_eq!(n.f.run(&u, LOCAL).unwrap(), 0);
            assert_eq!(u.hits(), 7);
            assert!(matches!(
                n.lock().tools()["npm"].installation,
                Installation::Managed { .. }
            ));
            let before = publication_guards::state(n.f.owned.root());
            let empty = LoopbackServer::new([], |_| {}).unwrap();
            assert_eq!(n.f.run(&empty, FROZEN).unwrap(), 0);
            assert_eq!(empty.hits(), 0);
            assert_eq!(publication_guards::state(n.f.owned.root()), before);
            assert_eq!(n.f.check(CHECK).unwrap(), 0);
            n.untouched();
        }
    }
}

#[test]
fn npm_check_is_readonly_with_locks_held_and_detects_damage_and_declaration_drift() {
    for bundled in [false, true] {
        let n = NpmFixture::new(bundled);
        n.f.run(&n.upstream(), FROZEN).unwrap();
        let sources = turborepo_setup::lock::Snapshot::capture(n.f.owned.root()).unwrap();
        let writer = sources.guard().unwrap();
        let store = turborepo_tool_install::Store::open(n.f.owned.root()).unwrap();
        std::thread::scope(|scope| {
            let (tx, rx) = std::sync::mpsc::channel();
            let f = &n.f;
            let check = scope.spawn(move || tx.send(f.check(CHECK)).unwrap());
            let result = rx.recv_timeout(std::time::Duration::from_secs(5));
            drop(store);
            drop(writer);
            check.join().unwrap();
            assert_eq!(result.unwrap().unwrap(), 0);
        });
        let current = turborepo_tool_install::Store::inspect(n.f.owned.root())
            .unwrap()
            .unwrap();
        let resource = if bundled {
            "../tools/node/lib/node_modules/npm/package.json"
        } else {
            "../tools/npm/node_modules/resource/data.json"
        };
        fs::write(current.bin.join(resource), "damage").unwrap();
        let error = n.f.check(CHECK).unwrap_err().to_string();
        assert!(error.contains("turbo setup"), "{error}");
        n.f.run(&n.upstream(), FROZEN).unwrap(); // Normal setup repairs installation drift.
        assert_eq!(n.f.check(CHECK).unwrap(), 0);
        fs::write(
            n.f.owned.root().join("package.json"),
            r#"{"packageManager":"npm@11.2.0"}"#,
        )
        .unwrap();
        assert!(
            n.f.check(CHECK)
                .unwrap_err()
                .to_string()
                .contains("turbo setup")
        );
        n.untouched();
    }
}

/// Bundled npm whose CLI records its arguments and first PATH entry, then
/// runs `body` in the repository.
fn installing(body: &str) -> NpmFixture {
    let mut n = NpmFixture::new(true);
    let prefix = n.f.node_path.rsplit('/').next().unwrap();
    let cli =
        format!("#!/bin/sh\necho \"$*\" >> npm-args\necho \"${{PATH%%:*}}\" > npm-path\n{body}\n");
    n.f.node = first_lock::bundled_with(prefix.trim_end_matches(".tar.gz"), &cli);
    let artifacts = &mut n.f.lock["tools"]["node"]["installation"]["artifacts"];
    let parts = artifacts
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap();
    parts["distribution"]["sha256"] = json!(format!("{:x}", Sha256::digest(&n.f.node)));
    let lock = Lock::parse(n.f.lock.to_string().as_bytes()).unwrap();
    fs::write(
        n.f.owned.root().join("turbo.lock"),
        lock.canonical_bytes().unwrap(),
    )
    .unwrap();
    n
}

impl NpmFixture {
    fn args(&self) -> Vec<String> {
        let args = fs::read_to_string(self.f.owned.root().join("npm-args"));
        args.unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
    fn no_tasks(&self) {
        for path in ["task-ran", "hook-ran", "probe-ran"] {
            assert!(!self.f.owned.root().join(path).exists(), "{path}");
        }
        assert_eq!(
            fs::read(self.f.owned.root().join("package-lock.json")).unwrap(),
            b"dependency lock sentinel"
        );
    }
}

#[test]
fn locked_npm_dependencies_install_with_managed_npm_and_repeat_safely() {
    let n = installing("mkdir -p node_modules && touch node_modules/.installed");
    let root = n.f.owned.root();
    let u = n.upstream();
    assert_eq!(n.f.run(&u, &["--frozen"]).unwrap(), 0);
    assert_eq!(n.args(), ["ci"]);
    // The managed bin directory comes first, and it is the one exporting npm.
    let path = fs::read_to_string(root.join("npm-path")).unwrap();
    assert!(
        std::path::Path::new(path.trim()).join("npm").exists(),
        "{path}"
    );
    assert!(path.contains(".turbo"), "{path}");
    assert!(root.join("node_modules/.installed").exists());
    // A repeat reruns npm ci, so removed dependencies are repaired.
    fs::remove_dir_all(root.join("node_modules")).unwrap();
    assert_eq!(n.f.run(&u, &["--frozen"]).unwrap(), 0);
    assert!(root.join("node_modules/.installed").exists());
    assert_eq!(n.args(), ["ci", "ci"]);
    // Checks never run the installer.
    assert_eq!(n.f.run(&u, &["--check"]).unwrap(), 0);
    assert_eq!(n.args().len(), 2);
    n.no_tasks();
}

#[test]
fn offline_npm_dependencies_install_from_the_npm_cache_without_traffic() {
    // npm fails offline while `cold` exists, like a cache miss.
    let n = installing("[ -f cold ] && exit 4\nmkdir -p node_modules");
    let (root, lock) = (n.f.owned.root(), n.lock());
    let u = n.upstream();
    assert_eq!(n.f.run(&u, &["--frozen"]).unwrap(), 0);
    let hits = u.hits();
    // An offline repeat reuses the trusted tools and runs npm offline.
    fs::remove_dir_all(root.join("node_modules")).unwrap();
    assert_eq!(n.f.run(&u, &["--frozen", "--offline"]).unwrap(), 0);
    assert!(root.join("node_modules").exists());
    // A cache miss passes npm's status through; nothing is reported ready.
    fs::write(root.join("cold"), "").unwrap();
    assert_eq!(n.f.run(&u, &["--offline"]).unwrap(), 4);
    assert_eq!(n.f.run(&u, &["--force", "--offline"]).unwrap(), 4);
    assert_eq!(u.hits(), hits);
    let offline = "ci --offline --no-update-notifier";
    assert_eq!(n.args(), ["ci", offline, offline, offline]);
    let plan = n.f.dependencies(&["--plan", "--offline"]).plan(root, &lock);
    assert_eq!(
        plan.unwrap()[0],
        "javascript dependencies: npm ci --offline --no-update-notifier from package-lock.json \
         with managed npm 11.6.1 (package-lock.json stays unchanged)"
    );
    n.no_tasks();
}

#[test]
fn npm_plans_and_checks_report_the_install_and_policy_without_running_it() {
    let n = installing("touch hook-ran");
    let (root, lock) = (n.f.owned.root(), n.lock());
    let disclosure = "  Permitted lifecycle scripts run package code that can start other \
                      commands. turbo setup runs no Turborepo tasks.";
    assert_eq!(
        n.f.dependencies(&["--plan"]).plan(root, &lock).unwrap(),
        [
            "javascript dependencies: npm ci from package-lock.json with managed npm 11.6.1 \
             (package-lock.json stays unchanged)",
            "  npm 11.6.1: native npm policy; project scripts run; dependency scripts run",
            disclosure,
        ]
    );
    let check = n.f.dependencies(&["--check"]).check(root, &lock).unwrap();
    assert_eq!(
        check[0],
        "javascript dependencies: not checked; turbo setup reruns npm ci from package-lock.json, \
         because dependency readiness is not tracked"
    );
    assert_eq!(check.last().unwrap(), "No tasks run.");
    let skipped = n.f.dependencies(&["--plan", "--ignore-scripts"]);
    assert_eq!(
        skipped.plan(root, &lock).unwrap()[1..],
        [
            "  npm 11.6.1: lifecycle scripts skipped (--ignore-scripts); passing --ignore-scripts \
             to npm"
        ]
    );
    assert_eq!(
        n.f.dependencies(&["--plan", "--tools-only"])
            .plan(root, &lock)
            .unwrap(),
        ["Dependencies skipped (--tools-only); dependency readiness was not checked."]
    );
    // Plans and checks run end to end without running npm.
    let u = n.upstream();
    assert_eq!(n.f.run(&u, &["--plan"]).unwrap(), 0);
    assert_eq!(n.f.run(&u, FROZEN).unwrap(), 0);
    assert_eq!(n.f.run(&u, &["--check"]).unwrap(), 0);
    assert!(n.args().is_empty());
    n.no_tasks();
}

#[test]
fn npm_lifecycle_skip_controls_pass_ignore_scripts_and_tools_only_skips_npm() {
    let n = installing("");
    let u = n.upstream();
    assert_eq!(n.f.run(&u, FROZEN).unwrap(), 0);
    assert!(n.args().is_empty());
    assert_eq!(n.f.run(&u, &["--frozen", "--ignore-scripts"]).unwrap(), 0);
    let skip = r#"{"futureFlags":{"experimentalSetup":true},"setup":{"lifecycleScripts":"skip"}}"#;
    fs::write(n.f.owned.root().join("turbo.json"), skip).unwrap();
    assert_eq!(n.f.run(&u, &["--frozen"]).unwrap(), 0);
    fs::write(n.f.owned.root().join("turbo.json"), ENABLED).unwrap();
    assert_eq!(n.f.run(&u, &["--frozen"]).unwrap(), 0);
    assert_eq!(
        n.args(),
        ["ci --ignore-scripts", "ci --ignore-scripts", "ci"]
    );
    n.no_tasks();
}

#[test]
fn missing_stale_and_rewritten_npm_locks_fail_readably_and_stay_unchanged() {
    let n = installing("");
    let root = n.f.owned.root();
    fs::remove_file(root.join("package-lock.json")).unwrap();
    let u = n.upstream();
    let error = n.f.run(&u, &["--frozen"]).unwrap_err().to_string();
    assert!(
        error.contains("requires a committed npm-shrinkwrap.json or package-lock.json"),
        "{error}"
    );
    // Checked before any download or storage, and never created.
    assert_eq!(u.hits(), 0);
    assert!(!root.join(".turbo").exists() && !root.join("package-lock.json").exists());

    // npm ci's failure status passes through; the next run recovers.
    let n = installing("[ -f stale ] && exit 3\nmkdir -p node_modules");
    let u = n.upstream();
    fs::write(n.f.owned.root().join("stale"), "").unwrap();
    assert_eq!(n.f.run(&u, &["--frozen"]).unwrap(), 3);
    fs::remove_file(n.f.owned.root().join("stale")).unwrap();
    assert_eq!(n.f.run(&u, &["--frozen"]).unwrap(), 0);
    assert!(n.f.owned.root().join("node_modules").exists());
    n.no_tasks();

    // A lockfile the manager rewrites is restored, and setup fails.
    let n = installing("echo rewritten > package-lock.json");
    let error = n.f.run(&n.upstream(), &["--frozen"]).unwrap_err();
    let error = error.to_string();
    assert!(
        error.contains("`npm ci` changed the committed package-lock.json"),
        "{error}"
    );
    n.no_tasks();
}
