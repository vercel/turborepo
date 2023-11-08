use std::{
    backtrace::Backtrace,
    env,
    fs::{self},
    path::PathBuf,
    process,
    process::Stdio,
    time::Duration,
};

use camino::Utf8PathBuf;
use const_format::formatcp;
use dunce::canonicalize as fs_canonicalize;
use semver::Version;
use serde::Deserialize;
use thiserror::Error;
use tiny_gradient::{GradientStr, RGB};
use tracing::debug;
use turbo_updater::check_for_updates;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_repository::{
    inference::{RepoMode, RepoState},
    package_json::PackageJson,
};
use turborepo_ui::UI;

use crate::{cli, get_version, spawn_child, tracing::TurboSubscriber, Payload};

#[derive(Debug, Error)]
pub enum Error {
    #[error("cannot have multiple `--cwd` flags in command")]
    MultipleCwd(#[backtrace] Backtrace),
    #[error("No value assigned to `--cwd` argument")]
    EmptyCwd(#[backtrace] Backtrace),
    #[error(transparent)]
    Cli(#[from] cli::Error),
    #[error(transparent)]
    Inference(#[from] turborepo_repository::inference::Error),
    #[error("failed to execute local turbo process")]
    LocalTurboProcess(#[source] std::io::Error),
    #[error("failed to resolve local turbo path: {0}")]
    LocalTurboPath(String),
    #[error("failed to resolve repository root: {0}")]
    RepoRootPath(AbsoluteSystemPathBuf),
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
}

// all arguments that result in a stdout that much be directly parsable and
// should not be paired with additional output (from the update notifier for
// example)
static TURBO_PURE_OUTPUT_ARGS: [&str; 6] = [
    "--json",
    "--dry",
    "--dry-run",
    "--dry=json",
    "--graph",
    "--dry-run=json",
];

static TURBO_SKIP_NOTIFIER_ARGS: [&str; 5] =
    ["--help", "--h", "--version", "--v", "--no-update-notifier"];

fn turbo_version_has_shim(version: &str) -> bool {
    let version = Version::parse(version).unwrap();
    // only need to check major and minor (this will include canaries)
    if version.major == 1 {
        return version.minor >= 7;
    }

    version.major > 1
}

#[derive(Debug)]
struct ShimArgs {
    cwd: AbsoluteSystemPathBuf,
    invocation_dir: AbsoluteSystemPathBuf,
    skip_infer: bool,
    verbosity: usize,
    force_update_check: bool,
    remaining_turbo_args: Vec<String>,
    forwarded_args: Vec<String>,
    color: bool,
    no_color: bool,
}

impl ShimArgs {
    pub fn parse() -> Result<Self, Error> {
        let mut found_cwd_flag = false;
        let mut cwd: Option<AbsoluteSystemPathBuf> = None;
        let mut skip_infer = false;
        let mut found_verbosity_flag = false;
        let mut verbosity = 0;
        let mut force_update_check = false;
        let mut remaining_turbo_args = Vec::new();
        let mut forwarded_args = Vec::new();
        let mut is_forwarded_args = false;
        let mut color = false;
        let mut no_color = false;
        let args = env::args().skip(1);
        for arg in args {
            // We've seen a `--` and therefore we do no parsing
            if is_forwarded_args {
                forwarded_args.push(arg);
            } else if arg == "--skip-infer" {
                skip_infer = true;
            } else if arg == "--check-for-update" {
                force_update_check = true;
            } else if arg == "--" {
                // If we've hit `--` we've reached the args forwarded to tasks.
                is_forwarded_args = true;
            } else if arg == "--verbosity" {
                // If we see `--verbosity` we expect the next arg to be a number.
                remaining_turbo_args.push(arg);
                found_verbosity_flag = true
            } else if arg.starts_with("--verbosity=") || found_verbosity_flag {
                let verbosity_count = if found_verbosity_flag {
                    found_verbosity_flag = false;
                    &arg
                } else {
                    arg.strip_prefix("--verbosity=").unwrap_or("0")
                };

                verbosity = verbosity_count.parse::<usize>().unwrap_or(0);
                remaining_turbo_args.push(arg);
            } else if arg == "-v" || arg.starts_with("-vv") {
                verbosity = arg[1..].len();
                remaining_turbo_args.push(arg);
            } else if found_cwd_flag {
                // We've seen a `--cwd` and therefore set the cwd to this arg.
                //cwd = Some(arg.into());
                cwd = Some(AbsoluteSystemPathBuf::from_cwd(arg)?);
                found_cwd_flag = false;
            } else if arg == "--cwd" {
                if cwd.is_some() {
                    return Err(Error::MultipleCwd(Backtrace::capture()));
                }
                // If we see a `--cwd` we expect the next arg to be a path.
                found_cwd_flag = true
            } else if let Some(cwd_arg) = arg.strip_prefix("--cwd=") {
                // In the case where `--cwd` is passed as `--cwd=./path/to/foo`, that
                // entire chunk is a single arg, so we need to split it up.
                if cwd.is_some() {
                    return Err(Error::MultipleCwd(Backtrace::capture()));
                }
                cwd = Some(AbsoluteSystemPathBuf::from_cwd(cwd_arg)?);
            } else if arg == "--color" {
                color = true;
            } else if arg == "--no-color" {
                no_color = true;
            } else {
                remaining_turbo_args.push(arg);
            }
        }

        if found_cwd_flag {
            Err(Error::EmptyCwd(Backtrace::capture()))
        } else {
            let invocation_dir = AbsoluteSystemPathBuf::cwd()?;
            let cwd = cwd.unwrap_or_else(|| invocation_dir.clone());

            Ok(ShimArgs {
                cwd,
                invocation_dir,
                skip_infer,
                verbosity,
                force_update_check,
                remaining_turbo_args,
                forwarded_args,
                color,
                no_color,
            })
        }
    }

    // returns true if any flags result in pure json output to stdout
    fn has_json_flags(&self) -> bool {
        self.remaining_turbo_args
            .iter()
            .any(|arg| TURBO_PURE_OUTPUT_ARGS.contains(&arg.as_str()))
    }

    // returns true if any flags should bypass the update notifier
    fn has_notifier_skip_flags(&self) -> bool {
        self.remaining_turbo_args
            .iter()
            .any(|arg| TURBO_SKIP_NOTIFIER_ARGS.contains(&arg.as_str()))
    }

    pub fn should_check_for_update(&self) -> bool {
        if self.force_update_check {
            return true;
        }

        if self.has_notifier_skip_flags() || self.has_json_flags() {
            return false;
        }

        true
    }

    pub fn ui(&self) -> UI {
        if self.no_color {
            UI::new(true)
        } else if self.color {
            UI::new(false)
        } else {
            UI::infer()
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct YarnRc {
    pnp_unplugged_folder: Utf8PathBuf,
}

impl Default for YarnRc {
    fn default() -> Self {
        Self {
            pnp_unplugged_folder: [".yarn", "unplugged"].iter().collect(),
        }
    }
}

#[derive(Debug)]
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
    pub const fn platform_name() -> &'static str {
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

        formatcp!("{}-{}", OS, ARCH)
    }

    pub const fn platform_package_name() -> &'static str {
        formatcp!("turbo-{}", TurboState::platform_name())
    }

    pub const fn binary_name() -> &'static str {
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
        include_str!("../../../version.txt")
            .lines()
            .next()
            .expect("Failed to read version from version.txt")
    }
}

#[derive(Debug)]
pub struct LocalTurboState {
    bin_path: PathBuf,
    version: String,
}

impl LocalTurboState {
    // Hoisted strategy:
    // - `bun install`
    // - `npm install`
    // - `yarn`
    // - `yarn install --flat`
    // - berry (nodeLinker: "node-modules")
    //
    // This also supports people directly depending upon the platform version.
    fn generate_hoisted_path(root_path: &AbsoluteSystemPath) -> Option<AbsoluteSystemPathBuf> {
        Some(root_path.join_component("node_modules"))
    }

    // Nested strategy:
    // - `npm install --install-strategy=shallow` (`npm install --global-style`)
    // - `npm install --install-strategy=nested` (`npm install --legacy-bundling`)
    // - berry (nodeLinker: "pnpm")
    fn generate_nested_path(root_path: &AbsoluteSystemPath) -> Option<AbsoluteSystemPathBuf> {
        Some(root_path.join_components(&["node_modules", "turbo", "node_modules"]))
    }

    // Linked strategy:
    // - `pnpm install`
    // - `npm install --install-strategy=linked`
    fn generate_linked_path(root_path: &AbsoluteSystemPath) -> Option<AbsoluteSystemPathBuf> {
        let canonical_path = fs_canonicalize(
            root_path
                .as_path()
                .join("node_modules")
                .join("turbo")
                .join(".."),
        )
        .ok()?;

        AbsoluteSystemPathBuf::try_from(canonical_path).ok()
    }

    // The unplugged directory doesn't have a fixed path.
    fn get_unplugged_base_path(root_path: &AbsoluteSystemPath) -> Utf8PathBuf {
        let yarn_rc_filename =
            env::var("YARN_RC_FILENAME").unwrap_or_else(|_| String::from(".yarnrc.yml"));
        let yarn_rc_filepath = root_path.as_path().join(yarn_rc_filename);

        let yarn_rc_yaml_string = fs::read_to_string(yarn_rc_filepath).unwrap_or_default();
        let yarn_rc: YarnRc = serde_yaml::from_str(&yarn_rc_yaml_string).unwrap_or_default();

        root_path.as_path().join(yarn_rc.pnp_unplugged_folder)
    }

    // Unplugged strategy:
    // - berry 2.1+
    fn generate_unplugged_path(root_path: &AbsoluteSystemPath) -> Option<AbsoluteSystemPathBuf> {
        let platform_package_name = TurboState::platform_package_name();
        let unplugged_base_path = Self::get_unplugged_base_path(root_path);

        unplugged_base_path
            .read_dir_utf8()
            .ok()
            .and_then(|mut read_dir| {
                // berry includes additional metadata in the filename.
                // We actually have to find the platform package.
                read_dir.find_map(|item| match item {
                    Ok(entry) => {
                        let file_name = entry.file_name();
                        if file_name.starts_with(platform_package_name) {
                            AbsoluteSystemPathBuf::new(
                                unplugged_base_path.join(file_name).join("node_modules"),
                            )
                            .ok()
                        } else {
                            None
                        }
                    }
                    Err(_) => None,
                })
            })
    }

    // We support six per-platform packages and one `turbo` package which handles
    // indirection. We identify the per-platform package and execute the appropriate
    // binary directly. We can choose to operate this aggressively because the
    // _worst_ outcome is that we run global `turbo`.
    //
    // In spite of that, the only known unsupported local invocation is Yarn/Berry <
    // 2.1 PnP
    pub fn infer(root_path: &AbsoluteSystemPath) -> Option<Self> {
        let platform_package_name = TurboState::platform_package_name();
        let binary_name = TurboState::binary_name();

        let platform_package_json_path_components = [platform_package_name, "package.json"];
        let platform_package_executable_path_components =
            [platform_package_name, "bin", binary_name];

        // These are lazy because the last two are more expensive.
        let search_functions = [
            Self::generate_hoisted_path,
            Self::generate_nested_path,
            Self::generate_linked_path,
            Self::generate_unplugged_path,
        ];

        // Detecting the package manager is more expensive than just doing an exhaustive
        // search.
        for root in search_functions
            .iter()
            .filter_map(|search_function| search_function(root_path))
        {
            // Needs borrow because of the loop.
            #[allow(clippy::needless_borrow)]
            let bin_path = root.join_components(&platform_package_executable_path_components);
            match fs_canonicalize(&bin_path) {
                Ok(bin_path) => {
                    let resolved_package_json_path =
                        root.join_components(&platform_package_json_path_components);
                    let platform_package_json =
                        PackageJson::load(&resolved_package_json_path).ok()?;
                    let local_version = platform_package_json.version?;

                    debug!("Local turbo path: {}", bin_path.display());
                    debug!("Local turbo version: {}", &local_version);
                    return Some(Self {
                        bin_path,
                        version: local_version,
                    });
                }
                Err(_) => debug!("No local turbo binary found at: {}", bin_path),
            }
        }

        None
    }

    fn supports_skip_infer_and_single_package(&self) -> bool {
        turbo_version_has_shim(&self.version)
    }

    /// Check to see if the detected local executable is the one currently
    /// running.
    fn local_is_self(&self) -> bool {
        std::env::current_exe().is_ok_and(|current_exe| {
            fs_canonicalize(current_exe)
                .is_ok_and(|canonical_current_exe| canonical_current_exe == self.bin_path)
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
fn run_correct_turbo(
    repo_state: RepoState,
    shim_args: ShimArgs,
    subscriber: &TurboSubscriber,
    ui: UI,
) -> Result<Payload, Error> {
    if let Some(turbo_state) = LocalTurboState::infer(&repo_state.root) {
        try_check_for_updates(&shim_args, &turbo_state.version);

        if turbo_state.local_is_self() {
            env::set_var(
                cli::INVOCATION_DIR_ENV_VAR,
                shim_args.invocation_dir.as_path(),
            );
            debug!("Currently running turbo is local turbo.");
            Ok(cli::run(Some(repo_state), subscriber, ui)?)
        } else {
            Ok(Payload::Rust(spawn_local_turbo(
                &repo_state,
                turbo_state,
                shim_args,
            )))
        }
    } else {
        try_check_for_updates(&shim_args, get_version());
        // cli::run checks for this env var, rather than an arg, so that we can support
        // calling old versions without passing unknown flags.
        env::set_var(
            cli::INVOCATION_DIR_ENV_VAR,
            shim_args.invocation_dir.as_path(),
        );
        debug!("Running command as global turbo");
        Ok(cli::run(Some(repo_state), subscriber, ui)?)
    }
}

fn spawn_local_turbo(
    repo_state: &RepoState,
    local_turbo_state: LocalTurboState,
    mut shim_args: ShimArgs,
) -> Result<i32, Error> {
    let local_turbo_path = fs_canonicalize(&local_turbo_state.bin_path).map_err(|_| {
        Error::LocalTurboPath(local_turbo_state.bin_path.to_string_lossy().to_string())
    })?;
    debug!(
        "Running local turbo binary in {}\n",
        local_turbo_path.display()
    );

    let supports_skip_infer_and_single_package =
        local_turbo_state.supports_skip_infer_and_single_package();
    let already_has_single_package_flag = shim_args
        .remaining_turbo_args
        .contains(&"--single-package".to_string());
    let should_add_single_package_flag = repo_state.mode == RepoMode::SinglePackage
        && !already_has_single_package_flag
        && supports_skip_infer_and_single_package;

    debug!(
        "supports_skip_infer_and_single_package {:?}",
        supports_skip_infer_and_single_package
    );
    let cwd = fs_canonicalize(&repo_state.root)
        .map_err(|_| Error::RepoRootPath(repo_state.root.clone()))?;

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
    let mut command = process::Command::new(local_turbo_path);
    command
        .args(&raw_args)
        // rather than passing an argument that local turbo might not understand, set
        // an environment variable that can be optionally used
        .env(
            cli::INVOCATION_DIR_ENV_VAR,
            shim_args.invocation_dir.as_path(),
        )
        .current_dir(cwd)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let child = spawn_child(command).map_err(Error::LocalTurboProcess)?;

    let exit_status = child.wait().map_err(Error::LocalTurboProcess)?;
    let exit_code = exit_status.code().unwrap_or_else(|| {
        debug!("go-turbo failed to report exit code");
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            let signal = exit_status.signal();
            let core_dumped = exit_status.core_dumped();
            debug!(
                "go-turbo caught signal {:?}. Core dumped? {}",
                signal, core_dumped
            );
        }
        2
    });

    Ok(exit_code)
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
            url = "https://x.com/turborepo"
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

pub fn run() -> Result<Payload, Error> {
    let args = ShimArgs::parse()?;
    let ui = args.ui();
    let subscriber = TurboSubscriber::new_with_verbosity(args.verbosity, &ui);

    debug!("Global turbo version: {}", get_version());

    // If skip_infer is passed, we're probably running local turbo with
    // global turbo having handled the inference. We can run without any
    // concerns.
    if args.skip_infer {
        return Ok(cli::run(None, &subscriber, ui)?);
    }

    // If the TURBO_BINARY_PATH is set, we do inference but we do not use
    // it to execute local turbo. We simply use it to set the `--single-package`
    // and `--cwd` flags.
    if is_turbo_binary_path_set() {
        let repo_state = RepoState::infer(&args.cwd)?;
        debug!("Repository Root: {}", repo_state.root);
        return Ok(cli::run(Some(repo_state), &subscriber, ui)?);
    }

    match RepoState::infer(&args.cwd) {
        Ok(repo_state) => {
            debug!("Repository Root: {}", repo_state.root);
            run_correct_turbo(repo_state, args, &subscriber, ui)
        }
        Err(err) => {
            // If we cannot infer, we still run global turbo. This allows for global
            // commands like login/logout/link/unlink to still work
            debug!("Repository inference failed: {}", err);
            debug!("Running command as global turbo");
            Ok(cli::run(None, &subscriber, ui)?)
        }
    }
}

#[cfg(test)]
mod test {
    use super::turbo_version_has_shim;

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
