mod package_manager;
mod paths;

use crate::package_manager::PackageManager;
use crate::paths::AncestorSearch;
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::Path;
use std::{
    env,
    ffi::CString,
    os::raw::{c_char, c_int},
    process,
};

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None, ignore_errors = true, disable_help_flag = true)]
struct Args {
    /// Current working directory
    #[clap(long, value_parser)]
    cwd: Option<String>,
    #[clap(subcommand)]
    commands: Option<Commands>,
    task: Option<String>,
}

/// Defines the subcommands for CLI. NOTE: If we change the commands in Go,
/// we must change these as well to avoid accidentally passing the --single-package
/// flag into non-build commands.
#[derive(Subcommand, Debug)]
enum Commands {
    Bin,
    Completion,
    Daemon,
    Help,
    Link,
    Login,
    Logout,
    Prune,
    Unlink,
    Run { tasks: Vec<String> },
}

extern "C" {
    pub fn nativeRunWithArgs(argc: c_int, argv: *mut *mut c_char) -> c_int;
}

/// Runs the turbo in the current binary
///
/// # Arguments
///
/// * `args`: Arguments for turbo
///
/// returns: Result<i32, Error>
///
fn run_current_turbo(args: Vec<String>) -> Result<i32> {
    let mut args = args
        .into_iter()
        .map(|s| {
            let c_string = CString::new(s)?;
            Ok(c_string.into_raw())
        })
        .collect::<Result<Vec<*mut c_char>>>()?;
    args.shrink_to_fit();
    let argc: c_int = args.len() as c_int;
    let argv = args.as_mut_ptr();
    let exit_code = unsafe { nativeRunWithArgs(argc, argv) };
    Ok(exit_code)
}

/// Checks if we are in "single package mode" by first seeing if there is a turbo.json
/// in the ancestor path, and then checking for workspaces.
///
/// # Arguments
///
/// * `current_dir`: Current working directory
///
/// returns: Result<bool, Error>
///
fn is_single_package_mode(current_dir: &Path) -> Result<bool> {
    let has_turbo_json = AncestorSearch::new(current_dir.to_path_buf(), "turbo.json")?
        .next()
        .is_some();

    if has_turbo_json {
        return Ok(false);
    }

    // We should detect which package manager and then determine workspaces from there,
    // but detection is not implemented yet and really we're either checking the `package.json`
    // or the `pnpm-workspace.yaml` file so we can do both.
    let npm = PackageManager::Npm;
    if npm.get_workspace_globs(current_dir).is_ok() {
        return Ok(false);
    };

    let pnpm = PackageManager::Pnpm;
    if pnpm.get_workspace_globs(current_dir).is_ok() {
        return Ok(false);
    };

    Ok(true)
}

/// Checks if either we have an explicit run command, i.e. `turbo run build`
/// or an implicit run, i.e. `turbo build`, where the command after `turbo` is
/// not one of the reserved commands like `link`, `login`, `bin`, etc.
///
/// # Arguments
///
/// * `clap_args`:
///
/// returns: bool
///
fn is_run_command(clap_args: &Args) -> bool {
    let is_explicit_run = matches!(clap_args.commands, Some(Commands::Run { .. }));
    let is_implicit_run = clap_args.commands.is_none() && clap_args.task.is_some();

    is_explicit_run || is_implicit_run
}

fn main() -> Result<()> {
    let clap_args = Args::parse();
    let current_dir = if let Some(cwd) = &clap_args.cwd {
        cwd.into()
    } else {
        env::current_dir()?
    };

    let mut args: Vec<_> = env::args().skip(1).collect();
    if is_single_package_mode(&current_dir)? && is_run_command(&clap_args) {
        args.push("--single-package".to_string());
    }

    let exit_code = match run_current_turbo(args) {
        Ok(exit_code) => exit_code,
        Err(e) => {
            println!("failed {:?}", e);
            2
        }
    };

    process::exit(exit_code)
}
