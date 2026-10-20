use miette::{Diagnostic, NamedSource, SourceSpan};
use thiserror::Error;
use turborepo_repository::package_graph::{PackageGraph, PackageName};
use turborepo_task_id::TaskId;
use turborepo_types::{TaskCommandOverride, TaskDefinition, UIMode};

use crate::{Built, Engine, TaskNode};

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
         of {concurrency}. Set `--concurrency` to at least {} or configure `\"concurrency\"` \
         in `turbo.json`", persistent_count+1
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

/// Returns whether a task resolves to a runnable command.
pub fn task_has_command(
    engine: &Engine<Built, TaskDefinition>,
    package_graph: &PackageGraph,
    task: &TaskId<'static>,
) -> bool {
    if task.task() == "proxy" {
        return true;
    }

    let Some(context) = package_graph.package_task_context(&PackageName::from(task.package()))
    else {
        return false;
    };

    match engine
        .task_definition(task)
        .and_then(|definition| definition.command.as_ref())
    {
        Some(TaskCommandOverride::Argv(_)) => true,
        Some(TaskCommandOverride::OptOut) => false,
        None => context.native_tasks().defines(task.task()),
    }
}

/// Returns whether a task participates in its package's native task catalog.
pub fn task_participates(
    engine: &Engine<Built, TaskDefinition>,
    package_graph: &PackageGraph,
    task: &TaskId<'static>,
) -> bool {
    let Some(context) = package_graph.package_task_context(&PackageName::from(task.package()))
    else {
        return false;
    };

    match engine
        .task_definition(task)
        .and_then(|definition| definition.command.as_ref())
    {
        Some(TaskCommandOverride::Argv(_)) => true,
        Some(TaskCommandOverride::OptOut) => false,
        None => context.native_tasks().participates(task.task()),
    }
}

impl Engine<Built, TaskDefinition> {
    /// Returns all tasks that have a command to run.
    pub fn tasks_with_command(&self, package_graph: &PackageGraph) -> Vec<String> {
        self.tasks()
            .filter_map(|node| match node {
                TaskNode::Root => None,
                TaskNode::Task(task) => Some(task),
            })
            .filter(|task| {
                // Ask the native-task catalog whether the task resolves to a
                // runnable command — the same authority execution uses. A
                // resolved `command` override is authoritative in both
                // directions: an argv executes even where the catalog defines
                // nothing, and an opt-out never executes even where it does.
                task_has_command(self, package_graph, task)
            })
            .map(ToString::to_string)
            .collect()
    }

    /// Validates the engine against a package graph and execution settings.
    #[allow(clippy::expect_used)]
    pub fn validate(
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
                let task_node = self
                    .task_graph()
                    .node_weight(node_index)
                    .expect("graph should contain weight for node index");
                let TaskNode::Task(task_id) = task_node else {
                    // No need to check the root node if that's where we are.
                    return Ok(false);
                };

                for dep_index in self
                    .task_graph()
                    .neighbors_directed(node_index, petgraph::Direction::Outgoing)
                {
                    let dep_node = self
                        .task_graph()
                        .node_weight(dep_index)
                        .expect("index comes from iterating the graph and must be present");
                    let TaskNode::Task(dep_id) = dep_node else {
                        // No need to check the root node
                        continue;
                    };

                    let task_definition =
                        self.task_definition(dep_id)
                            .ok_or_else(|| ValidateError::MissingTask {
                                task_id: dep_id.to_string(),
                                package_name: dep_id.package().to_string(),
                            })?;

                    let package_name = PackageName::from(dep_id.package());
                    let Some(_dep_context) = package_graph.package_task_context(&package_name)
                    else {
                        return Err(ValidateError::MissingPackageJson {
                            package: dep_id.package().to_string(),
                        });
                    };
                    if task_definition.persistent && task_has_command(self, package_graph, dep_id) {
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

                // check if the package for the task defines an executable native task
                let package_name = PackageName::from(task_id.package().to_string());
                let Some(_context) = package_graph.package_task_context(&package_name) else {
                    return Err(ValidateError::MissingPackageJson {
                        package: task_id.package().to_string(),
                    });
                };

                let package_has_task = task_has_command(self, package_graph, task_id);

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
fn validate_interactive(
    engine: &Engine<Built, TaskDefinition>,
    ui_mode: UIMode,
) -> Vec<ValidateError> {
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

    use turbopath::AbsoluteSystemPath;
    use turborepo_errors::Spanned;
    use turborepo_repository::{
        discovery::{DiscoveryResponse, PackageDiscovery, WorkspaceData},
        package_graph::PackageGraph,
        package_json::PackageJson,
    };

    use super::*;
    use crate::Building;

    type TaskDefinitionEngine<S = Built> = Engine<S, TaskDefinition>;

    struct DummyDiscovery(turbopath::AbsoluteSystemPathBuf);

    impl PackageDiscovery for DummyDiscovery {
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
                    let path = &self.0;
                    let package_json = path.join_component(&format!("{}.json", name));

                    let scripts = if had_build {
                        BTreeMap::from_iter([
                            ("build".to_string(), Spanned::new("echo built!".to_string())),
                            (
                                "dev".to_string(),
                                Spanned::new("echo running dev!".to_string()),
                            ),
                        ])
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

                    WorkspaceData::new(package_json, None).unwrap()
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

    #[tokio::test(flavor = "multi_thread")]
    async fn test_tasks_with_command_asks_toolchains() {
        // The TUI task list must come from the same authority execution
        // uses: the package's toolchain. JS packages resolve via
        // package.json scripts; Cargo packages resolve via the toolchain's
        // verb tables — no scripts anywhere.
        let tmp = tempfile::TempDir::with_prefix("tasks_with_command").unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();

        // A minimal Cargo workspace with one binary crate.
        root.join_component("Cargo.toml")
            .create_with_contents(
                "[workspace]\nmembers = [\"crates/*\"]\nresolver = \
                 \"2\"\n\n[workspace.metadata]\nname = \"acme\"\n",
            )
            .unwrap();
        let crate_dir = root.join_components(&["crates", "my-crate"]);
        crate_dir.join_component("src").create_dir_all().unwrap();
        crate_dir
            .join_component("Cargo.toml")
            .create_with_contents(
                "[package]\nname = \"my-crate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            )
            .unwrap();
        crate_dir
            .join_components(&["src", "main.rs"])
            .create_with_contents("fn main() {}\n")
            .unwrap();
        root.join_component("Cargo.lock")
            .create_with_contents(
                "version = 4\n\n[[package]]\nname = \"my-crate\"\nversion = \"0.1.0\"\n",
            )
            .unwrap();

        let mut engine: TaskDefinitionEngine<Building> = Engine::new();
        for (package, task) in [
            // JS package with a build script (DummyDiscovery gives "a" one).
            ("a", "build"),
            // JS package without any scripts.
            ("c", "build"),
            // The synthetic Cargo workspace package.
            ("acme", "test"),
            // A binary crate.
            ("my-crate", "build"),
        ] {
            let task_id = TaskId::new(package, task);
            engine.get_index(&task_id);
            engine.add_definition(task_id, TaskDefinition::default());
        }
        let engine = engine.seal();

        let graph = PackageGraph::builder(root, PackageJson::default())
            .with_package_discovery(DummyDiscovery(
                turbopath::AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap(),
            ))
            .with_cargo()
            .build()
            .await
            .unwrap();

        let mut tasks = engine.tasks_with_command(&graph);
        tasks.sort();
        // "c#build" is absent: no script defines it. Both Cargo tasks are
        // present without any package.json involvement.
        assert_eq!(tasks, vec!["a#build", "acme#test", "my-crate#build"]);
    }

    #[tokio::test]
    async fn command_overrides_are_authoritative() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let mut engine: TaskDefinitionEngine<Building> = Engine::new();

        let argv = TaskId::new("c", "custom");
        let opt_out = TaskId::new("a", "build");
        let native = TaskId::new("a", "dev");
        for (task, command) in [
            (
                argv.clone(),
                Some(TaskCommandOverride::Argv(vec!["custom".to_string()])),
            ),
            (opt_out.clone(), Some(TaskCommandOverride::OptOut)),
            (native.clone(), None),
        ] {
            engine.get_index(&task);
            engine.add_definition(
                task,
                TaskDefinition {
                    command,
                    ..Default::default()
                },
            );
        }
        let engine = engine.seal();
        let graph = PackageGraph::builder(root, PackageJson::default())
            .with_package_discovery(DummyDiscovery(
                turbopath::AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap(),
            ))
            .build()
            .await
            .unwrap();

        assert!(task_has_command(&engine, &graph, &argv));
        assert!(task_participates(&engine, &graph, &argv));
        assert!(!task_has_command(&engine, &graph, &opt_out));
        assert!(!task_participates(&engine, &graph, &opt_out));
        assert!(task_has_command(&engine, &graph, &native));
        assert!(task_participates(&engine, &graph, &native));
    }

    #[tokio::test]
    async fn issue_4291() {
        // we had an issue where our engine validation would reject running persistent
        // tasks if the number of _total packages_ exceeded the concurrency limit,
        // rather than the number of package that had that task. in this test, we
        // set up a workspace with three packages, two of which have a persistent build
        // task. we expect concurrency limit 1 to fail, but 2 and 3 to pass.

        let tmp = tempfile::TempDir::with_prefix("issue_4291").unwrap();

        let mut engine: TaskDefinitionEngine<Building> = Engine::new();

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
        .with_package_discovery(DummyDiscovery(
            turbopath::AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap(),
        ));

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

        let mut engine: TaskDefinitionEngine<Building> = Engine::new();

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
        .with_package_discovery(DummyDiscovery(
            turbopath::AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap(),
        ));

        let graph = graph_builder.build().await.unwrap();

        assert!(engine.validate(&graph, 3, UIMode::Stream, false).is_ok());
        assert!(engine.validate(&graph, 3, UIMode::Stream, true).is_err());
    }

    #[tokio::test]
    async fn test_dry_run_skips_concurrency_validation() {
        let tmp = tempfile::TempDir::new().unwrap();

        let mut engine: TaskDefinitionEngine<Building> = Engine::new();

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
        .with_package_discovery(DummyDiscovery(
            turbopath::AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap(),
        ));

        let graph = graph_builder.build().await.unwrap();

        assert!(engine.validate(&graph, 1, UIMode::Stream, false).is_ok());
        assert!(engine.validate(&graph, 1, UIMode::Stream, true).is_err());
    }

    #[tokio::test]
    async fn validation_rejects_dependency_on_persistent_task() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut engine: TaskDefinitionEngine<Building> = Engine::new();

        let build = TaskId::new("a", "build");
        let dev = TaskId::new("a", "dev");
        let build_idx = engine.get_index(&build);
        let dev_idx = engine.get_index(&dev);
        engine.add_definition(build.clone(), TaskDefinition::default());
        engine.add_definition(
            dev.clone(),
            TaskDefinition {
                persistent: true,
                ..Default::default()
            },
        );
        engine.task_graph_mut().add_edge(build_idx, dev_idx, ());
        engine.connect_to_root(&dev);
        let engine = engine.seal();

        let graph = PackageGraph::builder(
            AbsoluteSystemPath::from_std_path(tmp.path()).unwrap(),
            PackageJson::default(),
        )
        .with_package_discovery(DummyDiscovery(
            turbopath::AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap(),
        ))
        .build()
        .await
        .unwrap();

        let errors = engine
            .validate(&graph, 10, UIMode::Stream, true)
            .expect_err("persistent dependency should be rejected");

        assert!(errors.iter().any(|error| matches!(
            error,
            ValidateError::DependencyOnPersistentTask { persistent_task, dependant, .. }
                if persistent_task == "a#dev" && dependant == "a#build"
        )));
    }

    #[tokio::test]
    async fn validation_allows_dependency_on_persistent_task_without_script() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut engine: TaskDefinitionEngine<Building> = Engine::new();

        let build = TaskId::new("a", "build");
        let dev = TaskId::new("c", "dev");
        let build_idx = engine.get_index(&build);
        let dev_idx = engine.get_index(&dev);
        engine.add_definition(build.clone(), TaskDefinition::default());
        engine.add_definition(
            dev.clone(),
            TaskDefinition {
                persistent: true,
                ..Default::default()
            },
        );
        engine.task_graph_mut().add_edge(build_idx, dev_idx, ());
        engine.connect_to_root(&dev);
        let engine = engine.seal();

        let graph = PackageGraph::builder(
            AbsoluteSystemPath::from_std_path(tmp.path()).unwrap(),
            PackageJson::default(),
        )
        .with_package_discovery(DummyDiscovery(
            turbopath::AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap(),
        ))
        .build()
        .await
        .unwrap();

        engine.validate(&graph, 10, UIMode::Stream, true).unwrap();
    }
}
