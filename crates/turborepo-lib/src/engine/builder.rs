use std::collections::{HashMap, HashSet, VecDeque};

use convert_case::{Case, Casing};
use itertools::Itertools;
use miette::{Diagnostic, NamedSource, SourceSpan};
use turbopath::AbsoluteSystemPath;
use turborepo_errors::{Spanned, TURBO_SITE};
use turborepo_graph_utils as graph;
use turborepo_repository::package_graph::{PackageGraph, PackageName, PackageNode, ROOT_PKG_NAME};

use super::Engine;
use crate::{
    config,
    run::task_id::{TaskId, TaskName},
    task_graph::TaskDefinition,
    turbo_json::{
        validate_extends, validate_no_package_task_syntax, RawTaskDefinition, TurboJsonLoader,
    },
};

#[derive(Debug, thiserror::Error, Diagnostic)]
pub enum MissingTaskError {
    #[error("could not find task `{name}` in project")]
    MissingTaskDefinition {
        name: String,
        #[label]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource,
    },
    #[error("could not find package `{name}` in project")]
    MissingPackage { name: String },
}

#[derive(Debug, thiserror::Error, Diagnostic)]
pub enum Error {
    #[error("missing tasks in project")]
    MissingTasks(#[related] Vec<MissingTaskError>),
    #[error("No package.json for {workspace}")]
    MissingPackageJson { workspace: PackageName },
    #[error(
        "{task_id} needs an entry in turbo.json before it can be depended on because it is a task \
         declared in the root package.json"
    )]
    #[diagnostic(
        code(missing_root_task_in_turbo_json),
        url(
            "{}/messages/{}",
            TURBO_SITE, self.code().unwrap().to_string().to_case(Case::Kebab)
        )
    )]
    MissingRootTaskInTurboJson {
        task_id: String,
        #[label("add an entry in turbo.json for this task")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource,
    },
    #[error("Could not find package \"{package}\" from task \"{task_id}\" in project")]
    MissingPackageFromTask {
        #[label]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource,
        package: String,
        task_id: String,
    },
    #[error("Could not find \"{task_id}\" in root turbo.json or \"{task_name}\" in package")]
    MissingPackageTask {
        #[label]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource,
        task_id: String,
        task_name: String,
    },
    #[error(transparent)]
    #[diagnostic(transparent)]
    Config(#[from] crate::config::Error),
    #[error("invalid turbo json")]
    Validation {
        #[related]
        errors: Vec<config::Error>,
    },
    #[error(transparent)]
    Graph(#[from] graph::Error),
    #[error("invalid task name: {reason}")]
    InvalidTaskName {
        #[label]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource,
        task_name: String,
        reason: String,
    },
}

pub struct EngineBuilder<'a> {
    repo_root: &'a AbsoluteSystemPath,
    package_graph: &'a PackageGraph,
    turbo_json_loader: Option<TurboJsonLoader>,
    is_single: bool,
    workspaces: Vec<PackageName>,
    tasks: Vec<Spanned<TaskName<'static>>>,
    root_enabled_tasks: HashSet<TaskName<'static>>,
    tasks_only: bool,
    add_all_tasks: bool,
    should_validate_engine: bool,
}

impl<'a> EngineBuilder<'a> {
    pub fn new(
        repo_root: &'a AbsoluteSystemPath,
        package_graph: &'a PackageGraph,
        turbo_json_loader: TurboJsonLoader,
        is_single: bool,
    ) -> Self {
        Self {
            repo_root,
            package_graph,
            turbo_json_loader: Some(turbo_json_loader),
            is_single,
            workspaces: Vec::new(),
            tasks: Vec::new(),
            root_enabled_tasks: HashSet::new(),
            tasks_only: false,
            add_all_tasks: false,
            should_validate_engine: true,
        }
    }

    pub fn with_tasks_only(mut self, tasks_only: bool) -> Self {
        self.tasks_only = tasks_only;
        self
    }

    pub fn with_root_tasks<I: IntoIterator<Item = TaskName<'static>>>(mut self, tasks: I) -> Self {
        self.root_enabled_tasks = tasks
            .into_iter()
            .filter(|name| name.package() == Some(ROOT_PKG_NAME))
            .map(|name| name.into_non_workspace_task())
            .collect();
        self
    }

    pub fn with_workspaces(mut self, workspaces: Vec<PackageName>) -> Self {
        self.workspaces = workspaces;
        self
    }

    pub fn with_tasks<I: IntoIterator<Item = Spanned<TaskName<'static>>>>(
        mut self,
        tasks: I,
    ) -> Self {
        self.tasks = tasks.into_iter().collect();
        self
    }

    /// If set, we will include all tasks in the graph, even if they are not
    /// specified
    pub fn add_all_tasks(mut self) -> Self {
        self.add_all_tasks = true;
        self
    }

    pub fn do_not_validate_engine(mut self) -> Self {
        self.should_validate_engine = false;
        self
    }

    // Returns the set of allowed tasks that can be run if --only is used
    // The set is exactly the product of the packages in filter and tasks specified
    // by CLI
    fn allowed_tasks(&self) -> Option<HashSet<TaskId<'static>>> {
        if self.tasks_only {
            Some(
                self.workspaces
                    .iter()
                    .cartesian_product(self.tasks.iter())
                    .map(|(package, task_name)| {
                        task_name
                            .task_id()
                            .unwrap_or(TaskId::new(package.as_ref(), task_name.task()))
                            .into_owned()
                    })
                    .collect(),
            )
        } else {
            None
        }
    }

    pub fn build(mut self) -> Result<super::Engine, Error> {
        // If there are no affected packages, we don't need to go through all this work
        // we can just exit early.
        // TODO(mehulkar): but we still need to validate bad task names?
        if self.workspaces.is_empty() {
            return Ok(Engine::default().seal());
        }

        let mut turbo_json_loader = self
            .turbo_json_loader
            .take()
            .expect("engine builder cannot be constructed without TurboJsonLoader");
        let mut missing_tasks: HashMap<&TaskName<'_>, Spanned<()>> =
            HashMap::from_iter(self.tasks.iter().map(|spanned| spanned.as_ref().split()));
        let mut traversal_queue = VecDeque::with_capacity(1);
        let tasks: Vec<Spanned<TaskName<'static>>> = if self.add_all_tasks {
            let mut tasks = Vec::new();
            if let Ok(turbo_json) = turbo_json_loader.load(&PackageName::Root) {
                tasks.extend(
                    turbo_json
                        .tasks
                        .keys()
                        .map(|task| Spanned::new(task.clone())),
                );
            }

            for workspace in self.workspaces.iter() {
                let Ok(turbo_json) = turbo_json_loader.load(workspace) else {
                    continue;
                };

                tasks.extend(
                    turbo_json
                        .tasks
                        .keys()
                        .map(|task| Spanned::new(task.clone())),
                );
            }

            tasks
        } else {
            self.tasks.clone()
        };

        for (workspace, task) in self.workspaces.iter().cartesian_product(tasks.iter()) {
            let task_id = task
                .task_id()
                .unwrap_or_else(|| TaskId::new(workspace.as_ref(), task.task()));

            if Self::has_task_definition(&mut turbo_json_loader, workspace, task, &task_id)? {
                missing_tasks.remove(task.as_inner());

                // Even if a task definition was found, we _only_ want to add it as an entry
                // point to the task graph (i.e. the traversalQueue), if
                // it's:
                // - A task from the non-root workspace (i.e. tasks from every other workspace)
                // - A task that we *know* is rootEnabled task (in which case, the root
                //   workspace is acceptable)
                if !matches!(workspace, PackageName::Root) || self.root_enabled_tasks.contains(task)
                {
                    let task_id = task.to(task_id);
                    traversal_queue.push_back(task_id);
                }
            }
        }

        if !missing_tasks.is_empty() {
            let missing_pkgs: HashMap<_, _> = missing_tasks
                .iter()
                .filter_map(|(task, _)| {
                    let pkg = task.package()?;
                    let missing_pkg = self
                        .package_graph
                        .package_info(&PackageName::from(pkg))
                        .is_none();
                    missing_pkg.then(|| (task.to_string(), pkg.to_string()))
                })
                .collect();
            let mut missing_tasks = missing_tasks
                .into_iter()
                .map(|(task_name, span)| (task_name.to_string(), span))
                .collect::<Vec<_>>();
            // We sort the tasks mostly to keep it deterministic for our tests
            missing_tasks.sort_by(|a, b| a.0.cmp(&b.0));
            let errors = missing_tasks
                .into_iter()
                .map(|(name, span)| {
                    if let Some(pkg) = missing_pkgs.get(&name) {
                        MissingTaskError::MissingPackage { name: pkg.clone() }
                    } else {
                        let (span, text) = span.span_and_text("turbo.json");
                        MissingTaskError::MissingTaskDefinition { name, span, text }
                    }
                })
                .collect();

            return Err(Error::MissingTasks(errors));
        }

        let allowed_tasks = self.allowed_tasks();

        let mut visited = HashSet::new();
        let mut engine = Engine::default();

        while let Some(task_id) = traversal_queue.pop_front() {
            {
                let (task_id, span) = task_id.clone().split();
                engine.add_task_location(task_id.into_owned(), span);
            }

            if task_id.package() == ROOT_PKG_NAME
                && !self
                    .root_enabled_tasks
                    .contains(&task_id.as_non_workspace_task_name())
            {
                let (span, text) = task_id.span_and_text("turbo.json");
                return Err(Error::MissingRootTaskInTurboJson {
                    span,
                    text,
                    task_id: task_id.to_string(),
                });
            }

            validate_task_name(task_id.to(task_id.task()))?;

            if task_id.package() != ROOT_PKG_NAME
                && self
                    .package_graph
                    .package_json(&PackageName::from(task_id.package()))
                    .is_none()
            {
                // If we have a pkg it should be in PackageGraph.
                // If we're hitting this error something has gone wrong earlier when building
                // PackageGraph or the package really doesn't exist and
                // turbo.json is misconfigured.
                let (span, text) = task_id.span_and_text("turbo.json");
                return Err(Error::MissingPackageFromTask {
                    span,
                    text,
                    package: task_id.package().to_string(),
                    task_id: task_id.to_string(),
                });
            }

            let task_definition = self.task_definition(
                &mut turbo_json_loader,
                &task_id,
                &task_id.as_non_workspace_task_name(),
            )?;

            // Skip this iteration of the loop if we've already seen this taskID
            if visited.contains(task_id.as_inner()) {
                continue;
            }

            visited.insert(task_id.as_inner().clone());

            // Note that the Go code has a whole if/else statement for putting stuff into
            // deps or calling e.AddDep the bool is cannot be true so we skip to
            // just doing deps
            let deps = task_definition
                .task_dependencies
                .iter()
                .map(|spanned| spanned.as_ref().split())
                .collect::<HashMap<_, _>>();
            let topo_deps = task_definition
                .topological_dependencies
                .iter()
                .map(|spanned| spanned.as_ref().split())
                .collect::<HashMap<_, _>>();

            // Don't ask why, but for some reason we refer to the source as "to"
            // and the target node as "from"
            let to_task_id = task_id.as_inner().clone().into_owned();
            let to_task_index = engine.get_index(&to_task_id);

            let dep_pkgs = self
                .package_graph
                .immediate_dependencies(&PackageNode::Workspace(to_task_id.package().into()));

            let mut has_deps = false;
            let mut has_topo_deps = false;

            topo_deps
                .iter()
                .cartesian_product(dep_pkgs.iter().flatten())
                .for_each(|((from, span), dependency_workspace)| {
                    // We don't need to add an edge from the root node if we're in this branch
                    if let PackageNode::Workspace(dependency_workspace) = dependency_workspace {
                        let from_task_id = TaskId::from_graph(dependency_workspace, from);
                        if let Some(allowed_tasks) = &allowed_tasks {
                            if !allowed_tasks.contains(&from_task_id) {
                                return;
                            }
                        }
                        let from_task_index = engine.get_index(&from_task_id);
                        has_topo_deps = true;
                        engine
                            .task_graph
                            .add_edge(to_task_index, from_task_index, ());
                        let from_task_id = span.to(from_task_id);
                        traversal_queue.push_back(from_task_id);
                    }
                });

            for (dep, span) in deps {
                let from_task_id = dep
                    .task_id()
                    .unwrap_or_else(|| TaskId::new(to_task_id.package(), dep.task()))
                    .into_owned();
                if let Some(allowed_tasks) = &allowed_tasks {
                    if !allowed_tasks.contains(&from_task_id) {
                        continue;
                    }
                }
                has_deps = true;
                let from_task_index = engine.get_index(&from_task_id);
                engine
                    .task_graph
                    .add_edge(to_task_index, from_task_index, ());
                let from_task_id = span.to(from_task_id);
                traversal_queue.push_back(from_task_id);
            }

            engine.add_definition(task_id.as_inner().clone().into_owned(), task_definition);
            if !has_deps && !has_topo_deps {
                engine.connect_to_root(&to_task_id);
            }
        }

        graph::validate_graph(&engine.task_graph)?;

        Ok(engine.seal())
    }

    // Helper methods used when building the engine

    fn has_task_definition(
        loader: &mut TurboJsonLoader,
        workspace: &PackageName,
        task_name: &TaskName<'static>,
        task_id: &TaskId,
    ) -> Result<bool, Error> {
        let turbo_json = loader.load(workspace).map_or_else(
            |err| {
                if matches!(err, config::Error::NoTurboJSON)
                    && !matches!(workspace, PackageName::Root)
                {
                    Ok(None)
                } else {
                    Err(err)
                }
            },
            |turbo_json| Ok(Some(turbo_json)),
        )?;

        let Some(turbo_json) = turbo_json else {
            // If there was no turbo.json in the workspace, fallback to the root turbo.json
            return Self::has_task_definition(loader, &PackageName::Root, task_name, task_id);
        };

        let task_id_as_name = task_id.as_task_name();
        if
        // See if pkg#task is defined e.g. `docs#build`. This can only happen in root turbo.json
        turbo_json.tasks.contains_key(&task_id_as_name)
            // See if task is defined e.g. `build`. This can happen in root or workspace turbo.json
            // This will fail if the user provided a task id e.g. turbo `docs#build`
            || turbo_json.tasks.contains_key(task_name)
            // If user provided a task id, then we see if the task is defined
            // e.g. `docs#build` should resolve if there's a `build` in root turbo.json or docs workspace level turbo.json
            || (matches!(workspace, PackageName::Root) && turbo_json.tasks.contains_key(&TaskName::from(task_name.task())))
            || (workspace == &PackageName::from(task_id.package()) && turbo_json.tasks.contains_key(&TaskName::from(task_name.task())))
        {
            Ok(true)
        } else if !matches!(workspace, PackageName::Root) {
            Self::has_task_definition(loader, &PackageName::Root, task_name, task_id)
        } else {
            Ok(false)
        }
    }

    fn task_definition(
        &self,
        turbo_json_loader: &mut TurboJsonLoader,
        task_id: &Spanned<TaskId>,
        task_name: &TaskName,
    ) -> Result<TaskDefinition, Error> {
        let raw_task_definition = RawTaskDefinition::from_iter(self.task_definition_chain(
            turbo_json_loader,
            task_id,
            task_name,
        )?);

        Ok(TaskDefinition::try_from(raw_task_definition)?)
    }

    fn task_definition_chain(
        &self,
        turbo_json_loader: &mut TurboJsonLoader,
        task_id: &Spanned<TaskId>,
        task_name: &TaskName,
    ) -> Result<Vec<RawTaskDefinition>, Error> {
        let mut task_definitions = Vec::new();

        let root_turbo_json = turbo_json_loader.load(&PackageName::Root)?;

        if let Some(root_definition) = root_turbo_json.task(task_id, task_name) {
            task_definitions.push(root_definition)
        }

        if self.is_single {
            return match task_definitions.is_empty() {
                true => {
                    let (span, text) = task_id.span_and_text("turbo.json");
                    Err(Error::MissingRootTaskInTurboJson {
                        span,
                        text,
                        task_id: task_id.to_string(),
                    })
                }
                false => Ok(task_definitions),
            };
        }

        if task_id.package() != ROOT_PKG_NAME {
            match turbo_json_loader.load(&PackageName::from(task_id.package())) {
                Ok(workspace_json) => {
                    let validation_errors = workspace_json
                        .validate(&[validate_no_package_task_syntax, validate_extends]);
                    if !validation_errors.is_empty() {
                        return Err(Error::Validation {
                            errors: validation_errors,
                        });
                    }

                    if let Some(workspace_def) = workspace_json.tasks.get(task_name) {
                        task_definitions.push(workspace_def.value.clone());
                    }
                }
                Err(config::Error::NoTurboJSON) => (),
                Err(e) => {
                    return Err(e.into());
                }
            }
        }

        if task_definitions.is_empty() && self.should_validate_engine {
            let (span, text) = task_id.span_and_text("turbo.json");
            return Err(Error::MissingPackageTask {
                span,
                text,
                task_id: task_id.to_string(),
                task_name: task_name.to_string(),
            });
        }

        Ok(task_definitions)
    }
}

impl Error {
    fn is_missing_turbo_json(&self) -> bool {
        matches!(self, Self::Config(crate::config::Error::NoTurboJSON))
    }
}

// If/when we decide to be stricter about task names,
// we can expand the patterns here.
const INVALID_TOKENS: &[&str] = &["$colon$"];

fn validate_task_name(task: Spanned<&str>) -> Result<(), Error> {
    INVALID_TOKENS
        .iter()
        .find(|token| task.contains(**token))
        .map(|found_token| {
            let (span, text) = task.span_and_text("turbo.json");
            Err(Error::InvalidTaskName {
                span,
                text,
                task_name: task.to_string(),
                reason: format!("task contains invalid string '{found_token}'"),
            })
        })
        .unwrap_or(Ok(()))
}

#[cfg(test)]
mod test {
    use std::assert_matches::assert_matches;

    use insta::{assert_json_snapshot, assert_snapshot};
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use tempfile::TempDir;
    use test_case::test_case;
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_lockfiles::Lockfile;
    use turborepo_repository::{
        discovery::PackageDiscovery, package_json::PackageJson, package_manager::PackageManager,
    };

    use super::*;
    use crate::{
        engine::TaskNode,
        turbo_json::{RawTurboJson, TurboJson},
    };

    // Only used to prevent package graph construction from attempting to read
    // lockfile from disk
    #[derive(Debug)]
    struct MockLockfile;
    impl Lockfile for MockLockfile {
        fn resolve_package(
            &self,
            _workspace_path: &str,
            _name: &str,
            _version: &str,
        ) -> Result<Option<turborepo_lockfiles::Package>, turborepo_lockfiles::Error> {
            unreachable!()
        }

        fn all_dependencies(
            &self,
            _key: &str,
        ) -> Result<Option<HashMap<String, String>>, turborepo_lockfiles::Error> {
            unreachable!()
        }

        fn subgraph(
            &self,
            _workspace_packages: &[String],
            _packages: &[String],
        ) -> Result<Box<dyn Lockfile>, turborepo_lockfiles::Error> {
            unreachable!()
        }

        fn encode(&self) -> Result<Vec<u8>, turborepo_lockfiles::Error> {
            unreachable!()
        }

        fn global_change(&self, _other: &dyn Lockfile) -> bool {
            unreachable!()
        }

        fn turbo_version(&self) -> Option<String> {
            None
        }
    }

    struct MockDiscovery;
    impl PackageDiscovery for MockDiscovery {
        async fn discover_packages(
            &self,
        ) -> Result<
            turborepo_repository::discovery::DiscoveryResponse,
            turborepo_repository::discovery::Error,
        > {
            Ok(turborepo_repository::discovery::DiscoveryResponse {
                package_manager: PackageManager::Npm,
                workspaces: vec![], // we don't care about this
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

    macro_rules! package_jsons {
        {$root:expr, $($name:expr => $deps:expr),+} => {
            {
                let mut _map = HashMap::new();
                $(
                    let path = $root.join_components(&["packages", $name, "package.json"]);
                    let dependencies = Some($deps.iter().map(|dep: &&str| (dep.to_string(), "workspace:*".to_string())).collect());
                    let package_json = PackageJson { name: Some($name.to_string()), dependencies, ..Default::default() };
                    _map.insert(path, package_json);
                )+
                _map
            }
        };
    }

    fn mock_package_graph(
        repo_root: &AbsoluteSystemPath,
        jsons: HashMap<AbsoluteSystemPathBuf, PackageJson>,
    ) -> PackageGraph {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(
            PackageGraph::builder(repo_root, PackageJson::default())
                .with_package_discovery(MockDiscovery)
                .with_lockfile(Some(Box::new(MockLockfile)))
                .with_package_jsons(Some(jsons))
                .build(),
        )
        .unwrap()
    }

    fn turbo_json(value: serde_json::Value) -> TurboJson {
        let json_text = serde_json::to_string(&value).unwrap();
        let raw = RawTurboJson::parse(&json_text, "").unwrap();
        TurboJson::try_from(raw).unwrap()
    }

    #[test_case(PackageName::Root, "build", "//#build", true ; "root task")]
    #[test_case(PackageName::from("a"), "build", "a#build", true ; "workspace task in root")]
    #[test_case(PackageName::from("b"), "build", "b#build", true ; "workspace task in workspace")]
    #[test_case(PackageName::from("b"), "test", "b#test", true ; "task missing from workspace")]
    #[test_case(PackageName::from("c"), "missing", "c#missing", false ; "task missing")]
    #[test_case(PackageName::from("c"), "c#curse", "c#curse", true ; "root defined task")]
    #[test_case(PackageName::from("b"), "c#curse", "c#curse", true ; "non-workspace root defined task")]
    #[test_case(PackageName::from("b"), "b#special", "b#special", true ; "workspace defined task")]
    #[test_case(PackageName::from("c"), "b#special", "b#special", false ; "non-workspace defined task")]
    fn test_task_definition(
        workspace: PackageName,
        task_name: &'static str,
        task_id: &'static str,
        expected: bool,
    ) {
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "test": { "inputs": ["testing"] },
                        "build": { "inputs": ["primary"] },
                        "a#build": { "inputs": ["special"] },
                        "c#curse": {},
                    }
                })),
            ),
            (
                PackageName::from("b"),
                turbo_json(json!({
                    "tasks": {
                        "build": { "inputs": ["outer"]},
                        "special": {},
                    }
                })),
            ),
        ]
        .into_iter()
        .collect();
        let mut loader = TurboJsonLoader::noop(turbo_jsons);
        let task_name = TaskName::from(task_name);
        let task_id = TaskId::try_from(task_id).unwrap();

        let has_def =
            EngineBuilder::has_task_definition(&mut loader, &workspace, &task_name, &task_id)
                .unwrap();
        assert_eq!(has_def, expected);
    }

    macro_rules! deps {
        {} => {
            HashMap::new()
        };
        {$($key:expr => $value:expr),*} => {
            {
                let mut _map = HashMap::new();
                $(
                let key = TaskId::try_from($key).unwrap();
                let value = $value.iter().copied().map(|x| {
                    if x == "___ROOT___" {
                        TaskNode::Root
                    } else {
                        TaskNode::Task(TaskId::try_from(x).unwrap())
                    }
                }).collect::<HashSet<_>>();
                _map.insert(key, value);
                )*
                _map
            }
        };
    }

    fn all_dependencies(engine: &Engine) -> HashMap<TaskId<'static>, HashSet<TaskNode>> {
        engine
            .task_lookup
            .keys()
            .filter_map(|task_id| {
                let deps = engine.dependencies(task_id)?;
                Some((task_id.clone(), deps.into_iter().cloned().collect()))
            })
            .collect()
    }

    #[test]
    fn test_default_engine() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "a" => [],
                "b" => [],
                "c" => ["a", "b"]
            },
        );
        let turbo_jsons = vec![(
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "test": { "dependsOn": ["^build", "prepare"] },
                    "build": { "dependsOn": ["^build", "prepare"] },
                    "prepare": {},
                    "side-quest": { "dependsOn": ["prepare"] },
                }
            })),
        )]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, loader, false)
            .with_tasks(Some(Spanned::new(TaskName::from("test"))))
            .with_workspaces(vec![
                PackageName::from("a"),
                PackageName::from("b"),
                PackageName::from("c"),
            ])
            .build()
            .unwrap();

        let expected = deps! {
            "a#test" => ["a#prepare"],
            "a#build" => ["a#prepare"],
            "a#prepare" => ["___ROOT___"],
            "b#test" => ["b#prepare"],
            "b#build" => ["b#prepare"],
            "b#prepare" => ["___ROOT___"],
            "c#prepare" => ["___ROOT___"],
            "c#test" => ["a#build", "b#build", "c#prepare"]
        };
        assert_eq!(all_dependencies(&engine), expected);
    }

    #[test]
    fn test_dependencies_on_unspecified_packages() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        // app1 -> libA
        //              \
        //                > libB -> libD
        //              /
        //       app2 <
        //              \ libC
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app1" => ["libA"],
                "app2" => ["libB", "libC"],
                "libA" => ["libB"],
                "libB" => ["libD"],
                "libC" => [],
                "libD" => []
            },
        );
        let turbo_jsons = vec![(
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "test": { "dependsOn": ["^build"] },
                    "build": { "dependsOn": ["^build"] },
                }
            })),
        )]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, loader, false)
            .with_tasks(Some(Spanned::new(TaskName::from("test"))))
            .with_workspaces(vec![PackageName::from("app2")])
            .build()
            .unwrap();

        let expected = deps! {
            "app2#test" => ["libB#build", "libC#build"],
            "libB#build" => ["libD#build"],
            "libC#build" => ["___ROOT___"],
            "libD#build" => ["___ROOT___"]
        };
        assert_eq!(all_dependencies(&engine), expected);
    }

    #[test]
    fn test_run_package_task() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app1" => ["libA"],
                "libA" => []
            },
        );
        let turbo_jsons = vec![(
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": { "dependsOn": ["^build"] },
                    "app1#special": { "dependsOn": ["^build"] },
                }
            })),
        )]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, loader, false)
            .with_tasks(Some(Spanned::new(TaskName::from("special"))))
            .with_workspaces(vec![PackageName::from("app1"), PackageName::from("libA")])
            .build()
            .unwrap();

        let expected = deps! {
            "app1#special" => ["libA#build"],
            "libA#build" => ["___ROOT___"]
        };
        assert_eq!(all_dependencies(&engine), expected);
    }

    #[test]
    fn test_include_root_tasks() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app1" => ["libA"],
                "libA" => []
            },
        );
        let turbo_jsons = vec![(
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": { "dependsOn": ["^build"] },
                    "test": { "dependsOn": ["^build"] },
                    "//#test": {},
                }
            })),
        )]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, loader, false)
            .with_tasks(vec![
                Spanned::new(TaskName::from("build")),
                Spanned::new(TaskName::from("test")),
            ])
            .with_workspaces(vec![
                PackageName::Root,
                PackageName::from("app1"),
                PackageName::from("libA"),
            ])
            .with_root_tasks(vec![
                TaskName::from("//#test"),
                TaskName::from("build"),
                TaskName::from("test"),
            ])
            .build()
            .unwrap();

        let expected = deps! {
            "//#test" => ["___ROOT___"],
            "app1#build" => ["libA#build"],
            "app1#test" => ["libA#build"],
            "libA#build" => ["___ROOT___"],
            "libA#test" => ["___ROOT___"]
        };
        assert_eq!(all_dependencies(&engine), expected);
    }

    #[test]
    fn test_depend_on_root_task() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app1" => ["libA"],
                "libA" => []
            },
        );
        let turbo_jsons = vec![(
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": { "dependsOn": ["^build"] },
                    "libA#build": { "dependsOn": ["//#root-task"] },
                    "//#root-task": {},
                }
            })),
        )]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, loader, false)
            .with_tasks(Some(Spanned::new(TaskName::from("build"))))
            .with_workspaces(vec![PackageName::from("app1")])
            .with_root_tasks(vec![
                TaskName::from("//#root-task"),
                TaskName::from("libA#build"),
                TaskName::from("build"),
            ])
            .build()
            .unwrap();

        let expected = deps! {
            "//#root-task" => ["___ROOT___"],
            "app1#build" => ["libA#build"],
            "libA#build" => ["//#root-task"]
        };
        assert_eq!(all_dependencies(&engine), expected);
    }

    #[test]
    fn test_depend_on_missing_task() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app1" => ["libA"],
                "libA" => []
            },
        );
        let turbo_jsons = vec![(
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": { "dependsOn": ["^build"] },
                    "libA#build": { "dependsOn": ["//#root-task"] },
                }
            })),
        )]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, loader, false)
            .with_tasks(Some(Spanned::new(TaskName::from("build"))))
            .with_workspaces(vec![PackageName::from("app1")])
            .with_root_tasks(vec![TaskName::from("libA#build"), TaskName::from("build")])
            .build();

        assert_matches!(engine, Err(Error::MissingRootTaskInTurboJson { .. }));
    }

    #[test]
    fn test_depend_on_multiple_package_tasks() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app1" => ["libA"],
                "libA" => []
            },
        );
        let turbo_jsons = vec![(
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "libA#build": { "dependsOn": ["app1#compile", "app1#test"] },
                    "build": { "dependsOn": ["^build"] },
                    "compile": {},
                    "test": {}
                }
            })),
        )]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, loader, false)
            .with_tasks(Some(Spanned::new(TaskName::from("build"))))
            .with_workspaces(vec![PackageName::from("app1")])
            .with_root_tasks(vec![
                TaskName::from("libA#build"),
                TaskName::from("build"),
                TaskName::from("compile"),
            ])
            .build()
            .unwrap();

        let expected = deps! {
            "app1#compile" => ["___ROOT___"],
            "app1#test" => ["___ROOT___"],
            "app1#build" => ["libA#build"],
            "libA#build" => ["app1#compile", "app1#test"]
        };
        assert_eq!(all_dependencies(&engine), expected);
    }

    #[test]
    fn test_depends_on_disabled_root_task() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app1" => ["libA"],
                "libA" => []
            },
        );
        let turbo_jsons = vec![(
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": { "dependsOn": ["^build"] },
                    "foo": {},
                    "libA#build": { "dependsOn": ["//#foo"] }
                }
            })),
        )]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, loader, false)
            .with_tasks(Some(Spanned::new(TaskName::from("build"))))
            .with_workspaces(vec![PackageName::from("app1")])
            .with_root_tasks(vec![
                TaskName::from("libA#build"),
                TaskName::from("build"),
                TaskName::from("foo"),
            ])
            .build();

        assert_matches!(engine, Err(Error::MissingRootTaskInTurboJson { .. }));
    }

    #[test]
    fn test_engine_tasks_only() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "a" => [],
                "b" => [],
                "c" => ["a", "b"]
            },
        );
        let turbo_jsons = vec![(
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": { "dependsOn": ["^build", "prepare"] },
                    "test": { "dependsOn": ["^build", "prepare"] },
                    "prepare": {},
                }
            })),
        )]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, loader, false)
            .with_tasks_only(true)
            .with_tasks(Some(Spanned::new(TaskName::from("test"))))
            .with_workspaces(vec![
                PackageName::from("a"),
                PackageName::from("b"),
                PackageName::from("c"),
            ])
            .with_root_tasks(vec![
                TaskName::from("build"),
                TaskName::from("test"),
                TaskName::from("prepare"),
            ])
            .build()
            .unwrap();

        let expected = deps! {
            "a#test" => ["___ROOT___"],
            "b#test" => ["___ROOT___"],
            "c#test" => ["___ROOT___"]
        };
        assert_eq!(all_dependencies(&engine), expected);
    }

    #[test]
    fn test_engine_tasks_only_package_deps() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "a" => [],
                "b" => ["a"]
            },
        );
        let turbo_jsons = vec![(
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": { "dependsOn": ["^build"] },
                }
            })),
        )]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, loader, false)
            .with_tasks_only(true)
            .with_tasks(Some(Spanned::new(TaskName::from("build"))))
            .with_workspaces(vec![PackageName::from("b")])
            .with_root_tasks(vec![TaskName::from("build")])
            .build()
            .unwrap();

        // With task only we shouldn't do package tasks dependencies either
        let expected = deps! {
            "b#build" => ["___ROOT___"]
        };
        assert_eq!(all_dependencies(&engine), expected);
    }

    #[test]
    fn test_engine_tasks_only_task_dep() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "a" => [],
                "b" => []
            },
        );
        let turbo_jsons = vec![(
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "a#build": { },
                    "b#build": { "dependsOn": ["a#build"] }
                }
            })),
        )]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, loader, false)
            .with_tasks_only(true)
            .with_tasks(Some(Spanned::new(TaskName::from("build"))))
            .with_workspaces(vec![PackageName::from("b")])
            .with_root_tasks(vec![TaskName::from("build")])
            .build()
            .unwrap();

        // With task only we shouldn't do package tasks dependencies either
        let expected = deps! {
            "b#build" => ["___ROOT___"]
        };
        assert_eq!(all_dependencies(&engine), expected);
    }

    #[allow(clippy::duplicated_attributes)]
    #[test_case("build", None)]
    #[test_case("build:prod", None)]
    #[test_case("build$colon$prod", Some("task contains invalid string '$colon$'"))]
    fn test_validate_task_name(task_name: &str, reason: Option<&str>) {
        let result = validate_task_name(Spanned::new(task_name))
            .map_err(|e| {
                if let Error::InvalidTaskName { reason, .. } = e {
                    reason
                } else {
                    panic!("invalid error encountered {e:?}")
                }
            })
            .err();
        assert_eq!(result.as_deref(), reason);
    }

    #[test]
    fn test_run_package_task_exact() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app1" => ["libA"],
                "app2" => ["libA"],
                "libA" => []
            },
        );
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "build": { "dependsOn": ["^build"] },
                        "special": { "dependsOn": ["^build"] },
                    }
                })),
            ),
            (
                PackageName::from("app2"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "another": { "dependsOn": ["^build"] },
                    }
                })),
            ),
        ]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, loader, false)
            .with_tasks(vec![
                Spanned::new(TaskName::from("app1#special")),
                Spanned::new(TaskName::from("app2#another")),
            ])
            .with_workspaces(vec![PackageName::from("app1"), PackageName::from("app2")])
            .build()
            .unwrap();

        let expected = deps! {
            "app1#special" => ["libA#build"],
            "app2#another" => ["libA#build"],
            "libA#build" => ["___ROOT___"]
        };
        assert_eq!(all_dependencies(&engine), expected);
    }

    #[test]
    fn test_run_package_task_exact_error() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app1" => ["libA"],
                "libA" => []
            },
        );
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "build": { "dependsOn": ["^build"] },
                    }
                })),
            ),
            (
                PackageName::from("app1"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "another": { "dependsOn": ["^build"] },
                    }
                })),
            ),
        ]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, loader.clone(), false)
            .with_tasks(vec![Spanned::new(TaskName::from("app1#special"))])
            .with_workspaces(vec![PackageName::from("app1")])
            .build();
        assert!(engine.is_err());
        let report = miette::Report::new(engine.unwrap_err());
        let mut msg = String::new();
        miette::JSONReportHandler::new()
            .render_report(&mut msg, report.as_ref())
            .unwrap();
        assert_json_snapshot!(msg);

        let engine = EngineBuilder::new(&repo_root, &package_graph, loader, false)
            .with_tasks(vec![Spanned::new(TaskName::from("app1#another"))])
            .with_workspaces(vec![PackageName::from("libA")])
            .build();
        assert!(engine.is_err());
        let report = miette::Report::new(engine.unwrap_err());
        let mut msg = String::new();
        miette::JSONReportHandler::new()
            .render_report(&mut msg, report.as_ref())
            .unwrap();
        assert_json_snapshot!(msg);
    }

    #[test]
    fn test_run_package_task_invalid_package() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app1" => ["libA"],
                "libA" => []
            },
        );
        let turbo_jsons = vec![(
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": { "dependsOn": ["^build"] },
                }
            })),
        )]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, loader.clone(), false)
            .with_tasks(vec![Spanned::new(TaskName::from("app2#bad-task"))])
            .with_workspaces(vec![PackageName::from("app1"), PackageName::from("libA")])
            .build();
        assert!(engine.is_err());
        let report = miette::Report::new(engine.unwrap_err());
        let mut msg = String::new();
        miette::NarratableReportHandler::new()
            .render_report(&mut msg, report.as_ref())
            .unwrap();
        assert_snapshot!(msg);
    }
}
