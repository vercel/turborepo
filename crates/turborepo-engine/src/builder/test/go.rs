//! Go task selection from injected repository observations, without invoking Go
//! or turbo.

use std::{collections::HashMap, sync::Arc};

use serde_json::json;
use tempfile::TempDir;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_errors::Spanned;
use turborepo_repository::{
    native_tasks::{
        NativeCommandArguments, NativeCommandProgram, NativeTask, WorkingDirectoryPolicy,
    },
    package_graph::{PackageGraph, PackageName},
    package_json::PackageJson,
    relationships::{DependencyKind, Relationship},
    toolchain::{
        DiscoverPackageScopesFuture, DiscoverPackagesFuture, DiscoveredPackage,
        DiscoveredPackageScopes, DiscoveredPackages, RepositoryContributor, ToolchainId,
        WorkspaceRoot,
    },
};
use turborepo_task_id::{TaskId, TaskName};

use super::{EngineBuilder, TestTurboJsonLoader, all_dependencies, turbo_json};
use crate::TaskNode;

struct GoObservation {
    root: AbsoluteSystemPathBuf,
}

impl RepositoryContributor for GoObservation {
    fn id(&self) -> ToolchainId {
        ToolchainId::GO
    }

    fn discover_packages(&self) -> DiscoverPackagesFuture<'_> {
        Box::pin(async move {
            let native_build = || {
                NativeTask::command_task(
                    "build",
                    "go build".to_string(),
                    NativeCommandProgram::Tool("go".to_string()),
                    NativeCommandArguments::new(vec!["build".to_string(), "./...".to_string()]),
                    None,
                    WorkingDirectoryPolicy::PackageDirectory,
                )
            };
            let module = |name: &str, directory: &str, relationships| {
                DiscoveredPackage::package(
                    Some(name.to_string()),
                    PackageJson::default(),
                    self.root.join_components(
                        &directory.split('/').chain(["go.mod"]).collect::<Vec<_>>(),
                    ),
                )
                .with_native_relationships(relationships)
                .with_native_tasks(vec![native_build()])
            };
            let api = module(
                "api",
                "apps/api",
                vec![Relationship::internal("lib", DependencyKind::Production)],
            );
            let lib = module("lib", "packages/lib", Vec::new());
            let workspace = DiscoveredPackage::aggregate(
                "go-workspace".to_string(),
                PackageJson::default(),
                self.root.join_component("go.work"),
            )
            .with_native_relationships(vec![
                Relationship::internal("api", DependencyKind::Production),
                Relationship::internal("lib", DependencyKind::Production),
            ])
            .with_native_tasks(vec![NativeTask::command_task(
                "typecheck",
                "go test ./...".to_string(),
                NativeCommandProgram::Tool("go".to_string()),
                NativeCommandArguments::new(vec!["test".to_string(), "./...".to_string()]),
                None,
                WorkingDirectoryPolicy::RepositoryRoot,
            )]);
            Ok(DiscoveredPackages::new(
                vec![api, lib, workspace],
                vec![WorkspaceRoot::new("go", self.root.clone())],
            ))
        })
    }

    fn discover_package_scopes(&self) -> DiscoverPackageScopesFuture<'_> {
        Box::pin(async move {
            let observed = self.discover_packages().await?;
            Ok(DiscoveredPackageScopes::from_full_observation(
                observed.packages(),
                observed.workspace_roots(),
            ))
        })
    }
}

#[test]
fn go_build_definition_is_invariant_across_task_entrypoints() {
    let tmp = TempDir::new().unwrap();
    let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let graph = runtime
        .block_on(
            PackageGraph::builder_optional(&root, None)
                .with_package_jsons(Some(HashMap::new()))
                .with_contributor(Arc::new(GoObservation { root: root.clone() }))
                .build(),
        )
        .unwrap();
    let loader = TestTurboJsonLoader::new(HashMap::from([(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "build": { "dependsOn": ["^build"] },
                "typecheck": { "dependsOn": ["^build"] }
            }
        })),
    )]));
    let api_build = TaskId::new("api", "build");
    let lib_build = TaskNode::Task(TaskId::new("lib", "build").into_owned());
    let mut reference = None;

    // Indirect through the workspace aggregate, direct build, --only over
    // both selected modules, an explicit task, a package filter, and multiple
    // task entrypoints. filterUsingTasks pruning is tested in the CLI smoke.
    for (tasks, workspaces, only) in [
        (vec!["typecheck"], vec!["go-workspace"], false),
        (vec!["build"], vec!["api", "lib"], false),
        (vec!["build"], vec!["api", "lib"], true),
        (vec!["build"], vec!["api"], true),
        (vec!["api#build"], vec!["api"], false),
        (vec!["build"], vec!["api"], false),
        (vec!["build", "typecheck"], vec!["api"], false),
    ] {
        let engine = EngineBuilder::new(&root, &graph, &loader, false)
            .with_root_tasks([TaskName::from("build"), TaskName::from("typecheck")])
            .with_tasks(tasks.iter().map(|name| Spanned::new(TaskName::from(*name))))
            .with_workspaces(
                workspaces
                    .iter()
                    .map(|name| PackageName::from(*name))
                    .collect(),
            )
            .with_tasks_only(only)
            .build()
            .unwrap();
        let definition = engine.task_definition(&api_build).unwrap();
        if let Some(expected) = &reference {
            assert_eq!(
                definition, expected,
                "tasks={tasks:?}, workspaces={workspaces:?}"
            );
        } else {
            reference = Some(definition.clone());
        }
        let dependencies = all_dependencies(&engine);
        let api_deps = &dependencies[&api_build];
        // --only preserves an edge to lib when lib#build is also selected;
        // filtering to api alone omits that execution edge, not the authored
        // ^build in the resolved definition.
        assert_eq!(
            api_deps.contains(&lib_build),
            !only || workspaces.contains(&"lib"),
            "tasks={tasks:?}, workspaces={workspaces:?}"
        );
    }
}
