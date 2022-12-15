use std::{cmp, path::PathBuf};

use anyhow::{anyhow, Result};
use serde::Serialize;

use crate::cli::{DryRunMode, OutputLogsMode, RunArgs};

fn parse_concurrency(concurrency: &str) -> Result<usize> {
    if concurrency.starts_with('%') {
        let percent = concurrency
            .strip_prefix('%')
            .ok_or_else(|| anyhow!("Failed to strip % from concurrency"))?
            .parse::<f32>()
            .map_err(|err| {
                anyhow!(
                    "invalid value for --concurrency CLI flag. This should be a number \
                     --concurrency=4 or percentage of CPU cores --concurrency=50% : {}",
                    err
                )
            })?;

        if percent > 0.0 && percent.is_finite() {
            let cpu_count = num_cpus::get() as f32;

            Ok(cmp::max(1, (cpu_count * percent / 100.0) as usize))
        } else {
            Err(anyhow!(
                "invalid value for --concurrency CLI flag. This should be a number \
                 --concurrency=4 or percentage of CPU cores --concurrency=50%"
            ))
        }
    } else {
        let concurrency = concurrency.parse::<usize>().map_err(|err| {
            anyhow!(
                "invalid value for --concurrency CLI flag. This should be a number \
                 --concurrency=4 or percentage of CPU cores --concurrency=50% : {}",
                err
            )
        })?;

        if concurrency >= 1 {
            Ok(concurrency)
        } else {
            Err(anyhow!(
                "invalid value {} for --concurrency CLI flag. This should be a number \
                 --concurrency=4 or percentage of CPU cores --concurrency=50%",
                concurrency
            ))
        }
    }
}

const DEFAULT_CONCURRENCY: usize = 10;

#[derive(Serialize)]
struct Opts {
    run_opts: RunOpts,
    cache_opts: CacheOpts,
    run_cache_opts: RunCacheOpts,
    scope_opts: ScopeOpts,
}

impl TryFrom<RunArgs> for Opts {
    type Error = anyhow::Error;

    fn try_from(run_args: RunArgs) -> Result<Self, Self::Error> {
        let concurrency = if let Some(concurrency) = run_args.concurrency {
            parse_concurrency(&concurrency)?
        } else {
            DEFAULT_CONCURRENCY
        };

        let run_opts = RunOpts {
            concurrency,
            parallel: run_args.parallel,
            profile: run_args.profile,
            continue_on_error: run_args.continue_execution,
            pass_through_args: run_args.pass_through_args,
            only: run_args.only,
            dry_run: run_args.dry_run.is_some(),
            dry_run_json: run_args.dry_run == Some(DryRunMode::Json),
            graph_dot: run_args.graph.is_some(),
            graph_file: run_args.graph.map(|g| g.parse()).transpose()?,
            no_daemon: run_args.no_daemon,
            single_package: run_args.single_package,
        };

        let cache_opts = CacheOpts {
            override_dir: run_args.cache_dir.map(|c| c.parse()).transpose()?,
            skip_filesystem: run_args.remote_only,
            workers: run_args.cache_workers,
        };

        let run_cache_opts = RunCacheOpts {
            skip_reads: run_args.force,
            skip_writes: run_args.no_cache,
            task_output_mode_override: run_args.output_logs,
        };

        let scope_opts = ScopeOpts {
            legacy_filter: LegacyFilter {
                include_dependencies: run_args.include_dependencies,
                skip_dependents: run_args.no_deps,
                entrypoints: run_args.scope,
                since: run_args.since,
            },
            ignore_patterns: run_args.ignore,
            global_dep_patterns: run_args.global_deps,
            filter_patterns: run_args.filter,
        };

        Ok(Self {
            run_opts,
            cache_opts,
            run_cache_opts,
            scope_opts,
        })
    }
}

#[derive(Serialize)]
struct RunOpts {
    concurrency: usize,
    parallel: bool,
    profile: Option<String>,
    continue_on_error: bool,
    pass_through_args: Vec<String>,
    only: bool,
    dry_run: bool,
    dry_run_json: bool,
    graph_dot: bool,
    graph_file: Option<PathBuf>,
    no_daemon: bool,
    single_package: bool,
}

#[derive(Serialize)]
struct CacheOpts {
    override_dir: Option<PathBuf>,
    skip_filesystem: bool,
    workers: u32,
}

#[derive(Serialize)]
struct RemoteCacheOpts {
    team_id: String,
    signature: bool,
}

#[derive(Serialize)]
struct RunCacheOpts {
    skip_reads: bool,
    skip_writes: bool,
    task_output_mode_override: OutputLogsMode,
}

#[derive(Serialize)]
struct ScopeOpts {
    legacy_filter: LegacyFilter,
    ignore_patterns: Vec<String>,
    global_dep_patterns: Vec<String>,
    filter_patterns: Vec<String>,
}

#[derive(Serialize)]
struct LegacyFilter {
    include_dependencies: bool,
    skip_dependents: bool,
    entrypoints: Vec<String>,
    since: Option<String>,
}
