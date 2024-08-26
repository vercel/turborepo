use std::{backtrace::Backtrace, env, fmt, fmt::Display, io, mem, process};

use biome_deserialize_macros::Deserializable;
use camino::{Utf8Path, Utf8PathBuf};
use clap::{
    builder::NonEmptyStringValueParser, ArgAction, ArgGroup, CommandFactory, Parser, Subcommand,
    ValueEnum,
};
use clap_complete::{generate, Shell};
pub use error::Error;
use serde::{Deserialize, Serialize};
use tracing::{debug, error, log::warn};
use turbopath::AbsoluteSystemPathBuf;
use turborepo_api_client::AnonAPIClient;
use turborepo_repository::inference::{RepoMode, RepoState};
use turborepo_telemetry::{
    events::{command::CommandEventBuilder, generic::GenericEventBuilder, EventBuilder, EventType},
    init_telemetry, track_usage, TelemetryHandle,
};
use turborepo_ui::{ColorConfig, GREY};

use crate::{
    cli::error::print_potential_tasks,
    commands::{
        bin, config, daemon, generate, link, login, logout, ls, prune, query, run, scan, telemetry,
        unlink, CommandBase,
    },
    get_version,
    run::watch::WatchClient,
    shim::TurboState,
    tracing::TurboSubscriber,
    turbo_json::UIMode,
};

mod error;

// Global turbo sets this environment variable to its cwd so that local
// turbo can use it for package inference.
pub const INVOCATION_DIR_ENV_VAR: &str = "TURBO_INVOCATION_DIR";

// Default value for the --cache-workers argument
const DEFAULT_NUM_WORKERS: u32 = 10;
const SUPPORTED_GRAPH_FILE_EXTENSIONS: [&str; 8] =
    ["svg", "png", "jpg", "pdf", "json", "html", "mermaid", "dot"];

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum, Deserializable, Serialize)]
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

impl Display for OutputLogsMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            OutputLogsMode::Full => "full",
            OutputLogsMode::None => "none",
            OutputLogsMode::HashOnly => "hash-only",
            OutputLogsMode::NewOnly => "new-only",
            OutputLogsMode::ErrorsOnly => "errors-only",
        })
    }
}

impl From<OutputLogsMode> for turborepo_ui::tui::event::OutputLogs {
    fn from(value: OutputLogsMode) -> Self {
        match value {
            OutputLogsMode::Full => turborepo_ui::tui::event::OutputLogs::Full,
            OutputLogsMode::None => turborepo_ui::tui::event::OutputLogs::None,
            OutputLogsMode::HashOnly => turborepo_ui::tui::event::OutputLogs::HashOnly,
            OutputLogsMode::NewOnly => turborepo_ui::tui::event::OutputLogs::NewOnly,
            OutputLogsMode::ErrorsOnly => turborepo_ui::tui::event::OutputLogs::ErrorsOnly,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Serialize, ValueEnum)]
pub enum LogOrder {
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "stream")]
    Stream,
    #[serde(rename = "grouped")]
    Grouped,
}

impl Default for LogOrder {
    fn default() -> Self {
        Self::Auto
    }
}

impl Display for LogOrder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LogOrder::Auto => "auto",
            LogOrder::Stream => "stream",
            LogOrder::Grouped => "grouped",
        })
    }
}

impl LogOrder {
    pub fn compatible_with_tui(&self) -> bool {
        // If the user requested a specific order to the logs, then this isn't
        // compatible with the TUI and means we cannot use it.
        matches!(self, Self::Auto)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, ValueEnum)]
pub enum DryRunMode {
    Text,
    Json,
}

impl Display for DryRunMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DryRunMode::Text => "text",
            DryRunMode::Json => "json",
        })
    }
}

#[derive(
    Copy, Clone, Debug, Default, PartialEq, Serialize, ValueEnum, Deserialize, Eq, Deserializable,
)]
#[serde(rename_all = "lowercase")]
pub enum EnvMode {
    Loose,
    #[default]
    Strict,
}

impl fmt::Display for EnvMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            EnvMode::Loose => "loose",
            EnvMode::Strict => "strict",
        })
    }
}

#[derive(Parser, Clone, Default, Debug, PartialEq)]
#[clap(author, about = "The build system that makes ship happen", long_about = None)]
#[clap(disable_help_subcommand = true)]
#[clap(disable_version_flag = true)]
#[clap(arg_required_else_help = true)]
#[command(name = "turbo")]
pub struct Args {
    #[clap(long, global = true)]
    pub version: bool,
    #[clap(long, global = true)]
    /// Skip any attempts to infer which version of Turbo the project is
    /// configured to use
    pub skip_infer: bool,
    /// Disable the turbo update notification
    #[clap(long, global = true)]
    pub no_update_notifier: bool,
    /// Override the endpoint for API calls
    #[clap(long, global = true, value_parser)]
    pub api: Option<String>,
    /// Force color usage in the terminal
    #[clap(long, global = true)]
    pub color: bool,
    /// The directory in which to run turbo
    #[clap(long, global = true, value_parser)]
    pub cwd: Option<Utf8PathBuf>,
    /// Specify a file to save a pprof heap profile
    #[clap(long, global = true, value_parser)]
    pub heap: Option<String>,
    /// Specify whether to use the streaming UI or TUI
    #[clap(long, global = true, value_enum)]
    pub ui: Option<UIMode>,
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
    pub check_for_update: bool,
    #[clap(long = "__test-run", global = true, hide = true)]
    pub test_run: bool,
    /// Allow for missing `packageManager` in `package.json`.
    ///
    /// `turbo` will use hints from codebase to guess which package manager
    /// should be used.
    #[clap(long, global = true)]
    pub dangerously_disable_package_manager_check: bool,
    #[clap(flatten, next_help_heading = "Run Arguments")]
    pub run_args: Option<RunArgs>,
    // This should be inside `RunArgs` but clap currently has a bug
    // around nested flattened optional args: https://github.com/clap-rs/clap/issues/4697
    #[clap(flatten)]
    pub execution_args: Option<ExecutionArgs>,
    #[clap(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Parser, Clone, Copy, PartialEq, Eq, Default)]
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

#[derive(Subcommand, Copy, Clone, Debug, PartialEq)]
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
    /// Stops the turbo daemon if it is already running, and removes any stale
    /// daemon state
    Clean {
        /// Clean
        #[clap(long, default_value_t = true)]
        clean_logs: bool,
    },
    /// Shows the daemon logs
    Logs,
}

#[derive(Copy, Clone, Debug, Default, ValueEnum, Serialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    /// Output in a human-readable format
    #[default]
    Pretty,
    /// Output in JSON format for direct parsing
    Json,
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            OutputFormat::Pretty => "pretty",
            OutputFormat::Json => "json",
        })
    }
}

#[derive(Subcommand, Copy, Clone, Debug, PartialEq)]
pub enum TelemetryCommand {
    /// Enables anonymous telemetry
    Enable,
    /// Disables anonymous telemetry
    Disable,
    /// Reports the status of telemetry
    Status,
}

#[derive(Copy, Clone, Debug, PartialEq, ValueEnum)]
pub enum LinkTarget {
    RemoteCache,
    Spaces,
}

impl Args {
    pub fn new() -> Self {
        // We always pass --single-package in from the shim.
        // We need to omit it, and then add it in for run.
        let arg_separator_position = env::args_os().position(|input_token| input_token == "--");

        let single_package_position =
            env::args_os().position(|input_token| input_token == "--single-package");

        let is_single_package = match (arg_separator_position, single_package_position) {
            (_, None) => false,
            (None, Some(_)) => true,
            (Some(arg_separator_position), Some(single_package_position)) => {
                single_package_position < arg_separator_position
            }
        };

        // Clap supports arbitrary iterators as input.
        // We can remove all instances of --single-package
        let single_package_free = std::env::args_os()
            .enumerate()
            .filter(|(index, input_token)| {
                arg_separator_position
                    .is_some_and(|arg_separator_position| index > &arg_separator_position)
                    || input_token != "--single-package"
            })
            .map(|(_, input_token)| input_token);

        let mut clap_args = match Args::try_parse_from(single_package_free) {
            Ok(mut args) => {
                // And then only add them back in when we're in `run`.
                // The value can appear in two places in the struct.
                // We defensively attempt to set both.
                if let Some(ref mut execution_args) = args.execution_args {
                    execution_args.single_package = is_single_package
                }

                if let Some(Command::Run {
                    run_args: _,
                    ref mut execution_args,
                }) = args.command
                {
                    execution_args.single_package = is_single_package;
                }

                args
            }
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

        clap_args
    }

    pub fn get_tasks(&self) -> &[String] {
        match &self.command {
            Some(Command::Run {
                run_args: _,
                execution_args: box ExecutionArgs { tasks, .. },
            }) => tasks,
            _ => self
                .execution_args
                .as_ref()
                .map(|execution_args| execution_args.tasks.as_slice())
                .unwrap_or(&[]),
        }
    }

    pub fn track(&self, tel: &GenericEventBuilder) {
        // track usage only
        track_usage!(tel, self.skip_infer, |val| val);
        track_usage!(tel, self.no_update_notifier, |val| val);
        track_usage!(tel, self.color, |val| val);
        track_usage!(tel, self.no_color, |val| val);
        track_usage!(tel, self.preflight, |val| val);
        track_usage!(tel, &self.login, Option::is_some);
        track_usage!(tel, &self.cwd, Option::is_some);
        track_usage!(tel, &self.heap, Option::is_some);
        track_usage!(tel, &self.team, Option::is_some);
        track_usage!(tel, &self.token, Option::is_some);
        track_usage!(tel, &self.trace, Option::is_some);
        track_usage!(tel, &self.api, Option::is_some);

        // track values
        if let Some(remote_cache_timeout) = self.remote_cache_timeout {
            tel.track_arg_value(
                "remote-cache-timeout",
                remote_cache_timeout,
                turborepo_telemetry::events::EventType::NonSensitive,
            );
        }
        if self.verbosity.v > 0 {
            tel.track_arg_value(
                "v",
                self.verbosity.v,
                turborepo_telemetry::events::EventType::NonSensitive,
            );
        }
        if let Some(verbosity) = self.verbosity.verbosity {
            tel.track_arg_value(
                "verbosity",
                verbosity,
                turborepo_telemetry::events::EventType::NonSensitive,
            );
        }
    }
}

/// Defines the subcommands for CLI. NOTE: If we change the commands in Go,
/// we must change these as well to avoid accidentally passing the
/// --single-package flag into non-build commands.
#[derive(Subcommand, Clone, Debug, PartialEq)]
pub enum Command {
    /// Get the path to the Turbo binary
    Bin,
    /// Generate the autocompletion script for the specified shell
    Completion {
        shell: Shell,
    },
    /// Runs the Turborepo background daemon
    Daemon {
        /// Set the idle timeout for turbod
        #[clap(long, default_value_t = String::from("4h0m0s"))]
        idle_time: String,
        #[clap(subcommand)]
        command: Option<DaemonCommand>,
    },
    /// Generate a new app / package
    #[clap(aliases = ["g", "gen"])]
    Generate {
        #[clap(long, default_value_t = String::from("latest"), hide = true)]
        tag: String,
        /// The name of the generator to run
        generator_name: Option<String>,
        /// Generator configuration file
        #[clap(short = 'c', long)]
        config: Option<String>,
        /// The root of your repository (default: directory with root
        /// turbo.json)
        #[clap(short = 'r', long)]
        root: Option<String>,
        /// Answers passed directly to generator
        #[clap(short = 'a', long, num_args = 1..)]
        args: Vec<String>,

        #[clap(subcommand)]
        command: Option<Box<GenerateCommand>>,
    },
    /// Enable or disable anonymous telemetry
    Telemetry {
        #[clap(subcommand)]
        command: Option<TelemetryCommand>,
    },
    /// Turbo your monorepo by running a number of 'repo lints' to
    /// identify common issues, suggest fixes, and improve performance.
    Scan,
    #[clap(hide = true)]
    Config,
    /// EXPERIMENTAL: List packages in your monorepo.
    Ls {
        /// Show only packages that are affected by changes between
        /// the current branch and `main`
        #[clap(long, group = "scope-filter-group")]
        affected: bool,
        /// Use the given selector to specify package(s) to act as
        /// entry points. The syntax mirrors pnpm's syntax, and
        /// additional documentation and examples can be found in
        /// turbo's documentation https://turbo.build/repo/docs/reference/command-line-reference/run#--filter
        #[clap(short = 'F', long, group = "scope-filter-group")]
        filter: Vec<String>,
        /// Get insight into a specific package, such as
        /// its dependencies and tasks
        packages: Vec<String>,
        /// Output format
        #[clap(long, value_enum)]
        output: Option<OutputFormat>,
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
        /// Force a login to receive a new token. Will overwrite any existing
        /// tokens for the given login url.
        #[clap(long = "force", short = 'f')]
        force: bool,
    },
    /// Logout to your Vercel account
    Logout {
        /// Invalidate the token on the server
        #[clap(long)]
        invalidate: bool,
    },
    /// Prepare a subset of your monorepo.
    Prune {
        #[clap(hide = true, long)]
        scope: Option<Vec<String>>,
        /// Workspaces that should be included in the subset
        #[clap(
            required_unless_present("scope"),
            conflicts_with("scope"),
            value_name = "SCOPE"
        )]
        scope_arg: Option<Vec<String>>,
        #[clap(long)]
        docker: bool,
        #[clap(long = "out-dir", default_value_t = String::from(prune::DEFAULT_OUTPUT_DIR), value_parser)]
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
    Run {
        #[clap(flatten)]
        run_args: Box<RunArgs>,
        #[clap(flatten)]
        execution_args: Box<ExecutionArgs>,
    },
    /// Query your monorepo using GraphQL. If no query is provided, spins up a
    /// GraphQL server with GraphiQL.
    #[clap(hide = true)]
    Query {
        /// The query to run, either a file path or a query string
        query: Option<String>,
    },
    Watch(Box<ExecutionArgs>),
    /// Unlink the current directory from your Vercel organization and disable
    /// Remote Caching
    Unlink {
        /// Specify what should be unlinked (default "remote cache")
        #[clap(long, value_enum, default_value_t = LinkTarget::RemoteCache)]
        target: LinkTarget,
    },
}

#[derive(Parser, Clone, Debug, Default, Serialize, PartialEq)]
pub struct GenerateWorkspaceArgs {
    /// Name for the new workspace
    #[clap(short = 'n', long)]
    pub name: Option<String>,
    /// Generate an empty workspace
    #[clap(short = 'b', long, conflicts_with = "copy", default_value_t = true)]
    pub empty: bool,
    /// Generate a workspace using an existing workspace as a template. Can be
    /// the name of a local workspace within your monorepo, or a fully
    /// qualified GitHub URL with any branch and/or subdirectory
    #[clap(short = 'c', long, conflicts_with = "empty", num_args = 0..=1, default_missing_value = "")]
    pub copy: Option<String>,
    /// Where the new workspace should be created
    #[clap(short = 'd', long)]
    pub destination: Option<String>,
    /// The type of workspace to create
    #[clap(short = 't', long)]
    pub r#type: Option<String>,
    /// The root of your repository (default: directory with root turbo.json)
    #[clap(short = 'r', long)]
    pub root: Option<String>,
    /// In a rare case, your GitHub URL might contain a branch name with a slash
    /// (e.g. bug/fix-1) and the path to the example (e.g. foo/bar). In this
    /// case, you must specify the path to the example separately:
    /// --example-path foo/bar
    #[clap(short = 'p', long)]
    pub example_path: Option<String>,
    /// Do not filter available dependencies by the workspace type
    #[clap(long, default_value_t = false)]
    pub show_all_dependencies: bool,
}

#[derive(Parser, Clone, Debug, Default, PartialEq, Serialize)]
pub struct GeneratorCustomArgs {
    /// The name of the generator to run
    generator_name: Option<String>,
    /// Generator configuration file
    #[clap(short = 'c', long)]
    config: Option<String>,
    /// The root of your repository (default: directory with root
    /// turbo.json)
    #[clap(short = 'r', long)]
    root: Option<String>,
    /// Answers passed directly to generator
    #[clap(short = 'a', long, value_delimiter = ' ', num_args = 1..)]
    args: Vec<String>,
}

#[derive(Subcommand, Clone, Debug, PartialEq)]
pub enum GenerateCommand {
    /// Add a new package or app to your project
    #[clap(name = "workspace", alias = "w")]
    Workspace(GenerateWorkspaceArgs),
    #[clap(name = "run", alias = "r")]
    Run(GeneratorCustomArgs),
}

fn validate_graph_extension(s: &str) -> Result<String, String> {
    match s.is_empty() {
        true => Ok(s.to_string()),
        _ => match Utf8Path::new(s).extension() {
            Some(ext) if SUPPORTED_GRAPH_FILE_EXTENSIONS.contains(&ext) => Ok(s.to_string()),
            Some(ext) => Err(format!(
                "Invalid file extension: '{}'. Allowed extensions are: {:?}",
                ext, SUPPORTED_GRAPH_FILE_EXTENSIONS
            )),
            None => Err(format!(
                "The provided filename is missing a file extension. Allowed extensions are: {:?}",
                SUPPORTED_GRAPH_FILE_EXTENSIONS
            )),
        },
    }
}

fn path_non_empty(s: &str) -> Result<Utf8PathBuf, String> {
    if s.is_empty() {
        Err("path must not be empty".to_string())
    } else {
        Ok(Utf8Path::new(s).to_path_buf())
    }
}

/// Arguments used in run and watch
#[derive(Parser, Clone, Debug, Default, PartialEq)]
#[command(groups = [
ArgGroup::new("scope-filter-group").multiple(true).required(false),
])]
pub struct ExecutionArgs {
    /// Override the filesystem cache directory.
    #[clap(long, value_parser = path_non_empty, env = "TURBO_CACHE_DIR")]
    pub cache_dir: Option<Utf8PathBuf>,
    /// Limit the concurrency of task execution. Use 1 for serial (i.e.
    /// one-at-a-time) execution.
    #[clap(long)]
    pub concurrency: Option<String>,
    /// Continue execution even if a task exits with an error or non-zero
    /// exit code. The default behavior is to bail
    #[clap(long = "continue")]
    pub continue_execution: bool,
    /// Run turbo in single-package mode
    #[clap(long)]
    pub single_package: bool,
    /// Ignore the existing cache (to force execution)
    #[clap(long, env = "TURBO_FORCE", default_missing_value = "true")]
    pub force: Option<Option<bool>>,
    /// Specify whether or not to do framework inference for tasks
    #[clap(long, value_name = "BOOL", action = ArgAction::Set, default_value = "true", default_missing_value = "true", num_args = 0..=1)]
    pub framework_inference: bool,
    /// Specify glob of global filesystem dependencies to be hashed. Useful
    /// for .env and files
    #[clap(long = "global-deps", action = ArgAction::Append)]
    pub global_deps: Vec<String>,
    /// Environment variable mode.
    /// Use "loose" to pass the entire existing environment.
    /// Use "strict" to use an allowlist specified in turbo.json.
    #[clap(long = "env-mode", num_args = 0..=1, default_missing_value = "strict")]
    pub env_mode: Option<EnvMode>,
    /// Use the given selector to specify package(s) to act as
    /// entry points. The syntax mirrors pnpm's syntax, and
    /// additional documentation and examples can be found in
    /// turbo's documentation https://turbo.build/repo/docs/reference/command-line-reference/run#--filter
    #[clap(short = 'F', long, group = "scope-filter-group")]
    pub filter: Vec<String>,

    /// Run only tasks that are affected by changes between
    /// the current branch and `main`
    #[clap(long, group = "scope-filter-group", conflicts_with = "filter")]
    pub affected: bool,

    /// Set type of process output logging. Use "full" to show
    /// all output. Use "hash-only" to show only turbo-computed
    /// task hashes. Use "new-only" to show only new output with
    /// only hashes for cached tasks. Use "none" to hide process
    /// output. (default full)
    #[clap(long, value_enum)]
    pub output_logs: Option<OutputLogsMode>,
    /// Set type of task output order. Use "stream" to show
    /// output as soon as it is available. Use "grouped" to
    /// show output when a command has finished execution. Use "auto" to let
    /// turbo decide based on its own heuristics. (default auto)
    #[clap(long, env = "TURBO_LOG_ORDER", value_enum, default_value_t = LogOrder::Auto)]
    pub log_order: LogOrder,
    /// Only executes the tasks specified, does not execute parent tasks.
    #[clap(long)]
    pub only: bool,
    #[clap(long, hide = true)]
    pub pkg_inference_root: Option<String>,
    /// Ignore the local filesystem cache for all tasks. Only
    /// allow reading and caching artifacts using the remote cache.
    #[clap(long, env = "TURBO_REMOTE_ONLY", value_name = "BOOL", action = ArgAction::Set, default_value = "false", default_missing_value = "true", num_args = 0..=1)]
    pub remote_only: bool,
    /// Use "none" to remove prefixes from task logs. Use "task" to get task id
    /// prefixing. Use "auto" to let turbo decide how to prefix the logs
    /// based on the execution environment. In most cases this will be the same
    /// as "task". Note that tasks running in parallel interleave their
    /// logs, so removing prefixes can make it difficult to associate logs
    /// with tasks. Use --log-order=grouped to prevent interleaving. (default
    /// auto)
    #[clap(long, value_enum, default_value_t = LogPrefix::Auto)]
    pub log_prefix: LogPrefix,
    // NOTE: The following two are hidden because clap displays them in the help text incorrectly:
    // > Usage: turbo [OPTIONS] [TASKS]... [-- <FORWARDED_ARGS>...] [COMMAND]
    #[clap(hide = true)]
    pub tasks: Vec<String>,
    #[clap(last = true, hide = true)]
    pub pass_through_args: Vec<String>,
}

impl ExecutionArgs {
    fn track(&self, telemetry: &CommandEventBuilder) {
        // default to false
        track_usage!(telemetry, self.framework_inference, |val: bool| !val);

        track_usage!(telemetry, self.continue_execution, |val| val);
        track_usage!(telemetry, self.single_package, |val| val);
        track_usage!(telemetry, self.only, |val| val);
        track_usage!(telemetry, self.remote_only, |val| val);
        track_usage!(telemetry, &self.cache_dir, Option::is_some);
        track_usage!(telemetry, &self.force, Option::is_some);
        track_usage!(telemetry, &self.pkg_inference_root, Option::is_some);

        if let Some(concurrency) = &self.concurrency {
            telemetry.track_arg_value("concurrency", concurrency, EventType::NonSensitive);
        }

        if !self.global_deps.is_empty() {
            telemetry.track_arg_value(
                "global-deps",
                self.global_deps.join(", "),
                EventType::NonSensitive,
            );
        }

        if let Some(env_mode) = self.env_mode {
            telemetry.track_arg_value("env-mode", env_mode, EventType::NonSensitive);
        }

        if let Some(output_logs) = &self.output_logs {
            telemetry.track_arg_value("output-logs", output_logs, EventType::NonSensitive);
        }

        if self.log_order != LogOrder::default() {
            telemetry.track_arg_value("log-order", self.log_order, EventType::NonSensitive);
        }

        if self.log_prefix != LogPrefix::default() {
            telemetry.track_arg_value("log-prefix", self.log_prefix, EventType::NonSensitive);
        }

        // track sizes
        if !self.filter.is_empty() {
            telemetry.track_arg_value("filter:length", self.filter.len(), EventType::NonSensitive);
        }
    }
}

#[derive(Parser, Clone, Debug, PartialEq)]
#[command(groups = [
    ArgGroup::new("daemon-group").multiple(false).required(false),
])]
pub struct RunArgs {
    /// Set the number of concurrent cache operations (default 10)
    #[clap(long, default_value_t = DEFAULT_NUM_WORKERS)]
    pub cache_workers: u32,
    #[clap(alias = "dry", long = "dry-run", num_args = 0..=1, default_missing_value = "text")]
    pub dry_run: Option<DryRunMode>,
    /// Generate a graph of the task execution and output to a file when a
    /// filename is specified (.svg, .png, .jpg, .pdf, .json,
    /// .html, .mermaid, .dot). Outputs dot graph to stdout when if no filename
    /// is provided
    #[clap(long, num_args = 0..=1, default_missing_value = "", value_parser = validate_graph_extension)]
    pub graph: Option<String>,

    /// Avoid saving task results to the cache. Useful for development/watch
    /// tasks.
    #[clap(long)]
    pub no_cache: bool,

    // clap does not have negation flags such as --daemon and --no-daemon
    // so we need to use a group to enforce that only one of them is set.
    // we set the long name as [no-]daemon with an alias of daemon such
    // that we can merge the help text together for both flags
    // -----------------------
    /// Force turbo to either use or not use the local daemon. If unset
    /// turbo will use the default detection logic.
    #[clap(long = "[no-]daemon", alias = "daemon", group = "daemon-group")]
    pub daemon: bool,

    #[clap(long, group = "daemon-group", hide = true)]
    pub no_daemon: bool,

    /// File to write turbo's performance profile output into.
    /// You can load the file up in chrome://tracing to see
    /// which parts of your build were slow.
    #[clap(long, value_parser=NonEmptyStringValueParser::new(), conflicts_with = "anon_profile")]
    pub profile: Option<String>,
    /// File to write turbo's performance profile output into.
    /// All identifying data omitted from the profile.
    #[clap(long, value_parser=NonEmptyStringValueParser::new(), conflicts_with = "profile")]
    pub anon_profile: Option<String>,
    /// Treat remote cache as read only
    #[clap(long, env = "TURBO_REMOTE_CACHE_READ_ONLY", value_name = "BOOL", action = ArgAction::Set, default_value = "false", default_missing_value = "true", num_args = 0..=1)]
    pub remote_cache_read_only: bool,
    /// Generate a summary of the turbo run
    #[clap(long, env = "TURBO_RUN_SUMMARY", default_missing_value = "true")]
    pub summarize: Option<Option<bool>>,

    // Pass a string to enable posting Run Summaries to Vercel
    #[clap(long, hide = true)]
    pub experimental_space_id: Option<String>,

    /// Execute all tasks in parallel.
    #[clap(long)]
    pub parallel: bool,
}

impl Default for RunArgs {
    fn default() -> Self {
        Self {
            cache_workers: DEFAULT_NUM_WORKERS,
            dry_run: None,
            graph: None,
            no_cache: false,
            daemon: false,
            no_daemon: false,
            profile: None,
            anon_profile: None,
            remote_cache_read_only: false,
            summarize: None,
            experimental_space_id: None,
            parallel: false,
        }
    }
}

impl RunArgs {
    /// Some(true) means force the daemon
    /// Some(false) means force no daemon
    /// None means use the default detection
    pub fn daemon(&self) -> Option<bool> {
        match (self.daemon, self.no_daemon) {
            (true, false) => Some(true),
            (false, true) => Some(false),
            (false, false) => None,
            (true, true) => unreachable!(), // guaranteed by mutually exclusive `ArgGroup`
        }
    }

    pub fn profile_file_and_include_args(&self) -> Option<(&str, bool)> {
        match (self.profile.as_deref(), self.anon_profile.as_deref()) {
            (Some(file), None) => Some((file, true)),
            (None, Some(file)) => Some((file, false)),
            (Some(_), Some(_)) => unreachable!(),
            (None, None) => None,
        }
    }

    pub fn track(&self, telemetry: &CommandEventBuilder) {
        // default to true
        track_usage!(telemetry, self.no_cache, |val| val);
        track_usage!(telemetry, self.daemon, |val| val);
        track_usage!(telemetry, self.no_daemon, |val| val);
        track_usage!(telemetry, self.parallel, |val| val);
        track_usage!(telemetry, self.remote_cache_read_only, |val| val);

        // default to None
        track_usage!(telemetry, &self.profile, Option::is_some);
        track_usage!(telemetry, &self.anon_profile, Option::is_some);
        track_usage!(telemetry, &self.summarize, Option::is_some);
        track_usage!(telemetry, &self.experimental_space_id, Option::is_some);

        // track values
        if let Some(dry_run) = &self.dry_run {
            telemetry.track_arg_value("dry-run", dry_run, EventType::NonSensitive);
        }

        if self.cache_workers != DEFAULT_NUM_WORKERS {
            telemetry.track_arg_value("cache-workers", self.cache_workers, EventType::NonSensitive);
        }

        if let Some(graph) = &self.graph {
            // track the extension used only
            let extension = Utf8Path::new(graph).extension().unwrap_or("stdout");
            telemetry.track_arg_value("graph", extension, EventType::NonSensitive);
        }
    }
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Serialize)]
pub enum LogPrefix {
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "none")]
    None,
    #[serde(rename = "task")]
    Task,
}

impl Default for LogPrefix {
    fn default() -> Self {
        Self::Auto
    }
}

impl Display for LogPrefix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogPrefix::Auto => write!(f, "auto"),
            LogPrefix::None => write!(f, "none"),
            LogPrefix::Task => write!(f, "task"),
        }
    }
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
/// * `logger`: The logger to use for the run.
/// * `color_config`: The color configuration to use for the run, i.e. whether
///   we should colorize output.
///
/// returns: Result<Payload, Error>
#[tokio::main]
pub async fn run(
    repo_state: Option<RepoState>,
    #[allow(unused_variables)] logger: &TurboSubscriber,
    color_config: ColorConfig,
) -> Result<i32, Error> {
    // TODO: remove mutability from this function
    let mut cli_args = Args::new();
    let version = get_version();

    // track telemetry handle to close at the end of the run
    let mut telemetry_handle: Option<TelemetryHandle> = None;

    // initialize telemetry
    match AnonAPIClient::new("https://telemetry.vercel.com", 250, version) {
        Ok(anonymous_api_client) => {
            let handle = init_telemetry(anonymous_api_client, color_config);
            match handle {
                Ok(h) => telemetry_handle = Some(h),
                Err(error) => {
                    debug!("failed to start telemetry: {:?}", error)
                }
            }
        }
        Err(error) => {
            debug!("Failed to create AnonAPIClient: {:?}", error);
        }
    }

    let should_print_version = env::var("TURBO_PRINT_VERSION_DISABLED")
        .map_or(true, |disable| !matches!(disable.as_str(), "1" | "true"))
        && !turborepo_ci::is_ci();

    if should_print_version {
        eprintln!("{}\n", GREY.apply_to(format!("turbo {}", get_version())));
    }

    // If there is no command, we set the command to `Command::Run` with
    // `self.parsed_args.run_args` as arguments.
    let mut command = if let Some(command) = mem::take(&mut cli_args.command) {
        command
    } else {
        let run_args = cli_args.run_args.take().unwrap_or_default();
        let execution_args = cli_args
            .execution_args
            // We clone instead of take as take would leave the command base a copy of cli_args
            // missing any execution args.
            .clone()
            .ok_or_else(|| Error::NoCommand(Backtrace::capture()))?;

        if execution_args.tasks.is_empty() {
            let mut cmd = <Args as CommandFactory>::command();
            let _ = cmd.print_help();
            process::exit(1);
        }

        Command::Run {
            run_args: Box::new(run_args),
            execution_args: Box::new(execution_args),
        }
    };

    // Set some run flags if we have the data and are executing a Run
    if let Command::Run {
        run_args: _,
        execution_args,
    } = &mut command
    {
        // Don't overwrite the flag if it's already been set for whatever reason
        execution_args.single_package = execution_args.single_package
            || repo_state
                .as_ref()
                .map(|repo_state| matches!(repo_state.mode, RepoMode::SinglePackage))
                .unwrap_or(false);
        // If this is a run command, and we know the actual invocation path, set the
        // inference root, as long as the user hasn't overridden the cwd
        if cli_args.cwd.is_none() {
            if let Ok(invocation_dir) = env::var(INVOCATION_DIR_ENV_VAR) {
                // TODO: this calculation can probably be wrapped into the path library
                // and made a little more robust or clear
                let invocation_path = Utf8Path::new(&invocation_dir);

                // If repo state doesn't exist, we're either local turbo running at the root
                // (cwd), or inference failed.
                // If repo state does exist, we're global turbo, and want to calculate
                // package inference based on the repo root
                let this_dir = AbsoluteSystemPathBuf::cwd()?;
                let repo_root = repo_state.as_ref().map_or(&this_dir, |r| &r.root);
                if let Ok(relative_path) = invocation_path.strip_prefix(repo_root) {
                    if !relative_path.as_str().is_empty() {
                        debug!("pkg_inference_root set to \"{}\"", relative_path);
                        execution_args.pkg_inference_root = Some(relative_path.to_string());
                    }
                }
            } else {
                debug!("{} not set", INVOCATION_DIR_ENV_VAR);
            }
        }
    }

    // TODO: make better use of RepoState, here and below. We've already inferred
    // the repo root, we don't need to calculate it again, along with package
    // manager inference.
    let cwd = repo_state
        .as_ref()
        .map(|state| state.root.as_path())
        .or(cli_args.cwd.as_deref());

    let repo_root = if let Some(cwd) = cwd {
        AbsoluteSystemPathBuf::from_cwd(cwd)?
    } else {
        AbsoluteSystemPathBuf::cwd()?
    };

    cli_args.command = Some(command);
    cli_args.cwd = Some(repo_root.as_path().to_owned());

    let root_telemetry = GenericEventBuilder::new();
    root_telemetry.track_start();

    // track system info
    root_telemetry.track_platform(TurboState::platform_name());
    root_telemetry.track_version(TurboState::version());
    root_telemetry.track_cpus(num_cpus::get());
    // track args
    cli_args.track(&root_telemetry);

    let cli_result = match cli_args.command.as_ref().unwrap() {
        Command::Bin => {
            CommandEventBuilder::new("bin")
                .with_parent(&root_telemetry)
                .track_call();
            bin::run()?;

            Ok(0)
        }
        #[allow(unused_variables)]
        Command::Daemon { command, idle_time } => {
            CommandEventBuilder::new("daemon")
                .with_parent(&root_telemetry)
                .track_call();
            let base = CommandBase::new(cli_args.clone(), repo_root, version, color_config);

            match command {
                Some(command) => daemon::daemon_client(command, &base).await,
                None => daemon::daemon_server(&base, idle_time, logger).await,
            }?;

            Ok(0)
        }
        Command::Generate {
            tag,
            generator_name,
            config,
            root,
            args,
            command,
        } => {
            let event = CommandEventBuilder::new("generate").with_parent(&root_telemetry);
            event.track_call();
            // build GeneratorCustomArgs struct
            let args = GeneratorCustomArgs {
                generator_name: generator_name.clone(),
                config: config.clone(),
                root: root.clone(),
                args: args.clone(),
            };
            let child_event = event.child();
            generate::run(tag, command, &args, child_event)?;
            Ok(0)
        }
        Command::Telemetry { command } => {
            let event = CommandEventBuilder::new("telemetry").with_parent(&root_telemetry);
            event.track_call();
            let mut base = CommandBase::new(cli_args.clone(), repo_root, version, color_config);
            let child_event = event.child();
            telemetry::configure(command, &mut base, child_event);
            Ok(0)
        }
        Command::Scan {} => {
            let base = CommandBase::new(cli_args.clone(), repo_root, version, color_config);
            if scan::run(base).await {
                Ok(0)
            } else {
                Ok(1)
            }
        }
        Command::Config => {
            let base = CommandBase::new(cli_args.clone(), repo_root, version, color_config);
            config::run(base).await?;
            Ok(0)
        }
        Command::Ls {
            affected,
            filter,
            packages,
            output,
        } => {
            warn!("ls command is experimental and may change in the future");
            let event = CommandEventBuilder::new("info").with_parent(&root_telemetry);

            event.track_call();
            let affected = *affected;
            let output = *output;
            let filter = filter.clone();
            let packages = packages.clone();
            let base = CommandBase::new(cli_args, repo_root, version, color_config);

            ls::run(base, packages, event, filter, affected, output).await?;

            Ok(0)
        }
        Command::Link {
            no_gitignore,
            target,
        } => {
            CommandEventBuilder::new("link")
                .with_parent(&root_telemetry)
                .track_call();
            if cli_args.test_run {
                println!("Link test run successful");
                return Ok(0);
            }

            let modify_gitignore = !*no_gitignore;
            let to = *target;
            let mut base = CommandBase::new(cli_args, repo_root, version, color_config);

            if let Err(err) = link::link(&mut base, modify_gitignore, to).await {
                error!("error: {}", err.to_string())
            }

            Ok(0)
        }
        Command::Logout { invalidate } => {
            let event = CommandEventBuilder::new("logout").with_parent(&root_telemetry);
            event.track_call();
            let invalidate = *invalidate;

            let mut base = CommandBase::new(cli_args, repo_root, version, color_config);
            let event_child = event.child();

            logout::logout(&mut base, invalidate, event_child).await?;

            Ok(0)
        }
        Command::Login { sso_team, force } => {
            let event = CommandEventBuilder::new("login").with_parent(&root_telemetry);
            event.track_call();
            if cli_args.test_run {
                println!("Login test run successful");
                return Ok(0);
            }

            let sso_team = sso_team.clone();
            let force = *force;

            let mut base = CommandBase::new(cli_args, repo_root, version, color_config);
            let event_child = event.child();

            if let Some(sso_team) = sso_team {
                login::sso_login(&mut base, &sso_team, event_child, force).await?;
            } else {
                login::login(&mut base, event_child, force).await?;
            }

            Ok(0)
        }
        Command::Unlink { target } => {
            CommandEventBuilder::new("unlink")
                .with_parent(&root_telemetry)
                .track_call();
            if cli_args.test_run {
                println!("Unlink test run successful");
                return Ok(0);
            }

            let from = *target;
            let mut base = CommandBase::new(cli_args, repo_root, version, color_config);

            unlink::unlink(&mut base, from)?;

            Ok(0)
        }
        Command::Run {
            run_args,
            execution_args,
        } => {
            let event = CommandEventBuilder::new("run").with_parent(&root_telemetry);
            event.track_call();

            let base = CommandBase::new(cli_args.clone(), repo_root, version, color_config);

            if execution_args.tasks.is_empty() {
                print_potential_tasks(base, event).await?;
                return Ok(1);
            }

            if let Some((file_path, include_args)) = run_args.profile_file_and_include_args() {
                // TODO: Do we want to handle the result / error?
                let _ = logger.enable_chrome_tracing(file_path, include_args);
            }

            run_args.track(&event);
            let exit_code = run::run(base, event).await.inspect(|code| {
                if *code != 0 {
                    error!("run failed: command  exited ({code})");
                }
            })?;
            Ok(exit_code)
        }
        Command::Query { query } => {
            warn!("query command is experimental and may change in the future");
            let query = query.clone();
            let event = CommandEventBuilder::new("query").with_parent(&root_telemetry);
            event.track_call();
            let base = CommandBase::new(cli_args, repo_root, version, color_config);

            let query = query::run(base, event, query).await?;

            Ok(query)
        }
        Command::Watch(_) => {
            let event = CommandEventBuilder::new("watch").with_parent(&root_telemetry);
            event.track_call();
            let base = CommandBase::new(cli_args, repo_root, version, color_config);

            let mut client = WatchClient::new(base, event).await?;
            client.start().await?;
            // We only exit if we get a signal, so we return a non-zero exit code
            return Ok(1);
        }
        Command::Prune {
            scope,
            scope_arg,
            docker,
            output_dir,
        } => {
            let event = CommandEventBuilder::new("prune").with_parent(&root_telemetry);
            event.track_call();
            let scope = scope_arg
                .as_ref()
                .or(scope.as_ref())
                .cloned()
                .unwrap_or_default();
            let docker = *docker;
            let output_dir = output_dir.clone();
            let base = CommandBase::new(cli_args, repo_root, version, color_config);
            let event_child = event.child();
            prune::prune(&base, &scope, docker, &output_dir, event_child).await?;
            Ok(0)
        }
        Command::Completion { shell } => {
            CommandEventBuilder::new("completion")
                .with_parent(&root_telemetry)
                .track_call();
            generate(*shell, &mut Args::command(), "turbo", &mut io::stdout());
            Ok(0)
        }
    };

    root_telemetry.track_end();
    match telemetry_handle {
        Some(handle) => handle.close_with_timeout().await,
        None => debug!("Skipping telemetry close - not initialized"),
    }

    cli_result
}

#[cfg(test)]
mod test {
    use std::assert_matches::assert_matches;

    use camino::Utf8PathBuf;
    use clap::Parser;
    use itertools::Itertools;
    use pretty_assertions::assert_eq;

    use crate::cli::{ExecutionArgs, RunArgs};

    struct CommandTestCase {
        command: &'static str,
        command_args: Vec<Vec<&'static str>>,
        global_args: Vec<Vec<&'static str>>,
        expected_output: Args,
    }

    fn get_default_run_args() -> RunArgs {
        RunArgs {
            cache_workers: 10,
            ..RunArgs::default()
        }
    }

    fn get_default_execution_args() -> ExecutionArgs {
        ExecutionArgs {
            output_logs: None,
            remote_only: false,
            framework_inference: true,
            ..ExecutionArgs::default()
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

    use crate::cli::{Args, Command, DryRunMode, EnvMode, LogOrder, LogPrefix, OutputLogsMode};

    #[test_case::test_case(
        &["turbo", "run", "build"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
        } ;
        "default case"
    )]
    #[test_case::test_case(
        &["turbo", "run", "build"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    framework_inference: true,
                    ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
        } ;
        "framework_inference: default to true"
    )]
    #[test_case::test_case(
		&["turbo", "run", "build", "--framework-inference"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                     tasks: vec!["build".to_string()],
                     framework_inference: true,
                     ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
        } ;
        "framework_inference: flag only"
    )]
    #[test_case::test_case(
		&["turbo", "run", "build", "--framework-inference", "true"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    framework_inference: true,
                    ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
		} ;
        "framework_inference: flag set to true"
    )]
    #[test_case::test_case(
		&["turbo", "run", "build", "--framework-inference",
    "false"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    framework_inference: false,
                    ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
		} ;
        "framework_inference: flag set to false"
	)]
    #[test_case::test_case(
        &["turbo", "run", "build", "--env-mode"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    env_mode: Some(EnvMode::Strict),
                    ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
        } ;
        "env_mode: not fully-specified"
    )]
    #[test_case::test_case(
		&["turbo", "run", "build", "--env-mode", "loose"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    env_mode: Some(EnvMode::Loose),
                    ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
		} ;
        "env_mode: specified loose"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "--env-mode", "strict"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    env_mode: Some(EnvMode::Strict),
                    ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
		} ;
        "env_mode: specified strict"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "lint", "test"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string(), "lint".to_string(), "test".to_string()],
                    ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
        } ;
        "multiple tasks"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "--cache-dir", "foobar"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    cache_dir: Some(Utf8PathBuf::from("foobar")),
                    ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
        } ;
        "cache dir"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "--cache-workers", "100"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec ! ["build".to_string()],
                    ..get_default_execution_args()
                }),
                run_args: Box::new(RunArgs {
                    cache_workers: 100,
                    ..get_default_run_args()
                })
            }),
            ..Args::default()
        } ;
        "cache workers"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "--concurrency", "20"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    concurrency: Some("20".to_string()),
                    ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
        } ;
        "concurrency"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "--continue"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    continue_execution: true,
                    ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
        } ;
        "continue flag"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "--dry-run"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    ..get_default_execution_args()
                }),
                run_args: Box::new(RunArgs {
                    dry_run: Some(DryRunMode::Text),
                    ..get_default_run_args()
                })
            }),
            ..Args::default()
        } ;
        "dry run"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "--dry-run", "json"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    ..get_default_execution_args()
                }),
                run_args: Box::new(RunArgs {
                    dry_run: Some(DryRunMode::Json),
                    ..get_default_run_args()
                })
            }),
            ..Args::default()
        } ;
        "dry run json"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "--filter", "water", "--filter", "earth", "--filter", "fire", "--filter", "air"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    filter: vec![
                        "water".to_string(),
                        "earth".to_string(),
                        "fire".to_string(),
                        "air".to_string()
                    ],
                    ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
        } ;
        "multiple filters"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "-F", "water", "-F", "earth", "-F", "fire", "-F", "air"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    filter: vec![
                        "water".to_string(),
                        "earth".to_string(),
                        "fire".to_string(),
                        "air".to_string()
                    ],
                    ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
        } ;
        "multiple filters short"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "--filter", "water", "-F", "earth", "--filter", "fire", "-F", "air"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    filter: vec![
                        "water".to_string(),
                        "earth".to_string(),
                        "fire".to_string(),
                        "air".to_string()
                    ],
                    ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
        } ;
        "multiple filters short and long"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "--force"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    force: Some(Some(true)),
                    ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
        } ;
        "force"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "--global-deps", ".env"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    global_deps: vec![".env".to_string()],
                    ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
        } ;
        "global deps"
	)]
    #[test_case::test_case(
		&[ "turbo", "run", "build", "--global-deps", ".env", "--global-deps", ".env.development"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    global_deps: vec![".env".to_string(), ".env.development".to_string()],
                    ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
        } ;
        "multiple global deps"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "--graph"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    ..get_default_execution_args()
                }),
                run_args: Box::new(RunArgs {
                    graph: Some("".to_string()),
                    ..get_default_run_args()
                })
            }),
            ..Args::default()
        } ;
        "graph"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "--graph", "out.html"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    ..get_default_execution_args()
                }),
                run_args: Box::new(RunArgs {
                    graph: Some("out.html".to_string()),
                    ..get_default_run_args()
                })
            }),
            ..Args::default()
        } ;
        "graph with output"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "--no-cache"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    ..get_default_execution_args()
                }),
                run_args: Box::new(RunArgs {
                    no_cache: true,
                    ..get_default_run_args()
                })
            }),
            ..Args::default()
        } ;
        "no cache"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "--no-daemon"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    ..get_default_execution_args()
                }),
                run_args: Box::new(RunArgs {
                    no_daemon: true,
                    ..get_default_run_args()
                })
            }),
            ..Args::default()
        } ;
        "no daemon"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "--daemon"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    ..get_default_execution_args()
                }),
                run_args: Box::new(RunArgs {
                    daemon: true,
                    ..get_default_run_args()
                })
            }),
            ..Args::default()
        } ;
        "daemon"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "--output-logs", "full"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    output_logs: Some(OutputLogsMode::Full),
                    ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
        } ;
        "output logs full"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "--output-logs", "none"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    output_logs: Some(OutputLogsMode::None),
                    ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
        } ;
        "output logs none"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "--output-logs", "hash-only"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    output_logs: Some(OutputLogsMode::HashOnly),
                    ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
        } ;
        "output logs hash only"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "--log-order", "stream"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    log_order: LogOrder::Stream,
                    ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
        } ;
        "log order stream"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "--log-order", "grouped"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    log_order: LogOrder::Grouped,
                    ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
        };
        "log order grouped"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "--log-prefix", "auto"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    log_prefix: LogPrefix::Auto,
                    ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
        } ;
        "log prefix auto"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "--log-prefix", "none"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    log_prefix: LogPrefix::None,
                    ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
        } ;
        "log prefix none"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "--log-prefix", "task"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                     tasks: vec!["build".to_string()],
                     log_prefix: LogPrefix::Task,
                     ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
        } ;
        "log prefix task"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    log_order: LogOrder::Auto,
                    ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
        } ;
        "just build"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "--parallel"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    ..get_default_execution_args()
                }),
                run_args: Box::new(RunArgs {
                    parallel: true,
                    ..get_default_run_args()
                })
            }),
            ..Args::default()
        } ;
        "parallel"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "--profile", "profile_out"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    ..get_default_execution_args()
                }),
                run_args: Box::new(RunArgs {
                  profile: Some("profile_out".to_string()),
                  ..get_default_run_args()
                })
            }),
            ..Args::default()
        } ;
        "profile"
	)]
    // remote-only flag tests
    #[test_case::test_case(
		&["turbo", "run", "build"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    remote_only: false,
                    ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
		} ;
        "remote_only default to false"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "--remote-only"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    remote_only: true,
                    ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
		} ;
        "remote_only with no value, means true"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "--remote-only", "true"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    remote_only: true,
                    ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
		} ;
        "remote_only=true works"
	)]
    #[test_case::test_case(
		&["turbo", "run", "build", "--remote-only", "false"],
        Args {
            command: Some(Command::Run {
                execution_args: Box::new(ExecutionArgs {
                    tasks: vec!["build".to_string()],
                    remote_only: false,
                    ..get_default_execution_args()
                }),
                run_args: Box::new(get_default_run_args())
            }),
            ..Args::default()
		} ;
        "remote_only=false works"
	)]
    #[test_case::test_case(
		&["turbo", "build"],
        Args {
            execution_args: Some(ExecutionArgs {
                tasks: vec!["build".to_string()],
                ..get_default_execution_args()
            }),
            ..Args::default()
        } ;
        "build no run prefix"
    )]
    #[test_case::test_case(
    	&["turbo", "build", "lint", "test"],
        Args {
            execution_args: Some(ExecutionArgs {
                tasks: vec!["build".to_string(), "lint".to_string(), "test".to_string()],
                ..get_default_execution_args()
            }),
            ..Args::default()
        } ;
        "multiple tasks no run prefix"
    )]
    fn test_parse_run(args: &[&str], expected: Args) {
        assert_eq!(Args::try_parse_from(args).unwrap(), expected);
    }

    #[test_case::test_case(
        &["turbo", "watch", "build"],
        Args {
            command: Some(Command::Watch(Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                ..get_default_execution_args()
            }))),
            ..Args::default()
        };
        "default watch"
    )]
    #[test_case::test_case(
        &["turbo", "watch", "build", "--cache-dir", "foobar"],
        Args {
            command: Some(Command::Watch(Box::new(ExecutionArgs {
                tasks: vec!["build".to_string()],
                cache_dir: Some(Utf8PathBuf::from("foobar")),
                ..get_default_execution_args()
            }))),
            ..Args::default()
        };
        "with cache-dir"
    )]
    #[test_case::test_case(
        &["turbo", "watch", "build", "lint", "check"],
        Args {
            command: Some(Command::Watch(Box::new(ExecutionArgs {
                tasks: vec!["build".to_string(), "lint".to_string(), "check".to_string()],
                ..get_default_execution_args()
            }))),
            ..Args::default()
        };
        "with multiple tasks"
    )]
    fn test_parse_watch(args: &[&str], expected: Args) {
        assert_eq!(Args::try_parse_from(args).unwrap(), expected);
    }

    #[test_case::test_case(
        &["turbo", "run", "build", "--daemon", "--no-daemon"],
        "cannot be used with '--no-daemon'" ;
        "daemon and no-daemon at the same time"
    )]
    #[test_case::test_case(
        &["turbo", "run", "build", "--since", "foo"],
        "unexpected argument '--since' found" ;
        "since without filter or scope"
    )]
    #[test_case::test_case(
        &["turbo", "run", "build", "--include-dependencies"],
        "unexpected argument '--include-dependencies' found" ;
        "include-dependencies without filter or scope"
    )]
    #[test_case::test_case(
        &["turbo", "run", "build", "--no-deps"],
        "unexpected argument '--no-deps' found" ;
        "no-deps without filter or scope"
    )]
    fn test_parse_run_failures(args: &[&str], expected: &str) {
        assert_matches!(
            Args::try_parse_from(args),
            Err(err) if err.to_string().contains(expected)
        );
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
                cwd: Some(Utf8PathBuf::from("../examples/with-yarn")),
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
                command: Some(Command::Login {
                    sso_team: None,
                    force: false
                }),
                ..Args::default()
            }
        );

        CommandTestCase {
            command: "login",
            command_args: vec![],
            global_args: vec![vec!["--cwd", "../examples/with-yarn"]],
            expected_output: Args {
                command: Some(Command::Login {
                    sso_team: None,
                    force: false,
                }),
                cwd: Some(Utf8PathBuf::from("../examples/with-yarn")),
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
                    force: false,
                }),
                cwd: Some(Utf8PathBuf::from("../examples/with-yarn")),
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
                command: Some(Command::Logout { invalidate: false }),
                ..Args::default()
            }
        );

        CommandTestCase {
            command: "logout",
            command_args: vec![],
            global_args: vec![vec!["--cwd", "../examples/with-yarn"]],
            expected_output: Args {
                command: Some(Command::Logout { invalidate: false }),
                cwd: Some(Utf8PathBuf::from("../examples/with-yarn")),
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
                cwd: Some(Utf8PathBuf::from("../examples/with-yarn")),
                ..Args::default()
            },
        }
        .test();
    }

    #[test]
    fn test_parse_prune() {
        let default_prune = Command::Prune {
            scope: None,
            scope_arg: Some(vec!["foo".into()]),
            docker: false,
            output_dir: "out".to_string(),
        };

        assert_eq!(
            Args::try_parse_from(["turbo", "prune", "foo"]).unwrap(),
            Args {
                command: Some(default_prune.clone()),
                ..Args::default()
            }
        );

        CommandTestCase {
            command: "prune",
            command_args: vec![vec!["foo"]],
            global_args: vec![vec!["--cwd", "../examples/with-yarn"]],
            expected_output: Args {
                command: Some(default_prune),
                cwd: Some(Utf8PathBuf::from("../examples/with-yarn")),
                ..Args::default()
            },
        }
        .test();

        assert_eq!(
            Args::try_parse_from(["turbo", "prune", "--scope", "bar"]).unwrap(),
            Args {
                command: Some(Command::Prune {
                    scope: Some(vec!["bar".to_string()]),
                    scope_arg: None,
                    docker: false,
                    output_dir: "out".to_string(),
                }),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "prune", "foo", "bar"]).unwrap(),
            Args {
                command: Some(Command::Prune {
                    scope: None,
                    scope_arg: Some(vec!["foo".to_string(), "bar".to_string()]),
                    docker: false,
                    output_dir: "out".to_string(),
                }),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "prune", "--docker", "foo"]).unwrap(),
            Args {
                command: Some(Command::Prune {
                    scope: None,
                    scope_arg: Some(vec!["foo".into()]),
                    docker: true,
                    output_dir: "out".to_string(),
                }),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from(["turbo", "prune", "--out-dir", "dist", "foo"]).unwrap(),
            Args {
                command: Some(Command::Prune {
                    scope: None,
                    scope_arg: Some(vec!["foo".into()]),
                    docker: false,
                    output_dir: "dist".to_string(),
                }),
                ..Args::default()
            }
        );

        CommandTestCase {
            command: "prune",
            command_args: vec![vec!["foo"], vec!["--out-dir", "dist"], vec!["--docker"]],
            global_args: vec![],
            expected_output: Args {
                command: Some(Command::Prune {
                    scope: None,
                    scope_arg: Some(vec!["foo".into()]),
                    docker: true,
                    output_dir: "dist".to_string(),
                }),
                ..Args::default()
            },
        }
        .test();

        CommandTestCase {
            command: "prune",
            command_args: vec![vec!["foo"], vec!["--out-dir", "dist"], vec!["--docker"]],
            global_args: vec![vec!["--cwd", "../examples/with-yarn"]],
            expected_output: Args {
                command: Some(Command::Prune {
                    scope: None,
                    scope_arg: Some(vec!["foo".into()]),
                    docker: true,
                    output_dir: "dist".to_string(),
                }),
                cwd: Some(Utf8PathBuf::from("../examples/with-yarn")),
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
                    scope: Some(vec!["foo".to_string()]),
                    scope_arg: None,
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
                command: Some(Command::Run {
                    run_args: Box::new(RunArgs {
                        ..get_default_run_args()
                    }),
                    execution_args: Box::new(ExecutionArgs {
                        tasks: vec!["build".to_string()],
                        pass_through_args: vec!["--script-arg=42".to_string()],
                        ..get_default_execution_args()
                    }),
                }),
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
                command: Some(Command::Run {
                    run_args: Box::new(RunArgs {
                        ..get_default_run_args()
                    }),
                    execution_args: Box::new(ExecutionArgs {
                        tasks: vec!["build".to_string()],
                        pass_through_args: vec![
                            "--script-arg=42".to_string(),
                            "--foo".to_string(),
                            "--bar".to_string(),
                            "bat".to_string()
                        ],
                        ..get_default_execution_args()
                    }),
                }),
                ..Args::default()
            }
        );
    }

    #[test]
    fn test_parse_prune_no_mixed_arg_and_flag() {
        assert!(Args::try_parse_from(["turbo", "prune", "foo", "--scope", "bar"]).is_err(),);
    }

    #[test]
    fn test_parse_gen() {
        let default_gen = Command::Generate {
            tag: "latest".to_string(),
            generator_name: None,
            config: None,
            root: None,
            args: vec![],
            command: None,
        };

        assert_eq!(
            Args::try_parse_from(["turbo", "gen"]).unwrap(),
            Args {
                command: Some(default_gen.clone()),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from([
                "turbo",
                "gen",
                "--args",
                "my long arg string",
                "my-second-arg"
            ])
            .unwrap(),
            Args {
                command: Some(Command::Generate {
                    tag: "latest".to_string(),
                    generator_name: None,
                    config: None,
                    root: None,
                    args: vec![
                        "my long arg string".to_string(),
                        "my-second-arg".to_string()
                    ],
                    command: None,
                }),
                ..Args::default()
            }
        );

        assert_eq!(
            Args::try_parse_from([
                "turbo",
                "gen",
                "--tag",
                "canary",
                "--config",
                "~/custom-gen-config/gen",
                "my-generator"
            ])
            .unwrap(),
            Args {
                command: Some(Command::Generate {
                    tag: "canary".to_string(),
                    generator_name: Some("my-generator".to_string()),
                    config: Some("~/custom-gen-config/gen".to_string()),
                    root: None,
                    args: vec![],
                    command: None,
                }),
                ..Args::default()
            }
        );
    }

    #[test]
    fn test_profile_usage() {
        assert!(Args::try_parse_from(["turbo", "build", "--profile", ""]).is_err());
        assert!(Args::try_parse_from(["turbo", "build", "--anon-profile", ""]).is_err());
        assert!(Args::try_parse_from(["turbo", "build", "--profile", "foo.json"]).is_ok());
        assert!(Args::try_parse_from(["turbo", "build", "--anon-profile", "foo.json"]).is_ok());
        assert!(Args::try_parse_from([
            "turbo",
            "build",
            "--profile",
            "foo.json",
            "--anon-profile",
            "bar.json"
        ])
        .is_err());
    }

    #[test]
    fn test_empty_cache_dir() {
        assert!(Args::try_parse_from(["turbo", "build", "--cache-dir"]).is_err());
        assert!(Args::try_parse_from(["turbo", "build", "--cache-dir="]).is_err());
        assert!(Args::try_parse_from(["turbo", "build", "--cache-dir", ""]).is_err());
    }

    #[test]
    fn test_preflight() {
        assert!(!Args::try_parse_from(["turbo", "build",]).unwrap().preflight);
        assert!(
            Args::try_parse_from(["turbo", "build", "--preflight"])
                .unwrap()
                .preflight
        );
        assert!(Args::try_parse_from(["turbo", "build", "--preflight=true"]).is_err());
    }

    #[test]
    fn test_log_stream_tui_compatibility() {
        assert!(LogOrder::Auto.compatible_with_tui());
        assert!(!LogOrder::Stream.compatible_with_tui());
        assert!(!LogOrder::Grouped.compatible_with_tui());
    }

    #[test]
    fn test_dangerously_allow_no_package_manager() {
        assert!(
            !Args::try_parse_from(["turbo", "build",])
                .unwrap()
                .dangerously_disable_package_manager_check
        );
        assert!(
            Args::try_parse_from([
                "turbo",
                "build",
                "--dangerously-disable-package-manager-check"
            ])
            .unwrap()
            .dangerously_disable_package_manager_check
        );
    }

    #[test]
    fn test_prevent_affected_and_filter() {
        assert!(
            Args::try_parse_from(["turbo", "run", "build", "--affected", "--filter", "foo"])
                .is_err(),
        );
        assert!(Args::try_parse_from(["turbo", "build", "--affected", "--filter", "foo"]).is_err(),);
        assert!(Args::try_parse_from(["turbo", "build", "--filter", "foo", "--affected"]).is_err(),);
        assert!(Args::try_parse_from(["turbo", "ls", "--filter", "foo", "--affected"]).is_err(),);
    }
}
