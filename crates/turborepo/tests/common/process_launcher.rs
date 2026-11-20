//! Offline package-shaped entrypoints; no TURBO_BINARY_PATH escape hatch.
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::Duration,
};

use serde_json::Value;
use tempfile::TempDir;

#[cfg(unix)]
use super::process::unix::TaskTreeGuard;
use super::{ambient_turbo_env_keys, manifest_dir, process::wait_for_process_exit};

#[derive(Clone, Copy, Debug)]
pub enum Entrypoint {
    Standalone,
    NpmWrapper,
    #[cfg(unix)]
    NpmScript(&'static str),
}

/// Keep this alive until all launched processes have finished.
pub struct ProcessLauncher {
    install: TempDir,
    pub standalone: PathBuf,
    pub packaged_binary: PathBuf,
    pub wrapper: PathBuf,
    pub node: PathBuf,
}

impl ProcessLauncher {
    pub fn new() -> Self {
        let install = tempfile::Builder::new()
            .prefix("turbo installed wrapper ")
            .tempdir()
            .unwrap();
        let standalone = assert_cmd::cargo::cargo_bin("turbo");
        let (wrapper, packaged_binary) = Self::install_package(install.path(), &standalone);
        Self {
            install,
            standalone,
            packaged_binary,
            wrapper,
            node: which::which("node").expect("process launcher requires Node.js"),
        }
    }

    /// Materialize only published launcher/manifest and the host optional
    /// package. Copies (not symlinks) ensure native local handoff is
    /// exercised too.
    pub fn install_package(root: &Path, binary: &Path) -> (PathBuf, PathBuf) {
        let source = manifest_dir().join("../../packages/turbo");
        let manifest: Value =
            serde_json::from_slice(&fs::read(source.join("package.json")).unwrap()).unwrap();
        let os = match std::env::consts::OS {
            "windows" => "windows",
            "macos" => "darwin",
            "linux" => "linux",
            other => panic!("unsupported npm fixture platform: {other}"),
        };
        let arch = match std::env::consts::ARCH {
            "x86_64" => "64",
            "aarch64" => "arm64",
            other => panic!("unsupported npm fixture architecture: {other}"),
        };
        let name = format!("@turbo/{os}-{arch}");
        assert_eq!(manifest["optionalDependencies"][&name], manifest["version"]);
        let npm = root.join("node_modules");
        let wrapper = npm.join("turbo/bin/turbo");
        let package = npm.join(&name);
        let packaged_binary = package.join(if cfg!(windows) {
            "bin/turbo.exe"
        } else {
            "bin/turbo"
        });
        fs::create_dir_all(wrapper.parent().unwrap()).unwrap();
        fs::create_dir_all(packaged_binary.parent().unwrap()).unwrap();
        fs::copy(source.join("bin/turbo"), &wrapper).unwrap();
        fs::write(
            npm.join("turbo/package.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        fs::write(
            package.join("package.json"),
            serde_json::to_vec(&serde_json::json!({
                "name": name, "version": manifest["version"]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::copy(binary, &packaged_binary).unwrap();
        #[cfg(unix)]
        {
            let bin = npm.join(".bin");
            fs::create_dir_all(&bin).unwrap();
            std::os::unix::fs::symlink("../turbo/bin/turbo", bin.join("turbo")).unwrap();
        }
        (wrapper, packaged_binary)
    }

    /// Shared cwd/env/stdio defaults; callers may override args, env and stdio,
    /// or pass this command to a platform-specific terminal harness.
    pub fn command(&self, entry: Entrypoint, cwd: &Path) -> Command {
        let mut command = match entry {
            Entrypoint::Standalone => Command::new(&self.standalone),
            Entrypoint::NpmWrapper => {
                let mut command = Command::new(&self.node);
                command.arg(&self.wrapper);
                command
            }
            #[cfg(unix)]
            Entrypoint::NpmScript(script) => {
                let mut command =
                    Command::new(which::which("npm").expect("npm script fixture requires npm"));
                command.args(["run", "--silent", script, "--"]);
                command
            }
        };
        for key in ambient_turbo_env_keys() {
            command.env_remove(key);
        }
        for key in [
            "CI",
            "GITHUB_ACTIONS",
            "npm_command",
            "npm_lifecycle_event",
            "npm_config_user_agent",
        ] {
            command.env_remove(key);
        }
        command
            .current_dir(cwd)
            .env_remove("TURBO_BINARY_PATH")
            .env_remove("NODE_OPTIONS")
            .env_remove("NODE_PATH")
            .env("TURBO_CONFIG_DIR_PATH", self.install.path().join("config"))
            .env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
            .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
            .env("TURBO_PRINT_VERSION_DISABLED", "1")
            .env("TURBO_NO_UPDATE_NOTIFIER", "1")
            .env("TURBO_DOWNLOAD_LOCAL_ENABLED", "0")
            .env("DO_NOT_TRACK", "1")
            .env("NPM_CONFIG_UPDATE_NOTIFIER", "false")
            .env("npm_config_offline", "true")
            .env("npm_config_registry", "http://127.0.0.1:1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        command
    }
}

/// Drain pipes concurrently so a verbose fixture cannot deadlock wait/cleanup.
/// Keep at most 1 MiB per stream but continue draining discarded bytes.
fn capture(mut reader: impl Read + Send + 'static) -> Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut output = Vec::new();
        let mut buffer = [0; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    let keep = n.min((1024 * 1024_usize).saturating_sub(output.len()));
                    output.extend_from_slice(&buffer[..keep]);
                }
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(err) => panic!("failed reading fixture output: {err}"),
            }
        }
        let _ = tx.send(output);
    });
    rx
}

/// Owns the launcher and its group on panic/timeout. Register task groups that
/// native turbo starts separately with `track_task`, as in graceful shutdown
/// tests.
pub struct CapturedProcess {
    pub child: Child,
    stdout: Receiver<Vec<u8>>,
    stderr: Receiver<Vec<u8>>,
    #[cfg(unix)]
    trees: Vec<TaskTreeGuard>,
    #[cfg(unix)]
    task_pids: Vec<i32>,
}

impl CapturedProcess {
    pub fn spawn(command: &mut Command) -> Self {
        let child = command.spawn().expect("failed spawning fixture entrypoint");
        #[cfg(unix)]
        let trees = vec![TaskTreeGuard::new(child.id() as i32)];
        let mut process = Self {
            child,
            stdout: mpsc::channel().1,
            stderr: mpsc::channel().1,
            #[cfg(unix)]
            trees,
            #[cfg(unix)]
            task_pids: Vec::new(),
        };
        process.stdout = capture(process.child.stdout.take().expect("stdout must be piped"));
        process.stderr = capture(process.child.stderr.take().expect("stderr must be piped"));
        process
    }

    #[cfg(unix)]
    pub fn track_task(&mut self, pid: i32) {
        self.trees.push(TaskTreeGuard::new(pid));
        self.task_pids.push(pid);
    }

    pub fn finish(mut self, timeout: Duration) -> Output {
        let status = wait_for_process_exit(&mut self.child, timeout);
        let stdout = self
            .stdout
            .recv_timeout(timeout)
            .expect("stdout did not close");
        let stderr = self
            .stderr
            .recv_timeout(timeout)
            .expect("stderr did not close");
        #[cfg(unix)]
        {
            for pid in &self.task_pids {
                super::process::unix::wait_for_process_gone(*pid, timeout);
            }
            for tree in &mut self.trees {
                tree.disarm();
            }
        }
        Output {
            status,
            stdout,
            stderr,
        }
    }
}

impl Drop for CapturedProcess {
    fn drop(&mut self) {
        #[cfg(unix)]
        self.trees.clear();
        #[cfg(windows)]
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = Command::new("taskkill")
                .args(["/T", "/F", "/PID"])
                .arg(self.child.id().to_string())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
