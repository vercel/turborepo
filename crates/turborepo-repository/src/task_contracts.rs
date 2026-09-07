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

pub use crate::native_tasks::TaskEntrypoint;
use crate::toolchain::{TaskDefaults, ToolchainId};

/// Groups scopes whose preferred entrypoints compete with one another.
/// Deliberately independent from ecosystem provenance.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TaskEntrypointDomain(Cow<'static, str>);

impl TaskEntrypointDomain {
    pub fn new(value: impl Into<Cow<'static, str>>) -> Self {
        Self(value.into())
    }
}

/// Groups scopes that share one declared startup-environment projection.
/// Deliberately independent from ecosystem provenance.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskEnvironmentDomain(Cow<'static, str>);

impl TaskEnvironmentDomain {
    pub fn new(value: impl Into<Cow<'static, str>>) -> Self {
        Self(value.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskEnvironmentRequirement {
    domain: TaskEnvironmentDomain,
    vars: Vec<&'static str>,
}

impl TaskEnvironmentRequirement {
    pub fn new(domain: TaskEnvironmentDomain, vars: Vec<&'static str>) -> Self {
        Self { domain, vars }
    }
}

/// A public `command` map key supported by this scope's native command model.
/// This is explicit behavior, separate from open-ended ecosystem provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandMapTarget {
    JavaScript,
    Rust,
    Python,
    Go,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrunePackageMode {
    JavaScript,
    NativeCopy,
    NativeDomain(crate::prune_knowledge::PruneDomainId),
}

/// Whether this scope's source directory participates in dependent automatic
/// input derivation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencySourceInputs {
    /// The producer did not classify participation. Consumers must fail closed.
    Unknown,
    /// Include this scope's source directory in dependent input closures.
    Include,
    /// Authoritatively exclude this scope from dependent source inputs.
    Exclude,
}

impl CommandMapTarget {
    fn matches(self, key: &str) -> bool {
        matches!(
            (self, key),
            (Self::JavaScript, "javascript")
                | (Self::Rust, "rust")
                | (Self::Python, "python")
                | (Self::Go, "go")
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DynamicTaskContract {
    Cargo(crate::cargo::CargoTaskContract),
    Python(crate::uv::UvTaskContract),
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
    environment: Option<TaskEnvironmentRequirement>,
    /// Ecosystem provenance when the observation came from a known producer.
    toolchain: Option<ToolchainId>,
    command_map_target: Option<CommandMapTarget>,
    entrypoint_domain: Option<TaskEntrypointDomain>,
    prune_package_mode: Option<PrunePackageMode>,
    dependency_source_inputs: DependencySourceInputs,
    dynamic: Option<DynamicTaskContract>,
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
            environment: None,
            toolchain: Some(ToolchainId::JAVASCRIPT),
            command_map_target: Some(CommandMapTarget::JavaScript),
            entrypoint_domain: None,
            prune_package_mode: Some(PrunePackageMode::JavaScript),
            dependency_source_inputs: DependencySourceInputs::Exclude,
            dynamic: None,
            static_defaults: BTreeMap::new(),
            static_io: BTreeMap::new(),
            static_entrypoints: BTreeMap::new(),
        }
    }

    /// Go module and workspace scopes with static native task tables.
    pub fn go() -> Self {
        Self {
            derives_io: false,
            defaults: TaskDefaults::default(),
            environment: None,
            toolchain: Some(ToolchainId::GO),
            command_map_target: Some(CommandMapTarget::Go),
            entrypoint_domain: Some(TaskEntrypointDomain(Cow::Borrowed("go"))),
            prune_package_mode: None,
            dependency_source_inputs: DependencySourceInputs::Unknown,
            dynamic: None,
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
            environment: None,
            toolchain: None,
            command_map_target: None,
            entrypoint_domain: None,
            prune_package_mode: None,
            dependency_source_inputs: DependencySourceInputs::Unknown,
            dynamic: None,
            static_defaults: BTreeMap::new(),
            static_io: BTreeMap::new(),
            static_entrypoints: BTreeMap::new(),
        }
    }

    pub(crate) fn cargo(contract: crate::cargo::CargoTaskContract) -> Self {
        let dependency_source_inputs = contract.dependency_source_inputs();
        Self {
            derives_io: true,
            defaults: TaskDefaults::default(),
            environment: Some(TaskEnvironmentRequirement::new(
                TaskEnvironmentDomain(Cow::Borrowed("cargo-task-io")),
                crate::cargo::TASK_IO_ENV_VARS.to_vec(),
            )),
            toolchain: Some(ToolchainId::RUST),
            command_map_target: Some(CommandMapTarget::Rust),
            entrypoint_domain: Some(TaskEntrypointDomain(Cow::Borrowed("cargo"))),
            prune_package_mode: Some(PrunePackageMode::NativeDomain(
                crate::prune_knowledge::CARGO_PRUNE_DOMAIN.clone(),
            )),
            dependency_source_inputs,
            dynamic: Some(DynamicTaskContract::Cargo(contract)),
            static_defaults: BTreeMap::new(),
            static_io: BTreeMap::new(),
            static_entrypoints: BTreeMap::new(),
        }
    }

    pub(crate) fn python(contract: crate::uv::UvTaskContract) -> Self {
        let dependency_source_inputs = contract.dependency_source_inputs();
        let mut environment_vars = crate::uv::HASHED_ENV_VARS.to_vec();
        environment_vars.extend(crate::uv::PROJECTED_ONLY_ENV_VARS);
        Self {
            derives_io: true,
            defaults: TaskDefaults::default(),
            environment: Some(TaskEnvironmentRequirement::new(
                TaskEnvironmentDomain(Cow::Borrowed("uv-task-io")),
                environment_vars,
            )),
            toolchain: Some(ToolchainId::PYTHON),
            command_map_target: Some(CommandMapTarget::Python),
            entrypoint_domain: Some(TaskEntrypointDomain(Cow::Borrowed("uv"))),
            prune_package_mode: Some(PrunePackageMode::NativeDomain(
                crate::prune_knowledge::PYTHON_PRUNE_DOMAIN.clone(),
            )),
            dependency_source_inputs,
            dynamic: Some(DynamicTaskContract::Python(contract)),
            static_defaults: BTreeMap::new(),
            static_io: BTreeMap::new(),
            static_entrypoints: BTreeMap::new(),
        }
    }

    /// Static derived contract for simple native producers and tests.
    pub fn derived(
        toolchain: ToolchainId,
        environment: Option<TaskEnvironmentRequirement>,
        defaults: BTreeMap<String, TaskDefaults>,
        io: BTreeMap<String, crate::toolchain::DerivedTaskIO>,
    ) -> Self {
        Self {
            derives_io: true,
            defaults: TaskDefaults::default(),
            environment,
            toolchain: Some(toolchain),
            command_map_target: None,
            entrypoint_domain: None,
            prune_package_mode: Some(PrunePackageMode::NativeCopy),
            dependency_source_inputs: DependencySourceInputs::Unknown,
            dynamic: None,
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
        self.environment
            .as_ref()
            .map_or(&[], |requirement| requirement.vars.as_slice())
    }

    pub fn environment_domain(&self) -> Option<&TaskEnvironmentDomain> {
        self.environment
            .as_ref()
            .map(|requirement| &requirement.domain)
    }

    pub fn prune_package_mode(&self) -> Option<&PrunePackageMode> {
        self.prune_package_mode.as_ref()
    }

    pub fn with_prune_package_mode(mut self, mode: PrunePackageMode) -> Self {
        self.prune_package_mode = Some(mode);
        self
    }

    /// Classifies whether this scope's source directory may participate in a
    /// dependent task's derived input closure.
    pub fn dependency_source_inputs(&self) -> DependencySourceInputs {
        self.dependency_source_inputs
    }

    /// Explicitly classifies dependency source input participation.
    pub fn with_dependency_source_inputs(mut self, participation: DependencySourceInputs) -> Self {
        self.dependency_source_inputs = participation;
        self
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
        self.static_defaults
            .get(task)
            .cloned()
            .unwrap_or_else(|| self.defaults.clone())
    }

    pub fn derives_task_io(&self, task: &str) -> bool {
        self.static_io.contains_key(task)
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
        match self.dynamic.as_ref()? {
            DynamicTaskContract::Cargo(contract) => contract.derived_task_io(
                package,
                task,
                path_to_root,
                dependencies,
                wants_automatic_inputs,
                context,
            ),
            DynamicTaskContract::Python(contract) => contract.derived_task_io(
                package,
                task,
                path_to_root,
                dependencies,
                wants_automatic_inputs,
                context,
            ),
        }
    }

    pub fn task_entrypoint(&self, task: &str) -> Option<TaskEntrypoint> {
        self.static_entrypoints.get(task).copied()
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
        match self.dynamic.as_ref() {
            Some(DynamicTaskContract::Cargo(contract)) => {
                contract.compile_cache_env(endpoint, task_env)
            }
            _ => Vec::new(),
        }
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

    pub fn env_vars_by_domain(&self) -> BTreeMap<TaskEnvironmentDomain, Vec<&'static str>> {
        let mut patterns = BTreeMap::<TaskEnvironmentDomain, Vec<&'static str>>::new();
        for contract in self.by_scope.values() {
            let Some(requirement) = contract.environment.as_ref() else {
                continue;
            };
            patterns
                .entry(requirement.domain.clone())
                .or_default()
                .extend(requirement.vars.iter().copied());
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
            None,
            BTreeMap::new(),
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
    fn dependency_source_inputs_are_independent_of_provenance() {
        let contract = ScopeTaskContract::derived(
            ToolchainId::new("custom-cargo-producer"),
            None,
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .with_dependency_source_inputs(DependencySourceInputs::Include);

        assert_eq!(
            contract.dependency_source_inputs(),
            DependencySourceInputs::Include
        );
        assert_eq!(
            ScopeTaskContract::javascript().dependency_source_inputs(),
            DependencySourceInputs::Exclude
        );
        assert_eq!(
            ScopeTaskContract::empty().dependency_source_inputs(),
            DependencySourceInputs::Unknown
        );
        let excluded = ScopeTaskContract::derived(
            ToolchainId::new("custom-aggregate"),
            None,
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .with_dependency_source_inputs(DependencySourceInputs::Exclude);
        assert_eq!(
            excluded.dependency_source_inputs(),
            DependencySourceInputs::Exclude
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
