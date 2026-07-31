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

    // Iterate over authoritative execution scopes. This preserves aggregate
    // visualization while omitting the compatibility root in pure Cargo repos.
    for (name, path) in pkg_graph.package_scope_directories() {
        let (id, display_name, is_root) = match &name {
            PackageName::Root => (ROOT_PACKAGE_ID.to_string(), "(root)".to_string(), true),
            PackageName::Other(n) => (n.clone(), n.clone(), false),
        };

        // Available native tasks from the catalog (not a live PackageJson scripts
        // read).
        let scripts: Vec<String> = pkg_graph
            .package_task_context(&name)
            .map(|context| context.native_tasks().script_names())
            .unwrap_or_default();

        nodes.push(PackageNode {
            id: id.clone(),
            name: display_name,
            path: path.to_unix().to_string(),
            scripts,
            is_root,
        });

        // Get dependencies for this package and create edges
        // Note: All packages (including root) are stored as Workspace nodes in the
        // graph. PackageNode::Root is a separate synthetic node that all
        // workspace packages depend on.
        let pkg_node = RepoPackageNode::Workspace(name);

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
    use std::collections::HashMap;

    use serde_json::json;
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_repository::{
        package_graph::PackageGraph, package_json::PackageJson, package_manager::PackageManager,
    };

    use super::*;

    fn temp_root() -> (tempfile::TempDir, AbsoluteSystemPathBuf) {
        let tempdir = tempfile::tempdir().expect("create temporary repository");
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path())
            .expect("temporary repository has an absolute path");
        (tempdir, root)
    }

    fn package_json(value: serde_json::Value) -> PackageJson {
        PackageJson::from_value(value).expect("valid test package.json")
    }

    async fn javascript_graph() -> (tempfile::TempDir, PackageGraph) {
        let (tempdir, root) = temp_root();
        let root_manifest = package_json(json!({
            "name": "turbo",
            "scripts": { "build": "turbo build" }
        }));
        let package_path = root.join_components(&["packages", "web", "package.json"]);
        let web_manifest = package_json(json!({
            "name": "web",
            "scripts": { "dev": "next dev", "empty": "" }
        }));
        let graph = PackageGraph::builder(&root, root_manifest)
            .with_package_manager(PackageManager::Npm)
            .with_package_jsons(Some(HashMap::from([(package_path, web_manifest)])))
            .with_allow_no_package_manager(true)
            .build()
            .await
            .expect("build JavaScript package graph");

        (tempdir, graph)
    }

    #[test]
    fn test_root_package_id() {
        assert_eq!(ROOT_PACKAGE_ID, "__ROOT__");
    }

    #[tokio::test]
    async fn uses_knowledge_paths_and_preserves_root_serialization() {
        let (_tempdir, graph) = javascript_graph().await;
        let data = package_graph_to_data(&graph);

        let root = data
            .nodes
            .iter()
            .find(|node| node.id == ROOT_PACKAGE_ID)
            .expect("root JavaScript scope remains visible");
        assert_eq!(root.name, "(root)");
        assert_eq!(root.path, "");
        assert_eq!(root.scripts, ["build"]);
        assert!(root.is_root);

        let web = data
            .nodes
            .iter()
            .find(|node| node.id == "web")
            .expect("workspace is visible");
        assert_eq!(web.path, "packages/web");
        assert_eq!(web.scripts, ["dev", "empty"]);
        assert!(!web.is_root);

        assert_eq!(
            serde_json::to_value(web).expect("serialize package node"),
            json!({
                "id": "web",
                "name": "web",
                "path": "packages/web",
                "scripts": ["dev", "empty"],
                "isRoot": false
            })
        );
    }
}
