mod commands;
mod ffi;
mod package_manager;

use crate::ffi::nativeRunWithArgs;
use crate::package_manager::PackageManager;
use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use serde::Serialize;
use std::env::current_exe;
use std::path::{Path, PathBuf};

use std::process::Stdio;
use std::{
    env,
    ffi::CString,
    fs,
    os::raw::{c_char, c_int},
    process,
};

#[derive(Parser, Debug, Serialize)]
#[clap(author, version, about = "Turbocharge your monorepo", long_about = None, disable_help_subcommand = true)]
struct Args {
    /// Override the endpoint for API calls
    #[clap(long, value_parser)]
    api: Option<String>,
    /// Force color usage in the terminal
    #[clap(long, value_parser)]
    color: bool,
    /// Specify a file to save a cpu profile
    #[clap(long, value_parser)]
    cpuprofile: Option<String>,
    /// The directory in which to run turbo
    #[clap(long, value_parser)]
    cwd: Option<String>,
    /// Specify a file to save a pprof heap profile
    #[clap(long, value_parser)]
    heap: Option<String>,
    /// Override the login endpoint
    #[clap(long, value_parser)]
    login: Option<String>,
    /// Suppress color usage in the terminal
    #[clap(long, value_parser)]
    no_color: bool,
    /// When enabled, turbo will precede HTTP requests with an OPTIONS request for authorization
    #[clap(long, value_parser)]
    preflight: bool,
    /// Set the team slug for API calls
    #[clap(long, value_parser)]
    team: Option<String>,
    /// Set the auth token for API calls
    #[clap(long, value_parser)]
    token: Option<String>,
    /// Specify a file to save a pprof trace
    #[clap(long, value_parser)]
    trace: Option<String>,
    /// verbosity
    #[clap(short, long, value_parser)]
    verbosity: Option<u8>,
    #[clap(subcommand)]
    command: Option<Command>,
    task: Option<String>,
}

static TURBO_JSON: &str = "turbo.json";

/// Defines the subcommands for CLI. NOTE: If we change the commands in Go,
/// we must change these as well to avoid accidentally passing the --single-package
/// flag into non-build commands.
#[derive(Subcommand, Debug, Serialize)]
enum Command {
    /// Get the path to the Turbo binary
    Bin,
    /// Generate the autocompletion script for the specified shell
    Completion,
    /// Runs the Turborepo background daemon
    Daemon,
    /// Help about any command
    Help,
    /// Link your local directory to a Vercel organization and enable remote caching.
    Link,
    /// Login to your Vercel account
    Login,
    /// Logout to your Vercel account
    Logout,
    /// Prepare a subset of your monorepo.
    Prune,
    /// Run tasks across projects in your monorepo
    Run { tasks: Vec<String> },
    /// Unlink the current directory from your Vercel organization and disable Remote Caching
    Unlink,
}

#[derive(Debug, Clone, Serialize)]
struct RepoState {
    root: PathBuf,
    mode: RepoMode,
}

#[derive(Debug, Clone, Serialize)]
enum RepoMode {
    SinglePackage,
    MultiPackage,
}

/// The entire state of the execution, including args, repo state, etc.
#[derive(Debug, Serialize)]
struct TurboState {
    repo_state: RepoState,
    cli_args: Args,
}

/// Runs the Go code linked in current binary.
///
/// # Arguments
///
/// * `args`: Arguments for turbo
///
/// returns: Result<i32, Error>
///
fn run_current_turbo(clap_args: Args, args: Vec<String>) -> Result<i32> {
    if let Some(Command::Bin) = clap_args.command {
        commands::bin::run()?;
        return Ok(0);
    }

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
    Ok(exit_code.try_into().unwrap())
}

impl RepoState {
    /// Infers `RepoState` from current directory.
    ///
    /// # Arguments
    ///
    /// * `current_dir`: Current working directory
    ///
    /// returns: Result<RepoState, Error>
    ///
    pub fn infer(current_dir: &Path) -> Result<Self> {
        // First we look for a `turbo.json`. This iterator returns the first ancestor that contains
        // a `turbo.json` file.
        let root_path = current_dir
            .ancestors()
            .find(|p| fs::metadata(p.join(TURBO_JSON)).is_ok());

        // If that directory exists, then we figure out if there are workspaces defined in it
        // NOTE: This may change with multiple `turbo.json` files
        if let Some(root_path) = root_path {
            let pnpm = PackageManager::Pnpm;
            let npm = PackageManager::Npm;
            let is_workspace = pnpm.get_workspace_globs(root_path).is_ok()
                || npm.get_workspace_globs(root_path).is_ok();

            let mode = if is_workspace {
                RepoMode::MultiPackage
            } else {
                RepoMode::SinglePackage
            };

            return Ok(Self {
                root: root_path.to_path_buf(),
                mode,
            });
        }

        // What we look for next is a directory that contains a `package.json`.
        let potential_roots = current_dir
            .ancestors()
            .filter(|path| fs::metadata(path.join("package.json")).is_ok());

        let mut first_package_json_dir = None;
        // We loop through these directories and see if there are workspaces defined in them,
        // either in the `package.json` or `pnm-workspaces.yml`
        for dir in potential_roots {
            if first_package_json_dir.is_none() {
                first_package_json_dir = Some(dir)
            }

            let pnpm = PackageManager::Pnpm;
            let npm = PackageManager::Npm;
            let is_workspace =
                pnpm.get_workspace_globs(dir).is_ok() || npm.get_workspace_globs(dir).is_ok();

            if is_workspace {
                return Ok(Self {
                    root: dir.to_path_buf(),
                    mode: RepoMode::MultiPackage,
                });
            }
        }

        // Finally, if we don't detect any workspaces, go to the first `package.json`
        // and use that in single package mode.
        let root = first_package_json_dir
            .ok_or_else(|| {
                anyhow!(
                    "Unable to find `{}` or `package.json` in current path",
                    TURBO_JSON
                )
            })?
            .to_path_buf();

        Ok(Self {
            root,
            mode: RepoMode::SinglePackage,
        })
    }
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
    let is_explicit_run = matches!(clap_args.command, Some(Command::Run { .. }));
    let is_implicit_run = clap_args.command.is_none() && clap_args.task.is_some();

    is_explicit_run || is_implicit_run
}

/// Attempts to run correct turbo by finding nearest package.json,
/// then finding local turbo installation. If the current binary is the local
/// turbo installation, then we run current turbo. Otherwise we kick over to
/// the local turbo installation.
///
/// # Arguments
///
/// * `turbo_state`: state for current execution
///
/// returns: Result<i32, Error>
///
fn run_correct_turbo(turbo_state: TurboState) -> Result<i32> {
    let local_turbo_path = turbo_state
        .repo_state
        .root
        .join("node_modules")
        .join(".bin")
        .join("turbo");

    let mut args: Vec<_> = env::args().skip(1).collect();
    if matches!(turbo_state.repo_state.mode, RepoMode::SinglePackage)
        && is_run_command(&turbo_state.cli_args)
    {
        args.push("--single-package".to_string());
    }

    let current_turbo_is_local_turbo = local_turbo_path == current_exe()?;
    // If the local turbo path doesn't exist or if we are local turbo, then we go ahead and run
    if !local_turbo_path.try_exists()? || current_turbo_is_local_turbo {
        return run_current_turbo(turbo_state.cli_args, args);
    }

    let mut command = process::Command::new(local_turbo_path)
        .args(&args)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("Failed to execute turbo.");

    Ok(command.wait()?.code().unwrap_or(2))
}

fn main() -> Result<()> {
    let clap_args = Args::parse();
    let current_dir = if let Some(cwd) = &clap_args.cwd {
        fs::canonicalize::<PathBuf>(cwd.into())?
    } else {
        env::current_dir()?
    };

    if clap_args.command.is_none() && clap_args.task.is_none() {
        process::exit(1);
    }

    let repo_state = RepoState::infer(&current_dir)?;
    let turbo_state = TurboState {
        repo_state,
        cli_args: clap_args,
    };

    let exit_code = match run_correct_turbo(turbo_state) {
        Ok(exit_code) => exit_code,
        Err(e) => {
            eprintln!("failed {:?}", e);
            2
        }
    };

    process::exit(exit_code)
}
