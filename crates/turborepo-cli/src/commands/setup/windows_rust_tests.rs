//! Managed Rust on native Windows x64 through the real setup command. The
//! served `rustup-init.exe` is a copy of `cmd.exe`. A fixture runner
//! (System32's `cmd.exe` with a batch stand-in) receives the staged, verified
//! `rustup.exe` path first, byte-checks it, and lays out the toolchain as
//! rustup does. The published `rustup.exe` and its proxies are copies of that
//! image, so the tests run them through the activated PATH.
use std::{ffi::OsString, path::Path};

use turborepo_setup::{rust_discovery::Toolchain, rustup_provision::RustupTransport};

use super::*;

const RUST: &str =
    r#"{"futureFlags":{"experimentalSetup":true,"experimentalCargoWorkspaces":true}}"#;
const TOOLCHAIN: &str = "[toolchain]\nchannel = \"1.99.0\"\nprofile = \"minimal\"\n";
const HOST: &str = "x86_64-pc-windows-msvc";
const INSTALLER: &str = "/rustup/archive/1.29.1/x86_64-pc-windows-msvc/rustup-init.exe";
const INSTALL: &str = "toolchain install 1.99.0 --no-self-update --profile minimal";

/// The official channel manifest shape for the Windows x64 minimal profile.
fn manifest() -> String {
    let mut text = "manifest-version = \"2\"\ndate = \"2026-10-01\"\n[profiles]\nminimal = \
                    [\"rustc\", \"cargo\", \"rust-std\", \"rust-mingw\"]\n[pkg.rust]\nversion = \
                    \"1.99.0 (b9 2026-10-01)\"\n"
        .to_owned();
    for package in ["rustc", "cargo", "rust-std"] {
        let url =
            format!("https://static.rust-lang.org/dist/2026-10-01/{package}-1.99.0-{HOST}.tar.gz");
        text.push_str(&format!(
            "[pkg.{package}]\nversion = \"1.99.0\"\n[pkg.{package}.target.{HOST}]\navailable = \
             true\nurl = \"{url}\"\nhash = \"{:x}\"\n",
            Sha256::digest(&url)
        ));
    }
    text
}

struct Repo {
    owned: OwnedSetupFixture,
    installer: Vec<u8>,
    /// Outside the repository, in the spelling cmd.exe accepts.
    outside: PathBuf,
}

impl Repo {
    fn new() -> Self {
        let owned = OwnedSetupFixture::new().unwrap();
        let root = owned.root();
        owned.init_git().unwrap();
        fs::write(root.join("turbo.json"), RUST).unwrap();
        fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
        fs::write(root.join("rust-toolchain.toml"), TOOLCHAIN).unwrap();
        let outside = dunce::simplified(root.parent().unwrap()).to_path_buf();
        fs::write(outside.join("manifest.toml"), manifest()).unwrap();
        // Each step traces its errorlevel, for failure messages.
        let out = outside.display();
        let trace = format!(">>\"{out}\\rustup-trace\" echo");
        let script = [
            "@echo off".to_owned(),
            format!("{trace} [%CMDCMDLINE%] [%CD%] [%RUSTUP_HOME%]"),
            format!(">>\"{out}\\rustup-calls\" echo %*"),
            // %1 is the staged rustup.exe: the verified installer's bytes.
            "\"%SystemRoot%\\System32\\fc.exe\" /b \"%~1\" \"%SystemRoot%\\System32\\cmd.exe\" \
             >nul || exit /b 9"
                .to_owned(),
            format!("set \"tc=%RUSTUP_HOME%\\toolchains\\%4-{HOST}\""),
            "mkdir \"%tc%\\bin\" \"%tc%\\lib\\rustlib\"".to_owned(),
            format!("{trace} mkdir %ERRORLEVEL%"),
            "for %%e in (cargo rustc rustdoc) do >\"%tc%\\bin\\%%e.exe\" echo %%e".to_owned(),
            format!(
                "(echo cargo-{HOST}& echo rust-std-{HOST}& echo rustc-{HOST}) \
                 >\"%tc%\\lib\\rustlib\\components\""
            ),
            format!("{trace} layout %ERRORLEVEL%"),
            "set \"manifest=%tc%\\lib\\rustlib\\multirust-channel-manifest.toml\"".to_owned(),
            format!("copy /y \"{out}\\manifest.toml\" \"%manifest%\" >nul"),
            format!("{trace} copy %ERRORLEVEL%"),
            format!("if not exist \"%manifest%\" type \"{out}\\manifest.toml\" >\"%manifest%\""),
            format!("{trace} type %ERRORLEVEL%"),
            ">\"%RUSTUP_HOME%\\settings.toml\" echo version = \"12\"".to_owned(),
            "if exist \"%manifest%\" exit /b 0".to_owned(),
        ]
        .join("\r\n");
        fs::write(outside.join("rustup.cmd"), script).unwrap();
        Self {
            installer: fs::read(system32().join("cmd.exe")).unwrap(),
            owned,
            outside,
        }
    }

    fn lock(&self) {
        let lock = json!({"schemaVersion":0,"tools":{"rustup":{"adapter":"rustup","version":"1.29.1",
            "declarations":[{"file":"rust-toolchain.toml"}],
            "installation":{"kind":"managed","artifacts":{"windows-x64":{"rustup-init":{
                "url":format!("https://static.rust-lang.org{INSTALLER}"),
                "sha256":format!("{:x}", Sha256::digest(&self.installer)),
                "format":"binary","executables":{"rustup":"bin/rustup.exe"}}}}}}}});
        let mut document: turborepo_setup::lock::Document = serde_json::from_value(lock).unwrap();
        let toolchain = Toolchain::parse("rust-toolchain.toml", TOOLCHAIN).unwrap();
        let rust = turborepo_setup::rust_resolution::resolve(&toolchain, manifest().as_bytes());
        document.tools.insert("rust".into(), rust.unwrap());
        let lock = turborepo_setup::lock::Lock::new(document).unwrap();
        let bytes = lock.canonical_bytes().unwrap();
        fs::write(self.owned.root().join("turbo.lock"), bytes).unwrap();
    }

    fn upstream(&self) -> LoopbackServer {
        LoopbackServer::new([(INSTALLER.into(), self.installer.clone())], |_| {}).unwrap()
    }

    /// Official release metadata, every platform's checksum and the channel.
    fn resolver(&self) -> LoopbackServer {
        let digest = format!("{:x}\n", Sha256::digest(&self.installer));
        let release = b"schema-version = '1'\nversion = '1.29.1'\n".to_vec();
        let mut routes = vec![
            (INSTALLER.to_owned(), self.installer.clone()),
            ("/rustup/release-stable.toml".into(), release),
            ("/dist/channel-rust-1.99.0.toml".into(), manifest().into()),
        ];
        for arch in ["x86_64", "aarch64"] {
            for os in ["apple-darwin", "unknown-linux-gnu", "unknown-linux-musl"] {
                let path = format!("/rustup/archive/1.29.1/{arch}-{os}/rustup-init.sha256");
                routes.push((path, digest.clone().into_bytes()));
            }
            let path =
                format!("/rustup/archive/1.29.1/{arch}-pc-windows-msvc/rustup-init.exe.sha256");
            routes.push((path, digest.clone().into_bytes()));
        }
        LoopbackServer::new(routes, |_| {}).unwrap()
    }

    fn run(&self, upstream: &LoopbackServer, flags: &[&str]) -> Result<i32, Error> {
        let mut words = vec!["turbo".into(), "setup".into(), "--cwd".into()];
        words.push(self.owned.root().as_os_str().to_owned());
        words.extend(flags.iter().map(OsString::from));
        let args = Args::parse_args(words).unwrap();
        let origin = upstream.origin();
        let script = self.outside.join("rustup.cmd");
        let runner = (
            system32().join("cmd.exe"),
            [OsString::from("/d"), "/c".into(), script.into()],
        );
        crate::cli::dispatch_setup(&args, |args, setup_args| {
            run_with_policy(
                args,
                setup_args,
                Some(provision::Transports {
                    node: turborepo_setup::node_provision::NodeTransport::loopback_http_for_tests(origin).unwrap(),
                    pnpm: turborepo_setup::pnpm_provision::PnpmTransport::loopback_http_for_tests(origin).unwrap(),
                    registry: turborepo_setup::registry_resolution::RegistryTransport::loopback_http_for_tests(origin).unwrap(),
                    rustup: RustupTransport::loopback_http_for_tests(origin)
                        .unwrap()
                        .with_runner_for_tests(runner.0.clone(), runner.1.clone()),
                    uv: turborepo_setup::uv_resolution::UvTransport::loopback_http_for_tests(origin).unwrap(),
                    python: turborepo_setup::python_resolution::PythonTransport::loopback_http_for_tests(origin).unwrap(),
                    go: turborepo_setup::go_resolution::GoTransport::loopback_http_for_tests(origin).unwrap(),
                    bun: turborepo_setup::bun_provision::BunTransport::loopback_http_for_tests(origin).unwrap(),
                    artifact: None,
                    trust_base: Some(self.owned.home().to_owned()),
                }),
                || Ok(self.owned.policy_at(self.owned.root())?),
            )
        })
        .unwrap()
    }

    /// Setup that must succeed, with the stand-in's trace on failure.
    fn ok(&self, upstream: &LoopbackServer, flags: &[&str]) -> i32 {
        let status = self.run(upstream, flags);
        let trace = fs::read_to_string(self.outside.join("rustup-trace")).unwrap_or_default();
        let cwd = std::env::current_dir();
        let outside = self.outside.display();
        status.unwrap_or_else(|error| panic!("{error:?}\noutside={outside} cwd={cwd:?}\n{trace}"))
    }

    /// Each install's rustup arguments, after checking it ran the staged image.
    fn calls(&self) -> Vec<String> {
        let calls = fs::read_to_string(self.outside.join("rustup-calls")).unwrap_or_default();
        let staged = |line: &str| {
            let (exe, args) = line.split_once(' ').unwrap();
            assert!(exe.ends_with(r"\bin\rustup.exe"), "{line}");
            assert!(exe.contains(r"\.turbo\tools\"), "{line}");
            args.to_owned()
        };
        calls.lines().map(staged).collect()
    }

    fn manifest(&self) -> Option<Vec<u8>> {
        fs::read(self.owned.root().join(".turbo/tools/manifest.json")).ok()
    }

    /// What managed `turbo run` and `turbo exec` apply, plus the managed bin.
    fn overrides(&self) -> (turborepo_run::EnvironmentVariableMap, PathBuf) {
        use crate::commands::managed::{ManagedTools, context};
        let root = self.owned.root();
        let tools = ManagedTools::inspect(
            root,
            &root.join("turbo.json"),
            true,
            self.owned.home(),
            context,
        );
        let env = tools.unwrap().unwrap().revalidate().unwrap();
        let bin = std::env::split_paths(&env["PATH"]).next().unwrap();
        (env, bin)
    }
}

#[test]
fn frozen_rust_setup_runs_the_verified_rustup_and_activates_its_proxies() {
    if unqualified_host() {
        return;
    }
    let repo = Repo::new();
    repo.lock();
    let lock = fs::read(repo.owned.root().join("turbo.lock")).unwrap();
    let upstream = repo.upstream();
    assert_eq!(repo.ok(&upstream, FROZEN), 0);
    assert_eq!(upstream.requests(), [INSTALLER]);
    // One private install by the staged rustup.exe, never rustup-init's setup.
    assert_eq!(repo.calls(), [INSTALL]);

    let (env, bin) = repo.overrides();
    let mut names: Vec<_> = fs::read_dir(&bin)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(
        names,
        ["cargo.exe", "rustc.exe", "rustdoc.exe", "rustup.exe"]
    );
    let tools = bin.join("..").join("tools");
    let proxy = tools.join("rust").join("proxy").join("rustup.exe");
    for name in ["cargo.exe", "rustc.exe", "rustdoc.exe"] {
        assert!(
            same_file::is_same_file(bin.join(name), &proxy).unwrap(),
            "{name}"
        );
    }
    let rustup = tools.join("rustup").join("bin").join("rustup.exe");
    assert!(same_file::is_same_file(bin.join("rustup.exe"), rustup).unwrap());
    assert_eq!(fs::read(&proxy).unwrap(), repo.installer);

    // Proxies select the private toolchain from these, never user state.
    assert_eq!(env["RUSTUP_TOOLCHAIN"], format!("1.99.0-{HOST}"));
    assert_eq!(env["RUSTUP_AUTO_INSTALL"], "0");
    let home = Path::new(&env["RUSTUP_HOME"]);
    assert!(same_file::is_same_file(home, tools.join("rust")).unwrap());
    let toolchain = home.join("toolchains").join(format!("1.99.0-{HOST}"));
    assert!(toolchain.join("bin").join("rustc.exe").is_file());
    let cargo_home = repo.owned.root().join(".turbo").join("cargo-home");
    assert_eq!(
        Path::new(&env["CARGO_HOME"]),
        dunce::simplified(&cargo_home)
    );
    for key in ["PATH", "RUSTUP_HOME", "CARGO_HOME"] {
        assert!(!env[key].starts_with(r"\\?\"), "{key}");
    }
    // PATHEXT resolves each name to its proxy (a cmd.exe image here).
    let path = std::ffi::OsStr::new(&env["PATH"]);
    assert_eq!(exit_code(path, "cargo /d /c exit 7"), Some(7));
    assert_eq!(exit_code(path, "rustc /d /c exit 8"), Some(8));
    // The published verified rustup.exe runs too.
    assert_eq!(exit_code(path, "rustup /d /c exit 5"), Some(5));

    // Repeats and checks reuse everything without traffic or a rustup run.
    let manifest = repo.manifest();
    for flags in [FROZEN, CHECK] {
        assert_eq!(repo.ok(&upstream, flags), 0);
    }
    assert_eq!((upstream.hits(), repo.calls().len()), (1, 1));
    assert_eq!(repo.manifest(), manifest);
    assert_eq!(
        fs::read(repo.owned.root().join("turbo.lock")).unwrap(),
        lock
    );
    // Setup's trusted-root record is the only user state; no ~/.rustup.
    let home: Vec<_> = fs::read_dir(repo.owned.home())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name != "turborepo")
        .collect();
    assert_eq!(home, Vec::<OsString>::new());

    // A damaged proxy is reported by --check, and setup reinstalls it.
    fs::remove_file(bin.join("rustc.exe")).unwrap();
    assert!(repo.run(&upstream, CHECK).is_err());
    assert_eq!(repo.ok(&upstream, FROZEN), 0);
    assert_eq!(repo.calls(), [INSTALL, INSTALL]);
    assert_ne!(repo.manifest(), manifest);
    assert_eq!(repo.ok(&upstream, CHECK), 0);
}

#[test]
fn local_rust_setup_locks_windows_artifacts_then_reuses_them_offline() {
    if unqualified_host() {
        return;
    }
    let repo = Repo::new();
    assert_eq!(repo.ok(&repo.resolver(), LOCAL), 0);
    let lock: Value =
        serde_json::from_slice(&fs::read(repo.owned.root().join("turbo.lock")).unwrap()).unwrap();
    let rust = &lock["tools"]["rust"]["installation"]["artifacts"]["windows-x64"];
    assert_eq!(rust["cargo"]["executables"]["cargo"], "bin/cargo.exe");
    let rustup = &lock["tools"]["rustup"]["installation"]["artifacts"];
    assert_eq!(rustup.as_object().unwrap().len(), 8);
    let windows = &rustup["windows-x64"]["rustup-init"];
    assert_eq!(
        windows["url"],
        format!("https://static.rust-lang.org{INSTALLER}")
    );
    assert_eq!(windows["executables"]["rustup"], "bin/rustup.exe");
    assert_eq!(repo.calls(), [INSTALL]);

    let manifest = repo.manifest();
    let offline = LoopbackServer::new([], |_| {}).unwrap();
    for flags in [FROZEN, LOCAL, CHECK] {
        assert_eq!(repo.ok(&offline, flags), 0);
    }
    assert_eq!(offline.hits(), 0);
    assert_eq!((repo.calls().len(), repo.manifest()), (1, manifest));
}

#[test]
fn a_failed_private_install_publishes_nothing() {
    if unqualified_host() {
        return;
    }
    let repo = Repo::new();
    repo.lock();
    fs::write(repo.outside.join("rustup.cmd"), "@exit /b 3\r\n").unwrap();
    // The stand-in's own status proves the staged rustup.exe ran it.
    use turborepo_setup::rust_provision::Error::Rustup;
    let result = repo.run(&repo.upstream(), FROZEN);
    assert!(
        matches!(&result, Err(Error::Rust(Rustup(status))) if status.code() == Some(3)),
        "{result:?}"
    );
    assert!(repo.manifest().is_none());
}
