//! Native Windows parity for `turbo run` through the standalone binary and the
//! installed npm wrapper. Direct turbo receives Ctrl+C as a console control
//! event. Under a package manager with a terminal, the wrapper reads Ctrl+C in
//! raw mode and relays it over its fd 3 pipe instead. `turbo exec` with
//! managed `.exe` and `.cmd` programs (installed by a real `turbo setup` from a
//! loopback mirror) gets the same entrypoint matrix; it leaves the console to
//! the command, so no entrypoint relays Ctrl+C for it.
#![cfg(windows)]
#![feature(windows_process_extensions_show_window)]
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{
    fs,
    io::{Read, Write},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use common::{
    combined_output,
    process_launcher::{CapturedProcess, Entrypoint, ProcessLauncher},
};
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde_json::json;
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
const SW_HIDE: u16 = 0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Launch {
    Standalone,
    Wrapper,
    /// The wrapper with npm's environment, as `npm run` or `npx` starts it.
    /// With a terminal this selects the raw-mode Ctrl+C pipe.
    WrapperUnderNpm,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Terminal {
    /// Piped stdio on a dedicated hidden console that Ctrl+C can target.
    Console,
    /// A ConPTY, where Ctrl+C is a keypress.
    Pty,
}

const CASES: [(Terminal, Launch); 5] = [
    (Terminal::Console, Launch::Standalone),
    (Terminal::Console, Launch::Wrapper),
    (Terminal::Pty, Launch::Standalone),
    (Terminal::Pty, Launch::Wrapper),
    (Terminal::Pty, Launch::WrapperUnderNpm),
];

const TASK: &str = r#"
const fs = require('fs');
if (process.argv[2] !== undefined) {
  console.log('PARITY_STDOUT');
  console.error('PARITY_STDERR');
  process.exit(Number(process.argv[2]));
}
const child = require('child_process').spawn(process.execPath, ['-e',
  "process.on('SIGINT', () => {}); setTimeout(() => process.exit(98), 60000)"],
  { stdio: 'ignore' });
let interrupts = 0;
process.on('SIGINT', () => {
  fs.appendFileSync('interrupts', 'x');
  if (interrupts++) return;
  console.log('parity cleanup start');
  setInterval(() => {
    if (!fs.existsSync('release')) return;
    child.kill();
    fs.writeFileSync('cleanup.done', '');
    console.log('parity cleanup done');
    process.exit(0);
  }, 50);
});
setTimeout(() => { child.kill(); process.exit(99); }, 60000);
fs.writeFileSync('pids.json', JSON.stringify([process.pid, child.pid]));
setInterval(() => {}, 1000);
"#;

fn repo(root: &Path) {
    fs::write(
        root.join("package.json"),
        json!({
            "name": "windows-parity", "private": true, "packageManager": "npm@10.5.0",
            "scripts": { "dev": "node task.js" }
        })
        .to_string(),
    )
    .unwrap();
    fs::write(
        root.join("turbo.json"),
        r#"{"tasks":{"dev":{"cache":false,"persistent":true}}}"#,
    )
    .unwrap();
    fs::write(root.join("task.js"), TASK).unwrap();
    common::git(root, &["init", "--quiet"]);
}

/// Open handles pin process identity, so PID reuse cannot fake a survivor.
struct Tree(Vec<(u32, HANDLE)>);

impl Tree {
    /// The entrypoint and everything below it: the wrapper's native child, the
    /// package manager and its shells, the task, its descendant, and conhosts.
    fn capture(root: u32, required: &[u32]) -> Self {
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
        for pid in required {
            assert!(
                pids.contains(pid),
                "process {pid} is not below {root}: {pids:?}"
            );
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

    fn assert_gone(self) {
        let start = Instant::now();
        for (pid, handle) in &self.0 {
            let left = TIMEOUT.saturating_sub(start.elapsed()).as_millis() as u32;
            // SAFETY: The handle stays open until drop.
            let waited = unsafe { WaitForSingleObject(*handle, left) };
            assert_ne!(waited, WAIT_TIMEOUT, "process {pid} outlived the run");
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

type Input = Arc<Mutex<Box<dyn Write + Send>>>;

struct Pty {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    // Dropping the master closes the pseudoconsole, which ends the output.
    master: Option<Box<dyn MasterPty + Send>>,
    input: Input,
    output: Arc<Mutex<Vec<u8>>>,
    reader: Option<thread::JoinHandle<()>>,
}

/// Collects output and answers the pseudoconsole's cursor position queries
/// as a terminal would; some conhost versions block output until answered.
fn read_terminal(mut reader: Box<dyn Read + Send>, output: Arc<Mutex<Vec<u8>>>, input: Input) {
    let mut buffer = [0; 4096];
    let mut answered = 0;
    while let Ok(n @ 1..) = reader.read(&mut buffer) {
        let mut output = output.lock().unwrap();
        output.extend_from_slice(&buffer[..n]);
        let asked = output.windows(4).filter(|w| w == b"\x1b[6n").count();
        for _ in answered..asked {
            let mut input = input.lock().unwrap();
            let _ = input.write_all(b"\x1b[1;1R").and_then(|()| input.flush());
        }
        answered = asked;
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        let _ = self.child.kill();
        self.master.take();
        if let Some(reader) = self.reader.take()
            && reader.is_finished()
        {
            let _ = reader.join();
        }
    }
}

enum Run {
    Console(Option<CapturedProcess>),
    Pty(Pty),
}

struct Outcome {
    code: i32,
    output: String,
    streams: Option<[String; 2]>,
}

/// Drops terminal control sequences so text checks see what a user reads.
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
            '\r' => out.push('\n'),
            c => out.push(c),
        }
    }
    out
}

impl Run {
    fn spawn(
        launcher: &ProcessLauncher,
        launch: Launch,
        terminal: Terminal,
        root: &Path,
        args: &[&str],
    ) -> Self {
        let entry = match launch {
            Launch::Standalone => Entrypoint::Standalone,
            Launch::Wrapper | Launch::WrapperUnderNpm => Entrypoint::NpmWrapper,
        };
        let mut command = launcher.command(entry, root);
        command.args(args);
        if launch == Launch::WrapperUnderNpm {
            command.env("npm_config_user_agent", "npm/10.5.0 node/v22.0.0 win32 x64");
        }
        if terminal == Terminal::Console {
            command
                .creation_flags(CREATE_NEW_CONSOLE)
                .show_window(SW_HIDE);
            return Self::Console(Some(CapturedProcess::spawn(&mut command)));
        }
        let mut builder = CommandBuilder::new(command.get_program());
        builder.args(command.get_args());
        builder.cwd(root);
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
        let child = pair.slave.spawn_command(builder).unwrap();
        drop(pair.slave);
        let output = Arc::new(Mutex::new(Vec::new()));
        let input: Input = Arc::new(Mutex::new(pair.master.take_writer().unwrap()));
        let reader = pair.master.try_clone_reader().unwrap();
        let reader = {
            let (output, input) = (output.clone(), input.clone());
            thread::spawn(move || read_terminal(reader, output, input))
        };
        Self::Pty(Pty {
            child,
            master: Some(pair.master),
            input,
            output,
            reader: Some(reader),
        })
    }

    fn pid(&self) -> u32 {
        match self {
            Self::Console(process) => process.as_ref().unwrap().child.id(),
            Self::Pty(pty) => pty.child.process_id().unwrap(),
        }
    }

    fn exited(&mut self) -> bool {
        match self {
            Self::Console(process) => process
                .as_mut()
                .unwrap()
                .child
                .try_wait()
                .unwrap()
                .is_some(),
            Self::Pty(pty) => pty.child.try_wait().unwrap().is_some(),
        }
    }

    fn fail(&mut self, message: &str) -> ! {
        let output = match self {
            Self::Console(process) => {
                let mut process = process.take().unwrap();
                let _ = Command::new("taskkill")
                    .args(["/T", "/F", "/PID"])
                    .arg(process.child.id().to_string())
                    .output();
                let _ = process.child.kill();
                combined_output(&process.finish(TIMEOUT))
            }
            Self::Pty(pty) => plain(&pty.output.lock().unwrap()),
        };
        panic!("{message}\n{output}");
    }

    fn wait_for(&mut self, marker: &Path) {
        let start = Instant::now();
        while !marker.exists() {
            if self.exited() {
                self.fail(&format!("entrypoint exited before {}", marker.display()));
            }
            if start.elapsed() > TIMEOUT {
                self.fail(&format!("timed out waiting for {}", marker.display()));
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn assert_running(&mut self, context: &str) {
        if self.exited() {
            self.fail(&format!("entrypoint exited {context}"));
        }
    }

    fn ctrl_c(&mut self, launcher: &ProcessLauncher) {
        match self {
            Self::Pty(pty) => {
                let mut input = pty.input.lock().unwrap();
                input.write_all(b"\x03").unwrap();
                input.flush().unwrap();
            }
            // Turbo's own helper: attach to the entrypoint's console only and
            // raise a console Ctrl+C there, never in the test runner's console.
            Self::Console(process) => {
                let pid = process.as_ref().unwrap().child.id();
                let status = Command::new(&launcher.standalone)
                    .args(["__internal_windows_ctrl_c", "ctrl_c"])
                    .arg(pid.to_string())
                    .status()
                    .unwrap();
                assert!(status.success(), "failed to raise Ctrl+C for {pid}");
            }
        }
    }

    fn finish(mut self) -> Outcome {
        let start = Instant::now();
        while !self.exited() {
            if start.elapsed() > TIMEOUT {
                self.fail("entrypoint did not exit");
            }
            thread::sleep(Duration::from_millis(50));
        }
        match self {
            Self::Console(mut process) => {
                let output = process.take().unwrap().finish(TIMEOUT);
                Outcome {
                    code: output.status.code().unwrap(),
                    streams: Some(
                        [&output.stdout, &output.stderr]
                            .map(|stream| plain(stream).replace('\n', "")),
                    ),
                    output: plain(combined_output(&output).as_bytes()),
                }
            }
            Self::Pty(mut pty) => {
                let code = pty.child.wait().unwrap().exit_code() as i32;
                pty.master.take();
                let reader = pty.reader.take().unwrap();
                while !reader.is_finished() {
                    assert!(
                        start.elapsed() < TIMEOUT,
                        "pseudoconsole output did not close"
                    );
                    thread::sleep(Duration::from_millis(20));
                }
                reader.join().unwrap();
                let output = plain(&pty.output.lock().unwrap());
                Outcome {
                    code,
                    output,
                    streams: None,
                }
            }
        }
    }
}

/// Starts the blocked task and returns once it and its descendant run.
fn start(
    launcher: &ProcessLauncher,
    launch: Launch,
    terminal: Terminal,
    args: &[&str],
) -> (tempfile::TempDir, Run, Tree) {
    let dir = tempfile::tempdir().unwrap();
    repo(dir.path());
    let words = [&["run", "dev", "--ui=stream"], args].concat();
    let (run, tree) = start_in(launcher, launch, terminal, dir.path(), &words);
    (dir, run, tree)
}

fn start_in(
    launcher: &ProcessLauncher,
    launch: Launch,
    terminal: Terminal,
    root: &Path,
    args: &[&str],
) -> (Run, Tree) {
    // CI runners can start tests with Ctrl+C ignored, a state every child
    // inherits that hides console Ctrl+C from turbo and its tasks. Restore the
    // interactive default. Nothing here signals this process's own console.
    // SAFETY: Changes only this process's inheritable Ctrl+C disposition.
    unsafe { SetConsoleCtrlHandler(None, 0) };
    let mut run = Run::spawn(launcher, launch, terminal, root, args);
    let pids = root.join("pids.json");
    run.wait_for(&pids);
    let [task, descendant]: [u32; 2] =
        serde_json::from_str(&fs::read_to_string(pids).unwrap()).unwrap();
    let tree = Tree::capture(run.pid(), &[task, descendant]);
    (run, tree)
}

/// The pipe path synthesizes a Ctrl+C in each task's own console. The
/// console path never needs to, because tasks share the user's console.
fn assert_delivery_path(output: &str, launch: Launch, terminal: Terminal) {
    let pipe = launch == Launch::WrapperUnderNpm && terminal == Terminal::Pty;
    assert_eq!(
        output.contains("generated console Ctrl-C for child console"),
        pipe,
        "{launch:?} {terminal:?} took the wrong Ctrl+C path\n{output}"
    );
}

#[test]
fn exit_status_and_output_streams_match() {
    let launcher = ProcessLauncher::new();
    for terminal in [Terminal::Console, Terminal::Pty] {
        let mut results = Vec::new();
        for launch in [Launch::Standalone, Launch::Wrapper, Launch::WrapperUnderNpm] {
            eprintln!("case: {launch:?} {terminal:?}");
            let dir = tempfile::tempdir().unwrap();
            repo(dir.path());
            // Arguments after `--` reach the task script, which exits with 3.
            let args = ["run", "dev", "--ui=stream", "--", "3"];
            let outcome = Run::spawn(&launcher, launch, terminal, dir.path(), &args).finish();
            let context = format!("{launch:?} {terminal:?}\n{}", outcome.output);
            assert_eq!(outcome.code, 3, "{context}");
            for marker in ["PARITY_STDOUT", "PARITY_STDERR"] {
                assert!(outcome.output.contains(marker), "{context}");
            }
            let streams = outcome.streams.map(|streams| {
                streams.map(|stream| {
                    ["PARITY_STDOUT", "PARITY_STDERR"].map(|marker| stream.contains(marker))
                })
            });
            results.push((outcome.code, streams));
        }
        assert!(
            results.windows(2).all(|pair| pair[0] == pair[1]),
            "{terminal:?}: {results:?}"
        );
    }
}

#[test]
fn first_ctrl_c_waits_for_one_graceful_cleanup() {
    let launcher = ProcessLauncher::new();
    for (terminal, launch) in CASES {
        eprintln!("case: {launch:?} {terminal:?}");
        let (dir, mut run, tree) = start(&launcher, launch, terminal, &["-vv"]);
        run.ctrl_c(&launcher);
        run.wait_for(&dir.path().join("interrupts"));
        // A duplicated delivery would reach the task twice or force kill it.
        thread::sleep(Duration::from_millis(500));
        run.assert_running("while cleanup was blocked");
        fs::write(dir.path().join("release"), "").unwrap();
        let outcome = run.finish();
        let context = format!("{launch:?} {terminal:?}\n{}", outcome.output);
        assert_eq!(outcome.code, 0, "{context}");
        assert_eq!(
            fs::read(dir.path().join("interrupts")).unwrap(),
            b"x",
            "{context}"
        );
        assert!(dir.path().join("cleanup.done").exists(), "{context}");
        assert!(!outcome.output.contains("Force killed"), "{context}");
        assert_delivery_path(&outcome.output, launch, terminal);
        tree.assert_gone();
    }
}

#[test]
fn second_ctrl_c_force_kills_without_orphans() {
    let launcher = ProcessLauncher::new();
    for (terminal, launch) in CASES {
        eprintln!("case: {launch:?} {terminal:?}");
        let (dir, mut run, tree) = start(&launcher, launch, terminal, &[]);
        run.ctrl_c(&launcher);
        run.wait_for(&dir.path().join("interrupts"));
        run.assert_running("before the second Ctrl+C");
        run.ctrl_c(&launcher);
        let outcome = run.finish();
        let context = format!("{launch:?} {terminal:?}\n{}", outcome.output);
        assert_eq!(outcome.code, 0, "{context}");
        assert!(
            outcome.output.contains("Force killed Turborepo tasks"),
            "{context}"
        );
        assert!(!dir.path().join("cleanup.done").exists(), "{context}");
        tree.assert_gone();
    }
}

/// Windows has no SIGTERM: a supervisor terminates the entrypoint process.
/// Job objects must still take the whole task tree down with it.
#[test]
fn terminating_the_entrypoint_leaves_no_orphans() {
    let launcher = ProcessLauncher::new();
    for launch in [Launch::Standalone, Launch::Wrapper] {
        eprintln!("case: {launch:?}");
        let (_dir, mut run, tree) = start(&launcher, launch, Terminal::Console, &[]);
        let Run::Console(Some(process)) = &mut run else {
            unreachable!()
        };
        process.child.kill().unwrap();
        let outcome = run.finish();
        assert_eq!(outcome.code, 1, "{launch:?}\n{}", outcome.output);
        tree.assert_gone();
    }
}

/// A repository whose managed Node.js and pnpm a real `turbo setup` installed
/// from the loopback fixture mirror (`scripts/setup-fixture-mirror.mjs`, which
/// repackages the host's own Node.js). The mirror stays up, so any later
/// request is visible; nothing else is reachable.
struct Managed {
    _dir: tempfile::TempDir,
    root: PathBuf,
    mirror: Child,
    requests: PathBuf,
    served: usize,
    lock: Vec<u8>,
}

const CLOSED: &str = "http://127.0.0.1:9";

impl Managed {
    /// `None` where managed setup is unqualified: only Windows x64 hosts.
    fn new(launcher: &ProcessLauncher) -> Option<Self> {
        if !cfg!(target_arch = "x86_64") || common::windows_arm64_host() {
            return None;
        }
        let dir = tempfile::tempdir().unwrap();
        let (root, state) = (dir.path().join("repo"), dir.path().join("mirror"));
        fs::create_dir_all(root.join("apps/web")).unwrap();
        fs::create_dir(&state).unwrap();
        let scripts = common::manifest_dir().join("../../scripts");
        let mut mirror = Command::new(&launcher.node)
            .arg(scripts.join("setup-fixture-mirror.mjs"))
            .arg(&state)
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        let start = Instant::now();
        let ready = loop {
            if let Ok(ready) = fs::read_to_string(state.join("ready"))
                && ready.ends_with('\n')
            {
                break ready;
            }
            let running = mirror.try_wait().unwrap().is_none();
            assert!(
                running && start.elapsed() < TIMEOUT,
                "the mirror did not start"
            );
            thread::sleep(Duration::from_millis(50));
        };
        let (port, version) = ready.trim().split_once(' ').unwrap();
        let files = [
            ("turbo.json", r#"{"futureFlags":{"experimentalSetup":true}}"#.to_owned()),
            (".nvmrc", format!("{version}\n")),
            (
                "package.json",
                r#"{"name":"exec-parity","private":true,"packageManager":"pnpm@10.0.0"}"#.to_owned(),
            ),
            ("task.js", TASK.to_owned()),
            (
                "apps/web/package.json",
                r#"{"name":"web","scripts":{"probe":"node -e \"console.log('cmd-ok'); process.exit(6)\""}}"#
                    .to_owned(),
            ),
        ];
        for (name, contents) in files {
            fs::write(root.join(name), contents).unwrap();
        }
        common::git(&root, &["init", "--quiet"]);
        let mut setup = Command::new(&launcher.standalone);
        // CI selects frozen mode, and npm settings fail the official-source
        // preflight; neither belongs to this fixture.
        for (key, _) in std::env::vars_os() {
            let name = key.to_string_lossy().to_ascii_lowercase();
            if name.starts_with("turbo_") || name.starts_with("npm_") || name == "ci" {
                setup.env_remove(&key);
            }
        }
        let base = format!("https://127.0.0.1:{port}");
        let output = setup
            .current_dir(&root)
            .args(["setup", "--tools-only"])
            .env_remove("GITHUB_ACTIONS")
            .env(
                "SSL_CERT_FILE",
                scripts.join("fixtures/setup-mirror/ca.pem"),
            )
            .env("TURBO_SETUP_NODE_MIRROR", format!("{base}/node"))
            .env("TURBO_SETUP_NPM_REGISTRY", format!("{base}/npm"))
            .envs([
                ("HTTPS_PROXY", CLOSED),
                ("HTTP_PROXY", CLOSED),
                ("ALL_PROXY", CLOSED),
                ("NO_PROXY", "127.0.0.1"),
                ("TURBO_TELEMETRY_DISABLED", "1"),
            ])
            .output()
            .unwrap();
        if !output.status.success() {
            let _ = mirror.kill();
            panic!("managed setup failed: {}", combined_output(&output));
        }
        let requests = state.join("requests.log");
        Some(Self {
            served: fs::read_to_string(&requests).unwrap().lines().count(),
            lock: fs::read(root.join("turbo.lock")).unwrap(),
            requests,
            _dir: dir,
            root,
            mirror,
        })
    }

    /// `turbo exec` never downloads tools or changes the lock.
    fn assert_untouched(&self) {
        let served = fs::read_to_string(&self.requests).unwrap().lines().count();
        assert_eq!(served, self.served, "turbo exec contacted the mirror");
        assert_eq!(fs::read(self.root.join("turbo.lock")).unwrap(), self.lock);
    }

    /// Clears the task's markers between entrypoints.
    fn reset(&self) {
        for marker in ["pids.json", "interrupts", "release", "cleanup.done"] {
            let _ = fs::remove_file(self.root.join(marker));
        }
    }
}

impl Drop for Managed {
    fn drop(&mut self) {
        let _ = self.mirror.kill();
        let _ = self.mirror.wait();
    }
}

fn exec_command(launcher: &ProcessLauncher, launch: Launch, cwd: &Path) -> Command {
    let entry = match launch {
        Launch::Standalone => Entrypoint::Standalone,
        Launch::Wrapper | Launch::WrapperUnderNpm => Entrypoint::NpmWrapper,
    };
    let mut command = launcher.command(entry, cwd);
    command.envs([
        ("HTTPS_PROXY", CLOSED),
        ("HTTP_PROXY", CLOSED),
        ("ALL_PROXY", CLOSED),
    ]);
    if launch == Launch::WrapperUnderNpm {
        command.env("npm_config_user_agent", "npm/10.5.0 node/v22.0.0 win32 x64");
    }
    command
}

#[test]
fn managed_exec_delivers_one_ctrl_c_and_leaves_no_orphans() {
    let launcher = ProcessLauncher::new();
    let Some(managed) = Managed::new(&launcher) else {
        return;
    };
    let exec = ["exec", "--", "node", "task.js"];
    for (terminal, launch) in CASES {
        eprintln!("case: {launch:?} {terminal:?}");
        managed.reset();
        let (mut run, tree) = start_in(&launcher, launch, terminal, &managed.root, &exec);
        run.ctrl_c(&launcher);
        run.wait_for(&managed.root.join("interrupts"));
        // A duplicated delivery would reach the command twice.
        thread::sleep(Duration::from_millis(500));
        run.assert_running("while cleanup was blocked");
        fs::write(managed.root.join("release"), "").unwrap();
        let outcome = run.finish();
        let context = format!("{launch:?} {terminal:?}\n{}", outcome.output);
        assert_eq!(outcome.code, 0, "{context}");
        let interrupts = fs::read(managed.root.join("interrupts")).unwrap();
        assert_eq!(interrupts, b"x", "{context}");
        assert!(managed.root.join("cleanup.done").exists(), "{context}");
        // exec leaves the console to the command; nothing relays a second copy.
        assert!(
            !outcome.output.contains("generated console Ctrl-C"),
            "{context}"
        );
        tree.assert_gone();
    }
    // A terminated entrypoint takes the managed command's tree down with it.
    for launch in [Launch::Standalone, Launch::Wrapper] {
        eprintln!("case: {launch:?} terminated");
        managed.reset();
        let (mut run, tree) = start_in(&launcher, launch, Terminal::Console, &managed.root, &exec);
        let Run::Console(Some(process)) = &mut run else {
            unreachable!()
        };
        process.child.kill().unwrap();
        let outcome = run.finish();
        assert_eq!(outcome.code, 1, "{launch:?}\n{}", outcome.output);
        tree.assert_gone();
    }
    managed.assert_untouched();
}

#[test]
fn managed_exec_arguments_directory_stdin_and_exit_codes_match_across_entrypoints() {
    let launcher = ProcessLauncher::new();
    let Some(managed) = Managed::new(&launcher) else {
        return;
    };
    let web = managed.root.join("apps/web");
    let script = "let input = ''; process.stdin.on('data', (d) => (input += d)).on('end', () => { \
                  console.log(JSON.stringify([require('fs').realpathSync(process.execPath), \
                  process.cwd(), process.argv.slice(1), input])); process.exit(5); });";
    let words = ["a b", "", "100%", "q\"uote", "back\\slash\\", "^&|<>"];
    let mut results = Vec::new();
    for launch in [Launch::Standalone, Launch::Wrapper, Launch::WrapperUnderNpm] {
        let mut command = exec_command(&launcher, launch, &web);
        command
            .args(["exec", "--", "node", "-e", script])
            .args(words);
        let mut child = command.stdin(Stdio::piped()).spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"piped stdin")
            .unwrap();
        let output = child.wait_with_output().unwrap();
        let context = format!("{launch:?}\n{}", combined_output(&output));
        assert_eq!(output.status.code(), Some(5), "{context}");
        let (node, cwd, args, input): (String, PathBuf, Vec<String>, String) =
            serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            node.contains(r"\.turbo\tools\") && node.ends_with("node.exe"),
            "{context}"
        );
        assert_eq!(
            cwd.canonicalize().unwrap(),
            web.canonicalize().unwrap(),
            "{context}"
        );
        assert_eq!(
            (args, input.as_str()),
            (words.map(String::from).to_vec(), "piped stdin")
        );
        results.push(output.stdout);
    }
    assert!(
        results.windows(2).all(|pair| pair[0] == pair[1]),
        "{results:?}"
    );
    // Managed .cmd launchers resolve through PATHEXT and keep exit codes.
    for launch in [Launch::Standalone, Launch::Wrapper, Launch::WrapperUnderNpm] {
        let mut command = exec_command(&launcher, launch, &web);
        let output = command
            .args(["exec", "--", "pnpm", "run", "probe"])
            .output()
            .unwrap();
        let text = combined_output(&output);
        assert_eq!(output.status.code(), Some(6), "{launch:?}\n{text}");
        assert!(
            text.contains("fixture-pnpm 10.0.0 run probe") && text.contains("cmd-ok"),
            "{text}"
        );
    }
    managed.assert_untouched();
}
