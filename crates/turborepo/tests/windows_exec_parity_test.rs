//! Native Windows parity for `turbo exec` through the standalone binary, the
//! installed npm wrapper, and the wrapper under npm, on pipes and on a ConPTY
//! terminal. Covers every outcome that needs no managed tools: each entrypoint
//! reports it identically, and none runs the command, writes files, or
//! contacts the network. Running managed `.cmd` and `.exe` commands waits on
//! Windows managed activation (TURBO-6379 and Windows trust records).
#![cfg(windows)]
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{
    fs,
    io::{ErrorKind, Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime},
};

use common::process_launcher::{CapturedProcess, Entrypoint, ProcessLauncher};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};

const TIMEOUT: Duration = Duration::from_secs(30);
const ENABLED: &str = r#"{"futureFlags":{"experimentalSetup":true}}"#;

#[derive(Clone, Copy, Debug)]
enum Launch {
    Standalone,
    Wrapper,
    /// With a terminal, the wrapper under npm would relay Ctrl+C over its fd 3
    /// pipe for other commands, but leaves the console to exec.
    WrapperUnderNpm,
}

const LAUNCHES: [Launch; 3] = [Launch::Standalone, Launch::Wrapper, Launch::WrapperUnderNpm];

/// Repository with a nested invocation directory, a repository-local CLI so
/// handoff is exercised, and a `probe` that leaves a marker if anything runs
/// it.
fn repo(launcher: &ProcessLauncher) -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for dir in ["apps/web", "bin"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    fs::write(
        root.join("package.json"),
        r#"{"name":"exec-parity","packageManager":"pnpm@10.0.0"}"#,
    )
    .unwrap();
    fs::write(root.join("bin/probe.cmd"), "@type nul > \"%~dp0ran\"\r\n").unwrap();
    ProcessLauncher::install_package(root, &launcher.standalone);
    temp
}

/// Paths under `root`, with sizes and modification times for files. NTFS
/// updates directory times lazily, so those are left out.
fn files(root: &Path) -> Vec<(PathBuf, Option<(u64, SystemTime)>)> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_owned()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let metadata = entry.metadata().unwrap();
            let file = if metadata.is_dir() {
                pending.push(entry.path());
                None
            } else {
                Some((metadata.len(), metadata.modified().unwrap()))
            };
            found.push((entry.path(), file));
        }
    }
    found.sort();
    found
}

fn command(launcher: &ProcessLauncher, launch: Launch, root: &Path, proxy: &str) -> Command {
    let entry = match launch {
        Launch::Standalone => Entrypoint::Standalone,
        Launch::Wrapper | Launch::WrapperUnderNpm => Entrypoint::NpmWrapper,
    };
    let mut command = launcher.command(entry, &root.join("apps/web"));
    command
        .args(["exec", "--", "probe", "a b", "", "100%"])
        .env("PATH", root.join("bin"))
        .env("AI_AGENT", "exec-parity-test");
    for name in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"] {
        command.env(name, proxy);
    }
    if let Launch::WrapperUnderNpm = launch {
        command.env("npm_config_user_agent", "npm/10.5.0 node/v22.0.0 win32 x64");
    }
    command
}

/// Answers cursor position queries as a terminal would; some conhost
/// versions block output until they are answered.
fn read_terminal(
    mut reader: Box<dyn Read + Send>,
    output: Arc<Mutex<Vec<u8>>>,
    mut input: Box<dyn Write + Send>,
) {
    let mut buffer = [0; 4096];
    let mut answered = 0;
    while let Ok(n @ 1..) = reader.read(&mut buffer) {
        let mut output = output.lock().unwrap();
        output.extend_from_slice(&buffer[..n]);
        let asked = output.windows(4).filter(|w| w == b"\x1b[6n").count();
        for _ in answered..asked {
            let _ = input.write_all(b"\x1b[1;1R").and_then(|()| input.flush());
        }
        answered = asked;
    }
}

/// Drops terminal control sequences so comparisons see what a user reads.
fn plain(text: &[u8]) -> String {
    let text = String::from_utf8_lossy(text);
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' if chars.peek() == Some(&'[') => {
                chars.next();
                while chars.next().is_some_and(|c| !('@'..='~').contains(&c)) {}
            }
            '\x1b' if chars.peek() == Some(&']') => {
                while chars.next().is_some_and(|c| c != '\x07') {}
            }
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// Exit code and visible text of `command` on a ConPTY.
fn on_terminal(command: &Command, root: &Path) -> (u32, String) {
    let mut builder = CommandBuilder::new(command.get_program());
    builder.args(command.get_args());
    builder.cwd(root.join("apps/web"));
    for (key, value) in command.get_envs() {
        match value {
            Some(value) => builder.env(key, value),
            None => builder.env_remove(key),
        }
    }
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 40,
            cols: 200,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut child = pair.slave.spawn_command(builder).unwrap();
    drop(pair.slave);
    let output = Arc::new(Mutex::new(Vec::new()));
    let reader = {
        let (reader, input) = (
            pair.master.try_clone_reader().unwrap(),
            pair.master.take_writer().unwrap(),
        );
        let output = output.clone();
        thread::spawn(move || read_terminal(reader, output, input))
    };
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if start.elapsed() > TIMEOUT {
            let _ = child.kill();
            panic!(
                "entrypoint did not exit: {}",
                plain(&output.lock().unwrap())
            );
        }
        thread::sleep(Duration::from_millis(50));
    };
    // Closing the pseudoconsole ends the output stream.
    drop(pair.master);
    let _ = reader.join();
    let text = plain(&output.lock().unwrap());
    (status.exit_code(), text)
}

/// Miette wraps sentences differently per width; compare words only.
fn squash(text: &str) -> String {
    text.split([' ', '\n', '|', '│'])
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn unprepared_exec_matches_across_entrypoints_without_running_writing_or_downloading() {
    let launcher = ProcessLauncher::new();
    let temp = repo(&launcher);
    let root = temp.path();
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    proxy.set_nonblocking(true).unwrap();
    let proxy_url = format!("http://{}", proxy.local_addr().unwrap());
    for (config, lock, expected) in [
        (
            "{}",
            false,
            "`turbo exec` requires root futureFlags.experimentalSetup",
        ),
        // The repository-local CLI receives this one through handoff.
        (ENABLED, false, "`turbo exec` requires managed setup"),
        // Validated before handoff; Windows has no managed activation yet.
        (ENABLED, true, "managed setup validation failed"),
    ] {
        fs::write(root.join("turbo.json"), config).unwrap();
        if lock {
            fs::write(root.join("turbo.lock"), "{}").unwrap();
        }
        let before = files(root);
        let (mut pipes, mut terminals) = (Vec::new(), Vec::new());
        for launch in LAUNCHES {
            let mut piped = command(&launcher, launch, root, &proxy_url);
            let output = CapturedProcess::spawn(&mut piped).finish(TIMEOUT);
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            assert_eq!(output.status.code(), Some(1), "{launch:?}: {stderr}");
            assert!(output.stdout.is_empty(), "{launch:?}: {output:?}");
            pipes.push(stderr);
            let (code, text) = on_terminal(&command(&launcher, launch, root, &proxy_url), root);
            assert_eq!(code, 1, "{launch:?} on a terminal: {text}");
            terminals.push(text);
        }
        for output in [&pipes[0], &terminals[0]] {
            assert!(squash(output).contains(expected), "{output}");
        }
        assert!(pipes.iter().all(|stderr| *stderr == pipes[0]), "{pipes:#?}");
        assert!(
            terminals
                .iter()
                .all(|text| squash(text) == squash(&terminals[0])),
            "{terminals:#?}"
        );
        assert_eq!(files(root), before, "exec must not run or write anything");
    }
    assert_eq!(
        proxy.accept().map(drop).unwrap_err().kind(),
        ErrorKind::WouldBlock,
        "exec contacted the network"
    );
}
