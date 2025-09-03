use std::backtrace;

use camino::Utf8PathBuf;
use serde::Serialize;
use thiserror::Error;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
use turborepo_api_client::APIAuth;
use turborepo_cache::{CacheOpts, RemoteCacheOpts};
use turborepo_task_id::{TaskId, TaskName};

use crate::{
    cli::{
        Command, ContinueMode, DryRunMode, EnvMode, ExecutionArgs, LogOrder, LogPrefix,
        OutputLogsMode, RunArgs,
    },
    config::{ConfigurationOptions, CONFIG_FILE},
    turbo_json::{FutureFlags, UIMode},
    Args,
};

#[derive(Debug, Error)]
pub enum Error {
    #[error("Expected `run` command.")]
    ExpectedRun(#[backtrace] backtrace::Backtrace),
    #[error(transparent)]
    ParseFloat(#[from] std::num::ParseFloatError),
    #[error(
        "Invalid percentage value for `--concurrency` flag. This should be a percentage of CPU \
         cores, between 1% and 100%: {1}"
    )]
    InvalidConcurrencyPercentage(#[backtrace] backtrace::Backtrace, f64),
    #[error(
        "Invalid value for `--concurrency` flag. This should be a positive integer greater than \
         or equal to 1: {1}"
    )]
    ConcurrencyOutOfBounds(#[backtrace] backtrace::Backtrace, String),
    #[error(
        "Cannot set `cache` config and other cache options (`force`, `remoteOnly`, \
         `remoteCacheReadOnly`) at the same time."
    )]
    OverlappingCacheOptions,
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
    #[error(transparent)]
    Config(#[from] crate::config::Error),
}

#[derive(Debug, Clone, Serialize)]
pub struct APIClientOpts {
    pub api_url: String,
    pub timeout: u64,
    pub upload_timeout: u64,
    pub token: Option<String>,
    pub team_id: Option<String>,
    pub team_slug: Option<String>,
    pub login_url: String,
    pub preflight: bool,
    pub sso_login_callback_port: Option<u16>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoOpts {
    pub root_turbo_json_path: AbsoluteSystemPathBuf,
    pub allow_no_package_manager: bool,
    pub allow_no_turbo_json: bool,
}

/// The fully resolved options for Turborepo. This is the combination of config,
/// including all the layers (env, args, defaults, etc.), and the command line
/// arguments.
#[derive(Debug, Clone, Serialize)]
pub struct Opts {
    pub repo_opts: RepoOpts,
    pub api_client_opts: APIClientOpts,
    pub cache_opts: CacheOpts,
    pub run_opts: RunOpts,
    pub runcache_opts: RunCacheOpts,
    pub scope_opts: ScopeOpts,
    pub tui_opts: TuiOpts,
    pub future_flags: FutureFlags,
}

impl Opts {
    pub fn synthesize_command(&self) -> String {
        let mut cmd = format!("turbo run {}", self.run_opts.tasks.join(" "));
        for pattern in &self.scope_opts.filter_patterns {
            cmd.push_str(" --filter=");
            cmd.push_str(pattern);
        }

        if self.scope_opts.affected_range.is_some() {
            cmd.push_str(" --affected");
        }

        if self.run_opts.parallel {
            cmd.push_str(" --parallel");
        }

        match self.run_opts.continue_on_error {
            ContinueMode::Always => cmd.push_str(" --continue=always"),
            ContinueMode::DependenciesSuccessful => {
                cmd.push_str(" --continue=dependencies-successful")
            }
            _ => (),
        }

        if let Some(dry) = self.run_opts.dry_run {
            match dry {
                DryRunMode::Json => cmd.push_str(" --dry=json"),
                DryRunMode::Text => cmd.push_str(" --dry"),
            }
        }

        if self.run_opts.only {
            cmd.push_str(" --only");
        }

        if !self.run_opts.pass_through_args.is_empty() {
            cmd.push_str(" -- ");
            cmd.push_str(&self.run_opts.pass_through_args.join(" "));
        }

        cmd
    }
}

impl Opts {
    pub fn new(
        repo_root: &AbsoluteSystemPath,
        args: &Args,
        config: ConfigurationOptions,
    ) -> Result<Self, Error> {
        let team_id = config.team_id();
        let team_slug = config.team_slug();

        let api_auth = config.token().map(|token| APIAuth {
            team_id: team_id.map(|s| s.to_string()),
            token: token.to_string(),
            team_slug: team_slug.map(|s| s.to_string()),
        });

        let (execution_args, run_args) = match &args.command {
            Some(Command::Run {
                run_args,
                execution_args,
            }) => (execution_args, run_args),
            Some(Command::Watch { execution_args, .. }) => (execution_args, &Box::default()),
            Some(Command::Ls {
                affected, filter, ..
            }) => {
                let execution_args = ExecutionArgs {
                    filter: filter.clone(),
                    affected: *affected,
                    ..Default::default()
                };

                (&Box::new(execution_args), &Box::default())
            }
            Some(Command::Boundaries { filter, .. }) => {
                let execution_args = ExecutionArgs {
                    filter: filter.clone(),
                    ..Default::default()
                };

                (&Box::new(execution_args), &Box::default())
            }
            _ => (&Box::default(), &Box::default()),
        };

        let inputs = OptsInputs {
            repo_root,
            run_args: run_args.as_ref(),
            execution_args: execution_args.as_ref(),
            config: &config,
            api_auth: &api_auth,
        };
        let run_opts = RunOpts::try_from(inputs)?;
        let cache_opts = CacheOpts::try_from(inputs)?;
        let scope_opts = ScopeOpts::try_from(inputs)?;
        let runcache_opts = RunCacheOpts::from(inputs);
        let api_client_opts = APIClientOpts::from(inputs);
        let repo_opts = RepoOpts::from(inputs);
        let tui_opts = TuiOpts::from(inputs);
        let future_flags = config.future_flags();

        Ok(Self {
            repo_opts,
            run_opts,
            cache_opts,
            scope_opts,
            runcache_opts,
            api_client_opts,
            tui_opts,
            future_flags,
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct OptsInputs<'a> {
    repo_root: &'a AbsoluteSystemPath,
    run_args: &'a RunArgs,
    execution_args: &'a ExecutionArgs,
    config: &'a ConfigurationOptions,
    api_auth: &'a Option<APIAuth>,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct RunCacheOpts {
    pub(crate) task_output_logs_override: Option<OutputLogsMode>,
}

impl<'a> From<OptsInputs<'a>> for RunCacheOpts {
    fn from(inputs: OptsInputs<'a>) -> Self {
        RunCacheOpts {
            task_output_logs_override: inputs.execution_args.output_logs,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct RunOpts {
    pub(crate) tasks: Vec<String>,
    pub(crate) concurrency: u32,
    pub(crate) parallel: bool,
    pub(crate) env_mode: EnvMode,
    pub(crate) cache_dir: Utf8PathBuf,
    // Whether or not to infer the framework for each workspace.
    pub(crate) framework_inference: bool,
    pub profile: Option<String>,
    pub(crate) continue_on_error: ContinueMode,
    pub(crate) pass_through_args: Vec<String>,
    pub(crate) only: bool,
    pub(crate) dry_run: Option<DryRunMode>,
    pub graph: Option<GraphOpts>,
    pub(crate) daemon: Option<bool>,
    pub(crate) single_package: bool,
    pub log_prefix: ResolvedLogPrefix,
    pub log_order: ResolvedLogOrder,
    pub summarize: bool,
    pub is_github_actions: bool,
    pub ui_mode: UIMode,
}

/// Projection of `RunOpts` that only includes information necessary to compute
/// pass through args.
#[derive(Debug)]
pub struct TaskArgs<'a> {
    pass_through_args: &'a [String],
    tasks: &'a [String],
}

impl RunOpts {
    pub fn task_args(&self) -> TaskArgs<'_> {
        TaskArgs {
            pass_through_args: &self.pass_through_args,
            tasks: &self.tasks,
        }
    }
}

impl<'a> TaskArgs<'a> {
    pub fn args_for_task(&self, task_id: &TaskId) -> Option<&'a [String]> {
        if !self.pass_through_args.is_empty()
            && self
                .tasks
                .iter()
                .any(|task| TaskName::from(task.as_str()).task() == task_id.task())
        {
            Some(self.pass_through_args)
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub enum GraphOpts {
    Stdout,
    File(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum ResolvedLogOrder {
    Stream,
    Grouped,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub enum ResolvedLogPrefix {
    Task,
    None,
}

impl<'a> From<OptsInputs<'a>> for RepoOpts {
    fn from(inputs: OptsInputs<'a>) -> Self {
        let root_turbo_json_path = inputs
            .config
            .root_turbo_json_path(inputs.repo_root)
            .unwrap_or_else(|_| inputs.repo_root.join_component(CONFIG_FILE));
        let allow_no_package_manager = inputs.config.allow_no_package_manager();
        let allow_no_turbo_json = inputs.config.allow_no_turbo_json();

        RepoOpts {
            root_turbo_json_path,
            allow_no_package_manager,
            allow_no_turbo_json,
        }
    }
}

const DEFAULT_CONCURRENCY: u32 = 10;

impl<'a> TryFrom<OptsInputs<'a>> for RunOpts {
    type Error = self::Error;

    fn try_from(inputs: OptsInputs) -> Result<Self, Self::Error> {
        let concurrency = inputs
            .config
            .concurrency
            .as_deref()
            .map(parse_concurrency)
            .transpose()?
            .unwrap_or(DEFAULT_CONCURRENCY);

        let graph = inputs.run_args.graph.as_deref().map(|file| match file {
            "" => GraphOpts::Stdout,
            f => GraphOpts::File(f.to_string()),
        });

        let (is_github_actions, log_order, log_prefix) = match inputs.config.log_order() {
            LogOrder::Auto if turborepo_ci::Vendor::get_constant() == Some("GITHUB_ACTIONS") => (
                true,
                ResolvedLogOrder::Grouped,
                match inputs.execution_args.log_prefix {
                    LogPrefix::Task => ResolvedLogPrefix::Task,
                    _ => ResolvedLogPrefix::None,
                },
            ),

            // Streaming is the default behavior except when running on GitHub Actions
            LogOrder::Auto | LogOrder::Stream => (
                false,
                ResolvedLogOrder::Stream,
                inputs.execution_args.log_prefix.into(),
            ),
            LogOrder::Grouped => (
                false,
                ResolvedLogOrder::Grouped,
                inputs.execution_args.log_prefix.into(),
            ),
        };

        Ok(Self {
            tasks: inputs.execution_args.tasks.clone(),
            log_prefix,
            log_order,
            summarize: inputs.config.run_summary(),
            framework_inference: inputs.execution_args.framework_inference,
            concurrency,
            parallel: inputs.run_args.parallel,
            profile: inputs.run_args.profile.clone(),
            continue_on_error: inputs.execution_args.continue_execution,
            pass_through_args: inputs.execution_args.pass_through_args.clone(),
            only: inputs.execution_args.only,
            daemon: inputs.config.daemon(),
            single_package: inputs.execution_args.single_package,
            graph,
            dry_run: inputs.run_args.dry_run,
            env_mode: inputs.config.env_mode(),
            cache_dir: inputs.config.cache_dir().into(),
            is_github_actions,
            ui_mode: inputs.config.ui(),
        })
    }
}

fn parse_concurrency(concurrency_raw: &str) -> Result<u32, self::Error> {
    if let Some(percent) = concurrency_raw.strip_suffix('%') {
        let percent = percent.parse::<f64>()?;
        return if percent > 0.0 && percent.is_finite() {
            Ok((num_cpus::get() as f64 * percent / 100.0).max(1.0) as u32)
        } else {
            Err(Error::InvalidConcurrencyPercentage(
                backtrace::Backtrace::capture(),
                percent,
            ))
        };
    }
    match concurrency_raw.parse::<u32>() {
        Ok(concurrency) if concurrency >= 1 => Ok(concurrency),
        Ok(_) | Err(_) => Err(Error::ConcurrencyOutOfBounds(
            backtrace::Backtrace::capture(),
            concurrency_raw.to_string(),
        )),
    }
}

impl From<LogPrefix> for ResolvedLogPrefix {
    fn from(value: LogPrefix) -> Self {
        match value {
            // We default to task-prefixed logs
            LogPrefix::Auto | LogPrefix::Task => ResolvedLogPrefix::Task,
            LogPrefix::None => ResolvedLogPrefix::None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ScopeOpts {
    pub pkg_inference_root: Option<AnchoredSystemPathBuf>,
    pub global_deps: Vec<String>,
    pub filter_patterns: Vec<String>,
    pub affected_range: Option<(Option<String>, Option<String>)>,
}

impl<'a> TryFrom<OptsInputs<'a>> for ScopeOpts {
    type Error = self::Error;

    fn try_from(inputs: OptsInputs<'a>) -> Result<Self, Self::Error> {
        let pkg_inference_root = inputs
            .execution_args
            .pkg_inference_root
            .as_ref()
            .map(AnchoredSystemPathBuf::from_raw)
            .transpose()?;

        let affected_range = inputs.execution_args.affected.then(|| {
            let scm_base = inputs.config.scm_base();
            let scm_head = inputs.config.scm_head();
            (
                scm_base.map(|b| b.to_owned()),
                scm_head.map(|h| h.to_string()),
            )
        });

        Ok(Self {
            global_deps: inputs.execution_args.global_deps.clone(),
            pkg_inference_root,
            affected_range,
            filter_patterns: inputs.execution_args.filter.clone(),
        })
    }
}

impl<'a> From<OptsInputs<'a>> for APIClientOpts {
    fn from(inputs: OptsInputs<'a>) -> Self {
        let api_url = inputs.config.api_url().to_string();
        let timeout = inputs.config.timeout();
        let upload_timeout = inputs.config.upload_timeout();
        let preflight = inputs.config.preflight();
        let token = inputs.config.token().map(|s| s.to_string());
        let team_id = inputs.config.team_id().map(|s| s.to_string());
        let team_slug = inputs.config.team_slug().map(|s| s.to_string());
        let login_url = inputs.config.login_url().to_string();
        let sso_login_callback_port = inputs.config.sso_login_callback_port();

        APIClientOpts {
            api_url,
            timeout,
            upload_timeout,
            token,
            team_id,
            team_slug,
            login_url,
            preflight,
            sso_login_callback_port,
        }
    }
}

impl<'a> TryFrom<OptsInputs<'a>> for CacheOpts {
    type Error = self::Error;

    fn try_from(inputs: OptsInputs<'a>) -> Result<Self, Self::Error> {
        let is_linked = turborepo_api_client::is_linked(inputs.api_auth);
        let cache = inputs.config.cache();
        let has_old_cache_config = inputs.config.remote_only()
            || inputs.run_args.no_cache
            || inputs.config.remote_cache_read_only();

        if has_old_cache_config && cache.is_some() {
            return Err(Error::OverlappingCacheOptions);
        }

        // defaults to fully enabled cache
        let mut cache = cache.unwrap_or_default();

        if inputs.config.remote_only() {
            cache.local.read = false;
            cache.local.write = false;
        }

        if inputs.config.force() {
            cache.local.read = false;
            cache.remote.read = false;
        }

        if inputs.run_args.no_cache {
            cache.local.write = false;
            cache.remote.write = false;
        }

        if !is_linked {
            cache.remote.read = false;
            cache.remote.write = false;
        } else if let Some(false) = inputs.config.enabled {
            // We're linked, but if the user has explicitly disabled remote cache
            cache.remote.read = false;
            cache.remote.write = false;
        };

        if inputs.config.remote_cache_read_only() {
            cache.remote.write = false;
        }

        // Note that we don't currently use the team_id value here. In the future, we
        // should probably verify that we only use the signature value when the
        // configured team_id matches the final resolved team_id.
        let unused_remote_cache_opts_team_id =
            inputs.config.team_id().map(|team_id| team_id.to_string());
        let signature = inputs.config.signature();
        let remote_cache_opts = Some(RemoteCacheOpts::new(
            unused_remote_cache_opts_team_id,
            signature,
        ));

        Ok(CacheOpts {
            cache_dir: inputs.config.cache_dir().into(),
            cache,
            workers: inputs.run_args.cache_workers,
            remote_cache_opts,
        })
    }
}

impl RunOpts {
    pub fn should_redirect_stderr_to_stdout(&self) -> bool {
        // If we're running on GitHub Actions, force everything to stdout
        // so as not to have out-of-order log lines
        matches!(self.log_order, ResolvedLogOrder::Grouped) && self.is_github_actions
    }
}

impl ScopeOpts {
    pub fn get_filters(&self) -> Vec<String> {
        self.filter_patterns.clone()
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct TuiOpts {
    pub(crate) scrollback_length: u64,
}

impl<'a> From<OptsInputs<'a>> for TuiOpts {
    fn from(inputs: OptsInputs) -> Self {
        TuiOpts {
            scrollback_length: inputs.config.tui_scrollback_length(),
        }
    }
}

#[cfg(test)]
mod test {
    use clap::Parser;
    use itertools::Itertools;
    use serde_json::json;
    use tempfile::TempDir;
    use test_case::test_case;
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_cache::{CacheActions, CacheConfig, CacheOpts};
    use turborepo_task_id::TaskId;
    use turborepo_ui::ColorConfig;

    use super::{APIClientOpts, RepoOpts, RunOpts, TaskArgs};
    use crate::{
        cli::{Command, ContinueMode, DryRunMode, RunArgs},
        commands::CommandBase,
        config::{ConfigurationOptions, CONFIG_FILE},
        opts::{Opts, RunCacheOpts, ScopeOpts, TuiOpts},
        turbo_json::UIMode,
        Args,
    };

    #[derive(Default)]
    struct TestCaseOpts {
        filter_patterns: Vec<String>,
        tasks: Vec<String>,
        only: bool,
        pass_through_args: Vec<String>,
        parallel: bool,
        continue_on_error: ContinueMode,
        dry_run: Option<DryRunMode>,
        affected: Option<(String, String)>,
    }

    #[test_case(TestCaseOpts{
        filter_patterns: vec!["my-app".to_string()],
        tasks: vec!["build".to_string()],
        ..Default::default()
        },
        "turbo run build --filter=my-app")]
    #[test_case(
        TestCaseOpts{
            tasks: vec!["build".to_string()],
            only: true,
            ..Default::default()
            },
        "turbo run build --only"
    )]
    #[test_case(
        TestCaseOpts{
            filter_patterns: vec!["my-app".to_string()],
            tasks: vec!["build".to_string()],
            pass_through_args: vec!["-v".to_string(), "--foo=bar".to_string()],
            ..Default::default()
            },
        "turbo run build --filter=my-app -- -v --foo=bar"
    )]
    #[test_case(
        TestCaseOpts{
            filter_patterns: vec!["other-app".to_string(), "my-app".to_string()],
            tasks: vec!["build".to_string()],
            pass_through_args: vec!["-v".to_string(), "--foo=bar".to_string()],
            ..Default::default()
            },
        "turbo run build --filter=other-app --filter=my-app -- -v --foo=bar"
    )]
    #[test_case(
        TestCaseOpts{
            filter_patterns: vec!["my-app".to_string()],
            tasks: vec!["build".to_string()],
            parallel: true,
            continue_on_error: ContinueMode::Always,
            ..Default::default()
            },
        "turbo run build --filter=my-app --parallel --continue=always"
    )]
    #[test_case(
        TestCaseOpts{
            filter_patterns: vec!["my-app".to_string()],
            tasks: vec!["build".to_string()],
            parallel: true,
            continue_on_error: ContinueMode::DependenciesSuccessful,
            ..Default::default()
            },
        "turbo run build --filter=my-app --parallel --continue=dependencies-successful"
    )]
    #[test_case(
        TestCaseOpts{
            filter_patterns: vec!["my-app".to_string()],
            tasks: vec!["build".to_string()],
            dry_run: Some(DryRunMode::Text),
            ..Default::default()
            },
        "turbo run build --filter=my-app --dry"
    )]
    #[test_case(
        TestCaseOpts{
            filter_patterns: vec!["my-app".to_string()],
            tasks: vec!["build".to_string()],
            dry_run: Some(DryRunMode::Json),
            ..Default::default()
            },
        "turbo run build --filter=my-app --dry=json"
    )]
    #[test_case(
        TestCaseOpts{
            filter_patterns: vec!["my-app".to_string()],
            tasks: vec!["build".to_string()],
            affected: Some(("HEAD".to_string(), "my-branch".to_string())),
            ..Default::default()
            },
        "turbo run build --filter=my-app --affected"
    )]
    #[test_case(
        TestCaseOpts{
            tasks: vec!["build".to_string()],
            affected: Some(("HEAD".to_string(), "my-branch".to_string())),
            ..Default::default()
            },
        "turbo run build --affected"
    )]
    fn test_synthesize_command(opts_input: TestCaseOpts, expected: &str) {
        let run_opts = RunOpts {
            tasks: opts_input.tasks,
            concurrency: 10,
            parallel: opts_input.parallel,
            env_mode: crate::cli::EnvMode::Loose,
            cache_dir: camino::Utf8PathBuf::new(),
            framework_inference: true,
            profile: None,
            continue_on_error: opts_input.continue_on_error,
            pass_through_args: opts_input.pass_through_args,
            only: opts_input.only,
            dry_run: opts_input.dry_run,
            graph: None,
            ui_mode: UIMode::Stream,
            single_package: false,
            log_prefix: crate::opts::ResolvedLogPrefix::Task,
            log_order: crate::opts::ResolvedLogOrder::Stream,
            summarize: false,
            is_github_actions: false,
            daemon: None,
        };
        let cache_opts = CacheOpts {
            cache_dir: ".turbo/cache".into(),
            cache: Default::default(),
            workers: 0,
            remote_cache_opts: None,
        };
        let runcache_opts = RunCacheOpts::default();
        let scope_opts = ScopeOpts {
            pkg_inference_root: None,
            global_deps: vec![],
            filter_patterns: opts_input.filter_patterns,
            affected_range: opts_input
                .affected
                .map(|(base, head)| (Some(base), Some(head))),
        };
        let config = ConfigurationOptions::default();
        let root_turbo_json_path = config
            .root_turbo_json_path(&AbsoluteSystemPathBuf::default())
            .unwrap_or_else(|_| AbsoluteSystemPathBuf::default().join_component(CONFIG_FILE));

        let tui_opts = TuiOpts {
            scrollback_length: 2048,
        };

        let opts = Opts {
            repo_opts: RepoOpts {
                root_turbo_json_path,
                allow_no_package_manager: false,
                allow_no_turbo_json: false,
            },
            api_client_opts: APIClientOpts {
                api_url: "".to_string(),
                timeout: 0,
                upload_timeout: 0,
                token: None,
                team_id: None,
                team_slug: None,
                login_url: "".to_string(),
                preflight: false,
                sso_login_callback_port: None,
            },
            scope_opts,
            run_opts,
            cache_opts,
            runcache_opts,
            tui_opts,
            future_flags: Default::default(),
        };
        let synthesized = opts.synthesize_command();
        assert_eq!(synthesized, expected);
    }

    #[test_case(
         RunArgs {
             no_cache: true,
             ..Default::default()
         }, "no-cache"
     ; "no-cache" )]
    #[test_case(
         RunArgs {
             force: Some(Some(true)),
             ..Default::default()
         }, "force"
     ; "force")]
    #[test_case(
        RunArgs{
             remote_only: Some(Some(true)),
             ..Default::default()
            }, "remote-only"
    )]
    #[test_case(
        RunArgs{
             remote_cache_read_only: Some(Some(true)),
             ..Default::default()
            }, "remote-cache-read-only"
    )]
    #[test_case(
        RunArgs{
             no_cache: true,
             cache: Some("remote:w,local:rw".to_string()),
             ..Default::default()
            }, "no-cache_remote_w,local_rw"
    )]
    #[test_case(
        RunArgs{
             remote_only: Some(Some(true)),
             cache: Some("remote:r,local:rw".to_string()),
             ..Default::default()
            }, "remote-only_remote_r,local_rw"
    )]
    #[test_case(
        RunArgs{
             force: Some(Some(true)),
             cache: Some("remote:r,local:r".to_string()),
             ..Default::default()
            }, "force_remote_r,local_r"
    )]
    #[test_case(
        RunArgs{
              remote_cache_read_only: Some(Some(true)),
              cache: Some("remote:rw,local:r".to_string()),
              ..Default::default()
            }, "remote-cache-read-only_remote_rw,local_r"
    )]
    fn test_resolve_cache_config(run_args: RunArgs, name: &str) -> Result<(), anyhow::Error> {
        let mut args = Args::default();
        args.command = Some(Command::Run {
            execution_args: Box::default(),
            run_args: Box::new(run_args),
        });
        // set token and team to simulate a logged in/linked user
        args.token = Some("token".to_string());
        args.team = Some("team".to_string());

        let cache_config = CommandBase::new(
            args,
            AbsoluteSystemPathBuf::default(),
            "1.0.0",
            ColorConfig::new(true),
        )
        .map(|base| base.opts().cache_opts.cache);

        insta::assert_debug_snapshot!(name, cache_config);

        Ok(())
    }

    #[test]
    fn test_cache_config_force_remote_enable() -> Result<(), anyhow::Error> {
        let tmpdir = TempDir::new()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(tmpdir.path())?;

        repo_root.join_component(CONFIG_FILE).create_with_contents(
            serde_json::to_string_pretty(&serde_json::json!({
                "remoteCache": { "enabled": true }
            }))?,
        )?;

        let mut args = Args::default();
        args.command = Some(Command::Run {
            execution_args: Box::default(),
            run_args: Box::new(RunArgs {
                force: Some(Some(true)),
                ..Default::default()
            }),
        });

        // set token and team to simulate a logged in/linked user
        args.token = Some("token".to_string());
        args.team = Some("team".to_string());

        let base = CommandBase::new(args, repo_root, "1.0.0", ColorConfig::new(false))?;
        let actual = base.opts().cache_opts.cache;

        assert_eq!(
            actual,
            CacheConfig {
                remote: CacheActions {
                    read: false,
                    write: true
                },
                local: CacheActions {
                    read: false,
                    write: true
                }
            }
        );

        Ok(())
    }

    #[test_case(
        vec!["turbo", "watch", "build"];
        "watch"
    )]
    #[test_case(
        vec!["turbo", "run", "build"];
        "run"
    )]
    #[test_case(
        vec!["turbo", "ls", "--filter", "foo"];
        "ls"
    )]
    #[test_case(
        vec!["turbo", "boundaries", "--filter", "foo"];
        "boundaries"
    )]
    fn test_derive_opts_from_args(args_str: Vec<&str>) -> Result<(), anyhow::Error> {
        let args = Args::try_parse_from(&args_str)?;
        let opts = Opts::new(
            &AbsoluteSystemPathBuf::default(),
            &args,
            ConfigurationOptions::default(),
        )?;

        insta::assert_json_snapshot!(
            args_str.iter().join("_"),
            json!({ "tasks": opts.run_opts.tasks, "filter_patterns": opts.scope_opts.filter_patterns  })
        );

        Ok(())
    }

    #[test_case(
        vec!["build".to_string()],
        vec!["passthrough".to_string()],
        TaskId::new("web", "build"),
        Some(vec!["passthrough".to_string()]);
        "single task"
    )]
    #[test_case(
        vec!["lint".to_string(), "build".to_string()],
        vec!["passthrough".to_string()],
        TaskId::new("web", "build"),
        Some(vec!["passthrough".to_string()]);
        "multiple tasks"
    )]
    #[test_case(
        vec!["test".to_string()],
        vec!["passthrough".to_string()],
        TaskId::new("web", "build"),
        None;
        "different task"
    )]
    #[test_case(
        vec!["web#build".to_string()],
        vec!["passthrough".to_string()],
        TaskId::new("web", "build"),
        Some(vec!["passthrough".to_string()]);
        "task with package"
    )]
    #[test_case(
        vec!["lint".to_string()],
        vec![],
        TaskId::new("ui", "lint"),
        None;
        "no passthrough args"
    )]
    fn test_get_args_for_tasks(
        tasks: Vec<String>,
        pass_through_args: Vec<String>,
        expected_task: TaskId<'static>,
        expected_args: Option<Vec<String>>,
    ) -> Result<(), anyhow::Error> {
        let task_opts = TaskArgs {
            tasks: &tasks,
            pass_through_args: &pass_through_args,
        };

        assert_eq!(
            task_opts.args_for_task(&expected_task),
            expected_args.as_deref()
        );

        Ok(())
    }
}
