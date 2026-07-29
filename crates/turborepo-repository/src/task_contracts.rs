//! Foundational task-contract knowledge.
//!
//! Ecosystems contribute immutable observations about derived I/O defaults,
//! environment patterns, and whether a scope participates in toolchain-derived
//! hash wiring. Core composes these with turbo.json into effective contracts.
//!
//! JavaScript packages contribute empty contract observations: turbo.json is
//! the whole story. Cargo temporarily retains `Toolchain` derived-I/O
//! callbacks until its Rust port.

use std::collections::BTreeMap;

use crate::toolchain::{TaskDefaults, ToolchainId};

/// Per-scope task-contract observation produced at repository construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeTaskContract {
    /// Whether this scope participates in derived task I/O composition.
    ///
    /// JavaScript is always `false`. Cargo remains on toolchain callbacks
    /// until its port and is not recorded here yet.
    derives_io: bool,
    defaults: TaskDefaults,
    /// Startup environment patterns this scope needs for derived I/O.
    env_vars: Vec<&'static str>,
    /// Toolchain provenance when the observation came from a known ecosystem.
    toolchain: Option<ToolchainId>,
}

impl ScopeTaskContract {
    /// JavaScript packages: no derived I/O, no env patterns, no defaults.
    pub fn javascript() -> Self {
        Self {
            derives_io: false,
            defaults: TaskDefaults::default(),
            env_vars: Vec::new(),
            toolchain: Some(ToolchainId::JAVASCRIPT),
        }
    }

    /// Scope with no toolchain-derived contract (pure-native root, etc.).
    pub fn empty() -> Self {
        Self {
            derives_io: false,
            defaults: TaskDefaults::default(),
            env_vars: Vec::new(),
            toolchain: None,
        }
    }

    pub fn derives_io(&self) -> bool {
        self.derives_io
    }

    pub fn defaults(&self) -> &TaskDefaults {
        &self.defaults
    }

    pub fn env_vars(&self) -> &[&'static str] {
        &self.env_vars
    }

    pub fn toolchain(&self) -> Option<&ToolchainId> {
        self.toolchain.as_ref()
    }
}

/// Immutable catalog of per-scope task-contract observations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskContractKnowledge {
    by_scope: BTreeMap<String, ScopeTaskContract>,
    /// Root `package.json` `engines` captured at observation time for global
    /// hashing. Empty when there is no root JavaScript package.json.
    root_engines: BTreeMap<String, String>,
}

impl TaskContractKnowledge {
    pub fn build(
        observations: impl IntoIterator<Item = (String, ScopeTaskContract)>,
    ) -> Result<Self, TaskContractError> {
        Self::build_with_engines(observations, BTreeMap::new())
    }

    pub fn build_with_engines(
        observations: impl IntoIterator<Item = (String, ScopeTaskContract)>,
        root_engines: BTreeMap<String, String>,
    ) -> Result<Self, TaskContractError> {
        let mut by_scope = BTreeMap::new();
        for (scope, contract) in observations {
            if by_scope.insert(scope.clone(), contract).is_some() {
                return Err(TaskContractError::DuplicateScope { scope });
            }
        }
        Ok(Self {
            by_scope,
            root_engines,
        })
    }

    pub fn for_scope(&self, scope: &str) -> ScopeTaskContract {
        self.by_scope
            .get(scope)
            .cloned()
            .unwrap_or_else(ScopeTaskContract::empty)
    }

    pub fn root_engines(&self) -> &BTreeMap<String, String> {
        &self.root_engines
    }

    pub fn scopes(&self) -> impl Iterator<Item = (&str, &ScopeTaskContract)> {
        self.by_scope
            .iter()
            .map(|(scope, contract)| (scope.as_str(), contract))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TaskContractError {
    #[error("duplicate task-contract observation for scope {scope}")]
    DuplicateScope { scope: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn javascript_contract_is_empty() {
        let contract = ScopeTaskContract::javascript();
        assert!(!contract.derives_io());
        assert_eq!(contract.defaults().cache, None);
        assert!(contract.env_vars().is_empty());
        assert_eq!(contract.toolchain(), Some(&ToolchainId::JAVASCRIPT));
    }

    #[test]
    fn knowledge_indexes_by_scope() {
        let knowledge = TaskContractKnowledge::build([
            ("web".into(), ScopeTaskContract::javascript()),
            ("//".into(), ScopeTaskContract::javascript()),
        ])
        .unwrap();
        assert!(!knowledge.for_scope("web").derives_io());
        assert!(!knowledge.for_scope("missing").derives_io());
        assert_eq!(knowledge.scopes().count(), 2);
    }
}
