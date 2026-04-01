use sha2::{Digest, Sha256};
use tracing::{debug, warn};
use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
use turborepo_cache::AsyncCache;
use turborepo_types::{IncrementalPartition, TaskOutputs, TaskOutputsExt};

/// Per-partition result of an incremental fetch attempt.
#[derive(Debug, Clone)]
pub enum IncrementalFetchResult {
    /// Local files already existed, remote fetch was skipped.
    LocalFilesExist,
    /// Successfully restored from remote cache.
    RestoredFromRemote,
    /// No incremental state available (remote miss).
    NoState,
}

/// Aggregated result of all incremental fetches for a task.
#[derive(Debug, Clone, Default)]
pub struct IncrementalRestoreStatus {
    pub partition_results: Vec<IncrementalFetchResult>,
}

impl IncrementalRestoreStatus {
    pub fn any_restored(&self) -> bool {
        self.partition_results.iter().any(|r| {
            matches!(
                r,
                IncrementalFetchResult::LocalFilesExist
                    | IncrementalFetchResult::RestoredFromRemote
            )
        })
    }
}

/// Handles incremental cache operations for a single task execution.
///
/// Cache keys are derived from `(package, task, partition_index,
/// hash(partition_inputs))`. Branch is intentionally excluded: incremental
/// artifacts are tool-managed caches that the tool validates against current
/// source files. Getting "someone else's" state is never wrong, only
/// potentially suboptimal — the tool reconciles the delta.
///
/// When `inputs` are declared on a partition, they create separate cache
/// buckets so that incompatible tool states (e.g. different compiler versions)
/// don't clobber each other.
///
/// Uploads happen after every successful execution. The bandwidth cost is
/// accepted in v1 — upload diffing is a future optimization.
pub struct IncrementalTaskCache {
    partitions: Vec<IncrementalPartition>,
    package_name: String,
    task_name: String,
    cache: AsyncCache,
    repo_root: AbsoluteSystemPathBuf,
    package_dir: AbsoluteSystemPathBuf,
    /// When true, skip the on-disk file existence check and always fetch from
    /// remote. Driven by `--remote-only`.
    remote_only: bool,
}

impl IncrementalTaskCache {
    pub fn new(
        partitions: Vec<IncrementalPartition>,
        package_name: String,
        task_name: String,
        cache: AsyncCache,
        repo_root: AbsoluteSystemPathBuf,
        package_dir: AbsoluteSystemPathBuf,
        remote_only: bool,
    ) -> Self {
        Self {
            partitions,
            package_name,
            task_name,
            cache,
            repo_root,
            package_dir,
            remote_only,
        }
    }

    /// Fetch incremental artifacts for all partitions. Must complete before
    /// task execution begins.
    pub async fn fetch_all(&self) -> IncrementalRestoreStatus {
        let mut results = Vec::with_capacity(self.partitions.len());

        for (idx, partition) in self.partitions.iter().enumerate() {
            let result = self.fetch_partition(idx, partition).await;
            results.push(result);
        }

        IncrementalRestoreStatus {
            partition_results: results,
        }
    }

    /// Upload incremental artifacts for all partitions after successful
    /// task execution.
    pub async fn upload_all(&self) {
        for (idx, partition) in self.partitions.iter().enumerate() {
            if let Err(e) = self.upload_partition(idx, partition).await {
                warn!(
                    "incremental upload failed for {}#{} partition {}: {}",
                    self.package_name, self.task_name, idx, e
                );
            }
        }
    }

    async fn fetch_partition(
        &self,
        idx: usize,
        partition: &IncrementalPartition,
    ) -> IncrementalFetchResult {
        if !self.remote_only && self.local_files_exist(partition) {
            debug!(
                "incremental partition {} for {}#{}: local files exist, skipping remote",
                idx, self.package_name, self.task_name
            );
            return IncrementalFetchResult::LocalFilesExist;
        }

        let key = self.partition_cache_key(idx, partition);
        match self.cache.fetch(&self.repo_root, &key).await {
            Ok(Some(_)) => {
                debug!(
                    "incremental partition {} for {}#{}: restored from remote",
                    idx, self.package_name, self.task_name
                );
                IncrementalFetchResult::RestoredFromRemote
            }
            Ok(None) => {
                debug!(
                    "incremental partition {} for {}#{}: remote miss",
                    idx, self.package_name, self.task_name
                );
                IncrementalFetchResult::NoState
            }
            Err(e) => {
                warn!(
                    "incremental fetch failed for {}#{} partition {}: {}",
                    self.package_name, self.task_name, idx, e
                );
                IncrementalFetchResult::NoState
            }
        }
    }

    async fn upload_partition(
        &self,
        idx: usize,
        partition: &IncrementalPartition,
    ) -> Result<(), crate::Error> {
        let key = self.partition_cache_key(idx, partition);

        let files = self.collect_partition_files(partition)?;
        if files.is_empty() {
            debug!(
                "incremental upload: no files for partition {} of {}#{}",
                idx, self.package_name, self.task_name
            );
            return Ok(());
        }

        self.cache
            .put(self.repo_root.clone(), key, files, 0)
            .await
            .map_err(crate::Error::Cache)?;

        debug!(
            "incremental upload: partition {} of {}#{} uploaded",
            idx, self.package_name, self.task_name
        );

        Ok(())
    }

    /// Check if ANY files matching the partition's output globs exist on disk.
    fn local_files_exist(&self, partition: &IncrementalPartition) -> bool {
        let Ok(inclusions) = partition.outputs.validated_inclusions() else {
            return false;
        };
        let exclusions = partition
            .outputs
            .validated_exclusions()
            .unwrap_or_default();

        match globwalk::globwalk(
            &self.package_dir,
            &inclusions,
            &exclusions,
            globwalk::WalkType::Files,
        ) {
            Ok(files) => !files.is_empty(),
            Err(_) => false,
        }
    }

    /// Collect files matching the partition's output globs, returning
    /// repo-relative paths suitable for cache archiving.
    fn collect_partition_files(
        &self,
        partition: &IncrementalPartition,
    ) -> Result<Vec<AnchoredSystemPathBuf>, crate::Error> {
        let repo_relative_globs = self.repo_relative_outputs(&partition.outputs);
        let inclusions = repo_relative_globs.validated_inclusions()?;
        let exclusions = repo_relative_globs.validated_exclusions()?;

        let files = globwalk::globwalk(
            &self.repo_root,
            &inclusions,
            &exclusions,
            globwalk::WalkType::All,
        )?;

        let mut relative_paths: Vec<_> = files
            .into_iter()
            .map(|path| AnchoredSystemPathBuf::relative_path_between(&self.repo_root, &path))
            .collect();
        relative_paths.sort();
        Ok(relative_paths)
    }

    /// Convert package-relative output globs to repo-relative globs.
    fn repo_relative_outputs(&self, outputs: &TaskOutputs) -> TaskOutputs {
        let pkg_relative = AnchoredSystemPathBuf::relative_path_between(
            &self.repo_root,
            &self.package_dir,
        );
        let prefix = pkg_relative.to_string();

        let map_glob = |glob: &str, prefix: &str| -> String {
            if prefix.is_empty() {
                glob.to_string()
            } else {
                format!("{prefix}/{glob}")
            }
        };

        TaskOutputs {
            inclusions: outputs
                .inclusions
                .iter()
                .map(|g| map_glob(g, &prefix))
                .collect(),
            exclusions: outputs
                .exclusions
                .iter()
                .map(|g| map_glob(g, &prefix))
                .collect(),
        }
    }

    /// Compute a deterministic cache key for a partition.
    ///
    /// Key components: `(package, task, partition_index,
    /// hash(partition_inputs))`. The partition index ensures that multiple
    /// partitions on the same task get separate cache entries. When the
    /// partition has no inputs, the input hash is omitted — giving a single
    /// stable bucket for that partition.
    fn partition_cache_key(&self, idx: usize, partition: &IncrementalPartition) -> String {
        let input_hash = self.compute_input_hash(partition);
        let mut hasher = Sha256::new();
        hasher.update(b"incremental:v1:");
        hasher.update(self.package_name.as_bytes());
        hasher.update(b":");
        hasher.update(self.task_name.as_bytes());
        hasher.update(b":");
        hasher.update(idx.to_string().as_bytes());
        if !input_hash.is_empty() {
            hasher.update(b":");
            hasher.update(input_hash.as_bytes());
        }
        let hash = hex::encode(hasher.finalize());
        format!("incremental-{hash}")
    }

    /// Compute a hash of the partition's input files. Returns empty string if
    /// the partition has no inputs configured.
    fn compute_input_hash(&self, partition: &IncrementalPartition) -> String {
        if partition.inputs.is_empty() {
            return String::new();
        }

        let mut hasher = Sha256::new();

        let globs: Vec<_> = partition
            .inputs
            .iter()
            .filter(|g| !g.starts_with('!'))
            .cloned()
            .collect();
        let exclusions: Vec<_> = partition
            .inputs
            .iter()
            .filter_map(|g| g.strip_prefix('!').map(String::from))
            .collect();

        let Ok(inclusion_globs) = globs
            .iter()
            .map(|g| g.parse::<globwalk::ValidatedGlob>())
            .collect::<Result<Vec<_>, _>>()
        else {
            return String::new();
        };
        let Ok(exclusion_globs) = exclusions
            .iter()
            .map(|g| g.parse::<globwalk::ValidatedGlob>())
            .collect::<Result<Vec<_>, _>>()
        else {
            return String::new();
        };

        let Ok(files) = globwalk::globwalk(
            &self.package_dir,
            &inclusion_globs,
            &exclusion_globs,
            globwalk::WalkType::Files,
        ) else {
            return String::new();
        };

        let mut sorted_files: Vec<_> = files.into_iter().collect();
        sorted_files.sort();

        for file in &sorted_files {
            let relative =
                AnchoredSystemPathBuf::relative_path_between(&self.package_dir, file);
            hasher.update(relative.as_str().as_bytes());
            if let Ok(contents) = std::fs::read(file.as_std_path()) {
                hasher.update(&contents);
            }
        }

        hex::encode(hasher.finalize())
    }
}
