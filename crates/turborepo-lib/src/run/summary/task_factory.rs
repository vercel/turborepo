use std::collections::HashSet;

use turborepo_env::EnvironmentVariableMap;
use turborepo_repository::package_graph::{PackageGraph, PackageInfo, PackageName};

use super::{
    execution::TaskExecutionSummary,
    task::{SharedTaskSummary, TaskEnvVarSummary},
    SinglePackageTaskSummary, TaskSummary,
};
use crate::{
    cli,
    engine::{Engine, TaskNode},
    opts::RunOpts,
    run::task_id::TaskId,
    task_graph::TaskDefinition,
    task_hash::{get_external_deps_hash, TaskHashTracker},
};

pub struct TaskSummaryFactory<'a> {
    package_graph: &'a PackageGraph,
    engine: &'a Engine,
    hash_tracker: TaskHashTracker,
    env_at_start: &'a EnvironmentVariableMap,
    run_opts: &'a RunOpts,
    global_env_mode: cli::EnvMode,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no workspace found for {0}")]
    MissingWorkspace(String),
    #[error("no task definition found for {0}")]
    MissingTask(TaskId<'static>),
}

impl<'a> TaskSummaryFactory<'a> {
    pub fn new(
        package_graph: &'a PackageGraph,
        engine: &'a Engine,
        hash_tracker: TaskHashTracker,
        env_at_start: &'a EnvironmentVariableMap,
        run_opts: &'a RunOpts,
        global_env_mode: cli::EnvMode,
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
        let shared = self.shared(
            &task_id,
            execution,
            workspace_info,
            |task_node| match task_node {
                crate::engine::TaskNode::Task(task) => Some(task.clone()),
                crate::engine::TaskNode::Root => None,
            },
        )?;
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
        let shared = self.shared(
            &task_id,
            execution,
            workspace_info,
            |task_node| match task_node {
                crate::engine::TaskNode::Task(task) => Some(task.task().to_string()),
                crate::engine::TaskNode::Root => None,
            },
        )?;

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
        display_task: impl Fn(&TaskNode) -> Option<T> + Copy,
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
            .get_expanded_inputs(task_id)
            .expect("inputs not found")
            .0;

        let env_vars = self
            .hash_tracker
            .env_vars(task_id)
            .expect("env var map is inserted at the same time as hash");

        let cache_summary = self.hash_tracker.cache_status(task_id).into();

        let (dependencies, dependents) = self.dependencies_and_dependents(task_id, display_task);

        let log_file = {
            let path = workspace_info.package_path().to_owned();
            let relative_log_file = TaskDefinition::workspace_relative_log_file(task_id.task());
            path.join(&relative_log_file).to_string()
        };

        Ok(SharedTaskSummary {
            hash,
            inputs: expanded_inputs.into_iter().collect(),
            hash_of_external_dependencies: get_external_deps_hash(
                &workspace_info.transitive_dependencies,
            ),
            cache: cache_summary,
            command,
            cli_arguments: self.run_opts.pass_through_args.to_vec(),
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
            env_mode: self.global_env_mode,
            environment_variables: TaskEnvVarSummary::new(
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
        task_id: &TaskId,
        display_node: impl Fn(&TaskNode) -> Option<T> + Copy,
    ) -> (Vec<T>, Vec<T>) {
        let collect_nodes = |set: Option<HashSet<&TaskNode>>| {
            set.unwrap_or_default()
                .into_iter()
                .filter_map(display_node)
                .collect::<Vec<_>>()
        };
        let dependencies = collect_nodes(self.engine.dependencies(task_id));
        let dependents = collect_nodes(self.engine.dependents(task_id));
        (dependencies, dependents)
    }
}
