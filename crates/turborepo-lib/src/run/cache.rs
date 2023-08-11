use std::{
    fs::OpenOptions,
    io::{BufWriter, Write},
    rc::Rc,
};

use console::StyledObject;
use tracing::{debug, log::warn};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
use turborepo_cache::{AsyncCache, CacheError, CacheResponse, CacheSource};
use turborepo_ui::{replay_logs, ColorSelector, PrefixedUI, PrefixedWriter, GREY};

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
    daemon_client: Option<DaemonClient<DaemonConnector>>,
}

impl RunCache {
    pub fn new(
        cache: AsyncCache,
        repo_root: &AbsoluteSystemPath,
        opts: RunCacheOpts,
        color_selector: ColorSelector,
        daemon_client: Option<DaemonClient<DaemonConnector>>,
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
            daemon_client: self.daemon_client.clone(),
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
    daemon_client: Option<DaemonClient<DaemonConnector>>,
}

impl TaskCache {
    fn replay_log_file(
        &self,
        prefixed_ui: &mut PrefixedUI<impl Write>,
    ) -> Result<(), anyhow::Error> {
        if self.log_file_name.exists() {
            replay_logs(prefixed_ui, &self.log_file_name)?;
        }

        Ok(())
    }

    fn on_error(&self, prefixed_ui: &mut PrefixedUI<impl Write>) -> Result<(), anyhow::Error> {
        if self.task_output_mode == OutputLogsMode::ErrorsOnly {
            prefixed_ui.output(format!(
                "cache miss, executing {}",
                GREY.apply_to(&self.hash)
            ));
            self.replay_log_file(prefixed_ui)?;
        }

        Ok(())
    }

    fn output_writer(
        &self,
        prefix: StyledObject<String>,
        writer: impl Write,
    ) -> Result<Box<dyn Write>, anyhow::Error> {
        let pretty_writer = PrefixedWriter::new(prefix, writer);

        if self.caching_disabled || self.run_cache.writes_disabled {
            return Ok(Box::new(pretty_writer));
        }

        self.log_file_name.ensure_dir()?;

        let mut options = OpenOptions::new();
        options.create(true).write(true);

        let log_file = self.log_file_name.open_with_options(options)?;

        let buf_writer = BufWriter::new(log_file);

        if matches!(
            self.task_output_mode,
            OutputLogsMode::None | OutputLogsMode::HashOnly | OutputLogsMode::ErrorsOnly
        ) {
            Ok(Box::new(buf_writer))
        } else {
            Ok(Box::new(MultiWriter::new(vec![
                Box::new(pretty_writer),
                Box::new(buf_writer),
            ])))
        }
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
                    GREY.apply_to(&self.hash)
                ));
            }

            return Err(CacheError::CacheMiss.into());
        }

        let changed_output_count = if let Some(daemon_client) = &mut self.daemon_client {
            match daemon_client
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
                            GREY.apply_to(&self.hash)
                        ));
                    }

                    err
                })?;

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
                ));
            }
            OutputLogsMode::Full => {
                debug!("log file path: {}", self.log_file_name);
                prefixed_ui.output(format!(
                    "cache hit{}, suppressing logs {}",
                    more_context,
                    GREY.apply_to(&self.hash)
                ));
                self.replay_log_file(prefixed_ui)?;
            }
            _ => {}
        }

        Ok(cache_status)
    }
}
