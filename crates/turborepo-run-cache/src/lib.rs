//! Task-aware caching for Turborepo runs
//!
//! This crate provides the `RunCache` and `TaskCache` types that wrap the
//! lower-level `AsyncCache` from turborepo-cache with task-specific semantics,
//! including:
//! - Log file handling and output mode management
//! - Integration with output watchers for output tracking
//! - Task definition-aware output glob handling
//! - Incremental cache management for tool-specific artifacts

pub mod incremental;

use std::{
    collections::HashSet,
    io::Write,
    str::FromStr,
    sync::{Arc, Mutex},
    time::Duration,
};

use itertools::Itertools;
use tokio::sync::oneshot;
use tracing::{debug, info, log::warn};
use turbopath::{
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf,
};
use turborepo_cache::{
    AsyncCache, CacheError, CacheHitMetadata, CacheOpts, CacheSource, http::UploadMap,
};
use turborepo_hash::{FileHashes, TurboHash};
use turborepo_repository::package_graph::PackageInfo;
use turborepo_scm::SCM;
use turborepo_task_id::TaskId;
use turborepo_telemetry::events::{TrackedErrors, task::PackageTaskEventBuilder};
use turborepo_types::{
    OutputLogsMode, RunCacheOpts, TaskDefinition, TaskDefinitionExt, TaskOutputs, TaskOutputsExt,
};
use turborepo_ui::{ColorConfig, GREY, LogWriter, color, tui::event::CacheResult};

/// Errors that can occur during cache operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to replay logs: {0}")]
    Ui(#[from] turborepo_ui::Error),
    #[error("Failed to access cache: {0}")]
    Cache(#[from] turborepo_cache::CacheError),
    #[error("Failed to find outputs to save: {0}")]
    Globwalk(#[from] globwalk::WalkError),
    #[error("Invalid globwalk pattern: {0}")]
    Glob(#[from] globwalk::GlobError),
    #[error("Error with output watcher: {0}")]
    OutputWatcher(#[from] OutputWatcherError),
    #[error("Task spawn failed: {0}")]
    SpawnBlocking(String),
    #[error("Task output resolves outside of repository root: {0}")]
    OutputOutsideRepo(String),
    #[error(transparent)]
    Scm(#[from] turborepo_scm::Error),
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
}

/// Abstraction over output change tracking.
///
/// In watch mode, turbo needs to know which task outputs have changed since
/// they were last written. This prevents infinite rebuild loops: when a cache
/// restore writes output files to disk, those writes would otherwise trigger
/// the file watcher, causing the same tasks to re-run endlessly.
///
/// Implementors track registered output globs per task hash and report which
/// globs have been invalidated by subsequent file changes.
pub trait OutputWatcher: Send + Sync {
    /// Check which output globs have changed since they were last registered
    /// via [`notify_outputs_written`](Self::notify_outputs_written).
    ///
    /// Returns the subset of `output_globs` whose files have been modified.
    /// An empty result means all outputs are still on disk and unchanged.
    fn get_changed_outputs(
        &self,
        hash: String,
        output_globs: Vec<String>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<HashSet<String>, OutputWatcherError>> + Send>,
    >;

    /// Register output globs for a task hash so that future changes to
    /// matching files can be detected.
    fn notify_outputs_written(
        &self,
        hash: String,
        output_globs: Vec<String>,
        output_exclusion_globs: Vec<String>,
        time_saved: u64,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), OutputWatcherError>> + Send>>;
}

#[derive(Debug, thiserror::Error)]
#[error("output watcher error: {0}")]
pub struct OutputWatcherError(#[from] pub Box<dyn std::error::Error + Send + Sync>);

/// The run cache wraps an AsyncCache with task-aware semantics.
///
/// It manages:
/// - Output log mode overrides
/// - Cache read/write enable states
/// - Warning collection for missing outputs
pub struct RunCache {
    task_output_logs: Option<OutputLogsMode>,
    cache: AsyncCache,
    warnings: Arc<Mutex<Vec<String>>>,
    reads_disabled: bool,
    writes_disabled: bool,
    repo_root: AbsoluteSystemPathBuf,
    output_watcher: Option<Arc<dyn OutputWatcher>>,
    ui: ColorConfig,
    /// When using `outputLogs: "errors-only"`, show task hashes when tasks
    /// complete successfully. Controlled by the `errorsOnlyShowHash` future
    /// flag.
    errors_only_show_hash: bool,
    /// True when `--remote-only` is active, skips on-disk file checks for
    /// incremental.
    remote_only: bool,
}

/// Trait used to output cache information to user
impl RunCache {
    pub fn new(
        cache: AsyncCache,
        repo_root: &AbsoluteSystemPath,
        run_cache_opts: RunCacheOpts,
        cache_opts: &CacheOpts,
        output_watcher: Option<Arc<dyn OutputWatcher>>,
        ui: ColorConfig,
        is_dry_run: bool,
    ) -> Self {
        let task_output_logs = if is_dry_run {
            Some(OutputLogsMode::None)
        } else {
            run_cache_opts.task_output_logs_override
        };
        let remote_only = cache_opts.cache == turborepo_cache::CacheConfig::remote_only()
            || cache_opts.cache == turborepo_cache::CacheConfig::remote_read_only();
        RunCache {
            task_output_logs,
            cache,
            warnings: Default::default(),
            reads_disabled: !cache_opts.cache.remote.read && !cache_opts.cache.local.read,
            writes_disabled: !cache_opts.cache.remote.write && !cache_opts.cache.local.write,
            repo_root: repo_root.to_owned(),
            output_watcher,
            ui,
            errors_only_show_hash: run_cache_opts.errors_only_show_hash,
            remote_only,
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

        let mut task_output_logs = task_definition.output_logs;
        if let Some(task_output_logs_override) = self.task_output_logs {
            task_output_logs = task_output_logs_override;
        }

        let caching_disabled = !task_definition.cache;

        let incremental_cache = task_definition.incremental.as_ref().map(|partitions| {
            let package_dir = self.repo_root.resolve(workspace_info.package_path());
            incremental::IncrementalTaskCache::new(
                partitions.clone(),
                task_id.package().to_string(),
                task_id.task().to_string(),
                self.cache.clone(),
                self.repo_root.clone(),
                package_dir,
                self.remote_only,
            )
        });

        TaskCache {
            expanded_outputs: Vec::new(),
            run_cache: self.clone(),
            repo_relative_globs,
            hash: hash.to_owned(),
            task_id,
            task_output_logs,
            caching_disabled,
            log_file_path,
            output_watcher: self.output_watcher.clone(),
            ui: self.ui,
            warnings: self.warnings.clone(),
            errors_only_show_hash: self.errors_only_show_hash,
            incremental_cache,
        }
    }

    pub async fn shutdown_cache(
        &self,
    ) -> Result<(Arc<Mutex<UploadMap>>, oneshot::Receiver<()>), CacheError> {
        if let Ok(warnings) = self.warnings.lock() {
            for warning in warnings.iter().sorted() {
                warn!("{}", warning);
            }
        }
        // Ignore errors coming from cache already shutting down
        self.cache.start_shutdown().await
    }
}

fn expand_symlinked_output_roots(
    repo_root: &AbsoluteSystemPath,
    inclusions: &[globwalk::ValidatedGlob],
    exclusions: &[globwalk::ValidatedGlob],
) -> Result<HashSet<AbsoluteSystemPathBuf>, Error> {
    let mut followed_outputs = HashSet::new();

    for inclusion in inclusions {
        let Some((prefix, suffix)) = split_at_literal_prefix(inclusion.as_str()) else {
            continue;
        };

        let prefix_path = AnchoredSystemPath::new(&prefix)?;
        let absolute_prefix = repo_root.resolve(prefix_path);
        let Ok(metadata) = absolute_prefix.symlink_metadata() else {
            continue;
        };
        if !metadata.file_type().is_symlink() {
            continue;
        }

        let Ok(real_prefix) = absolute_prefix.to_realpath() else {
            continue;
        };
        if !repo_root.contains(&real_prefix) || !real_prefix.as_std_path().is_dir() {
            continue;
        }

        let include = [globwalk::ValidatedGlob::from_str(&suffix)?];
        let exclude = exclusions_for_symlinked_prefix(&prefix, exclusions)?;
        for real_output in
            globwalk::globwalk(&real_prefix, &include, &exclude, globwalk::WalkType::All)?
        {
            let relative_output =
                AnchoredSystemPathBuf::relative_path_between(&real_prefix, &real_output);
            followed_outputs.extend([absolute_prefix.resolve(&relative_output)]);
        }
    }

    Ok(followed_outputs)
}

fn split_at_literal_prefix(pattern: &str) -> Option<(String, String)> {
    let segments = pattern.split('/').collect::<Vec<_>>();
    let first_glob = segments
        .iter()
        .position(|segment| globwalk::is_glob_pattern(segment))?;
    if first_glob == 0 || first_glob == segments.len() {
        return None;
    }

    Some((
        segments[..first_glob].join("/"),
        segments[first_glob..].join("/"),
    ))
}

fn exclusions_for_symlinked_prefix(
    prefix: &str,
    exclusions: &[globwalk::ValidatedGlob],
) -> Result<Vec<globwalk::ValidatedGlob>, Error> {
    let prefix_with_separator = format!("{prefix}/");

    exclusions
        .iter()
        .filter_map(|exclusion| {
            exclusion
                .as_str()
                .strip_prefix(&prefix_with_separator)
                .map(globwalk::ValidatedGlob::from_str)
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

/// Cache state for a specific task execution.
///
/// Created by `RunCache::task_cache()`, this handles:
/// - Checking and restoring cached outputs
/// - Saving outputs after task execution
/// - Incremental cache fetch/upload for tool-specific artifacts
pub struct TaskCache {
    expanded_outputs: Vec<AnchoredSystemPathBuf>,
    run_cache: Arc<RunCache>,
    repo_relative_globs: TaskOutputs,
    hash: String,
    task_output_logs: OutputLogsMode,
    caching_disabled: bool,
    log_file_path: AbsoluteSystemPathBuf,
    output_watcher: Option<Arc<dyn OutputWatcher>>,
    ui: ColorConfig,
    task_id: TaskId<'static>,
    warnings: Arc<Mutex<Vec<String>>>,
    /// When using `outputLogs: "errors-only"`, show task hashes when tasks
    /// complete successfully. Controlled by the `errorsOnlyShowHash` future
    /// flag.
    errors_only_show_hash: bool,
    /// Incremental cache for tool-specific artifacts, present only when the
    /// task has `incremental` partitions configured.
    incremental_cache: Option<incremental::IncrementalTaskCache>,
}

impl TaskCache {
    pub fn output_logs(&self) -> OutputLogsMode {
        self.task_output_logs
    }

    pub fn is_caching_disabled(&self) -> bool {
        self.caching_disabled
    }

    /// Will read log file and write to output a line at a time
    fn replay_log_file(
        &self,
        task_handle: &mut turborepo_log::grouping::TaskHandle,
    ) -> Result<(), Error> {
        if self.log_file_path.exists() {
            let mut writer = task_handle.writer(turborepo_log::OutputChannel::Stdout);
            turborepo_ui::replay_logs(&mut writer, &self.log_file_path)?;
        }

        Ok(())
    }

    pub fn on_error(
        &self,
        task_handle: &mut turborepo_log::grouping::TaskHandle,
        tui_sender: Option<&turborepo_ui::sender::TaskSender>,
    ) -> Result<(), Error> {
        if self.task_output_logs == OutputLogsMode::ErrorsOnly {
            if !self.errors_only_show_hash {
                self.write_status(
                    task_handle,
                    tui_sender,
                    &format!(
                        "cache miss, executing {}",
                        color!(self.ui, GREY, "{}", self.hash)
                    ),
                    CacheResult::Miss,
                );
            }
            self.replay_log_file(task_handle)?;
        }

        Ok(())
    }

    /// Write a cache status message to the task output stream.
    ///
    /// This renders as plain text with the task's prefix — matching
    /// the old `PrefixedUI::output()` behavior. Empty messages are
    /// silently ignored.
    fn write_status(
        &self,
        task_handle: &mut turborepo_log::grouping::TaskHandle,
        tui_sender: Option<&turborepo_ui::sender::TaskSender>,
        message: &str,
        result: turborepo_ui::tui::event::CacheResult,
    ) {
        if let Some(sender) = tui_sender {
            sender.status(message, result);
        }
        if !message.is_empty() {
            let line = format!("{message}\n");
            task_handle.task_output(turborepo_log::OutputChannel::Stdout, line.as_bytes());
        }
    }

    pub fn output_writer<W: Write>(&self, writer: W) -> Result<LogWriter<W>, Error> {
        let mut log_writer = LogWriter::default();

        let cache_enabled = !self.caching_disabled && !self.run_cache.writes_disabled;
        // We need the log file when caching is enabled (normal case), but also
        // when the output mode is errors-only so that on_error can replay the
        // log file to show the output of a failed task.
        let needs_log_file = cache_enabled || self.task_output_logs == OutputLogsMode::ErrorsOnly;

        if needs_log_file {
            log_writer.with_log_file(&self.log_file_path)?;
        }

        match self.task_output_logs {
            OutputLogsMode::None | OutputLogsMode::HashOnly | OutputLogsMode::ErrorsOnly => {}
            OutputLogsMode::Full | OutputLogsMode::NewOnly => {
                log_writer.with_writer(writer);
            }
        }

        Ok(log_writer)
    }

    /// Check if a cache entry exists for this task.
    ///
    /// Used by dry runs to report cache status without restoring outputs.
    /// Mirrors the guard checks in `restore_outputs()` so that dry runs
    /// and real runs agree on cache status.
    pub async fn exists(&self) -> Result<Option<CacheHitMetadata>, CacheError> {
        if self.caching_disabled || self.run_cache.reads_disabled {
            return Ok(None);
        }
        self.run_cache.cache.exists(&self.hash).await
    }

    pub async fn restore_outputs(
        &mut self,
        task_handle: &mut turborepo_log::grouping::TaskHandle,
        tui_sender: Option<&turborepo_ui::sender::TaskSender>,
        telemetry: &PackageTaskEventBuilder,
    ) -> Result<Option<CacheHitMetadata>, Error> {
        if self.caching_disabled || self.run_cache.reads_disabled {
            let message = if self.task_output_logs == OutputLogsMode::ErrorsOnly
                && self.errors_only_show_hash
            {
                format!(
                    "cache bypass, force executing {} {}",
                    color!(self.ui, GREY, "{}", self.hash),
                    color!(self.ui, GREY, "(only logging errors)")
                )
            } else if matches!(
                self.task_output_logs,
                OutputLogsMode::None | OutputLogsMode::ErrorsOnly
            ) {
                String::new()
            } else {
                format!(
                    "cache bypass, force executing {}",
                    color!(self.ui, GREY, "{}", self.hash)
                )
            };
            self.write_status(
                task_handle,
                tui_sender,
                &message,
                turborepo_ui::tui::event::CacheResult::Miss,
            );

            return Ok(None);
        }

        let validated_inclusions = self.repo_relative_globs.validated_inclusions()?;

        // If an output watcher is connected, check whether outputs have changed
        // since they were last written. When outputs are already on disk and
        // unchanged, we can skip the cache restore entirely — avoiding file writes
        // that would otherwise trigger the file watcher and cause an infinite
        // rebuild loop in `turbo watch`.
        let inclusion_strings: Vec<String> = validated_inclusions
            .iter()
            .map(|g| g.as_ref().to_string())
            .collect();
        let changed_output_count = if let Some(output_watcher) = &self.output_watcher {
            match output_watcher
                .get_changed_outputs(self.hash.to_string(), inclusion_strings.clone())
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
                let message = if self.task_output_logs == OutputLogsMode::ErrorsOnly
                    && self.errors_only_show_hash
                {
                    format!(
                        "cache miss, executing {} {}",
                        color!(self.ui, GREY, "{}", self.hash),
                        color!(self.ui, GREY, "(only logging errors)")
                    )
                } else if matches!(
                    self.task_output_logs,
                    OutputLogsMode::None | OutputLogsMode::ErrorsOnly
                ) {
                    String::new()
                } else {
                    format!(
                        "cache miss, executing {}",
                        color!(self.ui, GREY, "{}", self.hash)
                    )
                };
                self.write_status(task_handle, tui_sender, &message, CacheResult::Miss);

                return Ok(None);
            };

            self.expanded_outputs = restored_files;

            if let Some(output_watcher) = &self.output_watcher {
                let exclusion_strings: Vec<String> = self
                    .repo_relative_globs
                    .validated_exclusions()?
                    .iter()
                    .map(|g| g.as_ref().to_string())
                    .collect();
                if let Err(err) = output_watcher
                    .notify_outputs_written(
                        self.hash.clone(),
                        inclusion_strings.clone(),
                        exclusion_strings,
                        cache_hit_metadata.time_saved,
                    )
                    .await
                {
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
                sha: None,
                dirty_hash: None,
            })
        };

        let more_context = if has_changed_outputs {
            ""
        } else {
            " (outputs already on disk)"
        };

        if let Some(sha_context) = format_sha_context(cache_status.as_ref()) {
            info!("{}: {sha_context}", self.hash);
        }

        match self.task_output_logs {
            OutputLogsMode::HashOnly | OutputLogsMode::NewOnly => {
                self.write_status(
                    task_handle,
                    tui_sender,
                    &format!(
                        "cache hit{}, suppressing logs {}",
                        more_context,
                        color!(self.ui, GREY, "{}", self.hash)
                    ),
                    CacheResult::Hit,
                );
            }
            OutputLogsMode::Full => {
                debug!("log file path: {}", self.log_file_path);
                self.write_status(
                    task_handle,
                    tui_sender,
                    &format!(
                        "cache hit{}, replaying logs {}",
                        more_context,
                        color!(self.ui, GREY, "{}", self.hash)
                    ),
                    CacheResult::Hit,
                );
                self.replay_log_file(task_handle)?;
            }
            OutputLogsMode::ErrorsOnly if self.errors_only_show_hash => {
                debug!("log file path: {}", self.log_file_path);
                self.write_status(
                    task_handle,
                    tui_sender,
                    &format!(
                        "cache hit{}, replaying logs (no errors) {}",
                        more_context,
                        color!(self.ui, GREY, "{}", self.hash)
                    ),
                    CacheResult::Hit,
                );
            }
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
        let files_to_be_cached = files_to_be_cached
            .into_iter()
            .chain(expand_symlinked_output_roots(
                &self.run_cache.repo_root,
                &validated_inclusions,
                &validated_exclusions,
            )?)
            .collect::<HashSet<_>>();

        for path in &files_to_be_cached {
            if !self.run_cache.repo_root.contains(path) {
                return Err(Error::OutputOutsideRepo(path.to_string()));
            }
        }

        // If we're only caching the log output, *and* output globs are not empty,
        // we should warn the user
        if files_to_be_cached.len() == 1 && !self.repo_relative_globs.is_empty() {
            let _ = self.warnings.lock().map(|mut warnings| {
                warnings.push(format!(
                    "no output files found for task {}. Please check your `outputs` key in \
                     `turbo.json`",
                    self.task_id
                ))
            });
        }

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

        if let Some(output_watcher) = &self.output_watcher {
            let inclusion_strings: Vec<String> = validated_inclusions
                .iter()
                .map(|g| g.as_ref().to_string())
                .collect();
            let exclusion_strings: Vec<String> = validated_exclusions
                .iter()
                .map(|g| g.as_ref().to_string())
                .collect();
            if let Err(err) = output_watcher
                .notify_outputs_written(
                    self.hash.to_string(),
                    inclusion_strings,
                    exclusion_strings,
                    duration.as_millis() as u64,
                )
                .await
            {
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

    /// Returns true if this task has incremental cache partitions configured
    /// AND caching is not fully disabled. Read/write flag checks are handled
    /// independently by `fetch_incremental` and `upload_incremental`.
    pub fn has_incremental(&self) -> bool {
        self.incremental_cache.is_some() && !self.caching_disabled
    }

    /// Fetch incremental artifacts for all partitions. Must complete before
    /// task execution begins. Returns the restore status for summary output.
    /// Respects --force (reads disabled) and --no-cache flags. Times out
    /// after 30 seconds to prevent blocking task execution on slow remote
    /// cache.
    pub async fn fetch_incremental(&self) -> incremental::IncrementalRestoreStatus {
        if self.caching_disabled || self.run_cache.reads_disabled {
            return incremental::IncrementalRestoreStatus::default();
        }
        let Some(incremental) = &self.incremental_cache else {
            return incremental::IncrementalRestoreStatus::default();
        };
        match tokio::time::timeout(std::time::Duration::from_secs(30), incremental.fetch_all())
            .await
        {
            Ok(status) => status,
            Err(_) => {
                warn!(
                    "incremental fetch timed out after 30s, proceeding without incremental state"
                );
                incremental::IncrementalRestoreStatus::default()
            }
        }
    }

    /// Upload incremental artifacts for all partitions after successful
    /// task execution. Failures are logged as warnings but do not affect
    /// the task result. Respects --no-cache flag (skips when writes are
    /// disabled). Not affected by --force (which only disables reads).
    /// Times out after 60 seconds to prevent hanging process exit on slow
    /// remote cache. Returns the number of partition upload failures
    /// (0 = all succeeded or none configured).
    pub async fn upload_incremental(&self) -> usize {
        if self.caching_disabled || self.run_cache.writes_disabled {
            return 0;
        }
        let Some(incremental) = &self.incremental_cache else {
            return 0;
        };
        match tokio::time::timeout(std::time::Duration::from_secs(60), incremental.upload_all())
            .await
        {
            Ok(failures) => failures,
            Err(_) => {
                warn!("incremental upload timed out after 60s, skipping remaining uploads");
                1
            }
        }
    }
}

/// Cache for configuration files (like task access tracing config).
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
        let hash_object =
            match scm.get_package_file_hashes(repo_root, anchored_root, &inputs, false, None, None)
            {
                Ok(hash_object) => hash_object,
                Err(_) => return Err(CacheError::ConfigCacheError),
            };

        let mut file_hashes: Vec<_> = hash_object.into_iter().collect();
        file_hashes.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));
        FileHashes(file_hashes)
            .try_hash()
            .map_err(|_| CacheError::ConfigCacheError)
    }
}

/// Build a "cache hit produced by sha: <sha>" or "cache hit produced by sha:
/// <sha> (dirty)" message for verbose logging. Returns `None` when no SHA is
/// available.
fn format_sha_context(meta: Option<&CacheHitMetadata>) -> Option<String> {
    meta.and_then(|m| m.sha.as_deref()).map(|sha| {
        let dirty = meta.and_then(|m| m.dirty_hash.as_deref()).is_some();
        if dirty {
            format!("cache hit produced by sha: {sha} (dirty)")
        } else {
            format!("cache hit produced by sha: {sha}")
        }
    })
}

#[cfg(test)]
mod test {
    use std::{
        collections::HashSet,
        io::Write,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use tempfile::tempdir;
    use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
    use turborepo_cache::{AsyncCache, CacheActions, CacheConfig, CacheOpts, LazyScmState};
    use turborepo_log::{LogSink, Logger, OutputChannel, grouping::GroupingLayer};
    use turborepo_task_id::TaskId;
    use turborepo_telemetry::events::task::PackageTaskEventBuilder;
    use turborepo_types::{OutputLogsMode, TaskOutputs};
    use turborepo_ui::ColorConfig;

    use super::{OutputWatcher, OutputWatcherError, RunCache, TaskCache};

    fn local_cache_opts(repo_root: &AbsoluteSystemPathBuf) -> CacheOpts {
        CacheOpts {
            cache_dir: repo_root.as_path().join(".turbo/cache"),
            cache: CacheConfig {
                local: CacheActions::enabled(),
                remote: CacheActions::disabled(),
            },
            workers: 1,
            remote_cache_opts: None,
            cache_max_age: None,
            cache_max_size: None,
        }
    }

    fn task_cache_for_outputs(
        repo_root: &AbsoluteSystemPathBuf,
        outputs: Vec<String>,
    ) -> TaskCache {
        task_cache_for_outputs_with_mode(repo_root, outputs, OutputLogsMode::None, false)
    }

    fn task_cache_for_outputs_with_mode(
        repo_root: &AbsoluteSystemPathBuf,
        outputs: Vec<String>,
        task_output_logs: OutputLogsMode,
        errors_only_show_hash: bool,
    ) -> TaskCache {
        let cache_opts = local_cache_opts(repo_root);
        let cache = AsyncCache::new(
            &cache_opts,
            repo_root,
            None,
            None,
            None,
            LazyScmState::resolved(None),
        )
        .unwrap();
        let warnings = Arc::new(Mutex::new(Vec::new()));
        let ui = ColorConfig::new(true);
        let run_cache = Arc::new(RunCache {
            task_output_logs: None,
            cache,
            warnings: warnings.clone(),
            reads_disabled: false,
            writes_disabled: false,
            repo_root: repo_root.to_owned(),
            output_watcher: None,
            ui,
            errors_only_show_hash,
            remote_only: false,
        });

        TaskCache {
            expanded_outputs: Vec::new(),
            run_cache,
            repo_relative_globs: TaskOutputs {
                inclusions: outputs,
                exclusions: Vec::new(),
            },
            hash: "test-hash".to_string(),
            task_id: TaskId::new("pkg", "build"),
            task_output_logs,
            caching_disabled: false,
            log_file_path: repo_root.join_components(&["pkg", ".turbo", "turbo-build.log"]),
            output_watcher: None,
            ui,
            warnings,
            errors_only_show_hash,
            incremental_cache: None,
        }
    }

    #[derive(Default)]
    struct RecordingSink {
        output: Mutex<Vec<u8>>,
    }

    impl RecordingSink {
        fn output_string(&self) -> String {
            String::from_utf8_lossy(&self.output.lock().unwrap()).into_owned()
        }
    }

    impl LogSink for RecordingSink {
        fn emit(&self, _event: &turborepo_log::LogEvent) {}

        fn task_output(&self, _task: &str, _channel: OutputChannel, bytes: &[u8]) {
            self.output.lock().unwrap().extend_from_slice(bytes);
        }
    }

    fn recording_task_handle() -> (Arc<RecordingSink>, turborepo_log::grouping::TaskHandle) {
        let sink = Arc::new(RecordingSink::default());
        let logger = Arc::new(Logger::new(vec![Box::new(sink.clone())]));
        let layer = GroupingLayer::new(logger, turborepo_log::grouping::GroupingMode::Passthrough);
        let handle = layer.task("pkg#build");
        (sink, handle)
    }

    /// Mock OutputWatcher that records calls and returns configurable results.
    struct MockOutputWatcher {
        changed_outputs: Result<HashSet<String>, &'static str>,
        notify_result: Result<(), &'static str>,
        get_changed_call_count: AtomicUsize,
        notify_call_count: AtomicUsize,
        was_called: AtomicBool,
    }

    impl MockOutputWatcher {
        fn returning_no_changes() -> Self {
            Self {
                changed_outputs: Ok(HashSet::new()),
                notify_result: Ok(()),
                get_changed_call_count: AtomicUsize::new(0),
                notify_call_count: AtomicUsize::new(0),
                was_called: AtomicBool::new(false),
            }
        }

        fn returning_all_changed(globs: Vec<String>) -> Self {
            Self {
                changed_outputs: Ok(globs.into_iter().collect()),
                notify_result: Ok(()),
                get_changed_call_count: AtomicUsize::new(0),
                notify_call_count: AtomicUsize::new(0),
                was_called: AtomicBool::new(false),
            }
        }

        fn returning_get_error() -> Self {
            Self {
                changed_outputs: Err("watcher unavailable"),
                notify_result: Ok(()),
                get_changed_call_count: AtomicUsize::new(0),
                notify_call_count: AtomicUsize::new(0),
                was_called: AtomicBool::new(false),
            }
        }

        fn returning_notify_error() -> Self {
            Self {
                changed_outputs: Ok(HashSet::new()),
                notify_result: Err("notify failed"),
                get_changed_call_count: AtomicUsize::new(0),
                notify_call_count: AtomicUsize::new(0),
                was_called: AtomicBool::new(false),
            }
        }

        fn get_changed_calls(&self) -> usize {
            self.get_changed_call_count.load(Ordering::SeqCst)
        }

        fn notify_calls(&self) -> usize {
            self.notify_call_count.load(Ordering::SeqCst)
        }
    }

    impl OutputWatcher for MockOutputWatcher {
        fn get_changed_outputs(
            &self,
            _hash: String,
            _output_globs: Vec<String>,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<HashSet<String>, OutputWatcherError>>
                    + Send,
            >,
        > {
            self.get_changed_call_count.fetch_add(1, Ordering::SeqCst);
            self.was_called.store(true, Ordering::SeqCst);
            let result = match &self.changed_outputs {
                Ok(set) => Ok(set.clone()),
                Err(msg) => Err(OutputWatcherError(Box::new(std::io::Error::other(*msg)))),
            };
            Box::pin(async move { result })
        }

        fn notify_outputs_written(
            &self,
            _hash: String,
            _output_globs: Vec<String>,
            _output_exclusion_globs: Vec<String>,
            _time_saved: u64,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<(), OutputWatcherError>> + Send>,
        > {
            self.notify_call_count.fetch_add(1, Ordering::SeqCst);
            self.was_called.store(true, Ordering::SeqCst);
            let result = match &self.notify_result {
                Ok(()) => Ok(()),
                Err(msg) => Err(OutputWatcherError(Box::new(std::io::Error::other(*msg)))),
            };
            Box::pin(async move { result })
        }
    }

    // The OutputWatcher trait defines the contract that both the DaemonClient
    // (current) and the in-process GlobWatcher (future) must satisfy. These
    // tests characterize the exact behaviors that TaskCache relies on.

    #[tokio::test]
    async fn output_watcher_no_changes_returns_empty_set() {
        let watcher = MockOutputWatcher::returning_no_changes();
        let result = watcher
            .get_changed_outputs("abc123".into(), vec!["dist/**".into()])
            .await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn output_watcher_some_changes_returns_changed_globs() {
        let watcher =
            MockOutputWatcher::returning_all_changed(vec!["dist/**".into(), ".next/**".into()]);
        let result = watcher
            .get_changed_outputs(
                "abc123".into(),
                vec!["dist/**".into(), ".next/**".into(), "build/**".into()],
            )
            .await;
        let changed = result.unwrap();
        assert!(changed.contains("dist/**"));
        assert!(changed.contains(".next/**"));
        assert!(!changed.contains("build/**"));
    }

    #[tokio::test]
    async fn output_watcher_get_error_is_recoverable() {
        // When get_changed_outputs fails, the caller should fall back to
        // treating all outputs as changed (normal cache restore path).
        let watcher = MockOutputWatcher::returning_get_error();
        let result = watcher
            .get_changed_outputs("abc123".into(), vec!["dist/**".into()])
            .await;
        assert!(result.is_err());
        // The error should be displayable for logging
        let err = result.unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("watcher unavailable"));
    }

    #[tokio::test]
    async fn output_watcher_notify_success() {
        let watcher = MockOutputWatcher::returning_no_changes();
        let result = watcher
            .notify_outputs_written(
                "abc123".into(),
                vec!["dist/**".into()],
                vec!["dist/cache/**".into()],
                1500,
            )
            .await;
        assert!(result.is_ok());
        assert_eq!(watcher.notify_calls(), 1);
    }

    #[tokio::test]
    async fn output_watcher_notify_error_is_recoverable() {
        // When notify_outputs_written fails, the caller should log and
        // continue — it's not a fatal error.
        let watcher = MockOutputWatcher::returning_notify_error();
        let result = watcher
            .notify_outputs_written("abc123".into(), vec!["dist/**".into()], vec![], 0)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn output_watcher_unchanged_then_notify_then_check_again() {
        // Simulates the lifecycle: outputs are on disk and unchanged, then
        // a cache restore writes new files and notifies, then a subsequent
        // check should reflect the new state.
        let watcher = MockOutputWatcher::returning_no_changes();

        // First check: nothing changed
        let result = watcher
            .get_changed_outputs("hash1".into(), vec!["dist/**".into()])
            .await
            .unwrap();
        assert!(result.is_empty());
        assert_eq!(watcher.get_changed_calls(), 1);

        // Notify after restore
        watcher
            .notify_outputs_written("hash1".into(), vec!["dist/**".into()], vec![], 500)
            .await
            .unwrap();
        assert_eq!(watcher.notify_calls(), 1);

        // Second check: still unchanged in this mock (real GlobWatcher would
        // track actual file changes between calls)
        let result = watcher
            .get_changed_outputs("hash1".into(), vec!["dist/**".into()])
            .await
            .unwrap();
        assert!(result.is_empty());
        assert_eq!(watcher.get_changed_calls(), 2);
    }

    #[tokio::test]
    async fn output_watcher_different_hashes_are_independent() {
        // Each task hash should be tracked independently. Getting changed
        // outputs for one hash should not affect another.
        let watcher = MockOutputWatcher::returning_all_changed(vec!["dist/**".into()]);

        let result1 = watcher
            .get_changed_outputs("hash-a".into(), vec!["dist/**".into()])
            .await
            .unwrap();
        let result2 = watcher
            .get_changed_outputs("hash-b".into(), vec!["dist/**".into()])
            .await
            .unwrap();

        assert_eq!(result1, result2);
        assert_eq!(watcher.get_changed_calls(), 2);
    }

    #[tokio::test]
    async fn output_watcher_error_type_is_send_sync() {
        // OutputWatcherError must be Send + Sync for use across async boundaries
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<OutputWatcherError>();
    }

    #[tokio::test]
    async fn output_writer_respects_output_log_modes() {
        let temp = tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(temp.path()).unwrap();

        for (mode, forwards_to_terminal, writes_log_file) in [
            (OutputLogsMode::Full, true, true),
            (OutputLogsMode::NewOnly, true, true),
            (OutputLogsMode::HashOnly, false, true),
            (OutputLogsMode::None, false, true),
            (OutputLogsMode::ErrorsOnly, false, true),
        ] {
            let task_cache = task_cache_for_outputs_with_mode(
                &repo_root,
                vec!["pkg/.turbo/turbo-build.log".to_string()],
                mode,
                false,
            );
            let _ = task_cache.log_file_path.remove_file();

            let mut terminal = Vec::new();
            {
                let mut writer = task_cache.output_writer(&mut terminal).unwrap();
                writer.write_all(b"task-output\n").unwrap();
            }

            assert_eq!(
                String::from_utf8_lossy(&terminal).contains("task-output"),
                forwards_to_terminal,
                "{mode:?} terminal forwarding"
            );
            assert_eq!(
                task_cache.log_file_path.exists(),
                writes_log_file,
                "{mode:?} log file creation"
            );
        }
    }

    #[tokio::test]
    async fn cache_miss_status_respects_output_log_modes() {
        let temp = tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(temp.path()).unwrap();
        let telemetry = PackageTaskEventBuilder::new("pkg", "build");

        for (mode, errors_only_show_hash, expected, unexpected) in [
            (
                OutputLogsMode::Full,
                false,
                Some("cache miss, executing test-hash"),
                None,
            ),
            (
                OutputLogsMode::NewOnly,
                false,
                Some("cache miss, executing test-hash"),
                None,
            ),
            (
                OutputLogsMode::HashOnly,
                false,
                Some("cache miss, executing test-hash"),
                None,
            ),
            (OutputLogsMode::None, false, None, Some("cache miss")),
            (OutputLogsMode::ErrorsOnly, false, None, Some("cache miss")),
            (
                OutputLogsMode::ErrorsOnly,
                true,
                Some("cache miss, executing test-hash (only logging errors)"),
                None,
            ),
        ] {
            let mut task_cache = task_cache_for_outputs_with_mode(
                &repo_root,
                vec!["pkg/.turbo/turbo-build.log".to_string()],
                mode,
                errors_only_show_hash,
            );
            let (sink, mut handle) = recording_task_handle();

            let result = task_cache
                .restore_outputs(&mut handle, None, &telemetry)
                .await
                .unwrap();
            assert!(result.is_none());

            let output = sink.output_string();
            if let Some(expected) = expected {
                assert!(output.contains(expected), "{mode:?}: {output}");
            }
            if let Some(unexpected) = unexpected {
                assert!(!output.contains(unexpected), "{mode:?}: {output}");
            }
        }
    }

    #[tokio::test]
    async fn cache_hit_status_respects_output_log_modes() {
        let temp = tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(temp.path()).unwrap();
        let log_file = repo_root.join_components(&["pkg", ".turbo", "turbo-build.log"]);
        log_file.ensure_dir().unwrap();
        log_file.create_with_contents("cached-output\n").unwrap();

        let telemetry = PackageTaskEventBuilder::new("pkg", "build");
        let mut warmer = task_cache_for_outputs_with_mode(
            &repo_root,
            vec!["pkg/.turbo/turbo-build.log".to_string()],
            OutputLogsMode::Full,
            false,
        );
        warmer
            .save_outputs(Duration::from_millis(10), &telemetry)
            .await
            .unwrap();
        warmer.run_cache.cache.wait().await.unwrap();
        log_file.remove_file().unwrap();

        for (mode, errors_only_show_hash, expected, replays_cached_output) in [
            (
                OutputLogsMode::Full,
                false,
                Some("cache hit, replaying logs test-hash"),
                true,
            ),
            (
                OutputLogsMode::NewOnly,
                false,
                Some("cache hit, suppressing logs test-hash"),
                false,
            ),
            (
                OutputLogsMode::HashOnly,
                false,
                Some("cache hit, suppressing logs test-hash"),
                false,
            ),
            (OutputLogsMode::None, false, None, false),
            (OutputLogsMode::ErrorsOnly, false, None, false),
            (
                OutputLogsMode::ErrorsOnly,
                true,
                Some("cache hit, replaying logs (no errors) test-hash"),
                false,
            ),
        ] {
            let mut task_cache = task_cache_for_outputs_with_mode(
                &repo_root,
                vec!["pkg/.turbo/turbo-build.log".to_string()],
                mode,
                errors_only_show_hash,
            );
            let (sink, mut handle) = recording_task_handle();

            let result = task_cache
                .restore_outputs(&mut handle, None, &telemetry)
                .await
                .unwrap();
            assert!(result.is_some());

            let output = sink.output_string();
            if let Some(expected) = expected {
                assert!(output.contains(expected), "{mode:?}: {output}");
            } else {
                assert!(output.is_empty(), "{mode:?}: {output}");
            }
            assert_eq!(
                output.contains("cached-output"),
                replays_cached_output,
                "{mode:?}: {output}"
            );
        }
    }

    #[tokio::test]
    async fn errors_only_replays_log_file_on_error() {
        let temp = tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(temp.path()).unwrap();
        let log_file = repo_root.join_components(&["pkg", ".turbo", "turbo-build.log"]);
        log_file.ensure_dir().unwrap();
        log_file.create_with_contents("error-output\n").unwrap();

        let task_cache = task_cache_for_outputs_with_mode(
            &repo_root,
            vec!["pkg/.turbo/turbo-build.log".to_string()],
            OutputLogsMode::ErrorsOnly,
            false,
        );
        let (sink, mut handle) = recording_task_handle();

        task_cache.on_error(&mut handle, None).unwrap();

        let output = sink.output_string();
        assert!(output.contains("cache miss, executing test-hash"));
        assert!(output.contains("error-output"));
    }

    #[tokio::test]
    async fn save_outputs_allows_files_inside_repo_root() {
        let temp = tempdir().unwrap();
        let repo_dir = temp.path().join("repo");
        std::fs::create_dir_all(repo_dir.join("dist")).unwrap();
        std::fs::write(repo_dir.join("dist/output.txt"), b"output").unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_dir.as_path()).unwrap();
        let mut task_cache = task_cache_for_outputs(&repo_root, vec!["dist/**".to_string()]);
        let telemetry = PackageTaskEventBuilder::new("pkg", "build");

        task_cache
            .save_outputs(Duration::from_millis(10), &telemetry)
            .await
            .unwrap();

        let expected = AnchoredSystemPathBuf::from_raw(format!(
            "dist{}output.txt",
            std::path::MAIN_SEPARATOR_STR
        ))
        .unwrap();

        assert!(task_cache.expanded_outputs().contains(&expected));
    }

    #[tokio::test]
    async fn save_outputs_rejects_glob_that_escapes_repo_root() {
        let temp = tempdir().unwrap();
        let repo_dir = temp.path().join("repo");
        let outside_dir = temp.path().join("outside");
        std::fs::create_dir_all(&repo_dir).unwrap();
        std::fs::create_dir_all(&outside_dir).unwrap();
        std::fs::write(outside_dir.join("output.txt"), b"outside").unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_dir.as_path()).unwrap();
        let mut task_cache = task_cache_for_outputs(&repo_root, vec!["../outside/**".to_string()]);
        let telemetry = PackageTaskEventBuilder::new("pkg", "build");

        let result = task_cache
            .save_outputs(Duration::from_millis(10), &telemetry)
            .await;

        assert!(
            result.is_err(),
            "outputs outside repo root should be rejected before cache upload"
        );
        assert!(task_cache.expanded_outputs().is_empty());
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn save_outputs_does_not_follow_symlink_that_escapes_repo_root() {
        let temp = tempdir().unwrap();
        let repo_dir = temp.path().join("repo");
        let outside_dir = temp.path().join("outside");
        std::fs::create_dir_all(&repo_dir).unwrap();
        std::fs::create_dir_all(&outside_dir).unwrap();
        std::fs::write(outside_dir.join("output.txt"), b"outside").unwrap();
        std::os::unix::fs::symlink("../outside", repo_dir.join("dist")).unwrap();

        let repo_root = AbsoluteSystemPathBuf::try_from(repo_dir.as_path()).unwrap();
        let mut task_cache = task_cache_for_outputs(&repo_root, vec!["dist/**".to_string()]);
        let telemetry = PackageTaskEventBuilder::new("pkg", "build");

        task_cache
            .save_outputs(Duration::from_millis(10), &telemetry)
            .await
            .unwrap();

        task_cache.run_cache.cache.wait().await.unwrap();
        let cached = task_cache
            .run_cache
            .cache
            .fetch(&repo_root, "test-hash")
            .await
            .unwrap();

        assert!(
            cached.is_none(),
            "symlinked output outside repo root should not write a cache artifact"
        );
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn save_outputs_follows_internal_symlinked_output_root() {
        let temp = tempdir().unwrap();
        let repo_dir = temp.path().join("repo");
        let package_dir = repo_dir.join("pkg");
        std::fs::create_dir_all(package_dir.join("actual-dist")).unwrap();
        std::fs::write(package_dir.join("actual-dist/output.txt"), b"output").unwrap();
        std::os::unix::fs::symlink("actual-dist", package_dir.join("dist")).unwrap();

        let repo_root = AbsoluteSystemPathBuf::try_from(repo_dir.as_path()).unwrap();
        let mut task_cache = task_cache_for_outputs(&repo_root, vec!["pkg/dist/**".to_string()]);
        let telemetry = PackageTaskEventBuilder::new("pkg", "build");

        task_cache
            .save_outputs(Duration::from_millis(10), &telemetry)
            .await
            .unwrap();

        task_cache.run_cache.cache.wait().await.unwrap();
        let (_metadata, cached_files) = task_cache
            .run_cache
            .cache
            .fetch(&repo_root, "test-hash")
            .await
            .unwrap()
            .expect("symlinked output root should be cached");

        assert!(
            cached_files.contains(
                &AnchoredSystemPathBuf::from_raw(format!(
                    "pkg{}dist{}output.txt",
                    std::path::MAIN_SEPARATOR,
                    std::path::MAIN_SEPARATOR
                ))
                .unwrap()
            )
        );
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn save_outputs_ignores_symlink_cycles() {
        let temp = tempdir().unwrap();
        let repo_dir = temp.path().join("repo");
        std::fs::create_dir_all(repo_dir.join("apps/app/node_modules/@vercel")).unwrap();
        std::fs::create_dir_all(repo_dir.join("apps/app/node_modules/.cache")).unwrap();
        std::fs::create_dir_all(
            repo_dir.join("packages/navigation-metrics/examples/basic/node_modules/@vercel"),
        )
        .unwrap();
        std::fs::write(
            repo_dir.join("apps/app/node_modules/.cache/tsbuildinfo.json"),
            b"{}",
        )
        .unwrap();

        std::os::unix::fs::symlink(
            "../../../../packages/navigation-metrics",
            repo_dir.join("apps/app/node_modules/@vercel/navigation-metrics"),
        )
        .unwrap();
        std::os::unix::fs::symlink(
            "../../../..",
            repo_dir
                .join("packages/navigation-metrics")
                .join("examples/basic/node_modules/@vercel/navigation-metrics"),
        )
        .unwrap();

        let repo_root = AbsoluteSystemPathBuf::try_from(repo_dir.as_path()).unwrap();
        let mut task_cache = task_cache_for_outputs(
            &repo_root,
            vec!["**/node_modules/.cache/tsbuildinfo.json".to_string()],
        );
        let telemetry = PackageTaskEventBuilder::new("pkg", "type-check");

        task_cache
            .save_outputs(Duration::from_millis(10), &telemetry)
            .await
            .unwrap();

        let expected = AnchoredSystemPathBuf::from_raw(format!(
            "apps{}app{}node_modules{}.cache{}tsbuildinfo.json",
            std::path::MAIN_SEPARATOR_STR,
            std::path::MAIN_SEPARATOR_STR,
            std::path::MAIN_SEPARATOR_STR,
            std::path::MAIN_SEPARATOR_STR,
        ))
        .unwrap();

        assert!(task_cache.expanded_outputs().contains(&expected));
    }
}
