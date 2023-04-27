use std::{
    env,
    env::current_dir,
    ffi::OsString,
    fs::{self},
    path::{Path, PathBuf},
    process,
    process::Stdio,
    time::Duration,
};

use anyhow::{anyhow, Result};
use const_format::formatcp;
use dunce::canonicalize as fs_canonicalize;
use is_terminal::IsTerminal;
use semver::Version;
use serde::{Deserialize, Serialize};
use tiny_gradient::{GradientStr, RGB};
use tracing::{debug, metadata::LevelFilter};
use tracing_subscriber::EnvFilter;
use turbo_updater::check_for_updates;

use crate::{
    cli, formatter::TurboFormatter, get_version, package_manager::Globs, spawn_child,
    PackageManager, Payload,
};

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
    cwd: PathBuf,
    invocation_dir: PathBuf,
    skip_infer: bool,
    verbosity: usize,
    force_update_check: bool,
    remaining_turbo_args: Vec<String>,
    forwarded_args: Vec<String>,
}

impl ShimArgs {
    pub fn parse() -> Result<Self> {
        let mut found_cwd_flag = false;
        let mut cwd: Option<PathBuf> = None;
        let mut skip_infer = false;
        let mut found_verbosity_flag = false;
        let mut verbosity = 0;
        let mut force_update_check = false;
        let mut remaining_turbo_args = Vec::new();
        let mut forwarded_args = Vec::new();
        let mut is_forwarded_args = false;
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
                found_verbosity_flag = true
            } else if arg.starts_with("--verbosity=") || found_verbosity_flag {
                let verbosity_count = if found_verbosity_flag {
                    found_verbosity_flag = false;
                    &arg
                } else {
                    arg.strip_prefix("--verbosity=").unwrap_or("0")
                };

                verbosity = verbosity_count.parse::<usize>().unwrap_or(0);
            } else if arg == "-v" || arg.starts_with("-vv") {
                verbosity = arg[1..].len();
            } else if found_cwd_flag {
                // We've seen a `--cwd` and therefore set the cwd to this arg.
                cwd = Some(arg.into());
                found_cwd_flag = false;
            } else if arg == "--cwd" {
                if cwd.is_some() {
                    return Err(anyhow!("cannot have multiple `--cwd` flags in command"));
                }
                // If we see a `--cwd` we expect the next arg to be a path.
                found_cwd_flag = true
            } else if let Some(cwd_arg) = arg.strip_prefix("--cwd=") {
                // In the case where `--cwd` is passed as `--cwd=./path/to/foo`, that
                // entire chunk is a single arg, so we need to split it up.
                if cwd.is_some() {
                    return Err(anyhow!("cannot have multiple `--cwd` flags in command"));
                }
                cwd = Some(cwd_arg.into());
            } else {
                remaining_turbo_args.push(arg);
            }
        }

        if found_cwd_flag {
            Err(anyhow!("No value assigned to `--cwd` argument"))
        } else {
            let invocation_dir = current_dir()?;
            let cwd = if let Some(cwd) = cwd {
                fs_canonicalize(cwd)?
            } else {
                invocation_dir.clone()
            };

            Ok(ShimArgs {
                cwd,
                invocation_dir,
                skip_infer,
                verbosity,
                force_update_check,
                remaining_turbo_args,
                forwarded_args,
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RepoMode {
    SinglePackage,
    MultiPackage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PackageJson {
    version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct YarnRc {
    pnp_unplugged_folder: PathBuf,
}

impl Default for YarnRc {
    fn default() -> Self {
        Self {
            pnp_unplugged_folder: [".yarn", "unplugged"].iter().collect(),
        }
    }
}

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
        include_str!("../../../version.txt")
            .lines()
            .next()
            .expect("Failed to read version from version.txt")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalTurboState {
    bin_path: PathBuf,
    version: String,
}

impl LocalTurboState {
    // Hoisted strategy:
    // - `npm install`
    // - `yarn`
    // - `yarn install --flat`
    // - berry (nodeLinker: "node-modules")
    //
    // This also supports people directly depending upon the platform version.
    fn generate_hoisted_path(root_path: &Path) -> Option<PathBuf> {
        Some(root_path.join("node_modules"))
    }

    // Nested strategy:
    // - `npm install --install-strategy=shallow` (`npm install --global-style`)
    // - `npm install --install-strategy=nested` (`npm install --legacy-bundling`)
    // - berry (nodeLinker: "pnpm")
    fn generate_nested_path(root_path: &Path) -> Option<PathBuf> {
        Some(
            root_path
                .join("node_modules")
                .join("turbo")
                .join("node_modules"),
        )
    }

    // Linked strategy:
    // - `pnpm install`
    // - `npm install --install-strategy=linked`
    fn generate_linked_path(root_path: &Path) -> Option<PathBuf> {
        fs_canonicalize(root_path.join("node_modules").join("turbo").join("..")).ok()
    }

    // The unplugged directory doesn't have a fixed path.
    fn get_unplugged_base_path(root_path: &Path) -> PathBuf {
        let yarn_rc_filename =
            env::var_os("YARN_RC_FILENAME").unwrap_or_else(|| OsString::from(".yarnrc.yml"));
        let yarn_rc_filepath = root_path.join(yarn_rc_filename);

        let yarn_rc_yaml_string = fs::read_to_string(yarn_rc_filepath).unwrap_or_default();
        let yarn_rc: YarnRc = serde_yaml::from_str(&yarn_rc_yaml_string).unwrap_or_default();

        root_path.join(yarn_rc.pnp_unplugged_folder)
    }

    // Unplugged strategy:
    // - berry 2.1+
    fn generate_unplugged_path(root_path: &Path) -> Option<PathBuf> {
        let platform_package_name = TurboState::platform_package_name();
        let unplugged_base_path = Self::get_unplugged_base_path(root_path);

        unplugged_base_path
            .read_dir()
            .ok()
            .and_then(|mut read_dir| {
                // berry includes additional metadata in the filename.
                // We actually have to find the platform package.
                read_dir.find_map(|item| match item {
                    Ok(entry) => {
                        let file_name = entry.file_name();
                        if file_name
                            .to_string_lossy()
                            .starts_with(platform_package_name)
                        {
                            Some(unplugged_base_path.join(file_name).join("node_modules"))
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
    pub fn infer(root_path: &Path) -> Option<Self> {
        let platform_package_name = TurboState::platform_package_name();
        let binary_name = TurboState::binary_name();

        let platform_package_json_path: PathBuf =
            [platform_package_name, "package.json"].iter().collect();
        let platform_package_executable_path: PathBuf =
            [platform_package_name, "bin", binary_name].iter().collect();

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
            let bin_path = root.join(&platform_package_executable_path);
            match fs_canonicalize(&bin_path) {
                Ok(bin_path) => {
                    let resolved_package_json_path = root.join(platform_package_json_path);
                    let platform_package_json_string =
                        fs::read_to_string(resolved_package_json_path).ok()?;
                    let platform_package_json: PackageJson =
                        serde_json::from_str(&platform_package_json_string).ok()?;

                    debug!("Local turbo path: {}", bin_path.display());
                    debug!("Local turbo version: {}", platform_package_json.version);
                    return Some(Self {
                        bin_path,
                        version: platform_package_json.version,
                    });
                }
                Err(_) => debug!("No local turbo binary found at: {}", bin_path.display()),
            }
        }

        None
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RepoState {
    pub root: PathBuf,
    pub mode: RepoMode,
    pub local_turbo_state: Option<LocalTurboState>,
}

#[derive(Debug)]
struct InferInfo {
    path: PathBuf,
    has_package_json: bool,
    has_turbo_json: bool,
    workspace_globs: Option<Globs>,
}

impl InferInfo {
    pub fn has_package_json(info: &'_ &InferInfo) -> bool {
        info.has_package_json
    }
    pub fn has_turbo_json(info: &'_ &InferInfo) -> bool {
        info.has_turbo_json
    }

    pub fn is_workspace_root_of(&self, target_path: &Path) -> bool {
        match &self.workspace_globs {
            Some(globs) => globs
                .test(self.path.to_path_buf(), target_path.to_path_buf())
                .unwrap_or(false),
            None => false,
        }
    }
}

impl RepoState {
    fn generate_potential_turbo_roots(reference_dir: &Path) -> Vec<InferInfo> {
        // Find all directories that contain a `package.json` or a `turbo.json`.
        // Gather a bit of additional metadata about them.
        let potential_turbo_roots = reference_dir
            .ancestors()
            .filter_map(|path| {
                let has_package_json = fs::metadata(path.join("package.json")).is_ok();
                let has_turbo_json = fs::metadata(path.join("turbo.json")).is_ok();

                if !has_package_json && !has_turbo_json {
                    return None;
                }

                // FIXME: This should be based upon detecting the pacakage manager.
                // However, we don't have that functionality implemented in Rust yet.
                // PackageManager::detect(path).get_workspace_globs().unwrap_or(None)
                let workspace_globs = PackageManager::Pnpm
                    .get_workspace_globs(path)
                    .unwrap_or_else(|_| {
                        PackageManager::Npm
                            .get_workspace_globs(path)
                            .unwrap_or(None)
                    });

                Some(InferInfo {
                    path: path.to_owned(),
                    has_package_json,
                    has_turbo_json,
                    workspace_globs,
                })
            })
            .collect();

        potential_turbo_roots
    }

    fn process_potential_turbo_roots(potential_turbo_roots: Vec<InferInfo>) -> Result<Self> {
        // Potential improvements:
        // - Detect invalid configuration where turbo.json isn't peer to package.json.
        // - There are a couple of possible early exits to prevent traversing all the
        //   way to root at significant code complexity increase.
        //
        //   1. [0].has_turbo_json && [0].workspace_globs.is_some()
        //   2. [0].has_turbo_json && [n].has_turbo_json && [n].is_workspace_root_of(0)
        //
        // If we elect to make any of the changes for early exits we need to expand test
        // suite which presently relies on the fact that the selection runs in a loop to
        // avoid creating those test cases.

        // We need to perform the same search strategy for _both_ turbo.json and _then_
        // package.json.
        let search_locations = [InferInfo::has_turbo_json, InferInfo::has_package_json];

        for check_set_comparator in search_locations {
            let mut check_roots = potential_turbo_roots
                .iter()
                .filter(check_set_comparator)
                .peekable();

            let current_option = check_roots.next();

            // No potential roots checking by this comparator.
            if current_option.is_none() {
                continue;
            }

            let current = current_option.unwrap();

            // If there is only one potential root, that's the winner.
            if check_roots.peek().is_none() {
                let local_turbo_state = LocalTurboState::infer(&current.path);
                return Ok(Self {
                    root: current.path.to_path_buf(),
                    mode: if current.workspace_globs.is_some() {
                        RepoMode::MultiPackage
                    } else {
                        RepoMode::SinglePackage
                    },
                    local_turbo_state,
                });

            // More than one potential root. See if we can stop at the first.
            // This is a performance optimization. We could remove this case,
            // and set the mode properly in the else and it would still work.
            } else if current.workspace_globs.is_some() {
                // If the closest one has workspaces then we stop there.
                let local_turbo_state = LocalTurboState::infer(&current.path);
                return Ok(Self {
                    root: current.path.to_path_buf(),
                    mode: RepoMode::MultiPackage,
                    local_turbo_state,
                });

            // More than one potential root.
            // Closest is not RepoMode::MultiPackage
            // We attempt to prove that the closest is a workspace of a parent.
            // Failing that we just choose the closest.
            } else {
                for ancestor_infer in check_roots {
                    if ancestor_infer.is_workspace_root_of(&current.path) {
                        let local_turbo_state = LocalTurboState::infer(&ancestor_infer.path);
                        return Ok(Self {
                            root: ancestor_infer.path.to_path_buf(),
                            mode: RepoMode::MultiPackage,
                            local_turbo_state,
                        });
                    }
                }

                // We have eliminated RepoMode::MultiPackage as an option.
                // We must exhaustively check before this becomes the answer.
                let local_turbo_state = LocalTurboState::infer(&current.path);
                return Ok(Self {
                    root: current.path.to_path_buf(),
                    mode: RepoMode::SinglePackage,
                    local_turbo_state,
                });
            }
        }

        // If we're here we didn't find a valid root.
        Err(anyhow!("Root could not be inferred."))
    }

    /// Infers `RepoState` from current directory.
    ///
    /// # Arguments
    ///
    /// * `current_dir`: Current working directory
    ///
    /// returns: Result<RepoState, Error>
    pub fn infer(reference_dir: &Path) -> Result<Self> {
        let potential_turbo_roots = RepoState::generate_potential_turbo_roots(reference_dir);
        RepoState::process_potential_turbo_roots(potential_turbo_roots)
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
    fn run_correct_turbo(self, shim_args: ShimArgs) -> Result<Payload> {
        if let Some(LocalTurboState { bin_path, version }) = &self.local_turbo_state {
            try_check_for_updates(&shim_args, version);
            let canonical_local_turbo = fs_canonicalize(bin_path)?;
            Ok(Payload::Rust(
                self.spawn_local_turbo(&canonical_local_turbo, shim_args),
            ))
        } else {
            try_check_for_updates(&shim_args, get_version());
            // cli::run checks for this env var, rather than an arg, so that we can support
            // calling old versions without passing unknown flags.
            env::set_var(cli::INVOCATION_DIR_ENV_VAR, &shim_args.invocation_dir);
            debug!("Running command as global turbo");
            cli::run(Some(self))
        }
    }

    fn local_turbo_supports_skip_infer_and_single_package(&self) -> Result<bool> {
        if let Some(LocalTurboState { version, .. }) = &self.local_turbo_state {
            Ok(turbo_version_has_shim(version))
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
        let should_add_single_package_flag = self.mode == RepoMode::SinglePackage
            && !already_has_single_package_flag
            && supports_skip_infer_and_single_package;

        debug!(
            "supports_skip_infer_and_single_package {:?}",
            supports_skip_infer_and_single_package
        );
        let cwd = fs_canonicalize(&self.root)?;
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
            .env(cli::INVOCATION_DIR_ENV_VAR, &shim_args.invocation_dir)
            .current_dir(cwd)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());

        let child = spawn_child(command)?;

        let exit_code = child.wait()?.code().unwrap_or(2);

        Ok(exit_code)
    }
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

fn init_subscriber(verbosity: usize) {
    let max_level = match verbosity {
        0 => LevelFilter::WARN,
        1 => LevelFilter::INFO,
        2 => LevelFilter::DEBUG,
        _ => LevelFilter::TRACE,
    };

    // respect TURBO_LOG_VERBOSITY env var
    // respect verbosity arg
    tracing_subscriber::fmt()
        .event_format(TurboFormatter::new_with_ansi(
            std::io::stdout().is_terminal(),
        ))
        .with_env_filter(EnvFilter::from_env("TURBO_LOG_VERBOSITY"))
        .with_max_level(max_level)
        .init();
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

pub fn run() -> Result<Payload> {
    let args = ShimArgs::parse()?;

    init_subscriber(args.verbosity);
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
            debug!("Repository Root: {}", repo_state.root.to_string_lossy());
            repo_state.run_correct_turbo(args)
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

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_process_potential_turbo_roots() {
        struct TestCase {
            description: &'static str,
            infer_infos: Vec<InferInfo>,
            output: Result<PathBuf>,
        }

        let tests = [
            // Test for zero, exhaustive.
            TestCase {
                description: "No matches found.",
                infer_infos: vec![],
                output: Err(anyhow!("Root could not be inferred.")),
            },
            // Test for one, exhaustive.
            TestCase {
                description: "Only one, is monorepo with turbo.json.",
                infer_infos: vec![InferInfo {
                    path: PathBuf::from("/path/to/root"),
                    has_package_json: true,
                    has_turbo_json: true,
                    workspace_globs: Some(Globs {
                        inclusions: vec!["packages/*".to_string()],
                        exclusions: vec![],
                    }),
                }],
                output: Ok(PathBuf::from("/path/to/root")),
            },
            TestCase {
                description: "Only one, is non-monorepo with turbo.json.",
                infer_infos: vec![InferInfo {
                    path: PathBuf::from("/path/to/root"),
                    has_package_json: true,
                    has_turbo_json: true,
                    workspace_globs: None,
                }],
                output: Ok(PathBuf::from("/path/to/root")),
            },
            TestCase {
                description: "Only one, is monorepo without turbo.json.",
                infer_infos: vec![InferInfo {
                    path: PathBuf::from("/path/to/root"),
                    has_package_json: true,
                    has_turbo_json: false,
                    workspace_globs: Some(Globs {
                        inclusions: vec!["packages/*".to_string()],
                        exclusions: vec![],
                    }),
                }],
                output: Ok(PathBuf::from("/path/to/root")),
            },
            TestCase {
                description: "Only one, is non-monorepo without turbo.json.",
                infer_infos: vec![InferInfo {
                    path: PathBuf::from("/path/to/root"),
                    has_package_json: true,
                    has_turbo_json: false,
                    workspace_globs: None,
                }],
                output: Ok(PathBuf::from("/path/to/root")),
            },
            // Tests for how to choose what is closest.
            TestCase {
                description: "Execution in a workspace.",
                infer_infos: vec![
                    InferInfo {
                        path: PathBuf::from("/path/to/root/packages/ui-library"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: None,
                    },
                    InferInfo {
                        path: PathBuf::from("/path/to/root"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: Some(Globs {
                            inclusions: vec!["packages/*".to_string()],
                            exclusions: vec![],
                        }),
                    },
                ],
                output: Ok(PathBuf::from("/path/to/root")),
            },
            TestCase {
                description: "Execution in a workspace, weird package layout.",
                infer_infos: vec![
                    InferInfo {
                        path: PathBuf::from("/path/to/root/packages/ui-library/css"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: None,
                    },
                    InferInfo {
                        path: PathBuf::from("/path/to/root/packages/ui-library"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: None,
                    },
                    InferInfo {
                        path: PathBuf::from("/path/to/root"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: Some(Globs {
                            // This `**` is important:
                            inclusions: vec!["packages/**".to_string()],
                            exclusions: vec![],
                        }),
                    },
                ],
                output: Ok(PathBuf::from("/path/to/root")),
            },
            TestCase {
                description: "Nested disjoint monorepo roots.",
                infer_infos: vec![
                    InferInfo {
                        path: PathBuf::from("/path/to/root-one/root-two"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: Some(Globs {
                            inclusions: vec!["packages/*".to_string()],
                            exclusions: vec![],
                        }),
                    },
                    InferInfo {
                        path: PathBuf::from("/path/to/root-one"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: Some(Globs {
                            inclusions: vec!["packages/*".to_string()],
                            exclusions: vec![],
                        }),
                    },
                ],
                output: Ok(PathBuf::from("/path/to/root-one/root-two")),
            },
            TestCase {
                description: "Nested disjoint monorepo roots, execution in a workspace of the \
                              closer root.",
                infer_infos: vec![
                    InferInfo {
                        path: PathBuf::from(
                            "/path/to/root-one/root-two/root-two-packages/ui-library",
                        ),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: None,
                    },
                    InferInfo {
                        path: PathBuf::from("/path/to/root-one/root-two"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: Some(Globs {
                            inclusions: vec!["root-two-packages/*".to_string()],
                            exclusions: vec![],
                        }),
                    },
                    InferInfo {
                        path: PathBuf::from("/path/to/root-one"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: Some(Globs {
                            inclusions: vec!["root-two/root-one-packages/*".to_string()],
                            exclusions: vec![],
                        }),
                    },
                ],
                output: Ok(PathBuf::from("/path/to/root-one/root-two")),
            },
            TestCase {
                description: "Nested disjoint monorepo roots, execution in a workspace of the \
                              farther root.",
                infer_infos: vec![
                    InferInfo {
                        path: PathBuf::from(
                            "/path/to/root-one/root-two/root-one-packages/ui-library",
                        ),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: None,
                    },
                    InferInfo {
                        path: PathBuf::from("/path/to/root-one/root-two"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: Some(Globs {
                            inclusions: vec!["root-two-packages/*".to_string()],
                            exclusions: vec![],
                        }),
                    },
                    InferInfo {
                        path: PathBuf::from("/path/to/root-one"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: Some(Globs {
                            inclusions: vec!["root-two/root-one-packages/*".to_string()],
                            exclusions: vec![],
                        }),
                    },
                ],
                output: Ok(PathBuf::from("/path/to/root-one")),
            },
            TestCase {
                description: "Disjoint package.",
                infer_infos: vec![
                    InferInfo {
                        path: PathBuf::from("/path/to/root/some-other-project"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: None,
                    },
                    InferInfo {
                        path: PathBuf::from("/path/to/root"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: Some(Globs {
                            inclusions: vec!["packages/*".to_string()],
                            exclusions: vec![],
                        }),
                    },
                ],
                output: Ok(PathBuf::from("/path/to/root/some-other-project")),
            },
            TestCase {
                description: "Monorepo trying to point to a monorepo. We choose the closer one \
                              and ignore the problem.",
                infer_infos: vec![
                    InferInfo {
                        path: PathBuf::from("/path/to/root-one/root-two"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: Some(Globs {
                            inclusions: vec!["packages/*".to_string()],
                            exclusions: vec![],
                        }),
                    },
                    InferInfo {
                        path: PathBuf::from("/path/to/root-one"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: Some(Globs {
                            inclusions: vec!["root-two".to_string()],
                            exclusions: vec![],
                        }),
                    },
                ],
                output: Ok(PathBuf::from("/path/to/root-one/root-two")),
            },
            TestCase {
                description: "Nested non-monorepo packages.",
                infer_infos: vec![
                    InferInfo {
                        path: PathBuf::from("/path/to/project-one/project-two"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: None,
                    },
                    InferInfo {
                        path: PathBuf::from("/path/to/project-one"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: None,
                    },
                ],
                output: Ok(PathBuf::from("/path/to/project-one/project-two")),
            },
            // The below test ensures that we privilege a valid `turbo.json` structure prior to
            // evaluation of a valid `package.json` structure. If you include `turbo.json` you are
            // able to "skip" deeper into the resolution by disregarding anything that does _not_
            // have a `turbo.json`. This will matter _far_ more in a multi-language environment.

            // Just one example test proves that the entire alternative chain construction works.
            // The selection logic from within this set is identical. If we attempt to optimize the
            // number of file system reads by early-exiting for matching we should expand this test
            // set to mirror the above section.
            TestCase {
                description: "Nested non-monorepo packages, turbo.json primacy.",
                infer_infos: vec![
                    InferInfo {
                        path: PathBuf::from("/path/to/project-one/project-two"),
                        has_package_json: true,
                        has_turbo_json: false,
                        workspace_globs: None,
                    },
                    InferInfo {
                        path: PathBuf::from("/path/to/project-one"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: None,
                    },
                ],
                output: Ok(PathBuf::from("/path/to/project-one")),
            },
        ];

        for test in tests {
            match RepoState::process_potential_turbo_roots(test.infer_infos) {
                Ok(repo_state) => assert_eq!(
                    repo_state.root,
                    test.output.unwrap(),
                    "{}",
                    test.description
                ),
                Err(err) => assert_eq!(
                    err.to_string(),
                    test.output.unwrap_err().to_string(),
                    "{}",
                    test.description
                ),
            };
        }
    }

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

    #[cfg(windows)]
    #[test]
    fn test_windows_path_normalization() -> Result<()> {
        let cwd = current_dir()?;
        let normalized = fs_canonicalize(&cwd)?;
        // Just make sure it isn't a UNC path
        assert!(!normalized.starts_with("\\\\?"));
        Ok(())
    }
}
