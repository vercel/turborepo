use std::{fmt::Display, io::Write, rc::Rc};

use console::StyledObject;
use tokio::sync::Mutex;
use tracing::{debug, log::warn};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
use turborepo_cache::{AsyncCache, CacheError, CacheResponse, CacheSource};
use turborepo_ui::{replay_logs, ColorSelector, PrefixedUI, GREY, UI};

use crate::{
    cli::OutputLogsMode,
    daemon::{DaemonClient, DaemonConnector},
    opts::RunCacheOpts,
    package_task::PackageTask,
    task_graph::TaskOutputs,
};

struct RunCache {
    task_output_mode: Option<OutputLogsMode>,
    cache: AsyncCache,
    reads_disabled: bool,
    writes_disabled: bool,
    repo_root: AbsoluteSystemPathBuf,
    color_selector: ColorSelector,
    daemon_client: Option<Mutex<DaemonClient<DaemonConnector>>>,
}

impl RunCache {
    pub fn new(
        cache: AsyncCache,
        repo_root: &AbsoluteSystemPath,
        opts: RunCacheOpts,
        color_selector: ColorSelector,
        daemon_client: Option<Mutex<DaemonClient<DaemonConnector>>>,
    ) -> Self {
        RunCache {
            task_output_mode: opts.task_output_mode_override,
            cache,
            reads_disabled: opts.skip_reads,
            writes_disabled: opts.skip_writes,
            repo_root: repo_root.to_owned(),
            color_selector,
            daemon_client,
        }
    }

    pub fn task_cache(self: &Rc<Self>, package_task: PackageTask, hash: &str) -> TaskCache {
        let log_file_name = self.repo_root.join_component(&package_task.log_file);
        let hashable_outputs = package_task.hashable_outputs();
        let mut repo_relative_globs = TaskOutputs {
            inclusions: Vec::with_capacity(hashable_outputs.inclusions.len()),
            exclusions: Vec::with_capacity(hashable_outputs.exclusions.len()),
        };

        for output in hashable_outputs.inclusions {
            let inclusion_glob = package_task.dir.join_component(&output);
            repo_relative_globs
                .inclusions
                .push(inclusion_glob.to_string());
        }

        for output in hashable_outputs.exclusions {
            let exclusion_glob = package_task.dir.join_component(&output);
            repo_relative_globs
                .exclusions
                .push(exclusion_glob.to_string());
        }

        let mut task_output_mode = package_task.task_definition.output_mode;
        if let Some(task_output_mode_override) = self.task_output_mode {
            task_output_mode = task_output_mode_override;
        }

        let caching_disabled = !package_task.task_definition.cache;

        TaskCache {
            expanded_outputs: Vec::new(),
            run_cache: self.clone(),
            repo_relative_globs,
            hash: hash.to_owned(),
            package_task,
            task_output_mode,
            caching_disabled,
            log_file_name,
        }
    }
}

struct TaskCache {
    expanded_outputs: Vec<AnchoredSystemPathBuf>,
    run_cache: Rc<RunCache>,
    repo_relative_globs: TaskOutputs,
    hash: String,
    package_task: PackageTask,
    task_output_mode: OutputLogsMode,
    caching_disabled: bool,
    log_file_name: AbsoluteSystemPathBuf,
}

impl TaskCache {
    fn replay_log_file<D: Display + Clone, W: Write>(
        &self,
        prefixed_ui: &mut PrefixedUI<D, W>,
    ) -> Result<(), anyhow::Error> {
        if self.log_file_name.try_exists()? {
            replay_logs(prefixed_ui, &self.log_file_name)?;
        }

        Ok(())
    }

    async fn restore_outputs(
        &mut self,
        team_id: &str,
        team_slug: Option<&str>,
        prefixed_ui: &mut PrefixedUI<impl Write>,
    ) -> Result<CacheResponse, anyhow::Error> {
        if self.caching_disabled || self.run_cache.reads_disabled {
            if self.task_output_mode != OutputLogsMode::None
                && self.task_output_mode != OutputLogsMode::ErrorsOnly
            {
                prefixed_ui.output(format!(
                    "cache bypass, force executing {}",
                    GREY.apply_to(self.hash)
                ))?;
            }

            return Err(CacheError::CacheMiss.into());
        }

        let changed_output_count = if let Some(daemon_client) = self.run_cache.daemon_client {
            // TODO: Hook up daemon client
            // Not implemented because unclear where we need to
            // lock the daemon client. Should also print if we
            // are failing to check client.
            match daemon_client
                .lock()
                .await
                .get_changed_outputs(
                    self.hash.to_string(),
                    self.repo_relative_globs.inclusions.clone(),
                )
                .await
            {
                Ok(changed_output_globs) => changed_output_globs.len(),
                Err(err) => {
                    warn!(
                        "Failed to check if we can skip restoring outputs for {}: {:?}. \
                         Proceeding to check cache",
                        self.package_task.task_id, err
                    );
                    prefixed_ui.warn(format!(
                        "Failed to check if we can skip restoring outputs for {}: {:?}. \
                         Proceeding to check cache",
                        self.package_task.task_id, err
                    ));
                    self.repo_relative_globs.inclusions.len()
                }
            }
        } else {
            self.repo_relative_globs.inclusions.len()
        };

        let has_changed_outputs = changed_output_count > 0;

        let cache_status = if has_changed_outputs {
            // Note that we currently don't use the output globs when restoring, but we
            // could in the future to avoid doing unnecessary file I/O. We also
            // need to pass along the exclusion globs as well.
            let (cache_status, restored_files) = self
                .run_cache
                .cache
                .fetch(&self.run_cache.repo_root, &self.hash, team_id, team_slug)
                .await
                .map_err(|err| {
                    if matches!(err, CacheError::CacheMiss) {
                        prefixed_ui.output(format!(
                            "cache miss, executing {}",
                            GREY.apply_to(self.hash)
                        ))?;
                    }
                });

            self.expanded_outputs = restored_files;
            // TODO: Add notify_outputs_written
            cache_status
        } else {
            CacheResponse {
                source: CacheSource::Local,
                time_saved: 0,
            }
        };

        let more_context = if has_changed_outputs {
            ""
        } else {
            " (outputs already on disk)"
        };

        match self.task_output_mode {
            OutputLogsMode::HashOnly => {
                prefixed_ui.output(format!(
                    "cache hit{}, suppressing logs {}",
                    more_context,
                    GREY.apply_to(&self.hash)
                ))?;
            }
            OutputLogsMode::Full => {
                debug!("log file path: {}", self.log_file_name);
                prefixed_ui.output(format!(
                    "cache hit{}, suppressing logs {}",
                    more_context,
                    GREY.apply_to(&self.hash)
                ))?;
                self.replay_log_file(prefixed_ui)?;
            }
            _ => {}
        }

        Ok(cache_status)
    }
}
