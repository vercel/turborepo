use std::{io::Write, sync::Arc, time::Duration};

use console::StyledObject;
use tracing::debug;
use turbopath::{
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf,
};
use turborepo_cache::{AsyncCache, CacheError, CacheHitMetadata, CacheSource};
use turborepo_repository::package_graph::PackageInfo;
use turborepo_scm::SCM;
use turborepo_telemetry::events::{task::PackageTaskEventBuilder, TrackedErrors};
use turborepo_ui::{
    color, replay_logs, ColorSelector, LogWriter, PrefixedUI, PrefixedWriter, GREY, UI,
};

use crate::{
    cli::OutputLogsMode,
    daemon::{DaemonClient, DaemonConnector},
    hash::{FileHashes, TurboHash},
    opts::RunCacheOpts,
    run::task_id::TaskId,
    task_graph::{TaskDefinition, TaskOutputs},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Error replaying logs: {0}")]
    Ui(#[from] turborepo_ui::Error),
    #[error("Error accessing cache: {0}")]
    Cache(#[from] turborepo_cache::CacheError),
    #[error("Error finding outputs to save: {0}")]
    Globwalk(#[from] globwalk::WalkError),
    #[error("Invalid globwalk pattern: {0}")]
    Glob(#[from] globwalk::GlobError),
    #[error("Error with daemon: {0}")]
    Daemon(#[from] crate::daemon::DaemonError),
    #[error("no connection to daemon")]
    NoDaemon,
    #[error(transparent)]
    Scm(#[from] turborepo_scm::Error),
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
}

pub struct RunCache {
    task_output_mode: Option<OutputLogsMode>,
    cache: AsyncCache,
    reads_disabled: bool,
    writes_disabled: bool,
    repo_root: AbsoluteSystemPathBuf,
    color_selector: ColorSelector,
    daemon_client: Option<DaemonClient<DaemonConnector>>,
    ui: UI,
}

impl RunCache {
    pub fn new(
        cache: AsyncCache,
        repo_root: &AbsoluteSystemPath,
        opts: &RunCacheOpts,
        color_selector: ColorSelector,
        daemon_client: Option<DaemonClient<DaemonConnector>>,
        ui: UI,
        is_dry_run: bool,
    ) -> Self {
        let task_output_mode = if is_dry_run {
            Some(OutputLogsMode::None)
        } else {
            opts.task_output_mode_override
        };
        RunCache {
            task_output_mode,
            cache,
            reads_disabled: opts.skip_reads,
            writes_disabled: opts.skip_writes,
            repo_root: repo_root.to_owned(),
            color_selector,
            daemon_client,
            ui,
        }
    }

    pub fn task_cache(
        self: &Arc<Self>,
        // TODO: Group these in a struct
        task_definition: &TaskDefinition,
        workspace_info: &PackageInfo,
        task_id: TaskId<'static>,
        hash: &str,
    ) -> TaskCache {
        let log_file_path = self
            .repo_root
            .resolve(workspace_info.package_path())
            .resolve(&TaskDefinition::workspace_relative_log_file(task_id.task()));
        let repo_relative_globs =
            task_definition.repo_relative_hashable_outputs(&task_id, workspace_info.package_path());

        let mut task_output_mode = task_definition.output_mode;
        if let Some(task_output_mode_override) = self.task_output_mode {
            task_output_mode = task_output_mode_override;
        }

        let caching_disabled = !task_definition.cache;

        TaskCache {
            expanded_outputs: Vec::new(),
            run_cache: self.clone(),
            repo_relative_globs,
            hash: hash.to_owned(),
            task_id,
            task_output_mode,
            caching_disabled,
            log_file_path,
            daemon_client: self.daemon_client.clone(),
            ui: self.ui,
        }
    }

    pub async fn shutdown_cache(&self) {
        // Ignore errors coming from cache already shutting down
        self.cache.shutdown().await.ok();
    }
}

pub struct TaskCache {
    expanded_outputs: Vec<AnchoredSystemPathBuf>,
    run_cache: Arc<RunCache>,
    repo_relative_globs: TaskOutputs,
    hash: String,
    task_output_mode: OutputLogsMode,
    caching_disabled: bool,
    log_file_path: AbsoluteSystemPathBuf,
    daemon_client: Option<DaemonClient<DaemonConnector>>,
    ui: UI,
    task_id: TaskId<'static>,
}

impl TaskCache {
    pub fn replay_log_file(&self, prefixed_ui: &mut PrefixedUI<impl Write>) -> Result<(), Error> {
        if self.log_file_path.exists() {
            replay_logs(prefixed_ui, &self.log_file_path)?;
        }

        Ok(())
    }

    pub fn on_error(&self, prefixed_ui: &mut PrefixedUI<impl Write>) -> Result<(), Error> {
        if self.task_output_mode == OutputLogsMode::ErrorsOnly {
            prefixed_ui.output(format!(
                "cache miss, executing {}",
                color!(self.ui, GREY, "{}", self.hash)
            ));
            self.replay_log_file(prefixed_ui)?;
        }

        Ok(())
    }

    pub fn output_writer<W: Write>(
        &self,
        prefix: StyledObject<String>,
        writer: W,
    ) -> Result<LogWriter<W>, Error> {
        let mut log_writer = LogWriter::default();
        let prefixed_writer = PrefixedWriter::new(self.run_cache.ui, prefix, writer);

        if self.caching_disabled || self.run_cache.writes_disabled {
            log_writer.with_prefixed_writer(prefixed_writer);
            return Ok(log_writer);
        }

        log_writer.with_log_file(&self.log_file_path)?;

        if !matches!(
            self.task_output_mode,
            OutputLogsMode::None | OutputLogsMode::HashOnly | OutputLogsMode::ErrorsOnly
        ) {
            log_writer.with_prefixed_writer(prefixed_writer);
        }

        Ok(log_writer)
    }

    pub async fn exists(&self) -> Result<Option<CacheHitMetadata>, CacheError> {
        self.run_cache.cache.exists(&self.hash).await
    }

    pub async fn restore_outputs(
        &mut self,
        prefixed_ui: &mut PrefixedUI<impl Write>,
        telemetry: &PackageTaskEventBuilder,
    ) -> Result<Option<CacheHitMetadata>, Error> {
        if self.caching_disabled || self.run_cache.reads_disabled {
            if !matches!(
                self.task_output_mode,
                OutputLogsMode::None | OutputLogsMode::ErrorsOnly
            ) {
                prefixed_ui.output(format!(
                    "cache bypass, force executing {}",
                    color!(self.ui, GREY, "{}", self.hash)
                ));
            }

            return Ok(None);
        }

        let validated_inclusions = self.repo_relative_globs.validated_inclusions()?;

        let changed_output_count = if let Some(daemon_client) = &mut self.daemon_client {
            match daemon_client
                .get_changed_outputs(self.hash.to_string(), &validated_inclusions)
                .await
            {
                Ok(changed_output_globs) => changed_output_globs.len(),
                Err(err) => {
                    telemetry.track_error(TrackedErrors::DaemonSkipOutputRestoreCheckFailed);
                    debug!(
                        "Failed to check if we can skip restoring outputs for {}: {}. Proceeding \
                         to check cache",
                        self.task_id, err
                    );
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
            let cache_status = self
                .run_cache
                .cache
                .fetch(&self.run_cache.repo_root, &self.hash)
                .await?;

            let Some((cache_hit_metadata, restored_files)) = cache_status else {
                if !matches!(
                    self.task_output_mode,
                    OutputLogsMode::None | OutputLogsMode::ErrorsOnly
                ) {
                    prefixed_ui.output(format!(
                        "cache miss, executing {}",
                        color!(self.ui, GREY, "{}", self.hash)
                    ));
                }

                return Ok(None);
            };

            self.expanded_outputs = restored_files;

            if let Some(daemon_client) = &mut self.daemon_client {
                // Do we want to error the process if we can't parse the globs? We probably
                // won't have even gotten this far if this fails...
                let validated_exclusions = self.repo_relative_globs.validated_exclusions()?;
                if let Err(err) = daemon_client
                    .notify_outputs_written(
                        self.hash.clone(),
                        &validated_inclusions,
                        &validated_exclusions,
                        cache_hit_metadata.time_saved,
                    )
                    .await
                {
                    // Don't fail the whole operation just because we failed to
                    // watch the outputs
                    telemetry.track_error(TrackedErrors::DaemonFailedToMarkOutputsAsCached);
                    let task_id = &self.task_id;
                    debug!("Failed to mark outputs as cached for {task_id}: {err}");
                }
            }

            Some(cache_hit_metadata)
        } else {
            Some(CacheHitMetadata {
                source: CacheSource::Local,
                time_saved: 0,
            })
        };

        let more_context = if has_changed_outputs {
            ""
        } else {
            " (outputs already on disk)"
        };

        match self.task_output_mode {
            OutputLogsMode::HashOnly | OutputLogsMode::NewOnly => {
                prefixed_ui.output(format!(
                    "cache hit{}, suppressing logs {}",
                    more_context,
                    color!(self.ui, GREY, "{}", self.hash)
                ));
            }
            OutputLogsMode::Full => {
                debug!("log file path: {}", self.log_file_path);
                prefixed_ui.output(format!(
                    "cache hit{}, replaying logs {}",
                    more_context,
                    color!(self.ui, GREY, "{}", self.hash)
                ));
                self.replay_log_file(prefixed_ui)?;
            }
            // Note that if we're restoring from cache, the task succeeded
            // so we know we don't need to print anything for errors
            OutputLogsMode::ErrorsOnly | OutputLogsMode::None => {}
        }

        Ok(cache_status)
    }

    pub async fn save_outputs(
        &mut self,
        duration: Duration,
        telemetry: &PackageTaskEventBuilder,
    ) -> Result<(), Error> {
        if self.caching_disabled || self.run_cache.writes_disabled {
            return Ok(());
        }

        debug!("caching outputs: outputs: {:?}", &self.repo_relative_globs);

        let validated_inclusions = self.repo_relative_globs.validated_inclusions()?;
        let validated_exclusions = self.repo_relative_globs.validated_exclusions()?;
        let files_to_be_cached = globwalk::globwalk(
            &self.run_cache.repo_root,
            &validated_inclusions,
            &validated_exclusions,
            globwalk::WalkType::All,
        )?;

        let mut relative_paths = files_to_be_cached
            .into_iter()
            .map(|path| {
                AnchoredSystemPathBuf::relative_path_between(&self.run_cache.repo_root, &path)
            })
            .collect::<Vec<_>>();
        relative_paths.sort();
        self.run_cache
            .cache
            .put(
                self.run_cache.repo_root.clone(),
                self.hash.clone(),
                relative_paths.clone(),
                duration.as_millis() as u64,
            )
            .await?;

        if let Some(daemon_client) = self.daemon_client.as_mut() {
            let notify_result = daemon_client
                .notify_outputs_written(
                    self.hash.to_string(),
                    &validated_inclusions,
                    &validated_exclusions,
                    duration.as_millis() as u64,
                )
                .await
                .map_err(Error::from);

            if let Err(err) = notify_result {
                telemetry.track_error(TrackedErrors::DaemonFailedToMarkOutputsAsCached);
                let task_id = &self.task_id;
                debug!("failed to mark outputs as cached for {task_id}: {err}");
            }
        }

        self.expanded_outputs = relative_paths;

        Ok(())
    }

    pub fn expanded_outputs(&self) -> &[AnchoredSystemPathBuf] {
        &self.expanded_outputs
    }
}

#[derive(Clone)]
pub struct ConfigCache {
    hash: String,
    repo_root: AbsoluteSystemPathBuf,
    config_file: AbsoluteSystemPathBuf,
    anchored_path: AnchoredSystemPathBuf,
    cache: AsyncCache,
}

impl ConfigCache {
    pub fn new(
        hash: String,
        repo_root: AbsoluteSystemPathBuf,
        config_path: &[&str],
        cache: AsyncCache,
    ) -> Self {
        let config_file = repo_root.join_components(config_path);
        ConfigCache {
            hash,
            repo_root: repo_root.clone(),
            config_file: config_file.clone(),
            anchored_path: AnchoredSystemPathBuf::relative_path_between(&repo_root, &config_file),
            cache,
        }
    }

    pub fn hash(&self) -> &str {
        &self.hash
    }

    pub fn exists(&self) -> bool {
        self.config_file.try_exists().unwrap_or(false)
    }

    pub async fn restore(
        &self,
    ) -> Result<Option<(CacheHitMetadata, Vec<AnchoredSystemPathBuf>)>, CacheError> {
        self.cache.fetch(&self.repo_root, &self.hash).await
    }

    pub async fn save(&self) -> Result<(), CacheError> {
        match self.exists() {
            true => {
                debug!("config file exists, caching");
                self.cache
                    .put(
                        self.repo_root.clone(),
                        self.hash.clone(),
                        vec![self.anchored_path.clone()],
                        0,
                    )
                    .await
            }
            false => {
                debug!("config file does not exist, skipping cache save");
                Ok(())
            }
        }
    }

    // The config hash is used for task access tracing, and is keyed off of all
    // files in the repository
    pub fn calculate_config_hash(
        scm: &SCM,
        repo_root: &AbsoluteSystemPathBuf,
    ) -> Result<String, CacheError> {
        // empty path to get all files
        let anchored_root = match AnchoredSystemPath::new("") {
            Ok(anchored_root) => anchored_root,
            Err(_) => return Err(CacheError::ConfigCacheInvalidBase),
        };

        // empty inputs to get all files
        let inputs: Vec<String> = vec![];
        let hash_object = match scm.get_package_file_hashes(repo_root, anchored_root, &inputs, None)
        {
            Ok(hash_object) => hash_object,
            Err(_) => return Err(CacheError::ConfigCacheError),
        };

        // return the hash
        Ok(FileHashes(hash_object).hash())
    }
}
