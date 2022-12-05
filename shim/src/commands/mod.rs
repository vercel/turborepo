pub(crate) mod bin;

use std::{env, process};

use anyhow::Result;
use clap::{ArgAction, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;
use serde::Serialize;

use crate::get_version;

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

#[derive(Parser, Clone, Default, Debug, PartialEq, Serialize)]
#[clap(author, about = "The build system that makes ship happen", long_about = None)]
#[clap(disable_help_subcommand = true)]
#[clap(disable_version_flag = true)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    #[clap(long, global = true)]
    pub version: bool,
    /// Override the endpoint for API calls
    #[clap(long, global = true, value_parser)]
    pub api: Option<String>,
    /// Force color usage in the terminal
    #[clap(long, global = true)]
    pub color: bool,
    /// Specify a file to save a cpu profile
    #[clap(long, global = true, value_parser)]
    pub cpu_profile: Option<String>,
    /// The directory in which to run turbo
    #[clap(long, global = true, value_parser)]
    pub cwd: Option<String>,
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
    #[clap(short, long, global = true, value_parser)]
    pub verbosity: Option<u8>,
    #[clap(long = "__test-run", global = true, hide = true)]
    pub test_run: bool,
    #[clap(flatten, next_help_heading = "Run Arguments")]
    pub run_args: Option<RunArgs>,
    #[clap(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Clone, Debug, Serialize, PartialEq)]
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

impl Args {
    pub fn new() -> Result<Self> {
        let mut clap_args = Args::parse();
        // --version flag doesn't work with ignore_errors in clap, so we have to handle
        // it manually
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
    Run(RunArgs),
    /// Unlink the current directory from your Vercel organization and disable
    /// Remote Caching
    Unlink {},
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
    /// Use the given selector to specify package(s) to act as
    /// entry points. The syntax mirrors pnpm's syntax, and
    /// additional documentation and examples can be found in
    /// turbo's documentation https://turbo.build/repo/docs/reference/command-line-reference#--filter
    #[clap(long, action = ArgAction::Append)]
    pub filter: Vec<String>,
    /// Ignore the existing cache (to force execution)
    #[clap(long)]
    pub force: bool,
    /// Specify glob of global filesystem dependencies to be hashed. Useful
    /// for .env and files
    #[clap(long = "global-deps", action = ArgAction::Append)]
    pub global_deps: Vec<String>,
    /// Generate a graph of the task execution and output to a file when a
    /// filename is specified (.svg, .png, .jpg, .pdf, .json,
    /// .html). Outputs dot graph to stdout when if no filename is provided
    #[clap(long, num_args = 0..=1)]
    pub graph: Option<Option<String>>,
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
    #[clap(long, value_enum, default_value_t = OutputLogsMode::Full)]
    pub output_logs: OutputLogsMode,
    #[clap(long, hide = true)]
    pub only: bool,
    /// Execute all tasks in parallel.
    #[clap(long)]
    pub parallel: bool,
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
    /// Run turbo in single-package mode
    #[clap(long)]
    pub single_package: bool,
    // NOTE: The following two are hidden because clap displays them in the help text incorrectly:
    // > Usage: turbo [OPTIONS] [TASKS]... [-- <FORWARDED_ARGS>...] [COMMAND]
    #[clap(hide = true)]
    pub tasks: Vec<String>,
    #[clap(last = true, hide = true)]
    pub pass_through_args: Vec<String>,
}
