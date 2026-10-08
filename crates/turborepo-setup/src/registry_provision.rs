//! Internal locked registry adapter shared by npm and pnpm. No resolution,
//! hooks, probes, dependency installation or activation.

use std::{fs, io, path::Path, time::Duration};

use sha2::{Digest, Sha256};
use turborepo_archive::{ExtractedArtifact, Layout};
use turborepo_download::{ApprovedOrigin, DownloadClient, ExpectedSha256, ExpectedSha512, Limits};
use turborepo_tool_install::{Store, Tool};

use crate::{
    lock::{self, Format, Installation, Lock, Platform},
    node_discovery::UniqueJson,
    node_provision::{NodePlan, copy_tree},
    package_manager::{CorepackIntegrity, Manager},
    registry_metadata::{self, MAX_METADATA_BYTES, PUBLIC_NPM_REGISTRY as REGISTRY},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid locked registry artifact or executable mapping; regenerate turbo.lock")]
    InvalidLock,
    #[error("package manager requires the selected managed Node in the complete desired inventory")]
    NodeBinding,
    #[error("registry package promotion is unqualified for Windows, musl or non-Unix hosts")]
    UnsupportedTarget,
    #[error("package identity or bin entries do not match the selected lock")]
    InvalidPackage,
    #[error(transparent)]
    Metadata(#[from] registry_metadata::RegistryMetadataError),
    #[error(transparent)]
    Download(#[from] turborepo_download::Error),
    #[error(transparent)]
    Archive(#[from] turborepo_archive::Error),
    #[error(transparent)]
    Install(#[from] turborepo_tool_install::Error),
    #[error("registry package staging I/O failed")]
    Io(#[from] io::Error),
}

pub struct RegistryTransport {
    client: DownloadClient,
    origin: String,
}
impl RegistryTransport {
    // TODO(TURBO-6219): Apply mirror/registry policy to requests and validation.
    // Never fall back to public npm or forward credentials across origins.
    pub fn official(policy: &crate::source_policy::OfficialSourcePolicy) -> Result<Self, Error> {
        Ok(Self {
            client: DownloadClient::new([ApprovedOrigin::https(policy.registry())?])?,
            origin: policy.registry().into(),
        })
    }

    /// Loopback fixtures only; canonical provenance, no env override.
    #[cfg(all(test, unix))]
    pub(crate) fn loopback_http_for_tests(origin: &str) -> Result<Self, Error> {
        Ok(Self {
            client: DownloadClient::with_http_builder(
                reqwest::Client::builder()
                    .no_proxy()
                    .tls_built_in_native_certs(false),
                [ApprovedOrigin::loopback_http_for_tests(origin)?],
            )?,
            origin: origin.trim_end_matches('/').into(),
        })
    }
}

pub(crate) struct RegistryPlan {
    tool: Tool,
    node: Tool,
    artifact: lock::Artifact,
    authored: Option<CorepackIntegrity>,
    launch_directory: String,
    manager: Manager,
}
impl RegistryPlan {
    /// Bind Node and authored integrity; retain locked archive SHA-256.
    pub(crate) fn from_lock(
        lock: &Lock,
        platform: Platform,
        node: &NodePlan,
        authored: Option<&CorepackIntegrity>,
        manager: Manager,
    ) -> Result<Self, Error> {
        let (name, mappings): (_, &[(&str, &str)]) = match manager {
            Manager::Npm => (
                "npm",
                &[("npm", "bin/npm-cli.js"), ("npx", "bin/npx-cli.js")],
            ),
            Manager::Pnpm => (
                "pnpm",
                &[("pnpm", "bin/pnpm.cjs"), ("pnpx", "bin/pnpx.cjs")],
            ),
            _ => return Err(Error::InvalidLock),
        };
        if !cfg!(unix)
            || !matches!(
                platform,
                Platform::MacosX64
                    | Platform::MacosArm64
                    | Platform::LinuxX64Gnu
                    | Platform::LinuxArm64Gnu
            )
        {
            return Err(Error::UnsupportedTarget);
        }
        let selected_node = NodePlan::from_lock(lock, platform).map_err(|_| Error::NodeBinding)?;
        if node.inventory_tool() != selected_node.inventory_tool() {
            return Err(Error::NodeBinding);
        }
        let package = lock.tools().get(name).ok_or(Error::InvalidLock)?;
        if package.adapter != name || !package.options.is_empty() {
            return Err(Error::InvalidLock);
        }
        let Installation::Managed { artifacts } = &package.installation else {
            return Err(Error::InvalidLock);
        };
        let parts = artifacts
            .get(&platform)
            .or_else(|| artifacts.get(&Platform::Any))
            .ok_or(Error::InvalidLock)?;
        if parts.len() != 1 {
            return Err(Error::InvalidLock);
        }
        let artifact = parts.values().next().ok_or(Error::InvalidLock)?.clone();
        if artifact.url != format!("{REGISTRY}/{name}/-/{name}-{}.tgz", package.version)
            || artifact.format != Format::TarGz
            || artifact.root_prefix.as_deref() != Some("package")
            || artifact.destination.is_some()
            || artifact.executables.get(name).map(String::as_str) != Some(mappings[0].1)
            || (manager == Manager::Npm && artifact.executables.len() != mappings.len())
            || artifact.executables.iter().any(|(name, path)| {
                !mappings.iter().any(|(expected_name, expected_path)| {
                    name == expected_name && path == expected_path
                })
            })
        {
            return Err(Error::InvalidLock);
        }
        ExpectedSha256::from_hex(&artifact.sha256)?;
        let node = node.inventory_tool().clone();
        // A changed authored constraint must re-verify, even when archive SHA-256
        // is unchanged. Hash framed identity into a bounded portable component.
        let binding =
            serde_json::to_vec(&(&node, authored.map(|pin| (pin.algorithm, &pin.digest))))
                .map_err(|_| Error::InvalidLock)?;
        let launch_directory = format!("managed-node-{:x}", Sha256::digest(binding));
        let executables = artifact
            .executables
            .keys()
            .map(|name| (name.clone(), format!("{launch_directory}/{name}")))
            .collect();
        Ok(Self {
            tool: Tool {
                id: name.into(),
                version: package.version.clone(),
                platform: node.platform.clone(),
                artifact_sha256: artifact.sha256.clone(),
                executables,
            },
            node,
            artifact,
            authored: authored.cloned(),
            launch_directory,
            manager,
        })
    }

    pub fn inventory_tool(&self) -> &Tool {
        &self.tool
    }

    /// Prepare without executing Node, then reconcile the SAME complete desired
    /// set with all adapter callbacks. Only healthy generations allow reuse.
    pub async fn prepare_if_needed(
        &self,
        store: &Store,
        desired: &[Tool],
        transport: &RegistryTransport,
    ) -> Result<Option<PreparedRegistry>, Error> {
        if !desired.contains(&self.tool) || !desired.contains(&self.node) {
            return Err(Error::NodeBinding);
        }
        if store.can_reuse(&self.tool)? {
            return Ok(None);
        }
        let metadata = transport
            .client
            .read_metadata(
                &format!(
                    "{}/{}/{}",
                    transport.origin, self.tool.id, self.tool.version
                ),
                Limits::new(MAX_METADATA_BYTES, Duration::from_secs(30))?,
            )
            .await?;
        let release = registry_metadata::parse_release(
            &metadata,
            self.manager,
            &self.tool.version,
            self.authored.as_ref(),
        )?;
        if release.tarball != self.artifact.url
            || release
                .additional_sha256
                .as_ref()
                .is_some_and(|pin| pin.digest != self.artifact.sha256)
        {
            return Err(Error::InvalidLock);
        }
        let bytes = transport
            .client
            .download_verified_sha512(
                &format!(
                    "{}/{name}/-/{name}-{}.tgz",
                    transport.origin,
                    self.tool.version,
                    name = self.tool.id
                ),
                ExpectedSha512::from_hex(&release.integrity.digest)?,
                Some(ExpectedSha256::from_hex(&self.artifact.sha256)?),
                Limits::new(64 * 1024 * 1024, Duration::from_secs(120))?,
            )
            .await?;
        let mut required = vec!["package.json"];
        required.extend(self.artifact.executables.values().map(String::as_str));
        let tree = turborepo_archive::extract(
            &bytes,
            turborepo_archive::Format::TarGz,
            turborepo_archive::Limits::new(256 * 1024 * 1024, 100_000, 4096, 64)?,
            Layout {
                root: "package",
                required_files: &required,
            },
        )?;
        let package_path = tree.root_path().join("package.json");
        if fs::metadata(&package_path)?.len() > MAX_METADATA_BYTES as u64 {
            return Err(Error::InvalidPackage);
        }
        let UniqueJson(package) =
            serde_json::from_slice(&fs::read(package_path)?).map_err(|_| Error::InvalidPackage)?;
        if package.get("name").and_then(|v| v.as_str()) != Some(self.tool.id.as_str())
            || package.get("version").and_then(|v| v.as_str()) != Some(&self.tool.version)
            || (self.manager == Manager::Npm
                && package
                    .get("bin")
                    .and_then(|v| v.as_object())
                    .is_none_or(|bin| bin.len() != self.artifact.executables.len()))
            || self.artifact.executables.iter().any(|(name, path)| {
                package
                    .get("bin")
                    .and_then(|v| v.get(name))
                    .and_then(|v| v.as_str())
                    != Some(path)
            })
            || tree.root_path().join(&self.launch_directory).try_exists()?
        {
            return Err(Error::InvalidPackage);
        }
        Ok(Some(PreparedRegistry {
            tree,
            tool: self.tool.clone(),
            launch_directory: self.launch_directory.clone(),
            entrypoints: self.artifact.executables.clone(),
        }))
    }
}

pub struct PreparedRegistry {
    tree: ExtractedArtifact,
    tool: Tool,
    launch_directory: String,
    entrypoints: std::collections::BTreeMap<String, String>,
}
impl PreparedRegistry {
    /// Copy verified resources; launch this generation's managed Node.
    pub fn stage(
        &self,
        tool: &Tool,
        destination: &Path,
    ) -> Result<(), turborepo_tool_install::Error> {
        if !cfg!(unix) {
            return Err(turborepo_tool_install::Error::UnsupportedPlatform);
        }
        if tool != &self.tool
            || fs::symlink_metadata(destination)?.file_type().is_symlink()
            || fs::read_dir(destination)?.next().is_some()
        {
            return Err(turborepo_tool_install::Error::InvalidInventory);
        }
        copy_tree(&self.tree.root_path(), destination)?;
        fs::create_dir(destination.join(&self.launch_directory))?;
        for (name, path) in &self.tool.executables {
            let script = format!(
                "#!/bin/sh\ncase \"$0\" in */*) ;; *) exit 126;; esac\nhere=${{0%/*}}\ncase \
                 \"${{here##*/}}\" in {}) tools=\"$here/../..\";; *) tools=\"$here/../tools\";; \
                 esac\ntools=$(CDPATH= cd -- \"$tools\" && pwd -P) || exit 126\nexec \
                 \"$tools/node/bin/node\" \"$tools/{}/{}\" \"$@\"\n",
                self.launch_directory, self.tool.id, self.entrypoints[name]
            );
            fs::write(destination.join(path), script)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(destination.join(path), fs::Permissions::from_mode(0o755))?;
            }
        }
        Ok(())
    }
}

/// Locked pnpm facade retaining the existing preparation/staging API.
pub struct PnpmPlan(RegistryPlan);
impl PnpmPlan {
    pub fn from_lock(
        lock: &Lock,
        platform: Platform,
        node: &NodePlan,
        authored: Option<&CorepackIntegrity>,
    ) -> Result<Self, Error> {
        RegistryPlan::from_lock(lock, platform, node, authored, Manager::Pnpm).map(Self)
    }

    pub fn inventory_tool(&self) -> &Tool {
        self.0.inventory_tool()
    }

    /// Prepare only; reconcile the same complete desired inventory afterward.
    pub async fn prepare_if_needed(
        &self,
        store: &Store,
        desired: &[Tool],
        transport: &RegistryTransport,
    ) -> Result<Option<PreparedRegistry>, Error> {
        self.0.prepare_if_needed(store, desired, transport).await
    }
}

#[cfg(all(test, unix))]
#[path = "pnpm_provision/tests.rs"]
mod pnpm_tests;
