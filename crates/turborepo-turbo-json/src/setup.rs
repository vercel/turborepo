//! Root `setup` policy for `turbo setup`.
//!
//! The policy is optional. Without it, `turbo setup` infers what to prepare
//! from native declaration files, and a committed `turbo.lock` alone makes a
//! repository managed. Root `tools` and `dependencies` apply to every
//! toolchain; a key named by a canonical toolchain ID overrides them for that
//! toolchain.

use std::{collections::BTreeMap, sync::Arc};

use biome_deserialize_macros::Deserializable;
use schemars::JsonSchema;
use serde::Serialize;
use ts_rs::TS;
use turborepo_errors::{Spanned, WithMetadata};
use turborepo_repository::toolchain::ToolchainId;

use crate::{error::Error, future_flags::FutureFlags};

/// How `turbo setup` prepares a toolchain's tools. `managed` provisions the
/// locked version for this repository. `verify-system` uses an installed tool
/// after verifying that it matches the locked version. `skip` neither
/// provisions nor verifies the tool, which also skips the toolchain's
/// dependencies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserializable, JsonSchema, TS)]
pub enum ToolPolicy {
    #[serde(rename = "managed")]
    Managed,
    #[serde(rename = "verify-system")]
    VerifySystem,
    #[serde(rename = "skip")]
    Skip,
}

/// How `turbo setup` prepares a toolchain's dependencies. `locked` installs
/// them from the committed ecosystem lockfile without changing it. `skip`
/// does not install them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserializable, JsonSchema, TS)]
pub enum DependencyPolicy {
    #[serde(rename = "locked")]
    Locked,
    #[serde(rename = "skip")]
    Skip,
}

/// Setup policy for one toolchain. Omitted values use the root `setup`
/// values, then the inferred default.
#[derive(Clone, Debug, Default, Serialize, Deserializable, JsonSchema)]
#[schemars(rename = "ToolchainSetupPolicy")]
#[deserializable(unknown_fields = "deny")]
pub struct RawToolchainSetupPolicy {
    /// How `turbo setup` prepares this toolchain's tools.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Spanned<ToolPolicy>>,
    /// How `turbo setup` prepares this toolchain's dependencies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependencies: Option<Spanned<DependencyPolicy>>,
}

/// Repository policy for `turbo setup`. Only allowed in the root turbo.json
/// and requires `futureFlags.experimentalSetup`.
///
/// Optional: without it, `turbo setup` infers what to prepare from native
/// declaration files, and detected tools are managed. Root `tools` and
/// `dependencies` apply to every toolchain. A toolchain key overrides them
/// for that toolchain.
#[derive(Clone, Debug, Default, Serialize, Deserializable, JsonSchema)]
#[schemars(rename = "SetupPolicy")]
#[deserializable(unknown_fields = "deny")]
pub struct RawSetupPolicy {
    /// How `turbo setup` prepares tools for every toolchain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Spanned<ToolPolicy>>,
    /// How `turbo setup` prepares dependencies for every toolchain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependencies: Option<Spanned<DependencyPolicy>>,
    /// Setup policy for the JavaScript toolchain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub javascript: Option<Spanned<RawToolchainSetupPolicy>>,
    /// Setup policy for the Rust toolchain. Requires
    /// `futureFlags.experimentalCargoWorkspaces`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rust: Option<Spanned<RawToolchainSetupPolicy>>,
    /// Setup policy for the Python toolchain. Requires
    /// `futureFlags.experimentalPythonWorkspaces`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub python: Option<Spanned<RawToolchainSetupPolicy>>,
    /// Setup policy for the Go toolchain. Requires
    /// `futureFlags.experimentalGoWorkspaces`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub go: Option<Spanned<RawToolchainSetupPolicy>>,
}

/// Repository-configured setup values for one toolchain. `None` means the
/// repository did not configure the value, so setup infers it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ToolchainSetupPolicy {
    pub tools: Option<ToolPolicy>,
    pub dependencies: Option<DependencyPolicy>,
}

/// The validated root `setup` policy.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetupPolicy {
    defaults: ToolchainSetupPolicy,
    overrides: BTreeMap<ToolchainId, ToolchainSetupPolicy>,
}

impl SetupPolicy {
    /// Validates a root `setup` value. An absent value is the empty policy.
    pub fn from_raw(
        raw: Option<&Spanned<RawSetupPolicy>>,
        future_flags: &FutureFlags,
    ) -> Result<Self, Error> {
        let Some(raw) = raw else {
            return Ok(Self::default());
        };
        if !future_flags.experimental_setup {
            let (span, text) = raw.span_and_text("turbo.json");
            return Err(Error::SetupRequiresFlag { span, text });
        }
        let raw = raw.as_inner();
        let defaults = ToolchainSetupPolicy {
            tools: raw.tools.as_ref().map(|tools| *tools.as_inner()),
            dependencies: raw.dependencies.as_ref().map(|deps| *deps.as_inner()),
        };
        check_dependencies(defaults.tools, raw.dependencies.as_ref())?;

        let mut overrides = BTreeMap::new();
        for (id, toolchain) in [
            (ToolchainId::JAVASCRIPT, &raw.javascript),
            (ToolchainId::RUST, &raw.rust),
            (ToolchainId::PYTHON, &raw.python),
            (ToolchainId::GO, &raw.go),
        ] {
            let Some(toolchain) = toolchain else {
                continue;
            };
            if let Some(flag) = missing_flag(&id, future_flags) {
                let (span, text) = toolchain.span_and_text("turbo.json");
                return Err(Error::SetupToolchainRequiresFlag {
                    key: id.as_str().to_string(),
                    flag,
                    span,
                    text,
                });
            }
            let toolchain = toolchain.as_inner();
            let tools = toolchain.tools.as_ref().map(|tools| *tools.as_inner());
            check_dependencies(tools.or(defaults.tools), toolchain.dependencies.as_ref())?;
            let policy = ToolchainSetupPolicy {
                tools,
                dependencies: toolchain.dependencies.as_ref().map(|deps| *deps.as_inner()),
            };
            overrides.insert(id, policy);
        }

        Ok(Self {
            defaults,
            overrides,
        })
    }

    /// The repository policy for `toolchain`. A toolchain value takes
    /// precedence over the root value. Skipped tools also skip dependencies
    /// unless the toolchain sets `dependencies` itself.
    pub fn toolchain(&self, toolchain: &ToolchainId) -> ToolchainSetupPolicy {
        let local = self.overrides.get(toolchain).copied().unwrap_or_default();
        let tools = local.tools.or(self.defaults.tools);
        let dependencies = local.dependencies.or(if tools == Some(ToolPolicy::Skip) {
            Some(DependencyPolicy::Skip)
        } else {
            self.defaults.dependencies
        });
        ToolchainSetupPolicy {
            tools,
            dependencies,
        }
    }

    /// Whether the repository configured any setup value.
    pub fn is_configured(&self) -> bool {
        self.defaults != ToolchainSetupPolicy::default()
            || self
                .overrides
                .values()
                .any(|policy| *policy != ToolchainSetupPolicy::default())
    }
}

/// The workspace flag that gates `toolchain`, when it is disabled.
fn missing_flag(toolchain: &ToolchainId, flags: &FutureFlags) -> Option<&'static str> {
    match toolchain.as_str() {
        "rust" if !flags.experimental_cargo_workspaces => Some("experimentalCargoWorkspaces"),
        "python" if !flags.experimental_python_workspaces => Some("experimentalPythonWorkspaces"),
        "go" if !flags.experimental_go_workspaces => Some("experimentalGoWorkspaces"),
        _ => None,
    }
}

/// Locked dependencies need a managed or verified tool to install them.
fn check_dependencies(
    tools: Option<ToolPolicy>,
    dependencies: Option<&Spanned<DependencyPolicy>>,
) -> Result<(), Error> {
    match dependencies {
        Some(dependencies)
            if tools == Some(ToolPolicy::Skip)
                && *dependencies.as_inner() == DependencyPolicy::Locked =>
        {
            let (span, text) = dependencies.span_and_text("turbo.json");
            Err(Error::SetupLockedDependenciesWithSkippedTools { span, text })
        }
        _ => Ok(()),
    }
}

impl WithMetadata for RawToolchainSetupPolicy {
    fn add_text(&mut self, text: Arc<str>) {
        self.tools.add_text(text.clone());
        self.dependencies.add_text(text);
    }

    fn add_path(&mut self, path: Arc<str>) {
        self.tools.add_path(path.clone());
        self.dependencies.add_path(path);
    }
}

impl RawSetupPolicy {
    fn toolchains_mut(&mut self) -> impl Iterator<Item = &mut Spanned<RawToolchainSetupPolicy>> {
        [
            &mut self.javascript,
            &mut self.rust,
            &mut self.python,
            &mut self.go,
        ]
        .into_iter()
        .flatten()
    }
}

impl WithMetadata for RawSetupPolicy {
    fn add_text(&mut self, text: Arc<str>) {
        self.tools.add_text(text.clone());
        self.dependencies.add_text(text.clone());
        for toolchain in self.toolchains_mut() {
            toolchain.add_text(text.clone());
            toolchain.value.add_text(text.clone());
        }
    }

    fn add_path(&mut self, path: Arc<str>) {
        self.tools.add_path(path.clone());
        self.dependencies.add_path(path.clone());
        for toolchain in self.toolchains_mut() {
            toolchain.add_path(path.clone());
            toolchain.value.add_path(path.clone());
        }
    }
}

#[cfg(test)]
mod tests;
