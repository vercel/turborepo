//! Root `setup` policy for `turbo setup`.
//!
//! The policy is optional. Without it, `turbo setup` infers what to prepare
//! from native declaration files, and a committed `turbo.lock` alone makes a
//! repository managed. Root `tools`, `dependencies`, and `lifecycleScripts`
//! apply to every toolchain; a key named by a canonical toolchain ID overrides
//! them for that toolchain. `additionalTools` declares tools that no built-in
//! adapter manages, and `mirrors` names where setup downloads from.

use std::{collections::BTreeMap, sync::Arc};

use biome_deserialize_macros::Deserializable;
use schemars::JsonSchema;
use serde::Serialize;
use ts_rs::TS;
use turborepo_errors::{Spanned, WithMetadata};
use turborepo_repository::toolchain::ToolchainId;

pub use self::{
    additional_tools::{
        ArtifactFormat, ArtifactPlatform, RawAdditionalTool, RawAdditionalToolArtifact,
    },
    mirrors::{RawSetupMirrors, SetupMirrors, mirror_url},
};
use crate::{error::Error, future_flags::FutureFlags};

mod additional_tools;
mod mirrors;

/// How `turbo setup` prepares a toolchain's tools. `managed` provisions the
/// locked version for this repository. `skip` neither
/// provisions nor verifies the tool, which also skips the toolchain's
/// dependencies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserializable, JsonSchema, TS)]
pub enum ToolPolicy {
    #[serde(rename = "managed")]
    Managed,
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

/// Whether dependency setup lets package managers run lifecycle scripts.
/// `native` keeps each package manager's defaults and committed allowlists or
/// trust settings. `skip` passes the package manager's own flag for skipping
/// them, and fails where the package manager has none.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserializable, JsonSchema, TS)]
pub enum LifecycleScripts {
    #[serde(rename = "native")]
    Native,
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
    /// Whether this toolchain's dependency setup runs lifecycle scripts.
    #[serde(rename = "lifecycleScripts", skip_serializing_if = "Option::is_none")]
    pub lifecycle_scripts: Option<Spanned<LifecycleScripts>>,
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
    /// Whether dependency setup runs lifecycle scripts for every toolchain.
    #[serde(rename = "lifecycleScripts", skip_serializing_if = "Option::is_none")]
    pub lifecycle_scripts: Option<Spanned<LifecycleScripts>>,
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
    /// Tools that no built-in adapter manages, keyed by tool ID. Each declares
    /// an exact version and explicit per-platform artifacts. `turbo setup`
    /// locks, verifies, and installs them without running tool-provided
    /// scripts.
    #[serde(rename = "additionalTools", skip_serializing_if = "Option::is_none")]
    pub additional_tools: Option<BTreeMap<String, Spanned<RawAdditionalTool>>>,
    /// Credential-free HTTPS mirrors for setup downloads. `turbo.lock` keeps
    /// the official URLs, and environment variables override these values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mirrors: Option<Spanned<RawSetupMirrors>>,
}

/// Repository-configured setup values for one toolchain. `None` means the
/// repository did not configure the value, so setup infers it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ToolchainSetupPolicy {
    pub tools: Option<ToolPolicy>,
    pub dependencies: Option<DependencyPolicy>,
    pub lifecycle_scripts: Option<LifecycleScripts>,
}

/// The validated root `setup` policy.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetupPolicy {
    defaults: ToolchainSetupPolicy,
    overrides: BTreeMap<ToolchainId, ToolchainSetupPolicy>,
    additional_tools: BTreeMap<String, RawAdditionalTool>,
    mirrors: SetupMirrors,
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
            lifecycle_scripts: raw.lifecycle_scripts.as_ref().map(|s| *s.as_inner()),
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
                lifecycle_scripts: toolchain.lifecycle_scripts.as_ref().map(|s| *s.as_inner()),
            };
            overrides.insert(id, policy);
        }

        Ok(Self {
            defaults,
            overrides,
            additional_tools: additional_tools::validate(raw.additional_tools.as_ref())?,
            mirrors: mirrors::validate(raw.mirrors.as_ref())?,
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
            lifecycle_scripts: local.lifecycle_scripts.or(self.defaults.lifecycle_scripts),
        }
    }

    /// Validated `additionalTools` declarations, keyed by tool ID.
    pub fn additional_tools(&self) -> &BTreeMap<String, RawAdditionalTool> {
        &self.additional_tools
    }

    /// Validated `setup.mirrors` bases.
    pub fn mirrors(&self) -> &SetupMirrors {
        &self.mirrors
    }

    /// Whether the repository configured any setup value.
    pub fn is_configured(&self) -> bool {
        !self.additional_tools.is_empty()
            || self.mirrors != SetupMirrors::default()
            || self.defaults != ToolchainSetupPolicy::default()
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
        self.dependencies.add_text(text.clone());
        self.lifecycle_scripts.add_text(text);
    }

    fn add_path(&mut self, path: Arc<str>) {
        self.tools.add_path(path.clone());
        self.dependencies.add_path(path.clone());
        self.lifecycle_scripts.add_path(path);
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
        self.lifecycle_scripts.add_text(text.clone());
        for toolchain in self.toolchains_mut() {
            toolchain.add_text(text.clone());
            toolchain.value.add_text(text.clone());
        }
        for tool in self
            .additional_tools
            .iter_mut()
            .flat_map(|tools| tools.values_mut())
        {
            tool.add_text(text.clone());
            tool.value.add_text(text.clone());
        }
        if let Some(mirrors) = &mut self.mirrors {
            mirrors.add_text(text.clone());
            mirrors.value.add_text(text);
        }
    }

    fn add_path(&mut self, path: Arc<str>) {
        self.tools.add_path(path.clone());
        self.dependencies.add_path(path.clone());
        self.lifecycle_scripts.add_path(path.clone());
        for toolchain in self.toolchains_mut() {
            toolchain.add_path(path.clone());
            toolchain.value.add_path(path.clone());
        }
        for tool in self
            .additional_tools
            .iter_mut()
            .flat_map(|tools| tools.values_mut())
        {
            tool.add_path(path.clone());
            tool.value.add_path(path.clone());
        }
        if let Some(mirrors) = &mut self.mirrors {
            mirrors.add_path(path.clone());
            mirrors.value.add_path(path);
        }
    }
}

#[cfg(test)]
mod tests;
