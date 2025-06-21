use std::collections::{HashMap, HashSet};

use biome_deserialize_macros::Deserializable;
use serde_json::Value;
use thiserror::Error;
use turbopath::AbsoluteSystemPath;
use turborepo_repository::{
    discovery::{LocalPackageDiscoveryBuilder, PackageDiscovery, PackageDiscoveryBuilder},
    package_json::PackageJson,
};
use turborepo_ui::ColorConfig;

use super::CommandBase;
use crate::turbo_json::RawTurboJson;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to read package.json: {0}")]
    PackageJsonRead(#[from] turborepo_repository::package_json::Error),
    #[error("Failed to read file: {0}")]
    FileRead(#[from] std::io::Error),
    #[error("Failed to parse JSON: {0}")]
    JsonParse(#[from] serde_json::Error),
    #[error("Failed to discover packages: {0}")]
    Discovery(#[from] turborepo_repository::discovery::Error),
    #[error("Failed to resolve package manager: {0}")]
    PackageManager(#[from] turborepo_repository::package_manager::Error),
    #[error("Failed to read turbo.json: {0}")]
    Config(#[from] crate::config::Error),
}

#[derive(Debug, Clone, Deserializable, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DepsSyncConfig {
    /// Dependencies that should be pinned to a specific version across all
    /// packages by default. Packages can be excluded using the `exceptions`
    /// field.
    #[serde(default)]
    pub pinned_dependencies: HashMap<String, PinnedDependency>,
    /// Dependencies that should be ignored in specific packages
    #[serde(default)]
    pub ignored_dependencies: Vec<IgnoredDependency>,
}

#[derive(Debug, Clone, Default, Deserializable, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinnedDependency {
    /// The version to pin this dependency to
    #[serde(default)]
    pub version: String,
    /// Packages where this dependency should NOT be pinned (exceptions to the
    /// rule)
    #[serde(default)]
    pub exceptions: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserializable, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IgnoredDependency {
    /// The name of the dependency to ignore
    #[serde(default)]
    pub dependency: String,
    /// The packages where this dependency should be ignored
    #[serde(default)]
    pub packages: Vec<String>,
}

impl Default for DepsSyncConfig {
    fn default() -> Self {
        Self {
            pinned_dependencies: HashMap::new(),
            ignored_dependencies: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
struct DependencyInfo {
    package_name: String,
    dependency_name: String,
    version: String,
    dep_type: DependencyType,
}

#[derive(Debug, Clone, PartialEq)]
enum DependencyType {
    Dependencies,
    DevDependencies,
    OptionalDependencies,
}

impl std::fmt::Display for DependencyType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DependencyType::Dependencies => write!(f, "dependencies"),
            DependencyType::DevDependencies => write!(f, "devDependencies"),
            DependencyType::OptionalDependencies => write!(f, "optionalDependencies"),
        }
    }
}

#[derive(Debug, Clone)]
struct DependencyUsage {
    package_name: String,
    version: String,
    dependency_type: DependencyType,
}

struct DependencyConflict {
    dependency_name: String,
    conflicting_packages: Vec<DependencyUsage>,
}

pub async fn run(base: &CommandBase) -> Result<i32, Error> {
    let color_config = base.color_config;

    println!("🔍 Scanning workspace packages for dependency conflicts...\n");

    let config = load_deps_sync_config(&base.repo_root).await?;
    let all_deps = collect_all_dependencies(&base.repo_root).await?;
    let filtered_deps = apply_configuration_filters(&all_deps, &config);
    let conflicts = find_dependency_conflicts(&filtered_deps);
    let pinned_conflicts = find_pinned_version_conflicts(&all_deps, &config);

    let all_conflicts: Vec<_> = conflicts.into_iter().chain(pinned_conflicts).collect();

    if all_conflicts.is_empty() {
        print_success("✅ All dependencies are in sync!", color_config);
        Ok(0)
    } else {
        print_conflicts(&all_conflicts, color_config);
        Ok(1)
    }
}

async fn load_deps_sync_config(repo_root: &AbsoluteSystemPath) -> Result<DepsSyncConfig, Error> {
    let turbo_json_path = repo_root.join_component("turbo.json");

    let raw_turbo_json = match RawTurboJson::read(repo_root, &turbo_json_path)? {
        Some(turbo_json) => turbo_json,
        None => return Ok(DepsSyncConfig::default()),
    };

    Ok(raw_turbo_json.deps_sync.unwrap_or_default())
}

async fn collect_all_dependencies(
    repo_root: &AbsoluteSystemPath,
) -> Result<Vec<DependencyInfo>, Error> {
    let mut all_deps = Vec::new();

    // Use workspace discovery to find only workspace packages
    let discovery = LocalPackageDiscoveryBuilder::new(repo_root.to_owned(), None, None).build()?;
    let workspace_response = discovery.discover_packages().await?;

    // Process each workspace package
    for workspace_data in workspace_response.workspaces {
        let package_json_path = &workspace_data.package_json;

        if let Ok(package_json) = PackageJson::load(package_json_path) {
            let package_name = package_json
                .name
                .as_ref()
                .map(|s| s.as_str().to_string())
                .unwrap_or_else(|| {
                    // Use directory name as fallback
                    package_json_path
                        .parent()
                        .unwrap()
                        .file_name()
                        .unwrap_or("unknown")
                        .to_string()
                });

            // Read raw JSON to get all dependency types
            let raw_content = std::fs::read_to_string(package_json_path)?;
            let raw_json: Value = serde_json::from_str(&raw_content)?;

            // Extract dependencies of each type
            if let Some(deps) = raw_json.get("dependencies").and_then(|v| v.as_object()) {
                for (dep_name, version) in deps {
                    if let Some(version_str) = version.as_str() {
                        all_deps.push(DependencyInfo {
                            package_name: package_name.clone(),
                            dependency_name: dep_name.clone(),
                            version: version_str.to_string(),
                            dep_type: DependencyType::Dependencies,
                        });
                    }
                }
            }

            if let Some(dev_deps) = raw_json.get("devDependencies").and_then(|v| v.as_object()) {
                for (dep_name, version) in dev_deps {
                    if let Some(version_str) = version.as_str() {
                        all_deps.push(DependencyInfo {
                            package_name: package_name.clone(),
                            dependency_name: dep_name.clone(),
                            version: version_str.to_string(),
                            dep_type: DependencyType::DevDependencies,
                        });
                    }
                }
            }

            // Skip peerDependencies - they're meant to be provided by the consuming
            // application and often intentionally have different version
            // constraints

            if let Some(opt_deps) = raw_json
                .get("optionalDependencies")
                .and_then(|v| v.as_object())
            {
                for (dep_name, version) in opt_deps {
                    if let Some(version_str) = version.as_str() {
                        all_deps.push(DependencyInfo {
                            package_name: package_name.clone(),
                            dependency_name: dep_name.clone(),
                            version: version_str.to_string(),
                            dep_type: DependencyType::OptionalDependencies,
                        });
                    }
                }
            }
        }
    }

    Ok(all_deps)
}

fn apply_configuration_filters(
    dependencies: &[DependencyInfo],
    config: &DepsSyncConfig,
) -> Vec<DependencyInfo> {
    dependencies
        .iter()
        .filter(|dep| {
            // Check if this dependency should be ignored in this package
            for ignored in &config.ignored_dependencies {
                if ignored.dependency == dep.dependency_name
                    && ignored.packages.contains(&dep.package_name)
                {
                    return false;
                }
            }
            true
        })
        .cloned()
        .collect()
}

fn find_pinned_version_conflicts(
    dependencies: &[DependencyInfo],
    config: &DepsSyncConfig,
) -> Vec<DependencyConflict> {
    let mut conflicts = Vec::new();

    for (dep_name, pinned_config) in &config.pinned_dependencies {
        let mut conflicting_packages = Vec::new();

        for dep in dependencies {
            if dep.dependency_name == *dep_name {
                // Check if this package is exempted from the pinned version
                if pinned_config.exceptions.contains(&dep.package_name) {
                    continue;
                }

                // Check if the version matches the pinned version
                if dep.version != pinned_config.version {
                    conflicting_packages.push(DependencyUsage {
                        package_name: dep.package_name.clone(),
                        version: dep.version.clone(),
                        dependency_type: dep.dep_type.clone(),
                    });
                }
            }
        }

        if !conflicting_packages.is_empty() {
            conflicts.push(DependencyConflict {
                dependency_name: dep_name.clone(),
                conflicting_packages,
            });
        }
    }

    conflicts
}

fn find_dependency_conflicts(all_deps: &[DependencyInfo]) -> Vec<DependencyConflict> {
    let mut dependency_map: HashMap<String, Vec<DependencyUsage>> = HashMap::new();

    // Group dependencies by name
    for dep in all_deps {
        dependency_map
            .entry(dep.dependency_name.clone())
            .or_default()
            .push(DependencyUsage {
                package_name: dep.package_name.clone(),
                version: dep.version.clone(),
                dependency_type: dep.dep_type.clone(),
            });
    }

    let mut conflicts = Vec::new();

    // Find conflicts (same dependency with different versions)
    for (dep_name, usages) in dependency_map {
        // Check if we have multiple different versions
        let unique_versions: HashSet<&String> = usages.iter().map(|usage| &usage.version).collect();

        if unique_versions.len() > 1 {
            conflicts.push(DependencyConflict {
                dependency_name: dep_name,
                conflicting_packages: usages,
            });
        }
    }

    // Sort conflicts by dependency name for consistent output
    conflicts.sort_by(|a, b| a.dependency_name.cmp(&b.dependency_name));

    conflicts
}

fn print_conflicts(conflicts: &[DependencyConflict], color_config: ColorConfig) {
    use turborepo_ui::{BOLD, BOLD_RED, CYAN, YELLOW};

    for conflict in conflicts {
        let dep_name = if color_config.should_strip_ansi {
            conflict.dependency_name.clone()
        } else {
            format!("{}", BOLD.apply_to(&conflict.dependency_name))
        };

        println!("  {}:", dep_name);

        // Group by version for cleaner output
        let mut version_groups: HashMap<String, Vec<(String, DependencyType)>> = HashMap::new();
        for usage in &conflict.conflicting_packages {
            version_groups
                .entry(usage.version.clone())
                .or_default()
                .push((usage.package_name.clone(), usage.dependency_type.clone()));
        }

        let mut sorted_versions: Vec<_> = version_groups.into_iter().collect();
        sorted_versions.sort_by(|a, b| a.0.cmp(&b.0));

        for (version, packages) in sorted_versions {
            let version_display = if color_config.should_strip_ansi {
                version
            } else {
                format!("{}", YELLOW.apply_to(&version))
            };

            println!("    {} →", version_display);

            for (package_name, dep_type) in packages {
                let package_display = if color_config.should_strip_ansi {
                    format!("{} ({})", package_name, dep_type)
                } else {
                    format!("{} ({})", CYAN.apply_to(&package_name), dep_type)
                };
                println!("      {}", package_display);
            }
        }
        println!();
    }

    println!(
        "💡 To fix these conflicts, ensure all packages use the same version for each dependency."
    );

    let error_prefix = if color_config.should_strip_ansi {
        "❌"
    } else {
        &format!("{}", BOLD_RED.apply_to("❌"))
    };

    println!(
        "\n{} Found {} dependency conflicts.",
        error_prefix,
        conflicts.len()
    );
}

fn print_success(message: &str, color_config: ColorConfig) {
    if color_config.should_strip_ansi {
        println!("{}", message);
    } else {
        use turborepo_ui::BOLD_GREEN;
        println!("{}", BOLD_GREEN.apply_to(message));
    }
}
