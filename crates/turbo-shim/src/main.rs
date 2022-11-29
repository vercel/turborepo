use std::{
    env,
    env::{current_dir, current_exe},
    path::PathBuf,
    process,
    process::Stdio,
};

use anyhow::{anyhow, Result};
use repo_state::{RepoMode, RepoState, MINIMUM_SUPPORTED_LOCAL_TURBO};
use semver::Version;
use turborepo::get_version;

mod package_manager;
mod repo_state;

static TURBO_JSON: &str = "turbo.json";

#[derive(Debug)]
struct Args {
    cwd: PathBuf,
    remaining_args: Vec<String>,
}
impl Args {
    pub fn parse() -> Result<Self> {
        let mut found_cwd_flag = false;
        let mut cwd: Option<PathBuf> = None;
        let mut remaining_args = Vec::new();
        let mut is_forwarded_args = false;
        let args = env::args().skip(1);
        for arg in args {
            // We've seen a `--` and therefore we do no parsing
            if is_forwarded_args {
                remaining_args.push(arg);
            } else if arg == "--" {
                // If we've hit `--` we've reached the args forwarded to tasks.
                remaining_args.push(arg);
                is_forwarded_args = true;
            } else if found_cwd_flag {
                // We've seen a `--cwd` and therefore set the cwd to this arg.
                // NOTE: We purposefully allow multiple --cwd flags and only use
                // the last one, as this is the Go parser's behavior.
                cwd = Some(arg.into());
                found_cwd_flag = false;
            } else if arg == "--cwd" {
                // If we see a `--cwd` we expect the next arg to be a path.
                found_cwd_flag = true
            } else {
                remaining_args.push(arg);
            }
        }

        if found_cwd_flag {
            Err(anyhow!("No value assigned to `--cwd` argument"))
        } else {
            let cwd = if let Some(cwd) = cwd {
                cwd
            } else {
                current_dir()?
            };

            Ok(Args {
                cwd,
                remaining_args,
            })
        }
    }

    /// Attempts to run correct turbo by finding nearest package.json,
    /// then finding local turbo installation. If the current binary is the
    /// local turbo installation, then we run current turbo. Otherwise we
    /// kick over to the local turbo installation.
    ///
    /// # Arguments
    ///
    /// * `turbo_state`: state for current execution
    ///
    /// returns: Result<i32, Error>
    fn run_correct_turbo(mut self) -> Result<i32> {
        let repo_state = RepoState::infer(&self.cwd)?;

        if let Some(local_turbo_version) = repo_state.infer_local_turbo_version()? {
            let minimum_supported_turbo_version = Version::parse(MINIMUM_SUPPORTED_LOCAL_TURBO)?;
            if local_turbo_version < minimum_supported_turbo_version {
                return Err(anyhow!(
                    "Your local turbo installation is too old. Please update it to at least {}.",
                    minimum_supported_turbo_version
                ));
            }

            let current_turbo_version: Version = get_version().parse()?;
            if local_turbo_version > current_turbo_version {
                return Err(anyhow!(
                    "Your local turbo installation ({}) is newer than your global turbo \
                     installation ({}). Please update your global turbo installation.",
                    local_turbo_version,
                    current_turbo_version
                ));
            }
        }

        let local_turbo_path = repo_state
            .root
            .join("../../../node_modules")
            .join(".bin")
            .join({
                #[cfg(windows)]
                {
                    "turbo.cmd"
                }
                #[cfg(not(windows))]
                {
                    "turbo"
                }
            });

        if matches!(repo_state.mode, RepoMode::SinglePackage) {
            self.remaining_args.push("--single-package".to_string());
        }
        let current_turbo_is_local_turbo = local_turbo_path == current_exe()?;
        // If the local turbo path doesn't exist or if we are local turbo, then we go
        // ahead and run the Go code linked in the current binary.
        if current_turbo_is_local_turbo || !local_turbo_path.try_exists()? {
            return turborepo::run();
        }

        // We add back the --cwd but canonicalized. It's added at the front to avoid
        // accidentally putting it in the arguments forwarded to tasks.
        let canonicalized_cwd = self.cwd.canonicalize()?;
        let mut raw_args = vec![
            "--cwd".to_string(),
            canonicalized_cwd
                .to_str()
                .ok_or_else(|| anyhow!("--cwd is invalid Unicode. Please rename path"))?
                .to_string(),
        ];

        raw_args.append(&mut self.remaining_args);
        // Otherwise, we spawn a process that executes the local turbo
        // that we've found in node_modules/.bin/turbo.
        let mut command = process::Command::new(local_turbo_path)
            .args(&raw_args)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("Failed to execute turbo.");

        Ok(command.wait()?.code().unwrap_or(2))
    }
}
fn main() -> Result<()> {
    let args = Args::parse()?;
    let exit_code = args.run_correct_turbo()?;
    process::exit(exit_code);
}
