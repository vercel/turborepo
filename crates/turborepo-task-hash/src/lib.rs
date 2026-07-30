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
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf,
    RelativeUnixPathBuf,
};
use turborepo_cache::CacheHitMetadata;
use turborepo_engine::TaskNode;
use turborepo_env::{
    BUILTIN_PASS_THROUGH_ENV, BySource, CompiledWildcards, DetailedMap, EnvironmentVariableMap,
    WildcardMapCache,
};
use turborepo_frameworks::{Framework, Slug as FrameworkSlug, infer_framework};
use turborepo_hash::{FileHashes, TaskHashable, TurboHash};
use turborepo_repository::package_graph::{PackageGraph, PackageName, PackageTaskContext};
use turborepo_scm::{RepoGitIndex, SCM};
use turborepo_task_id::TaskId;
use turborepo_telemetry::events::{generic::GenericEventBuilder, task::PackageTaskEventBuilder};
use turborepo_types::{
    EnvMode, HashTrackerCacheHitMetadata, HashTrackerDetailedMap, HashTrackerInfo, RunOptsHashInfo,
    TaskCommandOverride, TaskDefinitionHashInfo, TaskInputs,
};

fn env_var_names_for_debug_log(env_vars: &EnvironmentVariableMap) -> Vec<String> {
    env_vars.names()
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Missing pipeline entry: {0}")]
    MissingPipelineEntry(TaskId<'static>),
    #[error("Missing authoritative package task context for {0}.")]
    MissingPackageContext(String),
    #[error("Task {task_id} does not belong to package context {package}.")]
    TaskPackageMismatch {
        task_id: TaskId<'static>,
        package: PackageName,
    },
    #[error("Missing external dependency fingerprint for {0}.")]
    MissingExternalDependencyHash(PackageName),
    #[error(
        "Package context repository root {context_root} does not match hashing repository root \
         {repo_root}."
    )]
    ContextRepositoryRootMismatch {
        context_root: AbsoluteSystemPathBuf,
        repo_root: AbsoluteSystemPathBuf,
    },
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

fn validate_task_context(
    task_id: &TaskId<'static>,
    package_context: &PackageTaskContext<'_>,
    repository_root: &AbsoluteSystemPath,
) -> Result<(), Error> {
    if package_context.repository_root() != repository_root {
        return Err(Error::ContextRepositoryRootMismatch {
            context_root: package_context.repository_root().to_owned(),
            repo_root: repository_root.to_owned(),
        });
    }
    let task_package = task_id.to_workspace_name();
    if &task_package == package_context.package() {
        Ok(())
    } else {
        Err(Error::TaskPackageMismatch {
            task_id: task_id.clone(),
            package: package_context.package().clone(),
        })
    }
}

impl PackageInputsHashes {
    #[tracing::instrument(skip(
        all_tasks,
        package_graph,
        task_definitions,
        repo_root,
        scm,
        _telemetry,
        pre_built_index
    ))]
    pub fn calculate_file_hashes<'a, T>(
        scm: &SCM,
        all_tasks: impl Iterator<Item = &'a TaskNode>,
        package_graph: &PackageGraph,
        task_definitions: &HashMap<TaskId<'static>, T>,
        repo_root: &AbsoluteSystemPath,
        _telemetry: &GenericEventBuilder,
        pre_built_index: Option<&RepoGitIndex>,
        needs_expanded_hashes: bool,
    ) -> Result<PackageInputsHashes, Error>
    where
        T: TaskDefinitionHashInfo + Sync,
    {
        if package_graph.repo_root() != repo_root {
            return Err(Error::ContextRepositoryRootMismatch {
                context_root: package_graph.repo_root().to_owned(),
                repo_root: repo_root.to_owned(),
            });
        }
        tracing::trace!(scm_manual=%scm.is_manual(), "scm running in {} mode", if scm.is_manual() { "manual" } else { "git" });

        // Use the pre-built index if provided, otherwise build one on the spot.
        let owned_index;
        let repo_index = match pre_built_index {
            Some(idx) => Some(idx),
            None => {
                owned_index = scm.build_repo_index(package_graph.len());
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
            let package_path = package_graph
                .package_task_context(&workspace_name)
                .map(|context| context.directory())
                .ok_or_else(|| Error::MissingPackageContext(workspace_name.to_string()))?;
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

/// Collect the stored external resolution fingerprint for every task namespace.
#[tracing::instrument(skip_all)]
pub fn compute_external_deps_hashes(
    package_graph: &PackageGraph,
) -> Result<HashMap<String, String>, Error> {
    package_graph
        .package_resolution_states()
        .into_iter()
        .map(|(package, resolution)| {
            let hash = resolution.task_hash().ok_or_else(|| {
                Error::MissingExternalDependencyHash(PackageName::from(package.as_str()))
            })?;
            Ok((package, hash.to_string()))
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
    repository_root: &'a AbsoluteSystemPath,
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
    pub fn validate_package_context(
        &self,
        task_id: &TaskId<'static>,
        package_context: &PackageTaskContext<'_>,
    ) -> Result<(), Error> {
        validate_task_context(task_id, package_context, self.repository_root)
    }

    pub fn new(
        package_inputs_hashes: PackageInputsHashes,
        run_opts: &'a R,
        env_at_execution_start: &'a EnvironmentVariableMap,
        global_hash: &'a str,
        repository_root: &'a AbsoluteSystemPath,
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
            repository_root,
            global_env,
            global_env_patterns,
            task_hash_tracker: TaskHashTracker::new(expanded_hashes),
            builtin_pass_through_env,
            wildcard_cache: WildcardMapCache::default(),
            external_deps_hash_cache: HashMap::new(),
        }
    }

    /// Cache stored external resolution fingerprints for all task namespaces.
    #[tracing::instrument(skip_all)]
    pub fn precompute_external_deps_hashes(
        &mut self,
        package_graph: &PackageGraph,
    ) -> Result<(), Error> {
        if self.run_opts.single_package() {
            return Ok(());
        }
        self.external_deps_hash_cache = compute_external_deps_hashes(package_graph)?;
        Ok(())
    }

    /// Install an externally computed dependency-hash cache (see
    /// [`compute_external_deps_hashes`]). Lets callers compute the cache
    /// concurrently with other startup work instead of serially during
    /// hasher construction.
    pub fn set_external_deps_hash_cache(&mut self, cache: HashMap<String, String>) {
        self.external_deps_hash_cache = cache;
    }

    /// Per-package external dependency hashes computed for task hashing.
    /// Exposed so run-summary construction can reuse them instead of
    /// re-sorting and re-hashing each package's transitive closure.
    pub fn external_deps_hash_cache(&self) -> &HashMap<String, String> {
        &self.external_deps_hash_cache
    }

    #[tracing::instrument(skip(
        self,
        task_definition,
        task_env_mode,
        package_context,
        dependency_set
    ))]
    pub fn calculate_task_hash<T: TaskDefinitionHashInfo>(
        &self,
        task_id: &TaskId<'static>,
        task_definition: &T,
        task_env_mode: EnvMode,
        package_context: &PackageTaskContext<'_>,
        dependency_set: &[&TaskNode],
        telemetry: PackageTaskEventBuilder,
    ) -> Result<String, Error> {
        self.validate_package_context(task_id, package_context)?;
        let hash_of_files = self
            .hashes
            .get(task_id)
            .ok_or_else(|| Error::MissingPackageFileHash(task_id.to_string()))?;
        self.calculate_task_hash_with_file_hash(
            task_id,
            task_definition,
            task_env_mode,
            package_context,
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
        package_context,
        dependency_set,
        scm,
        repo_index
    ))]
    pub fn calculate_task_hash_with_deferred_inputs<T: TaskDefinitionHashInfo>(
        &self,
        task_id: &TaskId<'static>,
        task_definition: &T,
        task_env_mode: EnvMode,
        package_context: &PackageTaskContext<'_>,
        dependency_set: &[&TaskNode],
        telemetry: PackageTaskEventBuilder,
        scm: &SCM,
        repo_root: &AbsoluteSystemPath,
        repo_index: Option<&RepoGitIndex>,
        dependency_output_hashes: Option<Arc<FileHashes>>,
        dependency_output_producers: &HashSet<TaskId<'static>>,
    ) -> Result<String, Error> {
        validate_task_context(task_id, package_context, repo_root)?;
        self.validate_package_context(task_id, package_context)?;
        if repo_root != self.repository_root {
            return Err(Error::ContextRepositoryRootMismatch {
                context_root: self.repository_root.to_owned(),
                repo_root: repo_root.to_owned(),
            });
        }
        let package_path = package_context.directory();
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
            package_context,
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
        package_context: &PackageTaskContext<'_>,
    ) -> Result<(), Error> {
        self.validate_package_context(task_id, package_context)?;
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
        package_context: &PackageTaskContext<'_>,
        dependency_set: &[&TaskNode],
        telemetry: PackageTaskEventBuilder,
        hash_of_files: &str,
        excluded_dependency_hashes: Option<&HashSet<TaskId<'static>>>,
    ) -> Result<String, Error> {
        let do_framework_inference = self.run_opts.framework_inference();
        let is_monorepo = !self.run_opts.single_package();

        // See if we can infer a framework
        let framework = do_framework_inference
            .then(|| infer_framework(package_context.external_declarations(), is_monorepo))
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
        let external_deps_hash: Option<&str> = if !is_monorepo {
            None
        } else {
            Some(
                self.external_deps_hash_cache
                    .get(task_id.package())
                    .ok_or_else(|| {
                        Error::MissingExternalDependencyHash(package_context.package().clone())
                    })?,
            )
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

        let package_dir = package_context.directory().to_unix();
        // We wrap in an Option to mimic Go's serialization of nullable values
        // and retain the existing bytes for every context located at the
        // repository directory, including aggregate Cargo task namespaces.
        // Task identity was independently validated against the context.
        let optional_package_dir = (!package_dir.is_empty()).then_some(package_dir);

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
            command_override: match task_definition.command() {
                Some(TaskCommandOverride::Argv(argv)) => argv.as_slice(),
                _ => &[],
            },
            command_opt_out: matches!(task_definition.command(), Some(TaskCommandOverride::OptOut)),
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
    use serde_json::json;
    use tempfile::tempdir;
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_repository::{
        package_graph::{PackageGraph, PackageTaskContextKind},
        package_json::PackageJson,
    };
    use turborepo_types::{RunOptsHashInfo, TaskDefinition};

    use super::*;

    struct TestRunOpts {
        single_package: bool,
    }

    impl RunOptsHashInfo for TestRunOpts {
        fn framework_inference(&self) -> bool {
            false
        }

        fn single_package(&self) -> bool {
            self.single_package
        }

        fn pass_through_args(&self) -> &[String] {
            &[]
        }
    }

    async fn javascript_graph(repo_root: &AbsoluteSystemPathBuf) -> PackageGraph {
        javascript_graph_at(repo_root, "packages").await
    }

    async fn javascript_graph_at(
        repo_root: &AbsoluteSystemPathBuf,
        packages_dir: &str,
    ) -> PackageGraph {
        let root_json = json!({
            "name": "root",
            "packageManager": "npm@10.0.0",
            "workspaces": [format!("{packages_dir}/*")]
        });
        repo_root
            .join_component("package.json")
            .create_with_contents(serde_json::to_string(&root_json).unwrap())
            .unwrap();
        let app_json = repo_root.join_components(&[packages_dir, "app", "package.json"]);
        app_json.ensure_dir().unwrap();
        app_json
            .create_with_contents(serde_json::to_string(&json!({ "name": "app" })).unwrap())
            .unwrap();
        let other_json = repo_root.join_components(&[packages_dir, "other", "package.json"]);
        other_json.ensure_dir().unwrap();
        other_json
            .create_with_contents(serde_json::to_string(&json!({ "name": "other" })).unwrap())
            .unwrap();

        PackageGraph::builder(repo_root, PackageJson::from_value(root_json).unwrap())
            .build()
            .await
            .unwrap()
    }

    async fn cargo_graph(repo_root: &AbsoluteSystemPathBuf) -> PackageGraph {
        repo_root
            .join_component("Cargo.toml")
            .create_with_contents(
                "[workspace]\nmembers = [\"crates/app\"]\nresolver = \
                 \"2\"\n\n[workspace.metadata]\nname = \"cargo-workspace\"\n",
            )
            .unwrap();
        repo_root
            .join_components(&["crates", "app", "Cargo.toml"])
            .ensure_dir()
            .unwrap();
        repo_root
            .join_components(&["crates", "app", "Cargo.toml"])
            .create_with_contents(
                "[package]\nname = \"cargo-app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            )
            .unwrap();
        repo_root
            .join_components(&["crates", "app", "src", "lib.rs"])
            .ensure_dir()
            .unwrap();
        repo_root
            .join_components(&["crates", "app", "src", "lib.rs"])
            .create_with_contents("")
            .unwrap();
        repo_root
            .join_component("Cargo.lock")
            .create_with_contents(
                "version = 4\n\n[[package]]\nname = \"cargo-app\"\nversion = \"0.1.0\"\n",
            )
            .unwrap();

        PackageGraph::builder_optional(repo_root, None)
            .with_cargo()
            .build()
            .await
            .unwrap()
    }

    fn task_hasher<'a>(
        task_id: &TaskId<'static>,
        run_opts: &'a TestRunOpts,
        env: &'a EnvironmentVariableMap,
        repository_root: &'a AbsoluteSystemPath,
    ) -> TaskHasher<'a, TestRunOpts> {
        let mut hashes = HashMap::new();
        hashes.insert(task_id.clone(), FileHashes(Vec::new()).hash());
        TaskHasher::new(
            PackageInputsHashes {
                hashes,
                expanded_hashes: HashMap::new(),
            },
            run_opts,
            env,
            "global-hash",
            repository_root,
            EnvironmentVariableMap::default(),
            &[],
        )
    }

    fn context_hash(graph: &PackageGraph, package: PackageName, single_package: bool) -> String {
        let task_id = TaskId::new(package.as_str(), "build").into_owned();
        let definition = TaskDefinition::default();
        let opts = TestRunOpts { single_package };
        let env = EnvironmentVariableMap::default();
        let mut hasher = task_hasher(&task_id, &opts, &env, graph.repo_root());
        if !single_package {
            hasher.precompute_external_deps_hashes(graph).unwrap();
        }
        hasher
            .calculate_task_hash(
                &task_id,
                &definition,
                EnvMode::Strict,
                &graph.package_task_context(&package).unwrap(),
                &[],
                PackageTaskEventBuilder::new(package.as_str(), "build"),
            )
            .unwrap()
    }

    fn monorepo_context_hash(graph: &PackageGraph, package: PackageName) -> String {
        let task_id = TaskId::new(package.as_str(), "build").into_owned();
        let definition = TaskDefinition::default();
        let opts = TestRunOpts {
            single_package: false,
        };
        let env = EnvironmentVariableMap::default();
        let mut hasher = task_hasher(&task_id, &opts, &env, graph.repo_root());
        hasher.precompute_external_deps_hashes(graph).unwrap();
        hasher
            .calculate_task_hash(
                &task_id,
                &definition,
                EnvMode::Strict,
                &graph.package_task_context(&package).unwrap(),
                &[],
                PackageTaskEventBuilder::new(package.as_str(), "build"),
            )
            .unwrap()
    }

    #[tokio::test]
    async fn task_hash_uses_identity_bound_context_and_rejects_mismatch() {
        let tmp = tempdir().unwrap();
        let repo_root =
            AbsoluteSystemPathBuf::new(tmp.path().to_string_lossy().to_string()).unwrap();
        let graph = javascript_graph(&repo_root).await;
        let package_name = PackageName::from("app");
        let package_context = graph.package_task_context(&package_name).unwrap();
        assert_eq!(package_context.kind(), PackageTaskContextKind::Package);
        let task_id = TaskId::new("app", "build");
        let definition = TaskDefinition::default();
        let opts = TestRunOpts {
            single_package: true,
        };
        let env = EnvironmentVariableMap::default();
        let hasher = task_hasher(&task_id, &opts, &env, &repo_root);

        let package_hash = hasher
            .calculate_task_hash(
                &task_id,
                &definition,
                EnvMode::Strict,
                &package_context,
                &[],
                PackageTaskEventBuilder::new("app", "build"),
            )
            .unwrap();
        assert!(!package_hash.is_empty());
        assert_eq!(
            package_context.directory().to_unix().as_str(),
            "packages/app"
        );

        let mismatch = hasher
            .calculate_task_hash(
                &task_id,
                &definition,
                EnvMode::Strict,
                &graph.package_task_context(&PackageName::Root).unwrap(),
                &[],
                PackageTaskEventBuilder::new("app", "build"),
            )
            .unwrap_err();
        assert!(matches!(mismatch, Error::TaskPackageMismatch { .. }));

        let monorepo_opts = TestRunOpts {
            single_package: false,
        };
        let error = task_hasher(&task_id, &monorepo_opts, &env, &repo_root)
            .calculate_task_hash(
                &task_id,
                &definition,
                EnvMode::Strict,
                &package_context,
                &[],
                PackageTaskEventBuilder::new("app", "build"),
            )
            .unwrap_err();
        assert!(
            matches!(error, Error::MissingExternalDependencyHash(name) if name == package_name)
        );
    }

    #[tokio::test]
    async fn task_hash_uses_resolution_knowledge_without_compatibility_payload() {
        let tmp = tempdir().unwrap();
        let repo_root =
            AbsoluteSystemPathBuf::new(tmp.path().to_string_lossy().to_string()).unwrap();
        let mut graph = javascript_graph(&repo_root).await;
        let package = PackageName::from("app");
        assert!(graph.remove_package_info_for_test(&package).is_some());
        let context = graph
            .package_task_context(&package)
            .expect("knowledge scope remains authoritative");
        assert!(context.package_info().is_none());

        let task_id = TaskId::new("app", "build");
        let definition = TaskDefinition::default();
        let opts = TestRunOpts {
            single_package: true,
        };
        let env = EnvironmentVariableMap::default();
        let hasher = task_hasher(&task_id, &opts, &env, &repo_root);
        let hash = hasher
            .calculate_task_hash(
                &task_id,
                &definition,
                EnvMode::Strict,
                &context,
                &[],
                PackageTaskEventBuilder::new("app", "build"),
            )
            .unwrap();

        assert!(!hash.is_empty());
        hasher
            .insert_deferred_hash(&task_id, &definition, EnvMode::Strict, &context)
            .unwrap();
        assert_eq!(
            compute_external_deps_hashes(&graph)
                .unwrap()
                .get(package.as_str())
                .map(String::as_str),
            Some("")
        );
    }

    #[tokio::test]
    async fn deferred_hash_rejects_context_from_another_repository() {
        let first_tmp = tempdir().unwrap();
        let first_root =
            AbsoluteSystemPathBuf::new(first_tmp.path().to_string_lossy().to_string()).unwrap();
        let first_graph = javascript_graph_at(&first_root, "packages").await;
        let second_tmp = tempdir().unwrap();
        let second_root =
            AbsoluteSystemPathBuf::new(second_tmp.path().to_string_lossy().to_string()).unwrap();
        let second_graph = javascript_graph_at(&second_root, "apps").await;
        let package = PackageName::from("app");
        let foreign_context = first_graph.package_task_context(&package).unwrap();
        assert_ne!(
            foreign_context.directory(),
            second_graph
                .package_task_context(&package)
                .unwrap()
                .directory()
        );

        let task_id = TaskId::new("app", "build");
        let definition = TaskDefinition::default();
        let opts = TestRunOpts {
            single_package: true,
        };
        let env = EnvironmentVariableMap::default();
        let hasher = task_hasher(&task_id, &opts, &env, &second_root);
        let regular_error = hasher
            .calculate_task_hash(
                &task_id,
                &definition,
                EnvMode::Strict,
                &foreign_context,
                &[],
                PackageTaskEventBuilder::new("app", "build"),
            )
            .unwrap_err();
        assert!(matches!(
            regular_error,
            Error::ContextRepositoryRootMismatch { .. }
        ));

        let error = hasher
            .calculate_task_hash_with_deferred_inputs(
                &task_id,
                &definition,
                EnvMode::Strict,
                &foreign_context,
                &[],
                PackageTaskEventBuilder::new("app", "build"),
                &SCM::new(&second_root),
                &second_root,
                None,
                None,
                &HashSet::new(),
            )
            .unwrap_err();

        assert!(matches!(error, Error::ContextRepositoryRootMismatch { .. }));
    }

    #[tokio::test]
    async fn file_hashing_hashes_only_requested_graph_scopes() {
        let tmp = tempdir().unwrap();
        let repo_root =
            AbsoluteSystemPathBuf::new(tmp.path().to_string_lossy().to_string()).unwrap();
        let graph = javascript_graph(&repo_root).await;
        repo_root
            .join_components(&["packages", "app", "input.txt"])
            .create_with_contents("app")
            .unwrap();
        repo_root
            .join_components(&["packages", "other", "input.txt"])
            .create_with_contents("other")
            .unwrap();

        let task_id = TaskId::new("app", "build");
        let tasks = [TaskNode::Task(task_id.clone())];
        let definitions = HashMap::from([(task_id.clone(), TaskDefinition::default())]);
        let hashes = PackageInputsHashes::calculate_file_hashes(
            &SCM::new(&repo_root),
            tasks.iter(),
            &graph,
            &definitions,
            &repo_root,
            &GenericEventBuilder::new(),
            None,
            true,
        )
        .unwrap();
        let expanded = hashes.expanded_hashes.get(&task_id).unwrap();

        assert!(
            expanded
                .0
                .iter()
                .any(|(path, _)| path.as_str() == "input.txt")
        );
        assert!(
            expanded
                .0
                .iter()
                .all(|(path, _)| !path.as_str().contains("other"))
        );
        assert_eq!(hashes.hashes.len(), 1);

        let other_tmp = tempdir().unwrap();
        let other_root =
            AbsoluteSystemPathBuf::new(other_tmp.path().to_string_lossy().to_string()).unwrap();
        let error = PackageInputsHashes::calculate_file_hashes(
            &SCM::new(&other_root),
            tasks.iter(),
            &graph,
            &definitions,
            &other_root,
            &GenericEventBuilder::new(),
            None,
            false,
        )
        .unwrap_err();
        assert!(matches!(error, Error::ContextRepositoryRootMismatch { .. }));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn file_hashing_supports_pure_cargo_root_turbo_namespace() {
        let tmp = tempdir().unwrap();
        // dunce: `cargo metadata` reports plain (non-verbatim) paths on
        // Windows, so the fixture root must be plain too.
        let repo_root = AbsoluteSystemPathBuf::new(
            dunce::canonicalize(tmp.path())
                .unwrap()
                .to_string_lossy()
                .to_string(),
        )
        .unwrap();
        let graph = cargo_graph(&repo_root).await;
        let baseline = context_hash(&graph, PackageName::Root, false);
        assert!(
            !graph
                .package_task_context(&PackageName::Root)
                .unwrap()
                .requires_compatibility_payload()
        );
        let task_id = TaskId::new("//", "build");
        let tasks = [TaskNode::Task(task_id.clone())];
        let definitions = HashMap::from([(task_id.clone(), TaskDefinition::default())]);

        let hashes = PackageInputsHashes::calculate_file_hashes(
            &SCM::new(&repo_root),
            tasks.iter(),
            &graph,
            &definitions,
            &repo_root,
            &GenericEventBuilder::new(),
            None,
            false,
        )
        .unwrap();

        assert!(hashes.hashes.contains_key(&task_id));
        let opts = TestRunOpts {
            single_package: false,
        };
        let env = EnvironmentVariableMap::default();
        task_hasher(&task_id, &opts, &env, &repo_root)
            .insert_deferred_hash(
                &task_id,
                definitions.get(&task_id).unwrap(),
                EnvMode::Strict,
                &graph.package_task_context(&PackageName::Root).unwrap(),
            )
            .unwrap();
        assert_eq!(context_hash(&graph, PackageName::Root, false), baseline);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn task_context_hash_compatibility_literals() {
        let js_tmp = tempdir().unwrap();
        let js_root =
            AbsoluteSystemPathBuf::new(js_tmp.path().to_string_lossy().to_string()).unwrap();
        let js_graph = javascript_graph(&js_root).await;

        let cargo_tmp = tempdir().unwrap();
        // dunce: `cargo metadata` reports plain (non-verbatim) paths on
        // Windows, so the fixture root must be plain too.
        let cargo_root = AbsoluteSystemPathBuf::new(
            dunce::canonicalize(cargo_tmp.path())
                .unwrap()
                .to_string_lossy()
                .to_string(),
        )
        .unwrap();
        let cargo_graph = cargo_graph(&cargo_root).await;
        let cargo_package = cargo_graph
            .package_scope_directories()
            .find_map(|(name, directory)| {
                (directory.to_unix().as_str() == "crates/app").then_some(name)
            })
            .expect("Cargo package context is discovered");
        let cargo_aggregate = cargo_graph
            .package_scope_directories()
            .find_map(|(name, _)| cargo_graph.is_aggregate_scope(&name).then_some(name))
            .expect("Cargo aggregate context is discovered");

        assert_eq!(
            context_hash(&js_graph, PackageName::Root, true),
            "f296efc7e9b4061a",
            "root JavaScript hash bytes changed"
        );
        assert_eq!(
            context_hash(&cargo_graph, PackageName::Root, true),
            "f296efc7e9b4061a",
            "pure Cargo root Turbo hash bytes changed"
        );
        assert_eq!(
            context_hash(&cargo_graph, cargo_package, true),
            "d4636fbf97ab13d4",
            "Cargo package hash bytes changed"
        );
        assert_eq!(
            context_hash(&cargo_graph, cargo_aggregate, true,),
            "f296efc7e9b4061a",
            "Cargo aggregate hash bytes changed"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn external_hash_precompute_preserves_compatibility_bytes() {
        let js_tmp = tempdir().unwrap();
        let js_root =
            AbsoluteSystemPathBuf::new(js_tmp.path().to_string_lossy().to_string()).unwrap();
        let js_graph = javascript_graph(&js_root).await;
        let js_cache = compute_external_deps_hashes(&js_graph).unwrap();
        assert_eq!(
            js_cache,
            HashMap::from([
                ("//".to_string(), String::new()),
                ("app".to_string(), String::new()),
                ("other".to_string(), String::new()),
            ])
        );

        let cargo_tmp = tempdir().unwrap();
        // dunce: `cargo metadata` reports plain (non-verbatim) paths on
        // Windows, so the fixture root must be plain too.
        let cargo_root = AbsoluteSystemPathBuf::new(
            dunce::canonicalize(cargo_tmp.path())
                .unwrap()
                .to_string_lossy()
                .to_string(),
        )
        .unwrap();
        let cargo_graph = cargo_graph(&cargo_root).await;
        let cargo_cache = compute_external_deps_hashes(&cargo_graph).unwrap();
        let (external_hash, app_task_hash, workspace_task_hash) = match std::env::consts::OS {
            "macos" => ("2ccf3983a6195c83", "16148055db78eed5", "3adbee17ca01f306"),
            "linux" => ("9fae73876995db4d", "bed5df30b6563a22", "a5d3d2445a0e2df2"),
            "windows" => ("538ddb6706883af6", "9d061b914e2d64aa", "af00aca4864ca739"),
            os => panic!("add exact Cargo compatibility hashes for {os}"),
        };
        assert_eq!(
            cargo_cache,
            HashMap::from([
                ("//".to_string(), String::new()),
                ("cargo-app".to_string(), external_hash.to_string()),
                ("cargo-workspace".to_string(), external_hash.to_string()),
            ])
        );

        for (graph, package, expected) in [
            (&js_graph, PackageName::Root, "f952e84c0fa1b4b7"),
            (&js_graph, PackageName::from("app"), "ba33476f1a197a76"),
            (&cargo_graph, PackageName::Root, "f952e84c0fa1b4b7"),
        ] {
            assert_eq!(monorepo_context_hash(graph, package), expected);
        }

        for (package, expected) in [
            ("cargo-app", app_task_hash),
            ("cargo-workspace", workspace_task_hash),
        ] {
            assert_eq!(
                monorepo_context_hash(&cargo_graph, PackageName::from(package)),
                expected
            );
        }
    }

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

    /// The linear hash of a pre-sorted closure must be byte-identical to the
    /// legacy path, which collected a `HashSet` and sorted by
    /// `(key, version)` before hashing.

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
