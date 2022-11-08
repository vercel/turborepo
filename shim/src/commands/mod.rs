pub(crate) mod bin;

use clap::{Parser, Subcommand};
use serde::Serialize;

#[derive(Parser, Clone, Default, Debug, PartialEq, Serialize)]
#[clap(author, about = "The build system that makes ship happen", long_about = None)]
#[clap(disable_help_subcommand = true)]
#[clap(disable_version_flag = true)]
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
    #[clap(subcommand)]
    pub command: Option<Command>,
    pub tasks: Vec<String>,
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

/// Defines the subcommands for CLI. NOTE: If we change the commands in Go,
/// we must change these as well to avoid accidentally passing the
/// --single-package flag into non-build commands.
#[derive(Subcommand, Clone, Debug, Serialize, PartialEq)]
pub enum Command {
    /// Get the path to the Turbo binary
    Bin,
    /// Generate the autocompletion script for the specified shell
    Completion,
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
    Logout,
    /// Prepare a subset of your monorepo.
    Prune {
        #[clap(long)]
        scope: Option<String>,
        #[clap(long)]
        docker: bool,
        #[clap(long = "out-dir", default_value_t = String::from("out"), value_parser)]
        output_dir: String,
    },
    /// Run tasks across projects in your monorepo
    Run {
        /// Override the filesystem cache directory.
        #[clap(long = "cache-dir")]
        cache_dir: Option<String>,
        /// Set the number of concurrent cache operations (default 10)
        #[clap(long = "cache-workers", default_value_t = 10)]
        cache_workers: u32,
        /// Limit the concurrency of task execution. Use 1 for serial (i.e.
        /// one-at-a-time) execution.
        #[clap(long = "concurrency")]
        concurrency: Option<String>,
        /// Continue execution even if a task exits with an error or non-zero
        /// exit code. The default behavior is to bail
        #[clap(long = "continue")]
        continue_execution: bool,
        #[clap(long = "dry-run")]
        dry_run: Option<String>,
        /// Use the given selector to specify package(s) to act as
        /// entry points. The syntax mirrors pnpm's syntax, and
        /// additional documentation and examples can be found in
        /// turbo's documentation https://turbo.build/repo/docs/reference/command-line-reference#--filter
        #[clap(long)]
        filter: Option<String>,
        /// Ignore the existing cache (to force execution)
        #[clap(long)]
        force: bool,
        /// Specify glob of global filesystem dependencies to be hashed. Useful
        /// for .env and files
        #[clap(long = "global-deps")]
        global_deps: Vec<String>,
        /// Generate a graph of the task execution and output to a file when a
        /// filename is specified (.svg, .png, .jpg, .pdf, .json,
        /// .html). Outputs dot graph to stdout when if no filename is provided
        #[clap(long, num_args = 0..=1, require_equals = true, default_missing_value = "stdout")]
        graph: Option<String>,
        /// Files to ignore when calculating changed files (i.e. --since).
        /// Supports globs.
        #[clap(long)]
        ignore: Vec<String>,
        /// Include the dependencies of tasks in execution.
        #[clap(long = "ignore-dependencies")]
        include_dependencies: bool,
        /// Avoid saving task results to the cache. Useful for development/watch
        /// tasks.
        #[clap(long = "no-cache")]
        no_cache: bool,
        /// Run without using turbo's daemon process
        #[clap(long = "no-daemon")]
        no_daemon: bool,
        /// Exclude dependent task consumers from execution.
        #[clap(long = "no-deps")]
        no_deps: bool,
        /// Set type of process output logging. Use "full" to show
        /// all output. Use "hash-only" to show only turbo-computed
        /// task hashes. Use "new-only" to show only new output with
        /// only hashes for cached tasks. Use "none" to hide process
        /// output. (default full)
        #[clap(long = "output-logs")]
        output_logs: Option<String>,
        /// Execute all tasks in parallel.
        #[clap(long)]
        parallel: bool,
        /// File to write turbo's performance profile output into.
        /// You can load the file up in chrome://tracing to see
        /// which parts of your build were slow.
        #[clap(long)]
        profile: Option<String>,
        /// Ignore the local filesystem cache for all tasks. Only
        /// allow reading and caching artifacts using the remote cache.
        #[clap(long = "remote-only")]
        remote_only: bool,
        /// Specify package(s) to act as entry points for task execution.
        /// Supports globs.
        #[clap(long)]
        scope: Vec<String>,
        /// Limit/Set scope to changed packages since a mergebase.
        /// This uses the git diff ${target_branch}... mechanism
        /// to identify which packages have changed.
        #[clap(long)]
        since: Option<String>,
        tasks: Vec<String>,
    },
    /// Unlink the current directory from your Vercel organization and disable
    /// Remote Caching
    Unlink,
}
