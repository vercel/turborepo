//! Effective per-toolchain setup: CLI override > root `setup` policy >
//! inferred default. Reported before any download or installation.

use turborepo_repository::toolchain::ToolchainId;
use turborepo_turbo_json::{DependencyPolicy, SetupPolicy, ToolPolicy};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Source {
    Cli(&'static str),
    Policy,
    Inferred,
}

impl Source {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Cli(flag) => flag,
            Self::Policy => "turbo.json setup policy",
            Self::Inferred => "inferred",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Toolchain {
    pub id: ToolchainId,
    pub tools: (ToolPolicy, Source),
    pub dependencies: (DependencyPolicy, Source),
}

impl Toolchain {
    fn new(policy: &SetupPolicy, id: ToolchainId, tools_only: bool) -> Self {
        let configured = policy.toolchain(&id);
        let tools = configured
            .tools
            .map_or((ToolPolicy::Managed, Source::Inferred), |tools| {
                (tools, Source::Policy)
            });
        let dependencies = if tools_only {
            (DependencyPolicy::Skip, Source::Cli("--tools-only"))
        } else {
            configured
                .dependencies
                .map_or((DependencyPolicy::Locked, Source::Inferred), |deps| {
                    (deps, Source::Policy)
                })
        };
        Self {
            id,
            tools,
            dependencies,
        }
    }

    fn report(&self) -> String {
        let (tools, tools_source) = self.tools;
        let (dependencies, dependencies_source) = self.dependencies;
        let id = self.id.as_str();
        if tools == ToolPolicy::Skip {
            return format!(
                "{id}: skipped ({}); turbo setup neither provisions nor verifies its tools, so \
                 tasks use whatever is on PATH",
                tools_source.label()
            );
        }
        let tools = match tools {
            ToolPolicy::Managed => "managed",
            ToolPolicy::VerifySystem => "verify-system",
            ToolPolicy::Skip => "skipped",
        };
        let dependencies = match dependencies {
            DependencyPolicy::Locked => "locked",
            DependencyPolicy::Skip => "skipped",
        };
        let mut line = format!(
            "{id}: tools {tools} ({}); dependencies {dependencies} ({})",
            tools_source.label(),
            dependencies_source.label()
        );
        if self.tools.0 == ToolPolicy::VerifySystem {
            line.push_str(
                "; turbo setup verifies the installed tools against turbo.lock but does not \
                 install them, so consistency depends on each machine",
            );
        }
        line
    }
}

/// Effective setup for the eligible toolchains, JavaScript first.
pub(super) struct Plan(Vec<Toolchain>);

impl Plan {
    pub fn new(
        policy: &SetupPolicy,
        eligible: impl IntoIterator<Item = ToolchainId>,
        tools_only: bool,
    ) -> Self {
        Self(
            eligible
                .into_iter()
                .map(|id| Toolchain::new(policy, id, tools_only))
                .collect(),
        )
    }

    pub fn toolchain(&self, id: &ToolchainId) -> Option<&Toolchain> {
        self.0.iter().find(|toolchain| toolchain.id == *id)
    }

    pub fn toolchains(&self) -> &[Toolchain] {
        &self.0
    }

    /// Whether every eligible toolchain skips dependency setup.
    pub fn tools_only(&self) -> bool {
        self.0
            .iter()
            .all(|toolchain| toolchain.dependencies.0 == DependencyPolicy::Skip)
    }

    /// Eligible toolchains without a setup adapter yet, unless skipped.
    /// JavaScript has managed and verify-system adapters; Rust, Python and Go
    /// only managed.
    pub fn unsupported(&self) -> bool {
        self.0.iter().any(|toolchain| match toolchain.tools.0 {
            ToolPolicy::Skip => false,
            _ if toolchain.id == ToolchainId::JAVASCRIPT => false,
            ToolPolicy::Managed => {
                ![ToolchainId::RUST, ToolchainId::PYTHON, ToolchainId::GO].contains(&toolchain.id)
            }
            ToolPolicy::VerifySystem => true,
        })
    }

    pub fn report(&self) -> Vec<String> {
        self.0.iter().map(Toolchain::report).collect()
    }
}

#[cfg(test)]
mod tests {
    use turborepo_turbo_json::{FutureFlags, RawRootTurboJson, RawTurboJson};

    use super::*;

    fn policy(setup: &str) -> SetupPolicy {
        let json = format!(
            r#"{{"futureFlags":{{"experimentalSetup":true,"experimentalCargoWorkspaces":true}},"setup":{setup}}}"#
        );
        let raw: RawTurboJson = RawRootTurboJson::parse(&json, "turbo.json")
            .unwrap()
            .try_into()
            .unwrap();
        let flags: FutureFlags = raw.future_flags.clone().unwrap().into_inner();
        SetupPolicy::from_raw(raw.setup.as_ref(), &flags).unwrap()
    }

    fn plan(setup: &str, tools_only: bool) -> Plan {
        let eligible = [ToolchainId::JAVASCRIPT, ToolchainId::RUST];
        Plan::new(&policy(setup), eligible, tools_only)
    }

    #[test]
    fn cli_overrides_policy_which_overrides_inference() {
        let inferred = plan("{}", false);
        // Inferred (managed) Rust has a setup adapter.
        assert!(!inferred.tools_only() && !inferred.unsupported());
        let javascript = inferred.toolchain(&ToolchainId::JAVASCRIPT).unwrap();
        assert_eq!(javascript.tools, (ToolPolicy::Managed, Source::Inferred));
        assert_eq!(
            javascript.dependencies,
            (DependencyPolicy::Locked, Source::Inferred)
        );

        // A repository can choose tools-only setup without the CLI flag.
        let repository = plan(r#"{"dependencies":"skip","rust":{"tools":"skip"}}"#, false);
        assert!(repository.tools_only() && !repository.unsupported());
        // --tools-only overrides a locked repository policy for one invocation.
        let cli = plan(r#"{"dependencies":"locked","rust":{"tools":"skip"}}"#, true);
        assert!(cli.tools_only());
        assert_eq!(
            cli.report(),
            [
                "javascript: tools managed (inferred); dependencies skipped (--tools-only)",
                "rust: skipped (turbo.json setup policy); turbo setup neither provisions nor \
                 verifies its tools, so tasks use whatever is on PATH",
            ]
        );
    }

    #[test]
    fn verify_system_is_reported_as_policy() {
        let plan = plan(r#"{"javascript":{"tools":"verify-system"}}"#, true);
        assert_eq!(
            plan.report()[0],
            "javascript: tools verify-system (turbo.json setup policy); dependencies skipped \
             (--tools-only); turbo setup verifies the installed tools against turbo.lock but does \
             not install them, so consistency depends on each machine"
        );
    }
}
