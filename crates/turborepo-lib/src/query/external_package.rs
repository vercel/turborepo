use std::sync::Arc;

use async_graphql::Object;

use super::{package::Package, Array, Error};
use crate::run::Run;

#[derive(Clone)]
pub struct ExternalPackage {
    run: Arc<Run>,
    package: turborepo_lockfiles::Package,
}

impl ExternalPackage {
    pub fn new(run: Arc<Run>, package: turborepo_lockfiles::Package) -> Self {
        Self { run, package }
    }

    /// Converts the lockfile key to a human friendly name
    pub fn human_name(&self) -> String {
        self.run
            .pkg_dep_graph()
            .lockfile()
            .and_then(|lockfile| lockfile.human_name(&self.package))
            .unwrap_or_else(|| self.package.key.clone())
    }
}

#[Object]
impl ExternalPackage {
    async fn name(&self) -> String {
        self.human_name().to_string()
    }

    async fn internal_dependents(&self) -> Result<Array<Package>, Error> {
        let Some(names) = self
            .run
            .pkg_dep_graph()
            .internal_dependencies_for_external_dependency(&self.package)
        else {
            return Ok(Array::from(Vec::new()));
        };
        let mut packages = names
            .iter()
            .map(|name| Package::new(self.run.clone(), name.as_package_name().clone()))
            .collect::<Result<Array<_>, Error>>()?;
        packages.sort_by(|a, b| a.get_name().cmp(b.get_name()));
        Ok(packages)
    }
}
