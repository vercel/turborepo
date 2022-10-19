mod package_manager;

use crate::package_manager::PackageManager;
use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
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

static TURBO_JSON: &str = "turbo.json";

#[derive(Parser, Debug)]
#[clap(author, about, long_about = None, ignore_errors = true, disable_help_flag = true, disable_help_subcommand = true, disable_version_flag = true)]
struct Args {
    #[clap(long, global = true)]
    version: bool,
    #[clap(long, short, global = true)]
    help: bool,
    /// Current working directory
    #[clap(long, value_parser)]
    cwd: Option<String>,
    #[clap(subcommand)]
    command: Option<Command>,
    task: Option<String>,
}

/// Defines the subcommands for CLI. NOTE: If we change the commands in Go,
/// we must change these as well to avoid accidentally passing the --single-package
/// flag into non-build commands.
#[derive(Subcommand, Debug)]
enum Command {
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

#[derive(Debug)]
struct RepoState {
    root: PathBuf,
    mode: RepoMode,
}

#[derive(Debug)]
enum RepoMode {
    SinglePackage,
    MultiPackage,
}

extern "C" {
    pub fn nativeRunWithArgs(argc: c_int, argv: *mut *mut c_char) -> c_int;
}

/// Runs the Go code linked in current binary.
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
/// * `current_dir`: Current working directory as defined by the --cwd flag
///
/// returns: Result<i32, Error>
///
fn run_correct_turbo(repo_root: &Path, args: Vec<String>) -> Result<i32> {
    let local_turbo_path = repo_root.join("node_modules").join(".bin").join("turbo");

    let current_turbo_is_local_turbo = local_turbo_path == current_exe()?;
    // If the local turbo path doesn't exist or if we are local turbo, then we go ahead and run
    if !local_turbo_path.try_exists()? || current_turbo_is_local_turbo {
        return run_current_turbo(args);
    }

    let mut command = process::Command::new(local_turbo_path)
        .args(&args)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("Failed to execute turbo.");

    Ok(command.wait()?.code().unwrap_or(2))
}

fn get_version() -> &'static str {
    include_str!("../../version.txt")
        .split_once('\n')
        .expect("Failed to read version from version.txt")
        .0
}

fn main() -> Result<()> {
    let clap_args = Args::parse();
    // --version flag doesn't work with ignore_errors in clap, so we have to handle it manually
    if clap_args.version {
        println!("{}", get_version());
        process::exit(0);
    }

    let mut args: Vec<_> = env::args().skip(1).collect();
    // Quick fix for --help.
    if clap_args.help {
        let exit_code = run_current_turbo(args)?;
        process::exit(exit_code);
    }

    let current_dir = if let Some(cwd) = &clap_args.cwd {
        fs::canonicalize::<PathBuf>(cwd.into())?
    } else {
        env::current_dir()?
    };

    if args.is_empty() {
        process::exit(1);
    }
    let repo_state = RepoState::infer(&current_dir)?;

    if matches!(repo_state.mode, RepoMode::SinglePackage) && is_run_command(&clap_args) {
        args.push("--single-package".to_string());
    }

    let exit_code = match run_correct_turbo(&repo_state.root, args) {
        Ok(exit_code) => exit_code,
        Err(e) => {
            eprintln!("failed {:?}", e);
            2
        }
    };

    process::exit(exit_code)
}
