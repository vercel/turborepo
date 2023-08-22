use tracing::debug;
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};

use crate::package_graph::{self, PackageGraph};

pub struct PackageInference {
    package_name: Option<String>,
    directory_root: AnchoredSystemPathBuf,
}

impl PackageInference {
    // calculate, based on the directory that global turbo was invoked in,
    // the pieces of a filter spec that we will infer. If turbo was invoked
    // somewhere between the root and packages, scope turbo invocations to the
    // packages below where turbo was invoked. If turbo was invoked at or within
    // a particular package, scope the turbo invocation to just that package.
    pub fn calculate(
        turbo_root: &AbsoluteSystemPath,
        pkg_inference_path: &AnchoredSystemPathBuf,
        pkg_graph: &package_graph::PackageGraph,
    ) -> Self {
        debug!(
            "Using {} as a basis for selecting packages",
            pkg_inference_path
        );
        let full_inference_path = turbo_root.resolve(pkg_inference_path);
        for (workspace_name, workspace_entry) in pkg_graph.workspaces() {
            let pkg_path = turbo_root.resolve(workspace_entry.package_json_path());
            let inferred_path_is_below = pkg_path.contains(&full_inference_path);
            // We skip over the root package as the inferred path will always be below it
            if inferred_path_is_below && (&pkg_path as &AbsoluteSystemPath) != turbo_root {
                // set both. The user might have set a parent directory filter,
                // in which case we *should* fail to find any packages, but we should
                // do so in a consistent manner
                return Self {
                    package_name: Some(workspace_name.to_string()),
                    directory_root: workspace_entry.package_json_path().clone(),
                };
            }
            let inferred_path_is_between_root_and_pkg = full_inference_path.contains(&pkg_path);
            if inferred_path_is_between_root_and_pkg {
                // we've found *some* package below our inference directory. We can stop now and
                // conclude that we're looking for all packages in a
                // subdirectory
                break;
            }
        }
        Self {
            package_name: None,
            directory_root: pkg_inference_path.to_owned(),
        }
    }
}

struct Resolver<'a> {
    pkg_graph: &'a PackageGraph,
    turbo_root: &'a AbsoluteSystemPath,
    inference: Option<PackageInference>,
}
