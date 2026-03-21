use std::{collections::HashSet, fmt, str::FromStr};

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
}

/// Provider trait for dependency graph/dependency resolution logic.
pub trait WorkspaceDependencyProvider {
    fn provider_id(&self) -> WorkspaceProviderId;
}

/// Provider trait for command inference/resolution logic.
pub trait TaskCommandProvider {
    fn provider_id(&self) -> WorkspaceProviderId;
}

/// Provider trait for hash input contributions.
pub trait HashInputProvider {
    fn provider_id(&self) -> WorkspaceProviderId;
}

/// Node provider adapter used for backwards-compatible behavior.
#[derive(Debug, Default, Clone, Copy)]
pub struct NodeWorkspaceProvider;

impl WorkspaceDiscoveryProvider for NodeWorkspaceProvider {
    fn provider_id(&self) -> WorkspaceProviderId {
        WorkspaceProviderId::Node
    }
}

impl WorkspaceDependencyProvider for NodeWorkspaceProvider {
    fn provider_id(&self) -> WorkspaceProviderId {
        WorkspaceProviderId::Node
    }
}

impl TaskCommandProvider for NodeWorkspaceProvider {
    fn provider_id(&self) -> WorkspaceProviderId {
        WorkspaceProviderId::Node
    }
}

impl HashInputProvider for NodeWorkspaceProvider {
    fn provider_id(&self) -> WorkspaceProviderId {
        WorkspaceProviderId::Node
    }
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
        // Milestone 1: keep behavior unchanged and only enable Node.
        available.insert(WorkspaceProviderId::Node);
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
    fn unavailable_provider_errors() {
        let configured = vec!["cargo".to_string()];
        let err = WorkspaceProviderRegistry::new()
            .resolve(Some(configured.as_slice()))
            .unwrap_err();
        assert!(matches!(
            err,
            WorkspaceProviderResolveError::NotAvailable {
                requested: WorkspaceProviderId::Cargo,
                ..
            }
        ));
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
}
