pub(crate) mod bin;

use clap::{Parser, Subcommand};
use serde::Serialize;

#[derive(Parser, Clone, Default, Debug, PartialEq, Serialize)]
#[clap(author, about = "The build system that makes ship happen", long_about = None)]
#[clap(
    ignore_errors = true,
    disable_help_flag = true,
    disable_help_subcommand = true
)]
#[clap(disable_version_flag = true)]
pub struct Args {
    #[clap(long, short)]
    pub help: bool,
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
    Bin {
        /// Help flag
        #[clap(long, short)]
        help: bool,
    },
    /// Generate the autocompletion script for the specified shell
    Completion {
        /// Help flag
        #[clap(long, short)]
        help: bool,
    },
    /// Runs the Turborepo background daemon
    Daemon {
        /// Help flag
        #[clap(long, short)]
        help: bool,
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
        /// help for link
        #[clap(long, short)]
        help: bool,
        /// Do not create or modify .gitignore (default false)
        #[clap(long)]
        no_gitignore: bool,
    },
    /// Login to your Vercel account
    Login {
        /// Help flag
        #[clap(long, short)]
        help: bool,
        #[clap(long = "sso-team")]
        sso_team: Option<String>,
    },
    /// Logout to your Vercel account
    Logout {
        /// Help flag
        #[clap(long, short)]
        help: bool,
    },
    /// Prepare a subset of your monorepo.
    Prune {
        /// Help flag
        #[clap(long, short)]
        help: bool,
        #[clap(long)]
        scope: Option<String>,
        #[clap(long)]
        docker: bool,
        #[clap(long = "out-dir", default_value_t = String::from("out"), value_parser)]
        output_dir: String,
    },
    /// Run tasks across projects in your monorepo
    Run {
        /// Help flag
        #[clap(long, short)]
        help: bool,
        tasks: Vec<String>,
    },
    /// Unlink the current directory from your Vercel organization and disable
    /// Remote Caching
    Unlink {
        /// Help flag
        #[clap(long, short)]
        help: bool,
    },
}
