use std::backtrace;

use thiserror::Error;
use turbopath::AnchoredSystemPathBuf;
use turborepo_cache::CacheOpts;

use crate::{
    cli::{Command, DryRunMode, EnvMode, LogOrder, LogPrefix, OutputLogsMode, RunArgs},
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
pub struct Opts<'a> {
    pub cache_opts: CacheOpts<'a>,
    pub run_opts: RunOpts<'a>,
    pub runcache_opts: RunCacheOpts,
    pub scope_opts: ScopeOpts,
}

impl<'a> TryFrom<&'a Args> for Opts<'a> {
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
            skip_reads: args.force.flatten().is_some_and(|f| f),
            skip_writes: args.no_cache,
            task_output_mode_override: args.output_logs,
        }
    }
}

#[derive(Debug)]
pub struct RunOpts<'a> {
    pub(crate) tasks: &'a [String],
    pub(crate) concurrency: u32,
    pub(crate) parallel: bool,
    pub(crate) env_mode: EnvMode,
    // Whether or not to infer the framework for each workspace.
    pub(crate) framework_inference: bool,
    pub profile: Option<&'a str>,
    pub(crate) continue_on_error: bool,
    pub(crate) pass_through_args: &'a [String],
    pub(crate) only: bool,
    pub(crate) dry_run: bool,
    pub(crate) dry_run_json: bool,
    pub graph: Option<GraphOpts<'a>>,
    pub(crate) no_daemon: bool,
    pub(crate) single_package: bool,
    pub log_prefix: ResolvedLogPrefix,
    pub log_order: ResolvedLogOrder,
    pub summarize: Option<Option<bool>>,
    pub(crate) experimental_space_id: Option<String>,
    pub is_github_actions: bool,
}

#[derive(Debug)]
pub enum GraphOpts<'a> {
    Stdout,
    File(&'a str),
}

#[derive(Debug, Clone, Copy)]
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

impl<'a> TryFrom<&'a RunArgs> for RunOpts<'a> {
    type Error = self::Error;

    fn try_from(args: &'a RunArgs) -> Result<Self, Self::Error> {
        let concurrency = args
            .concurrency
            .as_deref()
            .map(parse_concurrency)
            .transpose()?
            .unwrap_or(DEFAULT_CONCURRENCY);

        let graph = args.graph.as_deref().map(|file| match file {
            "" => GraphOpts::Stdout,
            f => GraphOpts::File(f),
        });

        let (is_github_actions, log_order, log_prefix) = match args.log_order {
            LogOrder::Auto if turborepo_ci::Vendor::get_constant() == Some("GITHUB_ACTIONS") => (
                true,
                ResolvedLogOrder::Grouped,
                match args.log_prefix {
                    LogPrefix::Task => ResolvedLogPrefix::Task,
                    _ => ResolvedLogPrefix::None,
                },
            ),

            // Streaming is the default behavior except when running on GitHub Actions
            LogOrder::Auto | LogOrder::Stream => {
                (false, ResolvedLogOrder::Stream, args.log_prefix.into())
            }
            LogOrder::Grouped => (false, ResolvedLogOrder::Grouped, args.log_prefix.into()),
        };

        Ok(Self {
            tasks: args.tasks.as_slice(),
            log_prefix,
            log_order,
            summarize: args.summarize,
            experimental_space_id: args.experimental_space_id.clone(),
            framework_inference: args.framework_inference,
            env_mode: args.env_mode,
            concurrency,
            parallel: args.parallel,
            profile: args.profile.as_deref(),
            continue_on_error: args.continue_execution,
            pass_through_args: args.pass_through_args.as_ref(),
            only: args.only,
            no_daemon: args.no_daemon,
            single_package: args.single_package,
            graph,
            dry_run_json: matches!(args.dry_run, Some(DryRunMode::Json)),
            dry_run: args.dry_run.is_some(),
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
    since: Option<String>,
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
                .map_or_else(String::new, |s| format!("...{}", s));
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
            .pkg_inference_root
            .as_ref()
            .map(AnchoredSystemPathBuf::from_raw)
            .transpose()?;

        let legacy_filter = LegacyFilter {
            include_dependencies: args.include_dependencies,
            skip_dependents: args.no_deps,
            entrypoints: args.scope.clone(),
            since: args.since.clone(),
        };
        Ok(Self {
            global_deps: args.global_deps.clone(),
            pkg_inference_root,
            legacy_filter,
            filter_patterns: args.filter.clone(),
            ignore_patterns: args.ignore.clone(),
        })
    }
}

impl<'a> From<&'a RunArgs> for CacheOpts<'a> {
    fn from(run_args: &'a RunArgs) -> Self {
        CacheOpts {
            override_dir: run_args.cache_dir.as_deref(),
            skip_filesystem: run_args.remote_only,
            workers: run_args.cache_workers,
            ..CacheOpts::default()
        }
    }
}

impl<'a> RunOpts<'a> {
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

    use super::LegacyFilter;

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
        }, &["entry...since"])]
    fn basic_legacy_filter_pattern(filter: LegacyFilter, expected: &[&str]) {
        assert_eq!(
            filter.as_filter_pattern(),
            expected.iter().map(|s| s.to_string()).collect::<Vec<_>>()
        )
    }
}
