use anyhow::{anyhow, Result};
use turbopath::AnchoredSystemPathBuf;
use turborepo_cache::CacheOpts;

use crate::{
    cli::{Command, DryRunMode, EnvMode, LogPrefix, OutputLogsMode, RunArgs},
    Args,
};

#[derive(Debug)]
pub struct Opts<'a> {
    pub cache_opts: CacheOpts<'a>,
    pub run_opts: RunOpts<'a>,
    pub runcache_opts: RunCacheOpts,
    pub scope_opts: ScopeOpts,
}

impl<'a> TryFrom<&'a Args> for Opts<'a> {
    type Error = anyhow::Error;

    fn try_from(args: &'a Args) -> std::result::Result<Self, Self::Error> {
        let Some(Command::Run(run_args)) = &args.command else {
            return Err(anyhow!("Expected run command"));
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
    parallel: bool,
    pub(crate) env_mode: EnvMode,
    // Whether or not to infer the framework for each workspace.
    pub(crate) framework_inference: bool,
    profile: Option<&'a str>,
    continue_on_error: bool,
    passthrough_args: &'a [String],
    pub(crate) only: bool,
    dry_run: bool,
    pub(crate) dry_run_json: bool,
    pub graph: Option<GraphOpts<'a>>,
    pub(crate) no_daemon: bool,
    pub(crate) single_package: bool,
    log_prefix: LogPrefix,
    summarize: Option<Option<bool>>,
    pub(crate) experimental_space_id: Option<String>,
}

#[derive(Debug)]
pub enum GraphOpts<'a> {
    Stdout,
    File(&'a str),
}

const DEFAULT_CONCURRENCY: u32 = 10;

impl<'a> TryFrom<&'a RunArgs> for RunOpts<'a> {
    type Error = anyhow::Error;

    fn try_from(args: &'a RunArgs) -> Result<Self> {
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

        Ok(Self {
            tasks: args.tasks.as_slice(),
            log_prefix: args.log_prefix,
            summarize: args.summarize,
            experimental_space_id: args.experimental_space_id.clone(),
            framework_inference: args.framework_inference,
            env_mode: args.env_mode,
            concurrency,
            parallel: args.parallel,
            profile: args.profile.as_deref(),
            continue_on_error: args.continue_execution,
            passthrough_args: args.pass_through_args.as_ref(),
            only: args.only,
            no_daemon: args.no_daemon,
            single_package: args.single_package,
            graph,
            dry_run_json: matches!(args.dry_run, Some(DryRunMode::Json)),
            dry_run: args.dry_run.is_some(),
        })
    }
}

fn parse_concurrency(concurrency_raw: &str) -> Result<u32> {
    if let Some(percent) = concurrency_raw.strip_suffix('%') {
        let percent = percent.parse::<f64>()?;
        return if percent > 0.0 && percent.is_finite() {
            Ok((num_cpus::get() as f64 * percent / 100.0).max(1.0) as u32)
        } else {
            Err(anyhow!(
                "invalid percentage value for --concurrency CLI flag. This should be a percentage \
                 of CPU cores, between 1% and 100% : {}",
                percent
            ))
        };
    }
    match concurrency_raw.parse::<u32>() {
        Ok(concurrency) if concurrency > 1 => Ok(concurrency),
        Ok(_) | Err(_) => Err(anyhow!(
            "invalid value for --concurrency CLI flag. This should be a positive integer greater \
             than or equal to 1: {}",
            concurrency_raw
        )),
    }
}

// LegacyFilter holds the options in use before the filter syntax. They have
// their own rules for how they are compiled into filter expressions.
#[derive(Debug)]
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
                vec![format!("[{}{}{}]", prefix, since, suffix)]
            } else {
                Vec::new()
            }
        } else {
            let since = if let Some(since) = self.since.as_ref() {
                format!("...{}", since)
            } else {
                "".to_string()
            };
            self.entrypoints
                .iter()
                .map(|pattern| {
                    if pattern.starts_with("!") {
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
    pub filter_patterns: Vec<String>,
}

impl<'a> TryFrom<&'a RunArgs> for ScopeOpts {
    type Error = anyhow::Error;

    fn try_from(args: &'a RunArgs) -> std::result::Result<Self, Self::Error> {
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
            pkg_inference_root,
            legacy_filter,
            filter_patterns: args.filter.clone(),
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
