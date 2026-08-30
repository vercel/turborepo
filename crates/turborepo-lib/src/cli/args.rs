use std::{env, ffi::OsString, fmt, str::FromStr};

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;
use tracing::{error, log::warn};
use turborepo_telemetry::{
    events::{command::CommandEventBuilder, generic::GenericEventBuilder, EventType},
    track_usage,
};
use turborepo_types::{
    ContinueMode, DryRunMode, EnvMode, LogOrder, LogPrefix, OutputLogsMode, UIMode,
};
use usage::{Args as UsageArgs, Cli, Subcommands, ValueEnum};

use super::{exit_with_heap_profile, observability};
use crate::{commands::prune, get_version};

const DEFAULT_NUM_WORKERS: u32 = 10;
const SUPPORTED_GRAPH_FILE_EXTENSIONS: [&str; 8] =
    ["svg", "png", "jpg", "pdf", "json", "html", "mermaid", "dot"];

/// The parsed arguments from the command line. In general we should avoid using
/// or mutating this directly, and instead use the fully canonicalized `Opts`
/// struct.
#[derive(Cli, Clone, Default, Debug, PartialEq)]
#[usage(author = "Vercel", about = "The build system that makes ship happen")]
#[usage(disable_help_subcommand = true)]
#[usage(disable_version_flag = true)]
#[usage(
    unknown_flags = "error",
    args_override_self = false,
    default_subcommand = "run",
    completion
)]
#[usage(arg_required_else_help = true)]
#[usage(name = "turbo")]
pub struct Args {
    #[usage(long, global = true)]
    pub version: bool,
    #[usage(long, global = true)]
    /// Skip any attempts to infer which version of Turbo the project is
    /// configured to use
    pub skip_infer: bool,
    /// Disable the turbo update notification
    #[usage(long, global = true)]
    pub no_update_notifier: bool,
    /// Override the endpoint for API calls
    #[usage(long, global = true)]
    pub api: Option<String>,
    /// Force color usage in the terminal
    #[usage(long, global = true)]
    pub color: bool,
    /// The directory in which to run turbo
    #[usage(long, global = true)]
    pub cwd: Option<Utf8PathBuf>,
    /// Specify a file to save a pprof heap profile
    #[usage(long, global = true)]
    pub heap: Option<String>,
    /// Specify whether to use the streaming UI or TUI
    #[usage(long, global = true)]
    pub ui: Option<UiModeArg>,
    /// Override the login endpoint
    #[usage(long, global = true)]
    pub login: Option<String>,
    /// Suppress color usage in the terminal
    #[usage(long, global = true)]
    pub no_color: bool,
    /// When enabled, turbo will precede HTTP requests with an OPTIONS request
    /// for authorization
    #[usage(long, global = true)]
    pub preflight: bool,
    /// Set a timeout for all HTTP requests.
    #[usage(long, value_name = "TIMEOUT", global = true)]
    pub remote_cache_timeout: Option<u64>,
    /// Set the team slug for API calls
    #[usage(long, global = true)]
    pub team: Option<String>,
    /// Set the auth token for API calls
    #[usage(long, global = true)]
    pub token: Option<String>,
    /// Specify a file to save a pprof trace
    #[usage(long, global = true)]
    pub trace: Option<String>,
    /// verbosity
    #[usage(flatten)]
    pub verbosity: Verbosity,
    #[usage(flatten)]
    pub experimental_otel_args: observability::ExperimentalOtelCliArgs,
    /// Force a check for a new version of turbo
    #[usage(long, global = true, hide = true)]
    pub check_for_update: bool,
    #[usage(long = "__test-run", global = true, hide = true)]
    pub test_run: bool,
    /// Allow for missing `packageManager` in `package.json`.
    ///
    /// `turbo` will use hints from codebase to guess which package manager
    /// should be used.
    #[usage(long, global = true)]
    pub dangerously_disable_package_manager_check: bool,
    #[usage(long = "experimental-allow-no-turbo-json", hide = true, global = true)]
    pub allow_no_turbo_json: bool,
    /// Use the `turbo.json` located at the provided path instead of one at the
    /// root of the repository.
    #[usage(long, global = true)]
    pub root_turbo_json: Option<Utf8PathBuf>,
    /// The legacy flag is stripped before clap parsing. Non-run commands use
    /// this field because they have no command-local `ExecutionArgs`.
    #[usage(skip)]
    pub(crate) single_package: bool,
    #[usage(skip)]
    pub(crate) config_execution_args: Option<ExecutionArgs>,
    #[usage(skip)]
    pub(crate) config_run_args: Option<RunArgs>,
    #[usage(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, UsageArgs, Clone, Copy, PartialEq, Eq, Default)]
pub struct Verbosity {
    #[usage(
        long = "verbosity",
        global = true,
        conflicts = "v",
        value_name = "COUNT"
    )]
    /// Verbosity level. Useful when debugging Turborepo or creating logs for
    /// issue reports
    pub verbosity: Option<u8>,
    #[usage(
        short = 'v',
        count,
        global = true,
        hide = true,
        conflicts = "verbosity"
    )]
    pub v: u8,
}

impl From<Verbosity> for u8 {
    fn from(val: Verbosity) -> Self {
        let Verbosity { verbosity, v } = val;
        verbosity.unwrap_or(v)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContinueModeArg {
    #[default]
    Never,
    DependenciesSuccessful,
    Always,
}
impl FromStr for ContinueModeArg {
    type Err = String;
    fn from_str(v: &str) -> Result<Self, String> {
        match v {
            "never" => Ok(Self::Never),
            "dependencies-successful" => Ok(Self::DependenciesSuccessful),
            "always" => Ok(Self::Always),
            _ => Err(format!("invalid continue mode: {v}")),
        }
    }
}
impl From<ContinueModeArg> for ContinueMode {
    fn from(v: ContinueModeArg) -> Self {
        match v {
            ContinueModeArg::Never => Self::Never,
            ContinueModeArg::DependenciesSuccessful => Self::DependenciesSuccessful,
            ContinueModeArg::Always => Self::Always,
        }
    }
}
impl fmt::Display for ContinueModeArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Never => "never",
            Self::DependenciesSuccessful => "dependencies-successful",
            Self::Always => "always",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvModeArg {
    Loose,
    Strict,
}
impl FromStr for EnvModeArg {
    type Err = String;
    fn from_str(v: &str) -> Result<Self, String> {
        match v {
            "loose" => Ok(Self::Loose),
            "strict" => Ok(Self::Strict),
            _ => Err(format!("invalid env mode: {v}")),
        }
    }
}
impl From<EnvModeArg> for EnvMode {
    fn from(v: EnvModeArg) -> Self {
        match v {
            EnvModeArg::Loose => Self::Loose,
            EnvModeArg::Strict => Self::Strict,
        }
    }
}
impl fmt::Display for EnvModeArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Loose => "loose",
            Self::Strict => "strict",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogOrderArg {
    Auto,
    Stream,
    Grouped,
}
impl FromStr for LogOrderArg {
    type Err = String;
    fn from_str(v: &str) -> Result<Self, String> {
        match v {
            "auto" => Ok(Self::Auto),
            "stream" => Ok(Self::Stream),
            "grouped" => Ok(Self::Grouped),
            _ => Err(format!("invalid log order: {v}")),
        }
    }
}
impl From<LogOrderArg> for LogOrder {
    fn from(v: LogOrderArg) -> Self {
        match v {
            LogOrderArg::Auto => Self::Auto,
            LogOrderArg::Stream => Self::Stream,
            LogOrderArg::Grouped => Self::Grouped,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LogPrefixArg {
    #[default]
    Auto,
    None,
    Task,
}
impl FromStr for LogPrefixArg {
    type Err = String;
    fn from_str(v: &str) -> Result<Self, String> {
        match v {
            "auto" => Ok(Self::Auto),
            "none" => Ok(Self::None),
            "task" => Ok(Self::Task),
            _ => Err(format!("invalid log prefix: {v}")),
        }
    }
}
impl From<LogPrefixArg> for LogPrefix {
    fn from(v: LogPrefixArg) -> Self {
        match v {
            LogPrefixArg::Auto => Self::Auto,
            LogPrefixArg::None => Self::None,
            LogPrefixArg::Task => Self::Task,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputLogsModeArg {
    Full,
    None,
    HashOnly,
    NewOnly,
    ErrorsOnly,
}
impl FromStr for OutputLogsModeArg {
    type Err = String;
    fn from_str(v: &str) -> Result<Self, String> {
        match v {
            "full" => Ok(Self::Full),
            "none" => Ok(Self::None),
            "hash-only" => Ok(Self::HashOnly),
            "new-only" => Ok(Self::NewOnly),
            "errors-only" => Ok(Self::ErrorsOnly),
            _ => Err(format!("invalid output logs mode: {v}")),
        }
    }
}
impl From<OutputLogsModeArg> for OutputLogsMode {
    fn from(v: OutputLogsModeArg) -> Self {
        match v {
            OutputLogsModeArg::Full => Self::Full,
            OutputLogsModeArg::None => Self::None,
            OutputLogsModeArg::HashOnly => Self::HashOnly,
            OutputLogsModeArg::NewOnly => Self::NewOnly,
            OutputLogsModeArg::ErrorsOnly => Self::ErrorsOnly,
        }
    }
}

impl fmt::Display for LogOrderArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Auto => "auto",
            Self::Stream => "stream",
            Self::Grouped => "grouped",
        })
    }
}
impl fmt::Display for LogPrefixArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Auto => "auto",
            Self::None => "none",
            Self::Task => "task",
        })
    }
}
impl fmt::Display for OutputLogsModeArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Full => "full",
            Self::None => "none",
            Self::HashOnly => "hash-only",
            Self::NewOnly => "new-only",
            Self::ErrorsOnly => "errors-only",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiModeArg {
    Tui,
    Stream,
}
impl FromStr for UiModeArg {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "tui" => Ok(Self::Tui),
            "stream" => Ok(Self::Stream),
            _ => Err(format!("invalid UI mode: {value}")),
        }
    }
}
impl From<UiModeArg> for UIMode {
    fn from(value: UiModeArg) -> Self {
        match value {
            UiModeArg::Tui => Self::Tui,
            UiModeArg::Stream => Self::Stream,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DryRunModeArg {
    Text,
    Json,
}
impl FromStr for DryRunModeArg {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            _ => Err(format!("invalid dry-run mode: {value}")),
        }
    }
}
impl From<DryRunModeArg> for DryRunMode {
    fn from(value: DryRunModeArg) -> Self {
        match value {
            DryRunModeArg::Text => Self::Text,
            DryRunModeArg::Json => Self::Json,
        }
    }
}
impl fmt::Display for DryRunModeArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Text => "text",
            Self::Json => "json",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionShell {
    Bash,
    Elvish,
    Fish,
    PowerShell,
    Zsh,
}
impl FromStr for CompletionShell {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "bash" => Ok(Self::Bash),
            "elvish" => Ok(Self::Elvish),
            "fish" => Ok(Self::Fish),
            "powershell" => Ok(Self::PowerShell),
            "zsh" => Ok(Self::Zsh),
            _ => Err(format!("unsupported shell: {value}")),
        }
    }
}
impl From<CompletionShell> for usage::complete::Shell {
    fn from(value: CompletionShell) -> Self {
        match value {
            CompletionShell::Bash => Self::Bash,
            CompletionShell::Elvish => Self::Elvish,
            CompletionShell::Fish => Self::Fish,
            CompletionShell::PowerShell => Self::PowerShell,
            CompletionShell::Zsh => Self::Zsh,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NonEmptyPath(pub(crate) Utf8PathBuf);
impl FromStr for NonEmptyPath {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty() {
            Err("path must not be empty".into())
        } else {
            Ok(Self(value.into()))
        }
    }
}
impl AsRef<Utf8Path> for NonEmptyPath {
    fn as_ref(&self) -> &Utf8Path {
        &self.0
    }
}
impl From<NonEmptyPath> for Utf8PathBuf {
    fn from(value: NonEmptyPath) -> Self {
        value.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphOutput(pub(crate) String);
impl FromStr for GraphOutput {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if let Some(extension) = Utf8Path::new(value).extension() {
            if !SUPPORTED_GRAPH_FILE_EXTENSIONS.contains(&extension) {
                return Err(format!(
                    "Unsupported file extension: {extension}. Supported extensions are: {}",
                    SUPPORTED_GRAPH_FILE_EXTENSIONS.join(", ")
                ));
            }
        }
        Ok(Self(value.into()))
    }
}
impl AsRef<str> for GraphOutput {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
impl std::ops::Deref for GraphOutput {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

#[derive(Subcommands, Copy, Clone, Debug, PartialEq)]
pub enum DaemonCommand {
    /// Restarts the turbo daemon
    Restart,
    /// Ensures that the turbo daemon is running
    Start,
    /// Reports the status of the turbo daemon
    Status {
        /// Pass --json to report status in JSON format
        #[usage(long)]
        json: bool,
    },
    /// Stops the turbo daemon
    Stop,
    /// Stops the turbo daemon if it is already running, and removes any stale
    /// daemon state
    Clean {
        /// Clean
        #[usage(long, default = "true")]
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

impl FromStr for OutputFormat {
    type Err = String;
    fn from_str(v: &str) -> Result<Self, String> {
        match v {
            "pretty" => Ok(Self::Pretty),
            "json" => Ok(Self::Json),
            _ => Err(format!("invalid output format: {v}")),
        }
    }
}
impl FromStr for BoundariesIgnore {
    type Err = String;
    fn from_str(v: &str) -> Result<Self, String> {
        match v {
            "all" => Ok(Self::All),
            "prompt" => Ok(Self::Prompt),
            _ => Err(format!("invalid ignore mode: {v}")),
        }
    }
}

#[derive(Subcommands, Copy, Clone, Debug, PartialEq)]
pub enum TelemetryCommand {
    /// Enables anonymous telemetry
    Enable,
    /// Disables anonymous telemetry
    Disable,
    /// Reports the status of telemetry
    Status,
}

pub(super) fn unwrap_flag_help(help: &str) -> String {
    let mut output = String::with_capacity(help.len());
    let mut joining_flag = false;

    for line in help.lines() {
        let trimmed = line.trim_start();
        let is_flag = trimmed.starts_with('-');
        let is_continuation =
            joining_flag && line.starts_with("                                  ");

        if is_continuation {
            output.push(' ');
            output.push_str(trimmed);
        } else {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(line);
        }
        joining_flag = is_flag || is_continuation;
    }
    output.push('\n');
    output
}

impl Args {
    #[tracing::instrument(skip_all)]
    pub fn new(os_args: Vec<OsString>) -> Self {
        if os_args.len() == 1 {
            if let Some(help) = Args::render_help(Args::command(), false) {
                eprint!("{help}");
            }
            exit_with_heap_profile(1);
        }

        let help_requested = os_args
            .iter()
            .take_while(|arg| *arg != "--")
            .any(|arg| matches!(arg.to_str(), Some("-h" | "--help")));
        if help_requested {
            let help_args: Vec<_> = os_args.into_iter().skip(1).collect();
            if let usage::embedded::Outcome::Exit(exit) = Args::embedded_outcome(&help_args) {
                print!("{}", unwrap_flag_help(&exit.text));
                exit_with_heap_profile(exit.code);
            }
            unreachable!("a help flag must produce an exit outcome");
        }

        let clap_args = match Args::parse_args(os_args) {
            Ok(args) => args,
            Err(error_text) => {
                error!(
                    "{}",
                    error_text.strip_prefix("error: ").unwrap_or(&error_text)
                );
                exit_with_heap_profile(1);
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

    pub(crate) fn parse_args(os_args: Vec<OsString>) -> Result<Self, String> {
        let (is_single_package, single_package_free) = Self::remove_single_package(os_args);
        let mut words: Vec<OsString> = single_package_free.collect();
        let config_flags = [
            "--cache",
            "--cache-dir",
            "--concurrency",
            "--daemon",
            "--env-mode",
            "--force",
            "--log-order",
            "--no-daemon",
            "--remote-cache-read-only",
            "--remote-only",
            "--summarize",
        ];
        let trailing_config = words.last().is_some_and(|word| word == "config")
            && words[1..words.len() - 1].iter().any(|word| {
                word.to_str().is_some_and(|word| {
                    let flag = word.split_once('=').map_or(word, |(flag, _)| flag);
                    config_flags.contains(&flag)
                })
            });
        if trailing_config {
            words.pop();
            words.insert(1, OsString::from("__turbo_config_options"));
        }
        Self::reject_duplicate_scalar_flags(&words)?;
        for word in &words {
            if matches!(
                word.to_str(),
                Some(
                    "--preflight=true"
                        | "--preflight=false"
                        | "--color=true"
                        | "--color=false"
                        | "--no-color=true"
                        | "--no-color=false"
                )
            ) {
                return Err(format!(
                    "error: unexpected value for switch '{}'",
                    word.to_string_lossy()
                ));
            }
        }
        let refs: Vec<&std::ffi::OsStr> = words.iter().map(OsString::as_os_str).collect();
        let mut args = Args::try_parse_from(&refs)
            .map_err(|error| Args::render_failure(&refs[1..], &error))?;
        if trailing_config {
            let Some(Command::Run {
                mut execution_args,
                run_args,
            }) = args.command.take()
            else {
                return Err("error: expected config options to parse as run options".to_string());
            };
            execution_args.tasks.clear();
            args.command = Some(Command::Config);
            args.config_execution_args = Some(execution_args);
            args.config_run_args = Some(run_args);
        }
        if let Some(Command::Query {
            subcommand: Some(QuerySubcommand::Affected(affected)),
            ..
        }) = args.command.as_mut()
        {
            for values in [&mut affected.packages, &mut affected.tasks]
                .into_iter()
                .flatten()
            {
                values.retain(|value| !value.is_empty());
            }
        }
        // --single-package is stripped before clap parsing, so we need to
        // propagate it back. Preserve clap's optional global execution args;
        // creating them here conflicts with explicit run/watch arguments.
        args.single_package = is_single_package;
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

        Ok(args)
    }

    fn reject_duplicate_scalar_flags(words: &[OsString]) -> Result<(), String> {
        use std::collections::HashSet;

        let repeatable = [
            "--experimental-otel-header",
            "--experimental-otel-resource",
            "--filter",
            "--global-deps",
            "--packages",
            "--tasks",
        ];
        let mut seen = HashSet::new();
        for word in words.iter().skip(1).take_while(|word| *word != "--") {
            let Some(word) = word.to_str() else { continue };
            let flag = word.split_once('=').map_or(word, |(flag, _)| flag);
            if !flag.starts_with("--") || repeatable.contains(&flag) {
                continue;
            }
            if !seen.insert(flag) {
                return Err(format!(
                    "error: the argument '{flag}' cannot be used multiple times"
                ));
            }
        }
        Ok(())
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
        match &self.command {
            Some(Command::Run { run_args, .. }) => Some(run_args),
            Some(Command::Config) => self.config_run_args.as_ref(),
            _ => None,
        }
    }

    /// Fetch the execution args supplied to the command
    pub fn execution_args(&self) -> Option<&ExecutionArgs> {
        match &self.command {
            Some(Command::Run { execution_args, .. }) => Some(execution_args),
            Some(Command::Watch { execution_args, .. }) => Some(execution_args),
            Some(Command::Config) => self.config_execution_args.as_ref(),
            _ => None,
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
}

/// Defines the subcommandsds for CLI
#[derive(Subcommands, Clone, Debug, PartialEq)]
pub enum Command {
    /// Get the path to the Turbo binary
    Bin,
    /// Get the port assigned to the current microfrontend
    #[usage(name = "get-mfe-port")]
    GetMfePort,
    #[usage(hide = true)]
    Boundaries {
        #[usage(short = 'F', long)]
        filter: Vec<String>,
        #[usage(long, value_enum, default_missing = "prompt", num_args = 0..=1, require_equals = true)]
        ignore: Option<BoundariesIgnore>,
        #[usage(long, requires = "ignore")]
        reason: Option<String>,
    },
    /// Generate the autocompletion script for the specified shell
    Completion { shell: CompletionShell },
    /// Runs the Turborepo background daemon
    Daemon {
        /// Set the idle timeout for turbod
        #[usage(long, default_value_t = String::from("4h0m0s"), default = "4h0m0s")]
        idle_time: String,
        /// Path to a custom turbo.json file to watch from --root-turbo-json
        #[usage(long)]
        turbo_json_path: Option<Utf8PathBuf>,
        #[usage(subcommand)]
        command: Option<DaemonCommand>,
    },
    /// Visualize your monorepo's package graph in the browser
    Devtools {
        /// Port for the WebSocket server
        #[usage(long, default_value_t = turborepo_devtools::DEFAULT_PORT, default = "9789")]
        port: u16,
        /// Don't automatically open the browser
        #[usage(long)]
        no_open: bool,
    },
    /// Search the Turborepo documentation
    Docs {
        /// The search query
        query: String,
        /// Override the docs version (minimum: 2.7.5)
        #[usage(long)]
        docs_version: Option<String>,
    },
    /// Generate a new app / package
    #[usage(aliases = ["g", "gen"])]
    Generate {
        #[usage(long, hide = true)]
        tag: Option<String>,
        /// The name of the generator to run
        generator_name: Option<String>,
        /// Generator configuration file
        #[usage(short = 'c', long)]
        config: Option<String>,
        /// The root of your repository (default: directory with root
        /// turbo.json)
        #[usage(short = 'r', long)]
        root: Option<String>,
        /// Answers passed directly to generator
        #[usage(short = 'a', long, num_args = 1..)]
        args: Vec<String>,

        #[usage(subcommand)]
        command: Option<GenerateCommand>,
    },
    /// Enable or disable anonymous telemetry
    Telemetry {
        #[usage(subcommand)]
        command: Option<TelemetryCommand>,
    },
    /// [DEPRECATED] `turbo scan` has been removed. This command will be
    /// fully removed in a future major version.
    #[usage(hide = true)]
    Scan,
    #[usage(hide = true)]
    Config,
    /// List packages in your monorepo.
    Ls {
        /// Show only packages that are affected by changes between
        /// the current branch and `main`
        #[usage(long)]
        affected: bool,
        /// Use the given selector to specify package(s) to act as
        /// entry points. The syntax mirrors pnpm's syntax, and
        /// additional documentation and examples can be found in
        /// turbo's documentation https://turborepo.dev/docs/reference/command-line-reference/run#--filter
        #[usage(short = 'F', long)]
        filter: Vec<String>,
        /// Get insight into a specific package, such as
        /// its dependencies and tasks
        packages: Vec<String>,
        /// EXPERIMENTAL: Output format
        #[usage(long, value_enum)]
        output: Option<OutputFormat>,
    },
    /// Link your local directory to a Vercel organization and enable remote
    /// caching.
    Link {
        /// Do not create or modify .gitignore (default false)
        #[usage(long)]
        no_gitignore: bool,

        /// The scope, i.e. Vercel team, to which you are linking
        #[usage(long)]
        scope: Option<String>,

        /// Answer yes to all prompts (default false)
        #[usage(long, short)]
        yes: bool,
    },
    /// Login to your Vercel account
    Login {
        #[usage(long = "sso-team")]
        sso_team: Option<String>,
        /// Deprecated, no-op. Previously forced a new login even if a valid
        /// token existed.
        #[usage(long = "force", short = 'f', hide = true)]
        force: bool,
        /// Manually enter token instead of requesting one from the login
        /// service.
        #[usage(long, conflicts = "sso_team")]
        manual: bool,
    },
    /// Logout to your Vercel account
    Logout {
        /// Invalidate the token on the server. Pass `--invalidate=false` to
        /// skip the remote revoke.
        #[usage(long, value_name = "BOOL", default = "true", default_missing = "true", num_args = 0..=1)]
        invalidate: Option<bool>,
    },
    /// Print debugging information
    Info,
    /// Prepare a subset of your monorepo.
    Prune {
        /// DEPRECATED: Use positional arguments instead
        /// (e.g. `turbo prune web`)
        #[usage(hide = true, long)]
        scope: Option<Vec<String>>,
        /// Workspaces that should be included in the subset
        #[usage(required_unless("scope"), conflicts("scope"), value_name = "SCOPE")]
        scope_arg: Option<Vec<String>>,
        #[usage(long)]
        docker: bool,
        /// Exclude in-workspace devDependencies when selecting packages to
        /// include
        #[usage(long)]
        production: bool,
        #[usage(long = "out-dir", default_value_t = String::from(prune::DEFAULT_OUTPUT_DIR), default = "out")]
        output_dir: String,
        /// Respect `.gitignore` when copying files to <OUT-DIR>
        #[usage(long, default_missing = "true", num_args = 0..=1, require_equals = true)]
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
        #[usage(flatten)]
        run_args: RunArgs,
        #[usage(flatten)]
        execution_args: ExecutionArgs,
    },
    /// Query your monorepo using GraphQL. If no query is provided, spins up a
    /// GraphQL server with GraphiQL.
    Query {
        #[usage(subcommand)]
        subcommand: Option<QuerySubcommand>,
        /// Pass variables to the query via a JSON file
        #[usage(short = 'V', long, requires = "query")]
        variables: Option<Utf8PathBuf>,
        #[usage(long, conflicts = "query")]
        schema: bool,
        /// The query to run, either a file path or a query string
        query: Option<String>,
    },
    Watch {
        #[usage(flatten)]
        execution_args: ExecutionArgs,
        /// EXPERIMENTAL: Write to cache in watch mode.
        #[usage(long)]
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

#[derive(UsageArgs, Clone, Debug, Default, Serialize, PartialEq)]
pub struct GenerateWorkspaceArgs {
    /// Name for the new workspace
    #[usage(short = 'n', long)]
    pub name: Option<String>,
    /// Generate an empty workspace
    #[usage(short = 'b', long, conflicts = "copy")]
    pub empty: bool,
    /// Generate a workspace using an existing workspace as a template. Can be
    /// the name of a local workspace within your monorepo, or a fully
    /// qualified GitHub URL with any branch and/or subdirectory
    #[usage(short = 'c', long, conflicts = "empty", num_args = 0..=1, default_missing = "")]
    pub copy: Option<String>,
    /// Where the new workspace should be created
    #[usage(short = 'd', long)]
    pub destination: Option<String>,
    /// The type of workspace to create
    #[usage(short = 't', long)]
    pub r#type: Option<String>,
    /// The root of your repository (default: directory with root turbo.json)
    #[usage(short = 'r', long)]
    pub root: Option<String>,
    /// In a rare case, your GitHub URL might contain a branch name with a slash
    /// (e.g. bug/fix-1) and the path to the example (e.g. foo/bar). In this
    /// case, you must specify the path to the example separately:
    /// --example-path foo/bar
    #[usage(short = 'p', long)]
    pub example_path: Option<String>,
    /// Do not filter available dependencies by the workspace type
    #[usage(long)]
    pub show_all_dependencies: bool,
}

#[derive(UsageArgs, Clone, Debug, Default, PartialEq, Serialize)]
pub struct GeneratorCustomArgs {
    /// The name of the generator to run
    pub(super) generator_name: Option<String>,
    /// Generator configuration file
    #[usage(short = 'c', long)]
    pub(super) config: Option<String>,
    /// The root of your repository (default: directory with root
    /// turbo.json)
    #[usage(short = 'r', long)]
    pub(super) root: Option<String>,
    /// Answers passed directly to generator
    #[usage(short = 'a', long, delimiter = ' ', num_args = 1..)]
    pub(super) args: Vec<String>,
}

#[derive(Subcommands, Clone, Debug, PartialEq)]
pub enum GenerateCommand {
    /// Add a new package or app to your project
    #[usage(name = "workspace", alias = "w")]
    Workspace(GenerateWorkspaceArgs),
    #[usage(name = "run", alias = "r")]
    Run(GeneratorCustomArgs),
}

#[derive(Subcommands, Clone, Debug, PartialEq)]
pub enum QuerySubcommand {
    /// Check which packages or tasks are affected by changes between two git
    /// refs
    Affected(AffectedArgs),
    /// List packages in your monorepo (shorthand for a packages query)
    Ls(LsArgs),
}

#[derive(UsageArgs, Clone, Debug, PartialEq)]
pub struct LsArgs {
    /// Show only packages that are affected by changes between
    /// the current branch and `main`
    #[usage(long)]
    pub affected: bool,
    /// Use the given selector to specify package(s) to act as
    /// entry points. The syntax mirrors pnpm's syntax, and
    /// additional documentation and examples can be found in
    /// turbo's documentation https://turborepo.dev/docs/reference/command-line-reference/run#--filter
    #[usage(short = 'F', long)]
    pub filter: Vec<String>,
    /// Get insight into a specific package, such as
    /// its dependencies and tasks
    pub packages: Vec<String>,
    /// Output format
    #[usage(long, value_enum)]
    pub output: Option<OutputFormat>,
}

#[derive(UsageArgs, Clone, Debug, PartialEq)]
pub struct AffectedArgs {
    /// Return affected packages instead of tasks. Optionally filter by name.
    /// When combined with --tasks, returns affected tasks that match both
    /// the task name and package filters.
    #[usage(long, num_args = 0.., default_missing = "")]
    pub packages: Option<Vec<String>>,
    /// Filter to specific task names (e.g. build, test).
    /// When combined with --packages, returns affected tasks that match both
    /// the task name and package filters.
    #[usage(long, num_args = 0.., default_missing = "")]
    pub tasks: Option<Vec<String>>,
    /// Base git ref for comparison
    #[usage(long)]
    pub base: Option<String>,
    /// Head git ref for comparison
    #[usage(long)]
    pub head: Option<String>,
    /// Exit with code 1 when affected packages or tasks are found, 0 when
    /// none are found, or 2 on errors. Useful for CI gating. We recommend
    /// parsing the JSON output directly for more flexibility.
    #[usage(long)]
    pub exit_code: bool,
}

/// Arguments used in run and watch
#[derive(UsageArgs, Clone, Debug, Default, PartialEq)]
#[usage(args_override_self = false, group("scope-filter-group", multiple))]
pub struct ExecutionArgs {
    /// Override the filesystem cache directory.
    #[usage(long)]
    pub cache_dir: Option<NonEmptyPath>,
    /// Limit the concurrency of task execution. Use 1 for serial (i.e.
    /// one-at-a-time) execution.
    #[usage(long)]
    pub concurrency: Option<String>,
    /// Specify how task execution should proceed when an error occurs.
    /// Use "never" to cancel all tasks. Use "dependencies-successful" to
    /// continue running tasks whose dependencies have succeeded. Use "always"
    /// to continue running all tasks, even those whose dependencies have
    /// failed.
    #[usage(long = "continue", value_name = "CONTINUE", num_args = 0..=1, default = "never", default_missing = "always", require_equals = true)]
    pub continue_execution: ContinueModeArg,
    /// Run turbo in single-package mode
    #[usage(long)]
    pub single_package: bool,
    /// Specify whether or not to do framework inference for tasks
    #[usage(long, value_name = "BOOL", default = "true", default_missing = "true", num_args = 0..=1)]
    pub framework_inference: Option<bool>,
    /// Specify glob of global filesystem dependencies to be hashed. Useful
    /// for .env and files
    #[usage(long = "global-deps")]
    pub global_deps: Vec<String>,
    /// Environment variable mode.
    /// Use "loose" to pass the entire existing environment.
    /// Use "strict" to use an allowlist specified in turbo.json.
    #[usage(long = "env-mode", num_args = 0..=1, default_missing = "strict")]
    pub env_mode: Option<EnvModeArg>,
    /// Use the given selector to specify package(s) to act as
    /// entry points. The syntax mirrors pnpm's syntax, and
    /// additional documentation and examples can be found in
    /// turbo's documentation https://turborepo.dev/docs/reference/command-line-reference/run#--filter
    #[usage(short = 'F', long, group = "scope-filter-group")]
    pub filter: Vec<String>,

    /// Filter to only packages that are affected by changes between
    /// the current branch and `main`
    #[usage(long, group = "scope-filter-group")]
    pub affected: bool,

    /// Set type of process output logging. Use "full" to show
    /// all output. Use "hash-only" to show only turbo-computed
    /// task hashes. Use "new-only" to show only new output with
    /// only hashes for cached tasks. Use "none" to hide process
    /// output. (default full)
    #[usage(long)]
    pub output_logs: Option<OutputLogsModeArg>,
    /// Set type of task output order. Use "stream" to show
    /// output as soon as it is available. Use "grouped" to
    /// show output when a command has finished execution. Use "auto" to let
    /// turbo decide based on its own heuristics. (default auto)
    #[usage(long)]
    pub log_order: Option<LogOrderArg>,
    /// Output machine-readable NDJSON to stdout instead of human-readable
    /// text. Disables the TUI and forces stream mode.
    #[usage(long)]
    pub json: bool,
    /// Write structured JSON logs to a file. If no path is given, writes to
    /// `.turbo/logs/<epoch_millis>.json`.
    #[usage(long)]
    pub log_file: Option<Option<String>>,
    /// Only executes the tasks specified, does not execute parent tasks.
    #[usage(long)]
    pub only: bool,
    #[usage(long, hide = true)]
    pub pkg_inference_root: Option<String>,
    /// Use "none" to remove prefixes from task logs. Use "task" to get task id
    /// prefixing. Use "auto" to let turbo decide how to prefix the logs
    /// based on the execution environment. In most cases this will be the same
    /// as "task". Note that tasks running in parallel interleave their
    /// logs, so removing prefixes can make it difficult to associate logs
    /// with tasks. Use --log-order=grouped to prevent interleaving. (default
    /// auto)
    #[usage(long, default_value_t = LogPrefixArg::Auto, default = "auto")]
    pub log_prefix: LogPrefixArg,
    // NOTE: The following two are hidden because clap displays them in the help text incorrectly:
    // > Usage: turbo [OPTIONS] [TASKS]... [-- <FORWARDED_ARGS>...] [COMMAND]
    #[usage(hide = true)]
    pub tasks: Vec<String>,
    #[usage(double_dash = "required", hide = true)]
    pub pass_through_args: Vec<String>,
}

impl ExecutionArgs {
    fn track(&self, telemetry: &CommandEventBuilder) {
        // default to false
        track_usage!(
            telemetry,
            self.framework_inference.unwrap_or(true),
            |val: bool| !val
        );

        track_usage!(telemetry, self.continue_execution, |val| matches!(
            val,
            ContinueModeArg::Always | ContinueModeArg::DependenciesSuccessful
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

        if self.log_prefix != LogPrefixArg::default() {
            telemetry.track_arg_value("log-prefix", self.log_prefix, EventType::NonSensitive);
        }

        // track sizes
        if !self.filter.is_empty() {
            telemetry.track_arg_value("filter:length", self.filter.len(), EventType::NonSensitive);
        }
    }
}

#[derive(UsageArgs, Clone, Debug, PartialEq)]
#[usage(args_override_self = false, group("daemon-group"))]
pub struct RunArgs {
    /// Set the cache behavior for this run. Pass a list of comma-separated key,
    /// value pairs to enable reading and writing to either the local or
    /// remote cache.
    #[usage(long, conflicts = &["force", "remote_only", "remote_cache_read_only", "no_cache"])]
    pub cache: Option<String>,
    /// Ignore the existing cache (to force execution). Equivalent to
    /// `--cache=local:w,remote:w`
    #[usage(long, default_missing = "true")]
    pub force: Option<Option<bool>>,
    /// Ignore the local filesystem cache for all tasks. Only
    /// allow reading and caching artifacts using the remote cache.
    /// Equivalent to `--cache=remote:rw`
    #[usage(long, default_missing = "true")]
    pub remote_only: Option<Option<bool>>,
    /// Treat remote cache as read only. Equivalent to
    /// `--cache=remote:r;local:rw`
    #[usage(long, default_missing = "true")]
    pub remote_cache_read_only: Option<Option<bool>>,
    /// Avoid saving task results to the cache. Useful for development/watch
    /// tasks. Equivalent to `--cache=local:r,remote:r`
    #[usage(long)]
    pub no_cache: bool,

    /// Set the number of concurrent cache operations (default 10)
    #[usage(long, default_value_t = DEFAULT_NUM_WORKERS, default = "10")]
    pub cache_workers: u32,
    #[usage(alias = "dry", long = "dry-run", num_args = 0..=1, default_missing = "text")]
    pub dry_run: Option<DryRunModeArg>,
    /// Generate a graph of the task execution and output to a file when a
    /// filename is specified (.svg, .html, .mermaid, .dot). Outputs dot graph
    /// to stdout when no filename is provided.
    /// [DEPRECATED formats: .png, .jpg, .pdf, .json -- will be removed in 3.0]
    #[usage(long, num_args = 0..=1, default_missing = "")]
    pub graph: Option<GraphOutput>,
    // clap does not have negation flags such as --daemon and --no-daemon
    // so we need to use a group to enforce that only one of them is set.
    // -----------------------
    /// [DEPRECATED] The daemon is no longer used for `turbo run`.
    /// This flag will be removed in version 3.0.
    #[usage(long, group = "daemon-group")]
    pub daemon: bool,

    /// [DEPRECATED] The daemon is no longer used for `turbo run`.
    /// This flag will be removed in version 3.0.
    #[usage(long, group = "daemon-group")]
    pub no_daemon: bool,

    /// File to write turbo's performance profile output into.
    /// You can load the file up in chrome://tracing to see
    /// which parts of your build were slow.
    #[usage(long, num_args = 0..=1, default_missing = "", conflicts = "anon_profile")]
    pub profile: Option<String>,
    /// File to write turbo's performance profile output into.
    /// All identifying data omitted from the profile.
    #[usage(long, num_args = 0..=1, default_missing = "", conflicts = "profile")]
    pub anon_profile: Option<String>,
    /// Generate a summary of the turbo run
    #[usage(long, default_missing = "true")]
    pub summarize: Option<Option<bool>>,

    /// [DEPRECATED] Execute all tasks in parallel. Use task configuration
    /// (`persistent`, `with`) instead.
    #[usage(long)]
    pub parallel: bool,
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
