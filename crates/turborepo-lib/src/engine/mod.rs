mod builder;
pub(crate) mod task_inheritance;

pub use builder::{EngineBuilder, Error as BuilderError};
use miette::{Diagnostic, NamedSource, SourceSpan};
use thiserror::Error;
// Building state is used for engine construction
#[cfg(test)]
pub use turborepo_engine::Building;
// Re-export core types from turborepo-engine
pub use turborepo_engine::{
    Built, ExecuteError, ExecutionOptions, Message, StopExecution, TaskDefinitionInfo, TaskNode,
};
use turborepo_repository::package_graph::{PackageGraph, PackageName};
use turborepo_types::{TaskDefinition, UIMode};

/// Type alias for Engine specialized with TaskDefinition.
/// This allows existing code to continue using `Engine` without type
/// parameters.
pub type Engine<S = Built> = turborepo_engine::Engine<S, TaskDefinition>;

// Note: TaskDefinitionInfo is now implemented for TaskDefinition
// directly in turborepo-engine crate.

#[derive(Debug, Error, Diagnostic, PartialEq, PartialOrd, Eq, Ord)]
pub enum ValidateError {
    #[error("Cannot find task definition for {task_id} in package {package_name}")]
    MissingTask {
        task_id: String,
        package_name: String,
    },
    #[error("Cannot find package {package}")]
    MissingPackageJson { package: String },
    #[error("\"{persistent_task}\" is a persistent task, \"{dependant}\" cannot depend on it")]
    DependencyOnPersistentTask {
        #[label("persistent task")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
        persistent_task: String,
        dependant: String,
    },
    #[error(
        "You have {persistent_count} persistent tasks but `turbo` is configured for concurrency \
         of {concurrency}. Set --concurrency to at least {}", persistent_count+1
    )]
    PersistentTasksExceedConcurrency {
        persistent_count: u32,
        concurrency: u32,
    },
    #[error(
        "Cannot run interactive task \"{task}\" without Terminal UI. Set `\"ui\": \"tui\"` in \
         `turbo.json`, use the `--ui=tui` flag, or set `TURBO_UI=true` as an environment variable."
    )]
    InteractiveNeedsUI { task: String },
}

/// Extension trait for Engine<Built, TaskDefinition> that provides
/// turborepo-lib specific functionality.
pub trait EngineExt {
    /// Return all tasks that have a command to be run
    fn tasks_with_command(&self, pkg_graph: &PackageGraph) -> Vec<String>;

    /// Validate the engine against a package graph
    fn validate(
        &self,
        package_graph: &PackageGraph,
        concurrency: u32,
        ui_mode: UIMode,
        will_execute_tasks: bool,
    ) -> Result<(), Vec<ValidateError>>;
}

impl EngineExt for Engine<Built> {
    fn tasks_with_command(&self, pkg_graph: &PackageGraph) -> Vec<String> {
        self.tasks()
            .filter_map(|node| match node {
                TaskNode::Root => None,
                TaskNode::Task(task) => Some(task),
            })
            .filter_map(|task| {
                let pkg_name = PackageName::from(task.package());
                let json = pkg_graph.package_json(&pkg_name)?;
                // TODO: delegate to command factory to filter down tasks to those that will
                // have a runnable command.
                (task.task() == "proxy" || json.command(task.task()).is_some())
                    .then(|| task.to_string())
            })
            .collect()
    }

    fn validate(
        &self,
        package_graph: &PackageGraph,
        concurrency: u32,
        ui_mode: UIMode,
        will_execute_tasks: bool,
    ) -> Result<(), Vec<ValidateError>> {
        // TODO(olszewski) once this is hooked up to a real run, we should
        // see if using rayon to parallelize would provide a speedup
        let (persistent_count, mut validation_errors) = self
            .task_graph()
            .node_indices()
            .map(|node_index| {
                let TaskNode::Task(task_id) = self
                    .task_graph()
                    .node_weight(node_index)
                    .expect("graph should contain weight for node index")
                else {
                    // No need to check the root node if that's where we are.
                    return Ok(false);
                };

                for dep_index in self
                    .task_graph()
                    .neighbors_directed(node_index, petgraph::Direction::Outgoing)
                {
                    let TaskNode::Task(dep_id) = self
                        .task_graph()
                        .node_weight(dep_index)
                        .expect("index comes from iterating the graph and must be present")
                    else {
                        // No need to check the root node
                        continue;
                    };

                    let task_definition =
                        self.task_definition(dep_id)
                            .ok_or_else(|| ValidateError::MissingTask {
                                task_id: dep_id.to_string(),
                                package_name: dep_id.package().to_string(),
                            })?;

                    let package_json = package_graph
                        .package_json(&PackageName::from(dep_id.package()))
                        .ok_or_else(|| ValidateError::MissingPackageJson {
                            package: dep_id.package().to_string(),
                        })?;
                    if task_definition.persistent
                        && package_json.scripts.contains_key(dep_id.task())
                    {
                        let (span, text) = self
                            .task_locations()
                            .get(dep_id)
                            .map(|spanned| spanned.span_and_text("turbo.json"))
                            .unwrap_or((None, NamedSource::new("", String::new())));

                        return Err(ValidateError::DependencyOnPersistentTask {
                            span,
                            text,
                            persistent_task: dep_id.to_string(),
                            dependant: task_id.to_string(),
                        });
                    }
                }

                // check if the package for the task has that task in its package.json
                let info = package_graph
                    .package_info(&PackageName::from(task_id.package().to_string()))
                    .expect("package graph should contain workspace info for task package");

                let package_has_task = info
                    .package_json
                    .scripts
                    .get(task_id.task())
                    // handle legacy behaviour from go where an empty string may appear
                    .is_some_and(|script| !script.is_empty());

                let task_is_persistent = self
                    .task_definition(task_id)
                    .is_some_and(|task_def| task_def.persistent);

                Ok(task_is_persistent && package_has_task)
            })
            .fold((0, Vec::new()), |(mut count, mut errs), result| {
                match result {
                    Ok(true) => count += 1,
                    Ok(false) => (),
                    Err(e) => errs.push(e),
                }
                (count, errs)
            });

        // there must always be at least one concurrency 'slot' available for
        // non-persistent tasks otherwise we get race conditions
        if will_execute_tasks && persistent_count >= concurrency {
            validation_errors.push(ValidateError::PersistentTasksExceedConcurrency {
                persistent_count,
                concurrency,
            })
        }

        if will_execute_tasks {
            validation_errors.extend(validate_interactive(self, ui_mode));
        }

        validation_errors.sort();

        match validation_errors.is_empty() {
            true => Ok(()),
            false => Err(validation_errors),
        }
    }
}

// Validates that UI is setup if any interactive tasks will be executed
fn validate_interactive(engine: &Engine<Built>, ui_mode: UIMode) -> Vec<ValidateError> {
    // If experimental_ui is being used, then we don't need check for interactive
    // tasks
    if matches!(ui_mode, UIMode::Tui) {
        return Vec::new();
    }
    engine
        .task_definitions()
        .iter()
        .filter_map(|(task, definition)| {
            if definition.interactive {
                Some(ValidateError::InteractiveNeedsUI {
                    task: task.to_string(),
                })
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod test {

    use std::collections::BTreeMap;

    use tempfile::TempDir;
    use turbopath::AbsoluteSystemPath;
    use turborepo_errors::Spanned;
    use turborepo_repository::{
        discovery::{DiscoveryResponse, PackageDiscovery, WorkspaceData},
        package_json::PackageJson,
    };
    use turborepo_task_id::{TaskId, TaskName};

    use super::*;

    struct DummyDiscovery<'a>(&'a TempDir);

    impl<'a> PackageDiscovery for DummyDiscovery<'a> {
        async fn discover_packages(
            &self,
        ) -> Result<
            turborepo_repository::discovery::DiscoveryResponse,
            turborepo_repository::discovery::Error,
        > {
            // our workspace has three packages, two of which have a build script
            let workspaces = [("a", true), ("b", true), ("c", false)]
                .into_iter()
                .map(|(name, had_build)| {
                    let path = AbsoluteSystemPath::from_std_path(self.0.path()).unwrap();
                    let package_json = path.join_component(&format!("{}.json", name));

                    let scripts = if had_build {
                        BTreeMap::from_iter(
                            [
                                ("build".to_string(), Spanned::new("echo built!".to_string())),
                                (
                                    "dev".to_string(),
                                    Spanned::new("echo running dev!".to_string()),
                                ),
                            ]
                            .into_iter(),
                        )
                    } else {
                        BTreeMap::default()
                    };

                    let package = PackageJson {
                        name: Some(Spanned::new(name.to_string())),
                        scripts,
                        ..Default::default()
                    };

                    let file = std::fs::File::create(package_json.as_std_path()).unwrap();
                    serde_json::to_writer(file, &package).unwrap();

                    WorkspaceData {
                        package_json,
                        turbo_json: None,
                    }
                })
                .collect();

            Ok(DiscoveryResponse {
                package_manager: turborepo_repository::package_manager::PackageManager::Pnpm,
                workspaces,
            })
        }

        async fn discover_packages_blocking(
            &self,
        ) -> Result<
            turborepo_repository::discovery::DiscoveryResponse,
            turborepo_repository::discovery::Error,
        > {
            self.discover_packages().await
        }
    }

    #[tokio::test]
    async fn issue_4291() {
        // we had an issue where our engine validation would reject running persistent
        // tasks if the number of _total packages_ exceeded the concurrency limit,
        // rather than the number of package that had that task. in this test, we
        // set up a workspace with three packages, two of which have a persistent build
        // task. we expect concurrency limit 1 to fail, but 2 and 3 to pass.

        let tmp = tempfile::TempDir::with_prefix("issue_4291").unwrap();

        let mut engine: Engine<Building> = Engine::new();

        // add two packages with a persistent build task
        for package in ["a", "b"] {
            let task_id = TaskId::new(package, "build");
            engine.get_index(&task_id);
            engine.add_definition(
                task_id,
                TaskDefinition {
                    persistent: true,
                    ..Default::default()
                },
            );
        }

        let engine = engine.seal();

        let graph_builder = PackageGraph::builder(
            AbsoluteSystemPath::from_std_path(tmp.path()).unwrap(),
            PackageJson::default(),
        )
        .with_package_discovery(DummyDiscovery(&tmp));

        let graph = graph_builder.build().await.unwrap();

        // if our limit is less than, it should fail
        engine
            .validate(&graph, 1, UIMode::Stream, true)
            .expect_err("not enough");

        // if our limit is less than, it should fail
        engine
            .validate(&graph, 2, UIMode::Stream, true)
            .expect_err("not enough");

        // we have two persistent tasks, and a slot for all other tasks, so this should
        // pass
        engine
            .validate(&graph, 3, UIMode::Stream, true)
            .expect("ok");

        // if our limit is greater, then it should pass
        engine
            .validate(&graph, 4, UIMode::Stream, true)
            .expect("ok");
    }

    #[tokio::test]
    async fn test_interactive_validation() {
        let tmp = tempfile::TempDir::new().unwrap();

        let mut engine: Engine<Building> = Engine::new();

        // add two packages with a persistent build task
        for package in ["a", "b"] {
            let task_id = TaskId::new(package, "build");
            engine.get_index(&task_id);
            engine.add_definition(
                task_id,
                TaskDefinition {
                    persistent: true,
                    interactive: true,
                    ..Default::default()
                },
            );
        }

        let engine = engine.seal();

        let graph_builder = PackageGraph::builder(
            AbsoluteSystemPath::from_std_path(tmp.path()).unwrap(),
            PackageJson::default(),
        )
        .with_package_discovery(DummyDiscovery(&tmp));

        let graph = graph_builder.build().await.unwrap();

        assert!(engine.validate(&graph, 3, UIMode::Stream, false).is_ok());
        assert!(engine.validate(&graph, 3, UIMode::Stream, true).is_err());
    }

    #[tokio::test]
    async fn test_dry_run_skips_concurrency_validation() {
        let tmp = tempfile::TempDir::new().unwrap();

        let mut engine: Engine<Building> = Engine::new();

        // add two packages with a persistent build task
        for package in ["a", "b"] {
            let task_id = TaskId::new(package, "build");
            engine.get_index(&task_id);
            engine.add_definition(
                task_id,
                TaskDefinition {
                    persistent: true,
                    ..Default::default()
                },
            );
        }

        let engine = engine.seal();

        let graph_builder = PackageGraph::builder(
            AbsoluteSystemPath::from_std_path(tmp.path()).unwrap(),
            PackageJson::default(),
        )
        .with_package_discovery(DummyDiscovery(&tmp));

        let graph = graph_builder.build().await.unwrap();

        assert!(engine.validate(&graph, 1, UIMode::Stream, false).is_ok());
        assert!(engine.validate(&graph, 1, UIMode::Stream, true).is_err());
    }

    #[tokio::test]
    async fn test_prune_persistent_tasks() {
        // Verifies that we can prune the `Engine` to include only the persistent tasks
        // or only the non-persistent tasks.

        let mut engine: Engine<Building> = Engine::new();

        // add two packages with a persistent build task
        for package in ["a", "b"] {
            let build_task_id = TaskId::new(package, "build");
            let dev_task_id = TaskId::new(package, "dev");

            engine.get_index(&build_task_id);
            engine.add_definition(build_task_id.clone(), TaskDefinition::default());

            engine.get_index(&dev_task_id);
            engine.add_definition(
                dev_task_id,
                TaskDefinition {
                    persistent: true,
                    task_dependencies: vec![Spanned::new(TaskName::from(build_task_id))],
                    ..Default::default()
                },
            );
        }

        let engine = engine.seal();

        let non_interruptible_tasks_engine = engine.create_engine_for_non_interruptible_tasks();
        for node in non_interruptible_tasks_engine.tasks() {
            if let TaskNode::Task(task_id) = node {
                let def = non_interruptible_tasks_engine
                    .task_definition(task_id)
                    .expect("task should have definition");
                assert!(def.persistent, "task should be persistent");
            }
        }

        let interruptible_tasks_engine = engine.create_engine_for_interruptible_tasks();
        for node in interruptible_tasks_engine.tasks() {
            if let TaskNode::Task(task_id) = node {
                let def = interruptible_tasks_engine
                    .task_definition(task_id)
                    .expect("task should have definition");
                assert!(!def.persistent, "task should not be persistent");
            }
        }
    }

    #[tokio::test]
    async fn test_get_subgraph_for_package() {
        // Verifies that we can prune the `Engine` to include only the persistent tasks
        // or only the non-persistent tasks.

        let mut engine: Engine<Building> = Engine::new();

        // Add two tasks in package `a`
        let a_build_task_id = TaskId::new("a", "build");
        let a_dev_task_id = TaskId::new("a", "dev");

        let a_build_idx = engine.get_index(&a_build_task_id);
        engine.add_definition(a_build_task_id.clone(), TaskDefinition::default());

        engine.get_index(&a_dev_task_id);
        engine.add_definition(a_dev_task_id.clone(), TaskDefinition::default());

        // Add two tasks in package `b` where the `build` task depends
        // on the `build` task from package `a`
        let b_build_task_id = TaskId::new("b", "build");
        let b_dev_task_id = TaskId::new("b", "dev");

        let b_build_idx = engine.get_index(&b_build_task_id);
        engine.add_definition(
            b_build_task_id.clone(),
            TaskDefinition {
                task_dependencies: vec![Spanned::new(TaskName::from(a_build_task_id.clone()))],
                ..Default::default()
            },
        );

        engine.get_index(&b_dev_task_id);
        engine.add_definition(b_dev_task_id.clone(), TaskDefinition::default());
        engine
            .task_graph_mut()
            .add_edge(b_build_idx, a_build_idx, ());

        let engine = engine.seal();
        let subgraph =
            engine.create_engine_for_subgraph(&[PackageName::from("a")].into_iter().collect());

        // Verify that the subgraph only contains tasks from package `a` and the `build`
        // task from package `b`
        let tasks: Vec<_> = subgraph.tasks().collect();
        assert_eq!(tasks.len(), 3);
        assert!(tasks.contains(&&TaskNode::Task(a_build_task_id)));
        assert!(tasks.contains(&&TaskNode::Task(a_dev_task_id)));
        assert!(tasks.contains(&&TaskNode::Task(b_build_task_id)));
    }

    #[tokio::test]
    async fn test_tasks_impacted_by_packages() {
        // Tests the batched transitive dependents lookup for watch mode.
        //
        // Graph structure:
        //   a:build  <--  b:build  <--  c:build
        //   a:test       b:test        c:test
        //
        // Where b:build depends on a:build, and c:build depends on b:build.
        // Changing package "a" should impact: a:build, a:test, b:build, c:build
        // Changing package "b" should impact: b:build, b:test, c:build

        let mut engine: Engine<Building> = Engine::new();

        // Package a
        let a_build = TaskId::new("a", "build");
        let a_test = TaskId::new("a", "test");
        let a_build_idx = engine.get_index(&a_build);
        engine.get_index(&a_test);
        engine.add_definition(a_build.clone(), TaskDefinition::default());
        engine.add_definition(a_test.clone(), TaskDefinition::default());

        // Package b (b:build depends on a:build)
        let b_build = TaskId::new("b", "build");
        let b_test = TaskId::new("b", "test");
        let b_build_idx = engine.get_index(&b_build);
        engine.get_index(&b_test);
        engine.add_definition(b_build.clone(), TaskDefinition::default());
        engine.add_definition(b_test.clone(), TaskDefinition::default());
        engine
            .task_graph_mut()
            .add_edge(b_build_idx, a_build_idx, ());

        // Package c (c:build depends on b:build)
        let c_build = TaskId::new("c", "build");
        let c_test = TaskId::new("c", "test");
        let c_build_idx = engine.get_index(&c_build);
        engine.get_index(&c_test);
        engine.add_definition(c_build.clone(), TaskDefinition::default());
        engine.add_definition(c_test.clone(), TaskDefinition::default());
        engine
            .task_graph_mut()
            .add_edge(c_build_idx, b_build_idx, ());

        let engine = engine.seal();

        // Test: changing package "a" should impact a:build, a:test, b:build, c:build
        let impacted =
            engine.tasks_impacted_by_packages(&[PackageName::from("a")].into_iter().collect());

        // Filter out Root node and collect task IDs
        let impacted_tasks: std::collections::HashSet<_> = impacted
            .iter()
            .filter_map(|node| match node {
                TaskNode::Task(id) => Some(id.clone()),
                TaskNode::Root => None,
            })
            .collect();

        assert_eq!(impacted_tasks.len(), 4);
        assert!(impacted_tasks.contains(&a_build));
        assert!(impacted_tasks.contains(&a_test));
        assert!(impacted_tasks.contains(&b_build)); // transitive dependent
        assert!(impacted_tasks.contains(&c_build)); // transitive dependent

        // Test: changing package "b" should impact b:build, b:test, c:build
        let impacted =
            engine.tasks_impacted_by_packages(&[PackageName::from("b")].into_iter().collect());

        let impacted_tasks: std::collections::HashSet<_> = impacted
            .iter()
            .filter_map(|node| match node {
                TaskNode::Task(id) => Some(id.clone()),
                TaskNode::Root => None,
            })
            .collect();

        assert_eq!(impacted_tasks.len(), 3);
        assert!(impacted_tasks.contains(&b_build));
        assert!(impacted_tasks.contains(&b_test));
        assert!(impacted_tasks.contains(&c_build)); // transitive dependent

        // Test: changing multiple packages at once (a and c)
        // Should find: a:build, a:test, b:build, c:build, c:test
        let impacted = engine.tasks_impacted_by_packages(
            &[PackageName::from("a"), PackageName::from("c")]
                .into_iter()
                .collect(),
        );

        let impacted_tasks: std::collections::HashSet<_> = impacted
            .iter()
            .filter_map(|node| match node {
                TaskNode::Task(id) => Some(id.clone()),
                TaskNode::Root => None,
            })
            .collect();

        assert_eq!(impacted_tasks.len(), 5);
        assert!(impacted_tasks.contains(&a_build));
        assert!(impacted_tasks.contains(&a_test));
        assert!(impacted_tasks.contains(&b_build)); // transitive dependent of a
        assert!(impacted_tasks.contains(&c_build)); // both direct (c) and transitive (a->b->c)
        assert!(impacted_tasks.contains(&c_test)); // direct from c

        // Test: empty set returns empty
        let impacted = engine.tasks_impacted_by_packages(&std::collections::HashSet::new());
        assert!(impacted.is_empty());

        // Test: non-existent package returns empty
        let impacted = engine
            .tasks_impacted_by_packages(&[PackageName::from("nonexistent")].into_iter().collect());
        assert!(impacted.is_empty());
    }
}
