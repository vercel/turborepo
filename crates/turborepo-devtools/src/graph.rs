//! Graph conversion utilities.
//!
//! Converts the internal PackageGraph (petgraph-based) to our
//! serializable PackageGraphData format for sending over WebSocket.

use std::collections::HashSet;

use biome_json_parser::JsonParserOptions;
use biome_json_syntax::JsonRoot;
use tracing::debug;
use turbopath::AbsoluteSystemPath;
use turborepo_repository::package_graph::{
    PackageGraph, PackageName, PackageNode as RepoPackageNode,
};

use crate::types::{GraphEdge, PackageGraphData, PackageNode, TaskGraphData, TaskNode};

/// Identifier used for the root package in the graph
pub const ROOT_PACKAGE_ID: &str = "__ROOT__";

/// Reads task names from turbo.json at the repository root.
/// Returns a set of task names (without package prefixes like "build", not
/// "pkg#build"). Returns an empty set if turbo.json cannot be read or parsed.
pub fn read_pipeline_tasks(repo_root: &AbsoluteSystemPath) -> HashSet<String> {
    let turbo_json_path = repo_root.join_component("turbo.json");
    let turbo_jsonc_path = repo_root.join_component("turbo.jsonc");

    // Try turbo.json first, then turbo.jsonc
    let contents = turbo_json_path
        .read_to_string()
        .or_else(|_| turbo_jsonc_path.read_to_string());

    match contents {
        Ok(contents) => parse_pipeline_tasks(&contents),
        Err(e) => {
            debug!("Could not read turbo.json: {}", e);
            HashSet::new()
        }
    }
}

/// Parses turbo.json content and extracts task names.
/// Task names like "build" or "pkg#build" are normalized to just the task part.
fn parse_pipeline_tasks(contents: &str) -> HashSet<String> {
    // Use Biome's JSONC parser which handles comments natively
    let parse_result =
        biome_json_parser::parse_json(contents, JsonParserOptions::default().with_allow_comments());

    if parse_result.has_errors() {
        debug!(
            "Failed to parse turbo.json: {:?}",
            parse_result.diagnostics()
        );
        return HashSet::new();
    }

    let root: JsonRoot = parse_result.tree();

    // Navigate to the "tasks" object and extract its keys
    extract_task_keys_from_json(&root)
}

/// Extracts task keys from a parsed JSON root.
/// Returns task names normalized (without package prefixes).
fn extract_task_keys_from_json(root: &JsonRoot) -> HashSet<String> {
    use biome_json_syntax::AnyJsonValue;

    // Get the root value (should be an object)
    let Some(value) = root.value().ok() else {
        return HashSet::new();
    };

    let AnyJsonValue::JsonObjectValue(obj) = value else {
        return HashSet::new();
    };

    // Find the "tasks" member
    for member in obj.json_member_list() {
        let Ok(member) = member else { continue };
        let Ok(name) = member.name() else { continue };

        if get_member_name_text(&name) == "tasks" {
            let Ok(tasks_value) = member.value() else {
                continue;
            };

            if let AnyJsonValue::JsonObjectValue(tasks_obj) = tasks_value {
                let mut task_names = HashSet::new();
                extract_keys_from_object(&tasks_obj, &mut task_names);
                return task_names;
            }
        }
    }

    HashSet::new()
}

/// Helper to get the text content of a JSON member name
fn get_member_name_text(name: &biome_json_syntax::JsonMemberName) -> String {
    // The name is a string literal, we need to extract the text without quotes
    name.inner_string_text()
        .map(|t| t.to_string())
        .unwrap_or_default()
}

/// Extracts keys from a JSON object and normalizes task names
fn extract_keys_from_object(
    obj: &biome_json_syntax::JsonObjectValue,
    task_names: &mut HashSet<String>,
) {
    for member in obj.json_member_list() {
        let Ok(member) = member else { continue };
        let Ok(name) = member.name() else { continue };

        let task_name = get_member_name_text(&name);

        // Strip package prefix if present (e.g., "pkg#build" -> "build")
        // Also handle root tasks like "//#build" -> "build"
        let normalized = if let Some(pos) = task_name.find('#') {
            task_name[pos + 1..].to_string()
        } else {
            task_name
        };

        task_names.insert(normalized);
    }
}

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
/// package B, then for tasks defined in `pipeline_tasks`, A#task depends on
/// B#task.
///
/// The `pipeline_tasks` parameter should contain task names from turbo.json's
/// tasks configuration. Use `read_pipeline_tasks` to obtain these from the
/// repository's turbo.json file.
pub fn task_graph_to_data(
    pkg_graph: &PackageGraph,
    pipeline_tasks: &HashSet<String>,
) -> TaskGraphData {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    // First pass: collect all tasks and create nodes
    for (name, info) in pkg_graph.packages() {
        let package_id = match name {
            PackageName::Root => ROOT_PACKAGE_ID.to_string(),
            PackageName::Other(n) => n.clone(),
        };

        for (script_name, script_cmd) in info.package_json.scripts.iter() {
            let task_id = format!("{}#{}", package_id, script_name);
            nodes.push(TaskNode {
                id: task_id,
                package: package_id.clone(),
                task: script_name.clone(),
                script: script_cmd.value.clone(),
            });
        }
    }

    // Second pass: create edges based on package dependencies
    // For tasks defined in turbo.json, if package A depends on package B,
    // then A#task -> B#task
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
                    // For pipeline tasks that exist in both packages, create edges
                    for script in info.package_json.scripts.keys() {
                        if pipeline_tasks.contains(script)
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

    #[test]
    fn test_parse_pipeline_tasks_basic() {
        let turbo_json = r#"
        {
            "tasks": {
                "build": {},
                "test": {},
                "lint": {}
            }
        }
        "#;
        let tasks = parse_pipeline_tasks(turbo_json);
        assert!(tasks.contains("build"));
        assert!(tasks.contains("test"));
        assert!(tasks.contains("lint"));
        assert_eq!(tasks.len(), 3);
    }

    #[test]
    fn test_parse_pipeline_tasks_with_package_prefix() {
        let turbo_json = r#"
        {
            "tasks": {
                "build": {},
                "web#build": {},
                "//#test": {}
            }
        }
        "#;
        let tasks = parse_pipeline_tasks(turbo_json);
        // Both "build" and "web#build" should normalize to "build"
        assert!(tasks.contains("build"));
        assert!(tasks.contains("test"));
        // Should only have 2 unique task names after normalization
        assert_eq!(tasks.len(), 2);
    }

    #[test]
    fn test_parse_pipeline_tasks_with_comments() {
        let turbo_json = r#"
        {
            // This is a comment
            "tasks": {
                "build": {}, /* inline comment */
                "compile": {}
            }
        }
        "#;
        let tasks = parse_pipeline_tasks(turbo_json);
        assert!(tasks.contains("build"));
        assert!(tasks.contains("compile"));
        assert_eq!(tasks.len(), 2);
    }

    #[test]
    fn test_parse_pipeline_tasks_empty() {
        let turbo_json = r#"
        {
            "tasks": {}
        }
        "#;
        let tasks = parse_pipeline_tasks(turbo_json);
        // Empty tasks object should return empty set
        assert!(tasks.is_empty());
    }

    #[test]
    fn test_parse_pipeline_tasks_no_tasks_key() {
        let turbo_json = r#"
        {
            "globalEnv": ["NODE_ENV"]
        }
        "#;
        let tasks = parse_pipeline_tasks(turbo_json);
        // No tasks key should return empty set
        assert!(tasks.is_empty());
    }

    #[test]
    fn test_parse_pipeline_tasks_invalid_json() {
        let turbo_json = r#"{ invalid json }"#;
        let tasks = parse_pipeline_tasks(turbo_json);
        // Invalid JSON should return empty set
        assert!(tasks.is_empty());
    }

    #[test]
    fn test_parse_pipeline_tasks_custom_tasks() {
        let turbo_json = r#"
        {
            "tasks": {
                "compile": {},
                "bundle": {},
                "deploy": {}
            }
        }
        "#;
        let tasks = parse_pipeline_tasks(turbo_json);
        assert!(tasks.contains("compile"));
        assert!(tasks.contains("bundle"));
        assert!(tasks.contains("deploy"));
        // Should NOT contain defaults since we found tasks
        assert!(!tasks.contains("lint"));
    }
}
