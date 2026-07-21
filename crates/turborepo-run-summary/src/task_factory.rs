use std::collections::HashMap;

use turbopath::AnchoredSystemPath;
use turborepo_env::EnvironmentVariableMap;
use turborepo_lockfiles::Package;
use turborepo_repository::package_graph::{PackageGraph, PackageInfo, PackageName};
use turborepo_task_id::TaskId;
use turborepo_types::{
    EngineInfo, EnvMode, HashTrackerInfo, LOG_DIR, RunOptsInfo, TaskDefinition, task_log_filename,
};

use crate::{
    TaskExecutionSummary,
    task::{SharedTaskSummary, TaskCacheSummary, TaskEnvVarSummary, TaskSummary},
};

pub struct TaskSummaryFactory<'a, E, H, R> {
    package_graph: &'a PackageGraph,
    engine: &'a E,
    hash_tracker: &'a H,
    env_at_start: &'a EnvironmentVariableMap,
    run_opts: &'a R,
    global_env_mode: EnvMode,
    /// Per-package external dependency hashes computed during task hashing.
    /// When present, summaries reuse these instead of re-sorting and
    /// re-hashing each package's transitive closure per task.
    external_deps_hashes: Option<&'a HashMap<String, String>>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("No workspace found for {0}")]
    MissingWorkspace(String),
    #[error("No task definition found for {0}")]
    MissingTask(TaskId<'static>),
    #[error("No task hash found for {0}")]
    MissingHash(TaskId<'static>),
    #[error("No expanded inputs found for {0}")]
    MissingExpandedInputs(TaskId<'static>),
    #[error("No environment variables found for {0}")]
    MissingEnvVars(TaskId<'static>),
    #[error(transparent)]
    Env(#[from] turborepo_env::Error),
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
}

impl<'a, E, H, R> TaskSummaryFactory<'a, E, H, R>
where
    E: EngineInfo,
    H: HashTrackerInfo,
    R: RunOptsInfo,
{
    pub fn new(
        package_graph: &'a PackageGraph,
        engine: &'a E,
        hash_tracker: &'a H,
        env_at_start: &'a EnvironmentVariableMap,
        run_opts: &'a R,
        global_env_mode: EnvMode,
        external_deps_hashes: Option<&'a HashMap<String, String>>,
    ) -> Self {
        Self {
            package_graph,
            engine,
            hash_tracker,
            env_at_start,
            run_opts,
            global_env_mode,
            external_deps_hashes,
        }
    }

    pub fn task_summary(
        &self,
        task_id: TaskId<'static>,
        execution: Option<TaskExecutionSummary>,
    ) -> Result<TaskSummary, Error> {
        let workspace_info = self.workspace_info(&task_id)?;
        let shared = self.shared(&task_id, execution, workspace_info, |dep_task_id| {
            Some(dep_task_id.clone())
        })?;
        let package = task_id.package().to_string();
        let task = task_id.task().to_string();

        Ok(TaskSummary {
            task_id,
            task,
            package,
            shared,
        })
    }

    fn shared<T>(
        &self,
        task_id: &TaskId<'static>,
        execution: Option<TaskExecutionSummary>,
        workspace_info: &PackageInfo,
        display_task: impl Fn(&TaskId<'static>) -> Option<T> + Copy,
    ) -> Result<SharedTaskSummary<T>, Error> {
        let task_definition = self.task_definition(task_id)?;

        // TODO: command should be optional
        // A resolved `command` override displays as its literal argv —
        // truthful by construction. Otherwise the package's toolchain owns
        // the display string (JavaScript: the script text; Cargo: the cargo
        // invocation), derived from the same tables as execution so display
        // cannot drift from what runs.
        let command = match &task_definition.command {
            Some(turborepo_types::TaskCommandOverride::Argv(argv)) => argv.join(" "),
            Some(turborepo_types::TaskCommandOverride::OptOut) => "<OPT OUT>".to_string(),
            None => self
                .package_graph
                .toolchains()
                .get(&workspace_info.toolchain)
                .and_then(|toolchain| {
                    toolchain.task_display_command(workspace_info, task_id.task())
                })
                .unwrap_or_else(|| "<NONEXISTENT>".to_string()),
        };

        let expanded_outputs = self
            .hash_tracker
            .expanded_outputs(task_id)
            .unwrap_or_default();

        let framework = self.hash_tracker.framework(task_id).unwrap_or_default();

        let hash = self
            .hash_tracker
            .hash(task_id)
            .ok_or_else(|| Error::MissingHash(task_id.clone()))?;
        let hash_is_deferred = matches!(
            hash.as_ref(),
            "Deferred because JIT hashing mode was used."
                | "Deferred because dependencyOutputs hashing mode was used."
        );
        let hash_reason = hash_is_deferred.then(|| hash.to_string());
        let hash = (!hash_is_deferred).then_some(hash);

        let expanded_inputs: std::collections::BTreeMap<_, _> = self
            .hash_tracker
            .expanded_inputs(task_id)
            .ok_or_else(|| Error::MissingExpandedInputs(task_id.clone()))?
            .into_iter()
            .collect();

        let env_vars = self
            .hash_tracker
            .env_vars(task_id)
            .ok_or_else(|| Error::MissingEnvVars(task_id.clone()))?;

        let cache_summary = TaskCacheSummary::from(self.hash_tracker.cache_status(task_id));

        let (dependencies, dependents) = self.dependencies_and_dependents(task_id, display_task);

        let log_file = if task_definition.cache {
            let path = workspace_info.package_path().to_owned();
            let relative_log_file = workspace_relative_log_file(task_id.task())?;
            Some(path.join(&relative_log_file).to_string())
        } else {
            None
        };

        let with = task_definition
            .with
            .as_ref()
            .map(|with| {
                with.iter()
                    .map(|task| task.as_inner().to_string())
                    .collect()
            })
            .unwrap_or_default();

        // The hash is precomputed where the closure is computed; the
        // per-run cache and the recompute below only apply to graphs built
        // without a closure hasher. All three produce identical output.
        let hash_of_external_dependencies = workspace_info
            .external_deps_hash
            .clone()
            .or_else(|| {
                self.external_deps_hashes
                    .and_then(|hashes| hashes.get(task_id.package()).cloned())
            })
            .unwrap_or_else(|| get_external_deps_hash(&workspace_info.transitive_dependencies));

        Ok(SharedTaskSummary {
            hash,
            hash_reason,
            inputs: expanded_inputs,
            hash_of_external_dependencies,
            cache: cache_summary,
            command,
            cli_arguments: self.run_opts.pass_through_args().to_vec(),
            outputs: match task_definition.outputs.inclusions.is_empty() {
                false => Some(task_definition.outputs.inclusions.clone()),
                true => None,
            },
            excluded_outputs: match task_definition.outputs.exclusions.is_empty() {
                true => None,
                false => Some(task_definition.outputs.exclusions.clone()),
            },
            log_file,
            directory: Some(workspace_info.package_path().to_string()),
            resolved_task_definition: task_definition.clone().into(),
            expanded_outputs,
            framework,
            dependencies,
            dependents,
            with,
            env_mode: self.global_env_mode,
            environment_variables: TaskEnvVarSummary::from_hash_tracker(
                task_definition,
                env_vars,
                self.env_at_start,
            )?,
            execution,
        })
    }

    fn workspace_info(&self, task_id: &TaskId) -> Result<&PackageInfo, Error> {
        let workspace_name = PackageName::from(task_id.package());
        self.package_graph
            .package_info(&workspace_name)
            .ok_or_else(|| Error::MissingWorkspace(workspace_name.to_string()))
    }

    fn task_definition(&self, task_id: &TaskId<'static>) -> Result<&TaskDefinition, Error> {
        self.engine
            .task_definition(task_id)
            .ok_or_else(|| Error::MissingTask(task_id.clone().into_owned()))
    }

    fn dependencies_and_dependents<T>(
        &self,
        task_id: &TaskId<'static>,
        display_node: impl Fn(&TaskId<'static>) -> Option<T> + Copy,
    ) -> (Vec<T>, Vec<T>) {
        let collect_nodes = |iter: Option<E::TaskIter<'_>>| {
            iter.map(|iter| iter.filter_map(display_node).collect::<Vec<_>>())
                .unwrap_or_default()
        };
        let dependencies = collect_nodes(self.engine.dependencies(task_id));
        let dependents = collect_nodes(self.engine.dependents(task_id));
        (dependencies, dependents)
    }
}

/// Get the workspace-relative path to the log file for a task.
fn workspace_relative_log_file(
    task_name: &str,
) -> Result<turbopath::AnchoredSystemPathBuf, turbopath::PathError> {
    let log_dir = AnchoredSystemPath::new(LOG_DIR)?;
    Ok(log_dir.join_component(&task_log_filename(task_name)))
}

/// Computes a hash of external dependencies from a workspace's sorted
/// transitive dependency closure. The closure is already sorted by
/// `Package`'s `(key, version)` ordering, so no re-sort is needed.
pub fn get_external_deps_hash(
    transitive_dependencies: &Option<Vec<std::sync::Arc<Package>>>,
) -> String {
    use turborepo_hash::{LockFilePackagesRef, TurboHash};

    let Some(transitive_dependencies) = transitive_dependencies else {
        return "".into();
    };

    let transitive_deps: Vec<&Package> = transitive_dependencies.iter().map(|pkg| &**pkg).collect();

    LockFilePackagesRef(transitive_deps).hash()
}
