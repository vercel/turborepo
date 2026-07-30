use std::collections::HashMap;

use turbopath::AnchoredSystemPath;
use turborepo_env::EnvironmentVariableMap;
use turborepo_repository::package_graph::{PackageGraph, PackageName, PackageTaskContext};
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
    /// Per-package external resolution fingerprints computed for task hashing.
    /// Summaries reuse this exact cache so serialized and OpenTelemetry values
    /// cannot drift from task-hash inputs.
    external_deps_hashes: Option<&'a HashMap<String, String>>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("No workspace found for {0}")]
    MissingWorkspace(String),
    #[error("No external dependency hash found for {0}")]
    MissingExternalDependencyHash(PackageName),
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
        let package_context = self.package_context(&task_id)?;
        let shared = self.shared(&task_id, execution, &package_context, |dep_task_id| {
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
        package_context: &PackageTaskContext<'_>,
        display_task: impl Fn(&TaskId<'static>) -> Option<T> + Copy,
    ) -> Result<SharedTaskSummary<T>, Error> {
        let task_definition = self.task_definition(task_id)?;

        // TODO: command should be optional
        // A resolved `command` override displays as its literal argv —
        // truthful by construction. Otherwise the package's toolchain owns
        // the display string (JavaScript: the script text; Cargo: the cargo
        // invocation), derived from the same tables as execution so display
        // cannot drift from what runs.
        let command = summary_command(package_context, task_definition, task_id.task());

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
            let relative_log_file = workspace_relative_log_file(task_id.task())?;
            Some(
                package_context
                    .directory()
                    .to_owned()
                    .join(&relative_log_file)
                    .to_string(),
            )
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

        let hash_of_external_dependencies = self.hash_of_external_dependencies(task_id)?;

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
            directory: Some(package_context.directory().to_string()),
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

    fn package_context(&self, task_id: &TaskId) -> Result<PackageTaskContext<'_>, Error> {
        let workspace_name = PackageName::from(task_id.package());
        self.package_graph
            .package_task_context(&workspace_name)
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

    /// Resolve the same stored fingerprint task hashing uses.
    ///
    /// Prefer the per-run cache produced for hashing. When that cache is absent
    /// (tests or callers that did not precompute), read package resolution
    /// knowledge directly. Never rehash closures or read
    /// `PackageInfo::external_deps_hash`.
    fn hash_of_external_dependencies(&self, task_id: &TaskId) -> Result<String, Error> {
        let package = PackageName::from(task_id.package());
        if let Some(hashes) = self.external_deps_hashes {
            if let Some(hash) = hashes.get(task_id.package()) {
                return Ok(hash.clone());
            }
            // Single-package hashing leaves the cache empty; preserve the empty
            // serialized fingerprint used by dry-run/summary output.
            if hashes.is_empty() {
                return Ok(String::new());
            }
            return Err(Error::MissingExternalDependencyHash(package));
        }

        self.package_graph
            .package_resolution_states()
            .get(task_id.package())
            .and_then(|state| state.task_hash().map(str::to_string))
            .ok_or(Error::MissingExternalDependencyHash(package))
    }
}

fn summary_command(
    package_context: &PackageTaskContext<'_>,
    task_definition: &TaskDefinition,
    task: &str,
) -> String {
    match &task_definition.command {
        Some(turborepo_types::TaskCommandOverride::Argv(argv)) => argv.join(" "),
        Some(turborepo_types::TaskCommandOverride::OptOut) => "<OPT OUT>".to_string(),
        None => package_context
            .native_tasks()
            .get(task)
            .and_then(|native_task| native_task.display().map(str::to_string))
            .unwrap_or_else(|| "<NONEXISTENT>".to_string()),
    }
}

/// Get the workspace-relative path to the log file for a task.
fn workspace_relative_log_file(
    task_name: &str,
) -> Result<turbopath::AnchoredSystemPathBuf, turbopath::PathError> {
    let log_dir = AnchoredSystemPath::new(LOG_DIR)?;
    Ok(log_dir.join_component(&task_log_filename(task_name)))
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::Path, sync::Arc};

    use serde_json::json;
    use tempfile::tempdir;
    use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
    use turborepo_repository::{
        package_graph::PackageName, package_json::PackageJson, toolchain::ToolchainId,
    };
    use turborepo_types::{
        DryRunMode, HashTrackerCacheHitMetadata, HashTrackerDetailedMap, HashTrackerInfo,
        RunOptsInfo,
    };

    use super::*;

    struct TestEngine {
        definitions: HashMap<TaskId<'static>, TaskDefinition>,
        edges: Vec<TaskId<'static>>,
    }

    impl EngineInfo for TestEngine {
        type TaskIter<'a> = std::slice::Iter<'a, TaskId<'static>>;

        fn task_definition(&self, task_id: &TaskId<'static>) -> Option<&TaskDefinition> {
            self.definitions.get(task_id)
        }

        fn dependencies(&self, _task_id: &TaskId<'static>) -> Option<Self::TaskIter<'_>> {
            Some(self.edges.iter())
        }

        fn dependents(&self, _task_id: &TaskId<'static>) -> Option<Self::TaskIter<'_>> {
            Some(self.edges.iter())
        }
    }

    struct TestHashes;

    impl HashTrackerInfo for TestHashes {
        fn hash(&self, _task_id: &TaskId) -> Option<Arc<str>> {
            Some(Arc::from("hash"))
        }

        fn env_vars(&self, _task_id: &TaskId) -> Option<HashTrackerDetailedMap> {
            Some(HashTrackerDetailedMap::default())
        }

        fn cache_status(&self, _task_id: &TaskId) -> Option<HashTrackerCacheHitMetadata> {
            None
        }

        fn expanded_outputs(&self, _task_id: &TaskId) -> Option<Vec<AnchoredSystemPathBuf>> {
            None
        }

        fn framework(&self, _task_id: &TaskId) -> Option<String> {
            None
        }

        fn expanded_inputs(
            &self,
            _task_id: &TaskId,
        ) -> Option<Vec<(turbopath::RelativeUnixPathBuf, String)>> {
            Some(Vec::new())
        }
    }

    struct TestRunOpts;

    impl RunOptsInfo for TestRunOpts {
        fn dry_run(&self) -> Option<DryRunMode> {
            Some(DryRunMode::Json)
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

    async fn summary_graph() -> (tempfile::TempDir, PackageGraph) {
        let tempdir = tempdir().unwrap();
        let repo_root =
            AbsoluteSystemPathBuf::new(tempdir.path().to_string_lossy().to_string()).unwrap();
        let root_json = json!({
            "name": "root",
            "packageManager": "npm@10.0.0",
            "workspaces": ["packages/*"]
        });
        repo_root
            .join_component("package.json")
            .create_with_contents(serde_json::to_string(&root_json).unwrap())
            .unwrap();
        let app_json = repo_root.join_components(&["packages", "app", "package.json"]);
        app_json.ensure_dir().unwrap();
        app_json
            .create_with_contents(r#"{"name":"app","scripts":{"build":"echo build"}}"#)
            .unwrap();
        let graph = PackageGraph::builder(&repo_root, PackageJson::from_value(root_json).unwrap())
            .build()
            .await
            .unwrap();
        (tempdir, graph)
    }

    #[tokio::test]
    async fn summary_uses_authoritative_path_and_toolchain_provenance() {
        let (_tempdir, graph) = summary_graph().await;
        let app = PackageName::from("app");
        assert_eq!(
            graph.package_task_context(&app).unwrap().toolchain(),
            Some(&ToolchainId::JAVASCRIPT)
        );
        let task_id = TaskId::new("app", "build").into_owned();
        let engine = TestEngine {
            definitions: HashMap::from([(task_id.clone(), TaskDefinition::default())]),
            edges: Vec::new(),
        };
        let environment = EnvironmentVariableMap::default();
        let external_hashes = HashMap::from([("app".to_string(), "2ccf3983a6195c83".to_string())]);
        let factory = TaskSummaryFactory::new(
            &graph,
            &engine,
            &TestHashes,
            &environment,
            &TestRunOpts,
            EnvMode::Strict,
            Some(&external_hashes),
        );

        let summary = factory.task_summary(task_id, None).unwrap();
        assert_eq!(summary.shared.command, "echo build");
        let app_directory = Path::new("packages").join("app");
        assert_eq!(
            summary.shared.directory.as_deref().map(Path::new),
            Some(app_directory.as_path())
        );
        let app_log = app_directory.join(".turbo").join("turbo-build.log");
        assert_eq!(
            summary.shared.log_file.as_deref().map(Path::new),
            Some(app_log.as_path())
        );
        assert_eq!(
            summary.shared.hash_of_external_dependencies,
            "2ccf3983a6195c83"
        );
    }

    #[tokio::test]
    async fn summary_uses_knowledge_when_package_payload_is_missing() {
        let (_tempdir, mut graph) = summary_graph().await;
        let app = PackageName::from("app");
        assert!(graph.remove_package_info_for_test(&app).is_some());
        let task_id = TaskId::new("app", "build").into_owned();
        let engine = TestEngine {
            definitions: HashMap::from([(task_id.clone(), TaskDefinition::default())]),
            edges: Vec::new(),
        };
        let environment = EnvironmentVariableMap::default();
        let factory = TaskSummaryFactory::new(
            &graph,
            &engine,
            &TestHashes,
            &environment,
            &TestRunOpts,
            EnvMode::Strict,
            None,
        );

        let summary = factory.task_summary(task_id, None).unwrap();
        assert_eq!(summary.package, app.as_str());
        assert_eq!(summary.shared.hash_of_external_dependencies, "");
    }

    #[tokio::test]
    async fn summary_uses_resolution_fingerprint_without_hash_cache() {
        let (_tempdir, graph) = summary_graph().await;
        let app = PackageName::from("app");
        let expected = graph
            .package_resolution_states()
            .get(app.as_str())
            .and_then(|state| state.task_hash())
            .expect("resolution knowledge must expose a task-hash fingerprint")
            .to_string();
        let task_id = TaskId::new("app", "build").into_owned();
        let engine = TestEngine {
            definitions: HashMap::from([(task_id.clone(), TaskDefinition::default())]),
            edges: Vec::new(),
        };
        let environment = EnvironmentVariableMap::default();
        let factory = TaskSummaryFactory::new(
            &graph,
            &engine,
            &TestHashes,
            &environment,
            &TestRunOpts,
            EnvMode::Strict,
            None,
        );

        let summary = factory.task_summary(task_id, None).unwrap();
        assert_eq!(summary.shared.hash_of_external_dependencies, expected);
        // No-lockfile JavaScript graphs remain explicitly unavailable/empty,
        // never missing, so summaries preserve the empty serialized fingerprint.
        assert_eq!(summary.shared.hash_of_external_dependencies, "");
    }

    #[tokio::test]
    async fn summary_fails_closed_when_hash_cache_misses_package() {
        let (_tempdir, graph) = summary_graph().await;
        let app = PackageName::from("app");
        let task_id = TaskId::new("app", "build").into_owned();
        let engine = TestEngine {
            definitions: HashMap::from([(task_id.clone(), TaskDefinition::default())]),
            edges: Vec::new(),
        };
        let environment = EnvironmentVariableMap::default();
        let external_hashes = HashMap::from([("util".to_string(), "deadbeef".to_string())]);
        let factory = TaskSummaryFactory::new(
            &graph,
            &engine,
            &TestHashes,
            &environment,
            &TestRunOpts,
            EnvMode::Strict,
            Some(&external_hashes),
        );

        assert!(matches!(
            factory.task_summary(task_id, None),
            Err(Error::MissingExternalDependencyHash(name)) if name == app
        ));
    }

    #[tokio::test]
    async fn summary_single_package_empty_cache_serializes_empty_fingerprint() {
        let (_tempdir, graph) = summary_graph().await;
        let task_id = TaskId::new("app", "build").into_owned();
        let engine = TestEngine {
            definitions: HashMap::from([(task_id.clone(), TaskDefinition::default())]),
            edges: Vec::new(),
        };
        let environment = EnvironmentVariableMap::default();
        let external_hashes = HashMap::new();
        let factory = TaskSummaryFactory::new(
            &graph,
            &engine,
            &TestHashes,
            &environment,
            &TestRunOpts,
            EnvMode::Strict,
            Some(&external_hashes),
        );

        let summary = factory.task_summary(task_id, None).unwrap();
        assert_eq!(summary.shared.hash_of_external_dependencies, "");
    }
}
