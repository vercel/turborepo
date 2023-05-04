use std::{
    env::{self, current_dir},
    io, mem,
    path::{Path, PathBuf},
    process,
};

use anyhow::{anyhow, Result};
use clap::{ArgAction, CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{generate, Shell};
use dunce::canonicalize as fs_canonicalize;
use serde::Serialize;
use tracing::{debug, error};
use turbopath::AbsoluteSystemPathBuf;

use crate::{
    commands::{bin, daemon, link, login, logout, unlink, CommandBase},
    get_version,
    shim::{RepoMode, RepoState},
    ui::UI,
    Payload,
};

// Global turbo sets this environment variable to its cwd so that local
// turbo can use it for package inference.
pub const INVOCATION_DIR_ENV_VAR: &str = "TURBO_INVOCATION_DIR";

#[derive(Copy, Clone, Debug, PartialEq, Serialize, ValueEnum)]
pub enum OutputLogsMode {
    #[serde(rename = "full")]
    Full,
    #[serde(rename = "none")]
    None,
    #[serde(rename = "hash-only")]
    HashOnly,
    #[serde(rename = "new-only")]
    NewOnly,
    #[serde(rename = "errors-only")]
    ErrorsOnly,
}

impl Default for OutputLogsMode {
    fn default() -> Self {
        Self::Full
    }
}

// NOTE: These *must* be kept in sync with the `_dryRunJSONValue`
// and `_dryRunTextValue` constants in run.go.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, ValueEnum)]
pub enum DryRunMode {
    Text,
    Json,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Serialize, ValueEnum)]
pub enum EnvMode {
    #[default]
    Infer,
    Loose,
    Strict,
}

#[derive(Parser, Clone, Default, Debug, PartialEq, Serialize)]
#[clap(author, about = "The build system that makes ship happen", long_about = None)]
#[clap(disable_help_subcommand = true)]
#[clap(disable_version_flag = true)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    #[clap(long, global = true)]
    #[serde(skip)]
    pub version: bool,
    #[clap(long, global = true)]
    #[serde(skip)]
    /// Skip any attempts to infer which version of Turbo the project is
    /// configured to use
    pub skip_infer: bool,
    /// Disable the turbo update notification
    #[clap(long, global = true)]
    #[serde(skip)]
    pub no_update_notifier: bool,
    /// Override the endpoint for API calls
    #[clap(long, global = true, value_parser)]
    pub api: Option<String>,
    /// Force color usage in the terminal
    #[clap(long, global = true)]
    pub color: bool,
    /// Specify a file to save a cpu profile
    #[clap(long = "cpuprofile", global = true, value_parser)]
    pub cpu_profile: Option<String>,
    /// The directory in which to run turbo
    #[clap(long, global = true, value_parser)]
    pub cwd: Option<PathBuf>,
    /// Specify a file to save a pprof heap profile
    #[clap(long, global = true, value_parser)]
    pub heap: Option<String>,
    /// Override the login endpoint
    #[clap(long, global = true, value_parser)]
    pub login: Option<String>,
    /// Suppress color usage in the terminal
    #[clap(long, global = true)]
    pub no_color: bool,
    /// When enabled, turbo will precede HTTP requests with an OPTIONS request
    /// for authorization
    #[clap(long, global = true)]
    pub preflight: bool,
    /// Set a timeout for all HTTP requests.
    #[clap(long, value_name = "TIMEOUT", global = true, value_parser)]
    pub remote_cache_timeout: Option<u64>,
    /// Set the team slug for API calls
    #[clap(long, global = true, value_parser)]
    pub team: Option<String>,
    /// Set the auth token for API calls
    #[clap(long, global = true, value_parser)]
    pub token: Option<String>,
    /// Specify a file to save a pprof trace
    #[clap(long, global = true, value_parser)]
    pub trace: Option<String>,
    /// verbosity
    #[clap(flatten)]
    pub verbosity: Verbosity,
    /// Force a check for a new version of turbo
    #[clap(long, global = true, hide = true)]
    #[serde(skip)]
    pub check_for_update: bool,
    #[clap(long = "__test-run", global = true, hide = true)]
    pub test_run: bool,
    #[clap(flatten, next_help_heading = "Run Arguments")]
    // We don't serialize this because by the time we're calling
    // Go, we've moved it to the command field as a Command::Run
    #[serde(skip)]
    pub run_args: Option<RunArgs>,
    #[clap(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Parser, Serialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(into = "u8")]
pub struct Verbosity {
    #[clap(
        long = "verbosity",
        global = true,
        conflicts_with = "v",
        value_name = "COUNT"
    )]
    /// Verbosity level
    pub verbosity: Option<u8>,
    #[clap(
        short = 'v',
        action = clap::ArgAction::Count,
        global = true,
        hide = true,
        conflicts_with = "verbosity"
    )]
    pub v: u8,
}

impl From<Verbosity> for u8 {
    fn from(val: Verbosity) -> Self {
        let Verbosity { verbosity, v } = val;
        verbosity.unwrap_or(v)
    }
}

#[derive(Subcommand, Copy, Clone, Debug, Serialize, PartialEq)]
#[serde(tag = "command")]
pub enum DaemonCommand {
    /// Restarts the turbo daemon
    Restart,
    /// Ensures that the turbo daemon is running
    Start,
    /// Reports the status of the turbo daemon
    Status {
        /// Pass --json to report status in JSON format
        #[clap(long)]
        json: bool,
    },
    /// Stops the turbo daemon
    Stop,
}

#[derive(Copy, Clone, Debug, PartialEq, Serialize, ValueEnum)]
pub enum LinkTarget {
    RemoteCache,
    Spaces,
}

impl Args {
    pub fn new() -> Result<Self> {
        let mut clap_args = match Args::try_parse() {
            Ok(args) => args,
            // Don't use error logger when displaying help text
            Err(e)
                if matches!(
                    e.kind(),
                    clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
                ) =>
            {
                let _ = e.print();
                process::exit(1);
            }
            Err(e) if e.use_stderr() => {
                let err_str = e.to_string();
                // A cleaner solution would be to implement our own clap::error::ErrorFormatter
                // but that would require copying the default formatter just to remove this
                // line: https://docs.rs/clap/latest/src/clap/error/format.rs.html#100
                error!(
                    "{}",
                    err_str.strip_prefix("error: ").unwrap_or(err_str.as_str())
                );
                process::exit(1);
            }
            // If the clap error shouldn't be printed to stderr it indicates help text
            Err(e) => {
                let _ = e.print();
                process::exit(0);
            }
        };
        // We have to override the --version flag because we use `get_version`
        // instead of a hard-coded version or the crate version
        if clap_args.version {
            println!("{}", get_version());
            process::exit(0);
        }

        if env::var("TEST_RUN").is_ok() {
            clap_args.test_run = true;
        }

        Ok(clap_args)
    }
}

/// Defines the subcommands for CLI. NOTE: If we change the commands in Go,
/// we must change these as well to avoid accidentally passing the
/// --single-package flag into non-build commands.
#[derive(Subcommand, Clone, Debug, Serialize, PartialEq)]
pub enum Command {
    // NOTE: Empty variants still have an empty struct attached so that serde serializes
    // them as `{ "Bin": {} }` instead of as `"Bin"`.
    /// Get the path to the Turbo binary
    Bin {},
    /// Generate the autocompletion script for the specified shell
    #[serde(skip)]
    Completion { shell: Shell },
    /// Runs the Turborepo background daemon
    Daemon {
        /// Set the idle timeout for turbod (default 4h0m0s)
        #[clap(long)]
        idle_time: Option<String>,
        #[clap(subcommand)]
        #[serde(flatten)]
        command: Option<DaemonCommand>,
    },
    /// Link your local directory to a Vercel organization and enable remote
    /// caching.
    Link {
        /// Do not create or modify .gitignore (default false)
        #[clap(long)]
        no_gitignore: bool,

        /// Specify what should be linked (default "remote cache")
        #[clap(long, value_enum, default_value_t = LinkTarget::RemoteCache)]
        target: LinkTarget,
    },
    /// Login to your Vercel account
    Login {
        #[clap(long = "sso-team")]
        sso_team: Option<String>,
    },
    /// Logout to your Vercel account
    Logout {},
    /// Prepare a subset of your monorepo.
    Prune {
        #[clap(long)]
        scope: Vec<String>,
        #[clap(long)]
        docker: bool,
        #[clap(long = "out-dir", default_value_t = String::from("out"), value_parser)]
        output_dir: String,
    },

    /// Run tasks across projects in your monorepo
    ///
    /// By default, turbo executes tasks in topological order (i.e.
    /// dependencies first) and then caches the results. Re-running commands for
    /// tasks already in the cache will skip re-execution and immediately move
    /// artifacts from the cache into the correct output folders (as if the task
    /// occurred again).
    ///
    /// Arguments passed after '--' will be passed through to the named tasks.
    Run(Box<RunArgs>),
    /// Unlink the current directory from your Vercel organization and disable
    /// Remote Caching
    Unlink {
        /// Specify what should be unlinked (default "remote cache")
        #[clap(long, value_enum, default_value_t = LinkTarget::RemoteCache)]
        target: LinkTarget,
    },
}

#[derive(Parser, Clone, Debug, Default, Serialize, PartialEq)]
pub struct RunArgs {
    /// Override the filesystem cache directory.
    #[clap(long)]
    pub cache_dir: Option<String>,
    /// Set the number of concurrent cache operations (default 10)
    #[clap(long, default_value_t = 10)]
    pub cache_workers: u32,
    /// Limit the concurrency of task execution. Use 1 for serial (i.e.
    /// one-at-a-time) execution.
    #[clap(long)]
    pub concurrency: Option<String>,
    /// Continue execution even if a task exits with an error or non-zero
    /// exit code. The default behavior is to bail
    #[clap(long = "continue")]
    pub continue_execution: bool,
    #[clap(alias = "dry", long = "dry-run", num_args = 0..=1, default_missing_value = "text")]
    pub dry_run: Option<DryRunMode>,
    /// Run turbo in single-package mode
    #[clap(long, global = true)]
    pub single_package: bool,
    /// Use the given selector to specify package(s) to act as
    /// entry points. The syntax mirrors pnpm's syntax, and
    /// additional documentation and examples can be found in
    /// turbo's documentation https://turbo.build/repo/docs/reference/command-line-reference#--filter
    #[clap(short = 'F', long, action = ArgAction::Append)]
    pub filter: Vec<String>,
    /// Ignore the existing cache (to force execution)
    #[clap(long, env = "TURBO_FORCE", default_missing_value = "true")]
    pub force: Option<Option<bool>>,
    /// Specify glob of global filesystem dependencies to be hashed. Useful
    /// for .env and files
    #[clap(long = "global-deps", action = ArgAction::Append)]
    pub global_deps: Vec<String>,
    /// Generate a graph of the task execution and output to a file when a
    /// filename is specified (.svg, .png, .jpg, .pdf, .json,
    /// .html). Outputs dot graph to stdout when if no filename is provided
    #[clap(long, num_args = 0..=1, default_missing_value = "")]
    pub graph: Option<String>,
    /// Environment variable mode.
    /// Loose passes the entire environment.
    /// Strict uses an allowlist specified in turbo.json.
    #[clap(long = "experimental-env-mode", default_value = "infer", num_args = 0..=1, default_missing_value = "infer", hide = true)]
    pub env_mode: EnvMode,
    /// Files to ignore when calculating changed files (i.e. --since).
    /// Supports globs.
    #[clap(long)]
    pub ignore: Vec<String>,
    /// Include the dependencies of tasks in execution.
    #[clap(long)]
    pub include_dependencies: bool,
    /// Avoid saving task results to the cache. Useful for development/watch
    /// tasks.
    #[clap(long)]
    pub no_cache: bool,
    /// Run without using turbo's daemon process
    #[clap(long)]
    pub no_daemon: bool,
    /// Exclude dependent task consumers from execution.
    #[clap(long)]
    pub no_deps: bool,
    /// Set type of process output logging. Use "full" to show
    /// all output. Use "hash-only" to show only turbo-computed
    /// task hashes. Use "new-only" to show only new output with
    /// only hashes for cached tasks. Use "none" to hide process
    /// output. (default full)
    #[clap(long, value_enum)]
    pub output_logs: Option<OutputLogsMode>,
    #[clap(long, hide = true)]
    pub only: bool,
    /// Execute all tasks in parallel.
    #[clap(long)]
    pub parallel: bool,
    #[clap(long, hide = true, default_missing_value = "")]
    pub pkg_inference_root: Option<String>,
    /// File to write turbo's performance profile output into.
    /// You can load the file up in chrome://tracing to see
    /// which parts of your build were slow.
    #[clap(long)]
    pub profile: Option<String>,
    /// Ignore the local filesystem cache for all tasks. Only
    /// allow reading and caching artifacts using the remote cache.
    #[clap(long)]
    pub remote_only: bool,
    /// Specify package(s) to act as entry points for task execution.
    /// Supports globs.
    #[clap(long)]
    pub scope: Vec<String>,
    /// Limit/Set scope to changed packages since a mergebase.
    /// This uses the git diff ${target_branch}... mechanism
    /// to identify which packages have changed.
    #[clap(long)]
    pub since: Option<String>,
    /// Generate a summary of the turbo run
    #[clap(long, env = "TURBO_RUN_SUMMARY", default_missing_value = "true")]
    pub summarize: Option<Option<bool>>,
    /// Use "none" to remove prefixes from task logs. Note that tasks running
    /// in parallel interleave their logs and prefix is the only way
    /// to identify which task produced a log.
    #[clap(long, value_enum)]
    pub log_prefix: Option<LogPrefix>,
    // NOTE: The following two are hidden because clap displays them in the help text incorrectly:
    // > Usage: turbo [OPTIONS] [TASKS]... [-- <FORWARDED_ARGS>...] [COMMAND]
    #[clap(hide = true)]
    pub tasks: Vec<String>,
    #[clap(last = true, hide = true)]
    pub pass_through_args: Vec<String>,

    // Pass a string to enable posting Run Summaries to Vercel
    #[clap(long, hide = true)]
    pub experimental_space_id: Option<String>,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Serialize)]
pub enum LogPrefix {
    #[serde(rename = "none")]
    None,
}

/// Runs the CLI by parsing arguments with clap, then either calling Rust code
/// directly or returning a payload for the Go code to use.
///
/// Scenarios:
/// 1. inference failed, we're running this global turbo. no repo state
/// 2. --skip-infer was passed, assume we're local turbo and run. no repo state
/// 3. There is no local turbo, we're running the global one. repo state exists
/// 4. turbo binary path is set, and it's this one. repo state exists
///
/// # Arguments
///
/// * `repo_state`: If we have done repository inference and NOT executed
/// local turbo, such as in the case where `TURBO_BINARY_PATH` is set,
/// we use it here to modify clap's arguments.
///
/// returns: Result<Payload, Error>
#[tokio::main]
pub async fn run(repo_state: Option<RepoState>) -> Result<Payload> {
    let mut cli_args = Args::new()?;
    // If there is no command, we set the command to `Command::Run` with
    // `self.parsed_args.run_args` as arguments.
    if cli_args.command.is_none() {
        if let Some(run_args) = mem::take(&mut cli_args.run_args) {
            cli_args.command = Some(Command::Run(Box::new(run_args)));
        } else {
            return Err(anyhow!("No command specified"));
        }
    };

    // If this is a run command, and we know the actual invocation path, set the
    // inference root, as long as the user hasn't overridden the cwd
    if cli_args.cwd.is_none() {
        if let Some(Command::Run(run_args)) = &mut cli_args.command {
            if let Ok(invocation_dir) = env::var(INVOCATION_DIR_ENV_VAR) {
                let invocation_path = Path::new(&invocation_dir);

                // If repo state doesn't exist, we're either local turbo running at the root
                // (current_dir), or inference failed If repo state does exist,
                // we're global turbo, and want to calculate package inference based on the repo
                // root
                let this_dir = current_dir()?;
                let repo_root = repo_state.as_ref().map(|r| &r.root).unwrap_or(&this_dir);
                if let Ok(relative_path) = invocation_path.strip_prefix(repo_root) {
                    debug!("pkg_inference_root set to \"{}\"", relative_path.display());
                    let utf8_path = relative_path
                        .to_str()
                        .ok_or_else(|| anyhow!("invalid utf8 path: {:?}", relative_path))?;
                    run_args.pkg_inference_root = Some(utf8_path.to_owned());
                }
            } else {
                debug!("{} not set", INVOCATION_DIR_ENV_VAR);
            }
        }
    }

    // Do this after the above, since we're now always setting cwd.
    if let Some(repo_state) = repo_state {
        if let Some(Command::Run(run_args)) = &mut cli_args.command {
            run_args.single_package = matches!(repo_state.mode, RepoMode::SinglePackage);
        }
        cli_args.cwd = Some(repo_state.root);
    }

    let repo_root = if let Some(cwd) = &cli_args.cwd {
        let canonical_cwd = fs_canonicalize(cwd)?;
        // Update on clap_args so that Go gets a canonical path.
        cli_args.cwd = Some(canonical_cwd.clone());
        canonical_cwd
    } else {
        current_dir()?
    };

    // a non-absolute repo root is a bug
    let repo_root = AbsoluteSystemPathBuf::new(repo_root).expect("repo_root is not absolute");

    let version = get_version();

    match cli_args.command.as_ref().unwrap() {
        Command::Bin { .. } => {
            bin::run()?;

            Ok(Payload::Rust(Ok(0)))
        }
        Command::Logout { .. } => {
            let mut base = CommandBase::new(cli_args, repo_root, version)?;
            logout::logout(&mut base)?;

            Ok(Payload::Rust(Ok(0)))
        }
        Command::Login { sso_team } => {
            if cli_args.test_run {
                println!("Login test run successful");
                return Ok(Payload::Rust(Ok(0)));
            }

            let sso_team = sso_team.clone();

            let mut base = CommandBase::new(cli_args, repo_root, version)?;

            if let Some(sso_team) = sso_team {
                login::sso_login(&mut base, &sso_team).await?;
            } else {
                login::login(&mut base).await?;
            }

            Ok(Payload::Rust(Ok(0)))
        }
        Command::Link {
            no_gitignore,
            target,
        } => {
            if cli_args.test_run {
                println!("Link test run successful");
                return Ok(Payload::Rust(Ok(0)));
            }

            let modify_gitignore = !*no_gitignore;
            let to = *target;
            let mut base = CommandBase::new(cli_args, repo_root, version)?;

            if let Err(err) = link::link(&mut base, modify_gitignore, to).await {
                error!("error: {}", err.to_string())
            }

            Ok(Payload::Rust(Ok(0)))
        }
        Command::Unlink { target } => {
            if cli_args.test_run {
                println!("Unlink test run successful");
                return Ok(Payload::Rust(Ok(0)));
            }

            let from = *target;
            let mut base = CommandBase::new(cli_args, repo_root, version)?;

            unlink::unlink(&mut base, from)?;

            Ok(Payload::Rust(Ok(0)))
        }
        Command::Daemon {
            command: Some(command),
            ..
        } => {
            let command = *command;
            let base = CommandBase::new(cli_args, repo_root, version)?;
            daemon::main(&command, &base).await?;
            Ok(Payload::Rust(Ok(0)))
        },
        Command::Run(args) => {
            if args.tasks.is_empty() {
                return Err(anyhow!("at least one task must be specified"));
            }
            let base = CommandBase::new(cli_args, repo_root, version)?;
            Ok(Payload::Go(Box::new(base)))
        }
        Command::Prune { .. }
        // the daemon itself still delegates to Go
        | Command::Daemon { .. } => {
            let base = CommandBase::new(cli_args, repo_root, version)?;
            Ok(Payload::Go(Box::new(base)))
        },
        Command::Completion { shell } => {
            generate(*shell, &mut Args::command(), "turbo", &mut io::stdout());

            Ok(Payload::Rust(Ok(0)))
        }
    }
}

impl Args {
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

#[cfg(test)]
mod test {
    use std::path::PathBuf;

    use clap::Parser;
    use itertools::Itertools;
    use pretty_assertions::assert_eq;

    struct CommandTestCase {
        command: &'static str,
        command_args: Vec<Vec<&'static str>>,
        global_args: Vec<Vec<&'static str>>,
        expected_output: Args,
    }

    fn get_default_run_args() -> RunArgs {
        RunArgs {
            cache_workers: 10,
            output_logs: None,
            ..RunArgs::default()
        }
    }

    impl CommandTestCase {
        fn test(&self) {
            let permutations = self.create_all_arg_permutations();
            for command in permutations {
                assert_eq!(Args::try_parse_from(command).unwrap(), self.expected_output)
            }
        }

        fn create_all_arg_permutations(&self) -> Vec<Vec<&'static str>> {
            let mut permutations = Vec::new();
            let mut global_args = vec![vec![self.command]];
            global_args.extend(self.global_args.clone());
            let global_args_len = global_args.len();
            let command_args_len = self.command_args.len();

            // Iterate through all the different permutations of args
            for global_args_permutation in global_args.into_iter().permutations(global_args_len) {
                let command_args = self.command_args.clone();
                for command_args_permutation in
                    command_args.into_iter().permutations(command_args_len)
                {
                    let mut command = vec![vec!["turbo"]];
                    command.extend(global_args_permutation.clone());
                    command.extend(command_args_permutation);
                    permutations.push(command.into_iter().flatten().collect())
                }
            }

            permutations
        }
    }

    use anyhow::Result;

    use crate::cli::{Args, Command, DryRunMode, EnvMode, OutputLogsMode, RunArgs, Verbosity};

    #[test]
    fn test_parse_run() -> Result<()> {
        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build"]).unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build"]).unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    env_mode: EnvMode::Infer,
                    ..get_default_run_args()
                }))),
                ..Args::default()
            },
            "env_mode: default infer"
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--experimental-env-mode"]).unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    env_mode: EnvMode::Infer,
                    ..get_default_run_args()
                }))),
                ..Args::default()
            },
            "env_mode: not fully-specified"
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--experimental-env-mode", "infer"])
                .unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    env_mode: EnvMode::Infer,
                    ..get_default_run_args()
                }))),
                ..Args::default()
            },
            "env_mode: specified infer"
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--experimental-env-mode", "loose"])
                .unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    env_mode: EnvMode::Loose,
                    ..get_default_run_args()
                }))),
                ..Args::default()
            },
            "env_mode: specified loose"
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--experimental-env-mode", "strict"])
                .unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    env_mode: EnvMode::Strict,
                    ..get_default_run_args()
                }))),
                ..Args::default()
            },
            "env_mode: specified strict"
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "lint", "test"]).unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string(), "lint".to_string(), "test".to_string()],
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--cache-dir", "foobar"]).unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    cache_dir: Some("foobar".to_string()),
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--cache-workers", "100"]).unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    cache_workers: 100,
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--concurrency", "20"]).unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    concurrency: Some("20".to_string()),
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--continue"]).unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    continue_execution: true,
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--dry-run"]).unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    dry_run: Some(DryRunMode::Text),
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--dry-run", "json"]).unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    dry_run: Some(DryRunMode::Json),
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from([
                "turbo", "run", "build", "--filter", "water", "--filter", "earth", "--filter",
                "fire", "--filter", "air"
            ])
            .unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    filter: vec![
                        "water".to_string(),
                        "earth".to_string(),
                        "fire".to_string(),
                        "air".to_string()
                    ],
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from([
                "turbo", "run", "build", "-F", "water", "-F", "earth", "-F", "fire", "-F", "air"
            ])
            .unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    filter: vec![
                        "water".to_string(),
                        "earth".to_string(),
                        "fire".to_string(),
                        "air".to_string()
                    ],
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from([
                "turbo", "run", "build", "--filter", "water", "-F", "earth", "--filter", "fire",
                "-F", "air"
            ])
            .unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    filter: vec![
                        "water".to_string(),
                        "earth".to_string(),
                        "fire".to_string(),
                        "air".to_string()
                    ],
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--force"]).unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    force: Some(Some(true)),
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--global-deps", ".env"]).unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    global_deps: vec![".env".to_string()],
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from([
                "turbo",
                "run",
                "build",
                "--global-deps",
                ".env",
                "--global-deps",
                ".env.development"
            ])
            .unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    global_deps: vec![".env".to_string(), ".env.development".to_string()],
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--graph"]).unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    graph: Some("".to_string()),
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--graph", "out.html"]).unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    graph: Some("out.html".to_string()),
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--ignore", "foo.js"]).unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    ignore: vec!["foo.js".to_string()],
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from([
                "turbo", "run", "build", "--ignore", "foo.js", "--ignore", "bar.js"
            ])
            .unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    ignore: vec!["foo.js".to_string(), "bar.js".to_string()],
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--include-dependencies"]).unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    include_dependencies: true,
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--no-cache"]).unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    no_cache: true,
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--no-daemon"]).unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    no_daemon: true,
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--no-deps"]).unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    no_deps: true,
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        // Test that ouput-logs is not serialized by default
        assert_eq!(
            serde_json::to_string(&Args::try_parse_from(["turbo", "run", "build"]).unwrap())?
                .contains("\"output_logs\":null"),
            true
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--output-logs", "full"]).unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    output_logs: Some(OutputLogsMode::Full),
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--output-logs", "none"]).unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    output_logs: Some(OutputLogsMode::None),
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--output-logs", "hash-only"]).unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    output_logs: Some(OutputLogsMode::HashOnly),
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--parallel"]).unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    parallel: true,
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--profile", "profile_out"]).unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    profile: Some("profile_out".to_string()),
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--remote-only"]).unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    remote_only: true,
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--scope", "foo", "--scope", "bar"])
                .unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    scope: vec!["foo".to_string(), "bar".to_string()],
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--since", "foo"]).unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    since: Some("foo".to_string()),
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "build"]).unwrap(),
            Args {
                run_args: Some(RunArgs {
                    tasks: vec!["build".to_string()],
                    ..get_default_run_args()
                }),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "build", "lint", "test"]).unwrap(),
            Args {
                run_args: Some(RunArgs {
                    tasks: vec!["build".to_string(), "lint".to_string(), "test".to_string()],
                    ..get_default_run_args()
                }),
                ..Args::default()
            }
        );

        Ok(())
    }

    #[test]
    fn test_parse_bin() {
        assert_eq!(
            Args::try_parse_from(["turbo", "bin"]).unwrap(),
            Args {
                command: Some(Command::Bin {}),
                ..Args::default()
            }
        );

        CommandTestCase {
            command: "bin",
            command_args: vec![],
            global_args: vec![vec!["--cwd", "../examples/with-yarn"]],
            expected_output: Args {
                command: Some(Command::Bin {}),
                cwd: Some(PathBuf::from("../examples/with-yarn")),
                ..Args::default()
            },
        }
        .test();
    }

    #[test]
    fn test_parse_login() {
        assert_eq!(
            Args::try_parse_from(["turbo", "login"]).unwrap(),
            Args {
                command: Some(Command::Login { sso_team: None }),
                ..Args::default()
            }
        );

        CommandTestCase {
            command: "login",
            command_args: vec![],
            global_args: vec![vec!["--cwd", "../examples/with-yarn"]],
            expected_output: Args {
                command: Some(Command::Login { sso_team: None }),
                cwd: Some(PathBuf::from("../examples/with-yarn")),
                ..Args::default()
            },
        }
        .test();

        CommandTestCase {
            command: "login",
            command_args: vec![vec!["--sso-team", "my-team"]],
            global_args: vec![vec!["--cwd", "../examples/with-yarn"]],
            expected_output: Args {
                command: Some(Command::Login {
                    sso_team: Some("my-team".to_string()),
                }),
                cwd: Some(PathBuf::from("../examples/with-yarn")),
                ..Args::default()
            },
        }
        .test();
    }

    #[test]
    fn test_parse_logout() {
        assert_eq!(
            Args::try_parse_from(["turbo", "logout"]).unwrap(),
            Args {
                command: Some(Command::Logout {}),
                ..Args::default()
            }
        );

        CommandTestCase {
            command: "logout",
            command_args: vec![],
            global_args: vec![vec!["--cwd", "../examples/with-yarn"]],
            expected_output: Args {
                command: Some(Command::Logout {}),
                cwd: Some(PathBuf::from("../examples/with-yarn")),
                ..Args::default()
            },
        }
        .test();
    }

    #[test]
    fn test_parse_unlink() {
        assert_eq!(
            Args::try_parse_from(["turbo", "unlink"]).unwrap(),
            Args {
                command: Some(Command::Unlink {
                    target: crate::cli::LinkTarget::RemoteCache
                }),
                ..Args::default()
            }
        );

        CommandTestCase {
            command: "unlink",
            command_args: vec![],
            global_args: vec![vec!["--cwd", "../examples/with-yarn"]],
            expected_output: Args {
                command: Some(Command::Unlink {
                    target: crate::cli::LinkTarget::RemoteCache,
                }),
                cwd: Some(PathBuf::from("../examples/with-yarn")),
                ..Args::default()
            },
        }
        .test();
    }

    #[test]
    fn test_parse_prune() {
        let default_prune = Command::Prune {
            scope: Vec::new(),
            docker: false,
            output_dir: "out".to_string(),
        };

        assert_eq!(
            Args::try_parse_from(["turbo", "prune"]).unwrap(),
            Args {
                command: Some(default_prune.clone()),
                ..Args::default()
            }
        );

        CommandTestCase {
            command: "prune",
            command_args: vec![],
            global_args: vec![vec!["--cwd", "../examples/with-yarn"]],
            expected_output: Args {
                command: Some(default_prune),
                cwd: Some(PathBuf::from("../examples/with-yarn")),
                ..Args::default()
            },
        }
        .test();

        assert_eq!(
            Args::try_parse_from(["turbo", "prune", "--scope", "bar"]).unwrap(),
            Args {
                command: Some(Command::Prune {
                    scope: vec!["bar".to_string()],
                    docker: false,
                    output_dir: "out".to_string(),
                }),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "prune", "--docker"]).unwrap(),
            Args {
                command: Some(Command::Prune {
                    scope: Vec::new(),
                    docker: true,
                    output_dir: "out".to_string(),
                }),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "prune", "--out-dir", "dist"]).unwrap(),
            Args {
                command: Some(Command::Prune {
                    scope: Vec::new(),
                    docker: false,
                    output_dir: "dist".to_string(),
                }),
                ..Args::default()
            }
        );

        CommandTestCase {
            command: "prune",
            command_args: vec![vec!["--out-dir", "dist"], vec!["--docker"]],
            global_args: vec![],
            expected_output: Args {
                command: Some(Command::Prune {
                    scope: Vec::new(),
                    docker: true,
                    output_dir: "dist".to_string(),
                }),
                ..Args::default()
            },
        }
        .test();

        CommandTestCase {
            command: "prune",
            command_args: vec![vec!["--out-dir", "dist"], vec!["--docker"]],
            global_args: vec![vec!["--cwd", "../examples/with-yarn"]],
            expected_output: Args {
                command: Some(Command::Prune {
                    scope: Vec::new(),
                    docker: true,
                    output_dir: "dist".to_string(),
                }),
                cwd: Some(PathBuf::from("../examples/with-yarn")),
                ..Args::default()
            },
        }
        .test();

        CommandTestCase {
            command: "prune",
            command_args: vec![
                vec!["--out-dir", "dist"],
                vec!["--docker"],
                vec!["--scope", "foo"],
            ],
            global_args: vec![],
            expected_output: Args {
                command: Some(Command::Prune {
                    scope: vec!["foo".to_string()],
                    docker: true,
                    output_dir: "dist".to_string(),
                }),
                ..Args::default()
            },
        }
        .test();
    }

    #[test]
    fn test_pass_through_args() {
        assert_eq!(
            Args::try_parse_from(["turbo", "run", "build", "--", "--script-arg=42"]).unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    pass_through_args: vec!["--script-arg=42".to_string()],
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from([
                "turbo",
                "run",
                "build",
                "--",
                "--script-arg=42",
                "--foo",
                "--bar",
                "bat"
            ])
            .unwrap(),
            Args {
                command: Some(Command::Run(Box::new(RunArgs {
                    tasks: vec!["build".to_string()],
                    pass_through_args: vec![
                        "--script-arg=42".to_string(),
                        "--foo".to_string(),
                        "--bar".to_string(),
                        "bat".to_string()
                    ],
                    ..get_default_run_args()
                }))),
                ..Args::default()
            }
        );
    }

    #[test]
    fn test_verbosity_serialization() -> Result<(), serde_json::Error> {
        assert_eq!(
            serde_json::to_string(&Verbosity {
                verbosity: None,
                v: 0
            })?,
            "0"
        );
        assert_eq!(
            serde_json::to_string(&Verbosity {
                verbosity: Some(3),
                v: 0
            })?,
            "3"
        );
        assert_eq!(
            serde_json::to_string(&Verbosity {
                verbosity: None,
                v: 3
            })?,
            "3"
        );
        Ok(())
    }
}
