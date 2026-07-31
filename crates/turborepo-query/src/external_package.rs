use std::sync::Arc;

use async_graphql::Object;
use turborepo_repository::external_resolution::ExternalPackageIdentity;

use crate::{package::Package, Array, Error, QueryRun};

#[derive(Clone)]
pub struct ExternalPackage {
    run: Arc<dyn QueryRun>,
    identity: ExternalPackageIdentity,
}

impl ExternalPackage {
    pub fn new(run: Arc<dyn QueryRun>, package: turborepo_lockfiles::Package) -> Self {
        let identity = run
            .pkg_dep_graph()
            .resolve_external_package_identity(&package)
            .cloned()
            .unwrap_or_else(|| {
                ExternalPackageIdentity::new(package.key.clone(), package.version.clone())
            });
        Self { run, identity }
    }

    pub fn from_identity(run: Arc<dyn QueryRun>, identity: ExternalPackageIdentity) -> Self {
        Self { run, identity }
    }

    pub fn human_name(&self) -> String {
        self.identity.display_name().to_string()
    }
}

#[Object]
impl ExternalPackage {
    async fn name(&self) -> String {
        self.human_name()
    }

    async fn internal_dependents(&self) -> Result<Array<Package>, Error> {
        let Some(names) = self
            .run
            .pkg_dep_graph()
            .internal_dependencies_for_external_identity(&self.identity)
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
