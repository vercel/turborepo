//! Exact npm/pnpm/Yarn → verified portable artifact. No installation,
//! execution, repository I/O or platform-support claim. The coordinator owns
//! declarations, Node binding and final lock validation/publication.

use semver::Version;

pub use crate::registry_provision::{Error, RegistryTransport};
use crate::{
    lock::Artifact,
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
    /// SHA-224 of the verified archive bytes, for older Corepack pins.
    sha224: String,
    /// Modern Yarn `bin/yarn.js` SHA-256/SHA-512, which Corepack pins instead
    /// of the registry tarball.
    bundle: Option<(String, String)>,
}
impl ResolvedRegistry {
    pub(crate) fn new(
        release: RegistryArtifact,
        artifact: Artifact,
        sha224: String,
        bundle: Option<(String, String)>,
    ) -> Self {
        Self {
            manager: release.manager,
            version: release.version,
            artifact,
            integrity: release.integrity,
            sha224,
            bundle,
        }
    }
    pub fn manager(&self) -> Manager {
        self.manager
    }
    pub fn version(&self) -> &Version {
        &self.version
    }
    pub fn artifact(&self) -> &Artifact {
        &self.artifact
    }
    /// Registry SHA-512 hex digest verified against the actual downloaded
    /// bytes; digest equality pins bytes, not independent publisher
    /// authentication.
    pub fn integrity(&self) -> &CorepackIntegrity {
        &self.integrity
    }
    /// SHA-256 covered by authored pins: the bundle for modern Yarn, otherwise
    /// the locked archive. Pass this to Declaration::locked_integrity.
    pub fn pinned_sha256(&self) -> &str {
        self.bundle
            .as_ref()
            .map_or(&self.artifact.sha256, |(sha256, _)| sha256)
    }
    /// Check a native pin selected AFTER byte-derived SHA-256 is available via
    /// Declaration::locked_integrity. No refetch, metadata-only proof or OR
    /// flattening.
    pub fn verify_authored(&self, authored: Option<&CorepackIntegrity>) -> Result<(), Error> {
        let Some(pin) = authored else { return Ok(()) };
        match &self.bundle {
            // Bundle pins have no SHA-224 form; Corepack's SHA-224 pins name
            // registry archives.
            Some((sha256, sha512)) => check_pin(pin, None, sha256, sha512),
            None => check_pin(
                pin,
                Some(&self.sha224),
                &self.artifact.sha256,
                &self.integrity.digest,
            ),
        }
    }
    pub fn into_artifact(self) -> Artifact {
        self.artifact
    }
}

pub(crate) fn check_pin(
    pin: &CorepackIntegrity,
    sha224: Option<&str>,
    sha256: &str,
    sha512: &str,
) -> Result<(), Error> {
    let (length, actual) = match (pin.algorithm, sha224) {
        ("sha224", Some(sha224)) => (56, sha224),
        ("sha256", _) => (64, sha256),
        ("sha512", _) => (128, sha512),
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

#[cfg(all(test, unix))]
mod tests;
