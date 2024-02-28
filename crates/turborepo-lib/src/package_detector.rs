use thiserror::Error;
use turbopath::AnchoredSystemPath;
use turborepo_repository::{
    change_mapper::{DefaultPackageDetector, PackageDetector},
    package_graph::{PackageGraph, PackageName, WorkspacePackage},
};
use wax::{Any, BuildError, Program};

use crate::turbo_json::TurboJson;

const DEFAULT_GLOBAL_DEPS: [&'static str; 2] = ["package.json", "turbo.json"];

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    InvalidFilter(#[from] BuildError),
}

pub struct TurboJsonPackageDetector<'a> {
    pkg_dep_graph: &'a PackageGraph,
    global_deps_matcher: Any<'a>,
    turbo_json: &'a TurboJson,
}

impl<'a> TurboJsonPackageDetector<'a> {
    pub fn new(pkg_dep_graph: &'a PackageGraph, turbo_json: &'a TurboJson) -> Result<Self, Error> {
        let global_deps = turbo_json.global_deps.iter().map(|s| s.as_str());
        let filters = global_deps.chain(DEFAULT_GLOBAL_DEPS.iter().copied());
        let matcher = wax::any(filters)?;

        Ok(Self {
            pkg_dep_graph,
            global_deps_matcher: matcher,
            turbo_json,
        })
    }
}

impl<'a> PackageDetector for TurboJsonPackageDetector<'a> {
    fn detect_package(&self, path: &AnchoredSystemPath) -> Option<WorkspacePackage> {
        match DefaultPackageDetector::new(self.pkg_dep_graph).detect_package(path) {
            root @ Some(WorkspacePackage {
                name: PackageName::Root,
                ..
            }) => {
                let cleaned_path = path.clean();
                let in_global_deps = self.global_deps_matcher.is_match(cleaned_path.as_str());

                if in_global_deps {
                    root
                } else {
                    None
                }
            }
            result => result,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use tempfile::tempdir;
    use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};
    use turborepo_repository::{
        change_mapper::{ChangeMapper, DefaultPackageDetector, PackageChanges},
        package_graph::{builder::MockDiscovery, PackageGraphBuilder, WorkspacePackage},
        package_json::PackageJson,
    };

    use super::TurboJsonPackageDetector;
    use crate::turbo_json::TurboJson;

    #[tokio::test]
    async fn test_different_package_detectors() -> Result<(), anyhow::Error> {
        let repo_root = tempdir()?;
        let root_package_json = PackageJson::default();

        let pkg_graph = PackageGraphBuilder::new(
            AbsoluteSystemPath::from_std_path(repo_root.path())?,
            root_package_json,
        )
        .with_package_discovery(MockDiscovery)
        .build()
        .await?;

        let default_package_detector = DefaultPackageDetector::new(&pkg_graph);
        let change_mapper = ChangeMapper::new(&pkg_graph, vec![], default_package_detector);

        let package_changes = change_mapper.changed_packages(
            [AnchoredSystemPathBuf::from_raw("README.md")?]
                .into_iter()
                .collect(),
            None,
        )?;
        // We should have a root package change since we don't have global deps and
        // therefore must be conservative about changes
        assert_eq!(
            package_changes,
            PackageChanges::Some([WorkspacePackage::root()].into_iter().collect())
        );

        let turbo_json = TurboJson::default();
        let default_package_detector = TurboJsonPackageDetector::new(&pkg_graph, &turbo_json)?;
        let change_mapper = ChangeMapper::new(&pkg_graph, vec![], default_package_detector);

        let package_changes = change_mapper.changed_packages(
            [AnchoredSystemPathBuf::from_raw("README.md")?]
                .into_iter()
                .collect(),
            None,
        )?;
        // We shouldn't get any changes since we have global deps specified and
        // README.md is not one of them
        assert_eq!(package_changes, PackageChanges::Some(HashSet::new()));

        Ok(())
    }
}
