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

    /// Whether tasks of this type should be serialized across packages
    /// to avoid resource contention (e.g., Cargo's `target/` directory lock).
    /// When true, the engine chains these tasks so at most one runs at a time.
    fn requires_serial_execution(&self, _task_name: &str) -> bool {
        false
    }

    /// Resolve a task command that targets a specific package by name.
    /// Used when the provider runs commands from the workspace root
    /// with package-targeting flags (e.g., `cargo build -p <name>`).
    fn resolve_targeted_command(&self, _task_name: &str, _package_name: &str) -> Option<String> {
        None
    }
}

/// Provider trait for injecting environment variables into task execution.
/// This allows workspace providers to configure tool-specific behavior
/// (e.g., setting RUSTC_WRAPPER for Cargo compilation caching).
pub trait TaskEnvironmentProvider {
    fn provider_id(&self) -> WorkspaceProviderId;

    /// Returns environment variables to inject when running a task.
    /// The returned pairs are `(key, value)`. These are merged into
    /// the task's environment before execution.
    fn task_environment(&self, task_name: &str) -> Vec<(String, String)>;
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

impl TaskEnvironmentProvider for NodeWorkspaceProvider {
    fn provider_id(&self) -> WorkspaceProviderId {
        WorkspaceProviderId::Node
    }

    fn task_environment(&self, _task_name: &str) -> Vec<(String, String)> {
        Vec::new()
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

    fn requires_serial_execution(&self, _task_name: &str) -> bool {
        // All Cargo tasks share the `target/` directory and contend on its
        // file lock when run in parallel. Serializing prevents lock contention.
        true
    }

    fn resolve_targeted_command(&self, task_name: &str, package_name: &str) -> Option<String> {
        match task_name {
            "build" => Some(format!("cargo build -p {package_name}")),
            "check" => Some(format!("cargo check -p {package_name}")),
            "test" => Some(format!("cargo test -p {package_name}")),
            "lint" => Some(format!("cargo clippy -p {package_name}")),
            "fmt" | "format" => Some("cargo fmt --all".to_string()),
            "doc" => Some(format!("cargo doc -p {package_name}")),
            _ => None,
        }
    }
}

impl TaskEnvironmentProvider for CargoWorkspaceProvider {
    fn provider_id(&self) -> WorkspaceProviderId {
        WorkspaceProviderId::Cargo
    }

    fn task_environment(&self, task_name: &str) -> Vec<(String, String)> {
        // Only inject for tasks that invoke the Rust compiler
        let uses_rustc = matches!(task_name, "build" | "check" | "test" | "doc");
        if !uses_rustc {
            return Vec::new();
        }

        let mut env = Vec::new();

        // Find the turbo-rustc-cache binary. It ships alongside the turbo binary.
        if let Some(wrapper_path) = find_rustc_cache_binary() {
            env.push(("RUSTC_WRAPPER".to_string(), wrapper_path));
        }

        // In CI, disable incremental compilation for deterministic, cacheable output.
        // Locally, leave it alone so developers keep their fast iteration loop.
        if is_ci_environment() {
            env.push(("CARGO_INCREMENTAL".to_string(), "0".to_string()));
        }

        env
    }
}

fn find_rustc_cache_binary() -> Option<String> {
    // 1. Explicit override
    if let Ok(path) = std::env::var("TURBO_RUSTC_CACHE_BIN") {
        if !path.is_empty() {
            return Some(path);
        }
    }

    // 2. Look next to the current turbo binary
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(dir) = current_exe.parent() {
            let candidate = dir.join("turbo-rustc-cache");
            if candidate.exists() {
                return candidate.to_str().map(|s| s.to_string());
            }
            // Windows
            let candidate = dir.join("turbo-rustc-cache.exe");
            if candidate.exists() {
                return candidate.to_str().map(|s| s.to_string());
            }
        }
    }

    // 3. Check PATH
    which::which("turbo-rustc-cache")
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_string()))
}

fn is_ci_environment() -> bool {
    const CI_VARS: &[&str] = &[
        "CI",
        "BUILD_ID",
        "BUILD_NUMBER",
        "CI_APP_ID",
        "CI_BUILD_ID",
        "CI_BUILD_NUMBER",
        "CI_NAME",
        "CONTINUOUS_INTEGRATION",
        "TEAMCITY_VERSION",
    ];

    CI_VARS
        .iter()
        .any(|var| !std::env::var(var).unwrap_or_default().is_empty())
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

impl TaskEnvironmentProvider for UvWorkspaceProvider {
    fn provider_id(&self) -> WorkspaceProviderId {
        WorkspaceProviderId::Uv
    }

    fn task_environment(&self, _task_name: &str) -> Vec<(String, String)> {
        Vec::new()
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
    fn cargo_provider_requires_serial_execution() {
        let provider = CargoWorkspaceProvider;
        assert!(provider.requires_serial_execution("build"));
        assert!(provider.requires_serial_execution("check"));
        assert!(provider.requires_serial_execution("test"));
        assert!(provider.requires_serial_execution("lint"));
        assert!(provider.requires_serial_execution("fmt"));

        // Node and Uv don't require serialization
        assert!(!NodeWorkspaceProvider.requires_serial_execution("build"));
        assert!(!UvWorkspaceProvider.requires_serial_execution("build"));
    }

    #[test]
    fn cargo_provider_resolves_targeted_commands() {
        let provider = CargoWorkspaceProvider;
        assert_eq!(
            provider.resolve_targeted_command("build", "math-core"),
            Some("cargo build -p math-core".to_string())
        );
        assert_eq!(
            provider.resolve_targeted_command("check", "utils"),
            Some("cargo check -p utils".to_string())
        );
        assert_eq!(
            provider.resolve_targeted_command("test", "math-cli"),
            Some("cargo test -p math-cli".to_string())
        );
        assert_eq!(
            provider.resolve_targeted_command("lint", "auth"),
            Some("cargo clippy -p auth".to_string())
        );
        assert_eq!(
            provider.resolve_targeted_command("fmt", "anything"),
            Some("cargo fmt --all".to_string())
        );
        assert_eq!(
            provider.resolve_targeted_command("doc", "math-core"),
            Some("cargo doc -p math-core".to_string())
        );
        assert_eq!(provider.resolve_targeted_command("unknown", "foo"), None);
    }

    #[test]
    fn cargo_provider_injects_environment_for_build_tasks() {
        let provider = CargoWorkspaceProvider;

        // Build/check/test/doc use rustc, should get environment
        for task in &["build", "check", "test", "doc"] {
            let env = provider.task_environment(task);
            let has_rustc_wrapper = env.iter().any(|(k, _)| k == "RUSTC_WRAPPER");
            // RUSTC_WRAPPER is set only if the binary is found. In tests
            // it might not be, so we check the structure is correct.
            // The env should either have RUSTC_WRAPPER or be empty.
            assert!(
                env.is_empty() || has_rustc_wrapper,
                "task '{task}' should have RUSTC_WRAPPER or no env, got: {env:?}"
            );
        }

        // lint/fmt don't invoke rustc directly
        assert!(provider.task_environment("lint").is_empty());
        assert!(provider.task_environment("fmt").is_empty());
        assert!(provider.task_environment("format").is_empty());
    }

    #[test]
    fn node_provider_injects_no_environment() {
        let provider = NodeWorkspaceProvider;
        assert!(provider.task_environment("build").is_empty());
        assert!(provider.task_environment("test").is_empty());
    }

    #[test]
    fn uv_provider_injects_no_environment() {
        let provider = UvWorkspaceProvider;
        assert!(provider.task_environment("build").is_empty());
        assert!(provider.task_environment("test").is_empty());
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
