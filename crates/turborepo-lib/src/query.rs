use std::sync::Arc;

use async_graphql::*;
use miette::Diagnostic;
use thiserror::Error;
use turborepo_repository::package_graph::{PackageName, PackageNode};

use crate::run::Run;

#[derive(Error, Debug, Diagnostic)]
pub enum Error {
    #[error("package not found: {0}")]
    PackageNotFound(PackageName),
    #[error("failed to serialize result: {0}")]
    Serde(#[from] serde_json::Error),
    #[error(transparent)]
    Run(#[from] crate::run::Error),
}

pub struct Query {
    run: Arc<Run>,
}

impl Query {
    pub fn new(run: Run) -> Self {
        Self { run: Arc::new(run) }
    }
}

struct Package {
    run: Arc<Run>,
    name: PackageName,
}

#[Object]
impl Query {
    async fn package(&self, name: String) -> Result<Package, Error> {
        let name = PackageName::from(name);
        Ok(Package {
            run: self.run.clone(),
            name,
        })
    }
}

#[Object]
impl Package {
    async fn name(&self) -> String {
        self.name.to_string()
    }

    async fn dependents(&self) -> Result<Vec<Package>, Error> {
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
            .collect())
    }

    async fn dependencies(&self) -> Result<Vec<Package>, Error> {
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
            .collect())
    }
}
