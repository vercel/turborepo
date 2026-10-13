//! Pure selection and cache identity, not activation, tool verification or
//! runtime platform detection. Future execution must consume this same owned
//! selection; ignored installation inventory cannot reconstruct it yet.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use sha2::{Digest, Sha256};
use turborepo_platform::{Architecture, OperatingSystem, Platform as HostPlatform};

use crate::lock::{Artifact, Format, Installation, Lock, Platform};

pub const IDENTITY_DOMAIN: &str = "turborepo.setup.execution.v1";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("execution requires a supported platform and explicit ABI, target and CPU variants")]
    Execution,
    #[error("execution identity requires a nonempty selected tool set")]
    EmptySelection,
    #[error("selected tool is missing from the validated resolution")]
    MissingTool,
    #[error("selected adapter is not supported by the builtin identity projection")]
    UnsupportedAdapter,
    #[error("selected tool has no artifact set for the execution platform")]
    MissingArtifacts,
    #[error("native bundled npm and its installation owner must be selected together")]
    MissingOwner,
    #[error("cannot encode execution identity")]
    Encode(#[from] serde_json::Error),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "family", rename_all = "kebab-case")]
pub enum Libc {
    None,
    Gnu { abi: String },
    Musl { abi: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionContext {
    os: &'static str,
    arch: &'static str,
    libc: Libc,
    target: String,
    cpu_variants: BTreeSet<String>,
    #[serde(skip)]
    artifact_platform: Platform,
}

impl ExecutionContext {
    pub fn new(
        host: HostPlatform,
        libc: Libc,
        target: String,
        cpu_variants: BTreeSet<String>,
    ) -> Result<Self, Error> {
        use Architecture::{Arm64, X64};
        use OperatingSystem::{Linux, Macos, Windows};
        let artifact_platform = match (host.os(), host.arch(), &libc) {
            (Macos, X64, Libc::None) => Platform::MacosX64,
            (Macos, Arm64, Libc::None) => Platform::MacosArm64,
            (Windows, X64, Libc::None) => Platform::WindowsX64,
            (Windows, Arm64, Libc::None) => Platform::WindowsArm64,
            (Linux, X64, Libc::Gnu { .. }) => Platform::LinuxX64Gnu,
            (Linux, Arm64, Libc::Gnu { .. }) => Platform::LinuxArm64Gnu,
            (Linux, X64, Libc::Musl { .. }) => Platform::LinuxX64Musl,
            (Linux, Arm64, Libc::Musl { .. }) => Platform::LinuxArm64Musl,
            _ => return Err(Error::Execution),
        };
        if !token(&target)
            || cpu_variants.is_empty()
            || cpu_variants.len() > 32
            || cpu_variants.iter().any(|value| !token(value))
            || matches!(&libc, Libc::Gnu { abi } | Libc::Musl { abi } if !token(abi))
        {
            return Err(Error::Execution);
        }
        Ok(Self {
            os: match host.os() {
                Linux => "linux",
                Macos => "macos",
                Windows => "windows",
                _ => return Err(Error::Execution),
            },
            arch: match host.arch() {
                X64 => "x64",
                Arm64 => "arm64",
                _ => return Err(Error::Execution),
            },
            libc,
            target,
            cpu_variants,
            artifact_platform,
        })
    }

    pub fn artifact_platform(&self) -> Platform {
        self.artifact_platform
    }
}

fn token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-+".contains(&b))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedTool {
    pub adapter: String,
    pub version: String,
    pub options: BTreeMap<String, Vec<String>>,
    pub installation: SelectedInstallation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectedArtifact {
    pub sha256: String,
    pub format: Format,
    pub root_prefix: Option<String>,
    pub destination: Option<String>,
    pub executables: BTreeMap<String, String>,
}

impl From<&Artifact> for SelectedArtifact {
    fn from(artifact: &Artifact) -> Self {
        Self {
            sha256: artifact.sha256.clone(),
            format: artifact.format,
            root_prefix: artifact.root_prefix.clone(),
            destination: artifact.destination.clone(),
            executables: artifact.executables.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectedInstallation {
    Managed {
        platform: Platform,
        parts: BTreeMap<String, SelectedArtifact>,
    },
    Bundled {
        owner: String,
    },
    // Requirements only: adapters must verify exact version, semantic options
    // and these executable names before activation. No invented artifact hash.
    VerifySystem {
        executables: BTreeSet<String>,
    },
}

#[derive(Clone, Debug)]
pub struct ExecutionSnapshot {
    context: ExecutionContext,
    tools: BTreeMap<String, SelectedTool>,
    canonical: Vec<u8>,
    fingerprint: String,
}

impl ExecutionSnapshot {
    // Lock bounds, canonical option ordering and executable collision checks are
    // trusted here; adapter-specific semantic validation still precedes activation.
    pub fn select(
        lock: &Lock,
        context: ExecutionContext,
        selected_ids: &BTreeSet<String>,
    ) -> Result<Self, Error> {
        if selected_ids.is_empty() {
            return Err(Error::EmptySelection);
        }
        // Node exposes the bundled commands: selecting it cannot silently drop
        // the repository's native npm declaration/identity from this snapshot.
        if lock.tools().iter().any(|(id, tool)| {
            matches!(&tool.installation, Installation::Bundled { owner }
                if selected_ids.contains(owner) && !selected_ids.contains(id))
        }) {
            return Err(Error::MissingOwner);
        }
        let mut tools = BTreeMap::new();
        for id in selected_ids {
            let tool = lock.tools().get(id).ok_or(Error::MissingTool)?;
            if !matches!(tool.adapter.as_str(), "node" | "npm" | "pnpm") {
                return Err(Error::UnsupportedAdapter);
            }
            let installation = match &tool.installation {
                Installation::Managed { artifacts } => {
                    let (platform, parts) = artifacts
                        .get_key_value(&context.artifact_platform)
                        .or_else(|| artifacts.get_key_value(&Platform::Any))
                        .ok_or(Error::MissingArtifacts)?;
                    SelectedInstallation::Managed {
                        platform: *platform,
                        parts: parts
                            .iter()
                            .map(|(name, artifact)| {
                                (name.clone(), SelectedArtifact::from(artifact))
                            })
                            .collect(),
                    }
                }
                Installation::Bundled { owner } => {
                    if !selected_ids.contains(owner) {
                        return Err(Error::MissingOwner);
                    }
                    SelectedInstallation::Bundled {
                        owner: owner.clone(),
                    }
                }
                Installation::VerifySystem { executables } => SelectedInstallation::VerifySystem {
                    executables: executables.iter().cloned().collect(),
                },
            };
            tools.insert(
                id.clone(),
                SelectedTool {
                    adapter: tool.adapter.clone(),
                    version: tool.version.clone(),
                    options: tool.options.clone(),
                    installation,
                },
            );
        }
        let projected = tools
            .iter()
            .map(|(id, tool)| (id.as_str(), ToolIdentity::from(tool)))
            .collect();
        let canonical = serde_json::to_vec(&Identity {
            domain: IDENTITY_DOMAIN,
            execution: &context,
            tools: projected,
        })?;
        let fingerprint = format!("{:x}", Sha256::digest(&canonical));
        Ok(Self {
            context,
            tools,
            canonical,
            fingerprint,
        })
    }

    pub fn context(&self) -> &ExecutionContext {
        &self.context
    }
    pub fn tools(&self) -> &BTreeMap<String, SelectedTool> {
        &self.tools
    }
    pub fn canonical_identity(&self) -> &[u8] {
        &self.canonical
    }
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}

#[derive(Serialize)]
struct Identity<'a> {
    domain: &'static str,
    execution: &'a ExecutionContext,
    tools: BTreeMap<&'a str, ToolIdentity<'a>>,
}

#[derive(Serialize)]
struct ToolIdentity<'a> {
    adapter: &'a str,
    version: &'a str,
    options: &'a BTreeMap<String, Vec<String>>,
    installation: InstallationIdentity<'a>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum InstallationIdentity<'a> {
    Managed {
        parts: &'a BTreeMap<String, SelectedArtifact>,
    },
    Bundled {
        owner: &'a str,
    },
    VerifySystem {
        executables: &'a BTreeSet<String>,
    },
}

impl<'a> From<&'a SelectedTool> for ToolIdentity<'a> {
    fn from(tool: &'a SelectedTool) -> Self {
        let installation = match &tool.installation {
            // Selection policy (native vs any) is not execution compatibility.
            // Identical active bytes/layout have identical identity on the same
            // explicit execution context, regardless of the selector key.
            SelectedInstallation::Managed { parts, .. } => InstallationIdentity::Managed { parts },
            SelectedInstallation::Bundled { owner } => InstallationIdentity::Bundled { owner },
            SelectedInstallation::VerifySystem { executables } => {
                InstallationIdentity::VerifySystem { executables }
            }
        };
        Self {
            adapter: &tool.adapter,
            version: &tool.version,
            options: &tool.options,
            installation,
        }
    }
}

#[cfg(test)]
mod tests;
