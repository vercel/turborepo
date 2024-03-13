use std::backtrace;

use thiserror::Error;
use turbopath::AnchoredSystemPathBuf;
use turborepo_cache::CacheOpts;

use crate::{
    cli::{Command, DryRunMode, EnvMode, LogOrder, LogPrefix, OutputLogsMode, RunArgs},
    run::task_id::TaskId,
    Args,
};

#[derive(Debug, Error)]
pub enum Error {
    #[error("Expected run command")]
    ExpectedRun,
    #[error(transparent)]
    ParseFloat(#[from] std::num::ParseFloatError),
    #[error(
        "invalid percentage value for --concurrency CLI flag. This should be a percentage of CPU \
         cores, between 1% and 100% : {1}"
    )]
    InvalidConcurrencyPercentage(#[backtrace] backtrace::Backtrace, f64),
    #[error(
        "invalid value for --concurrency CLI flag. This should be a positive integer greater than \
         or equal to 1: {1}"
    )]
    ConcurrencyOutOfBounds(#[backtrace] backtrace::Backtrace, String),
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
}

#[derive(Debug)]
pub struct Opts {
    pub cache_opts: CacheOpts,
    pub run_opts: RunOpts,
    pub runcache_opts: RunCacheOpts,
    pub scope_opts: ScopeOpts,
}

impl Opts {
    pub fn synthesize_command(&self) -> String {
        let mut cmd = format!("turbo run {}", self.run_opts.tasks.join(" "));
        for pattern in &self.scope_opts.filter_patterns {
            cmd.push_str(" --filter=");
            cmd.push_str(pattern);
        }

        for pattern in &self.scope_opts.legacy_filter.as_filter_pattern() {
            cmd.push_str(" --filter=");
            cmd.push_str(pattern);
        }

        if self.run_opts.parallel {
            cmd.push_str(" --parallel");
        }

        if self.run_opts.continue_on_error {
            cmd.push_str(" --continue");
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

impl<'a> TryFrom<&'a Args> for Opts {
    type Error = self::Error;

    fn try_from(args: &'a Args) -> Result<Self, Self::Error> {
        let Some(Command::Run(run_args)) = &args.command else {
            return Err(Error::ExpectedRun);
        };
        let run_opts = RunOpts::try_from(run_args.as_ref())?;
        let cache_opts = CacheOpts::from(run_args.as_ref());
        let scope_opts = ScopeOpts::try_from(run_args.as_ref())?;
        let runcache_opts = RunCacheOpts::from(run_args.as_ref());

        Ok(Self {
            run_opts,
            cache_opts,
            scope_opts,
            runcache_opts,
        })
    }
}

#[derive(Debug, Default)]
pub struct RunCacheOpts {
    pub(crate) skip_reads: bool,
    pub(crate) skip_writes: bool,
    pub(crate) task_output_mode_override: Option<OutputLogsMode>,
}

impl<'a> From<&'a RunArgs> for RunCacheOpts {
    fn from(args: &'a RunArgs) -> Self {
        RunCacheOpts {
            skip_reads: args.execution_args.force.flatten().is_some_and(|f| f),
            skip_writes: args.no_cache,
            task_output_mode_override: args.execution_args.output_logs,
        }
    }
}

#[derive(Debug)]
pub struct RunOpts {
    pub(crate) tasks: Vec<String>,
    pub(crate) concurrency: u32,
    pub(crate) parallel: bool,
    pub(crate) env_mode: EnvMode,
    // Whether or not to infer the framework for each workspace.
    pub(crate) framework_inference: bool,
    pub profile: Option<String>,
    pub(crate) continue_on_error: bool,
    pub(crate) pass_through_args: Vec<String>,
    pub(crate) only: bool,
    pub(crate) dry_run: Option<DryRunMode>,
    pub graph: Option<GraphOpts>,
    pub(crate) daemon: Option<bool>,
    pub(crate) single_package: bool,
    pub log_prefix: ResolvedLogPrefix,
    pub log_order: ResolvedLogOrder,
    pub summarize: Option<Option<bool>>,
    pub(crate) experimental_space_id: Option<String>,
    pub is_github_actions: bool,
}

impl RunOpts {
    pub fn args_for_task(&self, task_id: &TaskId) -> Option<Vec<String>> {
        if !self.pass_through_args.is_empty()
            && self
                .tasks
                .iter()
                .any(|task| task.as_str() == task_id.task())
        {
            Some(self.pass_through_args.clone())
        } else {
            None
        }
    }
}

#[derive(Debug)]
pub enum GraphOpts {
    Stdout,
    File(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResolvedLogOrder {
    Stream,
    Grouped,
}

#[derive(Debug, Clone, Copy)]
pub enum ResolvedLogPrefix {
    Task,
    None,
}

const DEFAULT_CONCURRENCY: u32 = 10;

impl<'a> TryFrom<&'a RunArgs> for RunOpts {
    type Error = self::Error;

    fn try_from(args: &'a RunArgs) -> Result<Self, Self::Error> {
        let concurrency = args
            .execution_args
            .concurrency
            .as_deref()
            .map(parse_concurrency)
            .transpose()?
            .unwrap_or(DEFAULT_CONCURRENCY);

        let graph = args.graph.as_deref().map(|file| match file {
            "" => GraphOpts::Stdout,
            f => GraphOpts::File(f.to_string()),
        });

        let (is_github_actions, log_order, log_prefix) = match args.execution_args.log_order {
            LogOrder::Auto if turborepo_ci::Vendor::get_constant() == Some("GITHUB_ACTIONS") => (
                true,
                ResolvedLogOrder::Grouped,
                match args.execution_args.log_prefix {
                    LogPrefix::Task => ResolvedLogPrefix::Task,
                    _ => ResolvedLogPrefix::None,
                },
            ),

            // Streaming is the default behavior except when running on GitHub Actions
            LogOrder::Auto | LogOrder::Stream => (
                false,
                ResolvedLogOrder::Stream,
                args.execution_args.log_prefix.into(),
            ),
            LogOrder::Grouped => (
                false,
                ResolvedLogOrder::Grouped,
                args.execution_args.log_prefix.into(),
            ),
        };

        Ok(Self {
            tasks: args.execution_args.tasks.clone(),
            log_prefix,
            log_order,
            summarize: args.summarize,
            experimental_space_id: args.experimental_space_id.clone(),
            framework_inference: args.execution_args.framework_inference,
            env_mode: args.execution_args.env_mode,
            concurrency,
            parallel: args.parallel,
            profile: args.profile.clone(),
            continue_on_error: args.execution_args.continue_execution,
            pass_through_args: args.execution_args.pass_through_args.clone(),
            only: args.execution_args.only,
            daemon: args.daemon(),
            single_package: args.execution_args.single_package,
            graph,
            dry_run: args.dry_run,
            is_github_actions,
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

// LegacyFilter holds the options in use before the filter syntax. They have
// their own rules for how they are compiled into filter expressions.
#[derive(Debug, Default)]
pub struct LegacyFilter {
    // include_dependencies is whether to include pkg.dependencies in execution (defaults to false)
    include_dependencies: bool,
    // skip_dependents is whether to skip dependent impacted consumers in execution (defaults to
    // false)
    skip_dependents: bool,
    // entrypoints is a list of package entrypoints
    entrypoints: Vec<String>,
    // since is the git ref used to calculate changed packages
    pub since: Option<String>,
}

impl LegacyFilter {
    pub fn as_filter_pattern(&self) -> Vec<String> {
        let prefix = if self.skip_dependents { "" } else { "..." };
        let suffix = if self.include_dependencies { "..." } else { "" };
        if self.entrypoints.is_empty() {
            if let Some(since) = self.since.as_ref() {
                vec![format!("{}[{}]{}", prefix, since, suffix)]
            } else {
                Vec::new()
            }
        } else {
            let since = self
                .since
                .as_ref()
                .map_or_else(String::new, |s| format!("...[{}]", s));
            self.entrypoints
                .iter()
                .map(|pattern| {
                    if pattern.starts_with('!') {
                        pattern.to_owned()
                    } else {
                        format!("{}{}{}{}", prefix, pattern, since, suffix)
                    }
                })
                .collect()
        }
    }
}

#[derive(Debug)]
pub struct ScopeOpts {
    pub pkg_inference_root: Option<AnchoredSystemPathBuf>,
    pub legacy_filter: LegacyFilter,
    pub global_deps: Vec<String>,
    pub filter_patterns: Vec<String>,
    pub ignore_patterns: Vec<String>,
}

impl<'a> TryFrom<&'a RunArgs> for ScopeOpts {
    type Error = self::Error;

    fn try_from(args: &'a RunArgs) -> Result<Self, Self::Error> {
        let pkg_inference_root = args
            .execution_args
            .pkg_inference_root
            .as_ref()
            .map(AnchoredSystemPathBuf::from_raw)
            .transpose()?;

        let legacy_filter = LegacyFilter {
            include_dependencies: args.execution_args.include_dependencies,
            skip_dependents: args.execution_args.no_deps,
            entrypoints: args.execution_args.scope.clone(),
            since: args.execution_args.since.clone(),
        };
        Ok(Self {
            global_deps: args.execution_args.global_deps.clone(),
            pkg_inference_root,
            legacy_filter,
            filter_patterns: args.execution_args.filter.clone(),
            ignore_patterns: args.execution_args.ignore.clone(),
        })
    }
}

impl<'a> From<&'a RunArgs> for CacheOpts {
    fn from(run_args: &'a RunArgs) -> Self {
        CacheOpts {
            override_dir: run_args.execution_args.cache_dir.clone(),
            skip_filesystem: run_args.execution_args.remote_only,
            remote_cache_read_only: run_args.remote_cache_read_only,
            workers: run_args.cache_workers,
            ..CacheOpts::default()
        }
    }
}

impl RunOpts {
    pub fn should_redirect_stderr_to_stdout(&self) -> bool {
        // If we're running on Github Actions, force everything to stdout
        // so as not to have out-of-order log lines
        matches!(self.log_order, ResolvedLogOrder::Grouped) && self.is_github_actions
    }
}

impl ScopeOpts {
    pub fn get_filters(&self) -> Vec<String> {
        [
            self.filter_patterns.clone(),
            self.legacy_filter.as_filter_pattern(),
        ]
        .concat()
    }
}

#[cfg(test)]
mod test {
    use test_case::test_case;
    use turborepo_cache::CacheOpts;

    use super::{LegacyFilter, RunOpts};
    use crate::{
        cli::DryRunMode,
        opts::{Opts, RunCacheOpts, ScopeOpts},
    };

    #[test_case(LegacyFilter {
            include_dependencies: true,
            skip_dependents: false,
            entrypoints: vec![],
            since: Some("since".to_string()),
        }, &["...[since]..."])]
    #[test_case(LegacyFilter {
            include_dependencies: false,
            skip_dependents: true,
            entrypoints: vec![],
            since: Some("since".to_string()),
        }, &["[since]"])]
    #[test_case(LegacyFilter {
            include_dependencies: false,
            skip_dependents: true,
            entrypoints: vec!["entry".to_string()],
            since: Some("since".to_string()),
        }, &["entry...[since]"])]
    fn basic_legacy_filter_pattern(filter: LegacyFilter, expected: &[&str]) {
        assert_eq!(
            filter.as_filter_pattern(),
            expected.iter().map(|s| s.to_string()).collect::<Vec<_>>()
        )
    }
    #[derive(Default)]
    struct TestCaseOpts {
        filter_patterns: Vec<String>,
        tasks: Vec<String>,
        only: bool,
        pass_through_args: Vec<String>,
        parallel: bool,
        continue_on_error: bool,
        dry_run: Option<DryRunMode>,
        legacy_filter: Option<LegacyFilter>,
    }

    #[test_case(TestCaseOpts {
        filter_patterns: vec!["my-app".to_string()],
        tasks: vec!["build".to_string()],
        ..Default::default()
    },
    "turbo run build --filter=my-app")]
    #[test_case(
        TestCaseOpts {
            tasks: vec!["build".to_string()],
            only: true,
            ..Default::default()
        },
        "turbo run build --only"
    )]
    #[test_case(
        TestCaseOpts {
            filter_patterns: vec!["my-app".to_string()],
            tasks: vec!["build".to_string()],
            pass_through_args: vec!["-v".to_string(), "--foo=bar".to_string()],
            ..Default::default()
        },
        "turbo run build --filter=my-app -- -v --foo=bar"
    )]
    #[test_case(
        TestCaseOpts {
            legacy_filter: Some(LegacyFilter {
                include_dependencies: false,
                skip_dependents: true,
                entrypoints: vec!["my-app".to_string()],
                since: None,
            }),
            tasks: vec!["build".to_string()],
            pass_through_args: vec!["-v".to_string(), "--foo=bar".to_string()],
            ..Default::default()
        },
        "turbo run build --filter=my-app -- -v --foo=bar"
    )]
    #[test_case(
        TestCaseOpts {
            legacy_filter: Some(LegacyFilter {
                include_dependencies: false,
                skip_dependents: true,
                entrypoints: vec!["my-app".to_string()],
                since: None,
            }),
            filter_patterns: vec!["other-app".to_string()],
            tasks: vec!["build".to_string()],
            pass_through_args: vec!["-v".to_string(), "--foo=bar".to_string()],
            ..Default::default()
        },
        "turbo run build --filter=other-app --filter=my-app -- -v --foo=bar"
    )]
    #[test_case    (
        TestCaseOpts {
            legacy_filter: Some(LegacyFilter {
                include_dependencies: true,
                skip_dependents: false,
                entrypoints: vec!["my-app".to_string()],
                since: Some("some-ref".to_string()),
            }),
            filter_patterns: vec!["other-app".to_string()],
            tasks: vec!["build".to_string()],
            ..Default::default()
        },
        "turbo run build --filter=other-app --filter=...my-app...[some-ref]..."
    )]
    #[test_case    (
        TestCaseOpts {
            filter_patterns: vec!["my-app".to_string()],
            tasks: vec!["build".to_string()],
            parallel: true,
            continue_on_error: true,
            ..Default::default()
        },
        "turbo run build --filter=my-app --parallel --continue"
    )]
    #[test_case    (
        TestCaseOpts {
            filter_patterns: vec!["my-app".to_string()],
            tasks: vec!["build".to_string()],
            dry_run: Some(DryRunMode::Text),
            ..Default::default()
        },
        "turbo run build --filter=my-app --dry"
    )]
    #[test_case    (
        TestCaseOpts {
            filter_patterns: vec!["my-app".to_string()],
            tasks: vec!["build".to_string()],
            dry_run: Some(DryRunMode::Json),
            ..Default::default()
        },
        "turbo run build --filter=my-app --dry=json"
    )]
    fn test_synthesize_command(opts_input: TestCaseOpts, expected: &str) {
        let run_opts = RunOpts {
            tasks: opts_input.tasks,
            concurrency: 10,
            parallel: opts_input.parallel,
            env_mode: crate::cli::EnvMode::Loose,
            framework_inference: true,
            profile: None,
            continue_on_error: opts_input.continue_on_error,
            pass_through_args: opts_input.pass_through_args,
            only: opts_input.only,
            dry_run: opts_input.dry_run,
            graph: None,
            daemon: None,
            single_package: false,
            log_prefix: crate::opts::ResolvedLogPrefix::Task,
            log_order: crate::opts::ResolvedLogOrder::Stream,
            summarize: None,
            experimental_space_id: None,
            is_github_actions: false,
        };
        let cache_opts = CacheOpts::default();
        let runcache_opts = RunCacheOpts::default();
        let legacy_filter = opts_input.legacy_filter.unwrap_or_default();
        let scope_opts = ScopeOpts {
            pkg_inference_root: None,
            legacy_filter,
            global_deps: vec![],
            filter_patterns: opts_input.filter_patterns,
            ignore_patterns: vec![],
        };
        let opts = Opts {
            run_opts,
            cache_opts,
            scope_opts,
            runcache_opts,
        };
        let synthesized = opts.synthesize_command();
        assert_eq!(synthesized, expected);
    }
}
