use base64::{Engine, engine::general_purpose::STANDARD};
use semver::Version;
use serde::{
    Deserialize, Deserializer,
    de::{MapAccess, Visitor, value::MapAccessDeserializer},
};
use thiserror::Error;

use crate::{
    package_manager::{CorepackIntegrity, Manager},
    version_request::is_valid_release,
};

pub(crate) const PUBLIC_NPM_REGISTRY: &str = "https://registry.npmjs.org";
pub const MAX_METADATA_BYTES: usize = 256 * 1024;
const MAX_VERSION_BYTES: usize = 256;

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum RegistryMetadataError {
    #[error("registry metadata byte limit exceeded")]
    TooLarge,
    #[error("registry metadata supports only npm, pnpm, and Yarn")]
    UnsupportedManager,
    #[error("an exact canonical published version is required")]
    InvalidVersion,
    #[error("invalid registry release metadata")]
    InvalidMetadata,
    #[error("registry package or version does not match the selected release")]
    IdentityMismatch,
    #[error("registry tarball URL must match the canonical selected artifact")]
    InvalidTarball,
    #[error("canonical SHA-512 SRI and valid authored integrity are required")]
    InvalidIntegrity,
    #[error("authored SHA-512 integrity does not match registry metadata")]
    IntegrityConflict,
}

// Metadata only, not verified bytes or publisher authorization. After approving
// the source, callers require the shared SHA-512 verifier and additional pin
// below before deriving a SHA-256 lock identity. No transport/install wrapper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryArtifact {
    pub manager: Manager,
    pub version: Version,
    pub tarball: String,
    pub integrity: CorepackIntegrity,
    /// Authored SHA-256 or SHA-224 that registry SHA-512 cannot prove. Callers
    /// must verify it against the downloaded bytes.
    pub byte_pin: Option<CorepackIntegrity>,
}

#[derive(Deserialize)]
struct ReleaseDocument {
    name: String,
    version: String,
    #[serde(deserialize_with = "deserialize_object")]
    dist: Distribution,
}

#[derive(Deserialize)]
struct Distribution {
    tarball: String,
    integrity: String,
}

fn deserialize_object<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<T, D::Error> {
    struct Object<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> Visitor<'de> for Object<T> {
        type Value = T;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("an object")
        }
        fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<T, A::Error> {
            T::deserialize(MapAccessDeserializer::new(map))
        }
    }
    deserializer.deserialize_map(Object(std::marker::PhantomData))
}

pub fn parse_release(
    bytes: &[u8],
    manager: Manager,
    exact_version: &str,
    authored: Option<&CorepackIntegrity>,
) -> Result<RegistryArtifact, RegistryMetadataError> {
    parse_release_from(bytes, manager, exact_version, authored, None)
}

/// A mirror may point `dist.tarball` at its own canonical path. The returned
/// tarball is always the public registry URL, so locks never record mirrors.
pub(crate) fn parse_release_from(
    bytes: &[u8],
    manager: Manager,
    exact_version: &str,
    authored: Option<&CorepackIntegrity>,
    mirror: Option<&str>,
) -> Result<RegistryArtifact, RegistryMetadataError> {
    if bytes.len() > MAX_METADATA_BYTES {
        return Err(RegistryMetadataError::TooLarge);
    }
    let (name, _) = validate_selection(manager, exact_version)?;
    parse_package_from(bytes, manager, name, exact_version, authored, mirror)
}

/// Like `parse_release_from`, for an adapter-owned registry package name such
/// as pnpm's `@pnpm/exe.<target>` platform binaries.
pub(crate) fn parse_package_from(
    bytes: &[u8],
    manager: Manager,
    name: &str,
    exact_version: &str,
    authored: Option<&CorepackIntegrity>,
    mirror: Option<&str>,
) -> Result<RegistryArtifact, RegistryMetadataError> {
    if bytes.len() > MAX_METADATA_BYTES {
        return Err(RegistryMetadataError::TooLarge);
    }
    let (_, version) = validate_selection(manager, exact_version)?;
    // Typed deserialization rejects duplicate security-relevant fields; ignore
    // unrelated upstream enrichment within the bounded complete JSON document.
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let document: ReleaseDocument = deserialize_object(&mut deserializer)
        .map_err(|_| RegistryMetadataError::InvalidMetadata)?;
    deserializer
        .end()
        .map_err(|_| RegistryMetadataError::InvalidMetadata)?;
    if document.name != name || document.version != exact_version {
        return Err(RegistryMetadataError::IdentityMismatch);
    }
    // Compare the literal decoded string, not a URL parser's normalized form.
    let path = tarball_path(name, exact_version);
    let tarball = format!("{PUBLIC_NPM_REGISTRY}/{path}");
    let mirrored = mirror.map(|base| format!("{base}/{path}"));
    if document.dist.tarball != tarball && Some(&document.dist.tarball) != mirrored.as_ref() {
        return Err(RegistryMetadataError::InvalidTarball);
    }
    let integrity = sri_sha512(&document.dist.integrity)?;
    let byte_pin = match authored {
        None => None,
        Some(pin) => {
            let length = match pin.algorithm {
                "sha512" => 128,
                "sha256" => 64,
                "sha224" => 56,
                _ => return Err(RegistryMetadataError::InvalidIntegrity),
            };
            if pin.digest.len() != length || !pin.digest.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(RegistryMetadataError::InvalidIntegrity);
            }
            let pin = CorepackIntegrity {
                algorithm: pin.algorithm,
                digest: pin.digest.to_ascii_lowercase(),
            };
            if pin.algorithm == "sha512" {
                if pin != integrity {
                    return Err(RegistryMetadataError::IntegrityConflict);
                }
                None
            } else {
                // Registry SHA-512 cannot prove a SHA-256/SHA-224 pin. Preserve
                // it for mandatory verification against actual artifact bytes.
                Some(pin)
            }
        }
    };
    Ok(RegistryArtifact {
        manager,
        version,
        tarball,
        integrity,
        byte_pin,
    })
}

/// Registry package name for an exact selection, validated before traffic.
/// Like Corepack, Yarn 2+ is the single-file `@yarnpkg/cli-dist` bundle.
pub(crate) fn validate_selection(
    manager: Manager,
    exact_version: &str,
) -> Result<(&'static str, Version), RegistryMetadataError> {
    if !matches!(manager, Manager::Npm | Manager::Pnpm | Manager::Yarn) {
        return Err(RegistryMetadataError::UnsupportedManager);
    }
    if exact_version.len() > MAX_VERSION_BYTES {
        return Err(RegistryMetadataError::InvalidVersion);
    }
    let version =
        Version::parse(exact_version).map_err(|_| RegistryMetadataError::InvalidVersion)?;
    if !is_valid_release(&version) || version.to_string() != exact_version {
        return Err(RegistryMetadataError::InvalidVersion);
    }
    let name = match manager {
        Manager::Npm => "npm",
        Manager::Pnpm => "pnpm",
        _ if is_modern_yarn(manager, &version) => "@yarnpkg/cli-dist",
        _ => "yarn",
    };
    Ok((name, version))
}

/// Modern Yarn authored integrity covers `bin/yarn.js`, not the tarball.
pub(crate) fn is_modern_yarn(manager: Manager, version: &Version) -> bool {
    manager == Manager::Yarn && version.major >= 2
}

/// `{name}/-/{basename}-{version}.tgz`; scoped packages drop the scope.
pub(crate) fn tarball_path(name: &str, version: &str) -> String {
    let base = name.rsplit_once('/').map_or(name, |(_, base)| base);
    format!("{name}/-/{base}-{version}.tgz")
}

fn sri_sha512(value: &str) -> Result<CorepackIntegrity, RegistryMetadataError> {
    let encoded = value
        .strip_prefix("sha512-")
        .ok_or(RegistryMetadataError::InvalidIntegrity)?;
    if encoded.len() != 88 {
        return Err(RegistryMetadataError::InvalidIntegrity);
    }
    let digest = STANDARD
        .decode(encoded)
        .map_err(|_| RegistryMetadataError::InvalidIntegrity)?;
    if digest.len() != 64 || STANDARD.encode(&digest) != encoded {
        return Err(RegistryMetadataError::InvalidIntegrity);
    }
    Ok(CorepackIntegrity {
        algorithm: "sha512",
        digest: hex::encode(digest),
    })
}

#[cfg(test)]
mod tests;
