mod package_manager;
mod paths;

use crate::package_manager::PackageManager;
use crate::paths::AncestorSearch;
use anyhow::{anyhow, Result};
use clap::Parser;
use serde::Deserialize;
use std::collections::HashMap;
use std::env::current_exe;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{
    env,
    ffi::CString,
    fs, io,
    os::raw::{c_char, c_int},
    process,
};

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None, ignore_errors = true, disable_help_flag = true)]
struct Args {
    /// Current working directory
    #[clap(long, value_parser)]
    cwd: Option<String>,
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
fn find_local_turbo_path(package_json_path: &Path) -> Result<Option<PathBuf>> {
    let package_json_contents = fs::read_to_string(&package_json_path)?;
    let package_json: PackageJson = serde_json::from_str(&package_json_contents)?;

    let dev_dependencies_has_turbo = package_json
        .dev_dependencies
        .map_or(false, |deps| deps.contains_key("turbo"));
    let dependencies_has_turbo = package_json
        .dependencies
        .map_or(false, |deps| deps.contains_key("turbo"));

    if dev_dependencies_has_turbo || dependencies_has_turbo {
        let mut local_turbo_path = package_json_path
            .parent()
            .ok_or_else(|| anyhow!("An unexpected file system error occurred"))?
            .join("node_modules");
        local_turbo_path.push(".bin");
        local_turbo_path.push("turbo");

        Ok(Some(local_turbo_path))
    } else {
        Ok(None)
    }
}

/// Checks if we are in single package mode by first seeing if there is a turbo.json
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

/// Attempts to run local turbo by finding nearest package.json,
/// then finding local turbo installation, then running installation if exists.
/// If at any point this fails, return an error and let main run global turbo.
/// If successful, return the exit code of local turbo.
///
/// # Arguments
///
/// * `current_dir`: Current working directory as defined by the --cwd flag
///
/// returns: Result<i32, Error>
///
fn try_run_local_turbo(current_dir: PathBuf) -> Result<i32> {
    let package_json_path = AncestorSearch::new(current_dir, "package.json")?
        .next()
        .ok_or_else(|| anyhow!("No package.json found in ancestor path."))?;
    let local_turbo_path = find_local_turbo_path(&package_json_path)?
        .ok_or_else(|| anyhow!("No local turbo installation found in package.json."))?;

    let args = env::args().skip(1).collect::<Vec<_>>();
    if !local_turbo_path.try_exists()? {
        return Err(anyhow!(
            "No local turbo installation found in node_modules."
        ));
    }

    if local_turbo_path == current_exe()? {
        return Err(anyhow!(
            "Local turbo is current turbo. Running current turbo."
        ));
    }

    let output = Command::new(local_turbo_path)
        .args(&args)
        .output()
        .expect("Failed to execute turbo.");

    io::stdout().write_all(&output.stdout).unwrap();
    io::stderr().write_all(&output.stderr).unwrap();

    Ok(output.status.code().unwrap_or(2))
}

#[derive(Debug, Deserialize)]
struct PackageJson {
    dependencies: Option<HashMap<String, String>>,
    #[serde(rename = "devDependencies")]
    dev_dependencies: Option<HashMap<String, String>>,
}

fn main() -> Result<()> {
    let clap_args = Args::parse();

    let current_dir = if let Some(cwd) = clap_args.cwd {
        cwd.into()
    } else {
        env::current_dir()?
    };

    let mut args: Vec<_> = env::args().skip(1).collect();
    if is_single_package_mode(&current_dir)? {
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
