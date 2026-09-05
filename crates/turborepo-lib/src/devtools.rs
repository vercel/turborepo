//! Devtools integration for turborepo-lib.
//!
//! This module provides the proper task graph building implementation
//! for the devtools server, using the same logic as `turbo run`.

use std::{future::Future, pin::Pin};

use tracing::debug;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_devtools::{
    package_graph_to_data, GraphData, GraphEdge, RepositoryGraphBuilder, TaskGraphData,
    TaskGraphError, TaskNode,
};
use turborepo_repository::package_graph::{PackageGraph, PackageGraphBuilder, PackageName};
use turborepo_task_id::TaskName;

use crate::{
    commands::CommandBase,
    engine::{EngineBuilder, TaskNode as EngineTaskNode},
    opts::Opts,
    repository_graph::RepositoryGraphFeatures,
    turbo_json::{TurboJsonReader, UnifiedTurboJsonLoader},
    Args,
};

/// Task graph builder that uses the proper `EngineBuilder` logic.
///
/// This implementation builds task graphs using the same logic as `turbo run`,
/// ensuring consistency between what the devtools shows and what turbo actually
/// executes.
pub struct ProperTaskGraphBuilder {
    repo_root: AbsoluteSystemPathBuf,
    args: Args,
}

impl ProperTaskGraphBuilder {
    /// Create a new proper task graph builder
    pub fn new(repo_root: AbsoluteSystemPathBuf, args: Args) -> Self {
        Self { repo_root, args }
    }

    fn resolve_opts(&self) -> Result<Opts, TaskGraphError> {
        let config = CommandBase::load_config(&self.repo_root, &self.args)
            .map_err(|error| TaskGraphError::BuildError(error.to_string()))?;
        let root_turbo_json_path = config
            .root_turbo_json_path(&self.repo_root)
            .map_err(|error| TaskGraphError::BuildError(error.to_string()))?;
        if self.repo_root.anchor(&root_turbo_json_path).is_err() {
            return Err(TaskGraphError::BuildError(format!(
                "Devtools root Turbo config must be inside the repository: {} is outside {}",
                root_turbo_json_path, self.repo_root
            )));
        }
        Opts::new(&self.repo_root, &self.args, config)
            .map_err(|error| TaskGraphError::BuildError(error.to_string()))
    }

    /// Build the package graph for the repository
    async fn build_package_graph(&self, opts: &Opts) -> Result<PackageGraph, TaskGraphError> {
        let features = RepositoryGraphFeatures::new(&opts.future_flags);
        let root_package_json = features
            .load_root_package_json(&self.repo_root)
            .map_err(|e| TaskGraphError::BuildError(format!("Failed to load package.json: {e}")))?;

        let builder = PackageGraphBuilder::new_optional(&self.repo_root, root_package_json)
            .with_single_package_mode(opts.run_opts.single_package)
            .with_allow_no_package_manager(opts.repo_opts.allow_no_package_manager);

        features
            .configure(builder)
            .build()
            .await
            .map_err(|e| TaskGraphError::BuildError(format!("Failed to build package graph: {e}")))
    }

    /// Build the task graph using EngineBuilder
    fn build_engine_task_graph(
        &self,
        pkg_graph: &PackageGraph,
        opts: &Opts,
    ) -> Result<TaskGraphData, TaskGraphError> {
        // Create turbo json loader
        let reader =
            TurboJsonReader::new(self.repo_root.clone()).with_future_flags(opts.future_flags);
        let loader = match opts.run_opts.single_package {
            true => {
                let root_scripts = pkg_graph
                    .package_task_context(&PackageName::Root)
                    .map(|context| {
                        context
                            .native_tasks()
                            .tasks()
                            .iter()
                            .filter(|task| task.participates() || task.authored())
                            .map(|task| task.name().to_string())
                            .collect()
                    })
                    .unwrap_or_default();
                UnifiedTurboJsonLoader::single_package(
                    reader,
                    opts.repo_opts.root_turbo_json_path.clone(),
                    root_scripts,
                )
            }
            false => UnifiedTurboJsonLoader::workspace(
                reader,
                opts.repo_opts.root_turbo_json_path.clone(),
                pkg_graph.package_scope_directories(),
            ),
        };

        // Collect authoritative execution scopes, including aggregates.
        let workspaces: Vec<PackageName> = pkg_graph
            .package_scope_directories()
            .map(|(name, _)| name)
            .collect();

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

        // Also add catalog-backed root tasks so //#script tasks appear even
        // when not listed in turbo.json.
        if let Some(context) = pkg_graph.package_task_context(&PackageName::Root) {
            for native_task in context.native_tasks().tasks() {
                if native_task.script().is_none() {
                    continue;
                }
                let task_name = TaskName::from(format!("//#{}", native_task.name())).into_owned();
                if !root_tasks.contains(&task_name) {
                    root_tasks.push(task_name);
                }
            }
        }

        // Build engine with all tasks
        // We use `add_all_tasks` to get the complete task graph for visualization
        let engine = EngineBuilder::new(
            &self.repo_root,
            pkg_graph,
            &loader,
            opts.run_opts.single_package,
        )
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

                    // Get display/script text from the native-task catalog.
                    let script = pkg_graph
                        .package_task_context(&PackageName::from(task_id.package()))
                        .and_then(|context| context.native_tasks().get(task_id.task()))
                        .and_then(|task| task.script().map(|script| script.as_inner().clone()))
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

    async fn build_graph_data(&self) -> Result<GraphData, TaskGraphError> {
        let opts = self.resolve_opts()?;
        let pkg_graph = self.build_package_graph(&opts).await?;
        let package_graph = package_graph_to_data(&pkg_graph);
        let task_graph = self.build_engine_task_graph(&pkg_graph, &opts)?;
        Ok(GraphData {
            package_graph,
            task_graph,
        })
    }
}

impl RepositoryGraphBuilder for ProperTaskGraphBuilder {
    fn build_graphs(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<GraphData, TaskGraphError>> + Send + '_>> {
        Box::pin(self.build_graph_data())
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, ffi::OsString};

    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_devtools::RepositoryGraphBuilder;

    use super::ProperTaskGraphBuilder;
    use crate::Args;

    fn setup_cargo(root: &AbsoluteSystemPathBuf) {
        root.join_components(&["crates", "rust-app", "src"])
            .create_dir_all()
            .expect("create crate source directory");
        root.join_component("Cargo.toml")
            .create_with_contents(
                "[workspace]\nmembers = [\"crates/*\"]\nresolver = \
                 \"2\"\n\n[workspace.metadata]\nname = \"acme\"\n",
            )
            .expect("write workspace manifest");
        root.join_components(&["crates", "rust-app", "Cargo.toml"])
            .create_with_contents(
                "[package]\nname = \"rust-app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            )
            .expect("write crate manifest");
        root.join_components(&["crates", "rust-app", "src", "main.rs"])
            .create_with_contents("fn main() {}\n")
            .expect("write crate source");
        root.join_component("Cargo.lock")
            .create_with_contents(
                "version = 4\n\n[[package]]\nname = \"rust-app\"\nversion = \"0.1.0\"\n",
            )
            .expect("write Cargo lockfile");
    }

    fn builder(root: &AbsoluteSystemPathBuf) -> ProperTaskGraphBuilder {
        let args = Args::parse_args(
            ["turbo", "devtools", "--no-open"]
                .into_iter()
                .map(OsString::from)
                .collect(),
        )
        .expect("parse devtools arguments");
        ProperTaskGraphBuilder::new(root.clone(), args)
    }

    fn builder_with_config(
        root: &AbsoluteSystemPathBuf,
        config: &AbsoluteSystemPathBuf,
    ) -> ProperTaskGraphBuilder {
        let args = Args::parse_args(vec![
            OsString::from("turbo"),
            OsString::from(format!("--root-turbo-json={config}")),
            OsString::from("devtools"),
            OsString::from("--no-open"),
        ])
        .expect("parse devtools arguments");
        ProperTaskGraphBuilder::new(root.clone(), args)
    }

    fn setup_javascript(root: &AbsoluteSystemPathBuf) {
        root.join_components(&["packages", "web"])
            .create_dir_all()
            .expect("create JavaScript workspace");
        root.join_component("package.json")
            .create_with_contents(
                r#"{"name":"root","packageManager":"npm@10.5.0","workspaces":["packages/*"],"scripts":{"lint":"eslint ."}}"#,
            )
            .expect("write root package.json");
        root.join_components(&["packages", "web", "package.json"])
            .create_with_contents(r#"{"name":"web","scripts":{"build":"next build"}}"#)
            .expect("write workspace package.json");
        root.join_component("package-lock.json")
            .create_with_contents(
                r#"{"name":"root","lockfileVersion":3,"packages":{"":{"name":"root","workspaces":["packages/*"]},"packages/web":{"name":"web"}}}"#,
            )
            .expect("write npm lockfile");
    }

    fn package_names(graphs: &turborepo_devtools::GraphData) -> HashSet<&str> {
        graphs
            .package_graph
            .nodes
            .iter()
            .map(|node| node.name.as_str())
            .collect()
    }

    #[tokio::test]
    async fn pure_cargo_uses_the_same_generation_for_package_and_task_scopes() {
        let tempdir = tempfile::tempdir().expect("create temporary repository");
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).expect("absolute repository");
        setup_cargo(&root);
        root.join_component("turbo.json")
            .create_with_contents(
                r#"{"futureFlags":{"experimentalCargoWorkspaces":true},"tasks":{"build":{}}}"#,
            )
            .expect("write turbo.json");

        let graphs = builder(&root)
            .build_graphs()
            .await
            .expect("build devtools graphs");
        let packages = package_names(&graphs);

        assert_eq!(packages, HashSet::from(["acme", "rust-app"]));
        assert!(graphs.task_graph.nodes.iter().any(|node| {
            node.package == "rust-app" && node.task == "build" && node.script.is_empty()
        }));
        assert!(graphs
            .task_graph
            .nodes
            .iter()
            .filter(|node| node.package != "//")
            .all(|node| packages.contains(node.package.as_str())));
    }

    #[tokio::test]
    async fn malformed_javascript_root_is_not_hidden_by_cargo_support() {
        let tempdir = tempfile::tempdir().expect("create temporary repository");
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).expect("absolute repository");
        setup_cargo(&root);
        root.join_component("package.json")
            .create_with_contents("{")
            .expect("write malformed package.json");

        let error = builder(&root)
            .build_graphs()
            .await
            .expect_err("malformed package.json must fail");
        assert!(error.to_string().contains("package.json"));
    }

    #[tokio::test]
    async fn mixed_repository_preserves_javascript_scripts_and_native_scopes() {
        let tempdir = tempfile::tempdir().expect("create temporary repository");
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).expect("absolute repository");
        setup_cargo(&root);
        setup_javascript(&root);
        root.join_component("turbo.json")
            .create_with_contents(
                r#"{"futureFlags":{"experimentalCargoWorkspaces":true},"tasks":{"build":{},"lint":{}}}"#,
            )
            .expect("write turbo.json");

        let graphs = builder(&root)
            .build_graphs()
            .await
            .expect("build devtools graphs");
        let packages = package_names(&graphs);

        assert!(packages.is_superset(&HashSet::from(["(root)", "web", "acme", "rust-app"])));
        assert!(graphs.task_graph.nodes.iter().any(|node| {
            node.package == "web" && node.task == "build" && node.script == "next build"
        }));
        assert!(graphs.task_graph.nodes.iter().any(|node| {
            node.package == "//" && node.task == "lint" && node.script == "eslint ."
        }));
    }

    #[tokio::test]
    async fn refresh_reloads_cargo_future_flag() {
        let tempdir = tempfile::tempdir().expect("create temporary repository");
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).expect("absolute repository");
        setup_cargo(&root);
        setup_javascript(&root);
        root.join_component("turbo.json")
            .create_with_contents(r#"{"tasks":{"build":{}}}"#)
            .expect("write turbo.json");
        let builder = builder(&root);

        let before = builder.build_graphs().await.expect("build JS graph");
        assert!(!package_names(&before).contains("rust-app"));

        root.join_component("turbo.json")
            .create_with_contents(
                r#"{"futureFlags":{"experimentalCargoWorkspaces":true},"tasks":{"build":{}}}"#,
            )
            .expect("enable Cargo support");
        let after = builder.build_graphs().await.expect("build mixed graph");
        assert!(package_names(&after).contains("rust-app"));
    }

    #[tokio::test]
    async fn refresh_reselects_default_turbo_config_file() {
        let tempdir = tempfile::tempdir().expect("create temporary repository");
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).expect("absolute repository");
        setup_javascript(&root);
        root.join_component("turbo.json")
            .create_with_contents(r#"{"tasks":{"build":{},"lint":{}}}"#)
            .expect("write turbo.json");
        let builder = builder(&root);

        builder.build_graphs().await.expect("build from turbo.json");
        assert!(builder
            .resolve_opts()
            .expect("resolve turbo.json")
            .repo_opts
            .root_turbo_json_path
            .ends_with("turbo.json"));

        root.join_component("turbo.json")
            .remove()
            .expect("remove turbo.json");
        root.join_component("turbo.jsonc")
            .create_with_contents("// refreshed\n{\"tasks\":{\"build\":{},\"lint\":{}}}")
            .expect("write turbo.jsonc");

        builder
            .build_graphs()
            .await
            .expect("build from turbo.jsonc");
        assert!(builder
            .resolve_opts()
            .expect("resolve turbo.jsonc")
            .repo_opts
            .root_turbo_json_path
            .ends_with("turbo.jsonc"));
    }

    #[tokio::test]
    async fn refresh_keeps_explicit_turbo_config_selection() {
        let tempdir = tempfile::tempdir().expect("create temporary repository");
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).expect("absolute repository");
        setup_javascript(&root);
        let custom_config = root.join_component("custom.json");
        custom_config
            .create_with_contents(r#"{"tasks":{"build":{},"lint":{}}}"#)
            .expect("write custom config");
        root.join_component("turbo.json")
            .create_with_contents(r#"{"tasks":{}}"#)
            .expect("write default config");
        let builder = builder_with_config(&root, &custom_config);

        builder
            .build_graphs()
            .await
            .expect("build from custom config");
        root.join_component("turbo.json")
            .remove()
            .expect("remove default config");
        root.join_component("turbo.jsonc")
            .create_with_contents("{")
            .expect("write malformed default config");

        builder
            .build_graphs()
            .await
            .expect("refresh still uses custom config");
        assert_eq!(
            builder
                .resolve_opts()
                .expect("resolve custom config")
                .repo_opts
                .root_turbo_json_path,
            custom_config
        );
    }

    #[tokio::test]
    async fn rejects_explicit_turbo_config_outside_repository() {
        let tempdir = tempfile::tempdir().expect("create temporary repository");
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).expect("absolute repository");
        setup_javascript(&root);
        let external_dir = tempfile::tempdir().expect("create external config directory");
        let external_config = AbsoluteSystemPathBuf::try_from(external_dir.path())
            .expect("absolute external directory")
            .join_component("turbo.json");
        external_config
            .create_with_contents(r#"{"tasks":{"build":{}}}"#)
            .expect("write external config");

        let error = builder_with_config(&root, &external_config)
            .build_graphs()
            .await
            .expect_err("external config must be rejected");
        assert!(error.to_string().contains("must be inside the repository"));
        assert!(error.to_string().contains(external_config.as_str()));
    }
}
