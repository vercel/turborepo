//! Repository-scoped rustup and locked Rust toolchains through the setup
//! command, against an owned loopback origin. A stand-in rustup records every
//! invocation and lays out the toolchain it is asked for.
use turborepo_setup::{
    activation::Error as ActivationError, rust_discovery::Toolchain,
    rustup_provision::RustupTransport,
};

use super::*;

const FLAGS: &str =
    r#"{"futureFlags":{"experimentalSetup":true,"experimentalCargoWorkspaces":true}}"#;
const CHECK: &[&str] = &["--check", "--tools-only"];
const TOOLCHAIN: &str = "[toolchain]\nchannel = \"stable\"\nprofile = \"minimal\"\n";

// Official-shaped channel manifest for this host's minimal profile, plus a
// target-independent rust-src.
fn manifest(version: &str) -> String {
    let mut text = format!(
        "manifest-version = \"2\"\ndate = \"2026-10-01\"\n[profiles]\nminimal = [\"rustc\", \
         \"cargo\", \"rust-std\"]\n[pkg.rust]\nversion = \"{version} (b9 2026-10-01)\"\n"
    );
    for package in ["rustc", "cargo", "rust-std", "rust-src"] {
        let (target, suffix) = match package {
            "rust-src" => ("\"*\"".to_owned(), String::new()),
            _ => (triple().to_owned(), format!("-{}", triple())),
        };
        let url = format!(
            "https://static.rust-lang.org/dist/2026-10-01/{package}-{version}{suffix}.tar.gz"
        );
        text.push_str(&format!(
            "[pkg.{package}]\nversion = \"{version}\"\n[pkg.{package}.target.{target}]\navailable \
             = true\nurl = \"{url}\"\nhash = \"{:x}\"\n",
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
        for version in ["1.99.0", "1.100.0"] {
            let path = outside.join(format!("manifest-{version}.toml"));
            fs::write(path, manifest(version)).unwrap();
        }
        let installer = r#"#!/bin/sh
printf '%s|%s\n' "${0##*/}" "$*" >> 'OUTSIDE/rustup-calls'
tc="$RUSTUP_HOME/toolchains/$3-HOST"
mkdir -p "$tc/bin" "$tc/lib/rustlib"
for exe in cargo rustc rustdoc; do printf 'exit 0\n' > "$tc/bin/$exe"; chmod 755 "$tc/bin/$exe"; done
printf 'cargo-HOST\nrust-std-HOST\nrustc-HOST\n' > "$tc/lib/rustlib/components"
case " $* " in *" rust-src "*) echo rust-src >> "$tc/lib/rustlib/components";; esac
cp "OUTSIDE/manifest-$3.toml" "$tc/lib/rustlib/multirust-channel-manifest.toml"
"#
        .replace("OUTSIDE", &outside.display().to_string())
        .replace("HOST", triple());
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
        let rust =
            turborepo_setup::rust_resolution::resolve(&toolchain, manifest("1.99.0").as_bytes());
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
    // Official release metadata and checksums plus the installer.
    fn resolver(&self, stable: &str) -> LoopbackServer {
        let release = b"schema-version = '1'\nversion = '1.29.1'\n".to_vec();
        let mut routes = vec![
            (self.path.clone(), self.installer.clone().into_bytes()),
            ("/rustup/release-stable.toml".into(), release),
            (
                "/dist/channel-rust-stable.toml".into(),
                manifest(stable).into_bytes(),
            ),
        ];
        for version in ["1.99.0", "1.100.0"] {
            let path = format!("/dist/channel-rust-{version}.toml");
            routes.push((path, manifest(version).into_bytes()));
        }
        let digest = format!("{:x}\n", Sha256::digest(&self.installer)).into_bytes();
        for (arch, os) in ["x86_64", "aarch64"].into_iter().flat_map(|arch| {
            [
                "apple-darwin",
                "unknown-linux-gnu",
                "unknown-linux-musl",
                "pc-windows-msvc",
            ]
            .map(|os| (arch, os))
        }) {
            let exe = if os.contains("windows") { ".exe" } else { "" };
            let path = format!("/rustup/archive/1.29.1/{arch}-{os}/rustup-init{exe}.sha256");
            routes.push((path, digest.clone()));
        }
        LoopbackServer::new(routes, |_| {}).unwrap()
    }
    fn locked(&self, id: &str) -> turborepo_setup::lock::Tool {
        let lock = turborepo_setup::lock::Lock::read(self.owned.root())
            .unwrap()
            .unwrap();
        lock.tools()[id].clone()
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
                    uv: turborepo_setup::uv_resolution::UvTransport::loopback_http_for_tests(origin).unwrap(),
                    python: turborepo_setup::python_resolution::PythonTransport::loopback_http_for_tests(origin).unwrap(),
                    go: turborepo_setup::go_resolution::GoTransport::loopback_http_for_tests(origin).unwrap(),
                    bun: turborepo_setup::bun_provision::BunTransport::loopback_http_for_tests(origin).unwrap(),
                    artifact: None,
                    trust_base: Some(self.owned.home().to_owned()),
                    cache: None,
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
    // Setup's own trusted-root record (stack B) is the only expected entry.
    let home: Vec<_> = fs::read_dir(repo.owned.home())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name != "turborepo")
        .collect();
    assert_eq!(home, Vec::<std::ffi::OsString>::new());
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
fn managed_run_activates_locked_rust_in_cargo_repositories_by_policy() {
    let repo = Repo::new(FLAGS);
    repo.lock();
    assert_eq!(repo.run(&repo.upstream(false), FROZEN).unwrap(), 0);
    let (env, bin) = run_overrides(&repo.owned).unwrap();
    assert!(bin.join("cargo").exists() && bin.join("rustc").exists());
    assert_eq!(env["RUSTUP_TOOLCHAIN"], format!("1.99.0-{}", triple()));
    let home = std::path::Path::new(&env["RUSTUP_HOME"]);
    assert!(home.join("toolchains").is_dir(), "{env:?}");
    let cargo_home = repo.owned.root().join(".turbo/cargo-home");
    assert_eq!(std::path::Path::new(&env["CARGO_HOME"]), cargo_home);
    // Like setup, a Rust skip policy ignores rust-toolchain.toml, so the locked
    // selection is drift rather than an activation.
    let skip = r#"{"futureFlags":{"experimentalSetup":true,"experimentalCargoWorkspaces":true},"setup":{"rust":{"tools":"skip"}}}"#;
    fs::write(repo.owned.root().join("turbo.json"), skip).unwrap();
    let error = run_overrides(&repo.owned).unwrap_err();
    assert!(error.contains("no longer match turbo.lock"), "{error}");
    assert!(!cargo_home.exists(), "inspection never creates CARGO_HOME");
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
fn toolchain_files_need_the_cargo_flag() {
    let disabled = Repo::new(ENABLED);
    disabled.lock();
    let upstream = disabled.upstream(false);
    // Ignored declarations make the committed Rust selections stale.
    assert!(disabled.run(&upstream, FROZEN).is_err());
    fs::remove_file(disabled.owned.root().join("turbo.lock")).unwrap();
    let error = disabled
        .run(&upstream, &["--tools-only", "--no-frozen"])
        .unwrap_err();
    assert!(error.to_string().contains("a native Node declaration"));
    assert_eq!(upstream.hits(), 0);
    // Check ignores them the same way, so a Rust lock is drift there too.
    disabled.lock();
    assert!(matches!(
        disabled.run(&upstream, CHECK),
        Err(Error::Activation(ActivationError::DeclarationDrift))
    ));
}

#[test]
fn rust_readiness_checks_are_read_only_and_detect_damage() {
    let repo = Repo::new(FLAGS);
    repo.lock();
    let upstream = repo.upstream(false);
    assert!(matches!(
        repo.run(&upstream, CHECK),
        Err(Error::Activation(ActivationError::MissingInventory))
    ));
    assert_eq!(repo.manifest(), None);
    assert_eq!(repo.run(&upstream, FROZEN).unwrap(), 0);
    let manifest = repo.manifest();
    assert_eq!(repo.run(&upstream, CHECK).unwrap(), 0);
    // No download, rustup run, Cargo home or inventory change.
    assert_eq!((upstream.hits(), repo.calls().len()), (1, 1));
    assert_eq!(repo.manifest(), manifest);
    assert!(!repo.owned.root().join(".turbo/cargo-home").exists());

    // A damaged toolchain file is reported, never repaired.
    let current = turborepo_tool_install::Store::inspect(repo.owned.root())
        .unwrap()
        .unwrap();
    fs::write(current.bin.join("rustdoc"), "exit 1\n").unwrap();
    assert!(matches!(
        repo.run(&upstream, CHECK),
        Err(Error::Activation(ActivationError::DamagedInventory(_)))
    ));
    assert_eq!(repo.manifest(), manifest);
}

#[test]
fn local_setup_creates_targets_and_refreshes_rust_selections() {
    let repo = Repo::new(FLAGS);
    let file = repo.owned.root().join("rust-toolchain.toml");
    let local = &["--tools-only", "--no-frozen"];
    assert_eq!(repo.run(&repo.resolver("1.99.0"), local).unwrap(), 0);
    assert_eq!(repo.locked("rust").version, "1.99.0");
    let rustup = repo.locked("rustup");
    assert_eq!(rustup.version, "1.29.1");

    // A newer stable exists, but a component edit keeps the locked release.
    let newer = repo.resolver("1.100.0");
    let edited = TOOLCHAIN.replace("minimal\"", "minimal\"\ncomponents = [\"rust-src\"]");
    fs::write(&file, &edited).unwrap();
    assert_eq!(repo.run(&newer, local).unwrap(), 0);
    assert_eq!(newer.requests(), ["/dist/channel-rust-1.99.0.toml"]);
    assert_eq!(repo.locked("rust").version, "1.99.0");
    assert_eq!(repo.locked("rust").options["components"], ["rust-src"]);
    assert_eq!(repo.locked("rustup"), rustup);

    // Frozen setup rejects the stale declaration without writing or installing.
    let lock = fs::read(repo.owned.root().join("turbo.lock")).unwrap();
    fs::write(&file, TOOLCHAIN).unwrap();
    assert!(repo.run(&newer, FROZEN).is_err());
    assert_eq!(
        fs::read(repo.owned.root().join("turbo.lock")).unwrap(),
        lock
    );
    fs::write(&file, &edited).unwrap();

    // Only an explicit refresh moves the floating channel.
    let refresh = &["--tools-only", "--no-frozen", "--update-lock"];
    assert_eq!(repo.run(&newer, refresh).unwrap(), 0);
    assert_eq!(repo.locked("rust").version, "1.100.0");
    let installs = "rustup|toolchain install VERSION --no-self-update --profile minimal";
    assert_eq!(
        repo.calls(),
        [
            installs.replace("VERSION", "1.99.0"),
            installs.replace("VERSION", "1.99.0") + " --component rust-src",
            installs.replace("VERSION", "1.100.0") + " --component rust-src",
        ]
    );
}
