use std::{
    env,
    path::{Path, PathBuf},
    process,
    process::Stdio,
    time::Duration,
};

use anyhow::{anyhow, Result};
use const_format::formatcp;
use dunce::canonicalize as fs_canonicalize;
use log::debug;
use semver::Version;
use serde::{Deserialize, Serialize};
use tiny_gradient::{GradientStr, RGB};
use turbo_updater::check_for_updates;

use super::{local_turbo_state::LocalTurboState, repo_state::RepoState};
use crate::{
    cli,
    files::turbo_json,
    get_version,
    shim::{init_env_logger, ShimArgs},
    state::repo_state::RepoMode,
    Payload,
};

fn turbo_version_has_shim(version: &str) -> bool {
    let version = Version::parse(version).unwrap();
    // only need to check major and minor (this will include canaries)
    if version.major == 1 {
        return version.minor >= 7;
    }

    version.major > 1
}

/// Checks for `TURBO_BINARY_PATH` variable. If it is set,
/// we do not try to find local turbo, we simply run the command as
/// the current binary. This is due to legacy behavior of `TURBO_BINARY_PATH`
/// that lets users dynamically set the path of the turbo binary. Because
/// that conflicts with finding a local turbo installation and
/// executing that binary, these two features are fundamentally incompatible.
fn is_turbo_binary_path_set() -> bool {
    env::var("TURBO_BINARY_PATH").is_ok()
}

fn try_check_for_updates(args: &ShimArgs, current_version: &str) {
    if args.should_check_for_update() {
        // custom footer for update message
        let footer = format!(
            "Follow {username} for updates: {url}",
            username = "@turborepo".gradient([RGB::new(0, 153, 247), RGB::new(241, 23, 18)]),
            url = "https://twitter.com/turborepo"
        );

        let interval = if args.force_update_check {
            // force update check
            Some(Duration::ZERO)
        } else {
            // use default (24 hours)
            None
        };
        // check for updates
        let _ = check_for_updates(
            "turbo",
            "https://github.com/vercel/turbo",
            Some(&footer),
            current_version,
            // use default for timeout (800ms)
            None,
            interval,
        );
    }
}

/**
 * TurboState is used for calculating information about the
 * currently-running executable. It does not specify whether it is local or
 * global, it just reports information about itself. Depending upon the
 * environment and how it was invoked it _may_ go look for a different
 * executable to delegate to.
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurboState {
    bin_path: Option<PathBuf>,
    version: &'static str,
    repo_state: Option<RepoState>,
}

impl Default for TurboState {
    fn default() -> Self {
        Self {
            bin_path: env::current_exe().ok(),
            version: get_version(),
            repo_state: None,
        }
    }
}

impl TurboState {
    pub fn platform_package_name() -> &'static str {
        const ARCH: &str = {
            #[cfg(target_arch = "x86_64")]
            {
                "64"
            }
            #[cfg(target_arch = "aarch64")]
            {
                "arm64"
            }
            #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
            {
                "unknown"
            }
        };

        const OS: &str = {
            #[cfg(target_os = "macos")]
            {
                "darwin"
            }
            #[cfg(target_os = "windows")]
            {
                "windows"
            }
            #[cfg(target_os = "linux")]
            {
                "linux"
            }
            #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
            {
                "unknown"
            }
        };

        formatcp!("turbo-{}-{}", OS, ARCH)
    }

    pub fn binary_name() -> &'static str {
        {
            #[cfg(windows)]
            {
                "turbo.exe"
            }
            #[cfg(not(windows))]
            {
                "turbo"
            }
        }
    }

    #[allow(dead_code)]
    pub fn version() -> &'static str {
        include_str!("../../../../version.txt")
            .lines()
            .next()
            .expect("Failed to read version from version.txt")
    }

    pub fn run(&mut self) -> Result<Payload> {
        let args = ShimArgs::parse()?;

        init_env_logger(args.verbosity);
        debug!("Global turbo version: {}", get_version());

        // If skip_infer is passed, we're probably running local turbo with
        // global turbo having handled the inference. We can run without any
        // concerns.
        if args.skip_infer {
            return cli::run(None);
        }

        // If the TURBO_BINARY_PATH is set, we do inference but we do not use
        // it to execute local turbo. We simply use it to set the `--single-package`
        // and `--cwd` flags.
        if is_turbo_binary_path_set() {
            let repo_state = RepoState::infer(&args.cwd)?;
            debug!("Repository Root: {}", repo_state.root.to_string_lossy());
            return cli::run(Some(repo_state));
        }

        match RepoState::infer(&args.cwd) {
            Ok(repo_state) => {
                self.repo_state = Some(repo_state);
                debug!(
                    "Repository Root: {}",
                    self.repo_state.clone().unwrap().root.to_string_lossy()
                );
                self.run_correct_turbo(args)
            }
            Err(err) => {
                // If we cannot infer, we still run global turbo. This allows for global
                // commands like login/logout/link/unlink to still work
                debug!("Repository inference failed: {}", err);
                debug!("Running command as global turbo");
                cli::run(None)
            }
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
    fn run_correct_turbo(&self, shim_args: ShimArgs) -> Result<Payload> {
        if let Some(LocalTurboState { bin_path, version }) =
            self.repo_state.clone().unwrap().local_turbo_state
        {
            try_check_for_updates(&shim_args, &version);
            let canonical_local_turbo = fs_canonicalize(bin_path)?;
            Ok(Payload::Rust(
                self.spawn_local_turbo(&canonical_local_turbo, shim_args),
            ))
        } else {
            let global_version = get_version();
            try_check_for_updates(&shim_args, global_version);
            debug!("Running command as global turbo");

            // Absence of turbo.json is not an error per business logic.
            let turbo_json_root = self.repo_state.clone().unwrap().root;
            if let Ok(turbo_json) = turbo_json::read(&turbo_json_root) {
                match turbo_json.check_version(global_version) {
                    Ok(version_match) => {
                        if !version_match {
                            return Err(anyhow!(
                                "You specified needing `turbo` version {}, but you're running {}",
                                turbo_json.turbo_version,
                                global_version
                            ));
                        }
                    }
                    Err(err) => {
                        return Err(anyhow!(
                            "The version string in turbo.json at `turboVersion` is invalid: {}. {}",
                            turbo_json.turbo_version,
                            err.to_string()
                        ));
                    }
                }
            }

            // cli::run checks for this env var, rather than an arg, so that we can support
            // calling old versions without passing unknown flags.
            env::set_var(cli::INVOCATION_DIR_ENV_VAR, &shim_args.invocation_dir);
            cli::run(Some(self.repo_state.clone().unwrap()))
        }
    }

    fn local_turbo_supports_skip_infer_and_single_package(&self) -> Result<bool> {
        if let Some(LocalTurboState { version, .. }) =
            self.repo_state.clone().unwrap().local_turbo_state
        {
            Ok(turbo_version_has_shim(&version))
        } else {
            Ok(false)
        }
    }

    fn spawn_local_turbo(&self, local_turbo_path: &Path, mut shim_args: ShimArgs) -> Result<i32> {
        debug!(
            "Running local turbo binary in {}\n",
            local_turbo_path.display()
        );

        let supports_skip_infer_and_single_package =
            self.local_turbo_supports_skip_infer_and_single_package()?;
        let already_has_single_package_flag = shim_args
            .remaining_turbo_args
            .contains(&"--single-package".to_string());
        let should_add_single_package_flag = self.repo_state.clone().unwrap().mode
            == RepoMode::SinglePackage
            && !already_has_single_package_flag
            && supports_skip_infer_and_single_package;

        debug!(
            "supports_skip_infer_and_single_package {:?}",
            supports_skip_infer_and_single_package
        );
        let cwd = fs_canonicalize(self.repo_state.clone().unwrap().root)?;
        let mut raw_args: Vec<_> = if supports_skip_infer_and_single_package {
            vec!["--skip-infer".to_string()]
        } else {
            Vec::new()
        };

        raw_args.append(&mut shim_args.remaining_turbo_args);

        // We add this flag after the raw args to avoid accidentally passing it
        // as a global flag instead of as a run flag.
        if should_add_single_package_flag {
            raw_args.push("--single-package".to_string());
        }

        raw_args.push("--".to_string());
        raw_args.append(&mut shim_args.forwarded_args);

        // We spawn a process that executes the local turbo
        // that we've found in node_modules/.bin/turbo.
        let mut command = process::Command::new(local_turbo_path)
            .args(&raw_args)
            // rather than passing an argument that local turbo might not understand, set
            // an environment variable that can be optionally used
            .env(cli::INVOCATION_DIR_ENV_VAR, &shim_args.invocation_dir)
            .current_dir(cwd)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("Failed to execute turbo.");

        Ok(command.wait()?.code().unwrap_or(2))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_skip_infer_version_constraint() {
        let canary = "1.7.0-canary.0";
        let newer_canary = "1.7.0-canary.1";
        let newer_minor_canary = "1.7.1-canary.6";
        let release = "1.7.0";
        let old = "1.6.3";
        let old_canary = "1.6.2-canary.1";
        let new = "1.8.0";
        let new_major = "2.1.0";

        assert!(turbo_version_has_shim(release));
        assert!(turbo_version_has_shim(canary));
        assert!(turbo_version_has_shim(newer_canary));
        assert!(turbo_version_has_shim(newer_minor_canary));
        assert!(turbo_version_has_shim(new));
        assert!(turbo_version_has_shim(new_major));
        assert!(!turbo_version_has_shim(old));
        assert!(!turbo_version_has_shim(old_canary));
    }
}
