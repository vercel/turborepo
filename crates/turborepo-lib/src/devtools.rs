//! Devtools integration for turborepo-lib.
//!
//! This module provides the proper task graph building implementation
//! for the devtools server, using the same logic as `turbo run`.

use std::{future::Future, pin::Pin};

use tracing::debug;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_devtools::{GraphEdge, TaskGraphBuilder, TaskGraphData, TaskGraphError, TaskNode};
use turborepo_repository::{
    package_graph::{PackageGraph, PackageGraphBuilder, PackageName},
    package_json::PackageJson,
};
use turborepo_task_id::TaskName;

use crate::{
    config::CONFIG_FILE,
    engine::{EngineBuilder, TaskNode as EngineTaskNode},
    turbo_json::{TurboJsonLoader, TurboJsonReader},
};

/// Task graph builder that uses the proper `EngineBuilder` logic.
///
/// This implementation builds task graphs using the same logic as `turbo run`,
/// ensuring consistency between what the devtools shows and what turbo actually
/// executes.
pub struct ProperTaskGraphBuilder {
    repo_root: AbsoluteSystemPathBuf,
}

impl ProperTaskGraphBuilder {
    /// Create a new proper task graph builder
    pub fn new(repo_root: AbsoluteSystemPathBuf) -> Self {
        Self { repo_root }
    }

    /// Build the package graph for the repository
    async fn build_package_graph(&self) -> Result<PackageGraph, TaskGraphError> {
        let root_package_json_path = self.repo_root.join_component("package.json");
        let root_package_json = PackageJson::load(&root_package_json_path)
            .map_err(|e| TaskGraphError::BuildError(format!("Failed to load package.json: {e}")))?;

        PackageGraphBuilder::new(&self.repo_root, root_package_json)
            .with_allow_no_package_manager(true)
            .build()
            .await
            .map_err(|e| TaskGraphError::BuildError(format!("Failed to build package graph: {e}")))
    }

    /// Build the task graph using EngineBuilder
    fn build_engine_task_graph(
        &self,
        pkg_graph: &PackageGraph,
    ) -> Result<TaskGraphData, TaskGraphError> {
        // Create turbo json loader
        let root_turbo_json_path = self.repo_root.join_component(CONFIG_FILE);
        let reader = TurboJsonReader::new(self.repo_root.clone());
        let loader =
            TurboJsonLoader::workspace(reader, root_turbo_json_path.clone(), pkg_graph.packages());

        // Determine if this is a single package repo
        let is_single = pkg_graph.len() == 1;

        // Collect all workspaces
        let workspaces: Vec<PackageName> =
            pkg_graph.packages().map(|(name, _)| name.clone()).collect();

        // Collect all root tasks from turbo.json AND root package.json scripts
        // For devtools, we want to show all tasks including root tasks
        let mut root_tasks: Vec<TaskName<'static>> = loader
            .load(&PackageName::Root)
            .map(|turbo_json| {
                turbo_json
                    .tasks
                    .keys()
                    .map(|name| name.clone().into_owned())
                    .collect()
            })
            .unwrap_or_default();

        // Also add all scripts from root package.json as potential root tasks
        // This ensures tasks like //#build:ts are allowed even if not in turbo.json
        if let Some(root_pkg_json) = pkg_graph.package_json(&PackageName::Root) {
            for script_name in root_pkg_json.scripts.keys() {
                let task_name = TaskName::from(format!("//#{}", script_name)).into_owned();
                if !root_tasks.contains(&task_name) {
                    root_tasks.push(task_name);
                }
            }
        }

        // Build engine with all tasks
        // We use `add_all_tasks` to get the complete task graph for visualization
        let engine = EngineBuilder::new(&self.repo_root, pkg_graph, &loader, is_single)
            .with_workspaces(workspaces)
            .with_root_tasks(root_tasks)
            .add_all_tasks()
            .do_not_validate_engine() // Don't validate for devtools visualization
            .build()
            .map_err(|e| TaskGraphError::BuildError(format!("Failed to build task graph: {e}")))?;

        // Convert engine to TaskGraphData
        let mut nodes = Vec::new();
        let mut edges = Vec::new();

        // Collect task nodes
        for task_node in engine.tasks() {
            match task_node {
                EngineTaskNode::Root => {
                    // Skip the synthetic root node in the output
                }
                EngineTaskNode::Task(task_id) => {
                    let package = task_id.package().to_string();
                    let task = task_id.task().to_string();
                    let id = task_id.to_string();

                    // Get script from package.json
                    let script = pkg_graph
                        .package_json(&PackageName::from(task_id.package()))
                        .and_then(|pj| pj.scripts.get(task_id.task()))
                        .map(|s| s.value.clone())
                        .unwrap_or_default();

                    nodes.push(TaskNode {
                        id,
                        package,
                        task,
                        script,
                    });
                }
            }
        }

        // Collect edges from dependencies
        for task_node in engine.tasks() {
            if let EngineTaskNode::Task(task_id) = task_node {
                if let Some(deps) = engine.dependencies(task_id) {
                    for dep in deps {
                        if let EngineTaskNode::Task(dep_id) = dep {
                            edges.push(GraphEdge {
                                source: task_id.to_string(),
                                target: dep_id.to_string(),
                            });
                        }
                        // Skip edges to Root node
                    }
                }
            }
        }

        debug!(
            "Built task graph with {} nodes and {} edges",
            nodes.len(),
            edges.len()
        );

        Ok(TaskGraphData { nodes, edges })
    }
}

impl TaskGraphBuilder for ProperTaskGraphBuilder {
    fn build_task_graph(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<TaskGraphData, TaskGraphError>> + Send + '_>> {
        Box::pin(async move {
            let pkg_graph = self.build_package_graph().await?;
            self.build_engine_task_graph(&pkg_graph)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests would go here - we can verify that the ProperTaskGraphBuilder
    // produces the same results as a real turbo run would
}
