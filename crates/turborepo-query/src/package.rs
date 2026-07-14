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
            .package_info(&name)
            .ok_or_else(|| Error::PackageNotFound(name.clone()))?;

        Ok(Self { run, name })
    }

    pub fn run(&self) -> &Arc<dyn QueryRun> {
        &self.run
    }

    pub fn get_name(&self) -> &PackageName {
        &self.name
    }

    pub fn get_tasks(&self) -> HashMap<String, Spanned<String>> {
        self.run
            .pkg_dep_graph()
            .package_json(&self.name)
            .map(|json| {
                json.scripts
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn get_task_names(&self) -> BTreeSet<String> {
        let packages = HashSet::from([self.name.clone()]);
        let registered_tasks: HashSet<_> = self
            .run
            .pkg_dep_graph()
            .package_info(&self.name)
            .and_then(|package| {
                self.run
                    .pkg_dep_graph()
                    .toolchains()
                    .get(&package.toolchain)
                    .map(|toolchain| toolchain.registered_tasks(package))
            })
            .unwrap_or_default()
            .into_iter()
            .collect();
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
            .map_or(0, |pkgs| pkgs.len())
    }

    pub fn direct_dependencies_count(&self) -> usize {
        self.run
            .pkg_dep_graph()
            .immediate_dependencies(&PackageNode::Workspace(self.name.clone()))
            .map_or(0, |pkgs| pkgs.len())
    }

    pub fn indirect_dependents_count(&self) -> usize {
        let node: PackageNode = PackageNode::Workspace(self.name.clone());
        self.run
            .pkg_dep_graph()
            .ancestors(&node)
            .len()
            .saturating_sub(self.direct_dependents_count())
    }

    pub fn indirect_dependencies_count(&self) -> usize {
        let node: PackageNode = PackageNode::Workspace(self.name.clone());
        self.run
            .pkg_dep_graph()
            .dependencies(&node)
            .len()
            .saturating_sub(self.direct_dependencies_count())
    }

    pub fn all_dependents_count(&self) -> usize {
        self.run
            .pkg_dep_graph()
            .ancestors(&PackageNode::Workspace(self.name.clone()))
            .len()
    }

    pub fn all_dependencies_count(&self) -> usize {
        self.run
            .pkg_dep_graph()
            .dependencies(&PackageNode::Workspace(self.name.clone()))
            .len()
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
            .package_info(&self.name)
            .ok_or_else(|| Error::PackageNotFound(self.name.clone()))?
            .package_path()
            .to_string())
    }

    /// The upstream packages that have this package as a direct dependency
    async fn direct_dependents(&self) -> Result<Array<Package>, Error> {
        let node: PackageNode = PackageNode::Workspace(self.name.clone());
        Ok(self
            .run
            .pkg_dep_graph()
            .immediate_ancestors(&node)
            .iter()
            .flatten()
            .map(|package| Package {
                run: self.run.clone(),
                name: package.as_package_name().clone(),
            })
            .sorted_by(|a, b| a.name.cmp(&b.name))
            .collect())
    }

    /// The downstream packages that directly depend on this package
    async fn direct_dependencies(&self) -> Result<Array<Package>, Error> {
        let node: PackageNode = PackageNode::Workspace(self.name.clone());
        Ok(self
            .run
            .pkg_dep_graph()
            .immediate_dependencies(&node)
            .iter()
            .flatten()
            .map(|package| Package {
                run: self.run.clone(),
                name: package.as_package_name().clone(),
            })
            .sorted_by(|a, b| a.name.cmp(&b.name))
            .collect())
    }

    async fn all_dependents(&self) -> Result<Array<Package>, Error> {
        let node: PackageNode = PackageNode::Workspace(self.name.clone());
        Ok(self
            .run
            .pkg_dep_graph()
            .ancestors(&node)
            .iter()
            .map(|package| Package {
                run: self.run.clone(),
                name: package.as_package_name().clone(),
            })
            .sorted_by(|a, b| a.name.cmp(&b.name))
            .collect())
    }

    async fn all_dependencies(&self) -> Result<Array<Package>, Error> {
        let node: PackageNode = PackageNode::Workspace(self.name.clone());
        Ok(self
            .run
            .pkg_dep_graph()
            .dependencies(&node)
            .iter()
            .map(|package| Package {
                run: self.run.clone(),
                name: package.as_package_name().clone(),
            })
            .sorted_by(|a, b| a.name.cmp(&b.name))
            .collect())
    }

    /// The downstream packages that depend on this package, indirectly
    async fn indirect_dependents(&self) -> Result<Array<Package>, Error> {
        let node: PackageNode = PackageNode::Workspace(self.name.clone());
        let immediate_dependents = self
            .run
            .pkg_dep_graph()
            .immediate_ancestors(&node)
            .ok_or_else(|| Error::PackageNotFound(self.name.clone()))?;

        Ok(self
            .run
            .pkg_dep_graph()
            .ancestors(&node)
            .iter()
            .filter(|package| !immediate_dependents.contains(*package))
            .map(|package| Package {
                run: self.run.clone(),
                name: package.as_package_name().clone(),
            })
            .sorted_by(|a, b| a.name.cmp(&b.name))
            .collect())
    }

    /// The upstream packages that this package depends on, indirectly
    async fn indirect_dependencies(&self) -> Result<Array<Package>, Error> {
        let node: PackageNode = PackageNode::Workspace(self.name.clone());
        let immediate_dependencies = self
            .run
            .pkg_dep_graph()
            .immediate_dependencies(&node)
            .ok_or_else(|| Error::PackageNotFound(self.name.clone()))?;

        Ok(self
            .run
            .pkg_dep_graph()
            .dependencies(&node)
            .iter()
            .filter(|package| !immediate_dependencies.contains(*package))
            .map(|package| Package {
                run: self.run.clone(),
                name: package.as_package_name().clone(),
            })
            .sorted_by(|a, b| a.name.cmp(&b.name))
            .collect())
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
