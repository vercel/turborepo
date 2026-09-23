use std::collections::HashMap;

use turborepo_env::EnvironmentVariableMap;
use turborepo_repository::package_graph::{PackageGraph, PackageName, PackageTaskContext};
use turborepo_task_id::TaskId;
use turborepo_types::{
    EngineInfo, EnvMode, HashTrackerInfo, RunOptsInfo, TaskDefinition, TaskDefinitionExt,
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
            let relative_log_file = TaskDefinition::workspace_relative_log_file(
                task_id.task(),
                package_context.log_namespace(),
            );
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
    /// knowledge directly. Never rehash closures.
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
            .map(|native_task| match native_task.execution() {
                turborepo_repository::native_tasks::NativeTaskExecution::Aggregate(_) => {
                    "<AGGREGATE>".to_string()
                }
                _ => native_task
                    .display()
                    .map(str::to_string)
                    .unwrap_or_else(|| "<NONEXISTENT>".to_string()),
            })
            .unwrap_or_else(|| "<NONEXISTENT>".to_string()),
    }
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

    struct PlanRunOpts {
        args: Vec<String>,
    }

    impl RunOptsInfo for PlanRunOpts {
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
            &self.args
        }

        fn tasks(&self) -> &[String] {
            &[]
        }
    }

    /// The same package-graph boundary used by run planning, with discovery and
    /// manifests supplied in memory rather than reading files or spawning
    /// tools.
    async fn injected_summary_graph() -> (tempfile::TempDir, PackageGraph) {
        use turborepo_repository::{
            discovery::{DiscoveryResponse, WorkspaceData},
            package_manager::PackageManager,
        };

        let tempdir = tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        let app_path = root.join_components(&["packages", "app", "package.json"]);
        let lib_path = root.join_components(&["packages", "lib", "package.json"]);
        let response = DiscoveryResponse {
            package_manager: PackageManager::Npm,
            workspaces: [app_path.clone(), lib_path.clone()]
                .into_iter()
                .map(|path| WorkspaceData::new(path, None).unwrap())
                .collect(),
        };
        let manifests = HashMap::from([
            (
                app_path,
                PackageJson::from_value(json!({
                    "name": "app", "scripts": {"build": "echo app"},
                    "dependencies": {"lib": "*"}
                }))
                .unwrap(),
            ),
            (
                lib_path,
                PackageJson::from_value(json!({
                    "name": "lib", "scripts": {"build": "echo lib"}
                }))
                .unwrap(),
            ),
        ]);
        let graph = PackageGraph::builder(&root, PackageJson::default())
            .with_package_discovery(move || {
                let response = response.clone();
                async move { Ok(response) }
            })
            .with_package_json_loader(move |path: &turbopath::AbsoluteSystemPath| {
                manifests.get(path).cloned().ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::NotFound, "missing injected manifest")
                        .into()
                })
            })
            .without_external_dependencies()
            .build()
            .await
            .unwrap();
        assert_eq!(
            graph
                .filtering_relationships()
                .transitive_dependencies(&PackageName::from("app"))
                .unwrap(),
            [PackageName::from("lib")]
        );
        (tempdir, graph)
    }

    struct PlanEngine {
        definitions: HashMap<TaskId<'static>, TaskDefinition>,
        dependencies: HashMap<TaskId<'static>, Vec<TaskId<'static>>>,
        dependents: HashMap<TaskId<'static>, Vec<TaskId<'static>>>,
    }

    impl EngineInfo for PlanEngine {
        type TaskIter<'a> = std::slice::Iter<'a, TaskId<'static>>;

        fn task_definition(&self, task: &TaskId<'static>) -> Option<&TaskDefinition> {
            self.definitions.get(task)
        }

        fn dependencies(&self, task: &TaskId<'static>) -> Option<Self::TaskIter<'_>> {
            self.dependencies.get(task).map(|tasks| tasks.iter())
        }

        fn dependents(&self, task: &TaskId<'static>) -> Option<Self::TaskIter<'_>> {
            self.dependents.get(task).map(|tasks| tasks.iter())
        }
    }

    struct PlanHashes {
        hashes: HashMap<String, Arc<str>>,
        inputs: HashMap<String, Vec<(turbopath::RelativeUnixPathBuf, String)>>,
        env: Option<HashTrackerDetailedMap>,
        hit: Option<HashTrackerCacheHitMetadata>,
    }

    impl HashTrackerInfo for PlanHashes {
        fn hash(&self, task: &TaskId) -> Option<Arc<str>> {
            self.hashes.get(&task.to_string()).cloned()
        }

        fn env_vars(&self, _task: &TaskId) -> Option<HashTrackerDetailedMap> {
            self.env.clone()
        }

        fn cache_status(&self, _task: &TaskId) -> Option<HashTrackerCacheHitMetadata> {
            self.hit.clone()
        }

        fn expanded_outputs(&self, _task: &TaskId) -> Option<Vec<AnchoredSystemPathBuf>> {
            None
        }

        fn framework(&self, _task: &TaskId) -> Option<String> {
            None
        }

        fn expanded_inputs(
            &self,
            task: &TaskId,
        ) -> Option<Vec<(turbopath::RelativeUnixPathBuf, String)>> {
            self.inputs.get(&task.to_string()).cloned()
        }
    }

    #[tokio::test]
    async fn injected_dry_run_task_summary_projects_graph_definition_and_hash_facts() {
        let (_tmp, graph) = injected_summary_graph().await;
        let app = TaskId::new("app", "build").into_owned();
        let lib = TaskId::new("lib", "build").into_owned();
        let app_definition = TaskDefinition {
            outputs: turborepo_types::TaskOutputs {
                inclusions: vec!["dist/**".to_string()],
                exclusions: vec!["dist/tmp/**".to_string()],
            },
            env: vec!["API_URL".to_string()],
            pass_through_env: Some(vec!["SECRET_TOKEN".to_string()]),
            topological_dependencies: vec![turborepo_errors::Spanned::new(
                turborepo_task_id::TaskName::from("build"),
            )],
            ..Default::default()
        };
        let engine = PlanEngine {
            definitions: HashMap::from([
                (app.clone(), app_definition),
                (lib.clone(), TaskDefinition::default()),
            ]),
            dependencies: HashMap::from([(app.clone(), vec![lib.clone()])]),
            dependents: HashMap::from([(lib.clone(), vec![app.clone()])]),
        };
        let hashes = PlanHashes {
            hashes: HashMap::from([
                (app.to_string(), Arc::from("planned-hash")),
                (lib.to_string(), Arc::from("lib-hash")),
            ]),
            inputs: HashMap::from([
                (
                    app.to_string(),
                    vec![(
                        turbopath::RelativeUnixPathBuf::new("src/app.ts").unwrap(),
                        "app-file-hash".to_string(),
                    )],
                ),
                (
                    lib.to_string(),
                    vec![(
                        turbopath::RelativeUnixPathBuf::new("src/lib.ts").unwrap(),
                        "lib-file-hash".to_string(),
                    )],
                ),
            ]),
            env: Some(HashTrackerDetailedMap::default()),
            hit: None,
        };
        let environment = EnvironmentVariableMap::from(HashMap::from([(
            "SECRET_TOKEN".to_string(),
            "unprintable-secret".to_string(),
        )]));
        let opts = PlanRunOpts {
            args: vec!["--verbose".to_string()],
        };
        let external = HashMap::from([
            ("app".to_string(), "app-closure".to_string()),
            ("lib".to_string(), "lib-closure".to_string()),
        ]);
        let factory = TaskSummaryFactory::new(
            &graph,
            &engine,
            &hashes,
            &environment,
            &opts,
            EnvMode::Strict,
            Some(&external),
        );
        let app_plan =
            serde_json::to_value(factory.task_summary(app.clone(), None).unwrap()).unwrap();
        assert_eq!(app_plan["taskId"], "app#build");
        assert_eq!(app_plan["command"], "echo app");
        assert_eq!(app_plan["hash"], "planned-hash");
        assert_eq!(app_plan["inputs"]["src/app.ts"], "app-file-hash");
        assert_eq!(app_plan["hashOfExternalDependencies"], "app-closure");
        assert_eq!(app_plan["dependencies"], json!(["lib#build"]));
        assert_eq!(app_plan["dependents"], json!([]));
        assert_eq!(app_plan["cache"]["status"], "MISS");
        assert_eq!(app_plan["cliArguments"], json!(["--verbose"]));
        assert_eq!(app_plan["outputs"], json!(["dist/**"]));
        assert_eq!(app_plan["excludedOutputs"], json!(["dist/tmp/**"]));
        assert_eq!(app_plan["resolvedTaskDefinition"]["cache"], true);
        assert_eq!(
            app_plan["resolvedTaskDefinition"]["dependsOn"],
            json!(["^build"])
        );
        assert_eq!(
            app_plan["resolvedTaskDefinition"]["env"],
            json!(["API_URL"])
        );
        assert_eq!(
            app_plan["environmentVariables"]["specified"]["passThroughEnv"],
            json!(["SECRET_TOKEN"])
        );
        assert!(!app_plan.to_string().contains("unprintable-secret"));
        let lib_plan = serde_json::to_value(factory.task_summary(lib, None).unwrap()).unwrap();
        assert_eq!(lib_plan["command"], "echo lib");
        assert_eq!(lib_plan["hash"], "lib-hash");
        assert_eq!(lib_plan["inputs"]["src/lib.ts"], "lib-file-hash");
        assert!(lib_plan["inputs"].get("src/app.ts").is_none());
        assert_eq!(lib_plan["dependencies"], json!([]));
        assert_eq!(lib_plan["dependents"], json!(["app#build"]));
    }

    #[tokio::test]
    async fn injected_dry_run_task_summary_reports_hits_deferred_hashes_and_missing_facts() {
        let (_tmp, graph) = injected_summary_graph().await;
        let task = TaskId::new("app", "build").into_owned();
        let engine = PlanEngine {
            definitions: HashMap::from([(task.clone(), TaskDefinition::default())]),
            dependencies: HashMap::new(),
            dependents: HashMap::new(),
        };
        let environment = EnvironmentVariableMap::default();
        let external = HashMap::new();
        let mut hashes = PlanHashes {
            hashes: HashMap::from([(task.to_string(), Arc::from("hit-hash"))]),
            inputs: HashMap::from([(task.to_string(), Vec::new())]),
            env: Some(HashTrackerDetailedMap::default()),
            hit: Some(HashTrackerCacheHitMetadata {
                local: true,
                remote: false,
                time_saved: 123,
                sha: None,
                dirty_hash: None,
            }),
        };
        let plan = |hashes: &PlanHashes| {
            TaskSummaryFactory::new(
                &graph,
                &engine,
                hashes,
                &environment,
                &TestRunOpts,
                EnvMode::Strict,
                Some(&external),
            )
            .task_summary(task.clone(), None)
        };
        let hit = serde_json::to_value(plan(&hashes).unwrap()).unwrap();
        assert_eq!(hit["cache"]["status"], "HIT");
        assert_eq!(hit["cache"]["local"], true);
        hashes.hashes.insert(
            task.to_string(),
            Arc::from("Deferred because JIT hashing mode was used."),
        );
        let deferred = serde_json::to_value(plan(&hashes).unwrap()).unwrap();
        assert!(deferred["hash"].is_null());
        assert!(
            deferred["hashReason"]
                .as_str()
                .unwrap()
                .contains("JIT hashing")
        );
        hashes.hashes.remove(&task.to_string());
        assert!(matches!(plan(&hashes), Err(Error::MissingHash(id)) if id == task));
        hashes
            .hashes
            .insert(task.to_string(), Arc::from("restored"));
        hashes.inputs.remove(&task.to_string());
        assert!(matches!(plan(&hashes), Err(Error::MissingExpandedInputs(id)) if id == task));
        hashes.inputs.insert(task.to_string(), Vec::new());
        hashes.env = None;
        assert!(matches!(plan(&hashes), Err(Error::MissingEnvVars(id)) if id == task));
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
    async fn summary_uses_authoritative_package_context() {
        let (_tempdir, graph) = summary_graph().await;
        let app = PackageName::from("app");
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
