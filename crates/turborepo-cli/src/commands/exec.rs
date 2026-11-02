//! `turbo exec`: run one program with the managed environment that `turbo run`
//! gives tasks. Never provisions, downloads, or writes setup state.

use std::{collections::BTreeMap, env, ffi::OsString, io, path::Path, process::ExitStatus};

use miette::Diagnostic;
use thiserror::Error;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_run::{EnvironmentVariableMap, ManagedEnvironment};
use turborepo_shim::{INVOCATION_CWD_ENV_VAR, INVOCATION_DIR_ENV_VAR};
use turborepo_ui::ColorConfig;

use crate::{
    cli::{self, Args, PrintEnv},
    commands::{CommandBase, managed},
    get_version,
};

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("`turbo exec` requires root futureFlags.experimentalSetup to be true")]
    #[diagnostic(
        code(turbo::exec::disabled),
        help(
            "Set \"futureFlags\": {{\"experimentalSetup\": true}} in the root turbo.json or \
             turbo.jsonc. No command was run. To run a task named exec, use `turbo run exec`."
        )
    )]
    Disabled,
    #[error(
        "`turbo exec` requires managed setup, but the repository root has no turbo.lock or `turbo \
         setup --no-lock` tools"
    )]
    #[diagnostic(
        code(turbo::exec::managed_setup_required),
        help(
            "Run `turbo setup` (or `turbo setup --no-lock` for this clone only) first. No command \
             was run. `turbo exec` never falls back to tools on the host PATH."
        )
    )]
    ManagedSetupRequired,
    #[error("managed setup is no longer valid: {0}")]
    #[diagnostic(
        code(turbo::exec::stale),
        help("Run `turbo setup`, then retry. No command was run.")
    )]
    Stale(String),
    #[error("`{program}` was not found on the managed PATH")]
    #[diagnostic(
        code(turbo::exec::not_found),
        help("Managed tools come first on PATH, followed by the caller's PATH.")
    )]
    NotFound { program: String },
    #[error("failed to run `{program}`: {source}")]
    Spawn { program: String, source: io::Error },
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub fn run(
    repo_root: &AbsoluteSystemPath,
    args: &Args,
    command: &[OsString],
    color_config: ColorConfig,
) -> Result<i32, cli::Error> {
    let base = CommandBase::new(
        args.clone(),
        repo_root.to_owned(),
        get_version(),
        color_config,
    )?;
    if !base.opts.future_flags.experimental_setup {
        return Err(Error::Disabled.into());
    }
    let Some(tools) = managed::select(&base)? else {
        return Err(Error::ManagedSetupRequired.into());
    };
    let cwd = invocation_cwd(args, |name| env::var_os(name))?;
    // Killing `turbo exec` must kill the program it runs. That holds in a
    // local CLI too (`--skip-infer`): global turbo hands off the same way.
    Ok(spawn(
        tools.as_ref(),
        command,
        &cwd,
        turborepo_process::hand_off,
    )?)
}

/// `turbo exec --print-env`: the environment `turbo exec` would give a
/// program, under the same strict authorization. Never installs anything and
/// never runs a command.
pub fn print_env(
    repo_root: &AbsoluteSystemPath,
    args: &Args,
    format: PrintEnv,
    color_config: ColorConfig,
) -> Result<i32, cli::Error> {
    let base = CommandBase::new(
        args.clone(),
        repo_root.to_owned(),
        get_version(),
        color_config,
    )?;
    if !base.opts.future_flags.experimental_setup {
        return Err(Error::Disabled.into());
    }
    let Some(tools) = managed::select_tools(&base)? else {
        return Err(Error::ManagedSetupRequired.into());
    };
    let (overrides, bin) = tools.activation().map_err(Error::Stale)?;
    print!("{}", render(format, &overrides, &bin)?);
    Ok(0)
}

pub(crate) fn render(
    format: PrintEnv,
    overrides: &EnvironmentVariableMap,
    bin: &Path,
) -> Result<String, Error> {
    let vars: BTreeMap<&str, &str> = overrides
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    // POSIX and fish single quotes, PowerShell single quotes.
    let sh = |value: &str| format!("'{}'", value.replace('\'', r"'\''"));
    let fish = |value: &str| format!("'{}'", value.replace('\\', r"\\").replace('\'', r"\'"));
    let pwsh = |value: &str| format!("'{}'", value.replace('\'', "''"));
    let mut out = String::new();
    match format {
        PrintEnv::Bin => {
            let bin = bin
                .to_str()
                .ok_or_else(|| Error::Stale("the managed bin path is not UTF-8".into()))?;
            out.push_str(bin);
            out.push('\n');
        }
        PrintEnv::Sh => {
            for (key, value) in vars {
                out.push_str(&format!("export {key}={}\n", sh(value)));
            }
        }
        PrintEnv::Fish => {
            for (key, value) in vars {
                // fish keeps PATH as a list of directories.
                let words: Vec<String> = if key == "PATH" {
                    env::split_paths(value)
                        .map(|path| fish(&path.to_string_lossy()))
                        .collect()
                } else {
                    vec![fish(value)]
                };
                out.push_str(&format!("set -gx {key} {}\n", words.join(" ")));
            }
        }
        PrintEnv::Pwsh => {
            for (key, value) in vars {
                out.push_str(&format!("$env:{key} = {}\n", pwsh(value)));
            }
        }
        PrintEnv::Json => {
            out.push_str(&serde_json::to_string_pretty(&vars)?);
            out.push('\n');
        }
    }
    Ok(out)
}

/// The program runs in `--cwd`, else where the user invoked turbo. Global
/// turbo consumes `--cwd` and starts a local CLI (`--skip-infer`) at the
/// repository root, passing both directories through the environment.
fn invocation_cwd(
    args: &Args,
    var: impl Fn(&str) -> Option<OsString>,
) -> Result<AbsoluteSystemPathBuf, Error> {
    if let Some(cwd) = &args.cwd {
        return Ok(AbsoluteSystemPathBuf::from_cwd(cwd)?);
    }
    if args.skip_infer
        && let Some(dir) = var(INVOCATION_CWD_ENV_VAR).or_else(|| var(INVOCATION_DIR_ENV_VAR))
    {
        return Ok(AbsoluteSystemPathBuf::try_from(std::path::Path::new(&dir))?);
    }
    Ok(AbsoluteSystemPathBuf::cwd()?)
}

pub(crate) fn spawn(
    tools: &dyn ManagedEnvironment,
    command: &[OsString],
    cwd: &AbsoluteSystemPath,
    launch: impl FnOnce(std::process::Command) -> io::Result<ExitStatus>,
) -> Result<i32, Error> {
    let Some((program, arguments)) = command.split_first() else {
        unreachable!("the exec grammar requires a program");
    };
    let display = || program.to_string_lossy().into_owned();
    // Selection-time validation is not a lease: recheck at the spawn boundary.
    let overrides = tools.revalidate().map_err(Error::Stale)?;
    let path = overrides
        .get("PATH")
        .map(OsString::from)
        .or_else(|| env::var_os("PATH"));
    let executable = which::which_in(program, path, cwd.as_std_path())
        .map_err(|_| Error::NotFound { program: display() })?;
    let mut child = std::process::Command::new(executable);
    #[cfg(unix)]
    std::os::unix::process::CommandExt::arg0(&mut child, program);
    child
        .args(arguments)
        .current_dir(cwd.as_std_path())
        .envs(overrides.iter());
    let status = launch(child).map_err(|source| Error::Spawn {
        program: display(),
        source,
    })?;
    Ok(exit_code(status))
}

/// A child's signal death becomes the shell's `128 + signal`, the status a
/// shell reports for a program it ran directly. Windows exit codes, including
/// NTSTATUS values such as `STATUS_CONTROL_C_EXIT`, pass through unchanged.
pub(crate) fn exit_code(status: ExitStatus) -> i32 {
    #[cfg(unix)]
    if let Some(signal) = std::os::unix::process::ExitStatusExt::signal(&status) {
        return 128 + signal;
    }
    status.code().unwrap_or(1)
}

#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::{
        fs,
        os::unix::{fs::PermissionsExt, process::ExitStatusExt},
        sync::atomic::{AtomicUsize, Ordering},
    };

    use turborepo_run::EnvironmentVariableMap;

    use super::*;

    struct Tools {
        path: String,
        checks: AtomicUsize,
        stale: bool,
    }

    impl ManagedEnvironment for Tools {
        fn revalidate(&self) -> Result<EnvironmentVariableMap, String> {
            self.checks.fetch_add(1, Ordering::SeqCst);
            if self.stale {
                return Err("the managed pnpm generation changed".into());
            }
            let mut map = EnvironmentVariableMap::default();
            map.insert("PATH".into(), self.path.clone());
            map.insert("MANAGED_RUNTIME".into(), "1".into());
            Ok(map)
        }

        fn cache_identity(&self) -> &str {
            "fixture"
        }
    }

    fn tool(dir: &std::path::Path, name: &str, body: &str) {
        let path = dir.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn fixture(stale: bool) -> (tempfile::TempDir, Tools) {
        let temp = tempfile::tempdir().unwrap();
        for dir in ["managed", "host", "work"] {
            fs::create_dir(temp.path().join(dir)).unwrap();
        }
        // Managed tools shadow a host tool of the same name.
        tool(
            &temp.path().join("managed"),
            "node",
            "pwd > record; printf '[%s]\\n' \"$@\" >> record; echo \"$MANAGED_RUNTIME\" >> \
             record; exit 7",
        );
        tool(&temp.path().join("host"), "node", "touch host-ran");
        let path = env::join_paths([temp.path().join("managed"), temp.path().join("host")])
            .unwrap()
            .into_string()
            .unwrap();
        let tools = Tools {
            path,
            checks: AtomicUsize::new(0),
            stale,
        };
        (temp, tools)
    }

    fn plain(mut command: std::process::Command) -> io::Result<ExitStatus> {
        command.status()
    }

    #[test]
    fn managed_tool_runs_with_exact_arguments_cwd_environment_and_exit_status() {
        let (temp, tools) = fixture(false);
        let work = AbsoluteSystemPathBuf::try_from(temp.path().join("work")).unwrap();
        let command: Vec<OsString> = ["node", "a b", "", "--", "--cwd", "*"]
            .into_iter()
            .map(OsString::from)
            .collect();
        assert_eq!(spawn(&tools, &command, &work, plain).unwrap(), 7);
        assert_eq!(tools.checks.load(Ordering::SeqCst), 1);
        let record = fs::read_to_string(work.as_std_path().join("record")).unwrap();
        let mut lines = record.lines();
        assert_eq!(
            fs::canonicalize(lines.next().unwrap()).unwrap(),
            fs::canonicalize(work.as_std_path()).unwrap()
        );
        assert_eq!(
            lines.collect::<Vec<_>>(),
            ["[a b]", "[]", "[--]", "[--cwd]", "[*]", "1"]
        );
        assert!(!work.as_std_path().join("host-ran").exists());
        assert!(env::var_os("MANAGED_RUNTIME").is_none());
    }

    #[test]
    fn stale_or_missing_tools_never_start_a_process() {
        let (temp, tools) = fixture(true);
        let work = AbsoluteSystemPathBuf::try_from(temp.path().join("work")).unwrap();
        let never = |_| -> io::Result<ExitStatus> { panic!("must not spawn") };
        let command = [OsString::from("node")];
        assert!(matches!(
            spawn(&tools, &command, &work, never),
            Err(Error::Stale(_))
        ));
        let (temp, tools) = fixture(false);
        let work = AbsoluteSystemPathBuf::try_from(temp.path().join("work")).unwrap();
        let command = [OsString::from("pnpm")];
        assert!(matches!(
            spawn(&tools, &command, &work, never),
            Err(Error::NotFound { program }) if program == "pnpm"
        ));
    }

    const HOST: &str = "TURBO_EXEC_TEST_REPLACE_HOST";

    /// Runs exec like the CLI does, in a process the test can signal.
    fn exec_host(spec: &str) {
        let spec: serde_json::Value = serde_json::from_str(spec).unwrap();
        let tools = Tools {
            path: spec["path"].as_str().unwrap().to_owned(),
            checks: AtomicUsize::new(0),
            stale: false,
        };
        let work = AbsoluteSystemPathBuf::new(spec["work"].as_str().unwrap()).unwrap();
        let launch = turborepo_process::hand_off;
        let status = spawn(&tools, &[OsString::from("serve")], &work, launch).unwrap();
        fs::write(work.as_std_path().join("status"), status.to_string()).unwrap();
    }

    /// SIGKILL to exec is SIGKILL to the program: exec became it. A local
    /// CLI that global turbo handed off to does the same, because global
    /// turbo became that CLI.
    #[test]
    fn exec_becomes_the_program() {
        if let Ok(spec) = env::var(HOST) {
            return exec_host(&spec);
        }
        let (temp, tools) = fixture(false);
        let body = "trap 'exit 5' TERM\necho $$ > ready\nwhile :; do :; done";
        tool(&temp.path().join("managed"), "serve", body);
        let work = temp.path().join("work");
        let spec = serde_json::json!({ "path": tools.path, "work": work });
        let mut host = std::process::Command::new(env::current_exe().unwrap())
            .args(["commands::exec::tests::exec_becomes_the_program", "--exact"])
            .env(HOST, spec.to_string())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let ready = work.join("ready");
        let started = std::time::Instant::now();
        let pid: i32 = loop {
            if let Some(pid) = fs::read_to_string(&ready)
                .ok()
                .and_then(|text| text.trim().parse().ok())
            {
                break pid;
            }
            assert!(started.elapsed().as_secs() < 30, "never started");
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        let host_pid = host.id() as i32;
        if pid != host_pid {
            unsafe { libc::kill(pid, libc::SIGKILL) };
            let _ = host.kill();
            panic!("exec is {host_pid}, the program is {pid}");
        }
        unsafe { libc::kill(host_pid, libc::SIGKILL) };
        assert_eq!(host.wait().unwrap().signal(), Some(libc::SIGKILL));
    }

    #[test]
    fn signal_deaths_use_the_shell_status() {
        assert_eq!(exit_code(ExitStatus::from_raw(libc::SIGTERM)), 143);
        assert_eq!(exit_code(ExitStatus::from_raw(3 << 8)), 3);
    }

    #[test]
    fn handoff_directories_apply_only_to_a_local_cli() {
        let args = |words: &[&str]| {
            Args::parse_args(
                ["turbo"]
                    .iter()
                    .chain(words)
                    .chain(&["exec", "--", "node"])
                    .map(OsString::from)
                    .collect(),
            )
            .unwrap()
        };
        let temp = tempfile::tempdir().unwrap();
        let (flag, handed) = (temp.path().join("flag"), temp.path().join("handed"));
        let var = |name: &str| match name {
            INVOCATION_CWD_ENV_VAR => Some(handed.clone().into_os_string()),
            INVOCATION_DIR_ENV_VAR => Some(temp.path().as_os_str().to_owned()),
            _ => None,
        };
        let cwd = |words: &[&str]| invocation_cwd(&args(words), var).unwrap();
        assert_eq!(cwd(&["--cwd", flag.to_str().unwrap()]).as_std_path(), flag);
        assert_eq!(cwd(&["--skip-infer"]).as_std_path(), handed);
        let only_dir = |name: &str| {
            (name == INVOCATION_DIR_ENV_VAR)
                .then(|| var(name))
                .flatten()
        };
        let dir = invocation_cwd(&args(&["--skip-infer"]), only_dir).unwrap();
        assert_eq!(dir.as_std_path(), temp.path());
        // Without --skip-infer, inherited handoff variables are never trusted.
        assert_eq!(cwd(&[]), AbsoluteSystemPathBuf::cwd().unwrap());
    }
}

#[cfg(all(test, windows))]
#[allow(clippy::unwrap_used)]
mod windows_tests {
    use std::{
        fs,
        os::windows::process::CommandExt,
        path::{Path, PathBuf},
        process::Stdio,
        thread,
        time::{Duration, Instant},
    };

    use serde_json::{Value, json};
    use turborepo_run::EnvironmentVariableMap;
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

    const HOST: &str = "TURBO_EXEC_TEST_CTRL_C_HOST";
    const TIMEOUT: Duration = Duration::from_secs(30);

    /// Records how it started and exits 7. With `serve`, it and a descendant
    /// wait for Ctrl+C on the shared console.
    const PNPM: &str = r#"
const fs = require('fs');
if (process.argv[2] !== 'serve') {
  fs.writeFileSync('record.json', JSON.stringify({
    cwd: process.cwd(), args: process.argv.slice(2), runtime: process.env.MANAGED_RUNTIME,
  }));
  process.exit(7);
}
const child = require('child_process').spawn(process.execPath, ['-e',
  "process.on('SIGINT', () => process.exit(0)); setInterval(() => {}, 1000)"], { stdio: 'ignore' });
process.on('SIGINT', () => {
  fs.appendFileSync('interrupts', 'x');
  setTimeout(() => process.exit(6), 500);
});
setInterval(() => {}, 1000);
fs.writeFileSync('pids.json', JSON.stringify([process.pid, child.pid]));
"#;

    struct Tools(String);

    impl ManagedEnvironment for Tools {
        fn revalidate(&self) -> Result<EnvironmentVariableMap, String> {
            let mut map = EnvironmentVariableMap::default();
            map.insert("PATH".into(), self.0.clone());
            map.insert("MANAGED_RUNTIME".into(), "1".into());
            Ok(map)
        }

        fn cache_identity(&self) -> &str {
            "fixture"
        }
    }

    /// A generation laid out like Windows managed setup's: `bin\node.exe`, a
    /// single-binary `bin\uv.exe`, and a `bin\pnpm.cmd` launcher that runs
    /// `pnpm.cjs` with the generation's own Node. Host tools of the same
    /// names come later on PATH and must never run.
    struct Fixture {
        temp: tempfile::TempDir,
        tools: Tools,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let generation = temp.path().join("generation");
            let host = temp.path().join("host");
            for dir in [&generation.join("bin"), &host] {
                fs::create_dir_all(dir).unwrap();
            }
            fs::create_dir_all(generation.join(r"tools\node")).unwrap();
            fs::create_dir_all(generation.join(r"tools\pnpm\bin")).unwrap();
            let node = generation.join(r"tools\node\node.exe");
            let host_node = which::which("node").unwrap();
            fs::hard_link(&host_node, &node)
                .or_else(|_| fs::copy(&host_node, &node).map(drop))
                .unwrap();
            for name in ["node.exe", "uv.exe"] {
                fs::hard_link(&node, generation.join("bin").join(name)).unwrap();
            }
            fs::write(
                generation.join(r"bin\pnpm.cmd"),
                "@\"%~dp0..\\tools\\node\\node.exe\" \"%~dp0..\\tools\\pnpm\\bin\\pnpm.cjs\" \
                 %*\r\n",
            )
            .unwrap();
            fs::write(generation.join(r"tools\pnpm\bin\pnpm.cjs"), PNPM).unwrap();
            for name in ["node.cmd", "pnpm.cmd", "uv.cmd"] {
                fs::write(
                    host.join(name),
                    "@type nul > \"%~dp0ran\"\r\n@exit /b 97\r\n",
                )
                .unwrap();
            }
            let path = env::join_paths([generation.join("bin"), host])
                .unwrap()
                .into_string()
                .unwrap();
            Self {
                temp,
                tools: Tools(path),
            }
        }

        fn work(&self, name: &str) -> AbsoluteSystemPathBuf {
            let dir = self.temp.path().join(name);
            fs::create_dir(&dir).unwrap();
            AbsoluteSystemPathBuf::try_from(dir).unwrap()
        }

        fn script(&self) -> PathBuf {
            self.temp.path().join(r"generation\tools\pnpm\bin\pnpm.cjs")
        }

        fn host_ran(&self) -> bool {
            self.temp.path().join(r"host\ran").exists()
        }
    }

    fn plain(mut command: std::process::Command) -> io::Result<ExitStatus> {
        command.status()
    }

    fn words(command: &[&str]) -> Vec<OsString> {
        command.iter().map(OsString::from).collect()
    }

    #[test]
    fn managed_exe_and_cmd_tools_resolve_through_pathext_with_exact_arguments() {
        let fixture = Fixture::new();
        let work = fixture.work("work");
        let record = work.as_std_path().join("record.json");
        // Rust escapes `.cmd` arguments for cmd.exe; `%*` must pass them on.
        let args = [
            "a b",
            "",
            r#""quoted""#,
            "%PATH%",
            "&|<>^",
            "100%",
            "caf\u{e9}",
            r"trailing\",
            "--cwd",
        ];
        for program in ["pnpm", "PNPM.CMD"] {
            let command: Vec<_> = [program].iter().chain(&args).copied().collect();
            assert_eq!(
                spawn(&fixture.tools, &words(&command), &work, plain).unwrap(),
                7
            );
            let started: Value = serde_json::from_slice(&fs::read(&record).unwrap()).unwrap();
            assert_eq!(started["args"], json!(args), "{program}");
            assert_eq!(started["runtime"], "1");
            let cwd = started["cwd"].as_str().unwrap();
            assert!(cwd.eq_ignore_ascii_case(work.as_str()), "{cwd}");
        }
        let uv = "process.exit(require('path').basename(process.execPath) === 'uv.exe' ? 4 : 1)";
        assert_eq!(
            spawn(&fixture.tools, &words(&["uv", "-e", uv]), &work, plain).unwrap(),
            4
        );
        // cmd.exe would cut an argument at a line break, so nothing starts.
        fs::remove_file(&record).unwrap();
        assert!(matches!(
            spawn(&fixture.tools, &words(&["pnpm", "a\nb"]), &work, plain),
            Err(Error::Spawn { source, .. }) if source.kind() == io::ErrorKind::InvalidInput
        ));
        assert!(!record.exists());
        assert!(matches!(
            spawn(&fixture.tools, &words(&["missing"]), &work, plain),
            Err(Error::NotFound { program }) if program == "missing"
        ));
        assert!(!fixture.host_ran());
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
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn host_spec(spec: &str) -> (AbsoluteSystemPathBuf, Tools, Vec<OsString>) {
        let spec: Value = serde_json::from_str(spec).unwrap();
        let work = AbsoluteSystemPathBuf::new(spec["work"].as_str().unwrap()).unwrap();
        let tools = Tools(spec["path"].as_str().unwrap().to_owned());
        let command: Vec<String> = serde_json::from_value(spec["command"].clone()).unwrap();
        (
            work,
            tools,
            command.into_iter().map(OsString::from).collect(),
        )
    }

    /// Runs in a dedicated hidden console, where Ctrl+C reaches every attached
    /// process as a keypress would, but never the test runner's console.
    fn ctrl_c_host(spec: &str) {
        let (work, tools, command) = host_spec(spec);
        // GitHub's Windows runners start tests with Ctrl+C ignored, a state
        // every child inherits. Restore the interactive default.
        unsafe { SetConsoleCtrlHandler(None, 0) };
        let pids = work.as_std_path().join("pids.json");
        let exec = {
            let work = work.clone();
            thread::spawn(move || spawn(&tools, &command, &work, turborepo_process::hand_off))
        };
        let access = PROCESS_SYNCHRONIZE | PROCESS_TERMINATE;
        let tree: Vec<_> = wait_for(&pids)
            .into_iter()
            .map(|pid| unsafe { OpenProcess(access, 0, pid) })
            .collect();
        assert!(tree.iter().all(|handle| !handle.is_null()));
        assert_ne!(unsafe { GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0) }, 0);
        let status = exec.join().unwrap().unwrap();
        let mut survivors = 0;
        for handle in tree {
            unsafe {
                if WaitForSingleObject(handle, 10_000) != WAIT_OBJECT_0 {
                    survivors += 1;
                    TerminateProcess(handle, 1);
                }
                CloseHandle(handle);
            }
        }
        let result = json!({ "status": status, "survivors": survivors });
        fs::write(work.as_std_path().join("result.json"), result.to_string()).unwrap();
    }

    #[test]
    fn console_ctrl_c_reaches_managed_commands_once_and_exec_returns_their_status() {
        if let Ok(spec) = env::var(HOST) {
            return ctrl_c_host(&spec);
        }
        let fixture = Fixture::new();
        let script = fixture.script();
        let script = script.to_str().unwrap();
        for (name, command) in [
            ("exe", vec!["node", script, "serve"]),
            ("cmd", vec!["pnpm", "serve"]),
        ] {
            let work = fixture.work(name);
            let log = work.as_std_path().join("host.log");
            let spec =
                json!({ "work": work.as_str(), "path": fixture.tools.0, "command": command });
            let mut host = std::process::Command::new(env::current_exe().unwrap())
                .args([
                    "commands::exec::windows_tests::console_ctrl_c_reaches_managed_commands_once_and_exec_returns_their_status",
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
            let status = loop {
                if let Some(status) = host.try_wait().unwrap() {
                    break status;
                }
                if start.elapsed() > TIMEOUT {
                    let _ = host.kill();
                    panic!(
                        "{name}: exec did not return\n{}",
                        fs::read_to_string(&log).unwrap()
                    );
                }
                thread::sleep(Duration::from_millis(50));
            };
            let log = fs::read_to_string(&log).unwrap();
            assert!(status.success(), "{name}: {log}");
            let result = fs::read_to_string(work.as_std_path().join("result.json")).unwrap();
            let result: Value = serde_json::from_str(&result).unwrap();
            assert_eq!(
                result,
                json!({ "status": 6, "survivors": 0 }),
                "{name}: {log}"
            );
            let interrupts = work.as_std_path().join("interrupts");
            assert_eq!(fs::read_to_string(interrupts).unwrap(), "x", "{name}");
        }
        assert!(!fixture.host_ran());
    }

    const TREE_HOST: &str = "TURBO_EXEC_TEST_TREE_HOST";

    /// Starts a detached descendant at once, which nothing but a job object
    /// would take down with it. With `exit`, exits 3 once `release` exists.
    const TREE: &str = r#"
const fs = require('fs');
const child = require('child_process').spawn(process.execPath,
  ['-e', 'setInterval(() => {}, 1000)'], { stdio: 'ignore', detached: true });
fs.writeFileSync('pids.json', JSON.stringify([process.pid, child.pid]));
setInterval(() => process.argv[2] === 'exit' && fs.existsSync('release') && process.exit(3), 20);
"#;

    fn tree_host(spec: &str) {
        let (work, tools, command) = host_spec(spec);
        let status = spawn(&tools, &command, &work, turborepo_process::hand_off).unwrap();
        fs::write(work.as_std_path().join("status"), status.to_string()).unwrap();
    }

    /// TerminateProcess on exec (CI cancellation, an IDE's stop button) takes
    /// the command and all of its descendants with it. After the command
    /// exits on its own, a descendant it deliberately left running survives.
    #[test]
    fn terminating_exec_kills_the_command_tree_but_a_normal_exit_releases_it() {
        if let Ok(spec) = env::var(TREE_HOST) {
            return tree_host(&spec);
        }
        let fixture = Fixture::new();
        for terminate in [true, false] {
            let work = fixture.work(if terminate { "terminated" } else { "exited" });
            let script = work.as_std_path().join("tree.js");
            fs::write(&script, TREE).unwrap();
            let mode = if terminate { "run" } else { "exit" };
            let command = ["node", script.to_str().unwrap(), mode];
            let spec =
                json!({ "work": work.as_str(), "path": fixture.tools.0, "command": command });
            let mut host = std::process::Command::new(env::current_exe().unwrap())
                .args([
                    "commands::exec::windows_tests::terminating_exec_kills_the_command_tree_but_a_normal_exit_releases_it",
                    "--exact",
                ])
                .env(TREE_HOST, spec.to_string())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .spawn()
                .unwrap();
            let access = PROCESS_SYNCHRONIZE | PROCESS_TERMINATE;
            let tree: Vec<_> = wait_for(&work.as_std_path().join("pids.json"))
                .into_iter()
                .map(|pid| unsafe { OpenProcess(access, 0, pid) })
                .collect();
            assert!(tree.iter().all(|handle| !handle.is_null()));
            if terminate {
                host.kill().unwrap();
            } else {
                fs::write(work.as_std_path().join("release"), "").unwrap();
            }
            let start = Instant::now();
            let status = loop {
                if let Some(status) = host.try_wait().unwrap() {
                    break status;
                }
                if start.elapsed() > TIMEOUT {
                    let _ = host.kill();
                    panic!("exec did not return");
                }
                thread::sleep(Duration::from_millis(50));
            };
            let wait = if terminate { 10_000 } else { 500 };
            let alive: Vec<_> = tree
                .into_iter()
                .map(|handle| unsafe {
                    let alive = WaitForSingleObject(handle, wait) != WAIT_OBJECT_0;
                    TerminateProcess(handle, 1);
                    CloseHandle(handle);
                    alive
                })
                .collect();
            if terminate {
                assert_eq!(alive, [false, false], "the command tree outlived exec");
            } else {
                assert!(status.success(), "{status:?}");
                let code = fs::read_to_string(work.as_std_path().join("status")).unwrap();
                assert_eq!(code, "3");
                assert_eq!(alive, [false, true], "a normal exit must release the job");
            }
        }
        assert!(!fixture.host_ran());
    }
}
