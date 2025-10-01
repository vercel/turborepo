use std::sync::Arc;

use async_graphql::Object;

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
}
