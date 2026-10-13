//! Managed `turbo run` with no repository-local turbo, a compatible one (this
//! build, copied into `node_modules`), and incompatible ones, natively on
//! every CI platform. A verify-system lock needs no downloads or trust
//! records, so the journey is the same on macOS, Linux and Windows (and on
//! musl hosts, where only verify-system tools activate).
#![cfg(any(
    all(
        any(target_os = "macos", target_os = "linux"),
        any(target_arch = "x86_64", target_arch = "aarch64")
    ),
    all(windows, target_arch = "x86_64")
))]
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use turborepo_shim::TurboState;

const CONFIG: &str = r#"{"agentGuidance":false,
    "noUpdateNotifier":true,"tasks":{"build":{"cache":false}},
    "setup":{"dependencies":"skip","javascript":{"tools":"verify-system"}}}"#;

/// A system tool on PATH that reports `version` and echoes other calls.
fn tool(bin: &Path, name: &str, version: &str) {
    if cfg!(windows) {
        let script =
            format!("@if \"%1\"==\"--version\" (echo {version}) else (echo system-{name} %*)\r\n");
        fs::write(bin.join(format!("{name}.cmd")), script).unwrap();
        return;
    }
    let path = bin.join(name);
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = --version ]; then echo {version}; else echo \"system-{name} \
         $*\"; fi\n"
    );
    fs::write(&path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

struct Repo {
    _temp: tempfile::TempDir,
    root: PathBuf,
    path: OsString,
}

impl Repo {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let (root, bin) = (temp.path().join("repo"), temp.path().join("bin"));
        fs::create_dir_all(root.join("apps/web")).unwrap();
        fs::create_dir_all(&bin).unwrap();
        tool(&bin, "node", "v24.0.0");
        tool(&bin, "pnpm", "10.0.0");
        for (name, contents) in [
            ("turbo.json", CONFIG),
            (".nvmrc", "24.0.0\n"),
            (".gitignore", "/.turbo/\nnode_modules/\n"),
            (
                "package.json",
                r#"{"name":"pins","packageManager":"pnpm@10.0.0"}"#,
            ),
            ("pnpm-workspace.yaml", "packages: ['apps/*']\n"),
            (
                "pnpm-lock.yaml",
                "lockfileVersion: '9.0'\nimporters:\n  .: {}\n  apps/web: {}\n",
            ),
            (
                "apps/web/package.json",
                r#"{"name":"web","scripts":{"build":"build"}}"#,
            ),
        ] {
            fs::write(root.join(name), contents).unwrap();
        }
        common::git(&root, &["init", "--quiet"]);
        let git = which::which("git").unwrap();
        let mut dirs = vec![bin, git.parent().unwrap().to_owned()];
        if let Some(system) = std::env::var_os("SystemRoot") {
            dirs.push(Path::new(&system).join("System32"));
        }
        let path = std::env::join_paths(dirs).unwrap();
        Self {
            _temp: temp,
            root,
            path,
        }
    }

    fn turbo(&self, args: &[&str]) -> (i32, String) {
        let home = self.root.parent().unwrap().join("home");
        // Windows resolves the user state directory from these profile folders.
        let (local, roaming) = (home.join("AppData/Local"), home.join("AppData/Roaming"));
        fs::create_dir_all(&local).unwrap();
        fs::create_dir_all(&roaming).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_turbo"));
        command
            .env_clear()
            .current_dir(self.root.join("apps/web"))
            .args(args)
            .env("PATH", &self.path)
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("LOCALAPPDATA", &local)
            .env("APPDATA", &roaming)
            .env("XDG_DATA_HOME", home.join("data"))
            .env("TURBO_CONFIG_DIR_PATH", home.join("config"))
            .env("TURBO_TELEMETRY_DISABLED", "1")
            .env("DO_NOT_TRACK", "1")
            .env("HTTP_PROXY", "http://127.0.0.1:9")
            .env("HTTPS_PROXY", "http://127.0.0.1:9");
        // cmd.exe and PATHEXT lookup of `.cmd` tools need these.
        for key in [
            "SystemRoot",
            "SystemDrive",
            "windir",
            "ComSpec",
            "PATHEXT",
            "TEMP",
            "TMP",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        let output = command.output().unwrap();
        let text = common::combined_output(&output);
        (output.status.code().unwrap_or(-1), text)
    }

    /// Installs `binary` as the repository's native turbo package.
    fn pin(&self, binary: &Path, version: &str) -> PathBuf {
        let package = self
            .root
            .join("node_modules")
            .join(TurboState::scoped_platform_package_scope())
            .join(TurboState::scoped_platform_package_dir());
        fs::create_dir_all(package.join("bin")).unwrap();
        fs::write(
            package.join("package.json"),
            format!(r#"{{"version":"{version}"}}"#),
        )
        .unwrap();
        let pinned = package.join("bin").join(TurboState::binary_name());
        fs::copy(binary, &pinned).unwrap();
        pinned
    }
}

fn squash(text: &str) -> String {
    text.split_whitespace()
        .collect::<String>()
        .replace(['|', '│'], "")
}

fn host_target() -> String {
    let os = match std::env::consts::OS {
        "macos" => "macos",
        "linux" => "linux",
        _ => "windows",
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        _ => "x64",
    };
    format!("{os}-{arch}")
}

#[test]
fn managed_run_keeps_compatible_local_turbo_and_refuses_incompatible_pins() {
    let repo = Repo::new();
    let (code, text) = repo.turbo(&["setup", "--no-frozen"]);
    assert_eq!(code, 0, "{text}");
    assert!(repo.root.join("turbo.lock").exists(), "{text}");
    let build = ["run", "build", "--ui=stream", "--verbosity=2"];

    // No repository-local turbo: the global CLI runs the selection itself.
    let (code, text) = repo.turbo(&build);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("system-pnpm run build"), "{text}");
    assert!(!text.contains("Running local turbo binary"), "{text}");

    // Incompatible pins stop before any task with upgrade guidance: one that
    // predates the handshake, and one without verify-system support.
    let lacking = format!(
        r#"{{"abiVersion":1,"cliVersion":"9.9.9","managedRunSchemas":[],"experimentalSetup":{{"lockSchema":0,"target":"{}","features":["node","pnpm"]}}}}"#,
        host_target()
    );
    for (report, reason) in [
        ("stderr", "does not report managed-run support"),
        (
            lacking.as_str(),
            "lacks managed-run support for verify-system",
        ),
    ] {
        let pinned = repo.pin(Path::new(env!("CARGO_BIN_EXE_echo_args")), "9.9.9");
        fs::write(pinned.with_file_name("capability-peer.txt"), report).unwrap();
        let (code, text) = repo.turbo(&build);
        assert_ne!(code, 0, "{text}");
        for expected in [
            "repository-local turbo 9.9.9 at",
            "cannot run turbo.lock schema 0 (node, pnpm, verify-system)",
            reason,
            "Upgrade the repository's turbo dependency",
        ] {
            assert!(
                squash(&text).contains(&squash(expected)),
                "{expected}: {text}"
            );
        }
        assert!(!text.contains("system-pnpm run"), "a task ran: {text}");
    }

    // This build copied into the repository is compatible: managed run hands
    // off to it and the selection holds after handoff.
    let version = include_str!("../../../version.txt").lines().next().unwrap();
    repo.pin(Path::new(env!("CARGO_BIN_EXE_turbo")), version);
    let (code, text) = repo.turbo(&build);
    assert_eq!(code, 0, "{text}");
    let handoff = text
        .lines()
        .find(|line| line.contains("Running local turbo binary in"));
    let package = TurboState::scoped_platform_package_dir();
    assert!(handoff.is_some_and(|line| line.contains(package)), "{text}");
    assert!(text.contains("system-pnpm run build"), "{text}");
}
