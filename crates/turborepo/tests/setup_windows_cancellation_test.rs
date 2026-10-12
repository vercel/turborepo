//! Native Windows: interrupt `turbo setup` through the standalone binary and
//! the installed npm wrapper while the managed Node download is blocked. A
//! loopback proxy accepts the CONNECT tunnel and never answers, so the real
//! binary holds its setup transaction without contacting nodejs.org.
//!
//! Direct turbo and a plain wrapper receive Ctrl+C as a console event. Under a
//! package manager with a terminal, the wrapper reads Ctrl+C in raw mode and
//! relays it over its fd 3 pipe instead.
#![cfg(all(windows, target_arch = "x86_64"))]
#![feature(windows_process_extensions_show_window)]
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    os::windows::process::CommandExt,
    path::Path,
    process::Command,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

use common::process_launcher::{CapturedProcess, Entrypoint, ProcessLauncher};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_TIMEOUT},
    System::{
        Console::SetConsoleCtrlHandler,
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
        Threading::{
            CREATE_NEW_CONSOLE, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject,
        },
    },
};

const TIMEOUT: Duration = Duration::from_secs(30);
/// What the default console handler exits with, as reported by Windows.
const CTRL_C_EXIT: i32 = 0xC000013Au32 as i32;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Launch {
    Standalone,
    Wrapper,
    /// The wrapper under a package manager script marker. With a terminal
    /// this selects the raw-mode Ctrl+C pipe.
    WrapperUnderNpm,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Stop {
    /// A console Ctrl+C on a dedicated hidden console.
    Console,
    /// Ctrl+C typed into a ConPTY.
    Pty,
    /// A supervisor's TerminateProcess; Windows has no SIGTERM.
    Terminate,
}

/// Accepts proxy tunnels, reports each request line, and never answers.
fn stalled_proxy() -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            let _ = BufReader::new(&stream).read_line(&mut line);
            held.push(stream);
            if tx.send(line.trim_end().to_owned()).is_err() {
                return;
            }
        }
    });
    (url, rx)
}

/// A Node-only managed repository with a committed Windows x64 lock.
fn fixture(root: &Path) {
    let prefix = "node-v24.0.0-win-x64";
    let lock = serde_json::json!({"schemaVersion": 0, "tools": {"node": {
        "adapter": "node", "version": "24.0.0",
        "declarations": [{"file": ".nvmrc", "request": "24.0.0"}],
        "installation": {"kind": "managed", "artifacts": {"windows-x64": {"distribution": {
            "url": format!("https://nodejs.org/dist/v24.0.0/{prefix}.zip"),
            "sha256": "0".repeat(64), "format": "zip", "rootPrefix": prefix,
            "executables": {"node": "node.exe"}
        }}}}
    }}});
    std::fs::create_dir_all(root).unwrap();
    for (name, contents) in [
        (
            "turbo.json",
            r#"{"futureFlags":{"experimentalSetup":true}}"#.to_owned(),
        ),
        (
            "package.json",
            r#"{"name":"fixture","private":true}"#.to_owned(),
        ),
        (".nvmrc", "24.0.0\n".to_owned()),
        (".gitignore", "/.turbo/\n".to_owned()),
        // Canonical bytes, so local setup has no reason to rewrite the lock.
        ("turbo.lock", format!("{lock}\n")),
    ] {
        std::fs::write(root.join(name), contents).unwrap();
    }
    common::git(root, &["init", "--quiet"]);
}

/// Repository files with contents, plus the shape of ignored setup state.
fn state(root: &Path) -> (BTreeMap<String, Vec<u8>>, Vec<String>) {
    let (mut files, mut setup) = (BTreeMap::new(), Vec::new());
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let relative = path.strip_prefix(root).unwrap().to_string_lossy();
            let relative = relative.replace('\\', "/");
            if relative == ".git" {
                continue;
            }
            if relative.starts_with(".turbo") {
                setup.push(format!(
                    "{relative}{}",
                    if path.is_dir() { "/" } else { "" }
                ));
            } else if path.is_file() {
                files.insert(relative, std::fs::read(&path).unwrap());
            }
            if path.is_dir() {
                pending.push(path);
            }
        }
    }
    setup.sort();
    (files, setup)
}

/// The official source policy rejects npm configuration in the environment,
/// so start from a minimal Windows environment with owned profile folders.
fn command(launcher: &ProcessLauncher, launch: Launch, root: &Path, proxy: &str) -> Command {
    let entry = match launch {
        Launch::Standalone => Entrypoint::Standalone,
        _ => Entrypoint::NpmWrapper,
    };
    let mut command = launcher.command(entry, root);
    let home = root.parent().unwrap().join("home");
    command.env_clear().args(["setup", "--tools-only"]);
    for key in ["PATH", "SystemRoot", "SystemDrive", "windir", "TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    // The launcher's quieting settings, minus its npm_config_* overrides.
    for key in [
        "TURBO_NO_UPDATE_NOTIFIER",
        "TURBO_TELEMETRY_DISABLED",
        "TURBO_TELEMETRY_MESSAGE_DISABLED",
        "TURBO_GLOBAL_WARNING_DISABLED",
        "TURBO_PRINT_VERSION_DISABLED",
        "DO_NOT_TRACK",
    ] {
        command.env(key, "1");
    }
    command
        .env("USERPROFILE", &home)
        .env("APPDATA", home.join("AppData"))
        .env("TURBO_CONFIG_DIR_PATH", &home)
        .env("VERCEL_CONFIG_DIR_PATH", &home)
        .env("TURBO_DOWNLOAD_LOCAL_ENABLED", "0")
        .env("HTTP_PROXY", proxy)
        .env("HTTPS_PROXY", proxy)
        .env("ALL_PROXY", proxy)
        .env("NO_PROXY", "");
    if launch == Launch::WrapperUnderNpm {
        // A script marker that selects the wrapper's pipe. npm_config_* would
        // also select it, but official-only setup rejects npm configuration.
        command.env("npm_lifecycle_event", "setup");
    }
    command
}

/// Open handles pin process identity, so PID reuse cannot fake a survivor.
struct Tree(Vec<(u32, HANDLE)>);

impl Tree {
    /// The entrypoint and everything below it: the wrapper's native child and
    /// anything setup started.
    fn capture(root: u32) -> Self {
        let mut table = Vec::new();
        // SAFETY: Plain ToolHelp enumeration into an owned, sized entry.
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            assert_ne!(snapshot, INVALID_HANDLE_VALUE);
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut more = Process32FirstW(snapshot, &mut entry);
            while more != 0 {
                table.push((entry.th32ProcessID, entry.th32ParentProcessID));
                more = Process32NextW(snapshot, &mut entry);
            }
            CloseHandle(snapshot);
        }
        let mut pids = vec![root];
        let mut next = 0;
        while let Some(&parent) = pids.get(next) {
            next += 1;
            for &(pid, ppid) in &table {
                if ppid == parent && pid != parent && !pids.contains(&pid) {
                    pids.push(pid);
                }
            }
        }
        let access = PROCESS_SYNCHRONIZE | PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION;
        // SAFETY: Null handles (already exited processes) are skipped.
        Self(
            pids.into_iter()
                .map(|pid| (pid, unsafe { OpenProcess(access, 0, pid) }))
                .filter(|(_, handle)| !handle.is_null())
                .collect(),
        )
    }

    fn assert_gone(self, context: &str) {
        let start = Instant::now();
        for (pid, handle) in &self.0 {
            let left = TIMEOUT.saturating_sub(start.elapsed()).as_millis() as u32;
            // SAFETY: The handle stays open until drop.
            let waited = unsafe { WaitForSingleObject(*handle, left) };
            assert_ne!(
                waited, WAIT_TIMEOUT,
                "process {pid} outlived setup\n{context}"
            );
        }
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        for (_, handle) in self.0.drain(..) {
            // SAFETY: Owned handles. Terminating an exited process is a no-op.
            unsafe {
                TerminateProcess(handle, 1);
                CloseHandle(handle);
            }
        }
    }
}

enum Run {
    Console(CapturedProcess),
    Pty {
        child: Box<dyn portable_pty::Child + Send + Sync>,
        input: Arc<Mutex<Box<dyn Write + Send>>>,
        output: Arc<Mutex<Vec<u8>>>,
        _master: Box<dyn portable_pty::MasterPty + Send>,
    },
}

impl Run {
    fn spawn(mut command: Command, stop: Stop) -> Self {
        if stop != Stop::Pty {
            command.creation_flags(CREATE_NEW_CONSOLE).show_window(0);
            return Self::Console(CapturedProcess::spawn(&mut command));
        }
        let mut builder = CommandBuilder::new(command.get_program());
        builder.args(command.get_args());
        builder.cwd(command.get_current_dir().unwrap());
        builder.env_clear();
        for (key, value) in command.get_envs() {
            if let Some(value) = value {
                builder.env(key, value);
            }
        }
        let size = PtySize {
            rows: 40,
            cols: 200,
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = native_pty_system().openpty(size).unwrap();
        let child = pair.slave.spawn_command(builder).unwrap();
        drop(pair.slave);
        let input = Arc::new(Mutex::new(pair.master.take_writer().unwrap()));
        let output = Arc::new(Mutex::new(Vec::new()));
        let mut reader = pair.master.try_clone_reader().unwrap();
        let (sink, answer) = (output.clone(), input.clone());
        // Some conhost versions block output until a cursor query is answered.
        thread::spawn(move || {
            let mut buffer = [0; 4096];
            while let Ok(n @ 1..) = reader.read(&mut buffer) {
                let mut output = sink.lock().unwrap();
                output.extend_from_slice(&buffer[..n]);
                if buffer[..n].windows(4).any(|w| w == b"\x1b[6n") {
                    let _ = answer.lock().unwrap().write_all(b"\x1b[1;1R");
                }
            }
        });
        Self::Pty {
            child,
            input,
            output,
            _master: pair.master,
        }
    }

    fn pid(&self) -> u32 {
        match self {
            Self::Console(process) => process.child.id(),
            Self::Pty { child, .. } => child.process_id().unwrap(),
        }
    }

    fn exit_code(&mut self) -> Option<i32> {
        match self {
            Self::Console(process) => process.child.try_wait().unwrap().map(|s| s.code().unwrap()),
            Self::Pty { child, .. } => child.try_wait().unwrap().map(|s| s.exit_code() as i32),
        }
    }

    fn stop(&mut self, launcher: &ProcessLauncher) {
        match self {
            Self::Pty { input, .. } => {
                let mut input = input.lock().unwrap();
                input.write_all(b"\x03").unwrap();
                input.flush().unwrap();
            }
            // Turbo's helper raises Ctrl+C on the entrypoint's console only.
            Self::Console(process) => {
                let status = Command::new(&launcher.standalone)
                    .args(["__internal_windows_ctrl_c", "ctrl_c"])
                    .arg(process.child.id().to_string())
                    .status()
                    .unwrap();
                assert!(status.success());
            }
        }
    }

    fn output(self) -> String {
        match self {
            Self::Console(process) => common::combined_output(&process.finish(TIMEOUT)),
            Self::Pty { output, .. } => String::from_utf8_lossy(&output.lock().unwrap()).into(),
        }
    }
}

/// Starts setup and returns once it is blocked on the Node download.
fn start_blocked(
    launcher: &ProcessLauncher,
    launch: Launch,
    stop: Stop,
    root: &Path,
) -> (Run, Tree) {
    // CI runners can start tests with Ctrl+C ignored, a state every child
    // inherits. Restore the interactive default; nothing signals this console.
    // SAFETY: Changes only this process's inheritable Ctrl+C disposition.
    unsafe { SetConsoleCtrlHandler(None, 0) };
    let (proxy, requests) = stalled_proxy();
    let mut run = Run::spawn(command(launcher, launch, root, &proxy), stop);
    let start = Instant::now();
    let request = loop {
        if let Ok(request) = requests.recv_timeout(Duration::from_millis(50)) {
            break request;
        }
        if run.exit_code().is_some() {
            panic!("setup exited before downloading\n{}", run.output());
        }
        assert!(
            start.elapsed() < TIMEOUT,
            "setup never reached the download"
        );
    };
    assert_eq!(request, "CONNECT nodejs.org:443 HTTP/1.1");
    // The download runs inside the setup transaction.
    assert!(matches!(
        turborepo_tool_install::Store::open(root),
        Err(turborepo_tool_install::Error::Busy)
    ));
    let tree = Tree::capture(run.pid());
    (run, tree)
}

#[test]
fn interrupted_download_releases_locks_and_leaves_resumable_state() {
    let launcher = ProcessLauncher::new();
    let cases = [
        (
            Stop::Console,
            [Launch::Standalone, Launch::Wrapper].as_slice(),
        ),
        (
            Stop::Pty,
            &[Launch::Standalone, Launch::Wrapper, Launch::WrapperUnderNpm],
        ),
        (Stop::Terminate, &[Launch::Standalone, Launch::Wrapper]),
    ];
    for (stop, launches) in cases {
        let mut codes = Vec::new();
        for &launch in launches {
            let context = format!("{launch:?} {stop:?}");
            eprintln!("case: {context}");
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap().join("repo");
            fixture(&root);
            let (before, _) = state(&root);
            let (mut run, tree) = start_blocked(&launcher, launch, stop, &root);
            match (&mut run, stop) {
                (Run::Console(process), Stop::Terminate) => process.child.kill().unwrap(),
                _ => run.stop(&launcher),
            }
            // Setup has no graceful phase: one interrupt ends it promptly,
            // with no second Ctrl+C or force escalation needed.
            let started = Instant::now();
            let code = loop {
                if let Some(code) = run.exit_code() {
                    break code;
                }
                if started.elapsed() > TIMEOUT {
                    let pid = run.pid();
                    let _ = Command::new("taskkill")
                        .args(["/T", "/F", "/PID"])
                        .arg(pid.to_string())
                        .status();
                    panic!("{context}: setup did not stop\n{}", run.output());
                }
                thread::sleep(Duration::from_millis(50));
            };
            let context = format!("{context}\n{}", run.output());
            tree.assert_gone(&context);
            codes.push(code);
            let (after, setup) = state(&root);
            assert_eq!(
                after, before,
                "committed files, including turbo.lock\n{context}"
            );
            // Only the reusable lock files remain: no staging, partial
            // artifact, or published generation.
            assert_eq!(
                setup,
                [
                    ".turbo/",
                    ".turbo/setup-lock/",
                    ".turbo/setup-lock/writer",
                    ".turbo/tools/",
                    ".turbo/tools/transaction.lock",
                ],
                "{context}"
            );
            let store = turborepo_tool_install::Store::open(&root).expect(&context);
            assert!(store.current().unwrap().is_none(), "{context}");
            drop(store);
            // A retry reaches the download again instead of waiting on a stale
            // writer lock or rejecting the interrupted state.
            let (Run::Console(mut retry), retry_tree) =
                start_blocked(&launcher, Launch::Standalone, Stop::Terminate, &root)
            else {
                unreachable!()
            };
            retry.child.kill().unwrap();
            retry.finish(TIMEOUT);
            retry_tree.assert_gone(&context);
            assert_eq!(state(&root).0, before, "{context}");
        }
        let expected = if stop == Stop::Terminate {
            1
        } else {
            CTRL_C_EXIT
        };
        assert!(
            codes.iter().all(|code| *code == expected),
            "{stop:?}: {codes:?}"
        );
    }
}

/// Package managers also export npm_config_* settings, which official-only
/// setup rejects before any transaction, so the wrapper exits promptly.
#[test]
fn npm_configuration_fails_closed_before_any_transaction() {
    let launcher = ProcessLauncher::new();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("repo");
    fixture(&root);
    let before = state(&root);
    let (proxy, requests) = stalled_proxy();
    let mut command = command(&launcher, Launch::Wrapper, &root, &proxy);
    command.env("npm_config_user_agent", "npm/10.5.0 node/v22.0.0 win32 x64");
    let output = CapturedProcess::spawn(&mut command).finish(TIMEOUT);
    let text = common::combined_output(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(
        text.contains("rejects npm configuration environment overrides"),
        "{text}"
    );
    assert!(requests.try_recv().is_err(), "{text}");
    assert_eq!(state(&root), before);
}
