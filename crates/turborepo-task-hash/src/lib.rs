//! turborepo-task-hash: Task hashing utilities for Turborepo cache invalidation
//!
//! This crate provides the core task hashing logic for Turborepo. It computes
//! hashes for tasks based on their inputs (files, environment variables,
//! dependencies) to determine cache invalidation.

#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

pub mod global_hash;

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard},
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
use turborepo_engine::TaskNode;
use turborepo_env::{
    BUILTIN_PASS_THROUGH_ENV, BySource, CompiledWildcards, DetailedMap, EnvironmentVariableMap,
    WildcardMapCache,
};
use turborepo_frameworks::{Framework, Slug as FrameworkSlug, infer_framework};
use turborepo_hash::{FileHashes, LockFilePackagesRef, TaskHashable, TurboHash};
use turborepo_repository::package_graph::{PackageInfo, PackageName};
use turborepo_scm::{RepoGitIndex, SCM};
use turborepo_task_id::TaskId;
use turborepo_telemetry::events::{generic::GenericEventBuilder, task::PackageTaskEventBuilder};
use turborepo_types::{
    EnvMode, HashTrackerCacheHitMetadata, HashTrackerDetailedMap, HashTrackerInfo, RunOptsHashInfo,
    TaskDefinitionHashInfo, TaskInputs,
};

fn env_var_names_for_debug_log(env_vars: &EnvironmentVariableMap) -> Vec<String> {
    env_vars.names()
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
    #[error(transparent)]
    Hash(#[from] turborepo_hash::Error),
}

#[derive(Debug, Default)]
pub struct PackageInputsHashes {
    hashes: HashMap<TaskId<'static>, String>,
    expanded_hashes: HashMap<TaskId<'static>, Arc<FileHashes>>,
}

pub const JIT_DEFERRED_TASK_HASH_MESSAGE: &str = "Deferred because JIT hashing mode was used.";
pub const DEPENDENCY_OUTPUTS_DEFERRED_TASK_HASH_MESSAGE: &str =
    "Deferred because dependencyOutputs hashing mode was used.";

impl PackageInputsHashes {
    #[tracing::instrument(skip(
        all_tasks,
        workspaces,
        task_definitions,
        repo_root,
        scm,
        _telemetry,
        pre_built_index
    ))]
    pub fn calculate_file_hashes<'a, T>(
        scm: &SCM,
        all_tasks: impl Iterator<Item = &'a TaskNode>,
        workspaces: HashMap<&PackageName, &PackageInfo>,
        task_definitions: &HashMap<TaskId<'static>, T>,
        repo_root: &AbsoluteSystemPath,
        _telemetry: &GenericEventBuilder,
        pre_built_index: Option<&RepoGitIndex>,
        needs_expanded_hashes: bool,
    ) -> Result<PackageInputsHashes, Error>
    where
        T: TaskDefinitionHashInfo + Sync,
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

        let collect_span = tracing::info_span!("collect_task_hash_keys").entered();
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
                .unwrap_or_else(|| AnchoredSystemPath::empty());
            let inputs = task_definition.inputs();
            task_infos.push(TaskInfo {
                task_id: task_id.clone(),
                package_path,
                inputs,
            });
        }

        // Build dedup key: (package_path_str, globs, default, eager)
        type HashKey = (AnchoredSystemPathBuf, Vec<String>, bool, bool);
        let mut unique_keys: Vec<HashKey> = Vec::new();
        let mut key_indices: HashMap<HashKey, usize> = HashMap::new();
        let mut task_key_map: Vec<usize> = Vec::with_capacity(task_infos.len());

        for info in &task_infos {
            let key: HashKey = (
                info.package_path.to_owned(),
                info.inputs.globs.clone(),
                info.inputs.default,
                info.inputs.eager,
            );
            let idx = match key_indices.entry(key) {
                std::collections::hash_map::Entry::Occupied(e) => *e.get(),
                std::collections::hash_map::Entry::Vacant(e) => {
                    let idx = unique_keys.len();
                    unique_keys.push(e.key().clone());
                    e.insert(idx);
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
        drop(collect_span);

        // Phase 2: Compute file hashes in parallel across unique keys. The
        // summary hash of each `FileHashes` is computed here too, once per
        // unique key, so distribution below never re-hashes for the many
        // tasks that share a key.
        // EMFILE (too many open files) errors are handled via retry-with-backoff
        // in the globwalk and hash_objects layers, so we can safely parallelize
        // all keys on rayon without worrying about fd exhaustion.
        let hash_span = tracing::info_span!("hash_unique_inputs").entered();
        let file_hash_results: Vec<Result<(Arc<FileHashes>, String), Error>> = unique_keys
            .into_par_iter()
            .map(|(package_path, globs, default, eager)| {
                let file_hashes = if !eager {
                    Arc::new(FileHashes(Vec::new()))
                } else {
                    file_hashes_for_inputs(
                        scm,
                        repo_root,
                        &package_path,
                        &globs,
                        default,
                        repo_index,
                    )?
                };
                let hash = file_hashes.as_ref().hash();
                Ok((file_hashes, hash))
            })
            .collect();

        let file_hash_results: Vec<(Arc<FileHashes>, String)> = file_hash_results
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;
        drop(hash_span);

        // Phase 3: Distribute shared results to individual tasks.
        let _span = tracing::info_span!("distribute_task_file_hashes").entered();
        let mut hashes = HashMap::with_capacity(task_infos.len());
        let mut expanded_hashes = if needs_expanded_hashes {
            HashMap::with_capacity(task_infos.len())
        } else {
            HashMap::new()
        };

        for (i, info) in task_infos.into_iter().enumerate() {
            let key_idx = task_key_map[i];
            let (file_hashes, hash) = &file_hash_results[key_idx];

            hashes.insert(info.task_id.clone(), hash.clone());
            if needs_expanded_hashes || info.inputs.has_deferred_inputs() {
                expanded_hashes.insert(info.task_id, Arc::clone(file_hashes));
            }
        }

        Ok(PackageInputsHashes {
            hashes,
            expanded_hashes,
        })
    }
}

/// Collect the external dependency hash for every workspace, keyed by
/// package name. Hashes are precomputed where closures are computed (see
/// [`hash_sorted_closures`]); the per-package fallback only runs for graphs
/// built without a closure hasher.
#[tracing::instrument(skip_all)]
pub fn compute_external_deps_hashes<'b>(
    workspaces: impl Iterator<Item = (&'b PackageName, &'b PackageInfo)>,
) -> HashMap<String, String> {
    workspaces
        .map(|(name, info)| {
            let hash = info
                .external_deps_hash
                .clone()
                .unwrap_or_else(|| get_external_deps_hash(&info.transitive_dependencies));
            (name.as_str().to_owned(), hash)
        })
        .collect()
}

#[derive(Default, Debug, Clone)]
pub struct TaskHashTracker {
    state: Arc<RwLock<TaskHashTrackerState>>,
}

#[derive(Default, Debug, Serialize)]
pub struct TaskHashTrackerState {
    #[serde(skip)]
    package_task_env_vars: HashMap<TaskId<'static>, DetailedMap>,
    package_task_hashes: HashMap<TaskId<'static>, Arc<str>>,
    #[serde(skip)]
    package_task_framework: HashMap<TaskId<'static>, FrameworkSlug>,
    #[serde(skip)]
    package_task_outputs: HashMap<TaskId<'static>, Vec<AnchoredSystemPathBuf>>,
    #[serde(skip)]
    package_task_cache: HashMap<TaskId<'static>, CacheHitMetadata>,
    #[serde(skip)]
    package_task_inputs_expanded_hashes: HashMap<TaskId<'static>, Arc<FileHashes>>,
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
    /// Builtin pass-through env vars matched against the environment once at
    /// construction; the set is invariant for the lifetime of the hasher.
    builtin_pass_through_env: EnvironmentVariableMap,
    /// Memoized wildcard matches so tasks sharing the same `env` or
    /// `passThroughEnv` patterns don't recompile regexes and rescan the
    /// environment.
    wildcard_cache: WildcardMapCache,
    external_deps_hash_cache: HashMap<String, String>,
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

        let builtin_pass_through_env = CompiledWildcards::compile(BUILTIN_PASS_THROUGH_ENV)
            .ok()
            .map(|compiled| env_at_execution_start.from_compiled_wildcards(&compiled))
            .unwrap_or_default();

        Self {
            hashes,
            run_opts,
            env_at_execution_start,
            global_hash,
            global_env,
            global_env_patterns,
            task_hash_tracker: TaskHashTracker::new(expanded_hashes),
            builtin_pass_through_env,
            wildcard_cache: WildcardMapCache::default(),
            external_deps_hash_cache: HashMap::new(),
        }
    }

    /// Pre-compute and cache external dependency hashes for all packages.
    /// Many tasks share the same package, so this avoids re-sorting
    /// transitive dependencies for every task.
    #[tracing::instrument(skip_all)]
    pub fn precompute_external_deps_hashes<'b>(
        &mut self,
        workspaces: impl Iterator<Item = (&'b PackageName, &'b PackageInfo)>,
    ) {
        if self.run_opts.single_package() {
            return;
        }
        self.external_deps_hash_cache = compute_external_deps_hashes(workspaces);
    }

    /// Install an externally computed dependency-hash cache (see
    /// [`compute_external_deps_hashes`]). Lets callers compute the cache
    /// concurrently with other startup work instead of serially during
    /// hasher construction.
    pub fn set_external_deps_hash_cache(&mut self, cache: HashMap<String, String>) {
        self.external_deps_hash_cache = cache;
    }

    #[tracing::instrument(skip(self, task_definition, task_env_mode, workspace, dependency_set))]
    pub fn calculate_task_hash<T: TaskDefinitionHashInfo>(
        &self,
        task_id: &TaskId<'static>,
        task_definition: &T,
        task_env_mode: EnvMode,
        workspace: &PackageInfo,
        dependency_set: &[&TaskNode],
        telemetry: PackageTaskEventBuilder,
    ) -> Result<String, Error> {
        let hash_of_files = self
            .hashes
            .get(task_id)
            .ok_or_else(|| Error::MissingPackageFileHash(task_id.to_string()))?;
        self.calculate_task_hash_with_file_hash(
            task_id,
            task_definition,
            task_env_mode,
            workspace,
            dependency_set,
            telemetry,
            hash_of_files,
            None,
        )
    }

    #[tracing::instrument(skip(
        self,
        task_definition,
        task_env_mode,
        workspace,
        dependency_set,
        scm,
        repo_index
    ))]
    pub fn calculate_task_hash_with_deferred_inputs<T: TaskDefinitionHashInfo>(
        &self,
        task_id: &TaskId<'static>,
        task_definition: &T,
        task_env_mode: EnvMode,
        workspace: &PackageInfo,
        dependency_set: &[&TaskNode],
        telemetry: PackageTaskEventBuilder,
        scm: &SCM,
        repo_root: &AbsoluteSystemPath,
        repo_index: Option<&RepoGitIndex>,
        dependency_output_hashes: Option<Arc<FileHashes>>,
        dependency_output_producers: &HashSet<TaskId<'static>>,
    ) -> Result<String, Error> {
        let package_path = workspace.package_path();
        let jit_hashes = task_definition
            .inputs()
            .has_jit_inputs()
            .then(|| {
                file_hashes_for_inputs(
                    scm,
                    repo_root,
                    package_path,
                    &task_definition.inputs().jit_globs,
                    task_definition.inputs().jit_default,
                    repo_index,
                )
            })
            .transpose()?;
        let eager_hashes = self
            .task_hash_tracker
            .get_expanded_inputs(task_id)
            .ok_or_else(|| Error::MissingPackageFileHash(task_id.to_string()))?;
        let mut combined_hashes = eager_hashes;
        if let Some(jit_hashes) = jit_hashes {
            combined_hashes = combine_file_hashes(&combined_hashes, &jit_hashes);
        }
        if let Some(dependency_output_hashes) = dependency_output_hashes {
            combined_hashes = combine_file_hashes(&combined_hashes, &dependency_output_hashes);
        }
        let hash_of_files = combined_hashes.as_ref().hash();

        self.task_hash_tracker
            .insert_expanded_inputs(task_id.clone(), combined_hashes);

        self.calculate_task_hash_with_file_hash(
            task_id,
            task_definition,
            task_env_mode,
            workspace,
            dependency_set,
            telemetry,
            &hash_of_files,
            Some(dependency_output_producers),
        )
    }

    pub fn insert_deferred_hash<T: TaskDefinitionHashInfo>(
        &self,
        task_id: &TaskId<'static>,
        task_definition: &T,
        task_env_mode: EnvMode,
    ) -> Result<(), Error> {
        let env_vars = self.calculate_env_vars(task_id, task_definition, task_env_mode, None)?;
        self.task_hash_tracker.insert_hash(
            task_id.clone(),
            env_vars,
            Arc::from(deferred_task_hash_message(task_definition.inputs())),
            None,
        );
        Ok(())
    }

    fn calculate_task_hash_with_file_hash<T: TaskDefinitionHashInfo>(
        &self,
        task_id: &TaskId<'static>,
        task_definition: &T,
        task_env_mode: EnvMode,
        workspace: &PackageInfo,
        dependency_set: &[&TaskNode],
        telemetry: PackageTaskEventBuilder,
        hash_of_files: &str,
        excluded_dependency_hashes: Option<&HashSet<TaskId<'static>>>,
    ) -> Result<String, Error> {
        let do_framework_inference = self.run_opts.framework_inference();
        let is_monorepo = !self.run_opts.single_package();

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
        let framework_slug = framework.as_ref().map(|f| f.slug());
        let env_vars =
            self.calculate_env_vars(task_id, task_definition, task_env_mode, framework)?;

        let outputs = task_definition.hashable_outputs(task_id);
        let task_dependency_hashes =
            self.calculate_dependency_hashes(dependency_set, excluded_dependency_hashes)?;
        let ext_hash_fallback;
        let external_deps_hash: Option<&str> = if !is_monorepo {
            None
        } else if let Some(cached) = self.external_deps_hash_cache.get(task_id.package()) {
            Some(cached.as_str())
        } else {
            ext_hash_fallback = get_external_deps_hash(&workspace.transitive_dependencies);
            Some(ext_hash_fallback.as_str())
        };

        if !env_vars.all.is_empty() {
            debug!(
                "task hash env var names for {}:{}\n vars: {:?}",
                task_id.package(),
                task_id.task(),
                env_var_names_for_debug_log(&env_vars.all)
            );
        }

        let hashable_env_pairs = env_vars.all.to_hashable();

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

        let task_hash = task_hashable.calculate_task_hash()?;

        let task_hash_arc: Arc<str> = Arc::from(task_hash.as_str());
        self.task_hash_tracker.insert_hash(
            task_id.clone(),
            env_vars,
            task_hash_arc,
            framework_slug,
        );

        Ok(task_hash)
    }

    fn calculate_env_vars<T: TaskDefinitionHashInfo>(
        &self,
        task_id: &TaskId<'static>,
        task_definition: &T,
        _task_env_mode: EnvMode,
        framework: Option<&Framework>,
    ) -> Result<DetailedMap, Error> {
        if let Some(framework) = framework {
            let mut computed_wildcards = framework.env(self.env_at_execution_start);

            match self.env_at_execution_start.get("TURBO_CI_VENDOR_ENV_KEY") {
                Some(exclude_prefix) if !exclude_prefix.is_empty() => {
                    let computed_exclude = format!("!{exclude_prefix}*");
                    debug!("TURBO_CI_VENDOR_ENV_KEY present; excluding matching env vars");
                    computed_wildcards.push(computed_exclude);
                }
                Some(_) => {
                    debug!("TURBO_CI_VENDOR_ENV_KEY present but empty; no env vars excluded");
                }
                None => {
                    debug!("TURBO_CI_VENDOR_ENV_KEY not present; no env vars excluded");
                }
            }

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

            let inference = self
                .wildcard_cache
                .get_or_compute(self.env_at_execution_start, &computed_wildcards)
                .map_err(|err| Error::EnvPattern {
                    task_id: task_id.clone().into_owned(),
                    err,
                })?;
            let user_env_var_set = self
                .wildcard_cache
                .get_or_compute(self.env_at_execution_start, &combined_env_patterns)
                .map_err(|err| Error::EnvPattern {
                    task_id: task_id.clone().into_owned(),
                    err,
                })?;

            Ok(DetailedMap::from_task_env_parts(
                &inference.resolved,
                &user_env_var_set.maps,
            ))
        } else {
            let matched = self
                .wildcard_cache
                .get_or_compute(self.env_at_execution_start, task_definition.env())?;

            Ok(DetailedMap {
                by_source: BySource {
                    explicit: matched.resolved.clone(),
                    matching: EnvironmentVariableMap::default(),
                },
                all: matched.resolved.clone(),
            })
        }
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
        dependency_set: &[&TaskNode],
        excluded_dependency_hashes: Option<&HashSet<TaskId<'static>>>,
    ) -> Result<Vec<Arc<str>>, Error> {
        let mut dependency_hash_list = self.task_hash_tracker.with_state(|state| {
            let mut dependency_hash_list: Vec<Arc<str>> = Vec::with_capacity(dependency_set.len());
            for dependency_task in dependency_set {
                let TaskNode::Task(dependency_task_id) = dependency_task else {
                    continue;
                };
                if excluded_dependency_hashes
                    .is_some_and(|excluded| excluded.contains(dependency_task_id))
                {
                    continue;
                }

                let dependency_hash = state
                    .package_task_hashes
                    .get(dependency_task_id)
                    .ok_or_else(|| Error::MissingDependencyTaskHash(dependency_task.to_string()))?;
                dependency_hash_list.push(Arc::clone(dependency_hash));
            }

            Ok::<_, Error>(dependency_hash_list)
        })?;

        dependency_hash_list.sort_unstable();
        dependency_hash_list.dedup();

        Ok(dependency_hash_list)
    }

    pub fn into_task_hash_tracker_state(self) -> TaskHashTrackerState {
        self.task_hash_tracker.into_state()
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
                let task_pass_through = self.wildcard_cache.get_or_compute(
                    self.env_at_execution_start,
                    task_definition.pass_through_env().unwrap_or_default(),
                )?;

                let pass_through_env_vars = turborepo_env::pass_through_env_from_parts(
                    &self.builtin_pass_through_env,
                    &self.global_env,
                    &task_pass_through.maps,
                );

                let tracker_env = self
                    .task_hash_tracker
                    .env_vars(task_id)
                    .ok_or_else(|| Error::MissingEnvVars(task_id.clone().into_owned()))?;

                let mut full_task_env = EnvironmentVariableMap::default();
                full_task_env.union(&pass_through_env_vars);
                full_task_env.union(&tracker_env.all);

                Ok(full_task_env)
            }
            EnvMode::Loose => Ok(self.env_at_execution_start.clone()),
        }
    }
}

pub fn deferred_task_hash_message(inputs: &TaskInputs) -> &'static str {
    if inputs.has_dependency_outputs() {
        DEPENDENCY_OUTPUTS_DEFERRED_TASK_HASH_MESSAGE
    } else {
        JIT_DEFERRED_TASK_HASH_MESSAGE
    }
}

pub fn get_external_deps_hash(
    transitive_dependencies: &Option<Vec<Arc<turborepo_lockfiles::Package>>>,
) -> String {
    let Some(transitive_dependencies) = transitive_dependencies else {
        return "".into();
    };

    // The closure is already sorted by `Package`'s `(key, version)` ordering,
    // so hashing is a single linear pass.
    let transitive_deps: Vec<&turborepo_lockfiles::Package> =
        transitive_dependencies.iter().map(|pkg| &**pkg).collect();

    LockFilePackagesRef(transitive_deps).hash()
}

/// Hash every workspace's sorted external dependency closure, keyed by the
/// closure map's own keys. Intended as the `PackageGraphBuilder`
/// closure-hasher, so hashes are computed where closures are computed
/// (on the deferred-closure background thread) instead of after graph
/// construction.
pub fn hash_sorted_closures(
    closures: &HashMap<String, Vec<Arc<turborepo_lockfiles::Package>>>,
) -> HashMap<String, String> {
    closures
        .par_iter()
        .map(|(ws, closure)| {
            let refs: Vec<&turborepo_lockfiles::Package> =
                closure.iter().map(|pkg| &**pkg).collect();
            (ws.clone(), LockFilePackagesRef(refs).hash())
        })
        .collect()
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

    let merged = package_dirs
        .into_par_iter()
        .map(|package_dir| {
            scm.get_package_file_hashes::<&str>(root, package_dir, &[], false, None, repo_index)
        })
        .reduce(
            || Ok(HashMap::new()),
            |acc, hashes| {
                let mut acc = acc?;
                let hashes = hashes?;
                acc.extend(hashes);
                Ok(acc)
            },
        )?;

    let mut file_hashes: Vec<_> = merged.into_iter().collect();
    file_hashes.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));
    Ok(FileHashes(file_hashes).try_hash()?)
}

pub fn file_hashes_for_inputs<S: AsRef<str>>(
    scm: &SCM,
    repo_root: &AbsoluteSystemPath,
    package_path: &AnchoredSystemPath,
    globs: &[S],
    default: bool,
    repo_index: Option<&RepoGitIndex>,
) -> Result<Arc<FileHashes>, Error> {
    scm.get_package_file_hashes(repo_root, package_path, globs, default, None, repo_index)
        .map(|h| {
            let mut v: Vec<_> = h.into_iter().collect();
            v.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));
            Arc::new(FileHashes(v))
        })
        .map_err(Error::from)
}

pub fn combine_file_hashes(eager: &FileHashes, jit: &FileHashes) -> Arc<FileHashes> {
    let mut combined = BTreeMap::new();
    for (path, hash) in &eager.0 {
        combined.insert(path.clone(), *hash);
    }
    for (path, hash) in &jit.0 {
        combined.insert(path.clone(), *hash);
    }
    Arc::new(FileHashes(combined.into_iter().collect()))
}

impl TaskHashTracker {
    pub fn new(input_expanded_hashes: HashMap<TaskId<'static>, Arc<FileHashes>>) -> Self {
        Self {
            state: Arc::new(RwLock::new(TaskHashTrackerState {
                package_task_inputs_expanded_hashes: input_expanded_hashes,
                ..Default::default()
            })),
        }
    }

    fn read_state(&self) -> RwLockReadGuard<'_, TaskHashTrackerState> {
        match self.state.read() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn write_state(&self) -> RwLockWriteGuard<'_, TaskHashTrackerState> {
        match self.state.write() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn with_state<T>(&self, f: impl FnOnce(&TaskHashTrackerState) -> T) -> T {
        let state = self.read_state();
        f(&state)
    }

    fn with_state_mut<T>(&self, f: impl FnOnce(&mut TaskHashTrackerState) -> T) -> T {
        let mut state = self.write_state();
        f(&mut state)
    }

    fn into_state(self) -> TaskHashTrackerState {
        match Arc::try_unwrap(self.state) {
            Ok(lock) => match lock.into_inner() {
                Ok(state) => state,
                Err(poisoned) => poisoned.into_inner(),
            },
            Err(state) => {
                let mut state = match state.write() {
                    Ok(state) => state,
                    Err(poisoned) => poisoned.into_inner(),
                };
                std::mem::take(&mut *state)
            }
        }
    }

    pub fn hash(&self, task_id: &TaskId) -> Option<Arc<str>> {
        self.with_state(|state| state.package_task_hashes.get(task_id).cloned())
    }

    fn insert_hash(
        &self,
        task_id: TaskId<'static>,
        env_vars: DetailedMap,
        hash: Arc<str>,
        framework_slug: Option<FrameworkSlug>,
    ) {
        self.with_state_mut(|state| {
            state
                .package_task_env_vars
                .insert(task_id.clone(), env_vars);
            if let Some(framework) = framework_slug {
                // Only pay for one extra clone when framework inference is active.
                state
                    .package_task_framework
                    .insert(task_id.clone(), framework);
            }
            state.package_task_hashes.insert(task_id, hash);
        });
    }

    pub fn env_vars(&self, task_id: &TaskId) -> Option<DetailedMap> {
        self.with_state(|state| state.package_task_env_vars.get(task_id).cloned())
    }

    pub fn framework(&self, task_id: &TaskId) -> Option<FrameworkSlug> {
        self.with_state(|state| state.package_task_framework.get(task_id).cloned())
    }

    pub fn expanded_outputs(&self, task_id: &TaskId) -> Option<Vec<AnchoredSystemPathBuf>> {
        self.with_state(|state| state.package_task_outputs.get(task_id).cloned())
    }

    pub fn insert_expanded_outputs(
        &self,
        task_id: TaskId<'static>,
        outputs: Vec<AnchoredSystemPathBuf>,
    ) {
        self.with_state_mut(|state| {
            state.package_task_outputs.insert(task_id, outputs);
        });
    }

    pub fn insert_expanded_inputs(&self, task_id: TaskId<'static>, inputs: Arc<FileHashes>) {
        self.with_state_mut(|state| {
            state
                .package_task_inputs_expanded_hashes
                .insert(task_id, inputs);
        });
    }

    pub fn cache_status(&self, task_id: &TaskId) -> Option<CacheHitMetadata> {
        self.with_state(|state| state.package_task_cache.get(task_id).cloned())
    }

    pub fn insert_cache_status(&self, task_id: TaskId<'static>, cache_status: CacheHitMetadata) {
        self.with_state_mut(|state| {
            state.package_task_cache.insert(task_id, cache_status);
        });
    }

    pub fn get_expanded_inputs(&self, task_id: &TaskId) -> Option<Arc<FileHashes>> {
        self.with_state(|state| {
            state
                .package_task_inputs_expanded_hashes
                .get(task_id)
                .cloned()
        })
    }
}

// Implement HashTrackerInfo for TaskHashTracker to allow use with
// turborepo-run-summary. The trait is defined in turborepo-types to enable
// proper dependency direction (task-hash doesn't depend on run-summary).
impl HashTrackerInfo for TaskHashTracker {
    fn hash(&self, task_id: &TaskId) -> Option<Arc<str>> {
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
                sha: status.sha,
                dirty_hash: status.dirty_hash,
            }
        })
    }

    fn expanded_outputs(&self, task_id: &TaskId) -> Option<Vec<AnchoredSystemPathBuf>> {
        TaskHashTracker::expanded_outputs(self, task_id)
    }

    fn framework(&self, task_id: &TaskId) -> Option<String> {
        TaskHashTracker::framework(self, task_id).map(|f| f.to_string())
    }

    fn expanded_inputs(&self, task_id: &TaskId) -> Option<Vec<(RelativeUnixPathBuf, String)>> {
        TaskHashTracker::get_expanded_inputs(self, task_id).map(|file_hashes| {
            file_hashes
                .0
                .iter()
                .map(|(k, v)| (k.clone(), String::from(*v)))
                .collect()
        })
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

    #[test]
    fn test_task_hash_debug_env_vars_exclude_values() {
        let env_vars = EnvironmentVariableMap::from(HashMap::from([
            ("SECRET_TOKEN".to_string(), "super-secret-token".to_string()),
            ("PUBLIC_FLAG".to_string(), "true".to_string()),
        ]));

        let debug_env_vars = env_var_names_for_debug_log(&env_vars);

        assert_eq!(
            debug_env_vars,
            vec!["PUBLIC_FLAG".to_string(), "SECRET_TOKEN".to_string()]
        );

        let rendered_log_value = format!("{debug_env_vars:?}");
        assert!(!rendered_log_value.contains("super-secret-token"));
        assert!(!rendered_log_value.contains("true"));
    }

    #[test]
    fn test_hash_tracker_concurrent_reads() {
        let tracker = TaskHashTracker::new(HashMap::new());
        let task_id: TaskId<'static> = TaskId::new("pkg", "build");
        tracker.insert_hash(
            task_id.clone(),
            DetailedMap::default(),
            Arc::from("abc123"),
            None,
        );

        // Multiple concurrent reads should not deadlock or panic with RwLock
        std::thread::scope(|s| {
            for _ in 0..8 {
                let tracker = &tracker;
                let task_id = &task_id;
                s.spawn(move || {
                    for _ in 0..100 {
                        let h = tracker.hash(task_id);
                        assert_eq!(h.as_deref(), Some("abc123"));
                    }
                });
            }
        });
    }

    #[test]
    fn test_hash_tracker_concurrent_read_write() {
        let tracker = TaskHashTracker::new(HashMap::new());

        // Pre-create owned task IDs to avoid lifetime issues with TaskId borrows
        let task_ids: Vec<TaskId<'static>> = (0..50)
            .map(|i| TaskId::new("pkg", &format!("task-{i}")).into_owned())
            .collect();

        // One writer, many readers — verifies RwLock allows concurrent reads
        // while writes are exclusive, without deadlock.
        std::thread::scope(|s| {
            let tracker = &tracker;
            let task_ids = &task_ids;

            s.spawn(move || {
                for (i, task_id) in task_ids.iter().enumerate() {
                    tracker.insert_hash(
                        task_id.clone(),
                        DetailedMap::default(),
                        Arc::from(format!("hash-{i}").as_str()),
                        None,
                    );
                }
            });

            for _ in 0..4 {
                s.spawn(move || {
                    for task_id in task_ids {
                        // May or may not find the hash depending on timing — that's fine,
                        // we're testing for absence of panics/deadlocks.
                        let _ = tracker.hash(task_id);
                        let _ = tracker.env_vars(task_id);
                        let _ = tracker.cache_status(task_id);
                    }
                });
            }
        });
    }

    #[test]
    fn test_expanded_inputs_returns_cloned_data() {
        use turborepo_types::HashTrackerInfo;

        let task_id: TaskId<'static> = TaskId::new("pkg", "build");
        // Sorted by key (the invariant FileHashes requires)
        let file_hashes = FileHashes(vec![
            (
                RelativeUnixPathBuf::new("package.json").unwrap(),
                turborepo_hash::OidHash::from_hex_str("def456def456def456def456def456def456def4"),
            ),
            (
                RelativeUnixPathBuf::new("src/index.ts").unwrap(),
                turborepo_hash::OidHash::from_hex_str("abc123abc123abc123abc123abc123abc123abc1"),
            ),
            (
                RelativeUnixPathBuf::new("src/utils/helper.ts").unwrap(),
                turborepo_hash::OidHash::from_hex_str("0123456789abcdef0123456789abcdef01234567"),
            ),
        ]);

        let mut input_hashes = HashMap::new();
        input_hashes.insert(task_id.clone(), Arc::new(file_hashes));
        let tracker = TaskHashTracker::new(input_hashes);

        // Via concrete method
        let arc_result = tracker.get_expanded_inputs(&task_id);
        assert!(arc_result.is_some());
        let arc_hashes = arc_result.unwrap();
        assert_eq!(arc_hashes.0.len(), 3);
        assert_eq!(arc_hashes.0[1].0.as_str(), "src/index.ts");
        assert_eq!(
            arc_hashes.0[1].1,
            "abc123abc123abc123abc123abc123abc123abc1"
        );

        // Via trait method — returns sorted Vec of (path, String)
        let trait_result: Option<Vec<(RelativeUnixPathBuf, String)>> =
            HashTrackerInfo::expanded_inputs(&tracker, &task_id);
        assert!(trait_result.is_some());
        let trait_hashes = trait_result.unwrap();
        assert_eq!(trait_hashes.len(), 3);
        assert_eq!(trait_hashes[0].0.as_str(), "package.json");
        assert_eq!(
            trait_hashes[0].1,
            "def456def456def456def456def456def456def4"
        );
        // Must be sorted by key
        assert!(
            trait_hashes.windows(2).all(|w| w[0].0 < w[1].0),
            "expanded_inputs should return sorted keys"
        );

        // Missing task returns None
        let missing = TaskId::new("other", "test");
        assert!(tracker.get_expanded_inputs(&missing).is_none());
        assert!(HashTrackerInfo::expanded_inputs(&tracker, &missing).is_none());
    }

    // Regression: expanded_inputs data must contain all entries and be sorted
    // by key. This captures the invariant that must hold when switching the
    // return type from BTreeMap to sorted Vec.
    #[test]
    fn test_expanded_inputs_sorted_and_complete() {
        use turborepo_types::HashTrackerInfo;

        let task_id: TaskId<'static> = TaskId::new("pkg", "build");
        // Sorted by key (FileHashes invariant)
        let file_hashes = FileHashes(vec![
            (
                RelativeUnixPathBuf::new("a/first.ts").unwrap(),
                turborepo_hash::OidHash::from_hex_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            ),
            (
                RelativeUnixPathBuf::new("a/second.ts").unwrap(),
                turborepo_hash::OidHash::from_hex_str("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            ),
            (
                RelativeUnixPathBuf::new("m/middle.ts").unwrap(),
                turborepo_hash::OidHash::from_hex_str("cccccccccccccccccccccccccccccccccccccccc"),
            ),
            (
                RelativeUnixPathBuf::new("z/last.ts").unwrap(),
                turborepo_hash::OidHash::from_hex_str("dddddddddddddddddddddddddddddddddddddddd"),
            ),
        ]);

        let mut input_hashes = HashMap::new();
        input_hashes.insert(task_id.clone(), Arc::new(file_hashes));
        let tracker = TaskHashTracker::new(input_hashes);

        let result = HashTrackerInfo::expanded_inputs(&tracker, &task_id).unwrap();
        assert_eq!(result.len(), 4, "all entries must be present");

        // Entries must be sorted by key
        assert!(
            result.windows(2).all(|w| w[0].0 < w[1].0),
            "expanded_inputs must return keys in sorted order"
        );

        // Verify specific values
        assert_eq!(result[0].0.as_str(), "a/first.ts");
        assert_eq!(result[0].1, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert_eq!(result[3].0.as_str(), "z/last.ts");
        assert_eq!(result[3].1, "dddddddddddddddddddddddddddddddddddddddd");
    }

    fn sorted_closure(
        packages: Vec<turborepo_lockfiles::Package>,
    ) -> Vec<Arc<turborepo_lockfiles::Package>> {
        let mut closure: Vec<Arc<turborepo_lockfiles::Package>> =
            packages.into_iter().map(Arc::new).collect();
        closure.sort_unstable();
        closure
    }

    #[test]
    fn test_external_deps_hash_deterministic() {
        use turborepo_lockfiles::Package;

        let deps = sorted_closure(vec![
            Package {
                key: "react".to_string(),
                version: "18.0.0".to_string(),
            },
            Package {
                key: "lodash".to_string(),
                version: "4.17.21".to_string(),
            },
            Package {
                key: "typescript".to_string(),
                version: "5.0.0".to_string(),
            },
        ]);

        let hash1 = get_external_deps_hash(&Some(deps.clone()));
        let hash2 = get_external_deps_hash(&Some(deps));
        assert_eq!(hash1, hash2, "same deps should produce same hash");
        assert!(!hash1.is_empty(), "hash should be non-empty");
    }

    #[test]
    fn test_external_deps_hash_empty() {
        let hash_none = get_external_deps_hash(&None);
        assert_eq!(hash_none, "", "None deps should produce empty hash");

        let hash_empty = get_external_deps_hash(&Some(Vec::new()));
        assert!(
            !hash_empty.is_empty(),
            "empty closure should produce non-empty hash"
        );
    }

    /// The linear hash of a pre-sorted closure must be byte-identical to the
    /// legacy path, which collected a `HashSet` and sorted by
    /// `(key, version)` before hashing.
    #[test]
    fn test_external_deps_hash_matches_legacy_sort_then_hash() {
        use turborepo_lockfiles::Package;

        let packages = vec![
            Package {
                key: "b".to_string(),
                version: "2.0".to_string(),
            },
            Package {
                key: "a".to_string(),
                version: "1.1".to_string(),
            },
            Package {
                key: "a".to_string(),
                version: "1.0".to_string(),
            },
            Package {
                key: "c".to_string(),
                version: "0.1".to_string(),
            },
        ];

        let legacy_hash = {
            let set: HashSet<Package> = packages.iter().cloned().collect();
            let mut refs: Vec<&Package> = set.iter().collect();
            refs.sort_unstable_by(|a, b| match a.key.cmp(&b.key) {
                std::cmp::Ordering::Equal => a.version.cmp(&b.version),
                other => other,
            });
            LockFilePackagesRef(refs).hash()
        };

        let sorted_hash = get_external_deps_hash(&Some(sorted_closure(packages)));
        assert_eq!(
            legacy_hash, sorted_hash,
            "sorted-closure hash must match the legacy sort-then-hash path"
        );
    }

    #[test]
    fn test_tracker_pre_sized_hashmaps() {
        let mut input_hashes = HashMap::new();
        for i in 0..100 {
            let task_id = TaskId::new("pkg", &format!("task-{i}")).into_owned();
            input_hashes.insert(task_id, Arc::new(FileHashes(Vec::new())));
        }
        let tracker = TaskHashTracker::new(input_hashes);

        // Insert hashes and verify pre-sizing didn't break anything
        for i in 0..100 {
            let task_id = TaskId::new("pkg", &format!("task-{i}")).into_owned();
            tracker.insert_hash(
                task_id.clone(),
                DetailedMap::default(),
                Arc::from(format!("hash-{i}").as_str()),
                None,
            );
            assert_eq!(
                tracker.hash(&task_id).as_deref(),
                Some(format!("hash-{i}").as_str())
            );
        }
    }

    // Validates that sort+dedup produces the same result as the previous
    // HashSet→Vec→sort approach for dependency hash deduplication.
    #[test]
    fn test_sort_dedup_matches_hashset_behavior() {
        let inputs: Vec<Vec<&str>> = vec![
            vec!["abc", "def", "abc", "ghi", "def"],
            vec!["zzz", "aaa", "mmm"],
            vec!["same", "same", "same"],
            vec![],
            vec!["only-one"],
        ];

        for input in inputs {
            // New approach: sort + dedup
            let mut sort_dedup: Vec<String> = input.iter().map(|s| s.to_string()).collect();
            sort_dedup.sort_unstable();
            sort_dedup.dedup();

            // Old approach: HashSet → Vec → sort
            let hash_set: HashSet<String> = input.iter().map(|s| s.to_string()).collect();
            let mut hashset_sorted: Vec<String> = hash_set.into_iter().collect();
            hashset_sorted.sort();

            assert_eq!(
                sort_dedup, hashset_sorted,
                "sort+dedup and hashset+sort should produce identical results for: {input:?}"
            );
        }
    }

    /// When `needs_expanded_hashes` is false, `calculate_file_hashes` returns
    /// an empty `expanded_hashes` map. The tracker must gracefully return
    /// `None` for any task — not panic — even though the task's collapsed
    /// hash was computed.
    #[test]
    fn test_expanded_inputs_none_when_not_collected() {
        use turborepo_types::HashTrackerInfo;

        let task_id: TaskId<'static> = TaskId::new("pkg", "build");

        // Simulate needs_expanded_hashes=false: tracker has no expanded hashes
        let tracker = TaskHashTracker::new(HashMap::new());
        tracker.insert_hash(
            task_id.clone(),
            DetailedMap::default(),
            Arc::from("somehash"),
            None,
        );

        // The collapsed hash exists
        assert!(tracker.hash(&task_id).is_some());
        // But expanded inputs must return None, not panic
        assert!(tracker.get_expanded_inputs(&task_id).is_none());
        assert!(HashTrackerInfo::expanded_inputs(&tracker, &task_id).is_none());
    }
}
