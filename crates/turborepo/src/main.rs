// Bump all rust changes
#![deny(clippy::all)]

use std::{ffi::OsStr, future::Future, pin::Pin, process, sync::Arc};

use anyhow::Result;
use miette::Report;

const INTERNAL_LSP_COMMAND: &str = "__internal_lsp";
#[cfg(windows)]
const INTERNAL_WINDOWS_CTRL_C_COMMAND: &str = "__internal_windows_ctrl_c";

#[cfg(feature = "heap-dhat")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

// glibc's malloc loses 10-15% of total CPU to allocator overhead on
// allocation-heavy phases (lockfile parsing, package graph construction,
// task dispatch). mimalloc reclaims most of that. Windows is excluded
// because of a CRT conflict with libghostty-vt-sys (see Cargo.toml).
#[cfg(all(not(feature = "heap-dhat"), not(target_os = "windows")))]
#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Debug, PartialEq)]
enum InternalLspCommand {
    Probe,
    Server,
}

#[cfg(windows)]
enum InternalWindowsConsoleCommand {
    CtrlC(u32),
}

/// Concrete [`turborepo_query_api::QueryServer`] that delegates to
/// `turborepo_query`.
///
/// Lives in the binary crate because it's the only place that depends on both
/// `turborepo-lib` and `turborepo-query`, enabling the dependency inversion
/// that allows them to compile in parallel.
struct TurboQueryServer;

impl turborepo_query_api::QueryServer for TurboQueryServer {
    fn execute_query<'a>(
        &'a self,
        run: Arc<dyn turborepo_query_api::QueryRun>,
        query: &'a str,
        variables_json: Option<&'a str>,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<turborepo_query_api::QueryResult, turborepo_query_api::Error>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            turborepo_query::execute_query(run, query, variables_json)
                .await
                .map_err(Into::into)
        })
    }

    fn run_query_server(
        &self,
        run: Arc<dyn turborepo_query_api::QueryRun>,
        signal: turborepo_signals::SignalHandler,
    ) -> Pin<Box<dyn Future<Output = Result<(), turborepo_query_api::Error>> + Send + '_>> {
        Box::pin(async move {
            turborepo_query::run_query_server(run, signal)
                .await
                .map_err(Into::into)
        })
    }
}

// This function should not expanded. Please add any logic to
// `turborepo_lib::main` instead
fn main() -> Result<()> {
    #[cfg(windows)]
    if let Some(command) = internal_windows_ctrl_c_command(std::env::args_os()) {
        let exit_code = match command {
            InternalWindowsConsoleCommand::CtrlC(pid) => send_windows_ctrl_c(pid),
        };
        process::exit(exit_code);
    }

    if let Some(command) = internal_lsp_command(std::env::args_os()) {
        if command == InternalLspCommand::Probe {
            println!("turbo-lsp");
            return Ok(());
        }

        turborepo_lsp::run_lsp_server();
        return Ok(());
    }

    std::panic::set_hook(Box::new(turborepo_lib::panic_handler));

    let query_server: Arc<dyn turborepo_lib::QueryServer> = Arc::new(TurboQueryServer);
    let exit_code = turborepo_lib::main(Some(query_server)).unwrap_or_else(|err| {
        eprintln!("{:?}", Report::new(err));
        1
    });

    turborepo_lib::finish_heap_profile();
    process::exit(exit_code)
}

#[cfg(windows)]
fn attach_to_windows_console(pid: u32) -> bool {
    use windows_sys::Win32::System::Console::{AttachConsole, FreeConsole};

    unsafe { FreeConsole() };
    (unsafe { AttachConsole(pid) }) != 0
}

#[cfg(windows)]
fn send_windows_ctrl_c(pid: u32) -> i32 {
    use windows_sys::Win32::{
        Foundation::TRUE,
        System::Console::{
            CTRL_C_EVENT, FreeConsole, GenerateConsoleCtrlEvent, SetConsoleCtrlHandler,
        },
    };

    if !attach_to_windows_console(pid) {
        return 1;
    }

    unsafe {
        SetConsoleCtrlHandler(None, TRUE);
    }
    let success = unsafe { GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0) } != 0;
    std::thread::sleep(std::time::Duration::from_millis(100));
    unsafe { FreeConsole() };

    if success { 0 } else { 1 }
}

#[cfg(windows)]
fn internal_windows_ctrl_c_command<T>(
    args: impl IntoIterator<Item = T>,
) -> Option<InternalWindowsConsoleCommand>
where
    T: AsRef<OsStr>,
{
    let mut args = args.into_iter().skip(1);
    let first_arg = args.next()?;
    let command_arg = if first_arg.as_ref() == OsStr::new("--skip-infer") {
        args.next()?
    } else {
        first_arg
    };

    if command_arg.as_ref() != OsStr::new(INTERNAL_WINDOWS_CTRL_C_COMMAND) {
        return None;
    }

    let subcommand_or_pid = args.next()?;
    if subcommand_or_pid.as_ref() == OsStr::new("ctrl_c") {
        let pid = args.next()?.as_ref().to_str()?.parse().ok()?;
        Some(InternalWindowsConsoleCommand::CtrlC(pid))
    } else {
        let pid = subcommand_or_pid.as_ref().to_str()?.parse().ok()?;
        Some(InternalWindowsConsoleCommand::CtrlC(pid))
    }
}

fn internal_lsp_command<T>(args: impl IntoIterator<Item = T>) -> Option<InternalLspCommand>
where
    T: AsRef<OsStr>,
{
    let mut args = args.into_iter().skip(1);
    let first_arg = args.next()?;
    let command_arg = if first_arg.as_ref() == OsStr::new("--skip-infer") {
        args.next()?
    } else {
        first_arg
    };

    if command_arg.as_ref() != OsStr::new(INTERNAL_LSP_COMMAND) {
        return None;
    }

    if args
        .next()
        .is_some_and(|arg| arg.as_ref() == OsStr::new("--probe"))
    {
        Some(InternalLspCommand::Probe)
    } else {
        Some(InternalLspCommand::Server)
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{InternalLspCommand, internal_lsp_command};

    fn args(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn detects_internal_lsp_probe() {
        assert_eq!(
            internal_lsp_command(args(&["turbo", "__internal_lsp", "--probe"])),
            Some(InternalLspCommand::Probe)
        );
    }

    #[test]
    fn detects_shimmed_internal_lsp_probe() {
        assert_eq!(
            internal_lsp_command(args(&[
                "turbo",
                "--skip-infer",
                "__internal_lsp",
                "--probe",
                "--",
            ])),
            Some(InternalLspCommand::Probe)
        );
    }

    #[test]
    fn detects_internal_lsp_server() {
        assert_eq!(
            internal_lsp_command(args(&["turbo", "--skip-infer", "__internal_lsp", "--"])),
            Some(InternalLspCommand::Server)
        );
    }

    #[test]
    fn ignores_regular_turbo_command() {
        assert_eq!(internal_lsp_command(args(&["turbo", "run", "build"])), None);
    }

    #[cfg(unix)]
    #[test]
    fn ignores_non_utf8_arguments() {
        use std::os::unix::ffi::OsStringExt;

        assert_eq!(
            internal_lsp_command(vec![
                OsString::from("turbo"),
                OsString::from_vec(b"run-\xFF".to_vec()),
            ]),
            None
        );
    }
}
