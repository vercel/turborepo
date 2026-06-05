use std::{env, ffi::OsString, fmt};

use camino::{Utf8Path, Utf8PathBuf};
use clap::{ArgAction, ArgGroup, CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;
use serde::Serialize;
use tracing::{error, log::warn};
use turborepo_telemetry::{
    events::{command::CommandEventBuilder, generic::GenericEventBuilder, EventType},
    track_usage,
};
use turborepo_types::{
    ContinueMode, DryRunMode, EnvMode, LogOrder, LogPrefix, OutputLogsMode, UIMode,
};

use super::{exit_with_heap_profile, observability};
use crate::{commands::prune, get_version};

const DEFAULT_NUM_WORKERS: u32 = 10;
const SUPPORTED_GRAPH_FILE_EXTENSIONS: [&str; 8] =
    ["svg", "png", "jpg", "pdf", "json", "html", "mermaid", "dot"];

/// The parsed arguments from the command line. In general we should avoid using
/// or mutating this directly, and instead use the fully canonicalized `Opts`
/// struct.
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
    #[clap(flatten)]
    pub experimental_otel_args: observability::ExperimentalOtelCliArgs,
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
    #[clap(long = "experimental-allow-no-turbo-json", hide = true, global = true)]
    pub allow_no_turbo_json: bool,
    /// Use the `turbo.json` located at the provided path instead of one at the
    /// root of the repository.
    #[clap(long, global = true)]
    pub root_turbo_json: Option<Utf8PathBuf>,
    #[clap(flatten, next_help_heading = "Run Arguments")]
    // DO NOT MAKE THIS VISIBLE
    // This is explicitly set to None in `run`
    pub(super) run_args: Option<RunArgs>,
    // This should be inside `RunArgs` but clap currently has a bug
    // around nested flattened optional args: https://github.com/clap-rs/clap/issues/4697
    #[clap(flatten)]
    // DO NOT MAKE THIS VISIBLE
    // Instead use the getter method execution_args()
    pub(super) execution_args: Option<ExecutionArgs>,
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
    /// Verbosity level. Useful when debugging Turborepo or creating logs for
    /// issue reports
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

/// Returns formatted RunArgs options derived from clap's command definition.
/// These options aren't included in the usage line by clap because they're all
/// optional.
fn get_run_args_options() -> Vec<String> {
    RunArgs::command()
        .get_arguments()
        .filter(|arg| arg.get_long().is_some())
        .map(|arg| {
            let Some(long) = arg.get_long() else {
                return String::new();
            };
            let value_names: Vec<_> = arg.get_value_names().unwrap_or_default().to_vec();

            if value_names.is_empty() {
                // Boolean flag
                format!("--{}", long)
            } else {
                // Check if value is optional via num_args (0..=1 means optional)
                let is_optional = arg
                    .get_num_args()
                    .is_some_and(|range| range.min_values() == 0);

                let value_str = value_names
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join("> <");

                if is_optional {
                    format!("--{} [<{}>]", long, value_str)
                } else {
                    format!("--{} <{}>", long, value_str)
                }
            }
        })
        .collect()
}

/// Formats clap error messages to improve readability of pipe-separated options
fn format_error_message(mut err_str: String) -> String {
    // Replace pipe separators in usage line with newlines for better readability
    // The usage line typically looks like: "Usage: turbo <--opt1|--opt2|--opt3>"
    if let Some(usage_start) = err_str.find("Usage: ") {
        if let Some(usage_end) = err_str[usage_start..].find('\n') {
            let usage_end = usage_start + usage_end;
            let usage_line = &err_str[usage_start..usage_end];

            // Check if this usage line contains the pipe-separated options pattern
            if usage_line.contains('<') && usage_line.contains('>') && usage_line.contains('|') {
                // Find the angle bracket enclosed section
                if let Some(bracket_start) = usage_line.find('<') {
                    if let Some(bracket_end) = usage_line.rfind('>') {
                        let prefix = &usage_line[..bracket_start];
                        let options_str = &usage_line[bracket_start + 1..bracket_end];

                        // Split the options by pipe and format them as a list
                        let mut formatted_options: Vec<String> = options_str
                            .split('|')
                            .map(|opt| format!("    {}", opt))
                            .collect();

                        // Add RunArgs options that clap doesn't include
                        for opt in get_run_args_options() {
                            formatted_options.push(format!("    {}", opt));
                        }

                        // Build the new usage string
                        let new_usage = format!(
                            "{} [OPTIONS] [TASKS]... [-- <PASS_THROUGH_ARGS>...]\n\nOptions:\n{}",
                            prefix.trim_end(),
                            formatted_options.join("\n")
                        );

                        // Replace the old usage line with the new formatted one
                        err_str.replace_range(usage_start..usage_end, &new_usage);
                    }
                }
            }
        }
    }
    err_str
}

impl Args {
    #[tracing::instrument(skip_all)]
    pub fn new(os_args: Vec<OsString>) -> Self {
        let clap_args = match Args::parse(os_args) {
            Ok(args) => args,
            // Don't use error logger when displaying help text
            Err(e)
                if matches!(
                    e.kind(),
                    clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
                ) =>
            {
                let _ = e.print();
                exit_with_heap_profile(1);
            }
            Err(e) if e.use_stderr() => {
                let err_str = format_error_message(e.to_string());
                // A cleaner solution would be to implement our own clap::error::ErrorFormatter
                // but that would require copying the default formatter just to remove this
                // line: https://docs.rs/clap/latest/src/clap/error/format.rs.html#100
                error!(
                    "{}",
                    err_str.strip_prefix("error: ").unwrap_or(err_str.as_str())
                );
                exit_with_heap_profile(1);
            }
            // If the clap error shouldn't be printed to stderr it indicates help text
            Err(e) => {
                let _ = e.print();
                exit_with_heap_profile(0);
            }
        };
        // We have to override the --version flag because we use `get_version`
        // instead of a hard-coded version or the crate version
        if clap_args.version {
            println!("{}", get_version());
            exit_with_heap_profile(0);
        }

        if let Some(run_args) = clap_args.run_args() {
            if run_args.no_cache {
                warn!(
                    "--no-cache is deprecated and will be removed in a future major version. Use \
                     --cache=local:r,remote:r"
                );
            }
            if run_args.remote_only.is_some() {
                warn!(
                    "--remote-only is deprecated and will be removed in a future major version. \
                     Use --cache=remote:rw"
                );
            }
            if run_args.remote_cache_read_only.is_some() {
                warn!(
                    "--remote-cache-read-only is deprecated and will be removed in a future major \
                     version. Use --cache=local:rw,remote:r"
                );
            }
            if run_args.daemon {
                warn!(
                    "--daemon is deprecated and will be removed in version 3.0. The daemon is no \
                     longer used for `turbo run`."
                );
            }
            if run_args.no_daemon {
                warn!(
                    "--no-daemon is deprecated and will be removed in version 3.0. The daemon is \
                     no longer used for `turbo run`."
                );
            }
            if run_args.parallel {
                warn!(
                    "--parallel is deprecated and will be removed in a future major version. \
                     Instead, define task behavior in your turbo.json task definitions using \
                     `persistent` and `with`."
                );
            }
            if let Some(graph) = &run_args.graph {
                match Utf8Path::new(graph).extension() {
                    Some(ext @ ("png" | "jpg" | "pdf")) => {
                        warn!(
                            "--graph with .{ext} output is deprecated and will be removed in \
                             version 3.0. Use .svg, .html, .mermaid, or .dot instead.",
                        );
                    }
                    Some("json") => {
                        warn!(
                            "--graph with .json output is deprecated and will be removed in \
                             version 3.0. Use `turbo query` for programmatic access to the task \
                             graph."
                        );
                    }
                    _ => {}
                }
            }
        }

        if let Some(Command::Prune { ref scope, .. }) = clap_args.command {
            if scope.is_some() {
                warn!(
                    "--scope is deprecated and will be removed in a future major version. Use \
                     positional arguments instead (e.g. `turbo prune web`)"
                );
            }
        }

        clap_args
    }

    pub(crate) fn parse(os_args: Vec<OsString>) -> Result<Self, clap::Error> {
        let (is_single_package, single_package_free) = Self::remove_single_package(os_args);
        let mut args = Args::try_parse_from(single_package_free)?;
        // --single-package is stripped before clap parsing, so we need to
        // propagate it back. The value can appear in two places in the struct.
        // We defensively attempt to set both.
        if let Some(ref mut execution_args) = args.execution_args {
            execution_args.single_package = is_single_package
        }

        if let Some(
            Command::Run {
                ref mut execution_args,
                ..
            }
            | Command::Watch {
                ref mut execution_args,
                ..
            },
        ) = args.command.as_mut()
        {
            execution_args.single_package = is_single_package;
        }

        if env::var("TEST_RUN").is_ok() {
            args.test_run = true;
        }

        args.validate()?;

        Ok(args)
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

    /// Fetch the run args supplied to the command
    pub fn run_args(&self) -> Option<&RunArgs> {
        if let Some(Command::Run { run_args, .. }) = &self.command {
            Some(run_args)
        } else {
            self.run_args.as_ref()
        }
    }

    /// Fetch the execution args supplied to the command
    pub fn execution_args(&self) -> Option<&ExecutionArgs> {
        match &self.command {
            Some(Command::Run { execution_args, .. }) => Some(execution_args),
            Some(Command::Watch { execution_args, .. }) => Some(execution_args),
            _ => self.execution_args.as_ref(),
        }
    }

    pub(super) fn remove_single_package(
        args: Vec<OsString>,
    ) -> (bool, impl Iterator<Item = OsString>) {
        // We always pass --single-package in from the shim.
        // We need to omit it, and then add it in for run.
        let arg_separator_position = args.iter().position(|input_token| input_token == "--");

        let single_package_position = args
            .iter()
            .position(|input_token| input_token == "--single-package");

        let is_single_package = match (arg_separator_position, single_package_position) {
            (_, None) => false,
            (None, Some(_)) => true,
            (Some(arg_separator_position), Some(single_package_position)) => {
                single_package_position < arg_separator_position
            }
        };

        // Clap supports arbitrary iterators as input.
        // We can remove all instances of --single-package
        let single_package_free = args
            .into_iter()
            .enumerate()
            .filter(move |(index, input_token)| {
                arg_separator_position
                    .is_some_and(|arg_separator_position| index > &arg_separator_position)
                    || input_token != "--single-package"
            })
            .map(|(_, input_token)| input_token);

        (is_single_package, single_package_free)
    }

    fn validate(&self) -> Result<(), clap::Error> {
        if self.run_args.is_some()
            && !matches!(
                self.command,
                None | Some(Command::Run { .. })
                    | Some(Command::Config)
                    | Some(Command::Boundaries { .. })
            )
        {
            let mut cmd = Self::command();
            Err(cmd.error(
                clap::error::ErrorKind::UnknownArgument,
                "Cannot use run arguments outside of run command",
            ))
        } else if self.execution_args.is_some()
            && matches!(self.command, Some(Command::Watch { .. }))
        {
            let mut cmd = Self::command();
            Err(cmd.error(
                clap::error::ErrorKind::ArgumentConflict,
                "Cannot use watch arguments before `watch` subcommand",
            ))
        } else if matches!(self.command, Some(Command::Run { .. }))
            && (self.run_args.is_some() || self.execution_args.is_some())
        {
            let mut cmd = Self::command();
            Err(cmd.error(
                clap::error::ErrorKind::ArgumentConflict,
                "Cannot use run arguments before `run` subcommand",
            ))
        } else if matches!(self.command, Some(Command::Boundaries { .. }))
            && (self.run_args.is_some() || self.execution_args.is_some())
        {
            let mut cmd = Self::command();
            Err(cmd.error(
                clap::error::ErrorKind::ArgumentConflict,
                "Cannot use run arguments before `boundaries` subcommand",
            ))
        } else {
            Ok(())
        }
    }
}

/// Defines the subcommands for CLI
#[derive(Subcommand, Clone, Debug, PartialEq)]
pub enum Command {
    /// Get the path to the Turbo binary
    Bin,
    /// Get the port assigned to the current microfrontend
    #[clap(name = "get-mfe-port")]
    GetMfePort,
    #[clap(hide = true)]
    Boundaries {
        #[clap(short = 'F', long, group = "scope-filter-group")]
        filter: Vec<String>,
        #[clap(long, value_enum, default_missing_value = "prompt", num_args = 0..=1, require_equals = true)]
        ignore: Option<BoundariesIgnore>,
        #[clap(long, requires = "ignore")]
        reason: Option<String>,
    },
    /// Generate the autocompletion script for the specified shell
    Completion { shell: Shell },
    /// Runs the Turborepo background daemon
    Daemon {
        /// Set the idle timeout for turbod
        #[clap(long, default_value_t = String::from("4h0m0s"))]
        idle_time: String,
        /// Path to a custom turbo.json file to watch from --root-turbo-json
        #[clap(long)]
        turbo_json_path: Option<Utf8PathBuf>,
        #[clap(subcommand)]
        command: Option<DaemonCommand>,
    },
    /// Visualize your monorepo's package graph in the browser
    Devtools {
        /// Port for the WebSocket server
        #[clap(long, default_value_t = turborepo_devtools::DEFAULT_PORT)]
        port: u16,
        /// Don't automatically open the browser
        #[clap(long)]
        no_open: bool,
    },
    /// Search the Turborepo documentation
    Docs {
        /// The search query
        query: String,
        /// Override the docs version (minimum: 2.7.5)
        #[clap(long)]
        docs_version: Option<String>,
    },
    /// Generate a new app / package
    #[clap(aliases = ["g", "gen"])]
    Generate {
        #[clap(long, hide = true)]
        tag: Option<String>,
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
    /// [DEPRECATED] `turbo scan` has been removed. This command will be
    /// fully removed in a future major version.
    #[clap(hide = true)]
    Scan,
    #[clap(hide = true)]
    Config,
    /// EXPERIMENTAL: List packages in your monorepo.
    Ls {
        /// Show only packages that are affected by changes between
        /// the current branch and `main`
        #[clap(long)]
        affected: bool,
        /// Use the given selector to specify package(s) to act as
        /// entry points. The syntax mirrors pnpm's syntax, and
        /// additional documentation and examples can be found in
        /// turbo's documentation https://turborepo.dev/docs/reference/command-line-reference/run#--filter
        #[clap(short = 'F', long)]
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

        /// The scope, i.e. Vercel team, to which you are linking
        #[clap(long)]
        scope: Option<String>,

        /// Answer yes to all prompts (default false)
        #[clap(long, short)]
        yes: bool,
    },
    /// Login to your Vercel account
    Login {
        #[clap(long = "sso-team")]
        sso_team: Option<String>,
        /// Deprecated, no-op. Previously forced a new login even if a valid
        /// token existed.
        #[clap(long = "force", short = 'f', hide = true)]
        force: bool,
        /// Manually enter token instead of requesting one from the login
        /// service.
        #[clap(long, conflicts_with = "sso_team")]
        manual: bool,
    },
    /// Logout to your Vercel account
    Logout {
        /// Invalidate the token on the server. Pass `--invalidate=false` to
        /// skip the remote revoke.
        #[clap(long, value_name = "BOOL", action = ArgAction::Set, default_value = "true", default_missing_value = "true", num_args = 0..=1)]
        invalidate: bool,
    },
    /// Print debugging information
    Info,
    /// Prepare a subset of your monorepo.
    Prune {
        /// DEPRECATED: Use positional arguments instead
        /// (e.g. `turbo prune web`)
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
        /// Respect `.gitignore` when copying files to <OUT-DIR>
        #[clap(long, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        use_gitignore: Option<bool>,
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
    #[command(args_conflicts_with_subcommands = true)]
    Query {
        #[clap(subcommand)]
        subcommand: Option<QuerySubcommand>,
        /// Pass variables to the query via a JSON file
        #[clap(short = 'V', long, requires = "query")]
        variables: Option<Utf8PathBuf>,
        #[clap(long, conflicts_with = "query")]
        schema: bool,
        /// The query to run, either a file path or a query string
        query: Option<String>,
    },
    Watch {
        #[clap(flatten)]
        execution_args: Box<ExecutionArgs>,
        /// EXPERIMENTAL: Write to cache in watch mode.
        #[clap(long)]
        experimental_write_cache: bool,
    },
    /// Unlink the current directory from your Vercel organization and disable
    /// Remote Caching
    Unlink,
}

#[derive(Copy, Clone, Debug, Default, ValueEnum, Serialize, Eq, PartialEq)]
pub enum BoundariesIgnore {
    /// Adds a `@boundaries-ignore` comment everywhere possible
    All,
    /// Prompts user if they want to add `@boundaries-ignore` comment
    #[default]
    Prompt,
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
    pub(super) generator_name: Option<String>,
    /// Generator configuration file
    #[clap(short = 'c', long)]
    pub(super) config: Option<String>,
    /// The root of your repository (default: directory with root
    /// turbo.json)
    #[clap(short = 'r', long)]
    pub(super) root: Option<String>,
    /// Answers passed directly to generator
    #[clap(short = 'a', long, value_delimiter = ' ', num_args = 1..)]
    pub(super) args: Vec<String>,
}

#[derive(Subcommand, Clone, Debug, PartialEq)]
pub enum GenerateCommand {
    /// Add a new package or app to your project
    #[clap(name = "workspace", alias = "w")]
    Workspace(GenerateWorkspaceArgs),
    #[clap(name = "run", alias = "r")]
    Run(GeneratorCustomArgs),
}

#[derive(Subcommand, Clone, Debug, PartialEq)]
pub enum QuerySubcommand {
    /// Check which packages or tasks are affected by changes between two git
    /// refs
    Affected(AffectedArgs),
    /// List packages in your monorepo (shorthand for a packages query)
    Ls(LsArgs),
}

#[derive(clap::Args, Clone, Debug, PartialEq)]
pub struct LsArgs {
    /// Show only packages that are affected by changes between
    /// the current branch and `main`
    #[clap(long)]
    pub affected: bool,
    /// Use the given selector to specify package(s) to act as
    /// entry points. The syntax mirrors pnpm's syntax, and
    /// additional documentation and examples can be found in
    /// turbo's documentation https://turborepo.dev/docs/reference/command-line-reference/run#--filter
    #[clap(short = 'F', long)]
    pub filter: Vec<String>,
    /// Get insight into a specific package, such as
    /// its dependencies and tasks
    pub packages: Vec<String>,
    /// Output format
    #[clap(long, value_enum)]
    pub output: Option<OutputFormat>,
}

#[derive(clap::Args, Clone, Debug, PartialEq)]
pub struct AffectedArgs {
    /// Return affected packages instead of tasks. Optionally filter by name.
    /// When combined with --tasks, returns affected tasks that match both
    /// the task name and package filters.
    #[clap(long, num_args = 0..)]
    pub packages: Option<Vec<String>>,
    /// Filter to specific task names (e.g. build, test).
    /// When combined with --packages, returns affected tasks that match both
    /// the task name and package filters.
    #[clap(long, num_args = 0..)]
    pub tasks: Option<Vec<String>>,
    /// Base git ref for comparison
    #[clap(long)]
    pub base: Option<String>,
    /// Head git ref for comparison
    #[clap(long)]
    pub head: Option<String>,
    /// Exit with code 1 when affected packages or tasks are found, 0 when
    /// none are found, or 2 on errors. Useful for CI gating. We recommend
    /// parsing the JSON output directly for more flexibility.
    #[clap(long)]
    pub exit_code: bool,
}

fn validate_graph_extension(s: &str) -> Result<String, String> {
    match s.is_empty() {
        true => Ok(s.to_string()),
        _ => match Utf8Path::new(s).extension() {
            Some(ext) if SUPPORTED_GRAPH_FILE_EXTENSIONS.contains(&ext) => Ok(s.to_string()),
            Some(ext) => Err(format!(
                "Invalid file extension: '{ext}'. Allowed extensions are: \
                 {SUPPORTED_GRAPH_FILE_EXTENSIONS:?}"
            )),
            None => Err(format!(
                "The provided filename is missing a file extension. Allowed extensions are: \
                 {SUPPORTED_GRAPH_FILE_EXTENSIONS:?}"
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
    #[clap(long, value_parser = path_non_empty)]
    pub cache_dir: Option<Utf8PathBuf>,
    /// Limit the concurrency of task execution. Use 1 for serial (i.e.
    /// one-at-a-time) execution.
    #[clap(long)]
    pub concurrency: Option<String>,
    /// Specify how task execution should proceed when an error occurs.
    /// Use "never" to cancel all tasks. Use "dependencies-successful" to
    /// continue running tasks whose dependencies have succeeded. Use "always"
    /// to continue running all tasks, even those whose dependencies have
    /// failed.
    #[clap(long = "continue", value_name = "CONTINUE", num_args = 0..=1, default_value = "never", default_missing_value = "always", require_equals = true)]
    pub continue_execution: ContinueMode,
    /// Run turbo in single-package mode
    #[clap(long)]
    pub single_package: bool,
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
    /// turbo's documentation https://turborepo.dev/docs/reference/command-line-reference/run#--filter
    #[clap(short = 'F', long, group = "scope-filter-group")]
    pub filter: Vec<String>,

    /// Filter to only packages that are affected by changes between
    /// the current branch and `main`
    #[clap(long, group = "scope-filter-group")]
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
    #[clap(long, value_enum)]
    pub log_order: Option<LogOrder>,
    /// Output machine-readable NDJSON to stdout instead of human-readable
    /// text. Disables the TUI and forces stream mode.
    #[clap(long)]
    pub json: bool,
    /// Write structured JSON logs to a file. If no path is given, writes to
    /// `.turbo/logs/<epoch_millis>.json`.
    #[clap(long)]
    pub log_file: Option<Option<String>>,
    /// Only executes the tasks specified, does not execute parent tasks.
    #[clap(long)]
    pub only: bool,
    #[clap(long, hide = true)]
    pub pkg_inference_root: Option<String>,
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

        track_usage!(telemetry, self.continue_execution, |val| matches!(
            val,
            ContinueMode::Always | ContinueMode::DependenciesSuccessful
        ));
        telemetry.track_arg_value(
            "continue-execution-strategy",
            self.continue_execution,
            EventType::NonSensitive,
        );

        track_usage!(telemetry, self.single_package, |val| val);
        track_usage!(telemetry, self.only, |val| val);
        track_usage!(telemetry, &self.cache_dir, Option::is_some);
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

        if let Some(log_order) = self.log_order {
            telemetry.track_arg_value("log-order", log_order, EventType::NonSensitive);
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
    ArgGroup::new("shard-spec-group").multiple(false).required(false),
])]
pub struct RunArgs {
    /// Set the cache behavior for this run. Pass a list of comma-separated key,
    /// value pairs to enable reading and writing to either the local or
    /// remote cache.
    #[clap(long, conflicts_with_all = &["force", "remote_only", "remote_cache_read_only", "no_cache"])]
    pub cache: Option<String>,
    /// Ignore the existing cache (to force execution). Equivalent to
    /// `--cache=local:w,remote:w`
    #[clap(long, default_missing_value = "true")]
    pub force: Option<Option<bool>>,
    /// Ignore the local filesystem cache for all tasks. Only
    /// allow reading and caching artifacts using the remote cache.
    /// Equivalent to `--cache=remote:rw`
    #[clap(long, default_missing_value = "true", group = "cache-group")]
    pub remote_only: Option<Option<bool>>,
    /// Treat remote cache as read only. Equivalent to
    /// `--cache=remote:r;local:rw`
    #[clap(long, default_missing_value = "true")]
    pub remote_cache_read_only: Option<Option<bool>>,
    /// Avoid saving task results to the cache. Useful for development/watch
    /// tasks. Equivalent to `--cache=local:r,remote:r`
    #[clap(long)]
    pub no_cache: bool,

    /// Set the number of concurrent cache operations (default 10)
    #[clap(long, default_value_t = DEFAULT_NUM_WORKERS)]
    pub cache_workers: u32,
    #[clap(alias = "dry", long = "dry-run", num_args = 0..=1, default_missing_value = "text")]
    pub dry_run: Option<DryRunMode>,
    /// Generate a graph of the task execution and output to a file when a
    /// filename is specified (.svg, .html, .mermaid, .dot). Outputs dot graph
    /// to stdout when no filename is provided.
    /// [DEPRECATED formats: .png, .jpg, .pdf, .json -- will be removed in 3.0]
    #[clap(long, num_args = 0..=1, default_missing_value = "", value_parser = validate_graph_extension)]
    pub graph: Option<String>,
    // clap does not have negation flags such as --daemon and --no-daemon
    // so we need to use a group to enforce that only one of them is set.
    // -----------------------
    /// [DEPRECATED] The daemon is no longer used for `turbo run`.
    /// This flag will be removed in version 3.0.
    #[clap(long, group = "daemon-group")]
    pub daemon: bool,

    /// [DEPRECATED] The daemon is no longer used for `turbo run`.
    /// This flag will be removed in version 3.0.
    #[clap(long, group = "daemon-group")]
    pub no_daemon: bool,

    /// File to write turbo's performance profile output into.
    /// You can load the file up in chrome://tracing to see
    /// which parts of your build were slow.
    #[clap(long, num_args = 0..=1, default_missing_value = "", conflicts_with = "anon_profile")]
    pub profile: Option<String>,
    /// File to write turbo's performance profile output into.
    /// All identifying data omitted from the profile.
    #[clap(long, num_args = 0..=1, default_missing_value = "", conflicts_with = "profile")]
    pub anon_profile: Option<String>,
    /// Generate a summary of the turbo run
    #[clap(long, default_missing_value = "true")]
    pub summarize: Option<Option<bool>>,

    /// [DEPRECATED] Execute all tasks in parallel. Use task configuration
    /// (`persistent`, `with`) instead.
    #[clap(long)]
    pub parallel: bool,

    /// Execute only the given shard of the task graph (1-based). Requires
    /// `--max-shards` or `--max-nodes-per-shard` to determine how many shards
    /// the graph is divided into.
    #[clap(long, requires = "shard-spec-group")]
    pub shard: Option<usize>,

    /// Divide the task graph into at most this many shards, balancing the
    /// number of tasks across them. Mutually exclusive with
    /// `--max-nodes-per-shard`.
    #[clap(long, group = "shard-spec-group")]
    pub max_shards: Option<usize>,

    /// Divide the task graph into as many shards as needed so each shard holds
    /// at most this many task nodes. Mutually exclusive with `--max-shards`.
    #[clap(long, group = "shard-spec-group")]
    pub max_nodes_per_shard: Option<usize>,
}

impl Default for RunArgs {
    fn default() -> Self {
        Self {
            remote_only: None,
            cache: None,
            force: None,
            cache_workers: DEFAULT_NUM_WORKERS,
            dry_run: None,
            graph: None,
            no_cache: false,
            daemon: false,
            no_daemon: false,
            profile: None,
            anon_profile: None,
            remote_cache_read_only: None,
            summarize: None,
            parallel: false,
            shard: None,
            max_shards: None,
            max_nodes_per_shard: None,
        }
    }
}

impl RunArgs {
    pub fn remote_only(&self) -> Option<bool> {
        let remote_only = self.remote_only?;
        Some(remote_only.unwrap_or(true))
    }

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

    pub fn profile_file_and_include_args(&self) -> Option<(String, bool)> {
        let resolve = |file: &str| -> String {
            if file.is_empty() {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |duration| duration.as_millis());
                format!("profile.{now}")
            } else {
                file.to_string()
            }
        };

        match (self.profile.as_deref(), self.anon_profile.as_deref()) {
            (Some(file), None) => Some((resolve(file), true)),
            (None, Some(file)) => Some((resolve(file), false)),
            (Some(_), Some(_)) => unreachable!(),
            (None, None) => None,
        }
    }

    pub fn remote_cache_read_only(&self) -> Option<bool> {
        let remote_cache_read_only = self.remote_cache_read_only?;
        Some(remote_cache_read_only.unwrap_or(true))
    }

    pub fn summarize(&self) -> Option<bool> {
        let summarize = self.summarize?;
        Some(summarize.unwrap_or(true))
    }

    pub fn track(&self, telemetry: &CommandEventBuilder) {
        // default to true
        track_usage!(telemetry, self.no_cache, |val| val);
        track_usage!(telemetry, self.remote_only().unwrap_or_default(), |val| val);
        track_usage!(telemetry, &self.force, Option::is_some);
        track_usage!(telemetry, self.daemon, |val| val);
        track_usage!(telemetry, self.no_daemon, |val| val);
        track_usage!(telemetry, self.parallel, |val| val);
        track_usage!(
            telemetry,
            self.remote_cache_read_only().unwrap_or_default(),
            |val| val
        );

        // default to None
        track_usage!(telemetry, &self.profile, Option::is_some);
        track_usage!(telemetry, &self.anon_profile, Option::is_some);
        track_usage!(telemetry, &self.summarize, Option::is_some);

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
