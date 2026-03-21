use std::{collections::HashSet, fmt, path::Path, str::FromStr};

use toml::{Table, Value};

/// Identifier for a language/toolchain workspace provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum WorkspaceProviderId {
    Node,
    Cargo,
    Uv,
}

impl WorkspaceProviderId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::Cargo => "cargo",
            Self::Uv => "uv",
        }
    }
}

impl fmt::Display for WorkspaceProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ParseWorkspaceProviderIdError {
    #[error("Unknown workspace provider '{value}'")]
    Unknown { value: String },
}

impl FromStr for WorkspaceProviderId {
    type Err = ParseWorkspaceProviderIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "node" => Ok(Self::Node),
            "cargo" => Ok(Self::Cargo),
            "uv" => Ok(Self::Uv),
            _ => Err(ParseWorkspaceProviderIdError::Unknown {
                value: value.to_string(),
            }),
        }
    }
}

/// Provider trait for workspace discovery logic.
pub trait WorkspaceDiscoveryProvider {
    fn provider_id(&self) -> WorkspaceProviderId;

    fn is_workspace_manifest(&self, manifest_path: &str) -> bool;
}

/// Provider trait for dependency graph/dependency resolution logic.
pub trait WorkspaceDependencyProvider {
    fn provider_id(&self) -> WorkspaceProviderId;

    fn infer_internal_dependencies(&self, manifest_contents: &str) -> Vec<String>;
}

/// Provider trait for command inference/resolution logic.
pub trait TaskCommandProvider {
    fn provider_id(&self) -> WorkspaceProviderId;

    fn resolve_task_command(&self, task_name: &str) -> Option<String>;
}

/// Provider trait for hash input contributions.
pub trait HashInputProvider {
    fn provider_id(&self) -> WorkspaceProviderId;

    fn hash_inputs(&self) -> Vec<String>;
}

/// Node provider adapter used for backwards-compatible behavior.
#[derive(Debug, Default, Clone, Copy)]
pub struct NodeWorkspaceProvider;

impl WorkspaceDiscoveryProvider for NodeWorkspaceProvider {
    fn provider_id(&self) -> WorkspaceProviderId {
        WorkspaceProviderId::Node
    }

    fn is_workspace_manifest(&self, manifest_path: &str) -> bool {
        Path::new(manifest_path)
            .file_name()
            .and_then(|file_name| file_name.to_str())
            .is_some_and(|file_name| file_name.eq_ignore_ascii_case("package.json"))
    }
}

impl WorkspaceDependencyProvider for NodeWorkspaceProvider {
    fn provider_id(&self) -> WorkspaceProviderId {
        WorkspaceProviderId::Node
    }

    fn infer_internal_dependencies(&self, _manifest_contents: &str) -> Vec<String> {
        Vec::new()
    }
}

impl TaskCommandProvider for NodeWorkspaceProvider {
    fn provider_id(&self) -> WorkspaceProviderId {
        WorkspaceProviderId::Node
    }

    fn resolve_task_command(&self, _task_name: &str) -> Option<String> {
        None
    }
}

impl HashInputProvider for NodeWorkspaceProvider {
    fn provider_id(&self) -> WorkspaceProviderId {
        WorkspaceProviderId::Node
    }

    fn hash_inputs(&self) -> Vec<String> {
        vec!["package.json".to_string()]
    }
}

/// Cargo provider adapter for Rust workspaces.
#[derive(Debug, Default, Clone, Copy)]
pub struct CargoWorkspaceProvider;

impl WorkspaceDiscoveryProvider for CargoWorkspaceProvider {
    fn provider_id(&self) -> WorkspaceProviderId {
        WorkspaceProviderId::Cargo
    }

    fn is_workspace_manifest(&self, manifest_path: &str) -> bool {
        Path::new(manifest_path)
            .file_name()
            .and_then(|file_name| file_name.to_str())
            .is_some_and(|file_name| file_name.eq_ignore_ascii_case("cargo.toml"))
    }
}

impl WorkspaceDependencyProvider for CargoWorkspaceProvider {
    fn provider_id(&self) -> WorkspaceProviderId {
        WorkspaceProviderId::Cargo
    }

    fn infer_internal_dependencies(&self, manifest_contents: &str) -> Vec<String> {
        let Ok(manifest) = toml::from_str::<Table>(manifest_contents) else {
            return Vec::new();
        };

        let mut dependencies = HashSet::new();
        collect_dependency_names_for_table(&manifest, &mut dependencies);

        if let Some(target) = manifest.get("target").and_then(Value::as_table) {
            for target_table in target.values().filter_map(Value::as_table) {
                collect_dependency_names_for_table(target_table, &mut dependencies);
            }
        }

        let mut dependencies: Vec<_> = dependencies.into_iter().collect();
        dependencies.sort();
        dependencies
    }
}

impl TaskCommandProvider for CargoWorkspaceProvider {
    fn provider_id(&self) -> WorkspaceProviderId {
        WorkspaceProviderId::Cargo
    }

    fn resolve_task_command(&self, task_name: &str) -> Option<String> {
        match task_name {
            "build" => Some("cargo build".to_string()),
            "check" => Some("cargo check".to_string()),
            "test" => Some("cargo test".to_string()),
            "lint" => Some("cargo clippy".to_string()),
            "fmt" | "format" => Some("cargo fmt".to_string()),
            "doc" => Some("cargo doc".to_string()),
            _ => None,
        }
    }
}

impl HashInputProvider for CargoWorkspaceProvider {
    fn provider_id(&self) -> WorkspaceProviderId {
        WorkspaceProviderId::Cargo
    }

    fn hash_inputs(&self) -> Vec<String> {
        vec!["Cargo.toml".to_string(), "Cargo.lock".to_string()]
    }
}

/// uv provider adapter for Python workspaces.
#[derive(Debug, Default, Clone, Copy)]
pub struct UvWorkspaceProvider;

impl WorkspaceDiscoveryProvider for UvWorkspaceProvider {
    fn provider_id(&self) -> WorkspaceProviderId {
        WorkspaceProviderId::Uv
    }

    fn is_workspace_manifest(&self, manifest_path: &str) -> bool {
        Path::new(manifest_path)
            .file_name()
            .and_then(|file_name| file_name.to_str())
            .is_some_and(|file_name| file_name.eq_ignore_ascii_case("pyproject.toml"))
    }
}

impl WorkspaceDependencyProvider for UvWorkspaceProvider {
    fn provider_id(&self) -> WorkspaceProviderId {
        WorkspaceProviderId::Uv
    }

    fn infer_internal_dependencies(&self, manifest_contents: &str) -> Vec<String> {
        let Ok(manifest) = toml::from_str::<Table>(manifest_contents) else {
            return Vec::new();
        };

        let sources = manifest
            .get("tool")
            .and_then(Value::as_table)
            .and_then(|tool| tool.get("uv"))
            .and_then(Value::as_table)
            .and_then(|uv| uv.get("sources"))
            .and_then(Value::as_table);

        let mut dependencies = sources
            .into_iter()
            .flat_map(|sources| {
                sources.iter().filter_map(|(name, source)| {
                    let source_table = source.as_table()?;
                    let has_path_dependency = source_table
                        .get("path")
                        .and_then(Value::as_str)
                        .is_some_and(|path| !path.is_empty());
                    let has_workspace_dependency = source_table
                        .get("workspace")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    (has_path_dependency || has_workspace_dependency).then_some(name.clone())
                })
            })
            .collect::<Vec<_>>();

        dependencies.sort();
        dependencies
    }
}

impl TaskCommandProvider for UvWorkspaceProvider {
    fn provider_id(&self) -> WorkspaceProviderId {
        WorkspaceProviderId::Uv
    }

    fn resolve_task_command(&self, task_name: &str) -> Option<String> {
        match task_name {
            "build" => Some("uv build".to_string()),
            "test" => Some("uv run pytest".to_string()),
            "lint" => Some("uv run ruff check .".to_string()),
            "fmt" | "format" => Some("uv run ruff format .".to_string()),
            _ => None,
        }
    }
}

impl HashInputProvider for UvWorkspaceProvider {
    fn provider_id(&self) -> WorkspaceProviderId {
        WorkspaceProviderId::Uv
    }

    fn hash_inputs(&self) -> Vec<String> {
        vec!["pyproject.toml".to_string(), "uv.lock".to_string()]
    }
}

fn collect_dependency_names_for_table(table: &Table, dependencies: &mut HashSet<String>) {
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(dependency_table) = table.get(section).and_then(Value::as_table) {
            for (name, definition) in dependency_table {
                if is_internal_dependency_definition(definition) {
                    dependencies.insert(name.clone());
                }
            }
        }
    }
}

fn is_internal_dependency_definition(definition: &Value) -> bool {
    let Some(definition_table) = definition.as_table() else {
        return false;
    };

    let has_path_dependency = definition_table
        .get("path")
        .and_then(Value::as_str)
        .is_some_and(|path| !path.is_empty());
    let has_workspace_dependency = definition_table
        .get("workspace")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    has_path_dependency || has_workspace_dependency
}

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceProviderResolveError {
    #[error(transparent)]
    Parse(#[from] ParseWorkspaceProviderIdError),
    #[error(
        "Workspace provider '{requested}' is configured but not available in this build. \
         Available providers: {available}"
    )]
    NotAvailable {
        requested: WorkspaceProviderId,
        available: String,
    },
}

/// Registry of workspace providers available in the current build.
#[derive(Debug, Clone)]
pub struct WorkspaceProviderRegistry {
    available: HashSet<WorkspaceProviderId>,
}

impl Default for WorkspaceProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkspaceProviderRegistry {
    pub fn new() -> Self {
        let mut available = HashSet::new();
        available.insert(WorkspaceProviderId::Node);
        available.insert(WorkspaceProviderId::Cargo);
        available.insert(WorkspaceProviderId::Uv);
        Self { available }
    }

    #[cfg(test)]
    pub fn with_provider(mut self, provider: WorkspaceProviderId) -> Self {
        self.available.insert(provider);
        self
    }

    pub fn available(&self) -> Vec<WorkspaceProviderId> {
        let mut providers: Vec<_> = self.available.iter().copied().collect();
        providers.sort();
        providers
    }

    pub fn resolve(
        &self,
        configured: Option<&[String]>,
    ) -> Result<Vec<WorkspaceProviderId>, WorkspaceProviderResolveError> {
        let configured = configured.filter(|providers| !providers.is_empty());
        let mut resolved: Vec<WorkspaceProviderId> = configured
            .map(|providers| {
                providers
                    .iter()
                    .map(|provider| WorkspaceProviderId::from_str(provider))
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_else(|| vec![WorkspaceProviderId::Node]);

        // Dedup in order of declaration.
        let mut seen = HashSet::new();
        resolved.retain(|provider| seen.insert(*provider));

        let available_str = self
            .available()
            .into_iter()
            .map(|provider| provider.to_string())
            .collect::<Vec<_>>()
            .join(", ");

        for provider in &resolved {
            if !self.available.contains(provider) {
                return Err(WorkspaceProviderResolveError::NotAvailable {
                    requested: *provider,
                    available: available_str,
                });
            }
        }

        Ok(resolved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_registry_resolves_to_node() {
        let resolved = WorkspaceProviderRegistry::new().resolve(None).unwrap();
        assert_eq!(resolved, vec![WorkspaceProviderId::Node]);
    }

    #[test]
    fn configured_ids_are_deduped_in_order() {
        let configured = vec!["node".to_string(), "node".to_string(), "node".to_string()];
        let resolved = WorkspaceProviderRegistry::new()
            .resolve(Some(configured.as_slice()))
            .unwrap();
        assert_eq!(resolved, vec![WorkspaceProviderId::Node]);
    }

    #[test]
    fn unknown_provider_errors() {
        let configured = vec!["does-not-exist".to_string()];
        let err = WorkspaceProviderRegistry::new()
            .resolve(Some(configured.as_slice()))
            .unwrap_err();
        assert!(matches!(
            err,
            WorkspaceProviderResolveError::Parse(ParseWorkspaceProviderIdError::Unknown { .. })
        ));
    }

    #[test]
    fn cargo_provider_is_available_by_default() {
        let configured = vec!["cargo".to_string()];
        let resolved = WorkspaceProviderRegistry::new()
            .resolve(Some(configured.as_slice()))
            .unwrap();
        assert_eq!(resolved, vec![WorkspaceProviderId::Cargo]);
    }

    #[test]
    fn available_provider_resolves_when_registered() {
        let configured = vec!["cargo".to_string(), "node".to_string()];
        let resolved = WorkspaceProviderRegistry::new()
            .with_provider(WorkspaceProviderId::Cargo)
            .resolve(Some(configured.as_slice()))
            .unwrap();
        assert_eq!(
            resolved,
            vec![WorkspaceProviderId::Cargo, WorkspaceProviderId::Node]
        );
    }

    #[test]
    fn cargo_provider_recognizes_manifest_and_dependencies() {
        let provider = CargoWorkspaceProvider;
        assert!(provider.is_workspace_manifest("/repo/crates/app/Cargo.toml"));
        assert!(!provider.is_workspace_manifest("/repo/crates/app/package.json"));

        let manifest = r#"
[package]
name = "app"

[dependencies]
serde = { workspace = true }
utils = { path = "../utils" }
tokio = "1.0"

[target.'cfg(unix)'.dependencies]
shared = { path = "../shared" }
"#;
        assert_eq!(
            provider.infer_internal_dependencies(manifest),
            vec![
                "serde".to_string(),
                "shared".to_string(),
                "utils".to_string(),
            ]
        );
        assert_eq!(
            provider.resolve_task_command("lint"),
            Some("cargo clippy".to_string())
        );
        assert_eq!(
            provider.hash_inputs(),
            vec!["Cargo.toml".to_string(), "Cargo.lock".to_string()]
        );
    }

    #[test]
    fn uv_provider_recognizes_manifest_and_dependencies() {
        let provider = UvWorkspaceProvider;
        assert!(provider.is_workspace_manifest("/repo/apps/api/pyproject.toml"));
        assert!(!provider.is_workspace_manifest("/repo/apps/api/Cargo.toml"));

        let manifest = r#"
[project]
name = "api"

[tool.uv.sources]
shared = { workspace = true }
core = { path = "../core" }
requests = { index = "pypi" }
"#;
        assert_eq!(
            provider.infer_internal_dependencies(manifest),
            vec!["core".to_string(), "shared".to_string()]
        );
        assert_eq!(
            provider.resolve_task_command("test"),
            Some("uv run pytest".to_string())
        );
        assert_eq!(
            provider.hash_inputs(),
            vec!["pyproject.toml".to_string(), "uv.lock".to_string()]
        );
    }
}
