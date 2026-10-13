//! `turbo setup` then plain `turbo run` for each managed ecosystem through the
//! built `turbo` binary. Setup runs the CLI's own `turbo setup` against
//! loopback release artifacts. `turbo run` starts from a cleared environment
//! whose PATH holds only Git and refusing stand-ins for every language tool,
//! with dead proxies, so a host tool or a download would be visible. Mock
//! toolchains log every invocation, so the log proves that turbo's own probes
//! and the tasks used them.
#![cfg(all(
    unix,
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))
))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "setup_run_ecosystems/go.rs"]
mod go;
#[path = "setup_run_ecosystems/rust.rs"]
mod rust;
#[path = "setup_run_ecosystems/uv.rs"]
mod uv;
#[path = "setup_run_ecosystems/yarn_bun.rs"]
mod yarn_bun;

use std::{
    fs,
    io::Write,
    net::{TcpListener, TcpStream},
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

use turborepo_setup::test_support::{LoopbackServer, OwnedSetupFixture};
use turborepo_shim::TurboState;

/// Language tools a managed run must never take from the host PATH.
const HOST_TOOLS: &[&str] = &[
    "cargo", "rustc", "rustup", "go", "gofmt", "uv", "python", "python3", "node", "npm", "pnpm",
    "yarn", "bun",
];
/// Local tools-only setup, which creates turbo.lock even when CI is detected.
const LOCAL: &[&str] = &["--tools-only", "--no-frozen"];
/// Strict mode filters this caller variable; loose mode passes it.
const SECRET: &str = "JOURNEY_SECRET";
/// Commands that discover the repository without running tasks, besides
/// `turbo devtools`, and whether their lazy discovery probes a toolchain in
/// these fixtures. `run` and `watch` without tasks list tasks and exit 1.
const DISCOVERY: &[(&[&str], bool)] = &[
    (&["ls"], false),
    (&["ls", "app"], true),
    (&["query", "query { packages { items { name } } }"], true),
    (&["boundaries"], false),
    (&["run"], false),
    (&["watch"], false),
];

/// Guidance text without whitespace or the error frame's borders.
fn squash(text: &str) -> String {
    text.split_whitespace()
        .collect::<String>()
        .replace(['|', '│'], "")
}

fn executable(path: &Path, contents: &str) {
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn tar_gz(prefix: &str, files: &[(&str, &str)]) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for (path, body) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, format!("{prefix}/{path}"), body.as_bytes())
            .unwrap();
    }
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&tar.into_inner().unwrap()).unwrap();
    gzip.finish().unwrap()
}

struct Journey {
    owned: OwnedSetupFixture,
    upstream: LoopbackServer,
}

impl Journey {
    fn new(files: &[(&str, &str)], routes: Vec<(String, Vec<u8>)>) -> Self {
        let owned = OwnedSetupFixture::new().unwrap();
        owned.init_git().unwrap();
        for (name, contents) in files {
            let path = owned.root().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        let host = owned.home().join("host-bin");
        fs::create_dir(&host).unwrap();
        symlink(which::which("git").unwrap(), host.join("git")).unwrap();
        let marker = owned.home().join("host-ran");
        for tool in HOST_TOOLS {
            let script = format!(
                "#!/bin/sh\necho {tool} >> '{}'\nexit 97\n",
                marker.display()
            );
            executable(&host.join(tool), &script);
        }
        Self {
            upstream: LoopbackServer::new(routes, |_| {}).unwrap(),
            owned,
        }
    }

    fn root(&self) -> &Path {
        self.owned.root()
    }

    /// Where setup records trust and run reads it: the user data directory.
    fn user_base(&self) -> PathBuf {
        if cfg!(target_os = "macos") {
            self.owned.home().join("Library/Application Support")
        } else {
            self.owned.home().join("data")
        }
    }

    /// `turbo setup <flags>` from `dir`, against the loopback upstream only.
    fn setup(&self, dir: &str, flags: &[&str]) {
        let invocation = self.root().join(dir);
        let code = turborepo_cli::test_support::setup_for_tests(
            &invocation,
            &self.user_base(),
            self.upstream.origin(),
            flags,
            || self.owned.policy_at(&invocation),
        )
        .unwrap();
        assert_eq!(code, 0);
    }

    /// Plain `turbo <args>` from `dir`. It never downloads or runs a host tool.
    fn turbo(&self, dir: &str, args: &[&str]) -> (i32, String) {
        self.turbo_with(dir, args, &[])
    }

    fn turbo_with(&self, dir: &str, args: &[&str], env: &[(&str, &str)]) -> (i32, String) {
        let hits = self.upstream.hits();
        let output = self.command(dir, args, env).output().unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        self.hermetic(hits, &text);
        (output.status.code().unwrap_or(-1), text)
    }

    /// No download and no host tool since the upstream had `hits`.
    fn hermetic(&self, hits: usize, text: &str) {
        assert_eq!(self.upstream.hits(), hits, "turbo downloaded: {text}");
        let host = fs::read_to_string(self.owned.home().join("host-ran")).unwrap_or_default();
        assert!(host.is_empty(), "host tools ran ({host}): {text}");
    }

    fn command(&self, dir: &str, args: &[&str], env: &[(&str, &str)]) -> Command {
        let home = self.owned.home();
        let mut command = Command::new(env!("CARGO_BIN_EXE_turbo"));
        command
            .current_dir(self.root().join(dir))
            .args(args)
            .env_clear()
            .env("HOME", home)
            .env("XDG_DATA_HOME", home.join("data"))
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("TURBO_CONFIG_DIR_PATH", home.join("config"))
            .env("PATH", home.join("host-bin"))
            .env("TURBO_NO_UPDATE_NOTIFIER", "1")
            .env("TURBO_TELEMETRY_DISABLED", "1")
            .env("DO_NOT_TRACK", "1")
            .env("HTTP_PROXY", "http://127.0.0.1:9")
            .env("HTTPS_PROXY", "http://127.0.0.1:9")
            .env("ALL_PROXY", "http://127.0.0.1:9")
            .env(SECRET, "leaked")
            .envs(env.iter().copied());
        command
    }

    /// `turbo devtools` until it serves, which follows its first graph build,
    /// or its exit code and output when it stops first.
    fn devtools(&self, env: &[(&str, &str)]) -> (Option<i32>, String) {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let log = self.owned.home().join("devtools.log");
        let file = fs::File::create(&log).unwrap();
        let hits = self.upstream.hits();
        let port_arg = port.to_string();
        let mut child = self
            .command("", &["devtools", "--no-open", "--port", &port_arg], env)
            .stdout(file.try_clone().unwrap())
            .stderr(file)
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        let code = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break Some(status.code().unwrap_or(-1));
            }
            let served = TcpStream::connect(("127.0.0.1", port)).is_ok();
            if served || Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                assert!(served, "devtools neither served nor exited");
                break None;
            }
            thread::sleep(Duration::from_millis(100));
        };
        let text = fs::read_to_string(&log).unwrap();
        self.hermetic(hits, &text);
        (code, text)
    }

    /// Every command that discovers the repository without running tasks
    /// probes with the managed tools, logging `probe`, like `turbo run`.
    fn discovers_managed(&self, probe: &str) {
        for (args, probes) in DISCOVERY {
            let (code, text) = self.turbo("", args);
            assert!(
                code == 0 || text.contains("No tasks provided"),
                "{args:?}: {text}"
            );
            let calls = self.calls();
            assert!(
                !probes || calls.contains(probe),
                "{probe} in {args:?}: {calls}"
            );
        }
        let (code, text) = self.devtools(&[]);
        assert_eq!(code, None, "{text}");
        assert!(self.calls().contains(probe), "{probe} in devtools: {text}");
    }

    /// Every discovery command fails before discovery with `guidance`.
    fn discovery_fails(&self, guidance: &str, env: &[(&str, &str)]) {
        for (args, _) in DISCOVERY {
            let (code, text) = self.turbo_with("", args, env);
            assert_ne!(code, 0, "{args:?}: {text}");
            assert!(
                squash(&text).contains(&squash(guidance)),
                "{args:?}: {text}"
            );
        }
        let (code, text) = self.devtools(env);
        assert!(code.is_some_and(|code| code != 0), "{text}");
        assert!(squash(&text).contains(&squash(guidance)), "{text}");
        assert_eq!(self.calls(), "", "a probe ran");
    }

    /// With an incompatible repository-local CLI pin, every discovery command
    /// stops with upgrade guidance instead of handing off to it.
    fn pinned_cli_needs_upgrade(&self) {
        let modules = self.root().join("node_modules");
        let pin = modules
            .join(TurboState::scoped_platform_package_scope())
            .join(TurboState::scoped_platform_package_dir());
        fs::create_dir_all(pin.join("bin")).unwrap();
        let calls = self.owned.home().join("pin-calls");
        let old = format!("#!/bin/sh\necho \"$@\" >> '{}'\nexit 2\n", calls.display());
        executable(&pin.join("bin").join(TurboState::binary_name()), &old);
        fs::write(pin.join("package.json"), r#"{"version":"2.5.0"}"#).unwrap();
        let devtools: &[&str] = &["devtools", "--no-open"];
        for args in DISCOVERY.iter().map(|(args, _)| *args).chain([devtools]) {
            let (code, text) = self.turbo("", args);
            assert_ne!(code, 0, "{args:?}: {text}");
            for expected in ["repository-local CLI", "upgrade"] {
                assert!(
                    squash(&text).contains(&squash(expected)),
                    "{args:?}: {text}"
                );
            }
        }
        let queried = fs::read_to_string(&calls).unwrap();
        assert!(
            queried
                .lines()
                .all(|line| line == "--__internal-managed-run-capabilities"),
            "{queried}"
        );
        fs::remove_dir_all(modules).unwrap();
    }

    /// `turbo run <tasks>` in strict and loose modes from the root and
    /// `nested`; every run prints each `expected` line, and only loose mode
    /// passes the caller's secret through.
    fn runs_managed(&self, nested: &str, tasks: &[&str], expected: &[&str]) {
        for dir in ["", nested] {
            for (mode, secret) in [("strict", "filtered"), ("loose", "leaked")] {
                let mode_flag = format!("--env-mode={mode}");
                let args = ["run"]
                    .iter()
                    .chain(tasks)
                    .chain(&[mode_flag.as_str(), "--ui=stream"])
                    .copied()
                    .collect::<Vec<_>>();
                let (code, text) = self.turbo(dir, &args);
                assert_eq!(code, 0, "{dir:?} {mode}: {text}");
                for line in expected {
                    assert!(text.contains(line), "{line:?} in {dir:?} {mode}: {text}");
                }
                assert!(text.contains(&format!("secret={secret}")), "{mode}: {text}");
            }
        }
    }

    /// Plain `turbo run` fails before any task with `guidance`.
    fn fails(&self, task: &str, guidance: &str) {
        let (code, text) = self.turbo("", &["run", task, "--ui=stream"]);
        assert_ne!(code, 0, "{text}");
        assert!(
            squash(&text).contains(&squash(guidance)),
            "{guidance}: {text}"
        );
        assert!(!text.contains("managed-"), "a task ran: {text}");
    }

    /// Every command the mock toolchains logged, then clears the log.
    fn calls(&self) -> String {
        let log = self.owned.home().join("managed-calls");
        let calls = fs::read_to_string(&log).unwrap_or_default();
        let _ = fs::remove_file(log);
        calls
    }

    /// The managed bin directory setup selected.
    fn bin(&self) -> PathBuf {
        turborepo_tool_install::Store::inspect(self.root())
            .unwrap()
            .unwrap()
            .bin
    }
}
