use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use insta::{assert_json_snapshot, assert_snapshot};
use serde_json::json;
use tempfile::TempDir;
use test_case::test_case;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_errors::Spanned;
use turborepo_lockfiles::Lockfile;
use turborepo_repository::{
    discovery::PackageDiscovery,
    package_graph::{PackageGraph, PackageName, ROOT_PKG_NAME},
    package_json::PackageJson,
    package_manager::PackageManager,
    toolchain::{
        DerivedInputSafety, DerivedOutputs, DerivedTaskIO, DiscoverPackagesFuture,
        DiscoveredPackage, DiscoveredPackages, Toolchain, ToolchainId, WorkspaceRoot,
    },
};
use turborepo_task_id::{TaskId, TaskName};
use turborepo_turbo_json::{
    FutureFlags, RawPackageTurboJson, RawRootTurboJson, RawTurboJson, TurboJson,
};
use turborepo_types::{OutputLogsMode, TaskDefinition};

use crate::{
    BuilderError, Built, CyclicExtends, Engine, EngineBuilder, TaskInheritanceResolver, TaskNode,
};

/// Test implementation of TurboJsonLoader that returns pre-configured
/// TurboJson structures without reading from disk.
struct TestTurboJsonLoader {
    turbo_jsons: HashMap<PackageName, TurboJson>,
}

impl TestTurboJsonLoader {
    fn new(turbo_jsons: HashMap<PackageName, TurboJson>) -> Self {
        Self { turbo_jsons }
    }
}

impl crate::TurboJsonLoader for TestTurboJsonLoader {
    fn load(&self, package: &PackageName) -> Result<&TurboJson, BuilderError> {
        self.turbo_jsons
            .get(package)
            .ok_or_else(|| BuilderError::TurboJson(turborepo_turbo_json::Error::NoTurboJSON))
    }
}

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
    ) -> Result<
        Option<std::borrow::Cow<'_, std::collections::BTreeMap<String, String>>>,
        turborepo_lockfiles::Error,
    > {
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

struct AggregateToolchain {
    repo_root: AbsoluteSystemPathBuf,
}

impl Toolchain for AggregateToolchain {
    fn id(&self) -> ToolchainId {
        ToolchainId::new("aggregate-test")
    }

    fn discover_packages(&self) -> DiscoverPackagesFuture<'_> {
        Box::pin(async move {
            Ok(DiscoveredPackages::new(
                vec![DiscoveredPackage::aggregate(
                    "cargo-workspace".to_owned(),
                    PackageJson::default(),
                    self.repo_root.join_component("Cargo.toml"),
                )],
                vec![WorkspaceRoot::new("aggregate-test", self.repo_root.clone())],
            ))
        })
    }
}

type StubIOEngineResult = Engine<Built, TaskDefinition>;

struct StubIOToolchain {
    repo_root: AbsoluteSystemPathBuf,
    outputs: DerivedOutputs,
    input_safety: DerivedInputSafety,
    environment: Vec<&'static str>,
}

impl Toolchain for StubIOToolchain {
    fn id(&self) -> ToolchainId {
        ToolchainId::new("stub-io")
    }

    fn discover_packages(&self) -> DiscoverPackagesFuture<'_> {
        Box::pin(async move {
            let package = |name: &str, dependencies: &[(&str, &str)]| {
                DiscoveredPackage::package(
                    Some(name.to_string()),
                    PackageJson {
                        name: Some(Spanned::new(name.to_string())),
                        dependencies: Some(
                            dependencies
                                .iter()
                                .map(|(name, version)| (name.to_string(), version.to_string()))
                                .collect(),
                        ),
                        ..Default::default()
                    },
                    self.repo_root
                        .join_components(&["packages", name, "stub.json"]),
                )
                .with_task_contract(
                    turborepo_repository::task_contracts::ScopeTaskContract::derived(
                        ToolchainId::new("stub-io"),
                        std::collections::BTreeMap::new(),
                        self.environment.clone(),
                        ["build", "test"]
                            .into_iter()
                            .map(|task| {
                                (
                                    task.to_string(),
                                    DerivedTaskIO {
                                        outputs: self.outputs.clone(),
                                        input_safety: self.input_safety.clone(),
                                        ..Default::default()
                                    },
                                )
                            })
                            .collect(),
                    ),
                )
            };
            Ok(DiscoveredPackages::new(
                vec![
                    package("app", &[("lib", "workspace:*")]),
                    package("lib", &[]),
                ],
                vec![WorkspaceRoot::new("stub-io", self.repo_root.clone())],
            ))
        })
    }
}

fn stub_io_package_graph(
    repo_root: &turbopath::AbsoluteSystemPath,
    toolchain: Arc<StubIOToolchain>,
) -> PackageGraph {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(
        PackageGraph::builder(repo_root, PackageJson::default())
            .with_package_discovery(MockDiscovery)
            .with_lockfile(Some(Box::new(MockLockfile)))
            .with_package_jsons(Some(HashMap::new()))
            .with_toolchain(toolchain)
            .build(),
    )
    .unwrap()
}

#[test]
fn task_definition_repo_enumeration_uses_authoritative_scopes() {
    let repo = TempDir::new().unwrap();
    let repo_root = AbsoluteSystemPathBuf::try_from(repo.path().to_path_buf()).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let package_graph = runtime
        .block_on(
            PackageGraph::builder_optional(&repo_root, None)
                .with_package_discovery(MockDiscovery)
                .with_package_jsons(Some(HashMap::new()))
                .with_toolchain(Arc::new(AggregateToolchain {
                    repo_root: repo_root.clone(),
                }))
                .build(),
        )
        .unwrap();

    let aggregate_name = PackageName::from("cargo-workspace");
    let aggregate_config = turbo_json(json!({
        "extends": ["//"],
        "tasks": { "check": {} }
    }));
    let loader = TestTurboJsonLoader::new(HashMap::from([
        (aggregate_name.clone(), aggregate_config),
        (PackageName::Root, turbo_json(json!({ "tasks": {} }))),
    ]));

    assert!(
        EngineBuilder::has_task_definition_in_repo(
            &loader,
            &package_graph,
            &TaskName::from("check"),
        )
        .unwrap()
    );
    let scopes = package_graph
        .package_scope_directories()
        .map(|(name, _)| name)
        .collect::<HashSet<_>>();
    assert!(!scopes.contains(&PackageName::Root));
    assert!(scopes.contains(&PackageName::from("cargo-workspace")));

    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_workspaces(vec![aggregate_name])
        .with_tasks(Some(Spanned::new(TaskName::from("check"))))
        .build()
        .unwrap();
    assert!(
        engine
            .task_definition(&TaskId::new("cargo-workspace", "check"))
            .is_some(),
        "an aggregate scope with a Turbo task definition must build completely"
    );

    let graph_with_root = runtime
        .block_on(
            PackageGraph::builder(&repo_root, PackageJson::default())
                .with_package_discovery(MockDiscovery)
                .with_lockfile(Some(Box::new(MockLockfile)))
                .with_package_jsons(Some(HashMap::new()))
                .with_toolchain(Arc::new(AggregateToolchain {
                    repo_root: repo_root.clone(),
                }))
                .build(),
        )
        .unwrap();
    assert!(
        graph_with_root
            .package_scope_directories()
            .any(|(name, _)| name == PackageName::Root),
        "a contributed root package.json must expose the root JavaScript scope"
    );
}

#[test]
fn memberless_pure_cargo_preserves_root_task_namespace() {
    let repo = TempDir::new().unwrap();
    let repo_root = AbsoluteSystemPathBuf::try_from(repo.path().to_path_buf()).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let package_graph = runtime
        .block_on(
            PackageGraph::builder_optional(&repo_root, None)
                .with_package_discovery(MockDiscovery)
                .with_package_jsons(Some(HashMap::new()))
                .build(),
        )
        .unwrap();
    assert!(package_graph.package_view(&PackageName::Root).is_none());

    let loader = TestTurboJsonLoader::new(HashMap::from([(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "//#root-only": {}
            }
        })),
    )]));
    let root_task = TaskName::from("//#root-only");
    assert!(
        EngineBuilder::has_task_definition_in_repo(&loader, &package_graph, &root_task).unwrap()
    );

    let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_workspaces(vec![PackageName::Root])
        .with_tasks(Some(Spanned::new(root_task.clone().into_owned())))
        .with_root_tasks(vec![root_task.into_owned()])
        .build()
        .unwrap();
    assert!(
        engine
            .task_definition(&TaskId::new(ROOT_PKG_NAME, "root-only"))
            .is_some()
    );

    let missing_root_loader = TestTurboJsonLoader::new(HashMap::from([(
        PackageName::Root,
        turbo_json(json!({
            "tasks": {
                "//#root-only": { "dependsOn": ["//#missing"] }
            }
        })),
    )]));
    let error = EngineBuilder::new(&repo_root, &package_graph, &missing_root_loader, false)
        .with_workspaces(vec![PackageName::Root])
        .with_tasks(Some(Spanned::new(TaskName::from("//#root-only"))))
        .with_root_tasks(vec![TaskName::from("//#root-only")])
        .build()
        .unwrap_err();
    assert!(matches!(error, BuilderError::MissingRootTaskInTurboJson(_)));
}

macro_rules! package_jsons {
        {$root:expr, $($name:expr => $deps:expr),+} => {
            {
                let mut _map = HashMap::new();
                $(
                    let path = $root.join_components(&["packages", $name, "package.json"]);
                    let dependencies = Some($deps.iter().map(|dep: &&str| (dep.to_string(), "workspace:*".to_string())).collect());
                    let package_json = PackageJson { name: Some(Spanned::new($name.to_string())), dependencies, ..Default::default() };
                    _map.insert(path, package_json);
                )+
                _map
            }
        };
    }

fn mock_package_graph(
    repo_root: &turbopath::AbsoluteSystemPath,
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
    let is_package = value.as_object().unwrap().contains_key("extends");
    let json_text = serde_json::to_string(&value).unwrap();
    let raw: RawTurboJson = if is_package {
        RawPackageTurboJson::parse(&json_text, "").unwrap().into()
    } else {
        RawRootTurboJson::parse(&json_text, "")
            .unwrap()
            .try_into()
            .unwrap()
    };
    TurboJson::try_from(raw).unwrap()
}

/// Helper function to collect tasks from extends chain using
/// TaskInheritanceResolver
fn collect_tasks_from_extends_chain<L: crate::TurboJsonLoader>(
    loader: &L,
    workspace: &PackageName,
    tasks: &mut HashSet<TaskName<'static>>,
    _visited: &mut HashSet<PackageName>,
) -> Result<(), BuilderError> {
    let resolved_tasks = TaskInheritanceResolver::new(loader).resolve(workspace)?;
    tasks.extend(resolved_tasks);
    Ok(())
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
    let loader = TestTurboJsonLoader::new(turbo_jsons);
    let task_name = TaskName::from(task_name);
    let task_id = TaskId::try_from(task_id).unwrap();

    let has_def =
        EngineBuilder::has_task_definition_in_run(&loader, &workspace, &task_name, &task_id)
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

fn all_dependencies(
    engine: &crate::Engine<Built, TaskDefinition>,
) -> HashMap<TaskId<'static>, HashSet<TaskNode>> {
    engine
        .task_lookup()
        .keys()
        .filter_map(|task_id| {
            let deps = engine.dependencies(task_id)?;
            Some((task_id.clone(), deps.into_iter().cloned().collect()))
        })
        .collect()
}

fn task_definition<'a>(
    engine: &'a crate::Engine<Built, TaskDefinition>,
    task_id: &'static str,
) -> &'a TaskDefinition {
    engine
        .task_definition(&TaskId::try_from(task_id).unwrap())
        .unwrap()
}

fn task_names(tasks: &[Spanned<TaskName<'static>>]) -> Vec<String> {
    tasks
        .iter()
        .map(|task| task.as_inner().to_string())
        .collect()
}

mod core;
mod extends;
mod inheritance;
mod syntax;
mod workspace;
