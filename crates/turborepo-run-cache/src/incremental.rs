use std::io::Read;

use futures::future::join_all;
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
    /// No incremental state available (remote miss or error).
    NoState,
}

/// Aggregated result of all incremental fetches for a task.
#[derive(Debug, Clone, Default)]
pub struct IncrementalRestoreStatus {
    pub partition_results: Vec<IncrementalFetchResult>,
    /// Number of partitions skipped due to key computation errors.
    pub skipped_partitions: usize,
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
/// Cache keys include `(package, task, partition_index, output_globs,
/// hash(partition_inputs))` using length-prefixed encoding to prevent
/// separator collisions. Branch is intentionally excluded: incremental
/// artifacts are tool-managed caches that the tool validates against current
/// source files.
///
/// Keys are computed once and reused across fetch and upload to avoid
/// redundant filesystem hashing. If key computation fails for a partition
/// (invalid globs, unreadable files), that partition is skipped entirely
/// rather than falling back to a less-specific key.
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
    remote_only: bool,
    partition_keys: tokio::sync::OnceCell<Vec<Option<String>>>,
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
            partition_keys: tokio::sync::OnceCell::new(),
        }
    }

    /// Compute and cache all partition keys. Key computation runs
    /// concurrently since partitions are independent. The result is memoized
    /// so both fetch and upload reuse the same keys without redundant I/O.
    async fn get_partition_keys(&self) -> &[Option<String>] {
        self.partition_keys
            .get_or_init(|| async {
                let futures: Vec<_> = self
                    .partitions
                    .iter()
                    .enumerate()
                    .map(|(idx, partition)| {
                        let package_dir = self.package_dir.clone();
                        let pkg = self.package_name.clone();
                        let task = self.task_name.clone();
                        let inputs = partition.inputs.clone();
                        let outputs = partition.outputs.clone();
                        async move {
                            match tokio::task::spawn_blocking(move || {
                                compute_partition_key(
                                    &package_dir,
                                    &pkg,
                                    &task,
                                    idx,
                                    &inputs,
                                    &outputs,
                                )
                            })
                            .await
                            {
                                Ok(key) => key,
                                Err(e) => {
                                    warn!(
                                        "incremental: spawn_blocking panicked computing cache key \
                                         for partition {idx}: {e}"
                                    );
                                    None
                                }
                            }
                        }
                    })
                    .collect();
                join_all(futures).await
            })
            .await
    }

    /// Fetch incremental artifacts for all partitions sequentially.
    /// Sequential ordering ensures deterministic overlap resolution:
    /// later partitions overwrite earlier ones (last-write-wins by array
    /// position), matching the SPEC.
    pub async fn fetch_all(&self) -> IncrementalRestoreStatus {
        let keys = self.get_partition_keys().await;
        let mut results = Vec::with_capacity(self.partitions.len());
        let mut skipped = 0;

        for (idx, partition) in self.partitions.iter().enumerate() {
            let key = keys.get(idx).and_then(|k| k.as_deref());
            if key.is_none() {
                skipped += 1;
            }
            let result = self.fetch_partition(idx, partition, key).await;
            results.push(result);
        }

        IncrementalRestoreStatus {
            partition_results: results,
            skipped_partitions: skipped,
        }
    }

    /// Upload incremental artifacts for all partitions concurrently after
    /// successful task execution. Partitions with failed key computation
    /// are skipped. Returns the number of upload failures.
    pub async fn upload_all(&self) -> usize {
        let keys = self.get_partition_keys().await;
        let futures: Vec<_> = self
            .partitions
            .iter()
            .enumerate()
            .filter_map(|(idx, partition)| {
                let key = keys.get(idx)?.as_ref()?.clone();
                Some(async move { (idx, self.upload_partition(idx, partition, &key).await) })
            })
            .collect();
        let results = join_all(futures).await;

        let mut failures = 0;
        for (idx, result) in results {
            if let Err(e) = result {
                failures += 1;
                warn!(
                    "incremental upload failed for {}#{} partition {}: {}",
                    self.package_name, self.task_name, idx, e
                );
            }
        }
        failures
    }

    async fn fetch_partition(
        &self,
        idx: usize,
        partition: &IncrementalPartition,
        key: Option<&str>,
    ) -> IncrementalFetchResult {
        let Some(key) = key else {
            debug!(
                "incremental partition {} for {}#{}: skipped (key computation failed)",
                idx, self.package_name, self.task_name
            );
            return IncrementalFetchResult::NoState;
        };

        if !self.remote_only {
            let package_dir = self.package_dir.clone();
            let outputs = partition.outputs.clone();
            let has_local = match tokio::task::spawn_blocking(move || {
                local_files_exist(&package_dir, &outputs)
            })
            .await
            {
                Ok(val) => val,
                Err(e) => {
                    warn!(
                        "incremental: spawn_blocking panicked checking local files for partition \
                         {idx}: {e}"
                    );
                    false
                }
            };

            if has_local {
                debug!(
                    "incremental partition {} for {}#{}: local files exist, skipping remote",
                    idx, self.package_name, self.task_name
                );
                return IncrementalFetchResult::LocalFilesExist;
            }
        }

        match self.cache.fetch(&self.repo_root, key).await {
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
        key: &str,
    ) -> Result<(), crate::Error> {
        let repo_root = self.repo_root.clone();
        let package_dir = self.package_dir.clone();
        let outputs = partition.outputs.clone();
        let files = tokio::task::spawn_blocking(move || {
            collect_partition_files(&repo_root, &package_dir, &outputs)
        })
        .await
        .map_err(|e| crate::Error::SpawnBlocking(format!("collect_partition_files: {e}")))??;

        if files.is_empty() {
            debug!(
                "incremental upload: no files for partition {} of {}#{}",
                idx, self.package_name, self.task_name
            );
            return Ok(());
        }

        self.cache
            .put(self.repo_root.clone(), key.to_owned(), files, 0)
            .await
            .map_err(crate::Error::Cache)?;

        debug!(
            "incremental upload: partition {} of {}#{} uploaded",
            idx, self.package_name, self.task_name
        );

        Ok(())
    }
}

/// Compute a deterministic cache key for a partition.
///
/// Returns `None` if the key cannot be computed (invalid globs, unreadable
/// input files). The caller should skip the partition rather than use a
/// fallback key.
///
/// Uses length-prefixed encoding to prevent separator collisions (e.g.,
/// package "a:b" task "c" vs package "a" task "b:c"). Includes output glob
/// patterns so config changes invalidate the cache.
fn compute_partition_key(
    package_dir: &AbsoluteSystemPathBuf,
    package_name: &str,
    task_name: &str,
    idx: usize,
    inputs: &[String],
    outputs: &TaskOutputs,
) -> Option<String> {
    let input_hash = match compute_input_hash(package_dir, inputs, package_name, task_name) {
        Ok(hash) => hash,
        Err(msg) => {
            warn!("{}", msg);
            return None;
        }
    };

    let mut hasher = Sha256::new();
    hasher.update(b"incremental:v1:");

    // Length-prefixed encoding prevents separator collisions
    hasher.update((package_name.len() as u32).to_le_bytes());
    hasher.update(package_name.as_bytes());
    hasher.update((task_name.len() as u32).to_le_bytes());
    hasher.update(task_name.as_bytes());
    hasher.update((idx as u32).to_le_bytes());

    // Include output globs so config changes invalidate the cache
    let mut sorted_inclusions = outputs.inclusions.clone();
    sorted_inclusions.sort();
    hasher.update((sorted_inclusions.len() as u32).to_le_bytes());
    for glob in &sorted_inclusions {
        hasher.update((glob.len() as u32).to_le_bytes());
        hasher.update(glob.as_bytes());
    }
    let mut sorted_exclusions = outputs.exclusions.clone();
    sorted_exclusions.sort();
    hasher.update((sorted_exclusions.len() as u32).to_le_bytes());
    for glob in &sorted_exclusions {
        hasher.update((glob.len() as u32).to_le_bytes());
        hasher.update(glob.as_bytes());
    }

    if !input_hash.is_empty() {
        hasher.update(input_hash.as_bytes());
    }

    let hash = hex::encode(hasher.finalize());
    Some(format!("incremental-{hash}"))
}

/// Compute a hash of the partition's input files.
///
/// Returns:
/// - `Ok("")` when no inputs are configured (partition has no input key
///   component)
/// - `Ok(hash)` when inputs are successfully hashed
/// - `Err(msg)` when any error occurs (invalid glob, IO failure) — the caller
///   should skip this partition entirely rather than falling back to a
///   less-specific key
fn compute_input_hash(
    package_dir: &AbsoluteSystemPathBuf,
    inputs: &[String],
    package_name: &str,
    task_name: &str,
) -> Result<String, String> {
    if inputs.is_empty() {
        return Ok(String::new());
    }

    let mut hasher = Sha256::new();

    let globs: Vec<_> = inputs
        .iter()
        .filter(|g| !g.starts_with('!'))
        .cloned()
        .collect();
    let exclusions: Vec<_> = inputs
        .iter()
        .filter_map(|g| g.strip_prefix('!').map(String::from))
        .collect();

    let inclusion_globs = globs
        .iter()
        .map(|g| g.parse::<globwalk::ValidatedGlob>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| {
            format!(
                "incremental: invalid input inclusion glob for {}#{}: {e}",
                package_name, task_name
            )
        })?;

    let exclusion_globs = exclusions
        .iter()
        .map(|g| g.parse::<globwalk::ValidatedGlob>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| {
            format!(
                "incremental: invalid input exclusion glob for {}#{}: {e}",
                package_name, task_name
            )
        })?;

    let files = globwalk::globwalk(
        package_dir,
        &inclusion_globs,
        &exclusion_globs,
        globwalk::WalkType::Files,
    )
    .map_err(|e| {
        format!(
            "incremental: globwalk failed for input files of {}#{}: {e}",
            package_name, task_name
        )
    })?;

    let mut sorted_files: Vec<_> = files.into_iter().collect();
    sorted_files.sort();

    for file in &sorted_files {
        let relative = AnchoredSystemPathBuf::relative_path_between(package_dir, file);
        let path_bytes = relative.as_str().as_bytes();
        hasher.update((path_bytes.len() as u64).to_le_bytes());
        hasher.update(path_bytes);

        let metadata = std::fs::metadata(file.as_std_path()).map_err(|e| {
            format!(
                "incremental: failed to stat input file {}: {e}",
                relative.as_str()
            )
        })?;
        hasher.update(metadata.len().to_le_bytes());

        let f = std::fs::File::open(file.as_std_path()).map_err(|e| {
            format!(
                "incremental: failed to open input file {}: {e}",
                relative.as_str()
            )
        })?;
        let mut reader = std::io::BufReader::new(f);
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => hasher.update(&buf[..n]),
                Err(e) => {
                    return Err(format!(
                        "incremental: failed to read input file {}: {e}",
                        relative.as_str()
                    ));
                }
            }
        }
    }

    Ok(hex::encode(hasher.finalize()))
}

/// Check if ANY files matching the partition's output globs exist on disk.
// TODO: globwalk collects all matches into a HashSet. A `globwalk_any` variant
// that short-circuits on first match would make this near-free for packages
// with large incremental caches (e.g. target/debug/incremental/**).
fn local_files_exist(package_dir: &AbsoluteSystemPathBuf, outputs: &TaskOutputs) -> bool {
    let Ok(inclusions) = outputs.validated_inclusions() else {
        return false;
    };
    let exclusions = outputs.validated_exclusions().unwrap_or_default();

    match globwalk::globwalk(
        package_dir,
        &inclusions,
        &exclusions,
        globwalk::WalkType::Files,
    ) {
        Ok(files) => files.into_iter().next().is_some(),
        Err(_) => false,
    }
}

/// Collect files matching the partition's output globs, returning
/// repo-relative paths suitable for cache archiving.
fn collect_partition_files(
    repo_root: &AbsoluteSystemPathBuf,
    package_dir: &AbsoluteSystemPathBuf,
    outputs: &TaskOutputs,
) -> Result<Vec<AnchoredSystemPathBuf>, crate::Error> {
    let repo_relative_globs = repo_relative_outputs(repo_root, package_dir, outputs);
    let inclusions = repo_relative_globs.validated_inclusions()?;
    let exclusions = repo_relative_globs.validated_exclusions()?;

    let files = globwalk::globwalk(repo_root, &inclusions, &exclusions, globwalk::WalkType::All)?;

    let mut relative_paths: Vec<_> = files
        .into_iter()
        .map(|path| AnchoredSystemPathBuf::relative_path_between(repo_root, &path))
        .collect();
    relative_paths.sort();
    Ok(relative_paths)
}

/// Convert package-relative output globs to repo-relative globs.
fn repo_relative_outputs(
    repo_root: &AbsoluteSystemPathBuf,
    package_dir: &AbsoluteSystemPathBuf,
    outputs: &TaskOutputs,
) -> TaskOutputs {
    let pkg_relative = AnchoredSystemPathBuf::relative_path_between(repo_root, package_dir);
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

#[cfg(test)]
mod tests {
    use turborepo_types::TaskOutputs;

    use super::*;

    #[test]
    fn compute_input_hash_empty_inputs_returns_empty() {
        let dir = AbsoluteSystemPathBuf::try_from(std::env::temp_dir().to_str().unwrap()).unwrap();
        let result = compute_input_hash(&dir, &[], "pkg", "task");
        assert_eq!(result.unwrap(), "");
    }

    #[test]
    fn compute_input_hash_deterministic() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = AbsoluteSystemPathBuf::try_from(tmp.path().to_str().unwrap()).unwrap();
        std::fs::write(tmp.path().join("input.txt"), b"hello").unwrap();

        let h1 = compute_input_hash(&dir, &["input.txt".into()], "pkg", "task").unwrap();
        let h2 = compute_input_hash(&dir, &["input.txt".into()], "pkg", "task").unwrap();
        assert_eq!(h1, h2);
        assert!(!h1.is_empty());
    }

    #[test]
    fn compute_input_hash_different_contents_different_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = AbsoluteSystemPathBuf::try_from(tmp.path().to_str().unwrap()).unwrap();

        std::fs::write(tmp.path().join("input.txt"), b"hello").unwrap();
        let h1 = compute_input_hash(&dir, &["input.txt".into()], "pkg", "task").unwrap();

        std::fs::write(tmp.path().join("input.txt"), b"world").unwrap();
        let h2 = compute_input_hash(&dir, &["input.txt".into()], "pkg", "task").unwrap();

        assert_ne!(h1, h2);
    }

    #[test]
    fn compute_input_hash_invalid_glob_returns_error() {
        let dir = AbsoluteSystemPathBuf::try_from(std::env::temp_dir().to_str().unwrap()).unwrap();
        let result = compute_input_hash(&dir, &["[invalid".into()], "pkg", "task");
        assert!(result.is_err());
    }

    #[test]
    fn compute_input_hash_no_matching_files_returns_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = AbsoluteSystemPathBuf::try_from(tmp.path().to_str().unwrap()).unwrap();
        let result = compute_input_hash(&dir, &["*.nonexistent".into()], "pkg", "task").unwrap();
        // Inputs configured but no files matched: hash of nothing, still a valid hex
        // string. This is different from "no inputs configured" (empty string).
        assert!(!result.is_empty());
    }

    #[test]
    fn compute_partition_key_includes_output_globs() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = AbsoluteSystemPathBuf::try_from(tmp.path().to_str().unwrap()).unwrap();

        let outputs_a = TaskOutputs {
            inclusions: vec!["dist/**".into()],
            exclusions: vec![],
        };
        let outputs_b = TaskOutputs {
            inclusions: vec!["build/**".into()],
            exclusions: vec![],
        };

        let key_a = compute_partition_key(&dir, "pkg", "task", 0, &[], &outputs_a);
        let key_b = compute_partition_key(&dir, "pkg", "task", 0, &[], &outputs_b);
        assert_ne!(
            key_a, key_b,
            "different output globs must produce different keys"
        );
    }

    #[test]
    fn compute_partition_key_no_separator_collision() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = AbsoluteSystemPathBuf::try_from(tmp.path().to_str().unwrap()).unwrap();
        let outputs = TaskOutputs {
            inclusions: vec!["dist/**".into()],
            exclusions: vec![],
        };

        let key_ab_c = compute_partition_key(&dir, "a:b", "c", 0, &[], &outputs);
        let key_a_bc = compute_partition_key(&dir, "a", "b:c", 0, &[], &outputs);
        assert_ne!(
            key_ab_c, key_a_bc,
            "length-prefixed encoding must prevent separator collisions"
        );
    }

    #[test]
    fn compute_partition_key_returns_none_on_input_error() {
        let dir = AbsoluteSystemPathBuf::try_from(std::env::temp_dir().to_str().unwrap()).unwrap();
        let outputs = TaskOutputs {
            inclusions: vec!["dist/**".into()],
            exclusions: vec![],
        };
        let result = compute_partition_key(&dir, "pkg", "task", 0, &["[invalid".into()], &outputs);
        assert!(
            result.is_none(),
            "invalid input glob should produce None, not a fallback key"
        );
    }

    #[test]
    fn compute_partition_key_format() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = AbsoluteSystemPathBuf::try_from(tmp.path().to_str().unwrap()).unwrap();
        let outputs = TaskOutputs {
            inclusions: vec!["dist/**".into()],
            exclusions: vec![],
        };
        let key = compute_partition_key(&dir, "pkg", "task", 0, &[], &outputs).unwrap();
        assert!(
            key.starts_with("incremental-"),
            "key must start with 'incremental-'"
        );
        // 'incremental-' (12 chars) + 64 hex chars (sha256)
        assert_eq!(key.len(), 12 + 64);
    }

    #[test]
    fn restore_status_any_restored_empty() {
        let status = IncrementalRestoreStatus::default();
        assert!(!status.any_restored());
    }

    #[test]
    fn restore_status_any_restored_all_no_state() {
        let status = IncrementalRestoreStatus {
            partition_results: vec![IncrementalFetchResult::NoState],
            skipped_partitions: 0,
        };
        assert!(!status.any_restored());
    }

    #[test]
    fn restore_status_any_restored_with_local() {
        let status = IncrementalRestoreStatus {
            partition_results: vec![
                IncrementalFetchResult::NoState,
                IncrementalFetchResult::LocalFilesExist,
            ],
            skipped_partitions: 0,
        };
        assert!(status.any_restored());
    }

    #[test]
    fn restore_status_any_restored_with_remote() {
        let status = IncrementalRestoreStatus {
            partition_results: vec![IncrementalFetchResult::RestoredFromRemote],
            skipped_partitions: 0,
        };
        assert!(status.any_restored());
    }

    #[test]
    fn local_files_exist_empty_outputs() {
        let dir = AbsoluteSystemPathBuf::try_from(std::env::temp_dir().to_str().unwrap()).unwrap();
        let outputs = TaskOutputs {
            inclusions: vec![],
            exclusions: vec![],
        };
        assert!(!local_files_exist(&dir, &outputs));
    }

    #[test]
    fn local_files_exist_with_matching_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = AbsoluteSystemPathBuf::try_from(tmp.path().to_str().unwrap()).unwrap();
        std::fs::write(tmp.path().join("test.buildinfo"), b"data").unwrap();

        let outputs = TaskOutputs {
            inclusions: vec!["*.buildinfo".into()],
            exclusions: vec![],
        };
        assert!(local_files_exist(&dir, &outputs));
    }

    #[test]
    fn local_files_exist_no_matching_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = AbsoluteSystemPathBuf::try_from(tmp.path().to_str().unwrap()).unwrap();
        std::fs::write(tmp.path().join("unrelated.txt"), b"data").unwrap();

        let outputs = TaskOutputs {
            inclusions: vec!["*.buildinfo".into()],
            exclusions: vec![],
        };
        assert!(!local_files_exist(&dir, &outputs));
    }

    #[test]
    fn repo_relative_outputs_with_prefix() {
        let repo =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let pkg = AbsoluteSystemPathBuf::new(if cfg!(windows) {
            r"C:\repo\packages\foo"
        } else {
            "/repo/packages/foo"
        })
        .unwrap();
        let outputs = TaskOutputs {
            inclusions: vec!["dist/**".into()],
            exclusions: vec!["dist/tmp".into()],
        };

        let result = repo_relative_outputs(&repo, &pkg, &outputs);
        let expected_inclusion = if cfg!(windows) {
            r"packages\foo/dist/**"
        } else {
            "packages/foo/dist/**"
        };
        let expected_exclusion = if cfg!(windows) {
            r"packages\foo/dist/tmp"
        } else {
            "packages/foo/dist/tmp"
        };
        assert_eq!(result.inclusions, vec![expected_inclusion]);
        assert_eq!(result.exclusions, vec![expected_exclusion]);
    }

    #[test]
    fn repo_relative_outputs_root_package() {
        let repo =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let pkg =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let outputs = TaskOutputs {
            inclusions: vec!["dist/**".into()],
            exclusions: vec![],
        };

        let result = repo_relative_outputs(&repo, &pkg, &outputs);
        // When package IS the repo root, relative_path_between returns "."
        // so globs get prefixed with "./" — this is valid for globwalk.
        assert_eq!(result.inclusions.len(), 1);
        assert!(
            result.inclusions[0].ends_with("dist/**"),
            "expected glob ending with dist/**, got: {}",
            result.inclusions[0]
        );
    }

    #[test]
    fn collect_partition_files_returns_sorted() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = AbsoluteSystemPathBuf::try_from(tmp.path().to_str().unwrap()).unwrap();
        let pkg_path = tmp.path().join("pkg");
        std::fs::create_dir_all(&pkg_path).unwrap();
        std::fs::write(pkg_path.join("b.txt"), b"b").unwrap();
        std::fs::write(pkg_path.join("a.txt"), b"a").unwrap();

        let pkg_dir = AbsoluteSystemPathBuf::try_from(pkg_path.to_str().unwrap()).unwrap();
        let outputs = TaskOutputs {
            inclusions: vec!["*.txt".into()],
            exclusions: vec![],
        };

        let files = collect_partition_files(&repo, &pkg_dir, &outputs).unwrap();
        assert_eq!(files.len(), 2);
        let names: Vec<_> = files.iter().map(|f| f.to_string()).collect();
        assert!(names[0] < names[1], "files should be sorted: {:?}", names);
    }

    #[test]
    fn collect_partition_files_paths_resolve_to_package_dir() {
        // Verifies the critical invariant: repo-relative paths from
        // collect_partition_files, when joined with repo_root, point to
        // files inside the package directory. This is what makes cache
        // extraction (anchored at repo_root) restore files to the right place.
        let tmp = tempfile::tempdir().unwrap();
        let repo = AbsoluteSystemPathBuf::try_from(tmp.path().to_str().unwrap()).unwrap();

        let pkg_path = tmp.path().join("packages").join("mylib");
        std::fs::create_dir_all(&pkg_path).unwrap();
        std::fs::write(pkg_path.join("tsconfig.tsbuildinfo"), b"info").unwrap();
        std::fs::create_dir_all(pkg_path.join("dist")).unwrap();
        std::fs::write(pkg_path.join("dist").join("index.js"), b"code").unwrap();

        let pkg_dir = AbsoluteSystemPathBuf::try_from(pkg_path.to_str().unwrap()).unwrap();
        let outputs = TaskOutputs {
            inclusions: vec!["tsconfig.tsbuildinfo".into(), "dist/**".into()],
            exclusions: vec![],
        };

        let files = collect_partition_files(&repo, &pkg_dir, &outputs).unwrap();
        assert!(!files.is_empty());

        for anchored_path in &files {
            let full_path = repo.resolve(anchored_path);
            assert!(
                full_path.as_str().starts_with(pkg_dir.as_str()),
                "repo-relative path {:?} should resolve inside package dir {:?}, got {:?}",
                anchored_path,
                pkg_dir,
                full_path
            );
            assert!(
                full_path.exists(),
                "resolved path {:?} should exist on disk",
                full_path
            );
        }
    }

    #[test]
    fn compute_partition_key_stable_across_calls() {
        // Verifies keys are deterministic — same inputs produce same key.
        // This is the property that makes fetch and upload use the same key
        // (via OnceCell memoization).
        let tmp = tempfile::tempdir().unwrap();
        let dir = AbsoluteSystemPathBuf::try_from(tmp.path().to_str().unwrap()).unwrap();
        std::fs::write(tmp.path().join("input.txt"), b"data").unwrap();

        let outputs = TaskOutputs {
            inclusions: vec!["dist/**".into()],
            exclusions: vec![],
        };

        let k1 = compute_partition_key(&dir, "pkg", "build", 0, &["input.txt".into()], &outputs);
        let k2 = compute_partition_key(&dir, "pkg", "build", 0, &["input.txt".into()], &outputs);
        assert_eq!(k1, k2);
    }

    #[test]
    fn compute_input_hash_path_content_boundary_no_collision() {
        // File "ab" with content "cd" must hash differently from file "abc"
        // with content "d". Length-prefixed encoding prevents this collision.
        let tmp = tempfile::tempdir().unwrap();
        let dir = AbsoluteSystemPathBuf::try_from(tmp.path().to_str().unwrap()).unwrap();

        std::fs::write(tmp.path().join("ab"), b"cd").unwrap();
        let h1 = compute_input_hash(&dir, &["ab".into()], "pkg", "task").unwrap();

        std::fs::remove_file(tmp.path().join("ab")).unwrap();
        std::fs::write(tmp.path().join("abc"), b"d").unwrap();
        let h2 = compute_input_hash(&dir, &["abc".into()], "pkg", "task").unwrap();

        assert_ne!(
            h1, h2,
            "different path/content boundaries must produce different hashes"
        );
    }
}
