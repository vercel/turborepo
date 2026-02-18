//! turborepo-task-hash: Task hashing utilities for Turborepo cache invalidation
//!
//! This crate provides the core task hashing logic for Turborepo. It computes
//! hashes for tasks based on their inputs (files, environment variables,
//! dependencies) to determine cache invalidation.

pub mod global_hash;

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

pub use global_hash::*;
use rayon::prelude::*;
use serde::Serialize;
use thiserror::Error;
use tracing::debug;
use turbopath::{
    AbsoluteSystemPath, AnchoredSystemPath, AnchoredSystemPathBuf, RelativeUnixPathBuf,
};
use turborepo_cache::CacheHitMetadata;
// Re-export turborepo_engine::TaskNode for convenience
pub use turborepo_engine::TaskNode;
use turborepo_env::{BySource, DetailedMap, EnvironmentVariableMap};
use turborepo_frameworks::{Slug as FrameworkSlug, infer_framework};
use turborepo_hash::{FileHashes, LockFilePackagesRef, TaskHashable, TurboHash};
use turborepo_repository::package_graph::{PackageInfo, PackageName};
use turborepo_scm::{RepoGitIndex, SCM};
use turborepo_task_id::TaskId;
use turborepo_telemetry::events::{generic::GenericEventBuilder, task::PackageTaskEventBuilder};
use turborepo_types::{
    EnvMode, HashTrackerCacheHitMetadata, HashTrackerDetailedMap, HashTrackerInfo, RunOptsHashInfo,
    TaskDefinitionHashInfo, TaskInputs,
};

/// Trait for daemon client operations needed for file hashing.
pub trait DaemonFileHasher: Clone + Send {
    /// Get file hashes for a package path with the given inputs
    fn get_file_hashes(
        &mut self,
        package_path: &AnchoredSystemPath,
        inputs: &TaskInputs,
    ) -> impl std::future::Future<
        Output = Result<HashMap<String, String>, turborepo_daemon::DaemonError>,
    > + Send;
}

// Implement DaemonFileHasher for the actual daemon client
impl DaemonFileHasher for turborepo_daemon::DaemonClient<turborepo_daemon::DaemonConnector> {
    async fn get_file_hashes(
        &mut self,
        package_path: &AnchoredSystemPath,
        inputs: &TaskInputs,
    ) -> Result<HashMap<String, String>, turborepo_daemon::DaemonError> {
        let response =
            turborepo_daemon::DaemonClient::get_file_hashes(self, package_path, inputs).await?;
        Ok(response.file_hashes)
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Missing pipeline entry: {0}")]
    MissingPipelineEntry(TaskId<'static>),
    #[error("Missing package.json for {0}.")]
    MissingPackageJson(String),
    #[error("Cannot find package-file hash for {0}.")]
    MissingPackageFileHash(String),
    #[error("Missing hash for dependent task {0}.")]
    MissingDependencyTaskHash(String),
    #[error("Cannot acquire lock for task hash tracker.")]
    Mutex,
    #[error("Missing environment variables for {0}.")]
    MissingEnvVars(TaskId<'static>),
    #[error(
        "Error processing environment patterns for task {task_id} (including global exclusions): \
         {err}"
    )]
    EnvPattern {
        task_id: TaskId<'static>,
        #[source]
        err: turborepo_env::Error,
    },
    #[error(transparent)]
    Scm(#[from] turborepo_scm::Error),
    #[error(transparent)]
    Env(#[from] turborepo_env::Error),
    #[error(transparent)]
    Regex(#[from] regex::Error),
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
}

#[derive(Debug, Default)]
pub struct PackageInputsHashes {
    hashes: HashMap<TaskId<'static>, String>,
    expanded_hashes: HashMap<TaskId<'static>, FileHashes>,
}

impl PackageInputsHashes {
    #[tracing::instrument(skip(
        all_tasks,
        workspaces,
        task_definitions,
        repo_root,
        scm,
        _telemetry,
        daemon,
        pre_built_index
    ))]
    pub fn calculate_file_hashes<'a, T, D>(
        scm: &SCM,
        all_tasks: impl Iterator<Item = &'a TaskNode>,
        workspaces: HashMap<&PackageName, &PackageInfo>,
        task_definitions: &HashMap<TaskId<'static>, T>,
        repo_root: &AbsoluteSystemPath,
        _telemetry: &GenericEventBuilder,
        daemon: &Option<D>,
        pre_built_index: Option<&RepoGitIndex>,
    ) -> Result<PackageInputsHashes, Error>
    where
        T: TaskDefinitionHashInfo + Sync,
        D: DaemonFileHasher + Send + Sync,
    {
        tracing::trace!(scm_manual=%scm.is_manual(), "scm running in {} mode", if scm.is_manual() { "manual" } else { "git" });

        // Use the pre-built index if provided, otherwise build one on the spot.
        let owned_index;
        let repo_index = match pre_built_index {
            Some(idx) => Some(idx),
            None => {
                owned_index = scm.build_repo_index(workspaces.len());
                owned_index.as_ref()
            }
        };

        // Phase 1: Collect task metadata and group by (package_path, inputs) for dedup.
        // Multiple tasks in the same package with identical inputs produce the same
        // file hashes — no need to globwalk and hash the same files repeatedly.
        struct TaskInfo<'b> {
            task_id: TaskId<'static>,
            package_path: &'b AnchoredSystemPath,
            inputs: &'b TaskInputs,
        }

        let mut task_infos = Vec::new();
        for task in all_tasks {
            let TaskNode::Task(task_id) = task else {
                continue;
            };
            let task_definition = task_definitions
                .get(task_id)
                .ok_or_else(|| Error::MissingPipelineEntry(task_id.clone()))?;
            let workspace_name = task_id.to_workspace_name();
            let pkg = workspaces
                .get(&workspace_name)
                .ok_or_else(|| Error::MissingPackageJson(workspace_name.to_string()))?;
            let package_path = pkg
                .package_json_path
                .parent()
                .unwrap_or_else(|| AnchoredSystemPath::new("").unwrap());
            let inputs = task_definition.inputs();
            task_infos.push(TaskInfo {
                task_id: task_id.clone(),
                package_path,
                inputs,
            });
        }

        // Build dedup key: (package_path_str, globs, default)
        type HashKey = (AnchoredSystemPathBuf, Vec<String>, bool);
        let mut unique_keys: Vec<HashKey> = Vec::new();
        let mut key_indices: HashMap<HashKey, usize> = HashMap::new();
        let mut task_key_map: Vec<usize> = Vec::with_capacity(task_infos.len());

        for info in &task_infos {
            let key: HashKey = (
                info.package_path.to_owned(),
                info.inputs.globs.clone(),
                info.inputs.default,
            );
            let idx = match key_indices.get(&key) {
                Some(&idx) => idx,
                None => {
                    let idx = unique_keys.len();
                    key_indices.insert(key.clone(), idx);
                    unique_keys.push(key);
                    idx
                }
            };
            task_key_map.push(idx);
        }

        debug!(
            total_tasks = task_infos.len(),
            unique_hash_keys = unique_keys.len(),
            "file hash deduplication"
        );

        // Phase 2: Compute file hashes in parallel across unique keys.
        // EMFILE (too many open files) errors are handled via retry-with-backoff
        // in the globwalk and hash_objects layers, so we can safely parallelize
        // all keys on rayon without worrying about fd exhaustion.
        let file_hash_results: Vec<Result<Arc<FileHashes>, Error>> = unique_keys
            .into_par_iter()
            .map(|(package_path, globs, default)| {
                if cfg!(feature = "daemon-file-hashing") {
                    let handle = tokio::runtime::Handle::current();
                    let mut daemon = daemon.clone();
                    let inputs = TaskInputs {
                        globs: globs.clone(),
                        default,
                    };
                    let result = daemon.as_mut().and_then(|daemon| {
                        let handle = handle.clone();
                        handle
                            .block_on(async {
                                tokio::time::timeout(
                                    std::time::Duration::from_millis(100),
                                    daemon.get_file_hashes(&package_path, &inputs),
                                )
                                .await
                            })
                            .ok()
                    });
                    if let Some(Ok(file_hashes)) = result {
                        let hashes = file_hashes
                            .into_iter()
                            .map(|(path, hash)| {
                                (
                                    turbopath::RelativeUnixPathBuf::new(path)
                                        .expect("daemon returns relative unix paths"),
                                    hash,
                                )
                            })
                            .collect();
                        return Ok(Arc::new(FileHashes(hashes)));
                    }
                }

                scm.get_package_file_hashes(
                    repo_root,
                    &package_path,
                    &globs,
                    default,
                    None,
                    repo_index,
                )
                .map(|h| Arc::new(FileHashes(h)))
                .map_err(Error::from)
            })
            .collect();

        let file_hash_results: Vec<Arc<FileHashes>> = file_hash_results
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;

        // Phase 3: Distribute shared results to individual tasks.
        let mut hashes = HashMap::with_capacity(task_infos.len());
        let mut expanded_hashes = HashMap::with_capacity(task_infos.len());

        for (i, info) in task_infos.into_iter().enumerate() {
            let key_idx = task_key_map[i];
            let file_hashes = &file_hash_results[key_idx];

            let hash = file_hashes.as_ref().hash();

            hashes.insert(info.task_id.clone(), hash);
            // Clone the Arc'd FileHashes for tasks sharing the same inputs.
            // This is a reference count bump, not a deep clone.
            expanded_hashes.insert(info.task_id, FileHashes(file_hashes.0.clone()));
        }

        Ok(PackageInputsHashes {
            hashes,
            expanded_hashes,
        })
    }
}

#[derive(Default, Debug, Clone)]
pub struct TaskHashTracker {
    state: Arc<Mutex<TaskHashTrackerState>>,
}

#[derive(Default, Debug, Serialize)]
pub struct TaskHashTrackerState {
    #[serde(skip)]
    package_task_env_vars: HashMap<TaskId<'static>, DetailedMap>,
    package_task_hashes: HashMap<TaskId<'static>, String>,
    #[serde(skip)]
    package_task_framework: HashMap<TaskId<'static>, FrameworkSlug>,
    #[serde(skip)]
    package_task_outputs: HashMap<TaskId<'static>, Vec<AnchoredSystemPathBuf>>,
    #[serde(skip)]
    package_task_cache: HashMap<TaskId<'static>, CacheHitMetadata>,
    #[serde(skip)]
    package_task_inputs_expanded_hashes: HashMap<TaskId<'static>, FileHashes>,
}

/// Caches package-inputs hashes, and package-task hashes.
pub struct TaskHasher<'a, R> {
    hashes: HashMap<TaskId<'static>, String>,
    run_opts: &'a R,
    env_at_execution_start: &'a EnvironmentVariableMap,
    global_env: EnvironmentVariableMap,
    global_env_patterns: &'a [String],
    global_hash: &'a str,
    task_hash_tracker: TaskHashTracker,
}

impl<'a, R: RunOptsHashInfo> TaskHasher<'a, R> {
    pub fn new(
        package_inputs_hashes: PackageInputsHashes,
        run_opts: &'a R,
        env_at_execution_start: &'a EnvironmentVariableMap,
        global_hash: &'a str,
        global_env: EnvironmentVariableMap,
        global_env_patterns: &'a [String],
    ) -> Self {
        let PackageInputsHashes {
            hashes,
            expanded_hashes,
        } = package_inputs_hashes;
        Self {
            hashes,
            run_opts,
            env_at_execution_start,
            global_hash,
            global_env,
            global_env_patterns,
            task_hash_tracker: TaskHashTracker::new(expanded_hashes),
        }
    }

    #[tracing::instrument(skip(self, task_definition, task_env_mode, workspace, dependency_set))]
    pub fn calculate_task_hash<T: TaskDefinitionHashInfo>(
        &self,
        task_id: &TaskId<'static>,
        task_definition: &T,
        task_env_mode: EnvMode,
        workspace: &PackageInfo,
        dependency_set: HashSet<&TaskNode>,
        telemetry: PackageTaskEventBuilder,
    ) -> Result<String, Error> {
        let do_framework_inference = self.run_opts.framework_inference();
        let is_monorepo = !self.run_opts.single_package();

        let hash_of_files = self
            .hashes
            .get(task_id)
            .ok_or_else(|| Error::MissingPackageFileHash(task_id.to_string()))?;
        // See if we can infer a framework
        let framework = do_framework_inference
            .then(|| infer_framework(workspace, is_monorepo))
            .flatten()
            .inspect(|framework| {
                debug!("auto detected framework for {}", task_id.package());
                debug!(
                    "framework: {}, env_prefix: {:?}",
                    framework.slug(),
                    framework.env(self.env_at_execution_start)
                );
                telemetry.track_framework(framework.slug().to_string());
            });
        let framework_slug = framework.map(|f| f.slug());

        let env_vars = if let Some(framework) = framework {
            let mut computed_wildcards = framework.env(self.env_at_execution_start);

            if let Some(exclude_prefix) = self
                .env_at_execution_start
                .get("TURBO_CI_VENDOR_ENV_KEY")
                .filter(|prefix| !prefix.is_empty())
            {
                let computed_exclude = format!("!{exclude_prefix}*");
                debug!(
                    "excluding environment variables matching wildcard {}",
                    computed_exclude
                );
                computed_wildcards.push(computed_exclude);
            }

            // Combine task-specific env patterns with global env exclusions
            // Global exclusions (patterns starting with !) should apply to framework
            // inference
            let combined_env_patterns: Vec<String> = task_definition
                .env()
                .iter()
                .chain(
                    self.global_env_patterns
                        .iter()
                        .filter(|p| p.starts_with('!')),
                )
                .cloned()
                .collect();

            self.env_at_execution_start
                .hashable_task_env(&computed_wildcards, &combined_env_patterns)
                .map_err(|err| Error::EnvPattern {
                    task_id: task_id.clone().into_owned(),
                    err,
                })?
        } else {
            let all_env_var_map = self
                .env_at_execution_start
                .from_wildcards(task_definition.env())?;

            DetailedMap {
                all: all_env_var_map.clone(),
                by_source: BySource {
                    explicit: all_env_var_map,
                    matching: EnvironmentVariableMap::default(),
                },
            }
        };

        let hashable_env_pairs = env_vars.all.to_hashable();
        let outputs = task_definition.hashable_outputs(task_id);
        let task_dependency_hashes = self.calculate_dependency_hashes(dependency_set)?;
        let external_deps_hash =
            is_monorepo.then(|| get_external_deps_hash(&workspace.transitive_dependencies));

        if !hashable_env_pairs.is_empty() {
            debug!(
                "task hash env vars for {}:{}\n vars: {:?}",
                task_id.package(),
                task_id.task(),
                hashable_env_pairs
            );
        }

        let package_dir = workspace.package_path().to_unix();
        let is_root_package = package_dir.is_empty();
        // We wrap in an Option to mimic Go's serialization of nullable values
        let optional_package_dir = (!is_root_package).then_some(package_dir);

        let task_hashable = TaskHashable {
            global_hash: self.global_hash,
            task_dependency_hashes,
            package_dir: optional_package_dir,
            hash_of_files,
            external_deps_hash,
            task: task_id.task(),
            outputs,

            pass_through_args: self.run_opts.pass_through_args(),
            env: task_definition.env(),
            resolved_env_vars: hashable_env_pairs,
            pass_through_env: task_definition.pass_through_env().unwrap_or_default(),
            env_mode: task_env_mode,
        };

        let task_hash = task_hashable.calculate_task_hash();

        self.task_hash_tracker.insert_hash(
            task_id.clone(),
            env_vars,
            task_hash.clone(),
            framework_slug,
        );

        Ok(task_hash)
    }

    /// Gets the hashes of a task's dependencies. Because the visitor
    /// receives the nodes in topological order, we know that all of
    /// the dependencies have been processed before the current task.
    ///
    /// # Arguments
    ///
    /// * `dependency_set`: The dependencies of the current task
    ///
    /// returns: Result<Vec<String, Global>, Error>
    fn calculate_dependency_hashes(
        &self,
        dependency_set: HashSet<&TaskNode>,
    ) -> Result<Vec<String>, Error> {
        let mut dependency_hash_set = HashSet::new();

        for dependency_task in dependency_set {
            let TaskNode::Task(dependency_task_id) = dependency_task else {
                continue;
            };

            let dependency_hash = self
                .task_hash_tracker
                .hash(dependency_task_id)
                .ok_or_else(|| Error::MissingDependencyTaskHash(dependency_task.to_string()))?;
            dependency_hash_set.insert(dependency_hash.clone());
        }

        let mut dependency_hash_list = dependency_hash_set.into_iter().collect::<Vec<_>>();
        dependency_hash_list.sort_unstable();

        Ok(dependency_hash_list)
    }

    pub fn into_task_hash_tracker_state(self) -> TaskHashTrackerState {
        let mutex = Arc::into_inner(self.task_hash_tracker.state)
            .expect("multiple references to tracker state still exist");
        mutex.into_inner().unwrap()
    }

    pub fn task_hash_tracker(&self) -> TaskHashTracker {
        self.task_hash_tracker.clone()
    }

    pub fn env<T: TaskDefinitionHashInfo>(
        &self,
        task_id: &TaskId,
        task_env_mode: EnvMode,
        task_definition: &T,
    ) -> Result<EnvironmentVariableMap, Error> {
        match task_env_mode {
            EnvMode::Strict => {
                let mut full_task_env = EnvironmentVariableMap::default();
                let builtin_pass_through = &[
                    "HOME",
                    "USER",
                    "TZ",
                    "LANG",
                    "SHELL",
                    "PWD",
                    "XDG_RUNTIME_DIR",
                    "XAUTHORITY",
                    "DBUS_SESSION_BUS_ADDRESS",
                    "CI",
                    "NODE_OPTIONS",
                    "COREPACK_HOME",
                    "LD_LIBRARY_PATH",
                    "DYLD_FALLBACK_LIBRARY_PATH",
                    "LIBPATH",
                    "LD_PRELOAD",
                    "DYLD_INSERT_LIBRARIES",
                    "COLORTERM",
                    "TERM",
                    "TERM_PROGRAM",
                    "DISPLAY",
                    "TMP",
                    "TEMP",
                    // Windows
                    "WINDIR",
                    "ProgramFiles",
                    "ProgramFiles(x86)",
                    // VSCode IDE - https://github.com/microsoft/vscode-js-debug/blob/5b0f41dbe845d693a541c1fae30cec04c878216f/src/targets/node/nodeLauncherBase.ts#L320
                    "VSCODE_*",
                    "ELECTRON_RUN_AS_NODE",
                    // Docker - https://docs.docker.com/engine/reference/commandline/cli/#environment-variables
                    "DOCKER_*",
                    "BUILDKIT_*",
                    // Docker compose - https://docs.docker.com/compose/environment-variables/envvars/
                    "COMPOSE_*",
                    // Jetbrains IDE
                    "JB_IDE_*",
                    "JB_INTERPRETER",
                    "_JETBRAINS_TEST_RUNNER_RUN_SCOPE_TYPE",
                    // Vercel specific
                    "VERCEL",
                    "VERCEL_*",
                    "NEXT_*",
                    "USE_OUTPUT_FOR_EDGE_FUNCTIONS",
                    "NOW_BUILDER",
                    "VC_MICROFRONTENDS_CONFIG_FILE_NAME",
                    // GitHub Actions - https://docs.github.com/en/actions/reference/workflows-and-actions/variables
                    "GITHUB_*",
                    "RUNNER_*",
                    // Command Prompt casing of env variables
                    "APPDATA",
                    "PATH",
                    "PROGRAMDATA",
                    "SYSTEMROOT",
                    "SYSTEMDRIVE",
                    "USERPROFILE",
                    "HOMEDRIVE",
                    "HOMEPATH",
                    "PNPM_HOME",
                    "NPM_CONFIG_STORE_DIR",
                ];
                let pass_through_env_vars = self.env_at_execution_start.pass_through_env(
                    builtin_pass_through,
                    &self.global_env,
                    task_definition.pass_through_env().unwrap_or_default(),
                )?;

                let tracker_env = self
                    .task_hash_tracker
                    .env_vars(task_id)
                    .ok_or_else(|| Error::MissingEnvVars(task_id.clone().into_owned()))?;

                full_task_env.union(&pass_through_env_vars);
                full_task_env.union(&tracker_env.all);

                Ok(full_task_env)
            }
            EnvMode::Loose => Ok(self.env_at_execution_start.clone()),
        }
    }
}

pub fn get_external_deps_hash(
    transitive_dependencies: &Option<HashSet<turborepo_lockfiles::Package>>,
) -> String {
    let Some(transitive_dependencies) = transitive_dependencies else {
        return "".into();
    };

    // Collect references instead of cloning each Package (which has two Strings).
    let mut transitive_deps: Vec<&turborepo_lockfiles::Package> =
        transitive_dependencies.iter().collect();

    transitive_deps.sort_unstable_by(|a, b| match a.key.cmp(&b.key) {
        std::cmp::Ordering::Equal => a.version.cmp(&b.version),
        other => other,
    });

    LockFilePackagesRef(transitive_deps).hash()
}

pub fn get_internal_deps_hash(
    scm: &SCM,
    root: &AbsoluteSystemPath,
    package_dirs: Vec<&AnchoredSystemPath>,
    pre_built_index: Option<&RepoGitIndex>,
) -> Result<String, Error> {
    if package_dirs.is_empty() {
        return Ok("".into());
    }

    let owned_index;
    let repo_index = match pre_built_index {
        Some(idx) => Some(idx),
        None => {
            owned_index = scm.build_repo_index(package_dirs.len());
            owned_index.as_ref()
        }
    };

    let file_hashes = package_dirs
        .into_par_iter()
        .map(|package_dir| {
            scm.get_package_file_hashes::<&str>(root, package_dir, &[], false, None, repo_index)
        })
        .reduce(
            || Ok(HashMap::new()),
            |acc, hashes| {
                let mut acc = acc?;
                let hashes = hashes?;
                acc.extend(hashes.into_iter());
                Ok(acc)
            },
        )?;

    Ok(FileHashes(file_hashes).hash())
}

impl TaskHashTracker {
    pub fn new(input_expanded_hashes: HashMap<TaskId<'static>, FileHashes>) -> Self {
        Self {
            state: Arc::new(Mutex::new(TaskHashTrackerState {
                package_task_inputs_expanded_hashes: input_expanded_hashes,
                ..Default::default()
            })),
        }
    }

    pub fn hash(&self, task_id: &TaskId) -> Option<String> {
        let state = self.state.lock().expect("hash tracker mutex poisoned");
        state.package_task_hashes.get(task_id).cloned()
    }

    fn insert_hash(
        &self,
        task_id: TaskId<'static>,
        env_vars: DetailedMap,
        hash: String,
        framework_slug: Option<FrameworkSlug>,
    ) {
        let mut state = self.state.lock().expect("hash tracker mutex poisoned");
        state
            .package_task_env_vars
            .insert(task_id.clone(), env_vars);
        if let Some(framework) = framework_slug {
            state
                .package_task_framework
                .insert(task_id.clone(), framework);
        }
        state.package_task_hashes.insert(task_id, hash);
    }

    pub fn env_vars(&self, task_id: &TaskId) -> Option<DetailedMap> {
        let state = self.state.lock().expect("hash tracker mutex poisoned");
        state.package_task_env_vars.get(task_id).cloned()
    }

    pub fn framework(&self, task_id: &TaskId) -> Option<FrameworkSlug> {
        let state = self.state.lock().expect("hash tracker mutex poisoned");
        state.package_task_framework.get(task_id).cloned()
    }

    pub fn expanded_outputs(&self, task_id: &TaskId) -> Option<Vec<AnchoredSystemPathBuf>> {
        let state = self.state.lock().expect("hash tracker mutex poisoned");
        state.package_task_outputs.get(task_id).cloned()
    }

    pub fn insert_expanded_outputs(
        &self,
        task_id: TaskId<'static>,
        outputs: Vec<AnchoredSystemPathBuf>,
    ) {
        let mut state = self.state.lock().expect("hash tracker mutex poisoned");
        state.package_task_outputs.insert(task_id, outputs);
    }

    pub fn cache_status(&self, task_id: &TaskId) -> Option<CacheHitMetadata> {
        let state = self.state.lock().expect("hash tracker mutex poisoned");
        state.package_task_cache.get(task_id).copied()
    }

    pub fn insert_cache_status(&self, task_id: TaskId<'static>, cache_status: CacheHitMetadata) {
        let mut state = self.state.lock().expect("hash tracker mutex poisoned");
        state.package_task_cache.insert(task_id, cache_status);
    }

    pub fn get_expanded_inputs(&self, task_id: &TaskId) -> Option<FileHashes> {
        let state = self.state.lock().expect("hash tracker mutex poisoned");
        state
            .package_task_inputs_expanded_hashes
            .get(task_id)
            .cloned()
    }
}

// Implement HashTrackerInfo for TaskHashTracker to allow use with
// turborepo-run-summary. The trait is defined in turborepo-types to enable
// proper dependency direction (task-hash doesn't depend on run-summary).
impl HashTrackerInfo for TaskHashTracker {
    fn hash(&self, task_id: &TaskId) -> Option<String> {
        TaskHashTracker::hash(self, task_id)
    }

    fn env_vars(&self, task_id: &TaskId) -> Option<HashTrackerDetailedMap> {
        TaskHashTracker::env_vars(self, task_id).map(|detailed| HashTrackerDetailedMap {
            explicit: detailed.by_source.explicit.to_secret_hashable(),
            matching: detailed.by_source.matching.to_secret_hashable(),
        })
    }

    fn cache_status(&self, task_id: &TaskId) -> Option<HashTrackerCacheHitMetadata> {
        TaskHashTracker::cache_status(self, task_id).map(|status| {
            let (local, remote) = match status.source {
                turborepo_cache::CacheSource::Local => (true, false),
                turborepo_cache::CacheSource::Remote => (false, true),
            };
            HashTrackerCacheHitMetadata {
                local,
                remote,
                time_saved: status.time_saved,
            }
        })
    }

    fn expanded_outputs(&self, task_id: &TaskId) -> Option<Vec<AnchoredSystemPathBuf>> {
        TaskHashTracker::expanded_outputs(self, task_id)
    }

    fn framework(&self, task_id: &TaskId) -> Option<String> {
        TaskHashTracker::framework(self, task_id).map(|f| f.to_string())
    }

    fn expanded_inputs(
        &self,
        task_id: &TaskId,
    ) -> Option<std::collections::HashMap<RelativeUnixPathBuf, String>> {
        TaskHashTracker::get_expanded_inputs(self, task_id).map(|file_hashes| file_hashes.0)
    }
}

// Implement HashTrackerProvider for TaskHashTracker to allow use with
// turborepo-task-executor's TaskExecutor.
impl turborepo_task_executor::HashTrackerProvider for TaskHashTracker {
    fn insert_cache_status(&self, task_id: TaskId<'static>, status: CacheHitMetadata) {
        TaskHashTracker::insert_cache_status(self, task_id, status)
    }

    fn insert_expanded_outputs(
        &self,
        task_id: TaskId<'static>,
        outputs: Vec<AnchoredSystemPathBuf>,
    ) {
        TaskHashTracker::insert_expanded_outputs(self, task_id, outputs)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_hash_tracker_is_send_and_sync() {
        // We need the tracker to implement these traits as multiple tasks will query
        // and write to it
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}
        assert_send::<TaskHashTracker>();
        assert_sync::<TaskHashTracker>();
    }
}
