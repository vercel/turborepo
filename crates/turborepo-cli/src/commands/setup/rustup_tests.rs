//! Repository-scoped rustup bootstrap through the setup command, against an
//! owned loopback origin. The installer stand-in records any execution.
use turborepo_setup::rustup_provision::RustupTransport;

use super::*;

const FLAGS: &str =
    r#"{"futureFlags":{"experimentalSetup":true,"experimentalCargoWorkspaces":true}}"#;

fn triple() -> &'static str {
    match provision::platform().unwrap() {
        turborepo_setup::lock::Platform::MacosArm64 => "aarch64-apple-darwin",
        turborepo_setup::lock::Platform::MacosX64 => "x86_64-apple-darwin",
        turborepo_setup::lock::Platform::LinuxX64Gnu => "x86_64-unknown-linux-gnu",
        _ => "aarch64-unknown-linux-gnu",
    }
}

struct Repo {
    owned: OwnedSetupFixture,
    path: String,
    installer: String,
}
impl Repo {
    fn new(flags: &str) -> Self {
        let owned = OwnedSetupFixture::new().unwrap();
        let root = owned.root();
        owned.init_git().unwrap();
        fs::write(root.join("turbo.json"), flags).unwrap();
        fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
        fs::write(
            root.join("rust-toolchain.toml"),
            "[toolchain]\nchannel = \"stable\"\n",
        )
        .unwrap();
        let installer = format!(
            "#!/bin/sh\ntouch '{}'\n",
            root.join("installer-ran").display()
        );
        Self {
            owned,
            path: format!("/rustup/archive/1.29.1/{}/rustup-init", triple()),
            installer,
        }
    }
    fn lock(&self) {
        let platform = serde_json::to_value(provision::platform().unwrap()).unwrap();
        let lock = json!({"schemaVersion":0,"tools":{"rustup":{"adapter":"rustup","version":"1.29.1",
            "declarations":[{"file":"rust-toolchain.toml"}],
            "installation":{"kind":"managed","artifacts":{platform.as_str().unwrap():{"rustup-init":{
                "url":format!("https://static.rust-lang.org{}", self.path),
                "sha256":format!("{:x}", Sha256::digest(&self.installer)),
                "format":"binary","executables":{"rustup":"bin/rustup"}}}}}}}});
        let lock = turborepo_setup::lock::Lock::parse(lock.to_string().as_bytes()).unwrap();
        fs::write(
            self.owned.root().join("turbo.lock"),
            lock.canonical_bytes().unwrap(),
        )
        .unwrap();
    }
    fn upstream(&self, corrupt: bool) -> LoopbackServer {
        let body = if corrupt {
            "corrupt"
        } else {
            self.installer.as_str()
        };
        LoopbackServer::new([(self.path.clone(), body.as_bytes().to_vec())], |_| {}).unwrap()
    }
    fn run(&self, upstream: &LoopbackServer, flags: &[&str]) -> Result<i32, Error> {
        let mut words = vec!["turbo".into(), "setup".into(), "--cwd".into()];
        words.push(self.owned.root().as_os_str().to_owned());
        words.extend(flags.iter().map(std::ffi::OsString::from));
        let args = Args::parse_args(words).unwrap();
        let origin = upstream.origin();
        crate::cli::dispatch_setup(&args, |args, setup_args| {
            run_with_policy(
                args,
                setup_args,
                Some(provision::Transports {
                    trust_base: Some(self.owned.home().to_owned()),
                    node: turborepo_setup::node_provision::NodeTransport::loopback_http_for_tests(origin).unwrap(),
                    pnpm: turborepo_setup::pnpm_provision::PnpmTransport::loopback_http_for_tests(origin).unwrap(),
                    registry: turborepo_setup::registry_resolution::RegistryTransport::loopback_http_for_tests(origin).unwrap(),
                    rustup: RustupTransport::loopback_http_for_tests(origin).unwrap(),
                    uv: turborepo_setup::uv_resolution::UvTransport::loopback_http_for_tests(origin).unwrap(),
                }),
                || Ok(self.owned.policy_at(self.owned.root())?),
            )
        })
        .unwrap()
    }
    fn manifest(&self) -> Option<Vec<u8>> {
        fs::read(self.owned.root().join(".turbo/tools/manifest.json")).ok()
    }
}

#[test]
fn locked_rustup_bootstrap_is_private_atomic_and_repeatable() {
    let repo = Repo::new(FLAGS);
    repo.lock();
    let lock = fs::read(repo.owned.root().join("turbo.lock")).unwrap();
    let upstream = repo.upstream(false);
    for flags in [FROZEN, &["--tools-only", "--no-frozen"]] {
        assert_eq!(repo.run(&upstream, flags).unwrap(), 0);
        assert_eq!(upstream.hits(), 1);
    }
    let current = turborepo_tool_install::Store::inspect(repo.owned.root())
        .unwrap()
        .unwrap();
    let shims: Vec<_> = fs::read_dir(&current.bin)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(shims, ["rustup"]);
    assert_eq!(
        fs::read_link(current.bin.join("rustup")).unwrap(),
        std::path::Path::new("../tools/rustup/bin/rustup")
    );
    // Never execute the installer or create a Rust toolchain/user installation.
    // Only the producer's private, external setup receipt may be written.
    assert!(!repo.owned.root().join("installer-ran").exists());
    assert!(!repo.owned.home().join(".cargo").exists());
    assert!(!repo.owned.home().join(".rustup").exists());
    let home_entries: Vec<_> = fs::read_dir(repo.owned.home())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(home_entries, ["turborepo"]);
    let state = repo.owned.home().join("turborepo");
    let state_entries: Vec<_> = fs::read_dir(&state)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(state_entries, ["setup-roots"]);
    let records: Vec<_> = fs::read_dir(state.join("setup-roots"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(records.len(), 1);
    assert!(!records[0].starts_with(repo.owned.root()));
    let receipt: Value = serde_json::from_slice(&fs::read(&records[0]).unwrap()).unwrap();
    assert_eq!(receipt["schema"], 1);
    assert_eq!(
        receipt["lock_sha256"],
        format!("{:x}", Sha256::digest(&lock))
    );
    assert_eq!(
        receipt["installation"]["manifest_sha256"],
        format!("{:x}", Sha256::digest(repo.manifest().unwrap()))
    );
    assert_eq!(
        receipt["installation"]["tools"],
        serde_json::to_value(&current.tools).unwrap()
    );
    assert_eq!(
        fs::read(repo.owned.root().join("turbo.lock")).unwrap(),
        lock
    );

    // A failed fresh bootstrap leaves the published generation selected.
    let manifest = repo.manifest().unwrap();
    let corrupt = repo.upstream(true);
    assert!(
        repo.run(&corrupt, &["--frozen", "--tools-only", "--force"])
            .is_err()
    );
    assert_eq!(repo.manifest().unwrap(), manifest);
}

#[test]
fn failed_first_bootstrap_publishes_no_shims() {
    let repo = Repo::new(FLAGS);
    repo.lock();
    let corrupt = repo.upstream(true);
    assert!(matches!(repo.run(&corrupt, FROZEN), Err(Error::Rustup(_))));
    assert_eq!(corrupt.hits(), 1);
    assert!(repo.manifest().is_none());
    let tools = fs::read_dir(repo.owned.root().join(".turbo/tools")).unwrap();
    assert!(tools.into_iter().all(|entry| {
        let entry = entry.unwrap();
        !entry.path().join("bin").exists()
    }));
}

#[test]
fn toolchain_files_need_the_cargo_flag_and_are_not_resolved_yet() {
    let disabled = Repo::new(ENABLED);
    disabled.lock();
    let upstream = disabled.upstream(false);
    // Ignored declarations make the committed rustup selection stale.
    assert!(disabled.run(&upstream, FROZEN).is_err());
    fs::remove_file(disabled.owned.root().join("turbo.lock")).unwrap();
    let error = disabled
        .run(&upstream, &["--tools-only", "--no-frozen"])
        .unwrap_err();
    assert!(error.to_string().contains("a native Node declaration"));

    let enabled = Repo::new(FLAGS);
    let error = enabled
        .run(&upstream, &["--tools-only", "--no-frozen"])
        .unwrap_err();
    assert!(error.to_string().contains("not implemented yet"), "{error}");
    assert!(!enabled.owned.root().join("turbo.lock").exists());
    assert_eq!(upstream.hits(), 0);
    assert!(matches!(
        enabled.run(&upstream, &["--check", "--tools-only"]),
        Err(Error::Unsupported("Rust toolchain readiness checks"))
    ));
}
