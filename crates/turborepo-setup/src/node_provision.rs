//! Locked Node adapter: no resolution, dependency installation, or
//! subprocesses. Plans compose with the complete desired inventory; preparation
//! never publishes.

use std::{fs, io, path::Path, time::Duration};

use semver::Version;
use turborepo_archive::{ExtractedArtifact, Layout};
use turborepo_download::{ApprovedOrigin, DownloadClient, ExpectedSha256};
use turborepo_platform::OperatingSystem::Windows;
use turborepo_tool_install::{Store, Tool};

use crate::{
    NodeArtifact,
    lock::{self, Format, Installation, Lock, Platform},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid locked Node artifact or mappings; regenerate turbo.lock")]
    InvalidLock,
    #[error(
        "no locked Node artifact for this target; update turbo.lock on a supported target or \
         configure verify-system"
    )]
    MissingTarget,
    #[error(
        "managed Node promotion is not qualified for Windows or musl; use a supported GNU \
         Linux/macOS target or configure verify-system"
    )]
    UnsupportedTarget,
    #[error("the complete desired inventory must include this Node plan unchanged")]
    InventoryMismatch,
    #[error("bundled npm package identity or entrypoints do not match the selected resolution")]
    InvalidBundledNpm,
    #[error(transparent)]
    Download(#[from] turborepo_download::Error),
    #[error(transparent)]
    Archive(#[from] turborepo_archive::Error),
    #[error(transparent)]
    Install(#[from] turborepo_tool_install::Error),
    #[error("Node staging I/O failed")]
    Io(#[from] io::Error),
}

pub struct NodeTransport {
    client: DownloadClient,
    // Test transport changes only the origin; lock provenance remains official.
    origin: String,
}
impl NodeTransport {
    pub fn official(_policy: &crate::source_policy::OfficialSourcePolicy) -> Result<Self, Error> {
        Ok(Self {
            client: DownloadClient::new([ApprovedOrigin::https("https://nodejs.org")?])?,
            origin: "https://nodejs.org".into(),
        })
    }

    /// Explicit fixture-only HTTP opt-in; literal loopback IPs only, no env
    /// override.
    #[cfg(any(test, feature = "test-support"))]
    pub fn loopback_http_for_tests(origin: &str) -> Result<Self, Error> {
        Ok(Self {
            client: DownloadClient::loopback_http_for_tests(origin)?,
            origin: origin.trim_end_matches('/').into(),
        })
    }
}

pub struct NodePlan {
    tool: Tool,
    artifact: lock::Artifact,
    windows: bool,
    bundled_npm: Option<String>,
}
impl NodePlan {
    /// Validate only the selected platform's metadata, without modifying the
    /// lock. Executable ownership is exactly the lock's chosen mapping,
    /// never inferred.
    pub fn from_lock(lock: &Lock, platform: Platform) -> Result<Self, Error> {
        let (target, spelling) =
            crate::node::lock_target(platform).ok_or(Error::UnsupportedTarget)?;
        let node = lock.tools().get("node").ok_or(Error::InvalidLock)?;
        let version = Version::parse(&node.version).map_err(|_| Error::InvalidLock)?;
        if node.adapter != "node"
            || version.to_string() != node.version
            || !crate::node_resolution::valid_bundled_npm_option(&node.options)
        {
            return Err(Error::InvalidLock);
        }
        let Installation::Managed { artifacts } = &node.installation else {
            return Err(Error::InvalidLock);
        };
        let parts = artifacts.get(&platform).ok_or(Error::MissingTarget)?;
        if parts.len() != 1 {
            return Err(Error::InvalidLock);
        }
        let artifact = parts.values().next().ok_or(Error::InvalidLock)?.clone();
        let official =
            NodeArtifact::for_platform(&version, target).map_err(|_| Error::InvalidLock)?;
        let windows = target.os() == Windows;
        if !node.options.is_empty()
            && ["npm", "npx"]
                .iter()
                .any(|name| !artifact.executables.contains_key(*name))
        {
            return Err(Error::InvalidLock);
        }
        let extension = if windows { ".zip" } else { ".tar.gz" };
        let root = official
            .filename()
            .strip_suffix(extension)
            .ok_or(Error::InvalidLock)?;
        let node_path = if windows { "node.exe" } else { "bin/node" };
        if artifact.url != official.url()
            || artifact.format != if windows { Format::Zip } else { Format::TarGz }
            || artifact.root_prefix.as_deref() != Some(root)
            || artifact.destination.is_some()
            || artifact.executables.get("node").map(String::as_str) != Some(node_path)
            || artifact.executables.iter().any(|(name, path)| {
                let expected = match (name.as_str(), windows) {
                    ("node", _) => node_path,
                    ("npm", false) => "bin/npm",
                    ("npx", false) => "bin/npx",
                    ("npm", true) => "npm.cmd",
                    ("npx", true) => "npx.cmd",
                    _ => return true,
                };
                path != expected
            })
        {
            return Err(Error::InvalidLock);
        }
        Ok(Self {
            tool: Tool {
                id: "node".into(),
                version: node.version.clone(),
                platform: spelling.into(),
                artifact_sha256: artifact.sha256.clone(),
                executables: artifact.executables.clone(),
            },
            artifact,
            windows,
            bundled_npm: node
                .options
                .get("bundled-npm")
                .map(|values| values[0].clone()),
        })
    }

    pub fn inventory_tool(&self) -> &Tool {
        &self.tool
    }

    /// The caller supplies ALL desired tools and reconciles that same set,
    /// holding this Store lock throughout preparation and reconciliation.
    /// Reuse requires a healthy entire selected
    /// generation with an identical Node tool, not complete desired-set
    /// equality. Unrelated tool changes need no Node download. Windows
    /// trees may be inspected via download, but promotion remains
    /// explicitly unqualified.
    pub async fn prepare_if_needed(
        &self,
        store: &Store,
        desired: &[Tool],
        transport: &NodeTransport,
    ) -> Result<Option<PreparedNode>, Error> {
        if self.windows || !cfg!(unix) {
            return Err(Error::UnsupportedTarget);
        }
        if !desired.iter().any(|tool| tool == &self.tool) {
            return Err(Error::InventoryMismatch);
        }
        // Validate the full cohort's IDs/ownership before any download.
        if let Some(tree) = store.reusable_tree(&self.tool, desired)? {
            self.verify_bundled_npm(&tree)?;
            return Ok(None);
        }
        self.download(transport).await.map(Some)
    }

    /// Download/verify/extract only, with no installation or executable
    /// probing.
    pub async fn download(&self, transport: &NodeTransport) -> Result<PreparedNode, Error> {
        let url = format!(
            "{}{}",
            transport.origin,
            self.artifact
                .url
                .strip_prefix("https://nodejs.org")
                .ok_or(Error::InvalidLock)?
        );
        let bytes = transport
            .client
            .download_verified(
                &url,
                ExpectedSha256::from_hex(&self.artifact.sha256)?,
                turborepo_download::Limits::new(128 * 1024 * 1024, Duration::from_secs(120))?,
            )
            .await?;
        let mut required = vec![self.tool.executables["node"].clone()];
        let base = if self.windows { "" } else { "lib/" };
        for name in ["npm", "npx"] {
            if let Some(path) = self.tool.executables.get(name) {
                if self.windows {
                    required.push(path.clone());
                }
                required.push(format!("{base}node_modules/npm/bin/{name}-cli.js"));
                required.push(format!("{base}node_modules/npm/package.json"));
            }
        }
        let required: Vec<_> = required.iter().map(String::as_str).collect();
        let tree = turborepo_archive::extract(
            &bytes,
            if self.windows {
                turborepo_archive::Format::Zip
            } else {
                turborepo_archive::Format::TarGz
            },
            turborepo_archive::Limits::new(512 * 1024 * 1024, 100_000, 4096, 64)?,
            Layout {
                root: self
                    .artifact
                    .root_prefix
                    .as_deref()
                    .ok_or(Error::InvalidLock)?,
                required_files: &required,
            },
        )?;
        if !self.windows {
            for (name, cli) in [("npm", "npm-cli.js"), ("npx", "npx-cli.js")] {
                if self.tool.executables.contains_key(name)
                    && fs::read_link(tree.root_path().join("bin").join(name))?
                        != Path::new(&format!("../lib/node_modules/npm/bin/{cli}"))
                {
                    return Err(Error::InvalidLock);
                }
            }
        }
        self.verify_bundled_npm(&tree.root_path())?;
        Ok(PreparedNode {
            tree,
            tool: self.tool.clone(),
        })
    }

    pub(crate) fn verify_bundled_npm(&self, tree: &Path) -> Result<(), Error> {
        let Some(version) = &self.bundled_npm else {
            return Ok(());
        };
        let root = tree.join(if self.windows {
            "node_modules/npm"
        } else {
            "lib/node_modules/npm"
        });
        let path = root.join("package.json");
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_file()
            || metadata.len() > crate::registry_metadata::MAX_METADATA_BYTES as u64
        {
            return Err(Error::InvalidBundledNpm);
        }
        let crate::node_discovery::UniqueJson(package) =
            serde_json::from_slice(&fs::read(path)?).map_err(|_| Error::InvalidBundledNpm)?;
        if package.get("name").and_then(|v| v.as_str()) != Some("npm")
            || package.get("version").and_then(|v| v.as_str()) != Some(version)
            || package
                .get("bin")
                .and_then(|v| v.as_object())
                .is_none_or(|bin| bin.len() != 2)
        {
            return Err(Error::InvalidBundledNpm);
        }
        for name in ["npm", "npx"] {
            let cli = format!("bin/{name}-cli.js");
            if package
                .get("bin")
                .and_then(|v| v.get(name))
                .and_then(|v| v.as_str())
                != Some(&cli)
                || !fs::symlink_metadata(root.join(&cli))?.is_file()
                || (!self.windows
                    && fs::read_link(tree.join(format!("bin/{name}")))?
                        != Path::new(&format!("../lib/node_modules/npm/{cli}")))
            {
                return Err(Error::InvalidBundledNpm);
            }
        }
        Ok(())
    }
}

pub struct PreparedNode {
    tree: ExtractedArtifact,
    tool: Tool,
}
impl PreparedNode {
    /// Callback for Store::reconcile. Copy the FULL verified resource tree,
    /// preserving executable modes and safe relative links; never run hooks.
    pub fn stage(
        &self,
        tool: &Tool,
        destination: &Path,
    ) -> Result<(), turborepo_tool_install::Error> {
        if tool.platform.starts_with("windows-") {
            return Err(turborepo_tool_install::Error::UnsupportedPlatform);
        }
        if tool != &self.tool
            || fs::read_dir(destination)?.next().is_some()
            || fs::symlink_metadata(destination)?.file_type().is_symlink()
        {
            return Err(turborepo_tool_install::Error::InvalidInventory);
        }
        copy_tree(&self.tree.root_path(), destination)?;
        Ok(())
    }
}
pub(crate) fn copy_tree(source: &Path, destination: &Path) -> io::Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let path = entry.path();
        let target = destination.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(fs::read_link(path)?, target)?;
            #[cfg(not(unix))]
            return Err(io::Error::other("Node archive links require Unix"));
        } else if kind.is_dir() {
            fs::create_dir(&target)?;
            fs::set_permissions(&target, fs::metadata(&path)?.permissions())?;
            copy_tree(&path, &target)?;
        } else if kind.is_file() {
            fs::copy(path, target)?;
        } else {
            return Err(io::Error::other("unexpected Node resource type"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
