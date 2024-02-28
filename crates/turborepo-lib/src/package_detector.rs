use std::path::Path;

use path_clean::PathClean;
use turbopath::AnchoredSystemPath;
use turborepo_repository::{
    change_mapper::{DefaultPackageDetector, PackageDetector},
    package_graph::{PackageGraph, PackageName, WorkspacePackage},
};

use crate::turbo_json::TurboJson;

struct TurboJsonPackageDetector<'a> {
    pkg_dep_graph: &'a PackageGraph,
    turbo_json: &'a TurboJson,
}

impl<'a> PackageDetector for TurboJsonPackageDetector<'a> {
    fn detect_package(&self, path: &AnchoredSystemPath) -> Option<WorkspacePackage> {
        match DefaultPackageDetector::new(self.pkg_dep_graph).detect_package(path) {
            root @ Some(WorkspacePackage {
                name: PackageName::Root,
                ..
            }) => {
                let cleaned_path = path.clean();
                let in_global_deps = self.turbo_json.global_deps.iter().any(|dep| {
                    let dep = Path::new(dep).clean();
                    cleaned_path.as_path() == dep
                });

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
