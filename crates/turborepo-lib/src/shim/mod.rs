mod local_turbo_state;
mod turbo_state;

use std::{backtrace::Backtrace, env, process, process::Stdio, time::Duration};

use dunce::canonicalize as fs_canonicalize;
use itertools::Itertools;
use local_turbo_state::LocalTurboState;
use miette::{Diagnostic, SourceSpan};
use thiserror::Error;
use tiny_gradient::{GradientStr, RGB};
use tracing::{debug, warn};
pub use turbo_state::TurboState;
use turbo_updater::display_update_check;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_repository::inference::{RepoMode, RepoState};
use turborepo_ui::UI;

use crate::{cli, get_version, spawn_child, tracing::TurboSubscriber};

const TURBO_GLOBAL_WARNING_DISABLED: &str = "TURBO_GLOBAL_WARNING_DISABLED";

#[derive(Debug, Error, Diagnostic)]
#[error("cannot have multiple `--cwd` flags in command")]
#[diagnostic(code(turbo::shim::multiple_cwd))]
pub struct MultipleCwd {
    #[backtrace]
    backtrace: Backtrace,
    #[source_code]
    args_string: String,
    #[label("first flag declared here")]
    flag1: Option<SourceSpan>,
    #[label("but second flag declared here")]
    flag2: Option<SourceSpan>,
    #[label("and here")]
    flag3: Option<SourceSpan>,
    // The user should get the idea after the first 4 examples.
    #[label("and here")]
    flag4: Option<SourceSpan>,
}

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error(transparent)]
    #[diagnostic(transparent)]
    MultipleCwd(Box<MultipleCwd>),
    #[error("No value assigned to `--cwd` flag")]
    #[diagnostic(code(turbo::shim::empty_cwd))]
    EmptyCwd {
        #[backtrace]
        backtrace: Backtrace,
        #[source_code]
        args_string: String,
        #[label = "Requires a path to be passed after it"]
        flag_range: SourceSpan,
    },
    #[error(transparent)]
    #[diagnostic(transparent)]
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
        let mut cwd_flag_idx = None;
        let mut cwds = Vec::new();
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
        for (idx, arg) in args.enumerate() {
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
            } else if cwd_flag_idx.is_some() {
                // We've seen a `--cwd` and therefore add this to the cwds list along with
                // the index of the `--cwd` (*not* the value)
                cwds.push((AbsoluteSystemPathBuf::from_cwd(arg)?, idx - 1));
                cwd_flag_idx = None;
            } else if arg == "--cwd" {
                // If we see a `--cwd` we expect the next arg to be a path.
                cwd_flag_idx = Some(idx)
            } else if let Some(cwd_arg) = arg.strip_prefix("--cwd=") {
                // In the case where `--cwd` is passed as `--cwd=./path/to/foo`, that
                // entire chunk is a single arg, so we need to split it up.
                cwds.push((AbsoluteSystemPathBuf::from_cwd(cwd_arg)?, idx));
            } else if arg == "--color" {
                color = true;
            } else if arg == "--no-color" {
                no_color = true;
            } else {
                remaining_turbo_args.push(arg);
            }
        }

        if let Some(idx) = cwd_flag_idx {
            let (spans, args_string) =
                Self::get_spans_in_args_string(vec![idx], env::args().skip(1));

            return Err(Error::EmptyCwd {
                backtrace: Backtrace::capture(),
                args_string,
                flag_range: spans[0],
            });
        }

        if cwds.len() > 1 {
            let (indices, args_string) = Self::get_spans_in_args_string(
                cwds.iter().map(|(_, idx)| *idx).collect(),
                env::args().skip(1),
            );

            let mut flags = indices.into_iter();
            return Err(Error::MultipleCwd(Box::new(MultipleCwd {
                backtrace: Backtrace::capture(),
                args_string,
                flag1: flags.next(),
                flag2: flags.next(),
                flag3: flags.next(),
                flag4: flags.next(),
            })));
        }

        let invocation_dir = AbsoluteSystemPathBuf::cwd()?;
        let cwd = cwds
            .pop()
            .map(|(cwd, _)| cwd)
            .unwrap_or_else(|| invocation_dir.clone());

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

    /// Takes a list of indices into a Vec of arguments, i.e. ["--graph", "foo",
    /// "--cwd"] and converts them into `SourceSpan`'s into the string of those
    /// arguments, i.e. "-- graph foo --cwd". Returns the spans and the args
    /// string
    fn get_spans_in_args_string(
        mut args_indices: Vec<usize>,
        args: impl Iterator<Item = impl Into<String>>,
    ) -> (Vec<SourceSpan>, String) {
        // Sort the indices to keep the invariant
        // that if i > j then output[i] > output[j]
        args_indices.sort();
        let mut indices_in_args_string = Vec::new();
        let mut i = 0;
        let mut current_args_string_idx = 0;

        for (idx, arg) in args.enumerate() {
            let Some(arg_idx) = args_indices.get(i) else {
                break;
            };

            let arg = arg.into();

            if idx == *arg_idx {
                indices_in_args_string.push((current_args_string_idx, arg.len()).into());
                i += 1;
            }
            current_args_string_idx += arg.len() + 1;
        }

        let args_string = env::args().skip(1).join(" ");

        (indices_in_args_string, args_string)
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
            // Do our best to enable ansi colors, but even if the terminal doesn't support
            // still emit ansi escape sequences.
            Self::supports_ansi();
            UI::new(false)
        } else if Self::supports_ansi() {
            // If the terminal supports ansi colors, then we can infer if we should emit
            // colors
            UI::infer()
        } else {
            UI::new(true)
        }
    }

    #[cfg(windows)]
    fn supports_ansi() -> bool {
        // This call has the side effect of setting ENABLE_VIRTUAL_TERMINAL_PROCESSING
        // to true. https://learn.microsoft.com/en-us/windows/console/setconsolemode
        crossterm::ansi_support::supports_ansi()
    }

    #[cfg(not(windows))]
    fn supports_ansi() -> bool {
        true
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
) -> Result<i32, Error> {
    if let Some(turbo_state) = LocalTurboState::infer(&repo_state.root) {
        try_check_for_updates(&shim_args, turbo_state.version());

        if turbo_state.local_is_self() {
            env::set_var(
                cli::INVOCATION_DIR_ENV_VAR,
                shim_args.invocation_dir.as_path(),
            );
            debug!("Currently running turbo is local turbo.");
            Ok(cli::run(Some(repo_state), subscriber, ui)?)
        } else {
            spawn_local_turbo(&repo_state, turbo_state, shim_args)
        }
    } else {
        let version = get_version();
        try_check_for_updates(&shim_args, version);
        // cli::run checks for this env var, rather than an arg, so that we can support
        // calling old versions without passing unknown flags.
        env::set_var(
            cli::INVOCATION_DIR_ENV_VAR,
            shim_args.invocation_dir.as_path(),
        );
        debug!("Running command as global turbo");
        let should_warn_on_global = env::var(TURBO_GLOBAL_WARNING_DISABLED)
            .map_or(true, |disable| !matches!(disable.as_str(), "1" | "true"));
        if should_warn_on_global {
            warn!("No locally installed `turbo` found. Using version: {version}.");
        }
        Ok(cli::run(Some(repo_state), subscriber, ui)?)
    }
}

fn spawn_local_turbo(
    repo_state: &RepoState,
    local_turbo_state: LocalTurboState,
    mut shim_args: ShimArgs,
) -> Result<i32, Error> {
    let local_turbo_path = fs_canonicalize(local_turbo_state.binary()).map_err(|_| {
        Error::LocalTurboPath(local_turbo_state.binary().to_string_lossy().to_string())
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
        let _ = display_update_check(
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

pub fn run() -> Result<i32, Error> {
    let args = ShimArgs::parse()?;
    let ui = args.ui();
    if ui.should_strip_ansi {
        // Let's not crash just because we failed to set up the hook
        let _ = miette::set_hook(Box::new(|_| {
            Box::new(
                miette::MietteHandlerOpts::new()
                    .color(false)
                    .unicode(false)
                    .build(),
            )
        }));
    }
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
    use miette::SourceSpan;
    use test_case::test_case;

    use crate::shim::ShimArgs;

    #[test_case(vec![3], vec!["--graph", "foo", "--cwd", "apple"], vec![(18, 5).into()])]
    #[test_case(vec![0], vec!["--graph", "foo", "--cwd"], vec![(0, 7).into()])]
    #[test_case(vec![0, 2], vec!["--graph", "foo", "--cwd"], vec![(0, 7).into(), (12, 5).into()])]
    #[test_case(vec![], vec!["--cwd"], vec![])]
    fn test_get_indices_in_arg_string(
        arg_indices: Vec<usize>,
        args: Vec<&'static str>,
        expected_indices_in_arg_string: Vec<SourceSpan>,
    ) {
        let (indices_in_args_string, _) =
            ShimArgs::get_spans_in_args_string(arg_indices, args.into_iter());
        assert_eq!(indices_in_args_string, expected_indices_in_arg_string);
    }
}
