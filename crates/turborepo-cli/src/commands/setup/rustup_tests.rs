//! Repository-scoped rustup and locked Rust toolchains through the setup
//! command, against an owned loopback origin. A stand-in rustup records every
//! invocation and lays out the toolchain it is asked for.
use turborepo_setup::{rust_discovery::Toolchain, rustup_provision::RustupTransport};

use super::*;

const FLAGS: &str =
    r#"{"futureFlags":{"experimentalSetup":true,"experimentalCargoWorkspaces":true}}"#;
const TOOLCHAIN: &str = "[toolchain]\nchannel = \"stable\"\nprofile = \"minimal\"\n";

// Official-shaped channel manifest for this host's minimal profile.
fn manifest() -> String {
    let mut text = "manifest-version = \"2\"\ndate = \"2026-10-01\"\n[profiles]\nminimal = \
                    [\"rustc\", \"cargo\", \"rust-std\"]\n[pkg.rust]\nversion = \"1.99.0 (b9 \
                    2026-10-01)\"\n"
        .to_owned();
    for package in ["rustc", "cargo", "rust-std"] {
        let url = format!(
            "https://static.rust-lang.org/dist/2026-10-01/{package}-1.99.0-{}.tar.gz",
            triple()
        );
        text.push_str(&format!(
            "[pkg.{package}]\nversion = \"1.99.0\"\n[pkg.{package}.target.{}]\navailable = \
             true\nurl = \"{url}\"\nhash = \"{:x}\"\n",
            triple(),
            Sha256::digest(&url)
        ));
    }
    text
}

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
        fs::write(root.join("rust-toolchain.toml"), TOOLCHAIN).unwrap();
        let outside = root.parent().unwrap();
        fs::write(outside.join("manifest.toml"), manifest()).unwrap();
        let installer = format!(
            "#!/bin/sh\nprintf '%s|%s\\n' \"${{0##*/}}\" \"$*\" >> \
             '{log}'\ntc=\"$RUSTUP_HOME/toolchains/$3-{host}\"\nmkdir -p \"$tc/bin\" \
             \"$tc/lib/rustlib\"\nfor exe in cargo rustc rustdoc; do printf 'exit 0\\n' > \
             \"$tc/bin/$exe\"; chmod 755 \"$tc/bin/$exe\"; done\nprintf \
             'cargo-{host}\\nrust-std-{host}\\nrustc-{host}\\n' > \
             \"$tc/lib/rustlib/components\"\ncp '{manifest}' \
             \"$tc/lib/rustlib/multirust-channel-manifest.toml\"\n",
            log = outside.join("rustup-calls").display(),
            host = triple(),
            manifest = outside.join("manifest.toml").display(),
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
        let mut document: turborepo_setup::lock::Document = serde_json::from_value(lock).unwrap();
        let toolchain = fs::read_to_string(self.owned.root().join("rust-toolchain.toml")).unwrap();
        let toolchain = Toolchain::parse("rust-toolchain.toml", &toolchain).unwrap();
        let rust = turborepo_setup::rust_resolution::resolve(&toolchain, manifest().as_bytes());
        document.tools.insert("rust".into(), rust.unwrap());
        let lock = turborepo_setup::lock::Lock::new(document).unwrap();
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
                    node: turborepo_setup::node_provision::NodeTransport::loopback_http_for_tests(origin).unwrap(),
                    pnpm: turborepo_setup::pnpm_provision::PnpmTransport::loopback_http_for_tests(origin).unwrap(),
                    registry: turborepo_setup::registry_resolution::RegistryTransport::loopback_http_for_tests(origin).unwrap(),
                    rustup: RustupTransport::loopback_http_for_tests(origin).unwrap(),
                }),
                || Ok(self.owned.policy_at(self.owned.root())?),
            )
        })
        .unwrap()
    }
    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.owned.root().parent().unwrap().join("rustup-calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
    fn manifest(&self) -> Option<Vec<u8>> {
        fs::read(self.owned.root().join(".turbo/tools/manifest.json")).ok()
    }
}

#[test]
fn locked_rust_setup_is_private_atomic_repeatable_and_reinstallable() {
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
    let mut shims: Vec<_> = fs::read_dir(&current.bin)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    shims.sort();
    assert_eq!(shims, ["cargo", "rustc", "rustdoc", "rustup"]);
    assert_eq!(
        fs::read_link(current.bin.join("cargo")).unwrap(),
        std::path::Path::new(&format!(
            "../tools/rust/toolchains/1.99.0-{}/bin/cargo",
            triple()
        ))
    );
    // One multicall rustup install, never the installer, the caller's home or a
    // lock rewrite.
    let install = "rustup|toolchain install 1.99.0 --no-self-update --profile minimal";
    assert_eq!(repo.calls(), [install]);
    assert_eq!(fs::read_dir(repo.owned.home()).unwrap().count(), 0);
    assert_eq!(
        fs::read(repo.owned.root().join("turbo.lock")).unwrap(),
        lock
    );

    // A failed fresh bootstrap leaves the published generation selected.
    let manifest = repo.manifest().unwrap();
    let corrupt = repo.upstream(true);
    let force = &["--frozen", "--tools-only", "--force"];
    assert!(repo.run(&corrupt, force).is_err());
    assert_eq!(repo.manifest().unwrap(), manifest);
    assert_eq!(repo.calls().len(), 1);
    // Clean reinstall: the same lock, a fresh rustup run and a new generation.
    assert_eq!(repo.run(&upstream, force).unwrap(), 0);
    assert_eq!(repo.calls(), [install, install]);
    assert_ne!(repo.manifest().unwrap(), manifest);
}

#[test]
fn changed_components_or_targets_are_frozen_drift_before_any_install() {
    let repo = Repo::new(FLAGS);
    repo.lock();
    let upstream = repo.upstream(false);
    for changed in [
        TOOLCHAIN.replace(
            "minimal\"",
            "minimal\"\ntargets = [\"wasm32-unknown-unknown\"]",
        ),
        TOOLCHAIN.replace("minimal\"", "minimal\"\ncomponents = [\"clippy\"]"),
    ] {
        fs::write(repo.owned.root().join("rust-toolchain.toml"), changed).unwrap();
        assert!(repo.run(&upstream, FROZEN).is_err());
    }
    assert_eq!(upstream.hits(), 0);
    assert!(repo.calls().is_empty() && repo.manifest().is_none());
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
