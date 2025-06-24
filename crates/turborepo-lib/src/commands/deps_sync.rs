use std::collections::{HashMap, HashSet};

use biome_deserialize_macros::Deserializable;
use serde_json::Value;
use thiserror::Error;
use turbopath::AbsoluteSystemPath;
use turborepo_repository::{
    discovery::{
        DiscoveryResponse, LocalPackageDiscoveryBuilder, PackageDiscovery, PackageDiscoveryBuilder,
    },
    package_json::PackageJson,
};
use turborepo_ui::ColorConfig;

use super::CommandBase;
use crate::turbo_json::RawTurboJson;

const DEPENDENCY_TYPES: [&str; 3] = ["dependencies", "devDependencies", "optionalDependencies"];
const SUCCESS_PREFIX: &str = "✅";
const ERROR_PREFIX: &str = "❌";
const SCANNING_MESSAGE: &str = "🔍 Scanning workspace packages for dependency conflicts...";

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to read package.json at {path}: {source}")]
    PackageJsonRead {
        path: String,
        #[source]
        source: turborepo_repository::package_json::Error,
    },
    #[error("Failed to read file at {path}: {source}")]
    FileRead {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("Failed to parse JSON in {path}: {source}")]
    JsonParse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("Failed to discover packages: {0}")]
    Discovery(#[from] turborepo_repository::discovery::Error),
    #[error("Failed to resolve package manager: {0}")]
    PackageManager(#[from] turborepo_repository::package_manager::Error),
    #[error("Failed to read turbo.json: {0}")]
    Config(#[from] crate::config::Error),
    #[error(
        "deps-sync is not needed for single-package workspaces. This command analyzes dependency \
         conflicts across multiple packages in a workspace."
    )]
    SinglePackageWorkspace,
}

#[derive(Debug, Clone, Deserializable, serde::Deserialize, serde::Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DepsSyncConfig {
    /// Dependencies that should be pinned to a specific version across all
    /// packages by default. Packages can be excluded using the `exceptions`
    /// field.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub pinned_dependencies: HashMap<String, PinnedDependency>,
    /// Dependencies that should be ignored in specific packages.
    /// The `exceptions` field lists the packages where the dependency should be
    /// ignored.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub ignored_dependencies: HashMap<String, IgnoredDependency>,
    /// Whether to include optionalDependencies in conflict analysis.
    /// Defaults to false since optional dependencies are often
    /// platform-specific and should gracefully handle version differences.
    #[serde(default)]
    pub include_optional_dependencies: bool,
}

#[derive(
    Debug, Clone, Default, PartialEq, Deserializable, serde::Deserialize, serde::Serialize,
)]
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

#[derive(
    Debug, Clone, Default, PartialEq, Deserializable, serde::Deserialize, serde::Serialize,
)]
#[serde(rename_all = "camelCase")]
pub struct IgnoredDependency {
    /// Packages where this dependency should be ignored
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exceptions: Vec<String>,
}

#[derive(Debug, Clone)]
struct DependencyInfo {
    package_name: String,
    package_path: String,
    dependency_name: String,
    version: String,
    dependency_type: DependencyType,
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
    package_path: String,
}

#[derive(Debug, Clone)]
struct DependencyConflict {
    dependency_name: String,
    conflicting_packages: Vec<DependencyUsage>,
    conflict_reason: Option<String>,
}

/// Performance optimization: Create lookup sets for faster exception checking
#[derive(Debug)]
struct OptimizedConfig {
    pinned_dependencies: HashMap<String, PinnedDependency>,
    ignored_dependencies: HashMap<String, IgnoredDependency>,
    include_optional_dependencies: bool,
    // Optimized lookup sets
    pinned_dependency_names: HashSet<String>,
    ignored_exception_sets: HashMap<String, HashSet<String>>,
    pinned_exception_sets: HashMap<String, HashSet<String>>,
}

impl From<DepsSyncConfig> for OptimizedConfig {
    fn from(config: DepsSyncConfig) -> Self {
        let pinned_dependency_names = config.pinned_dependencies.keys().cloned().collect();

        let ignored_exception_sets = config
            .ignored_dependencies
            .iter()
            .map(|(dep_name, ignored_dep)| {
                (
                    dep_name.clone(),
                    ignored_dep.exceptions.iter().cloned().collect(),
                )
            })
            .collect();

        let pinned_exception_sets = config
            .pinned_dependencies
            .iter()
            .map(|(dep_name, pinned_dep)| {
                (
                    dep_name.clone(),
                    pinned_dep.exceptions.iter().cloned().collect(),
                )
            })
            .collect();

        Self {
            pinned_dependencies: config.pinned_dependencies,
            ignored_dependencies: config.ignored_dependencies,
            include_optional_dependencies: config.include_optional_dependencies,
            pinned_dependency_names,
            ignored_exception_sets,
            pinned_exception_sets,
        }
    }
}

impl From<OptimizedConfig> for DepsSyncConfig {
    fn from(config: OptimizedConfig) -> Self {
        Self {
            pinned_dependencies: config.pinned_dependencies,
            ignored_dependencies: config.ignored_dependencies,
            include_optional_dependencies: config.include_optional_dependencies,
        }
    }
}

pub async fn run(base: &CommandBase, allowlist: bool) -> Result<i32, Error> {
    let color_config = base.color_config;

    println!("{}", SCANNING_MESSAGE);

    let deps_sync_config = load_deps_sync_config(&base.repo_root).await?;
    let optimized_config = OptimizedConfig::from(deps_sync_config);

    // Validate workspace has multiple packages
    let workspace_response = discover_workspace_packages(&base.repo_root).await?;
    validate_multi_package_workspace(&workspace_response)?;

    // Print configuration summary
    print_configuration_summary(&optimized_config, color_config);

    // Collect and analyze dependencies
    let all_dependencies =
        collect_all_dependencies(&base.repo_root, workspace_response, &optimized_config).await?;
    let version_conflicts = find_version_conflicts(&all_dependencies, &optimized_config);
    let pinned_conflicts = find_pinned_version_conflicts(&all_dependencies, &optimized_config);

    let all_conflicts: Vec<_> = version_conflicts
        .into_iter()
        .chain(pinned_conflicts)
        .collect();

    // Handle results
    handle_analysis_results(
        all_conflicts,
        allowlist,
        &base.repo_root,
        &optimized_config.into(),
        color_config,
    )
    .await
}

async fn load_deps_sync_config(repo_root: &AbsoluteSystemPath) -> Result<DepsSyncConfig, Error> {
    let config_opts = crate::config::ConfigurationOptions::default();
    let turbo_json_path = config_opts
        .root_turbo_json_path(repo_root)
        .map_err(Error::Config)?;

    let raw_turbo_json = match RawTurboJson::read(repo_root, &turbo_json_path)? {
        Some(turbo_json) => turbo_json,
        None => return Ok(DepsSyncConfig::default()),
    };

    Ok(raw_turbo_json.deps_sync.unwrap_or_default())
}

async fn discover_workspace_packages(
    repo_root: &AbsoluteSystemPath,
) -> Result<DiscoveryResponse, Error> {
    let discovery = LocalPackageDiscoveryBuilder::new(repo_root.to_owned(), None, None).build()?;
    discovery
        .discover_packages()
        .await
        .map_err(Error::Discovery)
}

fn validate_multi_package_workspace(workspace_response: &DiscoveryResponse) -> Result<(), Error> {
    if workspace_response.workspaces.len() <= 1 {
        return Err(Error::SinglePackageWorkspace);
    }
    Ok(())
}

fn print_configuration_summary(config: &OptimizedConfig, color_config: ColorConfig) {
    if !config.ignored_dependencies.is_empty() {
        let ignored_count = config.ignored_dependencies.len();
        let dependency_word = if ignored_count == 1 {
            "dependency"
        } else {
            "dependencies"
        };
        let message = format!(
            "→ {} ignored {} in `turbo.json`",
            ignored_count, dependency_word
        );

        print_colored_message(&message, color_config, MessageType::Info);
    }
    println!();
}

enum MessageType {
    Success,
    Error,
    Info,
}

fn print_colored_message(message: &str, color_config: ColorConfig, message_type: MessageType) {
    if color_config.should_strip_ansi {
        println!("{}", message);
    } else {
        use turborepo_ui::{BOLD_GREEN, BOLD_RED, GREY};
        let styled_message = match message_type {
            MessageType::Success => format!("{}", BOLD_GREEN.apply_to(message)),
            MessageType::Error => format!("{}", BOLD_RED.apply_to(message)),
            MessageType::Info => format!("{}", GREY.apply_to(message)),
        };
        println!("{}", styled_message);
    }
}

async fn handle_analysis_results(
    conflicts: Vec<DependencyConflict>,
    allowlist: bool,
    repo_root: &AbsoluteSystemPath,
    config: &DepsSyncConfig,
    color_config: ColorConfig,
) -> Result<i32, Error> {
    if conflicts.is_empty() {
        print_colored_message(
            "✅ All dependencies are in sync!",
            color_config,
            MessageType::Success,
        );
        Ok(0)
    } else if allowlist {
        generate_and_write_allowlist(conflicts, repo_root, config, color_config).await
    } else {
        print_conflicts(&conflicts, color_config);
        Ok(1)
    }
}

async fn generate_and_write_allowlist(
    conflicts: Vec<DependencyConflict>,
    repo_root: &AbsoluteSystemPath,
    current_config: &DepsSyncConfig,
    color_config: ColorConfig,
) -> Result<i32, Error> {
    let allowlist_config = generate_allowlist_config(&conflicts, current_config);
    write_allowlist_config(repo_root, &allowlist_config).await?;

    let success_message = format!(
        "✅ Generated allowlist configuration for {} conflicts in turbo.json. Dependencies are \
         now synchronized!",
        conflicts.len()
    );
    print_colored_message(&success_message, color_config, MessageType::Success);
    Ok(0)
}

async fn collect_all_dependencies(
    repo_root: &AbsoluteSystemPath,
    workspace_response: DiscoveryResponse,
    config: &OptimizedConfig,
) -> Result<Vec<DependencyInfo>, Error> {
    let mut all_dependencies = Vec::new();

    for workspace_data in workspace_response.workspaces {
        let package_dependencies =
            collect_package_dependencies(repo_root, &workspace_data.package_json, config).await?;
        all_dependencies.extend(package_dependencies);
    }

    Ok(all_dependencies)
}

async fn collect_package_dependencies(
    repo_root: &AbsoluteSystemPath,
    package_json_path: &AbsoluteSystemPath,
    config: &OptimizedConfig,
) -> Result<Vec<DependencyInfo>, Error> {
    let package_json =
        PackageJson::load(package_json_path).map_err(|e| Error::PackageJsonRead {
            path: package_json_path.to_string(),
            source: e,
        })?;

    let package_name = extract_package_name(&package_json, package_json_path);
    let relative_package_path = calculate_relative_path(repo_root, package_json_path);

    let raw_content = std::fs::read_to_string(package_json_path).map_err(|e| Error::FileRead {
        path: package_json_path.to_string(),
        source: e,
    })?;

    let raw_json: Value = serde_json::from_str(&raw_content).map_err(|e| Error::JsonParse {
        path: package_json_path.to_string(),
        source: e,
    })?;

    Ok(extract_dependencies_from_json(
        &raw_json,
        &package_name,
        &relative_package_path,
        config,
    ))
}

fn extract_package_name(
    package_json: &PackageJson,
    package_json_path: &AbsoluteSystemPath,
) -> String {
    package_json
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
        })
}

fn calculate_relative_path(
    repo_root: &AbsoluteSystemPath,
    package_json_path: &AbsoluteSystemPath,
) -> String {
    repo_root
        .anchor(package_json_path.parent().unwrap())
        .map(|p| p.to_string())
        .unwrap_or_else(|_| package_json_path.parent().unwrap().to_string())
}

fn extract_dependencies_from_json(
    raw_json: &Value,
    package_name: &str,
    relative_package_path: &str,
    config: &OptimizedConfig,
) -> Vec<DependencyInfo> {
    let mut dependencies = Vec::new();

    let mut dependency_types = vec![
        ("dependencies", DependencyType::Dependencies),
        ("devDependencies", DependencyType::DevDependencies),
    ];

    // Only include optionalDependencies if the configuration allows it
    if config.include_optional_dependencies {
        dependency_types.push(("optionalDependencies", DependencyType::OptionalDependencies));
    }

    for (field_name, dependency_type) in &dependency_types {
        if let Some(deps) = raw_json.get(field_name).and_then(|v| v.as_object()) {
            for (dependency_name, version) in deps {
                if let Some(version_str) = version.as_str() {
                    dependencies.push(DependencyInfo {
                        package_name: package_name.to_string(),
                        package_path: relative_package_path.to_string(),
                        dependency_name: dependency_name.clone(),
                        version: version_str.to_string(),
                        dependency_type: dependency_type.clone(),
                    });
                }
            }
        }
    }

    dependencies
}

/// Check if a dependency should be ignored based on configuration
fn should_ignore_dependency(dependency: &DependencyInfo, config: &OptimizedConfig) -> bool {
    if let Some(exception_set) = config
        .ignored_exception_sets
        .get(&dependency.dependency_name)
    {
        // If package is in exceptions list, do NOT ignore it
        !exception_set.contains(&dependency.package_name)
    } else if config
        .ignored_dependencies
        .contains_key(&dependency.dependency_name)
    {
        // If dependency is in ignored list but no exceptions, ignore it
        true
    } else {
        // Not in ignored list at all
        false
    }
}

/// Check if a package is exempt from a pinned dependency
fn is_exempt_from_pinned_dependency(dependency: &DependencyInfo, config: &OptimizedConfig) -> bool {
    config
        .pinned_exception_sets
        .get(&dependency.dependency_name)
        .map(|exception_set| exception_set.contains(&dependency.package_name))
        .unwrap_or(false)
}

fn find_pinned_version_conflicts(
    dependencies: &[DependencyInfo],
    config: &OptimizedConfig,
) -> Vec<DependencyConflict> {
    let mut conflicts = Vec::new();

    for (dependency_name, pinned_config) in &config.pinned_dependencies {
        let conflicting_packages = dependencies
            .iter()
            .filter(|dep| dep.dependency_name == *dependency_name)
            .filter(|dep| !is_exempt_from_pinned_dependency(dep, config))
            .filter(|dep| !should_ignore_dependency(dep, config))
            .filter(|dep| dep.version != pinned_config.version)
            .map(|dep| DependencyUsage {
                package_name: dep.package_name.clone(),
                version: dep.version.clone(),
                package_path: dep.package_path.clone(),
            })
            .collect::<Vec<_>>();

        if !conflicting_packages.is_empty() {
            conflicts.push(DependencyConflict {
                dependency_name: dependency_name.clone(),
                conflicting_packages,
                conflict_reason: Some(format!("pinned to {}", pinned_config.version)),
            });
        }
    }

    conflicts
}

fn find_version_conflicts(
    all_dependencies: &[DependencyInfo],
    config: &OptimizedConfig,
) -> Vec<DependencyConflict> {
    let dependency_usage_map = build_dependency_usage_map(all_dependencies, config);
    let mut conflicts = Vec::new();

    for (dependency_name, usages) in dependency_usage_map {
        let version_conflict = analyze_version_conflict(dependency_name, usages, config);
        if let Some(conflict) = version_conflict {
            conflicts.push(conflict);
        }
    }

    // Sort conflicts by dependency name for consistent output
    conflicts.sort_by(|a, b| a.dependency_name.cmp(&b.dependency_name));
    conflicts
}

fn build_dependency_usage_map(
    all_dependencies: &[DependencyInfo],
    config: &OptimizedConfig,
) -> HashMap<String, Vec<DependencyUsage>> {
    let mut dependency_map: HashMap<String, Vec<DependencyUsage>> = HashMap::new();

    for dependency in all_dependencies {
        // Skip pinned dependencies - they're handled separately
        if config
            .pinned_dependency_names
            .contains(&dependency.dependency_name)
        {
            continue;
        }

        dependency_map
            .entry(dependency.dependency_name.clone())
            .or_default()
            .push(DependencyUsage {
                package_name: dependency.package_name.clone(),
                version: dependency.version.clone(),
                package_path: dependency.package_path.clone(),
            });
    }

    dependency_map
}

fn analyze_version_conflict(
    dependency_name: String,
    usages: Vec<DependencyUsage>,
    config: &OptimizedConfig,
) -> Option<DependencyConflict> {
    // Check if we have multiple different versions
    let unique_versions: HashSet<&String> = usages.iter().map(|usage| &usage.version).collect();

    if unique_versions.len() <= 1 {
        return None;
    }

    // Filter out ignored packages
    let filtered_usages = filter_ignored_packages(dependency_name.clone(), usages, config);

    if filtered_usages.len() <= 1 {
        return None;
    }

    // Check if we still have multiple versions after filtering
    let unique_filtered_versions: HashSet<&String> =
        filtered_usages.iter().map(|usage| &usage.version).collect();

    if unique_filtered_versions.len() > 1 {
        Some(DependencyConflict {
            dependency_name,
            conflicting_packages: filtered_usages,
            conflict_reason: None,
        })
    } else {
        None
    }
}

fn filter_ignored_packages(
    dependency_name: String,
    usages: Vec<DependencyUsage>,
    config: &OptimizedConfig,
) -> Vec<DependencyUsage> {
    if let Some(exception_set) = config.ignored_exception_sets.get(&dependency_name) {
        usages
            .into_iter()
            .filter(|usage| {
                // Keep packages that ARE in the exceptions list (i.e., should not be ignored)
                exception_set.contains(&usage.package_name)
            })
            .collect()
    } else {
        usages
    }
}

fn print_conflicts(conflicts: &[DependencyConflict], color_config: ColorConfig) {
    // Sort all conflicts alphabetically by dependency name
    let mut sorted_conflicts = conflicts.to_vec();
    sorted_conflicts.sort_by(|a, b| a.dependency_name.cmp(&b.dependency_name));

    for conflict in &sorted_conflicts {
        print_single_conflict(conflict, color_config);
        println!();
    }

    print_conflict_summary(conflicts.len(), color_config);
}

fn print_single_conflict(conflict: &DependencyConflict, color_config: ColorConfig) {
    let formatted_dependency_name = format_dependency_name(&conflict.dependency_name, color_config);

    if let Some(reason) = &conflict.conflict_reason {
        println!("  {} ({})", formatted_dependency_name, reason);
        print_pinned_conflict_packages(&conflict.conflicting_packages, color_config);
    } else {
        println!("  {} (version mismatch)", formatted_dependency_name);
        print_version_conflict_packages(&conflict.conflicting_packages, color_config);
    }
}

fn format_dependency_name(dependency_name: &str, color_config: ColorConfig) -> String {
    if color_config.should_strip_ansi {
        dependency_name.to_string()
    } else {
        use turborepo_ui::BOLD;
        format!("{}", BOLD.apply_to(dependency_name))
    }
}

fn print_pinned_conflict_packages(
    conflicting_packages: &[DependencyUsage],
    color_config: ColorConfig,
) {
    for usage in conflicting_packages {
        let version_display = format_version(&usage.version, color_config);
        let package_display =
            format_package_info(&usage.package_name, &usage.package_path, color_config);
        println!("    {} → {}", version_display, package_display);
    }
}

fn print_version_conflict_packages(
    conflicting_packages: &[DependencyUsage],
    color_config: ColorConfig,
) {
    let version_groups = group_packages_by_version(conflicting_packages);
    let mut sorted_versions: Vec<_> = version_groups.into_iter().collect();
    sorted_versions.sort_by(|a, b| a.0.cmp(&b.0));

    for (version, packages) in sorted_versions {
        let version_display = format_version(&version, color_config);
        println!("    {} →", version_display);

        for (package_name, package_path) in packages {
            let package_display = format_package_info(&package_name, &package_path, color_config);
            println!("      {}", package_display);
        }
    }
}

fn group_packages_by_version(
    conflicting_packages: &[DependencyUsage],
) -> HashMap<String, Vec<(String, String)>> {
    let mut version_groups: HashMap<String, Vec<(String, String)>> = HashMap::new();

    for usage in conflicting_packages {
        version_groups
            .entry(usage.version.clone())
            .or_default()
            .push((usage.package_name.clone(), usage.package_path.clone()));
    }

    version_groups
}

fn format_version(version: &str, color_config: ColorConfig) -> String {
    if color_config.should_strip_ansi {
        version.to_string()
    } else {
        use turborepo_ui::YELLOW;
        format!("{}", YELLOW.apply_to(version))
    }
}

fn format_package_info(
    package_name: &str,
    package_path: &str,
    color_config: ColorConfig,
) -> String {
    if color_config.should_strip_ansi {
        format!("{} ({})", package_name, package_path)
    } else {
        use turborepo_ui::CYAN;
        format!("{} ({})", CYAN.apply_to(package_name), package_path)
    }
}

fn print_conflict_summary(conflict_count: usize, color_config: ColorConfig) {
    let error_prefix = if color_config.should_strip_ansi {
        ERROR_PREFIX
    } else {
        use turborepo_ui::BOLD_RED;
        &format!("{}", BOLD_RED.apply_to(ERROR_PREFIX))
    };

    println!(
        "\n{} Found {} dependency conflicts.",
        error_prefix, conflict_count
    );
}

fn generate_allowlist_config(
    conflicts: &[DependencyConflict],
    current_config: &DepsSyncConfig,
) -> DepsSyncConfig {
    let mut new_config = DepsSyncConfig {
        pinned_dependencies: HashMap::new(),
        ignored_dependencies: HashMap::new(),
        include_optional_dependencies: current_config.include_optional_dependencies,
    };

    // Only copy existing pinned dependencies that are being modified
    for conflict in conflicts {
        if conflict.conflict_reason.is_some() {
            // This is a pinned dependency conflict
            // Copy the existing pinned dependency and add exceptions
            if let Some(existing_pinned_dep) = current_config
                .pinned_dependencies
                .get(&conflict.dependency_name)
            {
                let mut pinned_dep = existing_pinned_dep.clone();
                for usage in &conflict.conflicting_packages {
                    if !pinned_dep.exceptions.contains(&usage.package_name) {
                        pinned_dep.exceptions.push(usage.package_name.clone());
                    }
                }
                new_config
                    .pinned_dependencies
                    .insert(conflict.dependency_name.clone(), pinned_dep);
            }
        } else {
            // This is a regular version conflict
            // Add the dependency to ignored_dependencies with all conflicting packages as
            // exceptions
            let package_names: Vec<String> = conflict
                .conflicting_packages
                .iter()
                .map(|usage| usage.package_name.clone())
                .collect();

            new_config.ignored_dependencies.insert(
                conflict.dependency_name.clone(),
                IgnoredDependency {
                    exceptions: package_names,
                },
            );
        }
    }

    // Also copy any existing ignored dependencies
    for (dep_name, ignored_dep) in &current_config.ignored_dependencies {
        if !new_config.ignored_dependencies.contains_key(dep_name) {
            new_config
                .ignored_dependencies
                .insert(dep_name.clone(), ignored_dep.clone());
        }
    }

    // Copy any existing pinned dependencies that weren't modified
    for (dep_name, pinned_dep) in &current_config.pinned_dependencies {
        if !new_config.pinned_dependencies.contains_key(dep_name) {
            new_config
                .pinned_dependencies
                .insert(dep_name.clone(), pinned_dep.clone());
        }
    }

    new_config
}

async fn write_allowlist_config(
    repo_root: &AbsoluteSystemPath,
    config: &DepsSyncConfig,
) -> Result<(), Error> {
    let config_opts = crate::config::ConfigurationOptions::default();
    let turbo_json_path = config_opts
        .root_turbo_json_path(repo_root)
        .map_err(Error::Config)?;

    // Read the current turbo.json file
    let mut raw_turbo_json: crate::turbo_json::RawTurboJson =
        (RawTurboJson::read(repo_root, &turbo_json_path)?).unwrap_or_default();

    // Update the deps_sync configuration
    raw_turbo_json.deps_sync = Some(config.clone());

    // Write the updated configuration back to the file
    let json_content =
        serde_json::to_string_pretty(&raw_turbo_json).map_err(|e| Error::JsonParse {
            path: turbo_json_path.to_string(),
            source: e,
        })?;
    std::fs::write(&turbo_json_path, json_content).map_err(|e| Error::FileRead {
        path: turbo_json_path.to_string(),
        source: e,
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    // Helper function to create test dependency info
    fn create_dependency_info(
        package_name: &str,
        package_path: &str,
        dependency_name: &str,
        version: &str,
        dependency_type: DependencyType,
    ) -> DependencyInfo {
        DependencyInfo {
            package_name: package_name.to_string(),
            package_path: package_path.to_string(),
            dependency_name: dependency_name.to_string(),
            version: version.to_string(),
            dependency_type,
        }
    }

    // Helper function to create optimized config
    fn create_optimized_config(config: DepsSyncConfig) -> OptimizedConfig {
        OptimizedConfig::from(config)
    }

    #[test]
    fn test_deps_sync_config_default() {
        let config = DepsSyncConfig::default();
        assert!(config.pinned_dependencies.is_empty());
        assert!(config.ignored_dependencies.is_empty());
        assert!(!config.include_optional_dependencies);
    }

    #[test]
    fn test_optimized_config_conversion() {
        let mut config = DepsSyncConfig::default();
        config.pinned_dependencies.insert(
            "react".to_string(),
            PinnedDependency {
                version: "18.0.0".to_string(),
                exceptions: vec!["package-a".to_string()],
            },
        );
        config.ignored_dependencies.insert(
            "lodash".to_string(),
            IgnoredDependency {
                exceptions: vec!["package-b".to_string()],
            },
        );

        let optimized = OptimizedConfig::from(config.clone());
        assert!(optimized.pinned_dependency_names.contains("react"));
        assert_eq!(
            optimized.pinned_exception_sets.get("react").unwrap(),
            &["package-a".to_string()].into_iter().collect()
        );
        assert_eq!(
            optimized.ignored_exception_sets.get("lodash").unwrap(),
            &["package-b".to_string()].into_iter().collect()
        );

        let back_to_config = DepsSyncConfig::from(optimized);
        assert_eq!(
            config.pinned_dependencies,
            back_to_config.pinned_dependencies
        );
        assert_eq!(
            config.ignored_dependencies,
            back_to_config.ignored_dependencies
        );
    }

    #[test]
    fn test_extract_dependencies_from_json_basic() {
        let json = json!({
            "dependencies": {
                "react": "18.0.0",
                "lodash": "4.17.21"
            },
            "devDependencies": {
                "typescript": "5.0.0"
            }
        });

        let config = create_optimized_config(DepsSyncConfig::default());
        let deps = extract_dependencies_from_json(&json, "test-package", "./test", &config);

        assert_eq!(deps.len(), 3);

        let react_dep = deps.iter().find(|d| d.dependency_name == "react").unwrap();
        assert_eq!(react_dep.version, "18.0.0");
        assert_eq!(react_dep.dependency_type, DependencyType::Dependencies);

        let typescript_dep = deps
            .iter()
            .find(|d| d.dependency_name == "typescript")
            .unwrap();
        assert_eq!(
            typescript_dep.dependency_type,
            DependencyType::DevDependencies
        );
    }

    #[test]
    fn test_extract_dependencies_with_optional_dependencies() {
        let json = json!({
            "dependencies": {
                "react": "18.0.0"
            },
            "optionalDependencies": {
                "fsevents": "2.3.2"
            }
        });

        // Test without including optional dependencies
        let config_without_optional = create_optimized_config(DepsSyncConfig::default());
        let deps_without = extract_dependencies_from_json(
            &json,
            "test-package",
            "./test",
            &config_without_optional,
        );
        assert_eq!(deps_without.len(), 1);
        assert!(deps_without.iter().all(|d| d.dependency_name != "fsevents"));

        // Test with including optional dependencies
        let mut config_with_optional = DepsSyncConfig::default();
        config_with_optional.include_optional_dependencies = true;
        let config_with_optional = create_optimized_config(config_with_optional);
        let deps_with =
            extract_dependencies_from_json(&json, "test-package", "./test", &config_with_optional);
        assert_eq!(deps_with.len(), 2);

        let fsevents_dep = deps_with
            .iter()
            .find(|d| d.dependency_name == "fsevents")
            .unwrap();
        assert_eq!(
            fsevents_dep.dependency_type,
            DependencyType::OptionalDependencies
        );
    }

    #[test]
    fn test_should_ignore_dependency() {
        let mut config = DepsSyncConfig::default();
        config.ignored_dependencies.insert(
            "test-lib".to_string(),
            IgnoredDependency {
                exceptions: vec!["package-a".to_string()],
            },
        );
        let optimized_config = create_optimized_config(config);

        let dep_in_exception = create_dependency_info(
            "package-a",
            "./package-a",
            "test-lib",
            "1.0.0",
            DependencyType::Dependencies,
        );
        let dep_not_in_exception = create_dependency_info(
            "package-b",
            "./package-b",
            "test-lib",
            "1.0.0",
            DependencyType::Dependencies,
        );
        let dep_not_ignored = create_dependency_info(
            "package-c",
            "./package-c",
            "other-lib",
            "1.0.0",
            DependencyType::Dependencies,
        );

        // Package in exception should NOT be ignored
        assert!(!should_ignore_dependency(
            &dep_in_exception,
            &optimized_config
        ));

        // Package not in exception should be ignored
        assert!(should_ignore_dependency(
            &dep_not_in_exception,
            &optimized_config
        ));

        // Dependency not in ignored list should not be ignored
        assert!(!should_ignore_dependency(
            &dep_not_ignored,
            &optimized_config
        ));
    }

    #[test]
    fn test_is_exempt_from_pinned_dependency() {
        let mut config = DepsSyncConfig::default();
        config.pinned_dependencies.insert(
            "react".to_string(),
            PinnedDependency {
                version: "18.0.0".to_string(),
                exceptions: vec!["legacy-package".to_string()],
            },
        );
        let optimized_config = create_optimized_config(config);

        let exempt_dep = create_dependency_info(
            "legacy-package",
            "./legacy",
            "react",
            "17.0.0",
            DependencyType::Dependencies,
        );
        let non_exempt_dep = create_dependency_info(
            "modern-package",
            "./modern",
            "react",
            "17.0.0",
            DependencyType::Dependencies,
        );

        assert!(is_exempt_from_pinned_dependency(
            &exempt_dep,
            &optimized_config
        ));
        assert!(!is_exempt_from_pinned_dependency(
            &non_exempt_dep,
            &optimized_config
        ));
    }

    #[test]
    fn test_find_version_conflicts_simple() {
        let dependencies = vec![
            create_dependency_info(
                "pkg-a",
                "./pkg-a",
                "lodash",
                "4.17.20",
                DependencyType::Dependencies,
            ),
            create_dependency_info(
                "pkg-b",
                "./pkg-b",
                "lodash",
                "4.17.21",
                DependencyType::Dependencies,
            ),
            create_dependency_info(
                "pkg-c",
                "./pkg-c",
                "react",
                "18.0.0",
                DependencyType::Dependencies,
            ),
            create_dependency_info(
                "pkg-d",
                "./pkg-d",
                "react",
                "18.0.0",
                DependencyType::Dependencies,
            ),
        ];

        let config = create_optimized_config(DepsSyncConfig::default());
        let conflicts = find_version_conflicts(&dependencies, &config);

        assert_eq!(conflicts.len(), 1);
        let lodash_conflict = &conflicts[0];
        assert_eq!(lodash_conflict.dependency_name, "lodash");
        assert_eq!(lodash_conflict.conflicting_packages.len(), 2);
        assert!(lodash_conflict.conflict_reason.is_none());
    }

    #[test]
    fn test_find_version_conflicts_with_ignored_dependencies() {
        let dependencies = vec![
            create_dependency_info(
                "pkg-a",
                "./pkg-a",
                "lodash",
                "4.17.20",
                DependencyType::Dependencies,
            ),
            create_dependency_info(
                "pkg-b",
                "./pkg-b",
                "lodash",
                "4.17.21",
                DependencyType::Dependencies,
            ),
            create_dependency_info(
                "pkg-c",
                "./pkg-c",
                "lodash",
                "4.17.22",
                DependencyType::Dependencies,
            ),
        ];

        let mut config = DepsSyncConfig::default();
        config.ignored_dependencies.insert(
            "lodash".to_string(),
            IgnoredDependency {
                exceptions: vec!["pkg-a".to_string(), "pkg-b".to_string()],
            },
        );
        let optimized_config = create_optimized_config(config);

        let conflicts = find_version_conflicts(&dependencies, &optimized_config);

        // Should find conflict between pkg-a and pkg-b (they are exceptions so not
        // ignored) pkg-c is ignored completely, so no conflict involving it
        assert_eq!(conflicts.len(), 1);
        let lodash_conflict = &conflicts[0];
        assert_eq!(lodash_conflict.conflicting_packages.len(), 2);
        assert!(lodash_conflict
            .conflicting_packages
            .iter()
            .any(|p| p.package_name == "pkg-a"));
        assert!(lodash_conflict
            .conflicting_packages
            .iter()
            .any(|p| p.package_name == "pkg-b"));
        assert!(!lodash_conflict
            .conflicting_packages
            .iter()
            .any(|p| p.package_name == "pkg-c"));
    }

    #[test]
    fn test_find_pinned_version_conflicts() {
        let dependencies = vec![
            create_dependency_info(
                "pkg-a",
                "./pkg-a",
                "react",
                "17.0.0",
                DependencyType::Dependencies,
            ),
            create_dependency_info(
                "pkg-b",
                "./pkg-b",
                "react",
                "18.0.0",
                DependencyType::Dependencies,
            ),
            create_dependency_info(
                "pkg-c",
                "./pkg-c",
                "react",
                "16.0.0",
                DependencyType::Dependencies,
            ),
        ];

        let mut config = DepsSyncConfig::default();
        config.pinned_dependencies.insert(
            "react".to_string(),
            PinnedDependency {
                version: "18.0.0".to_string(),
                exceptions: vec![],
            },
        );
        let optimized_config = create_optimized_config(config);

        let conflicts = find_pinned_version_conflicts(&dependencies, &optimized_config);

        assert_eq!(conflicts.len(), 1);
        let react_conflict = &conflicts[0];
        assert_eq!(react_conflict.dependency_name, "react");
        assert_eq!(react_conflict.conflicting_packages.len(), 2);

        // Should include pkg-a and pkg-c (wrong versions), but not pkg-b (correct
        // version)
        assert!(react_conflict
            .conflicting_packages
            .iter()
            .any(|p| p.package_name == "pkg-a"));
        assert!(react_conflict
            .conflicting_packages
            .iter()
            .any(|p| p.package_name == "pkg-c"));
        assert!(!react_conflict
            .conflicting_packages
            .iter()
            .any(|p| p.package_name == "pkg-b"));

        assert!(react_conflict.conflict_reason.is_some());
        assert!(react_conflict
            .conflict_reason
            .as_ref()
            .unwrap()
            .contains("pinned to 18.0.0"));
    }

    #[test]
    fn test_find_pinned_version_conflicts_with_exceptions() {
        let dependencies = vec![
            create_dependency_info(
                "pkg-a",
                "./pkg-a",
                "react",
                "17.0.0",
                DependencyType::Dependencies,
            ),
            create_dependency_info(
                "pkg-b",
                "./pkg-b",
                "react",
                "18.0.0",
                DependencyType::Dependencies,
            ),
            create_dependency_info(
                "legacy-pkg",
                "./legacy",
                "react",
                "16.0.0",
                DependencyType::Dependencies,
            ),
        ];

        let mut config = DepsSyncConfig::default();
        config.pinned_dependencies.insert(
            "react".to_string(),
            PinnedDependency {
                version: "18.0.0".to_string(),
                exceptions: vec!["legacy-pkg".to_string()],
            },
        );
        let optimized_config = create_optimized_config(config);

        let conflicts = find_pinned_version_conflicts(&dependencies, &optimized_config);

        assert_eq!(conflicts.len(), 1);
        let react_conflict = &conflicts[0];

        // Should only include pkg-a (legacy-pkg is exempt)
        assert_eq!(react_conflict.conflicting_packages.len(), 1);
        assert_eq!(react_conflict.conflicting_packages[0].package_name, "pkg-a");
    }

    #[test]
    fn test_generate_allowlist_config_for_version_conflicts() {
        let conflicts = vec![DependencyConflict {
            dependency_name: "lodash".to_string(),
            conflicting_packages: vec![
                DependencyUsage {
                    package_name: "pkg-a".to_string(),
                    version: "4.17.20".to_string(),
                    package_path: "./pkg-a".to_string(),
                },
                DependencyUsage {
                    package_name: "pkg-b".to_string(),
                    version: "4.17.21".to_string(),
                    package_path: "./pkg-b".to_string(),
                },
            ],
            conflict_reason: None,
        }];

        let current_config = DepsSyncConfig::default();
        let allowlist_config = generate_allowlist_config(&conflicts, &current_config);

        assert_eq!(allowlist_config.ignored_dependencies.len(), 1);
        let lodash_ignored = allowlist_config.ignored_dependencies.get("lodash").unwrap();
        assert_eq!(lodash_ignored.exceptions.len(), 2);
        assert!(lodash_ignored.exceptions.contains(&"pkg-a".to_string()));
        assert!(lodash_ignored.exceptions.contains(&"pkg-b".to_string()));
    }

    #[test]
    fn test_generate_allowlist_config_for_pinned_conflicts() {
        let conflicts = vec![DependencyConflict {
            dependency_name: "react".to_string(),
            conflicting_packages: vec![DependencyUsage {
                package_name: "pkg-a".to_string(),
                version: "17.0.0".to_string(),
                package_path: "./pkg-a".to_string(),
            }],
            conflict_reason: Some("pinned to 18.0.0".to_string()),
        }];

        let mut current_config = DepsSyncConfig::default();
        current_config.pinned_dependencies.insert(
            "react".to_string(),
            PinnedDependency {
                version: "18.0.0".to_string(),
                exceptions: vec![],
            },
        );

        let allowlist_config = generate_allowlist_config(&conflicts, &current_config);

        assert_eq!(allowlist_config.pinned_dependencies.len(), 1);
        let react_pinned = allowlist_config.pinned_dependencies.get("react").unwrap();
        assert_eq!(react_pinned.version, "18.0.0");
        assert_eq!(react_pinned.exceptions.len(), 1);
        assert!(react_pinned.exceptions.contains(&"pkg-a".to_string()));
    }

    #[test]
    fn test_generate_allowlist_config_preserves_existing_config() {
        let conflicts = vec![];

        let mut current_config = DepsSyncConfig::default();
        current_config.pinned_dependencies.insert(
            "vue".to_string(),
            PinnedDependency {
                version: "3.0.0".to_string(),
                exceptions: vec!["legacy-vue-pkg".to_string()],
            },
        );
        current_config.ignored_dependencies.insert(
            "moment".to_string(),
            IgnoredDependency {
                exceptions: vec!["time-pkg".to_string()],
            },
        );

        let allowlist_config = generate_allowlist_config(&conflicts, &current_config);

        // Should preserve existing configuration even with no conflicts
        assert_eq!(allowlist_config.pinned_dependencies.len(), 1);
        assert_eq!(allowlist_config.ignored_dependencies.len(), 1);
        assert_eq!(
            allowlist_config
                .pinned_dependencies
                .get("vue")
                .unwrap()
                .version,
            "3.0.0"
        );
        assert_eq!(
            allowlist_config
                .ignored_dependencies
                .get("moment")
                .unwrap()
                .exceptions,
            vec!["time-pkg".to_string()]
        );
    }

    #[test]
    fn test_extract_package_name_fallback() {
        use turbopath::AbsoluteSystemPath;
        use turborepo_errors::Spanned;

        // Test with package.json without name
        let package_json = PackageJson {
            name: None,
            ..Default::default()
        };

        let path = AbsoluteSystemPath::new("/test/my-package/package.json").unwrap();
        let name = extract_package_name(&package_json, path);
        assert_eq!(name, "my-package");

        // Test with package.json with name
        let package_json = PackageJson {
            name: Some(Spanned::new("@scope/actual-name".to_string())),
            ..Default::default()
        };

        let name = extract_package_name(&package_json, path);
        assert_eq!(name, "@scope/actual-name");
    }

    #[test]
    fn test_group_packages_by_version() {
        let packages = vec![
            DependencyUsage {
                package_name: "pkg-a".to_string(),
                version: "1.0.0".to_string(),
                package_path: "./pkg-a".to_string(),
            },
            DependencyUsage {
                package_name: "pkg-b".to_string(),
                version: "1.0.0".to_string(),
                package_path: "./pkg-b".to_string(),
            },
            DependencyUsage {
                package_name: "pkg-c".to_string(),
                version: "2.0.0".to_string(),
                package_path: "./pkg-c".to_string(),
            },
        ];

        let groups = group_packages_by_version(&packages);

        assert_eq!(groups.len(), 2);
        assert_eq!(groups.get("1.0.0").unwrap().len(), 2);
        assert_eq!(groups.get("2.0.0").unwrap().len(), 1);

        let v1_packages = groups.get("1.0.0").unwrap();
        assert!(v1_packages.contains(&("pkg-a".to_string(), "./pkg-a".to_string())));
        assert!(v1_packages.contains(&("pkg-b".to_string(), "./pkg-b".to_string())));
    }

    #[test]
    fn test_dependency_type_display() {
        assert_eq!(DependencyType::Dependencies.to_string(), "dependencies");
        assert_eq!(
            DependencyType::DevDependencies.to_string(),
            "devDependencies"
        );
        assert_eq!(
            DependencyType::OptionalDependencies.to_string(),
            "optionalDependencies"
        );
    }

    #[test]
    fn test_build_dependency_usage_map_excludes_pinned() {
        let dependencies = vec![
            create_dependency_info(
                "pkg-a",
                "./pkg-a",
                "react",
                "18.0.0",
                DependencyType::Dependencies,
            ),
            create_dependency_info(
                "pkg-b",
                "./pkg-b",
                "lodash",
                "4.17.21",
                DependencyType::Dependencies,
            ),
        ];

        let mut config = DepsSyncConfig::default();
        config.pinned_dependencies.insert(
            "react".to_string(),
            PinnedDependency {
                version: "18.0.0".to_string(),
                exceptions: vec![],
            },
        );
        let optimized_config = create_optimized_config(config);

        let usage_map = build_dependency_usage_map(&dependencies, &optimized_config);

        // Should only include lodash, not react (since react is pinned)
        assert_eq!(usage_map.len(), 1);
        assert!(usage_map.contains_key("lodash"));
        assert!(!usage_map.contains_key("react"));
    }

    #[test]
    fn test_filter_ignored_packages() {
        let usages = vec![
            DependencyUsage {
                package_name: "pkg-a".to_string(),
                version: "1.0.0".to_string(),
                package_path: "./pkg-a".to_string(),
            },
            DependencyUsage {
                package_name: "pkg-b".to_string(),
                version: "1.0.0".to_string(),
                package_path: "./pkg-b".to_string(),
            },
            DependencyUsage {
                package_name: "pkg-c".to_string(),
                version: "1.0.0".to_string(),
                package_path: "./pkg-c".to_string(),
            },
        ];

        let mut config = DepsSyncConfig::default();
        config.ignored_dependencies.insert(
            "test-dep".to_string(),
            IgnoredDependency {
                exceptions: vec!["pkg-a".to_string(), "pkg-b".to_string()],
            },
        );
        let optimized_config = create_optimized_config(config);

        let filtered = filter_ignored_packages("test-dep".to_string(), usages, &optimized_config);

        // Should keep packages that are in the exception set (should not be ignored)
        // pkg-a and pkg-b are in the exception set, so they should be kept
        // pkg-c is not in the exception set, so it should be filtered out
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().any(|u| u.package_name == "pkg-a"));
        assert!(filtered.iter().any(|u| u.package_name == "pkg-b"));
        assert!(!filtered.iter().any(|u| u.package_name == "pkg-c"));
    }
}
