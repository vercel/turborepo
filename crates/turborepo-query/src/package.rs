use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fmt,
    sync::Arc,
};

use async_graphql::Object;
use itertools::Itertools;
use turborepo_errors::Spanned;
use turborepo_repository::package_graph::{PackageName, PackageNode};

use crate::{task::RepositoryTask, Array, Error, QueryRun};

#[derive(Clone)]
pub struct Package {
    run: Arc<dyn QueryRun>,
    name: PackageName,
}

impl fmt::Debug for Package {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Package").field("name", &self.name).finish()
    }
}

impl Package {
    pub fn new(run: Arc<dyn QueryRun>, name: PackageName) -> Result<Self, Error> {
        run.pkg_dep_graph()
            .package_view(&name)
            .ok_or_else(|| Error::PackageNotFound(name.clone()))?;

        Ok(Self { run, name })
    }

    pub fn for_task(run: Arc<dyn QueryRun>, name: PackageName) -> Result<Self, Error> {
        run.pkg_dep_graph()
            .package_task_context(&name)
            .ok_or_else(|| Error::PackageNotFound(name.clone()))?;

        Ok(Self { run, name })
    }

    pub fn run(&self) -> &Arc<dyn QueryRun> {
        &self.run
    }

    pub fn get_name(&self) -> &PackageName {
        &self.name
    }

    fn package_from_node(&self, node: &PackageNode) -> Option<Self> {
        let name = match node {
            PackageNode::Root => PackageName::Root,
            PackageNode::Workspace(name) => name.clone(),
        };
        Self::new(self.run.clone(), name).ok()
    }

    fn is_queryable_node(&self, node: &PackageNode) -> bool {
        self.package_from_node(node).is_some()
    }

    fn collect_nodes<'a>(&self, nodes: impl IntoIterator<Item = &'a PackageNode>) -> Array<Self> {
        nodes
            .into_iter()
            .filter_map(|node| self.package_from_node(node))
            .sorted_by(|a, b| a.name.cmp(&b.name))
            .collect()
    }

    fn count_nodes<'a>(&self, nodes: impl IntoIterator<Item = &'a PackageNode>) -> usize {
        nodes
            .into_iter()
            .filter(|node| self.is_queryable_node(node))
            .count()
    }

    pub fn get_tasks(&self) -> HashMap<String, Spanned<String>> {
        self.run
            .pkg_dep_graph()
            .package_task_context(&self.name)
            .map(|context| {
                context
                    .native_tasks()
                    .tasks()
                    .iter()
                    .filter_map(|task| {
                        task.script()
                            .cloned()
                            .map(|script| (task.name().to_string(), script))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn get_task_names(&self) -> BTreeSet<String> {
        let packages = HashSet::from([self.name.clone()]);
        let registered_tasks: HashSet<_> = self
            .run
            .pkg_dep_graph()
            .package_task_context(&self.name)
            .map(|context| {
                context
                    .native_tasks()
                    .registered_names()
                    .into_iter()
                    .collect()
            })
            .unwrap_or_default();
        self.get_tasks()
            .into_keys()
            .chain(
                self.run
                    .engine()
                    .task_ids_for_packages(&packages)
                    .into_iter()
                    .map(|task| task.task().to_string())
                    .filter(|task| registered_tasks.contains(task)),
            )
            .collect()
    }

    pub fn direct_dependents_count(&self) -> usize {
        self.run
            .pkg_dep_graph()
            .immediate_ancestors(&PackageNode::Workspace(self.name.clone()))
            .map_or(0, |packages| self.count_nodes(packages))
    }

    pub fn direct_dependencies_count(&self) -> usize {
        self.run
            .pkg_dep_graph()
            .immediate_dependencies(&PackageNode::Workspace(self.name.clone()))
            .map_or(0, |packages| self.count_nodes(packages))
    }

    pub fn indirect_dependents_count(&self) -> usize {
        let node: PackageNode = PackageNode::Workspace(self.name.clone());
        let graph = self.run.pkg_dep_graph();
        let immediate = graph.immediate_ancestors(&node);
        self.count_nodes(graph.ancestors(&node).into_iter().filter(|package| {
            immediate
                .as_ref()
                .is_none_or(|nodes| !nodes.contains(*package))
        }))
    }

    pub fn indirect_dependencies_count(&self) -> usize {
        let node: PackageNode = PackageNode::Workspace(self.name.clone());
        let graph = self.run.pkg_dep_graph();
        let immediate = graph.immediate_dependencies(&node);
        self.count_nodes(graph.dependencies(&node).into_iter().filter(|package| {
            immediate
                .as_ref()
                .is_none_or(|nodes| !nodes.contains(*package))
        }))
    }

    pub fn all_dependents_count(&self) -> usize {
        self.count_nodes(
            self.run
                .pkg_dep_graph()
                .ancestors(&PackageNode::Workspace(self.name.clone())),
        )
    }

    pub fn all_dependencies_count(&self) -> usize {
        self.count_nodes(
            self.run
                .pkg_dep_graph()
                .dependencies(&PackageNode::Workspace(self.name.clone())),
        )
    }
}

#[Object]
impl Package {
    /// The name of the package
    async fn name(&self) -> String {
        self.name.to_string()
    }

    /// The path to the package, relative to the repository root
    async fn path(&self) -> Result<String, Error> {
        Ok(self
            .run
            .pkg_dep_graph()
            .package_task_context(&self.name)
            .ok_or_else(|| Error::PackageNotFound(self.name.clone()))?
            .directory()
            .to_unix()
            .to_string())
    }

    /// The upstream packages that have this package as a direct dependency
    async fn direct_dependents(&self) -> Result<Array<Package>, Error> {
        let node: PackageNode = PackageNode::Workspace(self.name.clone());
        Ok(self.collect_nodes(
            self.run
                .pkg_dep_graph()
                .immediate_ancestors(&node)
                .into_iter()
                .flatten(),
        ))
    }

    /// The downstream packages that directly depend on this package
    async fn direct_dependencies(&self) -> Result<Array<Package>, Error> {
        let node: PackageNode = PackageNode::Workspace(self.name.clone());
        Ok(self.collect_nodes(
            self.run
                .pkg_dep_graph()
                .immediate_dependencies(&node)
                .into_iter()
                .flatten(),
        ))
    }

    async fn all_dependents(&self) -> Result<Array<Package>, Error> {
        let node: PackageNode = PackageNode::Workspace(self.name.clone());
        Ok(self.collect_nodes(self.run.pkg_dep_graph().ancestors(&node)))
    }

    async fn all_dependencies(&self) -> Result<Array<Package>, Error> {
        let node: PackageNode = PackageNode::Workspace(self.name.clone());
        Ok(self.collect_nodes(self.run.pkg_dep_graph().dependencies(&node)))
    }

    /// The downstream packages that depend on this package, indirectly
    async fn indirect_dependents(&self) -> Result<Array<Package>, Error> {
        let node: PackageNode = PackageNode::Workspace(self.name.clone());
        let immediate_dependents = self
            .run
            .pkg_dep_graph()
            .immediate_ancestors(&node)
            .ok_or_else(|| Error::PackageNotFound(self.name.clone()))?;

        Ok(self.collect_nodes(
            self.run
                .pkg_dep_graph()
                .ancestors(&node)
                .into_iter()
                .filter(|package| !immediate_dependents.contains(*package)),
        ))
    }

    /// The upstream packages that this package depends on, indirectly
    async fn indirect_dependencies(&self) -> Result<Array<Package>, Error> {
        let node: PackageNode = PackageNode::Workspace(self.name.clone());
        let immediate_dependencies = self
            .run
            .pkg_dep_graph()
            .immediate_dependencies(&node)
            .ok_or_else(|| Error::PackageNotFound(self.name.clone()))?;

        Ok(self.collect_nodes(
            self.run
                .pkg_dep_graph()
                .dependencies(&node)
                .into_iter()
                .filter(|package| !immediate_dependencies.contains(*package)),
        ))
    }

    async fn tasks(&self) -> Array<RepositoryTask> {
        let scripts = self.get_tasks();
        self.get_task_names()
            .into_iter()
            .map(|name| RepositoryTask {
                script: scripts.get(&name).cloned(),
                name,
                package: self.clone(),
            })
            .collect()
    }
}
