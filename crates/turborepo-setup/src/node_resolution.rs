//! Pure native Node → portable lock selection bridge. Callers supply approved
//! metadata and compose the returned Node tool with unaffected selections
//! before validating/publishing a complete lock. No fetching, installation, or
//! I/O.

use std::collections::BTreeMap;

use turborepo_platform::OperatingSystem;

use crate::{
    NodeDiscoveryError, NodeRequirements, ResolvedNode, VersionRequest,
    lock::{self, Artifact, Declaration, Document, Format, Installation, Lock, Platform, Tool},
    node::lock_target,
    node_metadata::{self, Checksums, ReleaseIndex},
};

const BUNDLED_NPM: &str = "bundled-npm";
pub(crate) const PLATFORMS: [Platform; 6] = [
    Platform::MacosX64,
    Platform::MacosArm64,
    Platform::LinuxX64Gnu,
    Platform::LinuxArm64Gnu,
    Platform::WindowsX64,
    Platform::WindowsArm64,
];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Discovery(#[from] NodeDiscoveryError),
    #[error(transparent)]
    Metadata(#[from] node_metadata::Error),
    #[error(transparent)]
    Lock(#[from] lock::Error),
    #[error("checksum manifest must identify the selected canonical Node version")]
    ManifestVersion,
    #[error("explicit bundled npm ownership requires exact npm release metadata")]
    MissingBundledNpm,
    #[error("bundled npm/npx ownership requires npm 5.2.0 or later")]
    UnsupportedBundledNpm,
}

/// SHASUMS256.txt bytes plus the exact version of the supplied release
/// manifest. This binding rejects wrong-version input; it does not authenticate
/// its origin.
pub struct ChecksumManifest<'a> {
    pub version: &'a str,
    pub bytes: &'a [u8],
}

/// Only this module constructs a validated Node-only selection. Native source
/// order and selection priority remain available separately from the lock's
/// canonical serialization (which sorts declaration locations).
#[derive(Clone, Debug)]
pub struct ToolResolution {
    tool: Tool,
    native: ResolvedNode,
}
impl ToolResolution {
    pub fn tool(&self) -> &Tool {
        &self.tool
    }
    pub fn native(&self) -> &ResolvedNode {
        &self.native
    }
    pub fn into_tool(self) -> Tool {
        self.tool
    }
}

/// Resolve using existing native alias/range/exact semantics, then bind every
/// advertised standard target to that SAME release and its exact SHA-256 entry.
/// Unadvertised targets are omitted, never substituted with an older release.
/// `include_bundled_npm` deliberately assigns npm/npx ownership to Node and
/// pins the index's npm identity in options. It is NOT an independent npm
/// override: callers must validate the composed Lock to reject executable
/// collisions.
pub fn resolve(
    requirements: &NodeRequirements,
    index_bytes: &[u8],
    manifest: ChecksumManifest<'_>,
    include_bundled_npm: bool,
) -> Result<ToolResolution, Error> {
    let index = ReleaseIndex::parse(index_bytes)?;
    let native = requirements.resolve(&index.releases())?;
    if manifest.version != native.version.to_string() {
        return Err(Error::ManifestVersion);
    }
    let checksums = Checksums::parse(manifest.bytes)?;
    let mut options = BTreeMap::new();
    if include_bundled_npm {
        let npm = index
            .release(&native.version)?
            .bundled_npm()
            .ok_or(Error::MissingBundledNpm)?;
        if !bundled_npm_supported(npm) {
            return Err(Error::UnsupportedBundledNpm);
        }
        options.insert(BUNDLED_NPM.into(), vec![npm.to_string()]);
    }
    let mut artifacts = BTreeMap::new();
    for platform in PLATFORMS {
        let (target, _) = lock_target(platform).ok_or(node_metadata::Error::UnsupportedTarget)?;
        let Some(metadata) = index.artifact(&native.version, target, &checksums)? else {
            continue;
        };
        let official = metadata.artifact();
        let windows = target.os() == OperatingSystem::Windows;
        let extension = if windows { ".zip" } else { ".tar.gz" };
        let prefix = official
            .filename()
            .strip_suffix(extension)
            .ok_or(node_metadata::Error::UnsupportedTarget)?;
        let mut executables = BTreeMap::from([(
            "node".into(),
            if windows { "node.exe" } else { "bin/node" }.into(),
        )]);
        if include_bundled_npm {
            executables.insert(
                "npm".into(),
                if windows { "npm.cmd" } else { "bin/npm" }.into(),
            );
            executables.insert(
                "npx".into(),
                if windows { "npx.cmd" } else { "bin/npx" }.into(),
            );
        }
        artifacts.insert(
            platform,
            BTreeMap::from([(
                "distribution".into(),
                Artifact {
                    url: official.url(),
                    sha256: metadata.sha256().into(),
                    format: if windows { Format::Zip } else { Format::TarGz },
                    root_prefix: Some(prefix.into()),
                    destination: None,
                    executables,
                },
            )]),
        );
    }
    let tool = Tool {
        adapter: "node".into(),
        version: native.version.to_string(),
        declarations: native
            .sources
            .iter()
            .map(|source| Declaration {
                file: source.file.into(),
                field: source.field.clone(),
                request: source.request.clone(),
            })
            .collect(),
        options,
        installation: Installation::Managed { artifacts },
    };
    Lock::new(Document {
        schema_version: lock::SCHEMA_VERSION,
        tools: BTreeMap::from([("node".into(), tool.clone())]),
    })?;
    Ok(ToolResolution { tool, native })
}

// Provisioning accepts only this exact adapter-owned option shape, not opaque
// options or guessed artifactVersion identities. Legacy empty options remain
// accepted; newly resolved bundled ownership always carries an exact identity.
pub(crate) fn valid_bundled_npm_option(options: &BTreeMap<String, Vec<String>>) -> bool {
    if options.is_empty() {
        return true;
    }
    if options.len() != 1 {
        return false;
    }
    let Some(values) = options.get(BUNDLED_NPM) else {
        return false;
    };
    let [value] = values.as_slice() else {
        return false;
    };
    VersionRequest::parse(value).is_ok_and(|request| {
        request
            .exact_version()
            .is_some_and(|v| v.to_string() == *value && bundled_npm_supported(v))
    })
}

fn bundled_npm_supported(version: &semver::Version) -> bool {
    // npx was first bundled in https://github.com/npm/npm/releases/tag/v5.2.0.
    version >= &semver::Version::new(5, 2, 0)
}

#[cfg(test)]
mod tests;
