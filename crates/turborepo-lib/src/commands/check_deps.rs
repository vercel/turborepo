use std::{collections::HashMap, process};

use miette::Diagnostic;
use serde_json::Value;
use thiserror::Error;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_ui::{cprintln, BOLD, BOLD_GREEN, BOLD_RED, GREY};

use crate::{cli, commands::CommandBase};

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("Failed to read package.json at {path}")]
    FailedToReadPackageJson {
        path: AbsoluteSystemPathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to parse package.json at {path}")]
    FailedToParsePackageJson {
        path: AbsoluteSystemPathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("Failed to find any package.json files")]
    NoPackageJsonFound,
}

#[derive(Debug)]
struct DependencyVersion {
    version: String,
    locations: Vec<String>,
}

fn find_inconsistencies(
    dep_map: &HashMap<String, DependencyVersion>,
) -> HashMap<String, HashMap<String, Vec<String>>> {
    let mut result: HashMap<String, HashMap<String, Vec<String>>> = HashMap::new();

    // First, group all dependencies by their base name (without version suffix)
    for (full_name, dep_info) in dep_map {
        // Extract base name from keys like "package@version" or just use the original
        // name, handling scoped packages properly
        let base_name = if full_name.starts_with('@') {
            // This is a scoped package like "@babel/core" or "@types/react"
            // If it also has @version, extract just the package name part
            if let Some(version_idx) = full_name[1..].find('@') {
                &full_name[0..=version_idx]
            } else {
                // It's a scoped package without version suffix
                full_name.as_str()
            }
        } else if full_name.contains('@') {
            // This is a versioned entry like "react@16.0.0"
            full_name.split('@').next().unwrap()
        } else {
            // This is a regular entry
            full_name.as_str()
        };

        // Add this version to the base dependency's version map
        result
            .entry(base_name.to_string())
            .or_insert_with(HashMap::new)
            .entry(dep_info.version.clone())
            .or_insert_with(Vec::new)
            .extend(dep_info.locations.clone());
    }

    // Filter out dependencies with only one version
    result.retain(|_, versions| versions.len() > 1);

    result
}

pub async fn run(base: CommandBase) -> Result<i32, cli::Error> {
    let repo_root = &base.repo_root;
    let color_config = base.color_config;

    // Find all package.json files
    let package_json_files = find_package_json_files(repo_root)?;

    if package_json_files.is_empty() {
        return Err(Error::NoPackageJsonFound.into());
    }

    cprintln!(
        color_config,
        BOLD,
        "Checking dependency versions across {} package.json files...",
        package_json_files.len()
    );

    // Collect all dependencies and their versions - we use a single map for all
    // dependency types
    let mut all_dependencies_map: HashMap<String, DependencyVersion> = HashMap::new();

    for package_json_path in &package_json_files {
        process_package_json(package_json_path, repo_root, &mut all_dependencies_map)?;
    }

    // Check for inconsistencies and build report
    let mut has_inconsistencies = false;
    let mut total_inconsistencies = 0;

    let inconsistencies = find_inconsistencies(&all_dependencies_map);

    if !inconsistencies.is_empty() {
        has_inconsistencies = true;
        total_inconsistencies += inconsistencies.len();

        cprintln!(color_config, BOLD, "\nInconsistent dependencies found:");

        for (dep_name, versions) in inconsistencies {
            cprintln!(
                color_config,
                BOLD_RED,
                "  {} has {} different versions:",
                dep_name,
                versions.len()
            );

            for (version, locations) in versions {
                println!("    {} version '{}' in:", GREY.apply_to("→"), version);

                for location in &locations {
                    println!("      {}", location);
                }
            }

            println!();
        }
    }

    if has_inconsistencies {
        cprintln!(
            color_config,
            BOLD_RED,
            "\nDependency check failed: {} inconsistent dependencies found.",
            total_inconsistencies
        );
        Ok(1)
    } else {
        cprintln!(
            color_config,
            BOLD_GREEN,
            "\nDependency check passed: no inconsistent versions found."
        );
        Ok(0)
    }
}

fn find_package_json_files(
    repo_root: &AbsoluteSystemPath,
) -> Result<Vec<AbsoluteSystemPathBuf>, Error> {
    let output = process::Command::new("find")
        .arg(repo_root.as_str())
        .arg("-name")
        .arg("package.json")
        .arg("-type")
        .arg("f")
        .arg("-not")
        .arg("-path")
        .arg("*/node_modules/*")
        .output()
        .map_err(|e| Error::FailedToReadPackageJson {
            path: repo_root.to_owned(),
            source: e,
        })?;

    if !output.status.success() {
        return Err(Error::NoPackageJsonFound);
    }

    let files_str = String::from_utf8_lossy(&output.stdout);
    let files: Vec<AbsoluteSystemPathBuf> = files_str
        .lines()
        .map(|line| AbsoluteSystemPathBuf::new(line.trim()).unwrap())
        .collect();

    Ok(files)
}

fn process_package_json(
    package_json_path: &AbsoluteSystemPath,
    repo_root: &AbsoluteSystemPath,
    all_dependencies_map: &mut HashMap<String, DependencyVersion>,
) -> Result<(), Error> {
    let package_json_content =
        package_json_path
            .read_to_string()
            .map_err(|e| Error::FailedToReadPackageJson {
                path: package_json_path.to_owned(),
                source: e,
            })?;

    let package_json: Value = serde_json::from_str(&package_json_content).map_err(|e| {
        Error::FailedToParsePackageJson {
            path: package_json_path.to_owned(),
            source: e,
        }
    })?;

    let relative_path = package_json_path
        .as_path()
        .strip_prefix(repo_root.as_path())
        .unwrap_or_else(|_| package_json_path.as_path())
        .to_string();

    let package_name = package_json
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("unnamed-package");

    let location = format!("{} ({})", package_name, relative_path);

    // Process both dependency types
    process_dependency_type(
        "dependencies",
        &package_json,
        &location,
        all_dependencies_map,
    );
    process_dependency_type(
        "devDependencies",
        &package_json,
        &location,
        all_dependencies_map,
    );

    Ok(())
}

fn process_dependency_type(
    dep_type: &str,
    package_json: &Value,
    location: &str,
    all_dependencies_map: &mut HashMap<String, DependencyVersion>,
) {
    if let Some(deps) = package_json.get(dep_type).and_then(|v| v.as_object()) {
        for (dep_name, version_value) in deps {
            if let Some(version) = version_value.as_str() {
                // Check if we already have this dependency
                if let Some(entry) = all_dependencies_map.get(&dep_name.clone()) {
                    if entry.version == version {
                        // Same version, just add the location
                        let mut locations = entry.locations.clone();
                        locations.push(location.to_string());

                        // Update with the new locations
                        all_dependencies_map.insert(
                            dep_name.clone(),
                            DependencyVersion {
                                version: version.to_string(),
                                locations,
                            },
                        );
                    } else {
                        // Different version - create new versioned entries in the result directly
                        // We DON'T want to modify the key names here

                        // We need to preserve both versions with their locations
                        // Create a versioned map in the find_inconsistencies function

                        // For now, just add this version to the result with a unique key
                        // that preserves the package name but includes version info
                        let versioned_key = format!("{}@{}", dep_name, version);

                        all_dependencies_map.insert(
                            versioned_key,
                            DependencyVersion {
                                version: version.to_string(),
                                locations: vec![location.to_string()],
                            },
                        );
                    }
                } else {
                    // New dependency, just add it with the clean name
                    all_dependencies_map.insert(
                        dep_name.clone(),
                        DependencyVersion {
                            version: version.to_string(),
                            locations: vec![location.to_string()],
                        },
                    );
                }
            }
        }
    }
}
