use std::collections::HashSet;

use turbopath::AnchoredSystemPath;
use turborepo_env::EnvironmentVariableMap;
use turborepo_lockfiles::Package;
use turborepo_repository::{
    package_graph::{PackageGraph, PackageInfo, PackageName},
    workspace_provider::{
        CargoWorkspaceProvider, TaskCommandProvider as WorkspaceTaskCommandProvider,
        UvWorkspaceProvider,
    },
};
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

    fn inferred_provider_command(workspace_info: &PackageInfo, task_name: &str) -> Option<String> {
        let manifest_name = workspace_info.package_json_path().as_path().file_name()?;
        if manifest_name.eq_ignore_ascii_case("Cargo.toml") {
            CargoWorkspaceProvider.resolve_task_command(task_name)
        } else if manifest_name.eq_ignore_ascii_case("pyproject.toml") {
            UvWorkspaceProvider.resolve_task_command(task_name)
        } else {
            None
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
        let task_definition = self.task_definition(task_id)?;
        // TODO: command should be optional
        let command = task_definition
            .command
            .clone()
            .or_else(|| {
                workspace_info
                    .package_json
                    .scripts
                    .get(task_id.task())
                    .map(|script| script.as_inner().clone())
            })
            .or_else(|| Self::inferred_provider_command(workspace_info, task_id.task()))
            .unwrap_or_else(|| "<NONEXISTENT>".to_string());

        let expanded_outputs = self
            .hash_tracker
            .expanded_outputs(task_id)
            .unwrap_or_default();

        let framework = self.hash_tracker.framework(task_id).unwrap_or_default();

        let hash = self
            .hash_tracker
            .hash(task_id)
            .unwrap_or_else(|| panic!("hash not found for {task_id}"));

        let expanded_inputs: std::collections::BTreeMap<_, _> = self
            .hash_tracker
            .expanded_inputs(task_id)
            .expect("inputs not found")
            .into_iter()
            .collect();

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
    use turborepo_hash::{LockFilePackagesRef, TurboHash};

    let Some(transitive_dependencies) = transitive_dependencies else {
        return "".into();
    };

    let mut transitive_deps: Vec<&Package> = transitive_dependencies.iter().collect();

    transitive_deps.sort_unstable_by(|a, b| match a.key.cmp(&b.key) {
        std::cmp::Ordering::Equal => a.version.cmp(&b.version),
        other => other,
    });

    LockFilePackagesRef(transitive_deps).hash()
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use serde_json::json;
    use turborepo_repository::{
        discovery::{DiscoveryResponse, PackageDiscovery},
        package_json::PackageJson,
    };
    use turborepo_types::{
        DryRunMode, HashTrackerCacheHitMetadata, HashTrackerDetailedMap, TaskOutputs,
    };

    use super::*;

    struct StaticDiscovery;

    impl PackageDiscovery for StaticDiscovery {
        async fn discover_packages(
            &self,
        ) -> Result<DiscoveryResponse, turborepo_repository::discovery::Error> {
            Ok(DiscoveryResponse {
                package_manager: turborepo_repository::package_manager::PackageManager::Npm,
                workspaces: vec![],
            })
        }

        async fn discover_packages_blocking(
            &self,
        ) -> Result<DiscoveryResponse, turborepo_repository::discovery::Error> {
            self.discover_packages().await
        }
    }

    #[derive(Default)]
    struct MockEngine {
        definitions: HashMap<TaskId<'static>, TaskDefinition>,
        dependencies: HashMap<TaskId<'static>, Vec<TaskId<'static>>>,
        dependents: HashMap<TaskId<'static>, Vec<TaskId<'static>>>,
    }

    impl EngineInfo for MockEngine {
        type TaskIter<'a>
            = std::slice::Iter<'a, TaskId<'static>>
        where
            Self: 'a;

        fn task_definition(&self, task_id: &TaskId<'static>) -> Option<&TaskDefinition> {
            self.definitions.get(task_id)
        }

        fn dependencies(&self, task_id: &TaskId<'static>) -> Option<Self::TaskIter<'_>> {
            self.dependencies.get(task_id).map(|deps| deps.iter())
        }

        fn dependents(&self, task_id: &TaskId<'static>) -> Option<Self::TaskIter<'_>> {
            self.dependents.get(task_id).map(|deps| deps.iter())
        }
    }

    struct MockHashTracker;

    impl HashTrackerInfo for MockHashTracker {
        fn hash(&self, _task_id: &TaskId) -> Option<Arc<str>> {
            Some(Arc::from("test-hash"))
        }

        fn env_vars(&self, _task_id: &TaskId) -> Option<HashTrackerDetailedMap> {
            Some(HashTrackerDetailedMap::default())
        }

        fn cache_status(&self, _task_id: &TaskId) -> Option<HashTrackerCacheHitMetadata> {
            Some(HashTrackerCacheHitMetadata {
                local: false,
                remote: false,
                time_saved: 0,
                sha: None,
                dirty_hash: None,
            })
        }

        fn expanded_outputs(
            &self,
            _task_id: &TaskId,
        ) -> Option<Vec<turbopath::AnchoredSystemPathBuf>> {
            Some(vec![])
        }

        fn framework(&self, _task_id: &TaskId) -> Option<String> {
            None
        }

        fn expanded_inputs(
            &self,
            _task_id: &TaskId,
        ) -> Option<Vec<(turbopath::RelativeUnixPathBuf, String)>> {
            Some(vec![])
        }
    }

    #[derive(Default)]
    struct MockRunOpts;

    impl RunOptsInfo for MockRunOpts {
        fn dry_run(&self) -> Option<DryRunMode> {
            None
        }

        fn single_package(&self) -> bool {
            false
        }

        fn summarize(&self) -> Option<&str> {
            None
        }

        fn framework_inference(&self) -> bool {
            false
        }

        fn pass_through_args(&self) -> &[String] {
            &[]
        }

        fn tasks(&self) -> &[String] {
            &[]
        }
    }

    async fn make_package_graph(
        manifest_path: &str,
        scripts: HashMap<String, String>,
    ) -> PackageGraph {
        let tmp = tempfile::tempdir().unwrap();
        let root = turbopath::AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let manifest = root.join_components(&["workspace", "pkg", manifest_path]);
        let mut package_jsons = HashMap::new();
        package_jsons.insert(
            manifest,
            PackageJson::from_value(json!({
                "name": "pkg",
                "scripts": scripts
            }))
            .unwrap(),
        );

        PackageGraph::builder(root, PackageJson::default())
            .with_package_discovery(StaticDiscovery)
            .with_package_jsons(Some(package_jsons))
            .build()
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn task_summary_uses_provider_inferred_cargo_command() {
        let package_graph = make_package_graph("Cargo.toml", HashMap::new()).await;
        let task_id = TaskId::new("pkg", "build");
        let mut engine = MockEngine::default();
        engine
            .definitions
            .insert(task_id.clone(), TaskDefinition::default());
        let env_at_start = EnvironmentVariableMap::default();
        let run_opts = MockRunOpts;
        let hash_tracker = MockHashTracker;

        let factory = TaskSummaryFactory::new(
            &package_graph,
            &engine,
            &hash_tracker,
            &env_at_start,
            &run_opts,
            EnvMode::Strict,
        );

        let summary = factory.task_summary(task_id, None).unwrap();
        assert_eq!(summary.shared.command, "cargo build");
    }

    #[tokio::test]
    async fn task_summary_prefers_explicit_command_over_provider_inference() {
        let package_graph = make_package_graph("Cargo.toml", HashMap::new()).await;
        let task_id = TaskId::new("pkg", "build");
        let mut engine = MockEngine::default();
        engine.definitions.insert(
            task_id.clone(),
            TaskDefinition {
                command: Some("custom build command".to_string()),
                outputs: TaskOutputs::default(),
                ..Default::default()
            },
        );
        let env_at_start = EnvironmentVariableMap::default();
        let run_opts = MockRunOpts;
        let hash_tracker = MockHashTracker;

        let factory = TaskSummaryFactory::new(
            &package_graph,
            &engine,
            &hash_tracker,
            &env_at_start,
            &run_opts,
            EnvMode::Strict,
        );

        let summary = factory.task_summary(task_id, None).unwrap();
        assert_eq!(summary.shared.command, "custom build command");
    }

    #[tokio::test]
    async fn task_summary_prefers_script_over_provider_inference_for_node() {
        let package_graph = make_package_graph(
            "package.json",
            HashMap::from([("build".to_string(), "echo from-script".to_string())]),
        )
        .await;
        let task_id = TaskId::new("pkg", "build");
        let mut engine = MockEngine::default();
        engine
            .definitions
            .insert(task_id.clone(), TaskDefinition::default());
        let env_at_start = EnvironmentVariableMap::default();
        let run_opts = MockRunOpts;
        let hash_tracker = MockHashTracker;

        let factory = TaskSummaryFactory::new(
            &package_graph,
            &engine,
            &hash_tracker,
            &env_at_start,
            &run_opts,
            EnvMode::Strict,
        );

        let summary = factory.task_summary(task_id, None).unwrap();
        assert_eq!(summary.shared.command, "echo from-script");
    }
}
