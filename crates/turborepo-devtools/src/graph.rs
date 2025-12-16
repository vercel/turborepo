//! Graph conversion utilities.
//!
//! Converts the internal PackageGraph (petgraph-based) to our
//! serializable PackageGraphData format for sending over WebSocket.

use std::collections::HashSet;

use turborepo_repository::package_graph::{
    PackageGraph, PackageName, PackageNode as RepoPackageNode,
};

use crate::types::{GraphEdge, PackageGraphData, PackageNode, TaskGraphData, TaskNode};

/// Identifier used for the root package in the graph
pub const ROOT_PACKAGE_ID: &str = "__ROOT__";

/// Converts a PackageGraph to our serializable PackageGraphData format.
pub fn package_graph_to_data(pkg_graph: &PackageGraph) -> PackageGraphData {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    // Iterate over all packages
    for (name, info) in pkg_graph.packages() {
        let (id, display_name, is_root) = match name {
            PackageName::Root => (ROOT_PACKAGE_ID.to_string(), "(root)".to_string(), true),
            PackageName::Other(n) => (n.clone(), n.clone(), false),
        };

        // Get available scripts from package.json
        let scripts: Vec<String> = info.package_json.scripts.keys().cloned().collect();

        // Get the package path (directory containing package.json)
        let path = info.package_path().to_string();

        nodes.push(PackageNode {
            id: id.clone(),
            name: display_name,
            path,
            scripts,
            is_root,
        });

        // Get dependencies for this package and create edges
        // Note: All packages (including root) are stored as Workspace nodes in the
        // graph. PackageNode::Root is a separate synthetic node that all
        // workspace packages depend on.
        let pkg_node = RepoPackageNode::Workspace(name.clone());

        if let Some(deps) = pkg_graph.immediate_dependencies(&pkg_node) {
            for dep in deps {
                // Skip the synthetic Root node - it's not a real package, just a graph anchor
                if matches!(dep, RepoPackageNode::Root) {
                    continue;
                }

                let dep_id = match dep {
                    RepoPackageNode::Root => unreachable!("filtered above"),
                    RepoPackageNode::Workspace(dep_name) => match dep_name {
                        PackageName::Root => ROOT_PACKAGE_ID.to_string(),
                        PackageName::Other(n) => n.clone(),
                    },
                };
                edges.push(GraphEdge {
                    source: id.clone(),
                    target: dep_id,
                });
            }
        }
    }

    PackageGraphData { nodes, edges }
}

/// Converts a PackageGraph to a task-level graph.
///
/// Creates a node for each package#script combination found in the monorepo.
/// Edges are created based on package dependencies - if package A depends on
/// package B, then for common tasks (like "build"), A#task depends on B#task.
pub fn task_graph_to_data(pkg_graph: &PackageGraph) -> TaskGraphData {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    // Common tasks that typically have cross-package dependencies
    let common_tasks: HashSet<&str> = ["build", "test", "lint", "typecheck", "dev"]
        .into_iter()
        .collect();

    // First pass: collect all tasks and create nodes
    for (name, info) in pkg_graph.packages() {
        let package_id = match name {
            PackageName::Root => ROOT_PACKAGE_ID.to_string(),
            PackageName::Other(n) => n.clone(),
        };

        for script in info.package_json.scripts.keys() {
            let task_id = format!("{}#{}", package_id, script);
            nodes.push(TaskNode {
                id: task_id,
                package: package_id.clone(),
                task: script.clone(),
            });
        }
    }

    // Second pass: create edges based on package dependencies
    // For common tasks, if package A depends on package B, then A#task -> B#task
    for (name, info) in pkg_graph.packages() {
        let package_id = match name {
            PackageName::Root => ROOT_PACKAGE_ID.to_string(),
            PackageName::Other(n) => n.clone(),
        };

        let pkg_node = RepoPackageNode::Workspace(name.clone());

        if let Some(deps) = pkg_graph.immediate_dependencies(&pkg_node) {
            for dep in deps {
                // Skip the synthetic Root node
                if matches!(dep, RepoPackageNode::Root) {
                    continue;
                }

                let dep_id = match dep {
                    RepoPackageNode::Root => continue,
                    RepoPackageNode::Workspace(dep_name) => match dep_name {
                        PackageName::Root => ROOT_PACKAGE_ID.to_string(),
                        PackageName::Other(n) => n.clone(),
                    },
                };

                // Get scripts from the dependency package
                let dep_info = match dep {
                    RepoPackageNode::Root => continue,
                    RepoPackageNode::Workspace(dep_name) => pkg_graph.package_info(dep_name),
                };

                if let Some(dep_info) = dep_info {
                    // For common tasks that exist in both packages, create edges
                    for script in info.package_json.scripts.keys() {
                        if common_tasks.contains(script.as_str())
                            && dep_info.package_json.scripts.contains_key(script)
                        {
                            edges.push(GraphEdge {
                                source: format!("{}#{}", package_id, script),
                                target: format!("{}#{}", dep_id, script),
                            });
                        }
                    }
                }
            }
        }
    }

    TaskGraphData { nodes, edges }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_root_package_id() {
        assert_eq!(ROOT_PACKAGE_ID, "__ROOT__");
    }
}
