use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use rayon::prelude::*;
use serde::Serialize;
use thiserror::Error;
use tracing::{debug, Span};
use turbopath::{AbsoluteSystemPath, AnchoredSystemPath, AnchoredSystemPathBuf};
use turborepo_env::{BySource, DetailedMap, EnvironmentVariableMap, ResolvedEnvMode};
use turborepo_scm::SCM;

use crate::{
    engine::TaskNode,
    framework::infer_framework,
    hash::{FileHashes, TaskHashable, TurboHash},
    opts::Opts,
    package_graph::{WorkspaceInfo, WorkspaceName},
    run::task_id::TaskId,
    task_graph::TaskDefinition,
};

#[derive(Debug, Error)]
pub enum Error {
    #[error("missing pipeline entry {0}")]
    MissingPipelineEntry(TaskId<'static>),
    #[error("missing package.json for {0}")]
    MissingPackageJson(String),
    #[error("cannot find package-file hash for {0}")]
    MissingPackageFileHash(String),
    #[error("missing hash for dependent task {0}")]
    MissingDependencyTaskHash(String),
    #[error("cannot acquire lock for task hash tracker")]
    Mutex,
    #[error("missing environment variables for {0}")]
    MissingEnvVars(TaskId<'static>),
    #[error(transparent)]
    Scm(#[from] turborepo_scm::Error),
    #[error(transparent)]
    Env(#[from] turborepo_env::Error),
    #[error(transparent)]
    Regex(#[from] regex::Error),
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
}

impl TaskHashable<'_> {
    fn calculate_task_hash(mut self) -> String {
        if matches!(self.env_mode, ResolvedEnvMode::Loose) {
            self.pass_through_env = &[];
        }

        self.hash()
    }
}

#[derive(Debug, Default)]
pub struct PackageInputsHashes {
    hashes: HashMap<TaskId<'static>, String>,
    expanded_hashes: HashMap<TaskId<'static>, FileHashes>,
}

impl PackageInputsHashes {
    #[tracing::instrument(skip(all_tasks, workspaces, task_definitions, repo_root, scm))]
    pub fn calculate_file_hashes<'a>(
        scm: &SCM,
        all_tasks: impl ParallelIterator<Item = &'a TaskNode>,
        workspaces: HashMap<&WorkspaceName, &WorkspaceInfo>,
        task_definitions: &HashMap<TaskId<'static>, TaskDefinition>,
        repo_root: &AbsoluteSystemPath,
    ) -> Result<PackageInputsHashes, Error> {
        tracing::trace!(scm_manual=%scm.is_manual(), "scm running in {} mode", if scm.is_manual() { "manual" } else { "git" });

        let span = Span::current();

        let (hashes, expanded_hashes): (HashMap<_, _>, HashMap<_, _>) = all_tasks
            .filter_map(|task| {
                let span = tracing::info_span!(parent: &span, "calculate_file_hash", ?task);
                let _enter = span.enter();
                let TaskNode::Task(task_id) = task else {
                    return None;
                };

                let task_definition = match task_definitions
                    .get(task_id)
                    .ok_or_else(|| Error::MissingPipelineEntry(task_id.clone()))
                {
                    Ok(def) => def,
                    Err(err) => return Some(Err(err)),
                };

                let workspace_name = task_id.to_workspace_name();

                let pkg = match workspaces
                    .get(&workspace_name)
                    .ok_or_else(|| Error::MissingPackageJson(workspace_name.to_string()))
                {
                    Ok(pkg) => pkg,
                    Err(err) => return Some(Err(err)),
                };

                let package_path = pkg
                    .package_json_path
                    .parent()
                    .unwrap_or_else(|| AnchoredSystemPath::new("").unwrap());

                let mut hash_object = match scm.get_package_file_hashes(
                    repo_root,
                    package_path,
                    &task_definition.inputs,
                ) {
                    Ok(hash_object) => hash_object,
                    Err(err) => return Some(Err(err.into())),
                };

                if !task_definition.dot_env.is_empty() {
                    let absolute_package_path = repo_root.resolve(package_path);
                    let dot_env_object = match scm.hash_existing_of(
                        &absolute_package_path,
                        task_definition
                            .dot_env
                            .iter()
                            .map(|p| p.to_anchored_system_path_buf()),
                    ) {
                        Ok(dot_env_object) => dot_env_object,
                        Err(err) => return Some(Err(err.into())),
                    };

                    for (key, value) in dot_env_object {
                        hash_object.insert(key, value);
                    }
                }

                let file_hashes = FileHashes(hash_object);
                let hash = file_hashes.clone().hash();

                Some(Ok((
                    (task_id.clone(), hash),
                    (task_id.clone(), file_hashes),
                )))
            })
            .collect::<Result<_, _>>()?;

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
    package_task_framework: HashMap<TaskId<'static>, String>,
    #[serde(skip)]
    package_task_outputs: HashMap<TaskId<'static>, Vec<AnchoredSystemPathBuf>>,
}

/// Caches package-inputs hashes, and package-task hashes.
pub struct TaskHasher<'a> {
    package_inputs_hashes: PackageInputsHashes,
    opts: &'a Opts<'a>,
    env_at_execution_start: &'a EnvironmentVariableMap,
    global_hash: &'a str,
    task_hash_tracker: TaskHashTracker,
}

impl<'a> TaskHasher<'a> {
    pub fn new(
        package_inputs_hashes: PackageInputsHashes,
        opts: &'a Opts,
        env_at_execution_start: &'a EnvironmentVariableMap,
        global_hash: &'a str,
    ) -> Self {
        Self {
            package_inputs_hashes,
            opts,
            env_at_execution_start,
            global_hash,
            task_hash_tracker: Default::default(),
        }
    }

    #[tracing::instrument(skip(self, task_definition, task_env_mode, workspace, dependency_set))]
    pub fn calculate_task_hash(
        &self,
        task_id: &TaskId<'static>,
        task_definition: &TaskDefinition,
        task_env_mode: ResolvedEnvMode,
        workspace: &WorkspaceInfo,
        dependency_set: HashSet<&TaskNode>,
    ) -> Result<String, Error> {
        let do_framework_inference = self.opts.run_opts.framework_inference;
        let is_monorepo = !self.opts.run_opts.single_package;

        let hash_of_files = self
            .package_inputs_hashes
            .hashes
            .get(task_id)
            .ok_or_else(|| Error::MissingPackageFileHash(task_id.to_string()))?;
        let mut explicit_env_var_map = EnvironmentVariableMap::default();
        let mut all_env_var_map = EnvironmentVariableMap::default();
        let mut matching_env_var_map = EnvironmentVariableMap::default();

        if do_framework_inference {
            // See if we infer a framework
            if let Some(framework) = infer_framework(workspace, is_monorepo) {
                debug!("auto detected framework for {}", task_id.package());
                debug!(
                    "framework: {}, env_prefix: {:?}",
                    framework.slug(),
                    framework.env_wildcards()
                );
                let mut computed_wildcards = framework
                    .env_wildcards()
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>();

                if let Some(exclude_prefix) =
                    self.env_at_execution_start.get("TURBO_CI_VENDOR_ENV_KEY")
                {
                    if !exclude_prefix.is_empty() {
                        let computed_exclude = format!("!{}*", exclude_prefix);
                        debug!(
                            "excluding environment variables matching wildcard {}",
                            computed_exclude
                        );
                        computed_wildcards.push(computed_exclude);
                    }
                }

                let inference_env_var_map = self
                    .env_at_execution_start
                    .from_wildcards(&computed_wildcards)?;

                let user_env_var_set = self
                    .env_at_execution_start
                    .wildcard_map_from_wildcards_unresolved(&task_definition.env)?;

                all_env_var_map.union(&user_env_var_set.inclusions);
                all_env_var_map.union(&inference_env_var_map);
                all_env_var_map.difference(&user_env_var_set.exclusions);

                explicit_env_var_map.union(&user_env_var_set.inclusions);
                explicit_env_var_map.difference(&user_env_var_set.exclusions);

                matching_env_var_map.union(&inference_env_var_map);
                matching_env_var_map.difference(&user_env_var_set.exclusions);
            } else {
                all_env_var_map = self
                    .env_at_execution_start
                    .from_wildcards(&task_definition.env)?;

                explicit_env_var_map.union(&all_env_var_map);
            }
        } else {
            all_env_var_map = self
                .env_at_execution_start
                .from_wildcards(&task_definition.env)?;

            explicit_env_var_map.union(&all_env_var_map);
        }

        let env_vars = DetailedMap {
            all: all_env_var_map,
            by_source: BySource {
                explicit: explicit_env_var_map,
                matching: matching_env_var_map,
            },
        };

        let hashable_env_pairs = env_vars.all.to_hashable();
        let outputs = task_definition.hashable_outputs(task_id);
        let task_dependency_hashes = self.calculate_dependency_hashes(dependency_set)?;
        let external_deps_hash = is_monorepo.then(|| workspace.get_external_deps_hash());

        debug!(
            "task hash env vars for {}:{}\n vars: {:?}",
            task_id.package(),
            task_id.task(),
            hashable_env_pairs
        );

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

            pass_through_args: self.opts.run_opts.pass_through_args,
            env: &task_definition.env,
            resolved_env_vars: hashable_env_pairs,
            pass_through_env: task_definition
                .pass_through_env
                .as_deref()
                .unwrap_or_default(),
            env_mode: task_env_mode,
            dot_env: &task_definition.dot_env,
        };

        let task_hash = task_hashable.calculate_task_hash();

        self.task_hash_tracker
            .insert_hash(task_id.clone(), env_vars, task_hash.clone());

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
        dependency_hash_list.sort();

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

    pub fn env(
        &self,
        task_id: &TaskId,
        task_env_mode: ResolvedEnvMode,
        task_definition: &TaskDefinition,
        global_env: &EnvironmentVariableMap,
    ) -> Result<EnvironmentVariableMap, Error> {
        match task_env_mode {
            ResolvedEnvMode::Strict => {
                let mut pass_through_env = EnvironmentVariableMap::default();
                let default_env_var_pass_through_map = self
                    .env_at_execution_start
                    .from_wildcards(&["PATH", "SHELL", "SYSTEMROOT"])?;
                let tracker_env = self
                    .task_hash_tracker
                    .env_vars(task_id)
                    .ok_or_else(|| Error::MissingEnvVars(task_id.clone().into_owned()))?;

                pass_through_env.union(&default_env_var_pass_through_map);
                pass_through_env.union(global_env);
                pass_through_env.union(&tracker_env.all);

                if let Some(definition_pass_through) = &task_definition.pass_through_env {
                    let env_var_pass_through_map = self
                        .env_at_execution_start
                        .from_wildcards(definition_pass_through)?;
                    pass_through_env.union(&env_var_pass_through_map);
                }

                Ok(pass_through_env)
            }
            ResolvedEnvMode::Loose => Ok(self.env_at_execution_start.clone()),
        }
    }
}

impl TaskHashTracker {
    // We only want hash to be queried and set from the TaskHasher
    fn hash(&self, task_id: &TaskId) -> Option<String> {
        let state = self.state.lock().expect("hash tracker mutex poisoned");
        state.package_task_hashes.get(task_id).cloned()
    }

    fn insert_hash(&self, task_id: TaskId<'static>, env_vars: DetailedMap, hash: String) {
        let mut state = self.state.lock().expect("hash tracker mutex poisoned");
        state
            .package_task_env_vars
            .insert(task_id.clone(), env_vars);
        state.package_task_hashes.insert(task_id, hash);
    }

    pub fn env_vars(&self, task_id: &TaskId) -> Option<DetailedMap> {
        let state = self.state.lock().expect("hash tracker mutex poisoned");
        state.package_task_env_vars.get(task_id).cloned()
    }

    pub fn framework(&self, task_id: &TaskId) -> Option<String> {
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
