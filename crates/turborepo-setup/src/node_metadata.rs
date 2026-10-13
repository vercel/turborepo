//! Pure bounded parsing of injected official Node release metadata. The caller
//! must obtain it from an approved source; parsing does not authenticate a
//! publisher or download artifacts. No lock, filesystem, or execution effects.

use std::collections::{BTreeMap, BTreeSet};

use semver::Version;
use serde_json::Value;
use turborepo_platform::{Architecture, OperatingSystem, Platform};

use crate::{
    NodeArtifact, NodeRelease,
    node_discovery::{MAX_RELEASES, UniqueJson},
};

pub const INDEX_URL: &str = "https://nodejs.org/dist/index.json";
pub const MAX_INDEX_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_CHECKSUM_BYTES: usize = 128 * 1024;
const MAX_CHECKSUM_ENTRIES: usize = 1024;
const MAX_RELEASE_FILES: usize = 64;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("Node release metadata exceeds its byte or entry limit")]
    Limit,
    #[error("invalid or ambiguous Node release index")]
    Index,
    #[error("invalid or ambiguous Node SHA-256 manifest")]
    Checksums,
    #[error("selected Node release is missing from the release index")]
    MissingRelease,
    #[error("no standard Node artifact for this OS or architecture")]
    UnsupportedTarget,
    #[error("advertised Node artifact is missing its SHA-256 checksum")]
    MissingChecksum,
}

#[derive(Clone, Debug)]
pub struct PublishedRelease {
    release: NodeRelease,
    bundled_npm: Option<Version>,
    files: BTreeSet<String>,
}
impl PublishedRelease {
    pub fn bundled_npm(&self) -> Option<&Version> {
        self.bundled_npm.as_ref()
    }
}

/// Ordered exact versions, independent of upstream entry order.
#[derive(Clone, Debug)]
pub struct ReleaseIndex(BTreeMap<Version, PublishedRelease>);
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
        let mut releases = BTreeMap::new();
        for entry in entries {
            let entry = entry.as_object().ok_or(Error::Index)?;
            let raw = entry
                .get("version")
                .and_then(Value::as_str)
                .ok_or(Error::Index)?;
            let version = raw
                .strip_prefix('v')
                .ok_or(Error::Index)
                .and_then(exact_version)?;
            let lts = match entry.get("lts") {
                None | Some(Value::Null | Value::Bool(false)) => None,
                Some(Value::String(name)) => Some(name.as_str()),
                _ => return Err(Error::Index),
            };
            let release = NodeRelease::new(&version.to_string(), lts).map_err(|_| Error::Index)?;
            let bundled_npm = match entry.get("npm") {
                None | Some(Value::Null) => None,
                Some(Value::String(value)) if value.is_empty() => None,
                Some(Value::String(value)) => Some(exact_version(value)?),
                _ => return Err(Error::Index),
            };
            let files = entry
                .get("files")
                .and_then(Value::as_array)
                .ok_or(Error::Index)?;
            if files.len() > MAX_RELEASE_FILES {
                return Err(Error::Limit);
            }
            let mut known = BTreeSet::new();
            for file in files {
                let file = file.as_str().ok_or(Error::Index)?;
                if file.is_empty()
                    || file.len() > 64
                    || !file
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
                    || !known.insert(file.to_owned())
                {
                    return Err(Error::Index);
                }
            }
            // Unknown upstream fields (e.g. V8/security) do not influence tool identity.
            if releases
                .insert(
                    version,
                    PublishedRelease {
                        release,
                        bundled_npm,
                        files: known,
                    },
                )
                .is_some()
            {
                return Err(Error::Index);
            }
        }
        Ok(Self(releases))
    }

    pub fn releases(&self) -> Vec<NodeRelease> {
        self.0.values().map(|r| r.release.clone()).collect()
    }
    pub fn release(&self, version: &Version) -> Result<&PublishedRelease, Error> {
        self.0.get(version).ok_or(Error::MissingRelease)
    }

    /// Unavailable vendor targets return None, never a different version.
    /// An advertised target without matching checksum is a fatal metadata
    /// error.
    pub fn artifact(
        &self,
        version: &Version,
        platform: Platform,
        checksums: &Checksums,
    ) -> Result<Option<ArtifactMetadata>, Error> {
        let published = self.release(version)?;
        let label = match (platform.os(), platform.arch()) {
            (OperatingSystem::Macos, Architecture::X64) => "osx-x64-tar",
            (OperatingSystem::Macos, Architecture::Arm64) => "osx-arm64-tar",
            (OperatingSystem::Linux, Architecture::X64) => "linux-x64",
            (OperatingSystem::Linux, Architecture::Arm64) => "linux-arm64",
            (OperatingSystem::Windows, Architecture::X64) => "win-x64-zip",
            (OperatingSystem::Windows, Architecture::Arm64) => "win-arm64-zip",
            _ => return Err(Error::UnsupportedTarget),
        };
        if !published.files.contains(label) {
            return Ok(None);
        }
        let artifact =
            NodeArtifact::for_platform(version, platform).map_err(|_| Error::UnsupportedTarget)?;
        let sha256 = checksums
            .0
            .get(artifact.filename())
            .ok_or(Error::MissingChecksum)?
            .clone();
        Ok(Some(ArtifactMetadata { artifact, sha256 }))
    }
}

#[derive(Clone, Debug)]
pub struct ArtifactMetadata {
    artifact: NodeArtifact,
    sha256: String,
}
impl ArtifactMetadata {
    pub fn artifact(&self) -> &NodeArtifact {
        &self.artifact
    }
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}

/// Exact filename lookup. Other safe upstream paths (e.g. win-x64/node.exe)
/// are retained but never interpreted as a download or filesystem destination.
#[derive(Clone, Debug)]
pub struct Checksums(BTreeMap<String, String>);
impl Checksums {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_CHECKSUM_BYTES {
            return Err(Error::Limit);
        }
        let text = std::str::from_utf8(bytes).map_err(|_| Error::Checksums)?;
        let mut checksums = BTreeMap::new();
        for line in text.lines() {
            if line.is_empty() {
                continue;
            }
            let mut fields = line.split_ascii_whitespace();
            let digest = fields.next().ok_or(Error::Checksums)?;
            let name = fields.next().ok_or(Error::Checksums)?;
            // GNU sha256sum's binary mode marks the filename with '*'.
            let name = name.strip_prefix('*').unwrap_or(name);
            if fields.next().is_some()
                || digest.len() != 64
                || !digest.bytes().all(|b| b.is_ascii_hexdigit())
                || !checksum_path(name)
            {
                return Err(Error::Checksums);
            }
            if checksums
                .insert(name.to_owned(), digest.to_ascii_lowercase())
                .is_some()
            {
                return Err(Error::Checksums);
            }
            if checksums.len() > MAX_CHECKSUM_ENTRIES {
                return Err(Error::Limit);
            }
        }
        if checksums.is_empty() {
            return Err(Error::Checksums);
        }
        Ok(Self(checksums))
    }
}

fn exact_version(value: &str) -> Result<Version, Error> {
    if value.len() > 128 {
        return Err(Error::Index);
    }
    let version = crate::VersionRequest::parse(value).map_err(|_| Error::Index)?;
    version
        .exact_version()
        .filter(|v| v.to_string() == value)
        .cloned()
        .ok_or(Error::Index)
}
fn checksum_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value.split('/').count() <= 4
        && value.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-_+".contains(&b))
        })
}

#[cfg(test)]
mod tests;
