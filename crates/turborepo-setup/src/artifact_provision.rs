//! Locked installer for repository-declared additional tools: one verified
//! download per tool, either safely extracted or placed as a single binary,
//! then staged as a full tree into a Store generation. This is the
//! installer-adapter boundary for additional tools; another installation
//! source would add its own plan type. No resolution, hooks, tool-provided
//! scripts, privileged installers, source builds, or PATH changes.

use std::{collections::BTreeSet, fs, io, path::Path, time::Duration};

use turborepo_archive::{ExtractedArtifact, Layout};
use turborepo_download::{ApprovedOrigin, DownloadClient, ExpectedSha256, VerifiedArtifact};
use turborepo_tool_install::{Store, Tool};
use url::Url;

use crate::{
    Preparation,
    lock::{
        self, Format, Installation, Lock, Platform,
        additional::{ADAPTER, PART},
    },
    node_provision::copy_tree,
};

const MAX_DOWNLOAD_BYTES: usize = 256 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: usize = 512 * 1024 * 1024;
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "invalid locked additional tool {0:?}; run `turbo setup` locally to rewrite turbo.lock"
    )]
    InvalidLock(String),
    #[error(
        "additional tool {tool:?} has no artifact for {platform}; declare one under \
         setup.additionalTools.{tool}.artifacts"
    )]
    MissingTarget { tool: String, platform: String },
    #[error("additional tool installation is not qualified for Windows or non-Unix hosts")]
    UnsupportedTarget,
    #[error("the complete desired inventory must include this additional tool unchanged")]
    InventoryMismatch,
    #[error("cannot download additional tool {tool:?}: {source}")]
    Download {
        tool: String,
        source: turborepo_download::Error,
    },
    #[error("cannot unpack additional tool {tool:?}: {source}")]
    Archive {
        tool: String,
        source: turborepo_archive::Error,
    },
    #[error(
        "additional tool {tool:?} exports {executable:?}, which {other:?} also exports; rename it \
         under setup.additionalTools.{tool}.artifacts"
    )]
    Collision {
        tool: String,
        executable: String,
        other: String,
    },
    #[error("cannot prepare additional tool downloads: {0}")]
    Transport(#[source] turborepo_download::Error),
    #[error(transparent)]
    Install(#[from] turborepo_tool_install::Error),
}

pub struct ArtifactTransport {
    client: DownloadClient,
    // Test transport changes only the origin; lock provenance is unchanged.
    loopback: Option<String>,
}

impl ArtifactTransport {
    /// Requests go exactly to the origins of the selected artifacts, never a
    /// prefix or wildcard. Release hosts often redirect to a CDN, so these
    /// digest-pinned downloads may follow HTTPS redirects to any origin; the
    /// lock still records only the declared URL. Construct only after the
    /// setup source preflight passes.
    pub fn new(
        _policy: &crate::source_policy::OfficialSourcePolicy,
        plans: &[ArtifactPlan],
    ) -> Result<Self, Error> {
        let origins: BTreeSet<_> = plans.iter().map(|plan| plan.origin.as_str()).collect();
        let approved = origins
            .into_iter()
            .map(ApprovedOrigin::https)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Error::Transport)?;
        let client = DownloadClient::new(approved).map_err(Error::Transport)?;
        Ok(Self {
            client: client.with_https_redirects(),
            loopback: None,
        })
    }

    /// Explicit fixture-only HTTP opt-in; literal loopback IPs only.
    #[cfg(any(test, feature = "test-support"))]
    pub fn loopback_http_for_tests(origin: &str) -> Result<Self, Error> {
        let client = DownloadClient::loopback_http_for_tests(origin).map_err(Error::Transport)?;
        Ok(Self {
            client: client.with_https_redirects(),
            loopback: Some(origin.trim_end_matches('/').into()),
        })
    }
}

pub struct ArtifactPlan {
    tool: Tool,
    artifact: lock::Artifact,
    origin: String,
    path: String,
}

impl ArtifactPlan {
    /// Every locked additional tool for `platform`, in ID order. The lock is
    /// never modified, and platforms are never substituted.
    pub fn from_lock(lock: &Lock, platform: Platform) -> Result<Vec<Self>, Error> {
        lock.tools()
            .iter()
            .filter(|(_, tool)| tool.adapter == ADAPTER)
            .map(|(id, tool)| Self::for_tool(id, tool, platform))
            .collect()
    }

    fn for_tool(id: &str, tool: &lock::Tool, platform: Platform) -> Result<Self, Error> {
        let invalid = || Error::InvalidLock(id.into());
        let spelling = serde_json::to_value(platform)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .ok_or_else(invalid)?;
        let Installation::Managed { artifacts } = &tool.installation else {
            return Err(invalid());
        };
        let parts = artifacts
            .get(&platform)
            .ok_or_else(|| Error::MissingTarget {
                tool: id.into(),
                platform: spelling.clone(),
            })?;
        let artifact = parts
            .get(PART)
            .filter(|_| parts.len() == 1)
            .ok_or_else(invalid)?;
        let binary = artifact.format == Format::Binary;
        let url = Url::parse(&artifact.url).map_err(|_| invalid())?;
        if !tool.options.is_empty()
            || artifacts.contains_key(&Platform::Any)
            || artifact.destination.is_some()
            || (binary && artifact.root_prefix.is_some())
            || (binary && artifact.executables.len() != 1)
            || artifact
                .root_prefix
                .as_ref()
                .is_some_and(|p| p.contains('/'))
            || url.scheme() != "https"
        {
            return Err(invalid());
        }
        Ok(Self {
            tool: Tool {
                id: id.into(),
                version: tool.version.clone(),
                platform: spelling,
                artifact_sha256: artifact.sha256.clone(),
                executables: artifact.executables.clone(),
            },
            artifact: artifact.clone(),
            origin: url.origin().ascii_serialization(),
            path: url.path().into(),
        })
    }

    pub fn inventory_tool(&self) -> &Tool {
        &self.tool
    }

    /// Download source for reports. Locked URLs never carry credentials, a
    /// query, or a fragment.
    pub fn source(&self) -> &str {
        &self.artifact.url
    }

    /// The caller supplies ALL desired tools and reconciles that same set,
    /// holding this Store lock through preparation and reconciliation. A
    /// healthy identical tool is reused unless `preparation` is fresh.
    pub async fn prepare(
        &self,
        store: &Store,
        desired: &[Tool],
        transport: &ArtifactTransport,
        preparation: Preparation,
    ) -> Result<Option<PreparedArtifact>, Error> {
        if self.tool.platform.starts_with("windows-") || !cfg!(unix) {
            return Err(Error::UnsupportedTarget);
        }
        if !desired.contains(&self.tool) {
            return Err(Error::InventoryMismatch);
        }
        // Validate the full cohort before any download.
        if store.reusable_tree(&self.tool, desired)?.is_some()
            && preparation == Preparation::IfNeeded
        {
            return Ok(None);
        }
        self.download(transport).await.map(Some)
    }

    /// Download, verify, and unpack into private staging. Declared executables
    /// must be regular files inside the archive root.
    pub async fn download(&self, transport: &ArtifactTransport) -> Result<PreparedArtifact, Error> {
        let url = match &transport.loopback {
            Some(origin) => format!("{origin}{}", self.path),
            None => self.artifact.url.clone(),
        };
        let download = |source| Error::Download {
            tool: self.tool.id.clone(),
            source,
        };
        let digest = ExpectedSha256::from_hex(&self.artifact.sha256).map_err(download)?;
        let limits = turborepo_download::Limits::new(MAX_DOWNLOAD_BYTES, DOWNLOAD_TIMEOUT)
            .map_err(download)?;
        let bytes = transport
            .client
            .download_verified(&url, digest, limits)
            .await
            .map_err(download)?;
        let format = match self.artifact.format {
            Format::Binary => {
                return Ok(PreparedArtifact {
                    payload: Payload::Binary(bytes),
                    tool: self.tool.clone(),
                });
            }
            Format::Tar => turborepo_archive::Format::Tar,
            Format::TarGz => turborepo_archive::Format::TarGz,
            Format::Zip => turborepo_archive::Format::Zip,
        };
        let unpack = |source| Error::Archive {
            tool: self.tool.id.clone(),
            source,
        };
        let required: Vec<_> = self.tool.executables.values().map(String::as_str).collect();
        // Without a `rootPrefix`, entries sit at the archive's top level.
        let root = self.artifact.root_prefix.as_deref().unwrap_or_default();
        let limits = turborepo_archive::Limits::new(MAX_EXTRACTED_BYTES, 100_000, 4096, 64)
            .map_err(unpack)?;
        let layout = Layout {
            root,
            required_files: &required,
        };
        let tree = turborepo_archive::extract(&bytes, format, limits, layout).map_err(unpack)?;
        Ok(PreparedArtifact {
            payload: Payload::Archive(tree),
            tool: self.tool.clone(),
        })
    }
}

/// Rejects an additional-tool export that shares a PATH name with any other
/// selected tool, built-in or additional, so one never shadows another in the
/// shared managed bin directory. Names fold ASCII case, and on Windows their
/// PATHEXT suffix, because either filesystem may treat them as one name.
pub fn check_exports<'a>(
    builtin: impl IntoIterator<Item = &'a Tool>,
    additional: impl IntoIterator<Item = &'a Tool>,
) -> Result<(), Error> {
    let mut owners = std::collections::BTreeMap::new();
    // Built-in exports first, so a collision always names the additional tool.
    let builtin = builtin.into_iter().map(|tool| (tool, false));
    for (tool, extra) in builtin.chain(additional.into_iter().map(|tool| (tool, true))) {
        let windows = tool.platform.starts_with("windows-");
        for name in tool.executables.keys() {
            if let Some(other) = owners.insert(fold(name, windows), &tool.id)
                && extra
            {
                return Err(Error::Collision {
                    tool: tool.id.clone(),
                    executable: name.clone(),
                    other: other.clone(),
                });
            }
        }
    }
    Ok(())
}

fn fold(name: &str, windows: bool) -> String {
    let name = name.to_ascii_lowercase();
    match [".exe", ".cmd", ".bat", ".com"]
        .iter()
        .find_map(|suffix| name.strip_suffix(suffix))
    {
        Some(stem) if windows => stem.to_owned(),
        _ => name,
    }
}

enum Payload {
    Archive(ExtractedArtifact),
    Binary(VerifiedArtifact),
}

pub struct PreparedArtifact {
    payload: Payload,
    tool: Tool,
}

impl PreparedArtifact {
    /// Callback for Store::reconcile. Copies the FULL verified tree, keeping
    /// adjacent resources, executable modes, and safe relative links.
    pub fn stage(
        &self,
        tool: &Tool,
        destination: &Path,
    ) -> Result<(), turborepo_tool_install::Error> {
        if tool != &self.tool
            || fs::read_dir(destination)?.next().is_some()
            || fs::symlink_metadata(destination)?.file_type().is_symlink()
        {
            return Err(turborepo_tool_install::Error::InvalidInventory);
        }
        match &self.payload {
            Payload::Archive(tree) => copy_tree(&tree.root_path(), destination)?,
            Payload::Binary(bytes) => {
                let relative = tool
                    .executables
                    .values()
                    .next()
                    .ok_or(turborepo_tool_install::Error::InvalidInventory)?;
                write_executable(&destination.join(relative), bytes.as_bytes())?;
            }
        }
        Ok(())
    }
}

#[cfg(unix)]
fn write_executable(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::{
        io::Write,
        os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    };
    if let Some(parent) = path.parent() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o700)
        .open(path)?;
    file.write_all(bytes)?;
    file.set_permissions(fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn write_executable(_: &Path, _: &[u8]) -> io::Result<()> {
    Err(io::Error::other(
        "additional tool binaries require a Unix host",
    ))
}

#[cfg(test)]
mod tests;
