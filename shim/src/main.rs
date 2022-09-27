use anyhow::Result;
use clap::Parser;
use serde::Deserialize;
use std::collections::HashMap;
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
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Current working directory
    #[clap(long, value_parser)]
    cwd: Option<String>,
}

extern "C" {
    pub fn nativeRunWithArgs(argc: c_int, argv: *mut *mut c_char) -> c_int;
}

fn run_turbo(args: Vec<String>) -> Result<i32> {
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

/// Starts at `current_dir` and searches up the directory tree for the specified `config_file`.
///
/// # Arguments
///
/// * `current_dir`: Current directory where we start search
/// * `config_file`: Name of config file that we are searching for
///
/// returns: Result<PathBuf, Error>
///
fn find_config_file_in_ancestor_path(
    mut current_dir: PathBuf,
    config_file: impl AsRef<Path>,
) -> Option<PathBuf> {
    while fs::metadata(current_dir.join(&config_file)).is_err() {
        // Pops off current folder and sets to `current_dir.parent`
        // if false, `current_dir` has no parent
        if !current_dir.pop() {
            return None;
        }
    }

    Some(current_dir.join(config_file))
}

/// Finds local turbo path given the package.json path
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
            .expect("Unexpected file system error occurred")
            .join("node_modules");
        local_turbo_path.push(".bin");
        local_turbo_path.push("turbo");

        Ok(Some(local_turbo_path))
    } else {
        Ok(None)
    }
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
    let args = std::env::args().skip(1).collect::<Vec<_>>();

    let turbo_path = if let Some(package_json_path) =
        find_config_file_in_ancestor_path(current_dir, "package.json")
    {
        find_local_turbo_path(&package_json_path)?
    } else {
        None
    };

    let exit_code = if let Some(turbo_path) = turbo_path {
        let output = Command::new(turbo_path)
            .args(&args)
            .output()
            .expect("Failed to execute turbo");
        io::stdout().write_all(&output.stdout).unwrap();
        io::stderr().write_all(&output.stderr).unwrap();

        output.status.code().unwrap_or(2)
    } else {
        match run_turbo(args) {
            Ok(exit_code) => exit_code,
            Err(e) => {
                println!("failed {:?}", e);
                2
            }
        }
    };

    process::exit(exit_code);
}
