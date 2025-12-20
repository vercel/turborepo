//! Graph conversion utilities.
//!
//! Converts the internal PackageGraph (petgraph-based) to our
//! serializable PackageGraphData format for sending over WebSocket.

use turborepo_repository::package_graph::{
    PackageGraph, PackageName, PackageNode as RepoPackageNode,
};

use crate::types::{GraphEdge, PackageGraphData, PackageNode};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_root_package_id() {
        assert_eq!(ROOT_PACKAGE_ID, "__ROOT__");
    }
}
