use std::collections::HashSet;

use turborepo_env::EnvironmentVariableMap;

use super::{
    execution::TaskExecutionSummary,
    task::{SharedTaskSummary, TaskEnvVarSummary},
    SingleTaskSummary, TaskSummary,
};
use crate::{
    cli::EnvMode,
    engine::{Engine, TaskNode},
    opts::RunOpts,
    package_graph::{PackageGraph, WorkspaceInfo, WorkspaceName},
    run::task_id::TaskId,
    task_graph::TaskDefinition,
    task_hash::TaskHashTracker,
};

pub struct TaskSummaryFactory<'a> {
    package_graph: &'a PackageGraph,
    engine: &'a Engine,
    hash_tracker: TaskHashTracker,
    env_at_start: &'a EnvironmentVariableMap,
    run_opts: &'a RunOpts<'a>,
    global_env_mode: EnvMode,
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
        run_opts: &'a RunOpts<'a>,
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
        let shared = self.shared(&task_id, execution, workspace_info)?;
        let package = task_id.package().to_string();
        let (dependencies, dependents) =
            self.dependencies_and_dependents(&task_id, |task_node| match task_node {
                crate::engine::TaskNode::Task(task) => Some(task.clone()),
                crate::engine::TaskNode::Root => None,
            });

        Ok(TaskSummary {
            task_id,
            dir: workspace_info.package_path().to_string(),
            package,
            dependencies,
            dependents,
            shared,
        })
    }

    pub fn single_task_summary(
        &self,
        task_id: TaskId<'static>,
        execution: Option<TaskExecutionSummary>,
    ) -> Result<SingleTaskSummary, Error> {
        let workspace_info = self.workspace_info(&task_id)?;
        let shared = self.shared(&task_id, execution, workspace_info)?;

        let (dependencies, dependents) =
            self.dependencies_and_dependents(&task_id, |task_node| match task_node {
                crate::engine::TaskNode::Task(task) => Some(task.task().to_string()),
                crate::engine::TaskNode::Root => None,
            });

        Ok(SingleTaskSummary {
            task_id: task_id.task().to_string(),
            dependencies,
            dependents,
            shared,
        })
    }

    fn shared(
        &self,
        task_id: &TaskId<'static>,
        execution: Option<TaskExecutionSummary>,
        workspace_info: &WorkspaceInfo,
    ) -> Result<SharedTaskSummary, Error> {
        // TODO: command should be optional
        let command = workspace_info
            .package_json
            .scripts
            .get(task_id.task())
            .cloned()
            .unwrap_or_default();

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

        Ok(SharedTaskSummary {
            hash,
            expanded_inputs,
            external_deps_hash: workspace_info.get_external_deps_hash(),
            cache_summary,
            command,
            command_arguments: self.run_opts.pass_through_args.to_vec(),
            outputs: task_definition.outputs.inclusions.clone(),
            excluded_outputs: task_definition.outputs.exclusions.clone(),
            log_file_relative_path: workspace_info.task_log_path(task_id).to_string(),
            resolved_task_definition: task_definition.clone(),
            expanded_outputs,
            framework,
            // TODO: this is some very messy code that appears in a few places
            // we should attempt to calculate this once and reuse it
            env_mode: match self.global_env_mode {
                EnvMode::Infer => {
                    if task_definition.pass_through_env.is_some() {
                        EnvMode::Strict
                    } else {
                        // If we're in infer mode we have just detected non-usage of strict env
                        // vars. But our behavior's actual meaning of this
                        // state is `loose`.
                        EnvMode::Loose
                    }
                }
                EnvMode::Strict => EnvMode::Strict,
                EnvMode::Loose => EnvMode::Loose,
            },
            env_vars: TaskEnvVarSummary::new(task_definition, env_vars, self.env_at_start)
                .expect("invalid glob in task definition should have been caught earlier"),
            dot_env: task_definition.dot_env.clone(),
            execution,
        })
    }

    fn workspace_info(&self, task_id: &TaskId) -> Result<&WorkspaceInfo, Error> {
        let workspace_name = WorkspaceName::from(task_id.package());
        self.package_graph
            .workspace_info(&workspace_name)
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
        f: impl Fn(&TaskNode) -> Option<T> + Copy,
    ) -> (Vec<T>, Vec<T>) {
        let collect_nodes = |set: Option<HashSet<&TaskNode>>| {
            set.unwrap_or_default()
                .into_iter()
                .filter_map(f)
                .collect::<Vec<_>>()
        };
        let dependencies = collect_nodes(self.engine.dependencies(task_id));
        let dependents = collect_nodes(self.engine.dependents(task_id));
        (dependencies, dependents)
    }
}
