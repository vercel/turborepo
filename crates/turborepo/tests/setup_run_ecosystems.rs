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

use std::{
    fs,
    io::Write,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::Command,
};

use turborepo_setup::test_support::{LoopbackServer, OwnedSetupFixture};

/// Language tools a managed run must never take from the host PATH.
const HOST_TOOLS: &[&str] = &[
    "cargo", "rustc", "rustup", "go", "gofmt", "uv", "python", "python3", "node", "npm", "pnpm",
    "yarn", "bun",
];
/// Local tools-only setup, which creates turbo.lock even when CI is detected.
const LOCAL: &[&str] = &["--tools-only", "--no-frozen"];
/// Strict mode filters this caller variable; loose mode passes it.
const SECRET: &str = "JOURNEY_SECRET";

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
        let home = self.owned.home();
        let hits = self.upstream.hits();
        let output = Command::new(env!("CARGO_BIN_EXE_turbo"))
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
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(self.upstream.hits(), hits, "turbo run downloaded: {text}");
        let host = fs::read_to_string(home.join("host-ran")).unwrap_or_default();
        assert!(host.is_empty(), "host tools ran ({host}): {text}");
        (output.status.code().unwrap_or(-1), text)
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
        let squash = |s: &str| {
            s.split_whitespace()
                .collect::<String>()
                .replace(['|', '│'], "")
        };
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
