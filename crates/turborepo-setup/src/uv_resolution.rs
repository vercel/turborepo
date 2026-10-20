//! uv requirement → portable `turbo.lock` selection. The highest published
//! release satisfying the root declaration wins; every official target in the
//! release's own `sha256.sum` is bound to that SAME release. No installation,
//! repository I/O, Python provisioning or dependency sync.

use std::{collections::BTreeMap, sync::Mutex, time::Duration};

use semver::Version;
use turborepo_download::{ApprovedOrigin, DownloadClient, Limits};

use crate::{
    lock::{self, Document, Installation, Lock, Tool},
    uv::{self, UvArtifact},
    uv_discovery::UvRequirement,
    uv_metadata::{self, MAX_CHECKSUM_BYTES, MAX_INDEX_BYTES, ReleaseIndex},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Metadata(#[from] uv_metadata::Error),
    #[error(transparent)]
    Lock(#[from] lock::Error),
    #[error(transparent)]
    Download(#[from] turborepo_download::Error),
    #[error("no published uv release satisfies {file} {field} = {request:?}")]
    NoMatchingRelease {
        file: &'static str,
        field: &'static str,
        request: String,
    },
    #[error("uv {0} checksums disagree with the release index; retry later or pin another release")]
    ChecksumConflict(Version),
    #[error(
        "uv {0} publishes no checksums for supported targets; pin a release with a published \
         sha256.sum"
    )]
    MissingChecksums(Version),
}

/// Highest exact release satisfying the authored request.
pub fn select(requirement: &UvRequirement, index: &ReleaseIndex) -> Result<Version, Error> {
    index
        .versions()
        .rev()
        .find(|version| requirement.matches(version))
        .cloned()
        .ok_or_else(|| {
            let source = requirement.source();
            Error::NoMatchingRelease {
                file: source.file,
                field: source.field,
                request: source.request.clone(),
            }
        })
}

/// Bind every published supported target of `version`. Targets missing from
/// the checksum file are omitted, never substituted with another release.
pub fn resolve(
    requirement: &UvRequirement,
    index: &ReleaseIndex,
    version: &Version,
    checksums: &[u8],
) -> Result<Tool, Error> {
    if !requirement.matches(version) || uv::exact_release(&version.to_string()).is_none() {
        return Err(lock::Error::Invalid("selected uv release contradicts its request").into());
    }
    let checksums = uv_metadata::checksums(checksums)?;
    let mut artifacts = BTreeMap::new();
    for platform in uv::PLATFORMS {
        let Some(official) = UvArtifact::for_platform(version, platform) else {
            continue;
        };
        let Some(sha256) = checksums.sha256(&official.filename()) else {
            continue;
        };
        if index
            .sha256(version, official.target())
            .is_some_and(|indexed| indexed != sha256)
        {
            return Err(Error::ChecksumConflict(version.clone()));
        }
        artifacts.insert(
            platform,
            BTreeMap::from([("distribution".into(), official.lock_artifact(sha256.into()))]),
        );
    }
    if artifacts.is_empty() {
        return Err(Error::MissingChecksums(version.clone()));
    }
    let tool = Tool {
        adapter: "uv".into(),
        version: version.to_string(),
        declarations: vec![requirement.declaration()],
        options: BTreeMap::new(),
        installation: Installation::Managed { artifacts },
    };
    Lock::new(Document {
        schema_version: lock::SCHEMA_VERSION,
        tools: BTreeMap::from([("uv".into(), tool.clone())]),
    })?;
    Ok(tool)
}

/// Network-free checks for a preserved selection: the release still satisfies
/// the request, and every recorded target is that release's official archive.
pub fn validate(tool: &Tool, requirement: &UvRequirement) -> Result<(), Error> {
    let invalid = || lock::Error::Invalid("locked uv contradicts its request or official layout");
    let version = uv::exact_release(&tool.version).ok_or_else(invalid)?;
    let Installation::Managed { artifacts } = &tool.installation else {
        return Err(invalid().into());
    };
    if tool.adapter != "uv" || !tool.options.is_empty() || !requirement.matches(&version) {
        return Err(invalid().into());
    }
    for (platform, parts) in artifacts {
        let official = UvArtifact::for_platform(&version, *platform).ok_or_else(invalid)?;
        match parts.get("distribution") {
            Some(artifact)
                if parts.len() == 1
                    && *artifact == official.lock_artifact(artifact.sha256.clone()) => {}
            _ => return Err(invalid().into()),
        }
    }
    Ok(())
}

pub struct UvTransport {
    pub(crate) client: DownloadClient,
    // Test transport changes only the origin; lock provenance remains official.
    pub(crate) origin: String,
    // Archive bytes kept by Python resolution's listing download so the same
    // setup run provisions that uv without downloading it again. Taken once
    // and re-verified against the locked digest before use.
    pub(crate) retained: Mutex<Option<Vec<u8>>>,
}

impl UvTransport {
    pub fn official(_policy: &crate::source_policy::ToolDownloadPolicy) -> Result<Self, Error> {
        Ok(Self {
            client: DownloadClient::new([ApprovedOrigin::https(uv::ORIGIN)?])?,
            origin: uv::ORIGIN.into(),
            retained: Mutex::default(),
        })
    }

    /// Send the official index and release paths to a validated mirror base.
    pub fn mirrored(
        _policy: &crate::source_policy::ToolDownloadPolicy,
        mirror: &crate::mirror::Mirror,
    ) -> Result<Self, Error> {
        Ok(Self {
            client: mirror.client()?,
            origin: mirror.base().into(),
            retained: Mutex::default(),
        })
    }

    /// Read only bounded official metadata: the version index, then the
    /// selected release's checksum file. No artifact download or execution.
    pub async fn resolve(&self, requirement: &UvRequirement) -> Result<Tool, Error> {
        let read = async |path: &str, limit| {
            self.client
                .read_metadata(
                    &format!("{}{path}", self.origin),
                    Limits::new(limit, Duration::from_secs(30))?,
                )
                .await
        };
        let index = ReleaseIndex::parse(&read(uv::INDEX_PATH, MAX_INDEX_BYTES).await?)?;
        let version = select(requirement, &index)?;
        let path = format!("{}/sha256.sum", uv::release_path(&version));
        let checksums = read(&path, MAX_CHECKSUM_BYTES).await?;
        resolve(requirement, &index, &version, &checksums)
    }

    /// Explicit fixture-only HTTP opt-in; literal loopback IPs only.
    #[cfg(any(test, feature = "test-support"))]
    pub fn loopback_http_for_tests(origin: &str) -> Result<Self, Error> {
        Ok(Self {
            client: DownloadClient::loopback_http_for_tests(origin)?,
            origin: origin.trim_end_matches('/').into(),
            retained: Mutex::default(),
        })
    }
}

#[cfg(test)]
mod tests;
