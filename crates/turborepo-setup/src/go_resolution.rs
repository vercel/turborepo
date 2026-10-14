//! Official Go release index → portable lock selection. Pure bounded parsing
//! and selection plus one metadata read; no artifact download or install.

use std::{collections::BTreeMap, time::Duration};

use serde_json::Value;
use turborepo_download::{ApprovedOrigin, DownloadClient, Limits};

use crate::{
    go_discovery::{GoDiscoveryError, GoRequirements, GoVersion},
    lock::{self, Artifact, Declaration, Document, Format, Installation, Lock, Platform, Tool},
    node_discovery::UniqueJson,
};

pub const INDEX_PATH: &str = "/dl/?mode=json&include=all";
pub const MAX_INDEX_BYTES: usize = 8 * 1024 * 1024;
const MAX_RELEASES: usize = 4096;
const MAX_RELEASE_FILES: usize = 256;
const METADATA_ORIGIN: &str = "https://go.dev";
// go.dev/dl redirects here; downloads never follow redirects.
pub(crate) const ARTIFACT_ORIGIN: &str = "https://dl.google.com";
// Same qualified artifact set as Node: no official musl claim.
pub(crate) const PLATFORMS: [Platform; 6] = crate::node_resolution::PLATFORMS;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Discovery(#[from] GoDiscoveryError),
    #[error("Go release index exceeds its byte or entry limit")]
    Limit,
    #[error("invalid or ambiguous Go release index")]
    Index,
    #[error("selected Go release lists {0} without a SHA-256 checksum")]
    MissingChecksum(String),
    #[error("selected Go release has no archive for any supported platform")]
    NoArtifacts,
    #[error(transparent)]
    Lock(#[from] lock::Error),
    #[error(transparent)]
    Download(#[from] turborepo_download::Error),
    #[error(transparent)]
    Storage(#[from] lock::StorageError),
    #[error(
        "locked Go no longer satisfies the root Go declarations or official layout; run `turbo \
         setup --update-lock`"
    )]
    Stale,
}

/// Complete a reconcile candidate with Go. Changed or refreshed declarations
/// resolve; otherwise the exact previous selection is kept after checking that
/// it still satisfies them and the official per-platform layout.
pub async fn complete(
    snapshot: &lock::Snapshot,
    resolve: bool,
    cohort: Lock,
    transport: &GoTransport,
) -> Result<Lock, Error> {
    let mut tools = cohort.tools().clone();
    if let Some(requirements) = snapshot.go_requirements()? {
        let tool = if resolve {
            transport.resolve(&requirements).await?
        } else {
            let previous = snapshot.previous_lock().ok_or(Error::Stale)?;
            let tool = previous.tools().get("go").ok_or(Error::Stale)?;
            let Installation::Managed { artifacts } = &tool.installation else {
                return Err(Error::Stale);
            };
            for platform in artifacts.keys() {
                crate::go_provision::GoPlan::from_lock(previous, *platform)
                    .map_err(|_| Error::Stale)?;
            }
            if !GoVersion::parse(&tool.version).is_some_and(|v| requirements.matches_locked(v)) {
                return Err(Error::Stale);
            }
            tool.clone()
        };
        tools.insert("go".into(), tool);
    }
    snapshot.ensure_current()?;
    Ok(Lock::new(Document {
        schema_version: lock::SCHEMA_VERSION,
        tools,
    })?)
}

/// Official archive name for a qualified lock platform.
pub(crate) fn archive(version: GoVersion, platform: Platform) -> Option<(String, Format)> {
    let (os, arch) = match platform {
        Platform::MacosX64 => ("darwin", "amd64"),
        Platform::MacosArm64 => ("darwin", "arm64"),
        Platform::LinuxX64Gnu => ("linux", "amd64"),
        Platform::LinuxArm64Gnu => ("linux", "arm64"),
        Platform::WindowsX64 => ("windows", "amd64"),
        Platform::WindowsArm64 => ("windows", "arm64"),
        _ => return None,
    };
    let (extension, format) = if os == "windows" {
        ("zip", Format::Zip)
    } else {
        ("tar.gz", Format::TarGz)
    };
    Some((format!("go{version}.{os}-{arch}.{extension}"), format))
}

/// Executables exposed from the distribution's `go/` root.
pub(crate) fn executables(format: Format) -> BTreeMap<String, String> {
    let suffix = if format == Format::Zip { ".exe" } else { "" };
    ["go", "gofmt"]
        .into_iter()
        .map(|name| (name.to_owned(), format!("bin/{name}{suffix}")))
        .collect()
}

/// Expected archive filename -> raw digest, per selectable release. Historical
/// spellings (`go1`, patch candidates) and other files are never selectable.
pub struct ReleaseIndex(BTreeMap<GoVersion, BTreeMap<String, String>>);

impl ReleaseIndex {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_INDEX_BYTES {
            return Err(Error::Limit);
        }
        let UniqueJson(value) = serde_json::from_slice(bytes).map_err(|_| Error::Index)?;
        let entries = value.as_array().ok_or(Error::Index)?;
        if entries.is_empty() || entries.len() > MAX_RELEASES {
            return Err(Error::Limit);
        }
        fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, Error> {
            value.get(key).and_then(Value::as_str).ok_or(Error::Index)
        }
        let mut releases = BTreeMap::new();
        for entry in entries {
            let raw = text(entry, "version")?;
            let files = entry
                .get("files")
                .and_then(Value::as_array)
                .ok_or(Error::Index)?;
            if files.len() > MAX_RELEASE_FILES {
                return Err(Error::Limit);
            }
            let Some(version) = raw
                .strip_prefix("go")
                .and_then(GoVersion::parse)
                .filter(|v| v.is_toolchain() && raw == format!("go{v}"))
            else {
                continue;
            };
            let expected: BTreeMap<_, _> = PLATFORMS
                .iter()
                .filter_map(|platform| archive(version, *platform))
                .collect();
            let mut archives = BTreeMap::new();
            for file in files {
                let name = text(file, "filename")?;
                if !expected.contains_key(name) {
                    continue;
                }
                let (os, arch) = name
                    .strip_prefix(&format!("{raw}."))
                    .and_then(|rest| rest.split('.').next()?.split_once('-'))
                    .ok_or(Error::Index)?;
                if text(file, "version")? != raw
                    || text(file, "kind")? != "archive"
                    || text(file, "os")? != os
                    || text(file, "arch")? != arch
                    || archives
                        .insert(name.to_owned(), text(file, "sha256")?.to_owned())
                        .is_some()
                {
                    return Err(Error::Index);
                }
            }
            if releases.insert(version, archives).is_some() {
                return Err(Error::Index);
            }
        }
        Ok(Self(releases))
    }

    pub fn versions(&self) -> Vec<GoVersion> {
        self.0.keys().copied().collect()
    }
}

/// Select one release and bind every published qualified archive of that
/// SAME release with its official SHA-256. Unpublished targets are omitted,
/// never substituted with another release.
pub fn resolve(requirements: &GoRequirements, index: &[u8]) -> Result<Tool, Error> {
    let index = ReleaseIndex::parse(index)?;
    let version = requirements.select(&index.versions())?;
    let published = &index.0[&version];
    let mut artifacts = BTreeMap::new();
    for platform in PLATFORMS {
        let Some((name, format)) = archive(version, platform) else {
            continue;
        };
        let Some(sha256) = published.get(&name) else {
            continue;
        };
        if sha256.len() != 64
            || !sha256
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(Error::MissingChecksum(name));
        }
        artifacts.insert(
            platform,
            BTreeMap::from([(
                "distribution".to_owned(),
                Artifact {
                    url: format!("{ARTIFACT_ORIGIN}/go/{name}"),
                    sha256: sha256.clone(),
                    format,
                    alternates: BTreeMap::new(),
                    root_prefix: Some("go".into()),
                    destination: None,
                    executables: executables(format),
                },
            )]),
        );
    }
    if artifacts.is_empty() {
        return Err(Error::NoArtifacts);
    }
    let tool = Tool {
        adapter: "go".into(),
        version: version.to_string(),
        declarations: requirements
            .sources()
            .iter()
            .map(|source| Declaration {
                file: source.file.into(),
                field: Some(source.field.into()),
                request: source.request.clone(),
            })
            .collect(),
        options: BTreeMap::new(),
        installation: Installation::Managed { artifacts },
    };
    Lock::new(Document {
        schema_version: lock::SCHEMA_VERSION,
        tools: BTreeMap::from([("go".into(), tool.clone())]),
    })?;
    Ok(tool)
}

pub struct GoTransport {
    pub(crate) client: DownloadClient,
    // Test transports change only origins; lock provenance stays official.
    metadata: String,
    pub(crate) artifacts: String,
}

impl GoTransport {
    pub fn official(_policy: &crate::source_policy::OfficialSourcePolicy) -> Result<Self, Error> {
        Ok(Self {
            client: DownloadClient::new([
                ApprovedOrigin::https(METADATA_ORIGIN)?,
                ApprovedOrigin::https(ARTIFACT_ORIGIN)?,
            ])?,
            metadata: METADATA_ORIGIN.into(),
            artifacts: ARTIFACT_ORIGIN.into(),
        })
    }

    /// Explicit fixture-only HTTP opt-in; literal loopback IPs only.
    #[cfg(any(test, feature = "test-support"))]
    pub fn loopback_http_for_tests(origin: &str) -> Result<Self, Error> {
        let origin = origin.trim_end_matches('/');
        Ok(Self {
            client: DownloadClient::loopback_http_for_tests(origin)?,
            metadata: origin.into(),
            artifacts: origin.into(),
        })
    }

    /// Read only bounded official metadata. No caller URL, artifact download,
    /// installation, or executable probing.
    pub async fn resolve(&self, requirements: &GoRequirements) -> Result<Tool, Error> {
        let index = self
            .client
            .read_metadata(
                &format!("{}{INDEX_PATH}", self.metadata),
                Limits::new(MAX_INDEX_BYTES, Duration::from_secs(30))?,
            )
            .await?;
        resolve(requirements, &index)
    }
}

#[cfg(test)]
mod tests;
