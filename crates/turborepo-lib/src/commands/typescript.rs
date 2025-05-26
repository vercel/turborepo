use std::{collections::HashSet, fs, io, path::PathBuf};

use camino::Utf8PathBuf;
use oxc_resolver::{ProjectReferenceSerde, TsConfigSerde};
use serde_json::Value;
use thiserror::Error;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_repository::package_graph::PackageNode;
use turborepo_signals::{listeners::get_signal, SignalHandler};
use turborepo_telemetry::events::command::CommandEventBuilder;

use crate::{cli, commands::CommandBase, run::builder::RunBuilder};

#[derive(Error, Debug)]
pub enum TypeScriptError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Parse error: {0}")]
    Parse(String),
    #[error("Resolve error: {0}")]
    Resolve(String),
}

impl From<TypeScriptError> for cli::Error {
    fn from(err: TypeScriptError) -> Self {
        match err {
            TypeScriptError::Io(e) => cli::Error::Path(e.into()),
            TypeScriptError::Json(e) => cli::Error::SerdeJson(e),
            TypeScriptError::Parse(e) => cli::Error::Parse(e),
            TypeScriptError::Resolve(e) => cli::Error::Parse(e),
        }
    }
}

fn get_all_dependencies(package_json: &Value) -> HashSet<String> {
    let mut deps = HashSet::new();

    // Helper to extract dependencies from a specific field
    let mut extract_deps = |field: &str| {
        if let Some(deps_obj) = package_json.get(field) {
            if let Some(obj) = deps_obj.as_object() {
                for key in obj.keys() {
                    deps.insert(key.clone());
                }
            }
        }
    };

    // Check all dependency types
    extract_deps("dependencies");
    extract_deps("devDependencies");
    extract_deps("peerDependencies");
    extract_deps("optionalDependencies");

    deps
}

fn update_tsconfig_references(
    tsconfig_path: &AbsoluteSystemPath,
    package_name: &str,
    dependencies: &HashSet<String>,
    package_paths: &[(String, AbsoluteSystemPathBuf)],
) -> Result<(), TypeScriptError> {
    let mut tsconfig_content = fs::read_to_string(tsconfig_path)?;
    let mut tsconfig =
        TsConfigSerde::parse(false, tsconfig_path.as_std_path(), &mut tsconfig_content)?;

    // Keep track of existing paths to avoid duplicates
    let mut existing_paths: HashSet<String> = tsconfig
        .references
        .iter()
        .map(|ref_obj| ref_obj.path.to_string_lossy().into_owned())
        .collect();

    // Get the current package's directory
    let current_package_dir = tsconfig_path.parent().unwrap();

    // Special case for root package
    if package_name == "//" {
        // Add references to all packages in the repository
        for (_, dep_path) in package_paths {
            let dep_tsconfig = dep_path.join_component("tsconfig.json");
            if dep_tsconfig.exists() {
                // Calculate relative path from root to package
                let relative_path = if let Ok(path) = dep_path
                    .as_path()
                    .strip_prefix(current_package_dir.as_path())
                {
                    let path_str = format!("./{}", path.to_string());
                    // Skip adding reference to root directory
                    if path_str != "./" {
                        Some(path_str)
                    } else {
                        None
                    }
                } else {
                    None
                };

                if let Some(path_str) = relative_path {
                    if !existing_paths.contains(&path_str) {
                        let path_buf = PathBuf::from(path_str.clone());
                        tsconfig.references.push(ProjectReferenceSerde {
                            path: path_buf,
                            tsconfig: None,
                        });
                        existing_paths.insert(path_str);
                    }
                }
            }
        }
    } else {
        // Regular package case - add references for each dependency that has a
        // tsconfig.json
        for dep in dependencies {
            if let Some((_, dep_path)) = package_paths.iter().find(|(name, _)| name == dep) {
                let dep_tsconfig = dep_path.join_component("tsconfig.json");
                if dep_tsconfig.exists() {
                    // Calculate relative path from current package to dependency
                    let relative_path = if let Ok(path) = dep_path
                        .as_path()
                        .strip_prefix(current_package_dir.as_path())
                    {
                        // If dependency is a child of current package
                        Some(format!("./{}", path.to_string()))
                    } else if let Ok(path) = current_package_dir
                        .as_path()
                        .strip_prefix(dep_path.as_path())
                    {
                        // If current package is a child of dependency
                        Some(format!("../{}", path.to_string()))
                    } else {
                        // If neither is a child of the other, calculate the common ancestor
                        let current_components: Vec<_> = current_package_dir.components().collect();
                        let dep_components: Vec<_> = dep_path.components().collect();

                        // Find the common prefix
                        let common_prefix_len = current_components
                            .iter()
                            .zip(dep_components.iter())
                            .take_while(|(a, b)| a == b)
                            .count();

                        // Calculate the number of ".." needed
                        let up_count = current_components.len() - common_prefix_len;
                        let down_path = dep_components[common_prefix_len..]
                            .iter()
                            .map(|c| c.as_str())
                            .collect::<Vec<_>>()
                            .join("/");

                        Some(format!("{}{}", "../".repeat(up_count), down_path))
                    };

                    if let Some(path_str) = relative_path {
                        // Only add if this path isn't already referenced
                        if !existing_paths.contains(&path_str) {
                            let path_buf = PathBuf::from(path_str.clone());
                            tsconfig.references.push(ProjectReferenceSerde {
                                path: path_buf,
                                tsconfig: None,
                            });
                            existing_paths.insert(path_str);
                        }
                    } else {
                        println!(
                            "Warning: Could not create relative path for dependency {} in package \
                             {}",
                            dep, package_name
                        );
                    }
                }
            }
        }
    }

    // Write back the updated tsconfig
    let mut json: serde_json::Value = serde_json::from_str(&tsconfig_content)?;

    // Update the references array and sort alphabetically by path
    let mut references: Vec<serde_json::Value> = tsconfig
        .references
        .iter()
        .map(|ref_obj| {
            serde_json::json!({
                "path": ref_obj.path.to_string_lossy()
            })
        })
        .collect();

    // Sort references alphabetically by path
    references.sort_by(|a, b| {
        let path_a = a["path"].as_str().unwrap_or("");
        let path_b = b["path"].as_str().unwrap_or("");
        path_a.cmp(path_b)
    });

    json["references"] = serde_json::json!(references);

    // Add empty files array for root package
    if package_name == "//" {
        json["files"] = serde_json::json!([]);
    }

    let updated_content = serde_json::to_string_pretty(&json)?;
    fs::write(tsconfig_path, updated_content)?;

    Ok(())
}

pub async fn run_typescript(
    _config: Option<Utf8PathBuf>,
    base: &CommandBase,
    telemetry: CommandEventBuilder,
) -> Result<(), cli::Error> {
    // Create a Run instance to access the package graph
    let run_builder = RunBuilder::new(base.clone())?;
    let signal_handler = SignalHandler::new(get_signal()?);
    let run = run_builder.build(&signal_handler, telemetry).await?;

    // First pass: collect all package paths
    let mut package_paths = Vec::new();
    for node in run.pkg_dep_graph().node_indices() {
        if let Some(package_node) = run.pkg_dep_graph().get_package_by_index(node) {
            if let PackageNode::Workspace(pkg_name) = package_node {
                if let Some(pkg_info) = run.pkg_dep_graph().package_info(pkg_name) {
                    let package_dir = base
                        .repo_root
                        .resolve(pkg_info.package_json_path())
                        .parent()
                        .unwrap()
                        .to_owned();
                    package_paths.push((pkg_name.to_string(), package_dir));
                }
            }
        }
    }

    // Create root tsconfig.json if it doesn't exist
    let root_tsconfig = base.repo_root.join_component("tsconfig.json");
    if !root_tsconfig.exists() {
        let root_config = serde_json::json!({
            "files": [],
            "references": []
        });
        fs::write(
            &root_tsconfig,
            serde_json::to_string_pretty(&root_config).map_err(TypeScriptError::Json)?,
        )
        .map_err(TypeScriptError::Io)?;
    }

    // Second pass: update tsconfig.json files
    for node in run.pkg_dep_graph().node_indices() {
        if let Some(package_node) = run.pkg_dep_graph().get_package_by_index(node) {
            if let PackageNode::Workspace(pkg_name) = package_node {
                if let Some(pkg_info) = run.pkg_dep_graph().package_info(pkg_name) {
                    let package_json_path = base.repo_root.resolve(pkg_info.package_json_path());
                    let package_dir = package_json_path.parent().unwrap();
                    let tsconfig_path = package_dir.join_component("tsconfig.json");

                    // Only process packages that have a tsconfig.json
                    if tsconfig_path.exists() {
                        // Read package.json
                        let package_json_content =
                            fs::read_to_string(&package_json_path).map_err(TypeScriptError::Io)?;
                        let package_json: Value = serde_json::from_str(&package_json_content)
                            .map_err(TypeScriptError::Json)?;

                        // Get all dependencies
                        let dependencies = get_all_dependencies(&package_json);

                        // Update tsconfig.json with project references
                        update_tsconfig_references(
                            &tsconfig_path,
                            &pkg_name.to_string(),
                            &dependencies,
                            &package_paths,
                        )?;
                    }
                }
            }
        }
    }

    Ok(())
}
