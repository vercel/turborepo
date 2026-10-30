//! Locked installer for repository-declared additional tools: one verified
//! download per tool, either safely extracted or placed as a single binary,
//! then staged as a full tree into a Store generation. This is the
//! installer-adapter boundary for additional tools; another installation
//! source would add its own plan type. No resolution, hooks, tool-provided
//! scripts, privileged installers, source builds, or PATH changes.

use std::{collections::BTreeSet, fs, io, path::Path, time::Duration};

use turborepo_archive::{ExtractedArtifact, Layout};
use turborepo_download::{ApprovedOrigin, DownloadClient, ExpectedSha256, VerifiedArtifact};
use turborepo_tool_install::{Export, Store, Tool};
use url::Url;

use crate::{
    Preparation,
    artifact_cache::ArtifactCache,
    lock::{
        self, Format, Installation, Lock, Platform,
        additional::{ADAPTER, PART},
    },
    mirror::Mirror,
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
    #[error(
        "additional tool installation is qualified for macOS, GNU Linux, and Windows x64 hosts \
         only"
    )]
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
    #[error("offline setup has no verified cached artifact and never contacts upstream")]
    Offline,
    #[error(transparent)]
    Install(#[from] turborepo_tool_install::Error),
}

pub struct ArtifactTransport {
    client: DownloadClient,
    // Test transport changes only the origin; lock provenance is unchanged.
    loopback: Option<String>,
    /// Serves `<base>/<host>/<path>` for each locked URL.
    mirror: Option<String>,
    cache: Option<ArtifactCache>,
    offline: bool,
}

impl ArtifactTransport {
    /// Requests go exactly to the origins of the selected artifacts, never a
    /// prefix or wildcard. Release hosts often redirect to a CDN, so these
    /// digest-pinned downloads may follow HTTPS redirects to any origin; the
    /// lock still records only the declared URL. Construct only after the
    /// setup source preflight passes.
    pub fn new(
        _policy: &crate::source_policy::ToolDownloadPolicy,
        plans: &[ArtifactPlan],
    ) -> Result<Self, Error> {
        let origins: BTreeSet<_> = plans.iter().map(|plan| plan.origin.as_str()).collect();
        let approved = origins
            .into_iter()
            .map(ApprovedOrigin::https)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Error::Transport)?;
        let client = DownloadClient::new(approved).map_err(Error::Transport)?;
        Ok(Self::with_client(client, None, None))
    }

    /// Send every request to a validated mirror instead, at
    /// `<mirror>/<host>/<path>` for each locked URL (the layout `wget -x`
    /// writes), so artifacts from several hosts share one mirror. turbo.lock
    /// keeps the declared URLs and every download is still verified against
    /// its locked digest.
    pub fn mirrored(
        _policy: &crate::source_policy::ToolDownloadPolicy,
        mirror: &Mirror,
    ) -> Result<Self, Error> {
        let client = mirror.client().map_err(Error::Transport)?;
        Ok(Self::with_client(client, None, Some(mirror.base().into())))
    }

    fn with_client(
        client: DownloadClient,
        loopback: Option<String>,
        mirror: Option<String>,
    ) -> Self {
        Self {
            client: client.with_https_redirects(),
            loopback,
            mirror,
            cache: None,
            offline: false,
        }
    }

    /// Reuse verified artifact bytes from a caller-selected user cache. Hits
    /// are re-verified against the locked digest; downloads are stored.
    pub fn with_cache(mut self, cache: Option<ArtifactCache>) -> Self {
        self.cache = cache;
        self
    }

    /// Serve only verified cache hits; a miss fails before any request.
    pub fn offline(mut self) -> Self {
        self.offline = true;
        self
    }

    /// Explicit fixture-only HTTP opt-in; literal loopback IPs only.
    #[cfg(any(test, feature = "test-support"))]
    pub fn loopback_http_for_tests(origin: &str) -> Result<Self, Error> {
        let client = DownloadClient::loopback_http_for_tests(origin).map_err(Error::Transport)?;
        let loopback = Some(origin.trim_end_matches('/').into());
        Ok(Self::with_client(client, loopback, None))
    }
}

pub struct ArtifactPlan {
    tool: Tool,
    platform: Platform,
    artifact: lock::Artifact,
    origin: String,
    /// `host[:port]` of the locked URL, the mirror path's first component.
    authority: String,
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
                executables: Export::files(&artifact.executables),
                vars: Default::default(),
            },
            platform,
            artifact: artifact.clone(),
            origin: url.origin().ascii_serialization(),
            authority: match url.port() {
                Some(port) => format!("{}:{port}", url.host_str().ok_or_else(invalid)?),
                None => url.host_str().ok_or_else(invalid)?.into(),
            },
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
        // Windows arm64 is unqualified, as for every other setup adapter.
        let qualified = match self.platform {
            Platform::WindowsX64 | Platform::WindowsArm64 => {
                self.platform == Platform::WindowsX64 && cfg!(windows)
            }
            _ => cfg!(unix),
        };
        if !qualified {
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

    /// Whether the shared cache holds verified bytes for this artifact.
    pub fn cache_available(&self, cache: Option<&ArtifactCache>) -> bool {
        cache
            .and_then(|cache| cache.get(self.platform, &self.artifact.sha256, MAX_DOWNLOAD_BYTES))
            .is_some()
    }

    /// Download (or read from the shared cache), verify, and unpack into
    /// private staging. Declared executables must be regular files inside the
    /// archive root.
    pub async fn download(&self, transport: &ArtifactTransport) -> Result<PreparedArtifact, Error> {
        let cached = transport
            .cache
            .as_ref()
            .and_then(|cache| cache.get(self.platform, &self.artifact.sha256, MAX_DOWNLOAD_BYTES));
        let from_cache = cached.is_some();
        let bytes = match cached {
            Some(bytes) => bytes,
            None if transport.offline => return Err(Error::Offline),
            None => {
                let bytes = self.fetch(transport).await?;
                // Best effort: a cache write failure never fails verified setup.
                if let Some(cache) = &transport.cache {
                    let _ = cache.put(self.platform, &bytes);
                }
                bytes
            }
        };
        let downloaded_bytes = if from_cache {
            0
        } else {
            bytes.as_bytes().len() as u64
        };
        let prepared = |payload| PreparedArtifact {
            payload,
            tool: self.tool.clone(),
            from_cache,
            downloaded_bytes,
        };
        let format = match self.artifact.format {
            Format::Binary => return Ok(prepared(Payload::Binary(bytes))),
            Format::Tar => turborepo_archive::Format::Tar,
            Format::TarGz => turborepo_archive::Format::TarGz,
            Format::Zip => turborepo_archive::Format::Zip,
        };
        let unpack = |source| Error::Archive {
            tool: self.tool.id.clone(),
            source,
        };
        let required: Vec<_> = self
            .artifact
            .executables
            .values()
            .map(String::as_str)
            .collect();
        // Without a `rootPrefix`, entries sit at the archive's top level.
        let root = self.artifact.root_prefix.as_deref().unwrap_or_default();
        let limits = turborepo_archive::Limits::new(MAX_EXTRACTED_BYTES, 100_000, 4096, 64)
            .map_err(unpack)?;
        let layout = Layout {
            root,
            required_files: &required,
        };
        let tree = turborepo_archive::extract(&bytes, format, limits, layout).map_err(unpack)?;
        Ok(prepared(Payload::Archive(tree)))
    }

    async fn fetch(&self, transport: &ArtifactTransport) -> Result<VerifiedArtifact, Error> {
        let url = match (&transport.loopback, &transport.mirror) {
            (Some(origin), _) => format!("{origin}{}", self.path),
            (None, Some(base)) => format!("{base}/{}{}", self.authority, self.path),
            (None, None) => self.artifact.url.clone(),
        };
        let download = |source| Error::Download {
            tool: self.tool.id.clone(),
            source,
        };
        let digest = ExpectedSha256::from_hex(&self.artifact.sha256).map_err(download)?;
        let limits = turborepo_download::Limits::new(MAX_DOWNLOAD_BYTES, DOWNLOAD_TIMEOUT)
            .map_err(download)?;
        transport
            .client
            .download_verified(&url, digest, limits)
            .await
            .map_err(download)
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
    from_cache: bool,
    downloaded_bytes: u64,
}

impl PreparedArtifact {
    /// Whether verified bytes came from the shared download cache.
    pub fn from_cache(&self) -> bool {
        self.from_cache
    }

    /// Bytes fetched over the network; zero for a cache hit.
    pub fn downloaded_bytes(&self) -> u64 {
        self.downloaded_bytes
    }

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
                let relative = (tool.executables.values().next())
                    .and_then(Export::as_file)
                    .ok_or(turborepo_tool_install::Error::InvalidInventory)?;
                // Lock paths use `/`; never join them unsplit onto a Windows
                // verbatim destination.
                let path: std::path::PathBuf = relative.split('/').collect();
                write_executable(&destination.join(path), bytes.as_bytes())?;
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

/// Windows has no executable bit; PATHEXT and the `.exe` name decide.
#[cfg(not(unix))]
fn write_executable(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
mod tests;
