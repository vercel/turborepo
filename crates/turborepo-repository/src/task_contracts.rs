//! Foundational task-contract knowledge.
//!
//! Ecosystems contribute immutable observations about derived I/O defaults,
//! environment patterns, and whether a scope participates in contract-derived
//! hash wiring. Core composes these with turbo.json into effective contracts.
//!
//! JavaScript packages contribute empty contract observations: turbo.json is
//! the whole story. Cargo contributes immutable derivation plans.
//! Execution-only decorations such as compile-cache variables are also
//! projected here, but deliberately do not participate in task hashes.

use std::{borrow::Cow, collections::BTreeMap};

use crate::toolchain::{TaskDefaults, ToolchainId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskEntrypoint {
    Preferred,
    PreferredOnly,
    Candidate,
    Excluded,
}

/// Groups scopes whose preferred entrypoints compete with one another.
/// Deliberately independent from ecosystem provenance.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TaskEntrypointDomain(Cow<'static, str>);

impl TaskEntrypointDomain {
    pub fn new(value: impl Into<Cow<'static, str>>) -> Self {
        Self(value.into())
    }
}

/// A public `command` map key supported by this scope's native command model.
/// This is explicit behavior, separate from open-ended ecosystem provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandMapTarget {
    JavaScript,
    Rust,
}

impl CommandMapTarget {
    fn matches(self, key: &str) -> bool {
        matches!(
            (self, key),
            (Self::JavaScript, "javascript") | (Self::Rust, "rust")
        )
    }
}

/// Per-scope task-contract observation produced at repository construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeTaskContract {
    /// Whether this scope participates in derived task I/O composition.
    ///
    /// JavaScript is always `false`; Cargo contributes immutable plans.
    derives_io: bool,
    defaults: TaskDefaults,
    /// Startup environment patterns this scope needs for derived I/O.
    env_vars: Vec<&'static str>,
    /// Ecosystem provenance when the observation came from a known producer.
    toolchain: Option<ToolchainId>,
    command_map_target: Option<CommandMapTarget>,
    entrypoint_domain: Option<TaskEntrypointDomain>,
    cargo: Option<crate::cargo::CargoTaskContract>,
    static_defaults: BTreeMap<String, TaskDefaults>,
    static_io: BTreeMap<String, crate::toolchain::DerivedTaskIO>,
    static_entrypoints: BTreeMap<String, TaskEntrypoint>,
}

impl ScopeTaskContract {
    /// JavaScript packages: no derived I/O, no env patterns, no defaults.
    pub fn javascript() -> Self {
        Self {
            derives_io: false,
            defaults: TaskDefaults::default(),
            env_vars: Vec::new(),
            toolchain: Some(ToolchainId::JAVASCRIPT),
            command_map_target: Some(CommandMapTarget::JavaScript),
            entrypoint_domain: None,
            cargo: None,
            static_defaults: BTreeMap::new(),
            static_io: BTreeMap::new(),
            static_entrypoints: BTreeMap::new(),
        }
    }

    /// Scope with no toolchain-derived contract (pure-native root, etc.).
    pub fn empty() -> Self {
        Self {
            derives_io: false,
            defaults: TaskDefaults::default(),
            env_vars: Vec::new(),
            toolchain: None,
            command_map_target: None,
            entrypoint_domain: None,
            cargo: None,
            static_defaults: BTreeMap::new(),
            static_io: BTreeMap::new(),
            static_entrypoints: BTreeMap::new(),
        }
    }

    pub(crate) fn cargo(contract: crate::cargo::CargoTaskContract) -> Self {
        Self {
            derives_io: true,
            defaults: TaskDefaults::default(),
            env_vars: crate::cargo::TASK_IO_ENV_VARS.to_vec(),
            toolchain: Some(ToolchainId::RUST),
            command_map_target: Some(CommandMapTarget::Rust),
            entrypoint_domain: Some(TaskEntrypointDomain(Cow::Borrowed("cargo"))),
            cargo: Some(contract),
            static_defaults: BTreeMap::new(),
            static_io: BTreeMap::new(),
            static_entrypoints: BTreeMap::new(),
        }
    }

    /// Static derived contract for simple native producers and tests.
    pub fn derived(
        toolchain: ToolchainId,
        defaults: BTreeMap<String, TaskDefaults>,
        env_vars: Vec<&'static str>,
        io: BTreeMap<String, crate::toolchain::DerivedTaskIO>,
    ) -> Self {
        Self {
            derives_io: true,
            defaults: TaskDefaults::default(),
            env_vars,
            toolchain: Some(toolchain),
            command_map_target: None,
            entrypoint_domain: None,
            cargo: None,
            static_defaults: defaults,
            static_io: io,
            static_entrypoints: BTreeMap::new(),
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

    pub fn with_command_map_target(mut self, target: CommandMapTarget) -> Self {
        self.command_map_target = Some(target);
        self
    }

    pub fn command_map_argv(&self, entries: &[(String, Vec<String>)]) -> Option<Vec<String>> {
        let target = self.command_map_target?;
        entries
            .iter()
            .find(|(key, _)| target.matches(key))
            .map(|(_, argv)| argv.clone())
    }

    pub fn defaults_for_task(&self, task: &str) -> TaskDefaults {
        self.cargo.as_ref().map_or_else(
            || {
                self.static_defaults
                    .get(task)
                    .cloned()
                    .unwrap_or_else(|| self.defaults.clone())
            },
            |cargo| cargo.task_defaults(task),
        )
    }

    pub fn derives_task_io(&self, task: &str) -> bool {
        self.static_io.contains_key(task)
            || self
                .cargo
                .as_ref()
                .is_some_and(|cargo| cargo.derives_task_io(task))
    }

    pub fn derived_task_io(
        &self,
        package: &crate::package_graph::PackageTaskContext<'_>,
        task: &str,
        path_to_root: &str,
        dependencies: &[crate::package_graph::PackageTaskContext<'_>],
        wants_automatic_inputs: bool,
        context: &crate::toolchain::TaskIOContext<'_>,
    ) -> Option<crate::toolchain::DerivedTaskIO> {
        if let Some(io) = self.static_io.get(task) {
            return Some(io.clone());
        }
        self.cargo.as_ref()?.derived_task_io(
            package,
            task,
            path_to_root,
            dependencies,
            wants_automatic_inputs,
            context,
        )
    }

    pub fn task_entrypoint(&self, task: &str) -> Option<TaskEntrypoint> {
        self.static_entrypoints
            .get(task)
            .copied()
            .or_else(|| self.cargo.as_ref()?.task_entrypoint(task))
    }

    pub fn task_entrypoint_domain(&self) -> Option<&TaskEntrypointDomain> {
        self.entrypoint_domain.as_ref()
    }

    pub fn with_task_entrypoints(
        mut self,
        domain: TaskEntrypointDomain,
        entrypoints: BTreeMap<String, TaskEntrypoint>,
    ) -> Self {
        self.entrypoint_domain = Some(domain);
        self.static_entrypoints = entrypoints;
        self
    }

    /// Environment decorations for a compiler cache served by Turborepo.
    /// These are output-transparent execution settings, not hash inputs.
    pub fn compile_cache_env(
        &self,
        endpoint: &crate::toolchain::CompileCacheEndpoint,
        task_env: &std::collections::HashMap<String, String>,
    ) -> Vec<(String, String)> {
        self.cargo.as_ref().map_or_else(Vec::new, |cargo| {
            cargo.compile_cache_env(endpoint, task_env)
        })
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

    pub fn env_vars_by_toolchain(&self) -> BTreeMap<ToolchainId, Vec<&'static str>> {
        let mut patterns = BTreeMap::<ToolchainId, Vec<&'static str>>::new();
        for contract in self.by_scope.values() {
            if contract.env_vars.is_empty() {
                continue;
            }
            let Some(toolchain) = contract.toolchain.clone() else {
                continue;
            };
            patterns
                .entry(toolchain)
                .or_default()
                .extend(contract.env_vars.iter().copied());
        }
        for values in patterns.values_mut() {
            values.sort_unstable();
            values.dedup();
        }
        patterns
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
        assert_eq!(
            contract.command_map_argv(&[("javascript".into(), vec!["node".into()])]),
            Some(vec!["node".into()])
        );
    }

    #[test]
    fn command_map_behavior_is_independent_of_provenance() {
        let contract = ScopeTaskContract::derived(
            ToolchainId::new("custom-rust-producer"),
            BTreeMap::new(),
            Vec::new(),
            BTreeMap::new(),
        )
        .with_command_map_target(CommandMapTarget::Rust);

        assert_eq!(
            contract.command_map_argv(&[
                ("javascript".into(), vec!["node".into()]),
                ("rust".into(), vec!["cargo".into()]),
            ]),
            Some(vec!["cargo".into()])
        );
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
