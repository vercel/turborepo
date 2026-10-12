//! Exact npm/pnpm → verified portable artifact. No installation, execution,
//! repository I/O or platform-support claim. The coordinator owns declarations,
//! Node binding and final lock validation/publication.

use std::collections::BTreeMap;

use semver::Version;

pub use crate::registry_provision::{Error, RegistryTransport};
use crate::{
    lock::{Artifact, Platform},
    package_manager::{CorepackIntegrity, Manager},
    registry_metadata::{RegistryArtifact, RegistryMetadataError},
};

/// Constructed only after mandatory SHA-512 byte verification and safe package
/// inspection. No download bytes or private extraction scratch escape
/// resolution.
#[derive(Debug)]
pub struct ResolvedRegistry {
    manager: Manager,
    version: Version,
    artifact: Artifact,
    integrity: CorepackIntegrity,
    natives: BTreeMap<Platform, Artifact>,
}
impl ResolvedRegistry {
    pub(crate) fn new(
        release: RegistryArtifact,
        artifact: Artifact,
        natives: BTreeMap<Platform, Artifact>,
    ) -> Self {
        Self {
            manager: release.manager,
            version: release.version,
            artifact,
            integrity: release.integrity,
            natives,
        }
    }
    pub fn manager(&self) -> Manager {
        self.manager
    }
    pub fn version(&self) -> &Version {
        &self.version
    }
    /// The registry package itself, which authored integrity pins describe.
    pub fn artifact(&self) -> &Artifact {
        &self.artifact
    }
    /// Registry SHA-512 hex digest verified against the actual downloaded
    /// bytes; digest equality pins bytes, not independent publisher
    /// authentication.
    pub fn integrity(&self) -> &CorepackIntegrity {
        &self.integrity
    }
    /// Check a native pin selected AFTER byte-derived SHA-256 is available via
    /// Declaration::locked_integrity. No refetch, metadata-only proof or OR
    /// flattening.
    pub fn verify_authored(&self, authored: Option<&CorepackIntegrity>) -> Result<(), Error> {
        let Some(pin) = authored else { return Ok(()) };
        let (length, actual) = match pin.algorithm {
            "sha256" => (64, &self.artifact.sha256),
            "sha512" => (128, &self.integrity.digest),
            _ => return Err(RegistryMetadataError::InvalidIntegrity.into()),
        };
        if pin.digest.len() != length || !pin.digest.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(RegistryMetadataError::InvalidIntegrity.into());
        }
        if !pin.digest.eq_ignore_ascii_case(actual) {
            return Err(if pin.algorithm == "sha512" {
                RegistryMetadataError::IntegrityConflict.into()
            } else {
                turborepo_download::Error::DigestMismatch.into()
            });
        }
        Ok(())
    }
    pub fn into_artifact(self) -> Artifact {
        self.artifact
    }
    /// Portable lock artifacts: the package for any platform, or, for native
    /// pnpm, each platform's verified binary package beside the same package.
    pub fn into_artifacts(self) -> BTreeMap<Platform, BTreeMap<String, Artifact>> {
        if self.natives.is_empty() {
            return BTreeMap::from([(
                Platform::Any,
                BTreeMap::from([("package".into(), self.artifact)]),
            )]);
        }
        let package = self.artifact;
        self.natives
            .into_iter()
            .map(|(platform, native)| {
                let parts = [
                    ("native".into(), native),
                    ("package".into(), package.clone()),
                ];
                (platform, BTreeMap::from(parts))
            })
            .collect()
    }
}

#[cfg(all(test, unix))]
mod tests;
