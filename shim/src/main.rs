mod package_manager;

use crate::package_manager::PackageManager;
use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use serde::Deserialize;
use std::collections::HashMap;
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

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None, ignore_errors = true, disable_help_flag = true, disable_help_subcommand = true)]
struct Args {
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

/// Finds local turbo path given the package.json path. We assume that the node_modules directory
/// is at the same level as the package.json file.
///
/// # Arguments
///
/// * `package_json_path`: The location of the package.json file
///
/// returns: Result<Option<PathBuf>, Error>
///
fn find_local_turbo_path(repo_root: &Path) -> Result<Option<PathBuf>> {
    let package_json_path = repo_root.join("package.json");
    let package_json_contents = fs::read_to_string(&package_json_path)?;
    let package_json: PackageJson = serde_json::from_str(&package_json_contents)?;

    let dev_dependencies_has_turbo = package_json
        .dev_dependencies
        .map_or(false, |deps| deps.contains_key("turbo"));
    let dependencies_has_turbo = package_json
        .dependencies
        .map_or(false, |deps| deps.contains_key("turbo"));

    if dev_dependencies_has_turbo || dependencies_has_turbo {
        let mut local_turbo_path = repo_root.join("node_modules");
        local_turbo_path.push(".bin");
        local_turbo_path.push("turbo");

        fs::metadata(&local_turbo_path).map_err(|_| {
            anyhow!(
                "Could not find binary in {}.",
                local_turbo_path.to_string_lossy()
            )
        })?;

        Ok(Some(local_turbo_path))
    } else {
        Ok(None)
    }
}

impl RepoState {
    /// Infers `RepoState` from current directory. Can either be `RepoState::MultiPackage` or `RepoState::SinglePackage`.
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
            .find(|p| fs::metadata(p.join("turbo.json")).is_ok());

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

        // What we look for next is either a `package.json` file or a `pnpm-workspace.yaml` file.
        let potential_roots = current_dir
            .ancestors()
            .filter(|path| fs::metadata(path.join("package.json")).is_ok());

        let mut first_package_json_dir = None;
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
                    mode: RepoMode::SinglePackage,
                });
            }
        }

        // Finally, if we don't detect any workspaces, we simply go to the first `package.json`
        // and use that in single package mode.
        let root = first_package_json_dir
            .ok_or_else(|| {
                anyhow!("Unable to find `turbo.json` or `package.json` in current path")
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
    let local_turbo_path = find_local_turbo_path(repo_root)?
        .ok_or_else(|| anyhow!("No local turbo installation found in package.json."))?;

    if !local_turbo_path.try_exists()? {
        return Err(anyhow!(
            "No local turbo installation found in node_modules."
        ));
    }

    if local_turbo_path == current_exe()? {
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

#[derive(Debug, Deserialize)]
struct PackageJson {
    dependencies: Option<HashMap<String, String>>,
    #[serde(rename = "devDependencies")]
    dev_dependencies: Option<HashMap<String, String>>,
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

    let mut args: Vec<_> = env::args().skip(1).collect();
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
