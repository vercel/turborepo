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

const CONFIG: &str = r#"{"futureFlags":{"experimentalSetup":true},"agentGuidance":false,
    "noUpdateNotifier":true,"tasks":{"build":{"cache":false}},
    "setup":{"dependencies":"skip","javascript":{"tools":"managed"}}}"#;

struct Repo {
    _temp: tempfile::TempDir,
    root: PathBuf,
    path: OsString,
    trust: PathBuf,
}

impl Repo {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let (root, bin) = (temp.path().join("repo"), temp.path().join("bin"));
        fs::create_dir_all(root.join("apps/web")).unwrap();
        fs::create_dir_all(&bin).unwrap();
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
        use turborepo_setup::{
            lock::{Lock, Platform},
            node_provision::NodePlan,
            pnpm_provision::PnpmPlan,
        };
        let (platform, spelling) = match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => (Platform::MacosArm64, "darwin-arm64"),
            ("macos", _) => (Platform::MacosX64, "darwin-x64"),
            ("linux", "aarch64") => (Platform::LinuxArm64Gnu, "linux-arm64"),
            ("linux", _) => (Platform::LinuxX64Gnu, "linux-x64"),
            _ => (Platform::WindowsX64, "win-x64"),
        };
        let prefix = format!("node-v24.0.0-{spelling}");
        let node_bin = if cfg!(windows) {
            "node.exe"
        } else {
            "bin/node"
        };
        let format = if cfg!(windows) { "zip" } else { "tar-gz" };
        let extension = if cfg!(windows) { "zip" } else { "tar.gz" };
        let platform_key = serde_json::to_value(platform).unwrap();
        let lock = serde_json::json!({"schemaVersion":0,"tools":{
            "node":{"adapter":"node","version":"24.0.0","declarations":[{"file":".nvmrc","request":"24.0.0"}],
                "installation":{"kind":"managed","artifacts":{platform_key.as_str().unwrap():{"distribution":{
                    "url":format!("https://nodejs.org/dist/v24.0.0/{prefix}.{extension}"),"sha256":"a".repeat(64),
                    "format":format,"rootPrefix":prefix,"executables":{"node":node_bin}}}}}},
            "pnpm":{"adapter":"pnpm","version":"10.0.0","declarations":[{"file":"package.json","field":"/packageManager","request":"pnpm@10.0.0"}],
                "installation":{"kind":"managed","artifacts":{"any":{"package":{
                    "url":"https://registry.npmjs.org/pnpm/-/pnpm-10.0.0.tgz","sha256":"b".repeat(64),
                    "format":"tar-gz","rootPrefix":"package","executables":{"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"}}}}}}
        }});
        fs::write(root.join("turbo.lock"), lock.to_string()).unwrap();
        let lock = Lock::parse(lock.to_string().as_bytes()).unwrap();
        let node = NodePlan::from_lock(&lock, platform).unwrap();
        let pnpm = PnpmPlan::from_lock(&lock, platform, &node, None).unwrap();
        let tools = [node.inventory_tool().clone(), pnpm.inventory_tool().clone()];
        turborepo_tool_install::Store::open(&root)
            .unwrap()
            .reconcile(&tools, |tool, tree| {
                for relative in tool.tree_files() {
                    let target = tree.join(relative);
                    fs::create_dir_all(target.parent().unwrap())?;
                    // The managed pnpm launcher runs managed Node, which
                    // echoes the pnpm entry point and arguments it received.
                    if tool.id == "node" {
                        fs::copy(env!("CARGO_BIN_EXE_echo_args"), &target)?;
                    } else {
                        fs::write(&target, "// managed pnpm entry point\n")?;
                    }
                }
                Ok(())
            })
            .unwrap();
        let home = root.parent().unwrap().join("home");
        let user_base = if cfg!(target_os = "macos") {
            home.join("Library/Application Support")
        } else if cfg!(windows) {
            home.join("AppData/Local")
        } else {
            home.join("data")
        };
        let trust =
            turborepo_cli::test_support::record_trust_for_tests(&root, Some(user_base.as_path()))
                .unwrap();
        Self {
            _temp: temp,
            root,
            path,
            trust,
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
    let build = ["run", "build", "--ui=stream", "--verbosity=2"];
    if matches!(
        turborepo_platform::host_libc(),
        Ok(Some(turborepo_platform::Libc::Musl { .. }))
    ) {
        let (code, text) = repo.turbo(&build);
        assert_eq!(code, 1, "{text}");
        assert!(text.contains("musl"), "{text}");
        assert!(!text.contains("pnpm.cjs run"), "{text}");
        return;
    }

    // No repository-local turbo: the global CLI runs the selection itself.
    let (code, text) = repo.turbo(&build);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("pnpm.cjs run build"), "{text}");
    assert!(!text.contains("Running local turbo binary"), "{text}");

    let lacking = format!(
        r#"{{"abiVersion":1,"cliVersion":"9.9.9","managedRunSchemas":[],"experimentalSetup":{{"lockSchema":0,"target":"{}","features":["node"]}}}}"#,
        host_target()
    );
    for (report, reason) in [
        ("stderr", "does not report managed-run support"),
        (lacking.as_str(), "lacks managed-run support for pnpm"),
    ] {
        let pinned = repo.pin(Path::new(env!("CARGO_BIN_EXE_echo_args")), "9.9.9");
        fs::write(pinned.with_file_name("capability-peer.txt"), report).unwrap();
        let (code, text) = repo.turbo(&build);
        assert_ne!(code, 0, "{text}");
        for expected in [
            "repository-local turbo 9.9.9 at",
            "cannot run turbo.lock schema 0 (node, pnpm)",
            reason,
            "Upgrade the repository's turbo dependency",
        ] {
            assert!(
                squash(&text).contains(&squash(expected)),
                "{expected}: {text}"
            );
        }
        assert!(!text.contains("pnpm.cjs run"), "a task ran: {text}");
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
    assert!(text.contains("pnpm.cjs run build"), "{text}");
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.trust);
    }
}
