//! Real `turbo exec` after fixture-provisioned managed Node/pnpm. Loopback
//! artifacts only: no vendor network, host tools, installs, or lock writes.
#![cfg(all(
    unix,
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))
))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::{
    fs,
    io::Write,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::{Command, Output},
};

use sha2::{Digest, Sha256};
use turborepo_setup::test_support::{LoopbackServer, OwnedSetupFixture};

/// Records how it was started outside the repository, then exits 7.
const NODE: &str = "#!/bin/sh\npwd > \"$HOME/exec-record\"\nprintf '%s\\n' \"$0\" \"$@\" >> \
                    \"$HOME/exec-record\"\nexit 7\n";

fn archive(root: &str, files: &[(&str, &str)]) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for (path, body) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, format!("{root}/{path}"), body.as_bytes())
            .unwrap();
    }
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&tar.into_inner().unwrap()).unwrap();
    gzip.finish().unwrap()
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

struct Repo {
    owned: OwnedSetupFixture,
    upstream: LoopbackServer,
}

impl Repo {
    fn provisioned() -> Self {
        Self::provisioned_with(NODE)
    }

    /// `turbo setup --frozen --tools-only` from a nested directory, with
    /// `node_script` as the managed Node executable.
    fn provisioned_with(node_script: &str) -> Self {
        let owned = OwnedSetupFixture::new().unwrap();
        let root = owned.root();
        owned.init_git().unwrap();
        let config = r#"{"futureFlags":{"experimentalSetup":true},"noUpdateNotifier":true,"agentGuidance":false}"#;
        fs::write(root.join("turbo.json"), config).unwrap();
        fs::write(root.join(".nvmrc"), "24.0.0\n").unwrap();
        let package = r#"{"name":"fixture","private":true,"packageManager":"pnpm@10.0.0"}"#;
        fs::write(root.join("package.json"), package).unwrap();
        fs::create_dir_all(root.join("apps/web/src")).unwrap();
        let (platform, spelling) = match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => ("macos-arm64", "darwin-arm64"),
            ("macos", _) => ("macos-x64", "darwin-x64"),
            ("linux", "aarch64") => ("linux-arm64-gnu", "linux-arm64"),
            _ => ("linux-x64-gnu", "linux-x64"),
        };
        let prefix = format!("node-v24.0.0-{spelling}");
        let node_path = format!("/dist/v24.0.0/{prefix}.tar.gz");
        let node = archive(&prefix, &[("bin/node", node_script)]);
        let pnpm = archive(
            "package",
            &[
                (
                    "package.json",
                    r#"{"name":"pnpm","version":"10.0.0","bin":{"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"}}"#,
                ),
                ("bin/pnpm.cjs", "throw new Error('fixture');"),
                ("bin/pnpx.cjs", "throw new Error('fixture');"),
            ],
        );
        let lock = serde_json::json!({"schemaVersion":0,"tools":{
            "node":{"adapter":"node","version":"24.0.0","declarations":[{"file":".nvmrc","request":"24.0.0"}],
                "installation":{"kind":"managed","artifacts":{platform:{"distribution":{
                    "url":format!("https://nodejs.org{node_path}"),"sha256":sha256(&node),
                    "format":"tar-gz","rootPrefix":prefix,"executables":{"node":"bin/node"}}}}}},
            "pnpm":{"adapter":"pnpm","version":"10.0.0","declarations":[{"file":"package.json","field":"/packageManager","request":"pnpm@10.0.0"}],
                "installation":{"kind":"managed","artifacts":{"any":{"package":{
                    "url":"https://registry.npmjs.org/pnpm/-/pnpm-10.0.0.tgz","sha256":sha256(&pnpm),
                    "format":"tar-gz","rootPrefix":"package","executables":{"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"}}}}}}
        }});
        fs::write(root.join("turbo.lock"), lock.to_string()).unwrap();
        let upstream = LoopbackServer::node_pnpm(node_path, node, pnpm, false, |_| {}).unwrap();
        let invocation = root.join("apps/web/src");
        let user_base = if cfg!(target_os = "macos") {
            owned.home().join("Library/Application Support")
        } else {
            owned.home().join("data")
        };
        let code = turborepo_cli::test_support::frozen_tools_for_tests(
            &invocation,
            &user_base.join("turborepo/tools"),
            upstream.origin(),
            || owned.policy_at(&invocation),
        )
        .unwrap();
        assert_eq!(code, 0);
        // Host tools of the same name would leave a marker if exec fell back.
        let host = owned.home().join("host-bin");
        fs::create_dir(&host).unwrap();
        symlink(which::which("git").unwrap(), host.join("git")).unwrap();
        for tool in ["node", "pnpm"] {
            let marker = owned.home().join("host-ran");
            let script = format!("#!/bin/sh\n: > '{}'\nexit 97\n", marker.display());
            fs::write(host.join(tool), script).unwrap();
            fs::set_permissions(host.join(tool), fs::Permissions::from_mode(0o755)).unwrap();
        }
        Self { owned, upstream }
    }

    fn exec(&self, dir: &str, words: &[&str]) -> (Output, String) {
        let home = self.owned.home();
        let output = Command::new(env!("CARGO_BIN_EXE_turbo"))
            .current_dir(self.owned.root().join(dir))
            .args(words)
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
            .output()
            .unwrap();
        let record = home.join("exec-record");
        let text = fs::read_to_string(&record).unwrap_or_default();
        let _ = fs::remove_file(record);
        (output, text)
    }

    fn state(&self) -> (Vec<u8>, Vec<u8>) {
        let root = self.owned.root();
        (
            fs::read(root.join("turbo.lock")).unwrap(),
            fs::read(root.join(".turbo/tools/manifest.json")).unwrap(),
        )
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn exec_runs_managed_tools_from_root_nested_and_cwd_without_installing() {
    let repo = Repo::provisioned();
    let root = repo.owned.root().to_owned();
    let (before, hits) = (repo.state(), repo.upstream.hits());
    let tail = ["--", "node", "a b", "", "--", "--cwd", "*"];
    for (dir, flags, cwd) in [
        ("", &[][..], root.clone()),
        ("apps/web/src", &[], root.join("apps/web/src")),
        ("", &["--cwd", "apps/web"], root.join("apps/web")),
    ] {
        let words: Vec<_> = ["exec"].iter().chain(flags).chain(&tail).copied().collect();
        let (output, record) = repo.exec(dir, &words);
        assert_eq!(
            output.status.code(),
            Some(7),
            "{words:?}: {}",
            stderr(&output)
        );
        assert!(output.stdout.is_empty(), "exec must not print to stdout");
        let lines: Vec<_> = record.lines().collect();
        assert_eq!(PathBuf::from(lines[0]).canonicalize().unwrap(), cwd);
        assert!(Path::new(lines[1]).starts_with(&root), "{}", lines[1]);
        assert_eq!(&lines[2..], ["a b", "", "--", "--cwd", "*"]);
    }
    // Managed pnpm dispatches through the managed Node runtime.
    let (output, record) = repo.exec("apps/web", &["exec", "--", "pnpm", "--version"]);
    assert_eq!(output.status.code(), Some(7), "{}", stderr(&output));
    let lines: Vec<_> = record.lines().collect();
    assert!(Path::new(lines[1]).starts_with(&root));
    assert!(lines[2].ends_with("pnpm.cjs"), "{record}");
    assert_eq!(lines[3], "--version");

    assert!(!repo.owned.home().join("host-ran").exists());
    assert_eq!(repo.upstream.hits(), hits, "exec must not download");
    assert_eq!(
        repo.state(),
        before,
        "exec must not install or write the lock"
    );
}

#[test]
fn exec_fails_after_drift_or_damage_and_runs_nothing() {
    let repo = Repo::provisioned();
    let root = repo.owned.root().to_owned();
    fs::write(root.join(".nvmrc"), "26.0.0\n").unwrap();
    let (output, record) = repo.exec("apps/web", &["exec", "--", "node"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("turbo setup"),
        "{}",
        stderr(&output)
    );
    assert!(record.is_empty());

    fs::write(root.join(".nvmrc"), "24.0.0\n").unwrap();
    let current = turborepo_tool_install::Store::inspect(&root)
        .unwrap()
        .unwrap();
    fs::remove_file(current.bin.join("pnpm")).unwrap();
    let (output, record) = repo.exec("", &["exec", "--", "node"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("turbo setup"),
        "{}",
        stderr(&output)
    );
    assert!(record.is_empty());
    assert!(!repo.owned.home().join("host-ran").exists());
}

/// TURBO-6260: the standalone binary and the installed npm wrapper must give
/// `turbo exec` the same user-visible result.
mod parity {
    use std::{
        io::Read,
        os::unix::process::ExitStatusExt,
        process::Stdio,
        thread,
        time::{Duration, Instant},
    };

    use nix::{
        sys::signal::{Signal, kill, killpg},
        unistd::Pid,
    };
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};

    use super::{
        common::process_launcher::{CapturedProcess, Entrypoint, ProcessLauncher},
        *,
    };

    const ENTRIES: [Entrypoint; 2] = [Entrypoint::Standalone, Entrypoint::NpmWrapper];
    const TIMEOUT: Duration = Duration::from_secs(60);
    const NODE: &str = r#"#!/bin/sh
case "$1" in
wait)
  trap 'printf "TERM\n" >> "$HOME/signals"; exit 5' TERM
  trap 'printf "INT\n" >> "$HOME/signals"; exit 6' INT
  printf '%s\n' "$$" > "$HOME/ready"
  while :; do :; done
  ;;
*)
  if [ -t 1 ]; then out=tty; else out=pipe; fi
  {
    pwd
    for arg in "$@"; do printf '[%s]\n' "$arg"; done
    printf 'env=%s unset=%s\n' "$EXEC_PARITY" "${EXEC_UNSET-unset}"
    if [ -t 0 ]; then echo stdin=tty; else IFS= read -r line; printf 'stdin=%s\n' "$line"; fi
    echo "stdout=$out"
  } > "$HOME/report"
  printf 'to stdout\n'
  printf 'to stderr\n' >&2
  exit "$2"
  ;;
esac
"#;

    fn command(
        repo: &Repo,
        launcher: &ProcessLauncher,
        entry: Entrypoint,
        words: &[&str],
    ) -> Command {
        let home = repo.owned.home();
        let mut command = launcher.command(entry, &repo.owned.root().join("apps/web/src"));
        command
            .args(words)
            .env("HOME", home)
            .env("XDG_DATA_HOME", home.join("data"))
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("HTTP_PROXY", "http://127.0.0.1:9")
            .env("HTTPS_PROXY", "http://127.0.0.1:9");
        command
    }

    fn take(path: &Path) -> String {
        let text = fs::read_to_string(path).unwrap_or_default();
        let _ = fs::remove_file(path);
        text
    }

    /// The managed command's PID, once it is running.
    fn wait_for(ready: &Path) -> i32 {
        let started = Instant::now();
        loop {
            if let Some(pid) = fs::read_to_string(ready)
                .ok()
                .and_then(|text| text.trim().parse().ok())
            {
                let _ = fs::remove_file(ready);
                return pid;
            }
            assert!(started.elapsed() < TIMEOUT, "command never started");
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Hosts where `turbo` qualifies managed handoff to a repository-local CLI.
    const HANDOFF: bool = cfg!(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "x86_64")
    ));

    /// Runs `case`, then, where handoff is qualified, again after the
    /// repository installs `turbo`, so global turbo hands off to that copy.
    fn with_handoff(repo: &Repo, launcher: &ProcessLauncher, mut case: impl FnMut(bool)) {
        case(false);
        if HANDOFF {
            ProcessLauncher::install_package(repo.owned.root(), &launcher.standalone);
            case(true);
        }
    }

    #[test]
    fn pipes_preserve_arguments_cwd_environment_streams_and_exit() {
        let repo = Repo::provisioned_with(NODE);
        let launcher = ProcessLauncher::new();
        let home = repo.owned.home().to_owned();
        let cwd = repo.owned.root().join("apps/web/src");
        with_handoff(&repo, &launcher, |handoff| {
            for code in ["0", "3"] {
                let args = ["a b", "quote\"d", "$(echo no)", "ünïcode", ""];
                let mut results = vec![];
                for entry in ENTRIES {
                    let words = ["exec", "--", "node", "report", code];
                    let words: Vec<_> = words.iter().chain(&args).copied().collect();
                    let mut command = command(&repo, &launcher, entry, &words);
                    command
                        .env("EXEC_PARITY", "")
                        .env_remove("EXEC_UNSET")
                        .stdin(Stdio::piped());
                    let mut process = CapturedProcess::spawn(&mut command);
                    let mut stdin = process.child.stdin.take().unwrap();
                    stdin.write_all(b"hello\n").unwrap();
                    drop(stdin);
                    let output = process.finish(TIMEOUT);
                    results.push((
                        output.status.code(),
                        String::from_utf8_lossy(&output.stdout).into_owned(),
                        String::from_utf8_lossy(&output.stderr).into_owned(),
                        take(&home.join("report")),
                    ));
                }
                let parity = format!("standalone vs npm wrapper, handoff={handoff}");
                assert_eq!(results[0], results[1], "{parity}");
                let (status, stdout, stderr, report) = &results[0];
                assert_eq!(*status, Some(code.parse().unwrap()), "{stderr}");
                assert_eq!(stdout, "to stdout\n");
                assert_eq!(stderr, "to stderr\n");
                let mut expected = format!("{}\n[report]\n[{code}]\n", cwd.display());
                for arg in args {
                    expected.push_str(&format!("[{arg}]\n"));
                }
                expected.push_str("env= unset=unset\nstdin=hello\nstdout=pipe\n");
                assert_eq!(report, &expected);
            }
        });
    }

    #[test]
    fn sigterm_and_group_sigint_reach_the_command_once() {
        let repo = Repo::provisioned_with(NODE);
        let launcher = ProcessLauncher::new();
        let home = repo.owned.home().to_owned();
        with_handoff(&repo, &launcher, |handoff| {
            for (group, signal, code, record) in [
                (false, Signal::SIGTERM, 5, "TERM\n"),
                (true, Signal::SIGINT, 6, "INT\n"),
            ] {
                for entry in ENTRIES {
                    let words = ["exec", "--", "node", "wait"];
                    let mut process =
                        CapturedProcess::spawn(&mut command(&repo, &launcher, entry, &words));
                    process.track_task(wait_for(&home.join("ready")));
                    // Let the npm wrapper finish starting before signaling.
                    thread::sleep(Duration::from_millis(200));
                    let target = Pid::from_raw(process.child.id() as i32);
                    if group {
                        killpg(target, signal).unwrap();
                    } else {
                        kill(target, signal).unwrap();
                    }
                    // finish() also waits until the managed command itself is gone.
                    let output = process.finish(TIMEOUT);
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    assert_eq!(
                        output.status.code(),
                        Some(code),
                        "{entry:?} {signal} handoff={handoff}: {stderr}"
                    );
                    assert_eq!(
                        take(&home.join("signals")),
                        record,
                        "{entry:?} {signal} handoff={handoff}"
                    );
                }
            }
        });
    }

    fn parent(pid: i32) -> i32 {
        let ps = Command::new("ps")
            .args(["-o", "ppid=", "-p", &pid.to_string()])
            .output()
            .unwrap();
        String::from_utf8_lossy(&ps.stdout).trim().parse().unwrap()
    }

    /// TURBO-6424: exec becomes the managed command, so no turbo process is
    /// left that SIGKILL could take down without it. After a handoff, global
    /// turbo became the local CLI, and that CLI became the command.
    #[test]
    fn sigkill_to_exec_leaves_no_command_running() {
        let repo = Repo::provisioned_with(NODE);
        let launcher = ProcessLauncher::new();
        let home = repo.owned.home().to_owned();
        with_handoff(&repo, &launcher, |handoff| {
            for entry in ENTRIES {
                let words = ["-vv", "exec", "--", "node", "wait"];
                let mut process =
                    CapturedProcess::spawn(&mut command(&repo, &launcher, entry, &words));
                let pid = wait_for(&home.join("ready"));
                process.track_task(pid);
                let entrypoint = process.child.id() as i32;
                // The wrapper is a Node parent; its native turbo became the command.
                let exec = match entry {
                    Entrypoint::NpmWrapper => parent(pid),
                    _ => pid,
                };
                let case = format!("{entry:?} handoff={handoff}");
                assert_eq!(exec, entrypoint, "{case}: exec is not the command");
                kill(Pid::from_raw(pid), Signal::SIGKILL).unwrap();
                // finish() also waits until the managed command itself is gone.
                let output = process.finish(TIMEOUT);
                assert_eq!(output.status.signal(), Some(9), "{case}");
                assert_eq!(take(&home.join("signals")), "", "{case}");
                let stderr = String::from_utf8_lossy(&output.stderr);
                let handed_off = stderr.contains("Running local turbo binary in");
                assert_eq!(handed_off, handoff, "{case}: {stderr}");
            }
        });
    }

    /// Runs `command` on a new terminal, optionally typing `keys` once the
    /// managed command is ready.
    fn on_terminal(command: &Command, keys: Option<(&Path, &[u8])>) -> Option<u32> {
        let mut builder = CommandBuilder::new(command.get_program());
        builder.args(command.get_args());
        builder.cwd(command.get_current_dir().unwrap());
        for (key, value) in command.get_envs() {
            match value {
                Some(value) => builder.env(key, value),
                None => builder.env_remove(key),
            }
        }
        let size = PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = native_pty_system().openpty(size).unwrap();
        let mut reader = pair.master.try_clone_reader().unwrap();
        thread::spawn(move || {
            let mut sink = [0; 1024];
            while matches!(reader.read(&mut sink), Ok(n) if n > 0) {}
        });
        let mut child = pair.slave.spawn_command(builder).unwrap();
        drop(pair.slave);
        if let Some((ready, keys)) = keys {
            wait_for(ready);
            thread::sleep(Duration::from_millis(200));
            pair.master.take_writer().unwrap().write_all(keys).unwrap();
        }
        let started = Instant::now();
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                return Some(status.exit_code());
            }
            if started.elapsed() > TIMEOUT {
                let _ = child.kill();
                return None;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn terminals_and_ctrl_c_match() {
        let repo = Repo::provisioned_with(NODE);
        let launcher = ProcessLauncher::new();
        let home = repo.owned.home().to_owned();
        with_handoff(&repo, &launcher, |handoff| {
            for entry in ENTRIES {
                let report = command(
                    &repo,
                    &launcher,
                    entry,
                    &["exec", "--", "node", "report", "0"],
                );
                assert_eq!(
                    on_terminal(&report, None),
                    Some(0),
                    "{entry:?} handoff={handoff}"
                );
                let report = take(&home.join("report"));
                assert!(
                    report.ends_with("stdin=tty\nstdout=tty\n"),
                    "{entry:?} handoff={handoff}: {report}"
                );
                let wait = command(&repo, &launcher, entry, &["exec", "--", "node", "wait"]);
                let ready = home.join("ready");
                assert_eq!(
                    on_terminal(&wait, Some((&ready, b"\x03"))),
                    Some(6),
                    "{entry:?} handoff={handoff}"
                );
                assert_eq!(
                    take(&home.join("signals")),
                    "INT\n",
                    "{entry:?} handoff={handoff}"
                );
            }
        });
    }
}
