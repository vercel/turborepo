#![allow(clippy::unwrap_used)]
//! Windows installer waits through a managed-style batch launcher chain and
//! the host's node.exe: exact arguments from a checkout path with cmd.exe
//! metacharacters, exit codes, the `%` refusal, a console Ctrl+C that reaches
//! the installer once, and no survivors when setup ends first.
use std::{
    env, fs,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Stdio,
    thread,
    time::Instant,
};

use serde_json::{Value, json};
use windows_sys::Win32::{
    Foundation::{CloseHandle, WAIT_OBJECT_0},
    System::{
        Console::{CTRL_C_EVENT, GenerateConsoleCtrlEvent, SetConsoleCtrlHandler},
        Threading::{
            CREATE_NO_WINDOW, OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
            TerminateProcess, WaitForSingleObject,
        },
    },
};

use super::*;

const HOST: &str = "TURBO_SETUP_INSTALLER_TEST_HOST";
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// What the default console handler exits with, as reported by Windows.
const CTRL_C_EXIT: i32 = 0xC000013Au32 as i32;

/// Records its arguments and exits 7. With `serve`, it waits for Ctrl+C
/// beside a descendant that ignores Ctrl+C.
const INSTALLER: &str = r#"
const fs = require('fs');
if (process.argv[2] !== 'serve') {
  fs.writeFileSync('args.json', JSON.stringify(process.argv.slice(2)));
  process.exit(7);
}
const child = require('child_process').spawn(process.execPath, ['-e',
  "process.on('SIGINT', () => {}); setInterval(() => {}, 1000)"], { stdio: 'ignore' });
process.on('SIGINT', () => {
  fs.appendFileSync('interrupts', 'x');
  setTimeout(() => process.exit(6), 500);
});
setInterval(() => {}, 1000);
fs.writeFileSync('pids.json', JSON.stringify([process.pid, child.pid]));
"#;

/// Setup's launcher shape under `dir`: `bin\pnpm.cmd` forwards to a tool
/// launcher that runs the installer script with a copy of the host node.exe.
fn launcher(dir: &Path) -> PathBuf {
    let tool = dir.join(r"tools\pnpm\managed-node-fixture");
    for path in [&tool, &dir.join(r"tools\node"), &dir.join("bin")] {
        fs::create_dir_all(path).unwrap();
    }
    let (host, node) = (
        which::which("node").unwrap(),
        dir.join(r"tools\node\node.exe"),
    );
    fs::hard_link(&host, &node)
        .or_else(|_| fs::copy(&host, &node).map(drop))
        .unwrap();
    fs::write(dir.join(r"tools\pnpm\installer.cjs"), INSTALLER).unwrap();
    let run = "@\"%~dp0..\\..\\node\\node.exe\" \"%~dp0..\\installer.cjs\" %*\r\n";
    fs::write(tool.join("pnpm.cmd"), run).unwrap();
    let forward = "@\"%~dp0..\\tools\\pnpm\\managed-node-fixture\\pnpm.cmd\" %*\r\n";
    fs::write(dir.join(r"bin\pnpm.cmd"), forward).unwrap();
    dir.join(r"bin\pnpm.cmd")
}

#[test]
fn batch_installers_get_exact_arguments_and_exit_codes_from_metacharacter_paths() {
    let temp = tempfile::tempdir().unwrap();
    let work = temp.path().join("work");
    fs::create_dir(&work).unwrap();
    // cmd.exe metacharacters that a checkout path may contain.
    let pnpm = launcher(&temp.path().join("a b&c^(d)!e"));
    let args = [
        "install",
        "--frozen-lockfile",
        "--reporter=append-only",
        "--offline",
    ];
    let mut command = Command::new(&pnpm);
    command.args(args).current_dir(&work).stdin(Stdio::null());
    assert_eq!(status(&mut command).unwrap().code(), Some(7));
    let recorded: Value =
        serde_json::from_slice(&fs::read(work.join("args.json")).unwrap()).unwrap();
    assert_eq!(recorded, json!(args));

    // cmd.exe would expand `%PATH%` in the launcher path, so nothing starts.
    fs::remove_file(work.join("args.json")).unwrap();
    let pnpm = launcher(&temp.path().join("100%PATH%"));
    let mut command = Command::new(&pnpm);
    command
        .arg("install")
        .current_dir(&work)
        .stdin(Stdio::null());
    let error = status(&mut command).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(!work.join("args.json").exists());
}

fn wait_for(path: &Path) -> Vec<u32> {
    let start = Instant::now();
    loop {
        if let Ok(bytes) = fs::read(path)
            && let Ok(pids) = serde_json::from_slice(&bytes)
        {
            return pids;
        }
        assert!(start.elapsed() < TIMEOUT, "no {}", path.display());
        thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// Processes among `pids` still running after up to 10 seconds; ends them.
fn survivors(pids: &[u32]) -> usize {
    let mut survivors = 0;
    for &pid in pids {
        // SAFETY: plain handle queries; every opened handle is closed.
        unsafe {
            let handle = OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, 0, pid);
            if handle.is_null() {
                continue; // Already gone.
            }
            if WaitForSingleObject(handle, 10_000) != WAIT_OBJECT_0 {
                survivors += 1;
                TerminateProcess(handle, 1);
            }
            CloseHandle(handle);
        }
    }
    survivors
}

/// Runs in a dedicated hidden console, where Ctrl+C reaches every attached
/// process as a keypress would, but never the test runner's console.
fn host(spec: &str) {
    let spec: Value = serde_json::from_str(spec).unwrap();
    let work = PathBuf::from(spec["work"].as_str().unwrap());
    let pnpm = PathBuf::from(spec["pnpm"].as_str().unwrap());
    // GitHub's Windows runners start tests with Ctrl+C ignored, a state
    // every child inherits. Restore the interactive default.
    unsafe { SetConsoleCtrlHandler(None, 0) };
    let installer = {
        let work = work.clone();
        thread::spawn(move || {
            let mut command = Command::new(pnpm);
            command.arg("serve").current_dir(work).stdin(Stdio::null());
            status(&mut command)
        })
    };
    let pids = wait_for(&work.join("pids.json"));
    if spec["stop"] == "exit" {
        // Like the npm wrapper's relayed Ctrl+C: setup ends while it waits.
        crate::process_exit::exit(CTRL_C_EXIT);
    }
    assert_ne!(unsafe { GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0) }, 0);
    let status = installer.join().unwrap().unwrap();
    let result = json!({ "status": status.code(), "survivors": survivors(&pids) });
    fs::write(work.join("result.json"), result.to_string()).unwrap();
}

#[test]
fn ctrl_c_reaches_the_installer_once_and_no_installer_process_outlives_setup() {
    if let Ok(spec) = env::var(HOST) {
        return host(&spec);
    }
    let temp = tempfile::tempdir().unwrap();
    let pnpm = launcher(&temp.path().join("generation"));
    for stop in ["ctrl-c", "exit"] {
        let work = temp.path().join(stop);
        fs::create_dir(&work).unwrap();
        let log = work.join("host.log");
        let spec = json!({ "work": work, "pnpm": pnpm, "stop": stop });
        let mut host = Command::new(env::current_exe().unwrap())
            .args([
                "commands::setup::installer::windows_tests::ctrl_c_reaches_the_installer_once_and_no_installer_process_outlives_setup",
                "--exact",
                "--nocapture",
            ])
            .env(HOST, spec.to_string())
            .stdin(Stdio::null())
            .stdout(fs::File::create(&log).unwrap())
            .stderr(Stdio::inherit())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .unwrap();
        let start = Instant::now();
        let code = loop {
            if let Some(status) = host.try_wait().unwrap() {
                break status.code();
            }
            if start.elapsed() > TIMEOUT {
                let _ = host.kill();
                panic!("{stop}: host hung\n{}", fs::read_to_string(&log).unwrap());
            }
            thread::sleep(std::time::Duration::from_millis(50));
        };
        let log = fs::read_to_string(&log).unwrap();
        if stop == "exit" {
            // The job ends the installer, its cmd.exe and its descendant.
            assert_eq!(code, Some(CTRL_C_EXIT), "{log}");
            assert_eq!(survivors(&wait_for(&work.join("pids.json"))), 0, "{log}");
            assert!(!work.join("interrupts").exists());
            continue;
        }
        assert_eq!(code, Some(0), "{log}");
        // The installer's own status, after it handled exactly one Ctrl+C;
        // its Ctrl+C-ignoring descendant ended with the job.
        let result = fs::read_to_string(work.join("result.json")).unwrap();
        let result: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(result, json!({ "status": 6, "survivors": 0 }), "{log}");
        assert_eq!(fs::read_to_string(work.join("interrupts")).unwrap(), "x");
    }
}
