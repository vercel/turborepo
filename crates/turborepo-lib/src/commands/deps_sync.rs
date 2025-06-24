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

#[derive(Debug, Clone, Deserializable, serde::Deserialize, serde::Serialize)]
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
    /// Packages where this dependency should be ignored
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exceptions: Vec<String>,
}

impl Default for DepsSyncConfig {
    fn default() -> Self {
        Self {
            pinned_dependencies: HashMap::new(),
            ignored_dependencies: HashMap::new(),
            include_optional_dependencies: false,
        }
    }
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
        .map_err(|e| Error::Config(e))?;

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
                // Keep packages that are NOT in the exceptions list (i.e., not ignored)
                !exception_set.contains(&usage.package_name)
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
        .map_err(|e| Error::Config(e))?;

    // Read the current turbo.json file
    let mut raw_turbo_json = match RawTurboJson::read(repo_root, &turbo_json_path)? {
        Some(turbo_json) => turbo_json,
        None => RawTurboJson::default(),
    };

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

#[test]
fn test_ignored_dependencies_with_exceptions() {
    let dependencies = vec![
        DependencyInfo {
            package_name: "app1".to_string(),
            package_path: "packages/app1".to_string(),
            dependency_name: "lodash".to_string(),
            version: "4.17.0".to_string(),
            dependency_type: DependencyType::Dependencies,
        },
        DependencyInfo {
            package_name: "app2".to_string(),
            package_path: "packages/app2".to_string(),
            dependency_name: "lodash".to_string(),
            version: "4.18.0".to_string(),
            dependency_type: DependencyType::Dependencies,
        },
    ];

    let deps_sync_config = DepsSyncConfig {
        pinned_dependencies: HashMap::new(),
        ignored_dependencies: HashMap::from([(
            "lodash".to_string(),
            IgnoredDependency {
                exceptions: vec!["app1".to_string()],
            },
        )]),
        include_optional_dependencies: false,
    };
    let config = OptimizedConfig::from(deps_sync_config);

    let conflicts = find_version_conflicts(&dependencies, &config);

    // Should have conflict for lodash since app1 is not ignored (it's in
    // exceptions)
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].dependency_name, "lodash");
    assert_eq!(conflicts[0].conflicting_packages.len(), 2);

    // Both packages should be in the conflict (app1 is not ignored, app2 is ignored
    // but still shown)
    let package_names: Vec<_> = conflicts[0]
        .conflicting_packages
        .iter()
        .map(|p| &p.package_name)
        .collect();
    assert!(package_names.contains(&&"app1".to_string()));
    assert!(package_names.contains(&&"app2".to_string()));
}

#[test]
fn test_pinned_and_ignored_dependencies_no_conflicts() {
    // Test that dependencies that are both pinned and ignored work correctly with
    // exceptions
    let dependencies = vec![
        DependencyInfo {
            package_name: "app1".to_string(),
            package_path: "packages/app1".to_string(),
            dependency_name: "react".to_string(),
            version: "17.0.0".to_string(), // Wrong version but app1 is in exceptions (NOT ignored)
            dependency_type: DependencyType::Dependencies,
        },
        DependencyInfo {
            package_name: "app2".to_string(),
            package_path: "packages/app2".to_string(),
            dependency_name: "react".to_string(),
            version: "16.0.0".to_string(), // Wrong version but app2 is ignored
            dependency_type: DependencyType::Dependencies,
        },
        DependencyInfo {
            package_name: "app3".to_string(),
            package_path: "packages/app3".to_string(),
            dependency_name: "react".to_string(),
            version: "18.0.0".to_string(), // Correct version - no conflict
            dependency_type: DependencyType::Dependencies,
        },
    ];

    let mut pinned_dependencies = HashMap::new();
    pinned_dependencies.insert(
        "react".to_string(),
        PinnedDependency {
            version: "18.0.0".to_string(),
            exceptions: vec![],
        },
    );

    let deps_sync_config = DepsSyncConfig {
        pinned_dependencies,
        ignored_dependencies: HashMap::from([(
            "react".to_string(),
            IgnoredDependency {
                exceptions: vec!["app1".to_string()],
            },
        )]),
        include_optional_dependencies: false,
    };
    let config = OptimizedConfig::from(deps_sync_config);

    let pinned_conflicts = find_pinned_version_conflicts(&dependencies, &config);

    // Should only have conflict for app1 (app1 is in exceptions so NOT ignored,
    // app2 is ignored, app3 has correct version)
    assert_eq!(pinned_conflicts.len(), 1);
    let conflict = &pinned_conflicts[0];
    assert_eq!(conflict.dependency_name, "react");
    assert_eq!(conflict.conflicting_packages.len(), 1);
    assert_eq!(conflict.conflicting_packages[0].package_name, "app1");
    assert_eq!(conflict.conflicting_packages[0].version, "17.0.0");
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn test_ignored_dependencies_all_packages() {
        let dependencies = vec![
            DependencyInfo {
                package_name: "app1".to_string(),
                package_path: "packages/app1".to_string(),
                dependency_name: "lodash".to_string(),
                version: "4.17.0".to_string(),
                dependency_type: DependencyType::Dependencies,
            },
            DependencyInfo {
                package_name: "app2".to_string(),
                package_path: "packages/app2".to_string(),
                dependency_name: "lodash".to_string(),
                version: "4.18.0".to_string(),
                dependency_type: DependencyType::Dependencies,
            },
            DependencyInfo {
                package_name: "app3".to_string(),
                package_path: "packages/app3".to_string(),
                dependency_name: "react".to_string(),
                version: "18.0.0".to_string(),
                dependency_type: DependencyType::Dependencies,
            },
        ];

        let deps_sync_config = DepsSyncConfig {
            pinned_dependencies: HashMap::new(),
            ignored_dependencies: HashMap::from([(
                "lodash".to_string(),
                IgnoredDependency {
                    exceptions: vec![], // No exceptions - ignore lodash in ALL packages
                },
            )]),
            include_optional_dependencies: false,
        };
        let config = OptimizedConfig::from(deps_sync_config);

        let conflicts = find_version_conflicts(&dependencies, &config);

        // Should have no conflicts since lodash is ignored in all packages (no
        // exceptions)
        assert_eq!(conflicts.len(), 0);
    }

    #[test]
    fn test_ignored_dependencies_with_specific_exceptions() {
        let dependencies = vec![
            DependencyInfo {
                package_name: "app1".to_string(),
                package_path: "packages/app1".to_string(),
                dependency_name: "lodash".to_string(),
                version: "4.17.0".to_string(),
                dependency_type: DependencyType::Dependencies,
            },
            DependencyInfo {
                package_name: "app2".to_string(),
                package_path: "packages/app2".to_string(),
                dependency_name: "lodash".to_string(),
                version: "4.18.0".to_string(),
                dependency_type: DependencyType::Dependencies,
            },
            DependencyInfo {
                package_name: "app3".to_string(),
                package_path: "packages/app3".to_string(),
                dependency_name: "lodash".to_string(),
                version: "4.19.0".to_string(),
                dependency_type: DependencyType::Dependencies,
            },
        ];

        let deps_sync_config = DepsSyncConfig {
            pinned_dependencies: HashMap::new(),
            ignored_dependencies: HashMap::from([(
                "lodash".to_string(),
                IgnoredDependency {
                    exceptions: vec!["app2".to_string(), "app3".to_string()],
                },
            )]),
            include_optional_dependencies: false,
        };
        let config = OptimizedConfig::from(deps_sync_config);

        let conflicts = find_version_conflicts(&dependencies, &config);

        // Should have conflict for lodash since app2 and app3 are not ignored (they're
        // in exceptions)
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].dependency_name, "lodash");
        assert_eq!(conflicts[0].conflicting_packages.len(), 3);

        // All packages should be in the conflict (app2/app3 are not ignored, app1 is
        // ignored but still shown)
        let package_names: Vec<_> = conflicts[0]
            .conflicting_packages
            .iter()
            .map(|p| &p.package_name)
            .collect();
        assert!(package_names.contains(&&"app1".to_string()));
        assert!(package_names.contains(&&"app2".to_string()));
        assert!(package_names.contains(&&"app3".to_string()));
    }

    #[test]
    fn test_pinned_and_ignored_dependencies_no_conflicts() {
        // Test that dependencies that are both pinned and ignored don't show up in
        // diagnostics
        let dependencies = vec![
            DependencyInfo {
                package_name: "app1".to_string(),
                package_path: "packages/app1".to_string(),
                dependency_name: "react".to_string(),
                version: "17.0.0".to_string(), // Wrong version but should be ignored
                dependency_type: DependencyType::Dependencies,
            },
            DependencyInfo {
                package_name: "app2".to_string(),
                package_path: "packages/app2".to_string(),
                dependency_name: "react".to_string(),
                version: "16.0.0".to_string(), // Wrong version and should show conflict
                dependency_type: DependencyType::Dependencies,
            },
            DependencyInfo {
                package_name: "app3".to_string(),
                package_path: "packages/app3".to_string(),
                dependency_name: "react".to_string(),
                version: "18.0.0".to_string(), // Correct version - no conflict
                dependency_type: DependencyType::Dependencies,
            },
        ];

        let mut pinned_dependencies = HashMap::new();
        pinned_dependencies.insert(
            "react".to_string(),
            PinnedDependency {
                version: "18.0.0".to_string(),
                exceptions: vec![],
            },
        );

        let deps_sync_config = DepsSyncConfig {
            pinned_dependencies,
            ignored_dependencies: HashMap::from([(
                "react".to_string(),
                IgnoredDependency {
                    exceptions: vec!["app1".to_string()],
                },
            )]),
            include_optional_dependencies: false,
        };
        let config = OptimizedConfig::from(deps_sync_config);

        let pinned_conflicts = find_pinned_version_conflicts(&dependencies, &config);

        // Should only have conflict for app2 (app1 is ignored, app3 has correct
        // version)
        assert_eq!(pinned_conflicts.len(), 1);
        let conflict = &pinned_conflicts[0];
        assert_eq!(conflict.dependency_name, "react");
        assert_eq!(conflict.conflicting_packages.len(), 1);
        assert_eq!(conflict.conflicting_packages[0].package_name, "app2");
        assert_eq!(conflict.conflicting_packages[0].version, "16.0.0");
    }

    #[test]
    fn test_include_optional_dependencies_config() {
        // Test that optionalDependencies are excluded by default
        let config_exclude_optional = DepsSyncConfig {
            pinned_dependencies: HashMap::new(),
            ignored_dependencies: HashMap::new(),
            include_optional_dependencies: false,
        };
        let optimized_config = OptimizedConfig::from(config_exclude_optional);

        let raw_json = serde_json::json!({
            "dependencies": {
                "react": "^18.0.0"
            },
            "devDependencies": {
                "typescript": "^4.0.0"
            },
            "optionalDependencies": {
                "fsevents": "^2.3.0"
            }
        });

        let dependencies = extract_dependencies_from_json(
            &raw_json,
            "test-package",
            "packages/test",
            &optimized_config,
        );

        // Should only have 2 dependencies (not 3), excluding optionalDependencies
        assert_eq!(dependencies.len(), 2);
        assert!(dependencies.iter().any(|d| d.dependency_name == "react"));
        assert!(dependencies
            .iter()
            .any(|d| d.dependency_name == "typescript"));
        assert!(!dependencies.iter().any(|d| d.dependency_name == "fsevents"));

        // Test that optionalDependencies are included when configured
        let config_include_optional = DepsSyncConfig {
            pinned_dependencies: HashMap::new(),
            ignored_dependencies: HashMap::new(),
            include_optional_dependencies: true,
        };
        let optimized_config_include = OptimizedConfig::from(config_include_optional);

        let dependencies_with_optional = extract_dependencies_from_json(
            &raw_json,
            "test-package",
            "packages/test",
            &optimized_config_include,
        );

        // Should have all 3 dependencies, including optionalDependencies
        assert_eq!(dependencies_with_optional.len(), 3);
        assert!(dependencies_with_optional
            .iter()
            .any(|d| d.dependency_name == "react"));
        assert!(dependencies_with_optional
            .iter()
            .any(|d| d.dependency_name == "typescript"));
        assert!(dependencies_with_optional
            .iter()
            .any(|d| d.dependency_name == "fsevents"));
    }

    #[test]
    fn test_generate_allowlist_config() {
        let conflicts = vec![
            // Regular version conflict
            DependencyConflict {
                dependency_name: "lodash".to_string(),
                conflicting_packages: vec![
                    DependencyUsage {
                        package_name: "app1".to_string(),
                        version: "4.17.0".to_string(),
                        package_path: "packages/app1".to_string(),
                    },
                    DependencyUsage {
                        package_name: "app2".to_string(),
                        version: "4.18.0".to_string(),
                        package_path: "packages/app2".to_string(),
                    },
                ],
                conflict_reason: None,
            },
            // Pinned dependency conflict
            DependencyConflict {
                dependency_name: "react".to_string(),
                conflicting_packages: vec![DependencyUsage {
                    package_name: "app3".to_string(),
                    version: "17.0.0".to_string(),
                    package_path: "packages/app3".to_string(),
                }],
                conflict_reason: Some("pinned to 18.0.0".to_string()),
            },
        ];

        let current_config = DepsSyncConfig {
            pinned_dependencies: HashMap::from([(
                "react".to_string(),
                PinnedDependency {
                    version: "18.0.0".to_string(),
                    exceptions: vec![],
                },
            )]),
            ignored_dependencies: HashMap::new(),
            include_optional_dependencies: false,
        };

        let allowlist_config = generate_allowlist_config(&conflicts, &current_config);

        // lodash should be added to ignored_dependencies with all conflicting packages
        // as exceptions
        assert!(allowlist_config.ignored_dependencies.contains_key("lodash"));
        let lodash_exceptions = &allowlist_config.ignored_dependencies["lodash"].exceptions;
        assert_eq!(lodash_exceptions.len(), 2);
        assert!(lodash_exceptions.contains(&"app1".to_string()));
        assert!(lodash_exceptions.contains(&"app2".to_string()));

        // app3 should be added to react's exceptions
        assert!(allowlist_config.pinned_dependencies.contains_key("react"));
        assert!(allowlist_config.pinned_dependencies["react"]
            .exceptions
            .contains(&"app3".to_string()));
    }
}
