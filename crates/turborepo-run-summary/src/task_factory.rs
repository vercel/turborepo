use std::collections::HashSet;

use turbopath::AnchoredSystemPath;
use turborepo_env::EnvironmentVariableMap;
use turborepo_lockfiles::Package;
use turborepo_repository::package_graph::{PackageGraph, PackageInfo, PackageName};
use turborepo_task_id::TaskId;
use turborepo_types::{EnvMode, LOG_DIR, TaskDefinition, task_log_filename};

use crate::{
    EngineInfo, HashTrackerInfo, RunOptsInfo, TaskExecutionSummary,
    task::{
        SharedTaskSummary, SinglePackageTaskSummary, TaskCacheSummary, TaskEnvVarSummary,
        TaskSummary,
    },
};

pub struct TaskSummaryFactory<'a, E, H, R> {
    package_graph: &'a PackageGraph,
    engine: &'a E,
    hash_tracker: &'a H,
    env_at_start: &'a EnvironmentVariableMap,
    run_opts: &'a R,
    global_env_mode: EnvMode,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("No workspace found for {0}")]
    MissingWorkspace(String),
    #[error("No task definition found for {0}")]
    MissingTask(TaskId<'static>),
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
    ) -> Self {
        Self {
            package_graph,
            engine,
            hash_tracker,
            env_at_start,
            run_opts,
            global_env_mode,
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

    pub fn single_task_summary(
        &self,
        task_id: TaskId<'static>,
        execution: Option<TaskExecutionSummary>,
    ) -> Result<SinglePackageTaskSummary, Error> {
        let workspace_info = self.workspace_info(&task_id)?;
        let shared = self.shared(&task_id, execution, workspace_info, |dep_task_id| {
            Some(dep_task_id.task().to_string())
        })?;

        Ok(SinglePackageTaskSummary {
            task_id: task_id.task().to_string(),
            task: task_id.task().to_string(),
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
        // TODO: command should be optional
        let command = workspace_info
            .package_json
            .scripts
            .get(task_id.task())
            .map(|script| script.as_inner())
            .cloned()
            .unwrap_or_else(|| "<NONEXISTENT>".to_string());

        let task_definition = self.task_definition(task_id)?;

        let expanded_outputs = self
            .hash_tracker
            .expanded_outputs(task_id)
            .unwrap_or_default();

        let framework = self.hash_tracker.framework(task_id).unwrap_or_default();

        let hash = self
            .hash_tracker
            .hash(task_id)
            .unwrap_or_else(|| panic!("hash not found for {task_id}"));

        let expanded_inputs = self
            .hash_tracker
            .expanded_inputs(task_id)
            .expect("inputs not found");

        let env_vars = self
            .hash_tracker
            .env_vars(task_id)
            .expect("env var map is inserted at the same time as hash");

        let cache_summary = TaskCacheSummary::from(self.hash_tracker.cache_status(task_id));

        let (dependencies, dependents) = self.dependencies_and_dependents(task_id, display_task);

        let log_file = task_definition.cache.then(|| {
            let path = workspace_info.package_path().to_owned();
            let relative_log_file = workspace_relative_log_file(task_id.task());
            path.join(&relative_log_file).to_string()
        });

        let with = task_definition
            .with
            .as_ref()
            .map(|with| {
                with.iter()
                    .map(|task| task.as_inner().to_string())
                    .collect()
            })
            .unwrap_or_default();

        // Compute external deps hash from workspace info
        let hash_of_external_dependencies =
            get_external_deps_hash(&workspace_info.transitive_dependencies);

        Ok(SharedTaskSummary {
            hash,
            inputs: expanded_inputs.into_iter().collect(),
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
            )
            .expect("invalid glob in task definition should have been caught earlier"),
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
fn workspace_relative_log_file(task_name: &str) -> turbopath::AnchoredSystemPathBuf {
    let log_dir =
        AnchoredSystemPath::new(LOG_DIR).expect("LOG_DIR should be a valid AnchoredSystemPath");
    log_dir.join_component(&task_log_filename(task_name))
}

/// Computes a hash of external dependencies from transitive dependencies.
/// This is a pure function that doesn't require any trait access.
pub fn get_external_deps_hash(transitive_dependencies: &Option<HashSet<Package>>) -> String {
    use turborepo_hash::{LockFilePackages, TurboHash};

    let Some(transitive_dependencies) = transitive_dependencies else {
        return "".into();
    };

    let mut transitive_deps = Vec::with_capacity(transitive_dependencies.len());

    for dependency in transitive_dependencies.iter() {
        transitive_deps.push(dependency.clone());
    }

    transitive_deps.sort_by(|a, b| match a.key.cmp(&b.key) {
        std::cmp::Ordering::Equal => a.version.cmp(&b.version),
        other => other,
    });

    LockFilePackages(transitive_deps).hash()
}
