use std::collections::{HashMap, HashSet};

use insta::{assert_json_snapshot, assert_snapshot};
use serde_json::json;
use tempfile::TempDir;
use test_case::test_case;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_errors::Spanned;
use turborepo_lockfiles::Lockfile;
use turborepo_repository::{
    discovery::PackageDiscovery,
    package_graph::{PackageGraph, PackageName},
    package_json::PackageJson,
    package_manager::PackageManager,
};
use turborepo_task_id::{TaskId, TaskName};
use turborepo_turbo_json::{
    FutureFlags, RawPackageTurboJson, RawRootTurboJson, RawTurboJson, TurboJson,
};
use turborepo_types::{OutputLogsMode, TaskDefinition};

use crate::{BuilderError, Built, CyclicExtends, EngineBuilder, TaskInheritanceResolver, TaskNode};

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

mod cargo;
mod core;
mod extends;
mod inheritance;
mod syntax;
mod uv;
mod workspace;
