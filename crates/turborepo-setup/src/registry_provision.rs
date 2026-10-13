//! Shared exact registry verification and locked preparation for npm, pnpm and
//! Yarn. No hooks, probes, Corepack, dependency installation or activation.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::Path,
    time::Duration,
};

use semver::Version;
use sha2::{Digest, Sha224, Sha256, Sha512};
use turborepo_archive::{ExtractedArtifact, Layout};
use turborepo_download::{
    ApprovedOrigin, DownloadClient, ExpectedSha256, ExpectedSha512, Limits, VerifiedArtifact,
};
use turborepo_tool_install::{Store, Tool};

use crate::{
    artifact_cache::ArtifactCache,
    lock::{self, Format, Installation, Lock, Platform},
    mirror::Mirror,
    node_discovery::UniqueJson,
    node_provision::{NodePlan, copy_tree},
    package_manager::{CorepackIntegrity, Manager},
    registry_metadata::{self, MAX_METADATA_BYTES, PUBLIC_NPM_REGISTRY as REGISTRY},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid locked registry artifact or executable mapping; regenerate turbo.lock")]
    InvalidLock,
    #[error("package manager requires the selected managed Node in the complete desired inventory")]
    NodeBinding,
    #[error(
        "registry package promotion is unqualified for this host, Windows arm64 or musl; use \
         macOS, GNU Linux or Windows x64"
    )]
    UnsupportedTarget,
    #[error("package identity or bin entries do not match the selected lock")]
    InvalidPackage,
    #[error("offline setup has no verified cached package artifact and never contacts upstream")]
    Offline,
    #[error(transparent)]
    Authored(#[from] crate::package_manager::Error),
    #[error(transparent)]
    Metadata(#[from] registry_metadata::RegistryMetadataError),
    #[error(transparent)]
    Download(#[from] turborepo_download::Error),
    #[error(transparent)]
    Archive(#[from] turborepo_archive::Error),
    #[error(transparent)]
    Install(#[from] turborepo_tool_install::Error),
    #[error("registry package staging I/O failed")]
    Io(#[from] io::Error),
}

const PACKAGE_LIMIT: usize = 64 * 1024 * 1024;

pub struct RegistryTransport {
    client: DownloadClient,
    origin: String,
    // Mirrors may serve metadata whose tarball points at the mirror itself.
    mirrored: bool,
    cache: Option<ArtifactCache>,
    offline: bool,
}
impl RegistryTransport {
    // TODO(TURBO-6216): Apply repository-configured mirrors. Never fall back
    // to public npm or forward credentials across origins.
    pub fn official(policy: &crate::source_policy::OfficialSourcePolicy) -> Result<Self, Error> {
        Ok(Self {
            client: DownloadClient::new([ApprovedOrigin::https(policy.registry())?])?,
            origin: policy.registry().into(),
            mirrored: false,
            cache: None,
            offline: false,
        })
    }

    /// Send the same registry requests to a validated mirror base only.
    pub fn mirrored(
        _policy: &crate::source_policy::OfficialSourcePolicy,
        mirror: &Mirror,
    ) -> Result<Self, Error> {
        Ok(Self {
            client: mirror.client()?,
            origin: mirror.base().into(),
            mirrored: true,
            cache: None,
            offline: false,
        })
    }

    /// Reuse locked package bytes during locked preparation only; resolution
    /// always verifies registry metadata.
    pub fn with_cache(mut self, cache: Option<ArtifactCache>) -> Self {
        self.cache = cache;
        self
    }

    /// Serve only verified cache hits; metadata and artifact requests fail
    /// before traffic with [`Error::Offline`].
    pub fn offline(mut self) -> Self {
        self.offline = true;
        self
    }

    // A hit is already verified against the locked SHA-256. An authored
    // SHA-256 equals that digest (`locked_integrity`); an authored SHA-512 is
    // verified here. Any failure falls back to the registry path, which owns
    // the canonical error.
    fn cached(
        &self,
        platform: Platform,
        sha256: &str,
        authored: Option<&CorepackIntegrity>,
    ) -> Option<VerifiedArtifact> {
        let bytes = self.cache.as_ref()?.get(platform, sha256, PACKAGE_LIMIT)?;
        match authored {
            None => Some(bytes),
            Some(pin) if pin.algorithm == "sha256" => (pin.digest == sha256).then_some(bytes),
            Some(pin) if pin.algorithm == "sha512" => VerifiedArtifact::verify_bytes_sha512(
                bytes.into_bytes(),
                ExpectedSha512::from_hex(&pin.digest).ok()?,
                None,
            )
            .ok(),
            Some(_) => None,
        }
    }

    /// Resolve only a canonical exact npm/pnpm/Yarn release, verifying registry
    /// SHA-512 and optional authored integrity against bytes before
    /// deriving lock SHA-256. Native pnpm also verifies each platform package
    /// the lock covers. Private bounded scratch is dropped before return; no
    /// repository path or I/O.
    pub async fn resolve_exact(
        &self,
        manager: Manager,
        exact_version: &str,
        authored: Option<&CorepackIntegrity>,
    ) -> Result<crate::registry_resolution::ResolvedRegistry, Error> {
        // A selected version must also fit the portable lock's identity bound.
        if exact_version.len() > 128 {
            return Err(registry_metadata::RegistryMetadataError::InvalidVersion.into());
        }
        // Select the published layout before any traffic.
        let (_, version) = registry_metadata::validate_selection(manager, exact_version)?;
        let bundled = registry_metadata::is_modern_yarn(manager, &version);
        let (release, bytes) = self
            .verified_release(manager, exact_version, authored.filter(|_| !bundled), None)
            .await?;
        let mut natives = BTreeMap::new();
        let mut bundle = None;
        let artifact = if native_release(manager, &version) {
            drop(extract_wrapper(&bytes, exact_version)?);
            // Every platform Node locks, so a new host never mutates the lock.
            for platform in crate::node_resolution::PLATFORMS {
                let (name, binary) = native_package(platform).ok_or(Error::UnsupportedTarget)?;
                let (native, native_bytes) = self
                    .verified_package(manager, name, exact_version, None, None)
                    .await?;
                drop(extract_native(&native_bytes, name, exact_version, binary)?);
                let mappings = standard_mappings(manager, &version, platform)?;
                let artifact = registry_artifact(native.tarball, &native_bytes, mappings);
                natives.insert(platform, artifact);
            }
            registry_artifact(release.tarball.clone(), &bytes, &[])
        } else {
            let artifact = registry_artifact(
                release.tarball.clone(),
                &bytes,
                standard_mappings(manager, &version, Platform::Any)?,
            );
            let tree =
                extract_registry(&bytes, manager, exact_version, &artifact.executables, true)?;
            bundle = bundled.then(|| bundle_digests(&tree)).transpose()?;
            if let (Some((sha256, sha512)), Some(pin)) = (&bundle, authored) {
                crate::registry_resolution::check_pin(pin, None, sha256, sha512)?;
            }
            drop(tree);
            artifact
        };
        Ok(crate::registry_resolution::ResolvedRegistry::new(
            release,
            artifact,
            sha224_hex(&bytes),
            bundle,
            natives,
        ))
    }

    async fn verified_release(
        &self,
        manager: Manager,
        version: &str,
        authored: Option<&CorepackIntegrity>,
        locked_sha256: Option<&str>,
    ) -> Result<(registry_metadata::RegistryArtifact, VerifiedArtifact), Error> {
        // Validate before interpolating any request path or making traffic.
        let (name, _) = registry_metadata::validate_selection(manager, version)?;
        self.verified_package(manager, name, version, authored, locked_sha256)
            .await
    }

    /// `name` is adapter-owned, never authored.
    async fn verified_package(
        &self,
        manager: Manager,
        name: &str,
        version: &str,
        authored: Option<&CorepackIntegrity>,
        locked_sha256: Option<&str>,
    ) -> Result<(registry_metadata::RegistryArtifact, VerifiedArtifact), Error> {
        if self.offline {
            return Err(Error::Offline);
        }
        // Validate before interpolating any request path or making traffic.
        registry_metadata::validate_selection(manager, version)?;
        let metadata = self
            .client
            .read_metadata(
                &format!("{}/{name}/{version}", self.origin),
                Limits::new(MAX_METADATA_BYTES, Duration::from_secs(30))?,
            )
            .await?;
        let release = registry_metadata::parse_package_from(
            &metadata,
            manager,
            name,
            version,
            authored,
            self.mirrored.then_some(self.origin.as_str()),
        )?;
        let additional = release
            .byte_pin
            .as_ref()
            .filter(|pin| pin.algorithm == "sha256")
            .map(|pin| pin.digest.as_str());
        if locked_sha256
            .zip(additional)
            .is_some_and(|(locked, pin)| locked != pin)
        {
            return Err(Error::InvalidLock);
        }
        let sha256 = locked_sha256
            .or(additional)
            .map(ExpectedSha256::from_hex)
            .transpose()?;
        let bytes = self
            .client
            .download_verified_sha512(
                &format!(
                    "{}/{}",
                    self.origin,
                    registry_metadata::tarball_path(name, version)
                ),
                ExpectedSha512::from_hex(&release.integrity.digest)?,
                sha256,
                Limits::new(PACKAGE_LIMIT, Duration::from_secs(120))?,
            )
            .await?;
        // Older Corepack pins: checked in addition to the registry SHA-512.
        if let Some(pin) = release.byte_pin.as_ref()
            && pin.algorithm == "sha224"
            && sha224_hex(&bytes) != pin.digest
        {
            return Err(turborepo_download::Error::DigestMismatch.into());
        }
        Ok((release, bytes))
    }

    /// Loopback fixtures only; canonical provenance, no env override.
    #[cfg(any(all(test, unix), feature = "test-support"))]
    pub fn loopback_http_for_tests(origin: &str) -> Result<Self, Error> {
        Ok(Self {
            client: DownloadClient::loopback_http_for_tests(origin)?,
            origin: origin.trim_end_matches('/').into(),
            mirrored: false,
            cache: None,
            offline: false,
        })
    }
}

type Mappings = &'static [(&'static str, &'static str)];
fn sha224_hex(bytes: &VerifiedArtifact) -> String {
    format!("{:x}", Sha224::digest(bytes.as_bytes()))
}

/// The lock tool id, which is also the primary executable.
fn tool_id(manager: Manager) -> Result<&'static str, Error> {
    match manager {
        Manager::Npm => Ok("npm"),
        Manager::Pnpm => Ok("pnpm"),
        Manager::Yarn => Ok("yarn"),
        _ => Err(Error::InvalidLock),
    }
}

/// pnpm 12 and later run a native binary published as the `@pnpm/exe.<target>`
/// optional dependency of a wrapper whose own bins are placeholders that its
/// install script replaces. Setup runs no scripts: it installs the verified
/// binary where the wrapper keeps one provisioned without them, beside `dist/`
/// (pnpm finds its bundled node-gyp relative to its own executable).
fn native_release(manager: Manager, version: &Version) -> bool {
    manager == Manager::Pnpm && version.major >= 12
}
const NATIVE_BINARY: &str = "pnpm-native";
const NATIVE_NAMES: [&str; 4] = ["pn", "pnpm", "pnpx", "pnx"];
fn native_package(platform: Platform) -> Option<(&'static str, &'static str)> {
    Some(match platform {
        Platform::MacosX64 => ("@pnpm/exe.darwin-x64", "pnpm"),
        Platform::MacosArm64 => ("@pnpm/exe.darwin-arm64", "pnpm"),
        Platform::LinuxX64Gnu => ("@pnpm/exe.linux-x64", "pnpm"),
        Platform::LinuxArm64Gnu => ("@pnpm/exe.linux-arm64", "pnpm"),
        Platform::WindowsX64 => ("@pnpm/exe.win32-x64", "pnpm.exe"),
        Platform::WindowsArm64 => ("@pnpm/exe.win32-arm64", "pnpm.exe"),
        _ => return None,
    })
}

/// Published `bin` entries, primary first. pnpm 11 moved its entry points to
/// ES modules and added the `pn`/`pnx` aliases (its `.cjs` files remain only
/// as Corepack shims); every pnpm 12 entry point is the platform binary.
fn standard_mappings(
    manager: Manager,
    version: &Version,
    platform: Platform,
) -> Result<Mappings, Error> {
    match (manager, version.major) {
        (Manager::Npm, _) => Ok(&[("npm", "bin/npm-cli.js"), ("npx", "bin/npx-cli.js")]),
        (Manager::Pnpm, ..=10) => Ok(&[("pnpm", "bin/pnpm.cjs"), ("pnpx", "bin/pnpx.cjs")]),
        (Manager::Pnpm, 11) => Ok(&[
            ("pnpm", "bin/pnpm.mjs"),
            ("pnpx", "bin/pnpx.mjs"),
            ("pn", "bin/pnpm.mjs"),
            ("pnx", "bin/pnpx.mjs"),
        ]),
        (Manager::Pnpm, _) => match native_package(platform) {
            Some((_, "pnpm.exe")) => Ok(&[
                ("pnpm", "pnpm.exe"),
                ("pnpx", "pnpm.exe"),
                ("pn", "pnpm.exe"),
                ("pnx", "pnpm.exe"),
            ]),
            Some(_) => Ok(&[
                ("pnpm", "pnpm"),
                ("pnpx", "pnpm"),
                ("pn", "pnpm"),
                ("pnx", "pnpm"),
            ]),
            None => Err(Error::UnsupportedTarget),
        },
        (Manager::Yarn, _) => Ok(&[("yarn", "bin/yarn.js"), ("yarnpkg", "bin/yarn.js")]),
        _ => Err(Error::InvalidLock),
    }
}

fn registry_artifact(url: String, bytes: &VerifiedArtifact, mappings: Mappings) -> lock::Artifact {
    lock::Artifact {
        url,
        sha256: bytes.sha256_hex(),
        format: Format::TarGz,
        root_prefix: Some("package".into()),
        destination: None,
        executables: mappings
            .iter()
            .map(|(name, path)| (name.to_string(), path.to_string()))
            .collect(),
    }
}

/// Pure shape check of one platform's locked parts, shared by lock validation
/// (every variant) and provisioning. Returns the registry package and, for
/// native pnpm, the platform package that provides every executable: the
/// primary entry point plus only published mappings (npm locks all of them).
pub(crate) fn locked_parts<'a>(
    manager: Manager,
    version: &str,
    platform: Platform,
    parts: &'a BTreeMap<String, lock::Artifact>,
) -> Result<(&'a lock::Artifact, Option<&'a lock::Artifact>), Error> {
    let id = tool_id(manager)?;
    // The registry package name, such as `@yarnpkg/cli-dist` for modern Yarn.
    let (name, parsed) =
        registry_metadata::validate_selection(manager, version).map_err(|_| Error::InvalidLock)?;
    let mappings = standard_mappings(manager, &parsed, platform)?;
    let canonical = |artifact: &lock::Artifact, package: &str| {
        artifact.url
            == format!(
                "{REGISTRY}/{}",
                registry_metadata::tarball_path(package, version)
            )
            && artifact.format == Format::TarGz
            && artifact.root_prefix.as_deref() == Some("package")
            && artifact.destination.is_none()
    };
    let (package, native) = if native_release(manager, &parsed) {
        let (binary_package, _) = native_package(platform).ok_or(Error::UnsupportedTarget)?;
        match (parts.get("package"), parts.get("native")) {
            (Some(package), Some(native))
                if parts.len() == 2
                    && package.executables.is_empty()
                    && canonical(native, binary_package) =>
            {
                (package, Some(native))
            }
            _ => return Err(Error::InvalidLock),
        }
    } else {
        match parts.values().next() {
            Some(package) if parts.len() == 1 => (package, None),
            _ => return Err(Error::InvalidLock),
        }
    };
    let executables = &native.unwrap_or(package).executables;
    if !canonical(package, name)
        || executables.get(id).map(String::as_str) != Some(mappings[0].1)
        || (manager == Manager::Npm && executables.len() != mappings.len())
        || executables
            .iter()
            .any(|(name, path)| !mappings.iter().any(|&(n, p)| n == name && p == path))
    {
        return Err(Error::InvalidLock);
    }
    Ok((package, native))
}

/// SHA-256/SHA-512 of modern Yarn's verified single-file bundle.
fn bundle_digests(tree: &ExtractedArtifact) -> Result<(String, String), Error> {
    let bundle = fs::read(tree.root_path().join("bin/yarn.js"))?;
    Ok((
        hex::encode(Sha256::digest(&bundle)),
        hex::encode(Sha512::digest(&bundle)),
    ))
}

fn extract_registry(
    bytes: &VerifiedArtifact,
    manager: Manager,
    version: &str,
    executables: &BTreeMap<String, String>,
    exact_bins: bool,
) -> Result<ExtractedArtifact, Error> {
    let (name, _) = registry_metadata::validate_selection(manager, version)?;
    let required: Vec<_> = executables.values().map(String::as_str).collect();
    let (tree, package) = extract_package(bytes, name, version, &required)?;
    let bin = package.get("bin").and_then(|v| v.as_object());
    if (exact_bins && bin.is_none_or(|bin| bin.len() != executables.len()))
        || executables.iter().any(|(name, path)| {
            // Yarn classic spells its published bins as `./bin/yarn.js`.
            bin.and_then(|bin| bin.get(name))
                .and_then(|v| v.as_str())
                .map(|bin| bin.strip_prefix("./").unwrap_or(bin))
                != Some(path)
        })
    {
        return Err(Error::InvalidPackage);
    }
    Ok(tree)
}

/// The wrapper's placeholder bins are never run, but they name what the
/// native binary serves.
fn extract_wrapper(bytes: &VerifiedArtifact, version: &str) -> Result<ExtractedArtifact, Error> {
    let (tree, package) = extract_package(bytes, "pnpm", version, &[])?;
    let names = package
        .get("bin")
        .and_then(|v| v.as_object())
        .map(|bin| bin.keys().map(String::as_str).collect::<BTreeSet<_>>());
    if names != Some(BTreeSet::from(NATIVE_NAMES))
        || tree.root_path().join(NATIVE_BINARY).try_exists()?
    {
        return Err(Error::InvalidPackage);
    }
    Ok(tree)
}

fn extract_native(
    bytes: &VerifiedArtifact,
    name: &str,
    version: &str,
    binary: &str,
) -> Result<ExtractedArtifact, Error> {
    extract_package(bytes, name, version, &[binary]).map(|(tree, _)| tree)
}

/// Extract `package/` with these regular files and check the manifest
/// identity.
fn extract_package(
    bytes: &VerifiedArtifact,
    name: &str,
    version: &str,
    files: &[&str],
) -> Result<(ExtractedArtifact, serde_json::Value), Error> {
    let mut required = vec!["package.json"];
    required.extend(files);
    let tree = turborepo_archive::extract(
        bytes,
        turborepo_archive::Format::TarGz,
        turborepo_archive::Limits::new(256 * 1024 * 1024, 100_000, 4096, 64)?,
        Layout {
            root: "package",
            required_files: &required,
        },
    )?;
    let package_path = tree.root_path().join("package.json");
    if fs::metadata(&package_path)?.len() > MAX_METADATA_BYTES as u64 {
        return Err(Error::InvalidPackage);
    }
    let UniqueJson(package) =
        serde_json::from_slice(&fs::read(package_path)?).map_err(|_| Error::InvalidPackage)?;
    if package.get("name").and_then(|v| v.as_str()) != Some(name)
        || package.get("version").and_then(|v| v.as_str()) != Some(version)
    {
        return Err(Error::InvalidPackage);
    }
    Ok((tree, package))
}

pub(crate) struct RegistryPlan {
    tool: Tool,
    node: Tool,
    // The lock key that selected this artifact (host platform or `any`).
    platform: Platform,
    artifact: lock::Artifact,
    /// Native pnpm's platform package name and locked artifact.
    native: Option<(&'static str, lock::Artifact)>,
    authored: Option<CorepackIntegrity>,
    launch_directory: String,
    manager: Manager,
    /// Authored pins cover modern Yarn's `bin/yarn.js`, not the archive.
    bundled: bool,
}
impl RegistryPlan {
    /// Validate every applicable authored pin before storage or transport, and
    /// bind the effective strong pin into the inventory's reuse identity.
    /// The lock cannot prove bundle pins, so preparation checks verified bytes.
    pub(crate) fn from_declaration(
        lock: &Lock,
        platform: Platform,
        node: &NodePlan,
        declaration: &crate::package_manager::Declaration,
        manager: Manager,
    ) -> Result<Self, Error> {
        if declaration.manager != manager {
            return Err(Error::InvalidLock);
        }
        let plan = Self::from_lock(lock, platform, node, None, manager)?;
        let version = semver::Version::parse(&plan.tool.version).map_err(|_| Error::InvalidLock)?;
        // Authored pins describe the package Corepack would download: the
        // registry package (native pnpm's wrapper), never a platform package.
        let authored = if plan.bundled {
            declaration.preflight_integrity(&version)?
        } else {
            declaration.locked_integrity(&version, &plan.artifact.sha256)?
        };
        Self::from_lock(lock, platform, node, authored.as_ref(), manager)
    }

    /// Bind Node and authored integrity; retain locked archive SHA-256.
    pub(crate) fn from_lock(
        lock: &Lock,
        platform: Platform,
        node: &NodePlan,
        authored: Option<&CorepackIntegrity>,
        manager: Manager,
    ) -> Result<Self, Error> {
        let id = tool_id(manager)?;
        let windows = platform == Platform::WindowsX64;
        let qualified = match platform {
            Platform::MacosX64
            | Platform::MacosArm64
            | Platform::LinuxX64Gnu
            | Platform::LinuxArm64Gnu => cfg!(unix),
            Platform::WindowsX64 => cfg!(windows),
            _ => false,
        };
        if !qualified {
            return Err(Error::UnsupportedTarget);
        }
        let selected_node = NodePlan::from_lock(lock, platform).map_err(|_| Error::NodeBinding)?;
        if node.inventory_tool() != selected_node.inventory_tool() {
            return Err(Error::NodeBinding);
        }
        let package = lock.tools().get(id).ok_or(Error::InvalidLock)?;
        if package.adapter != id || !package.options.is_empty() {
            return Err(Error::InvalidLock);
        }
        let (_, version) = registry_metadata::validate_selection(manager, &package.version)?;
        let Installation::Managed { artifacts } = &package.installation else {
            return Err(Error::InvalidLock);
        };
        let (&artifact_platform, parts) = artifacts
            .get_key_value(&platform)
            .or_else(|| artifacts.get_key_value(&Platform::Any))
            .ok_or(Error::InvalidLock)?;
        let (artifact, native) = locked_parts(manager, &package.version, platform, parts)?;
        let (artifact, native) = (artifact.clone(), native.cloned());
        // Native pnpm's launchers are POSIX shell scripts; Windows stays
        // fail-closed for it, as in #14743.
        if native.is_some() && windows {
            return Err(Error::UnsupportedTarget);
        }
        ExpectedSha256::from_hex(&artifact.sha256)?;
        let node = node.inventory_tool().clone();
        // A changed authored constraint must re-verify, even when archive SHA-256
        // is unchanged. Hash framed identity into a bounded portable component.
        let binding =
            serde_json::to_vec(&(&node, authored.map(|pin| (pin.algorithm, &pin.digest))))
                .map_err(|_| Error::InvalidLock)?;
        let launch_directory = format!("managed-node-{:x}", Sha256::digest(binding));
        // Windows launchers are batch files so PATHEXT resolves `pnpm`.
        let extension = if windows { ".cmd" } else { "" };
        let executables = native
            .as_ref()
            .unwrap_or(&artifact)
            .executables
            .keys()
            .map(|name| {
                (
                    name.clone(),
                    format!("{launch_directory}/{name}{extension}"),
                )
            })
            .collect();
        // Reuse identity binds every verified part.
        let artifact_sha256 = match &native {
            None => artifact.sha256.clone(),
            Some(native) => {
                ExpectedSha256::from_hex(&native.sha256)?;
                let parts = format!("pnpm-native\0{}\0{}", artifact.sha256, native.sha256);
                format!("{:x}", Sha256::digest(parts))
            }
        };
        let native = native
            .map(|native| native_package(platform).map(|(name, _)| (name, native)))
            .map(|native| native.ok_or(Error::UnsupportedTarget))
            .transpose()?;
        Ok(Self {
            tool: Tool {
                id: id.into(),
                version: package.version.clone(),
                platform: node.platform.clone(),
                artifact_sha256,
                executables,
            },
            node,
            platform: artifact_platform,
            artifact,
            native,
            authored: authored.cloned(),
            launch_directory,
            manager,
            bundled: registry_metadata::is_modern_yarn(manager, &version),
        })
    }

    pub fn inventory_tool(&self) -> &Tool {
        &self.tool
    }

    /// Prepare without executing Node, then reconcile the SAME complete desired
    /// set with all adapter callbacks. Only healthy generations allow reuse.
    pub async fn prepare_if_needed(
        &self,
        store: &Store,
        desired: &[Tool],
        transport: &RegistryTransport,
    ) -> Result<Option<PreparedRegistry>, Error> {
        self.prepare(store, desired, transport, crate::Preparation::IfNeeded)
            .await
    }

    pub async fn prepare(
        &self,
        store: &Store,
        desired: &[Tool],
        transport: &RegistryTransport,
        preparation: crate::Preparation,
    ) -> Result<Option<PreparedRegistry>, Error> {
        if !desired.contains(&self.tool) || !desired.contains(&self.node) {
            return Err(Error::NodeBinding);
        }
        if store.reusable_tree(&self.tool, desired)?.is_some()
            && preparation == crate::Preparation::IfNeeded
        {
            return Ok(None);
        }
        // Corepack pins modern Yarn's bundle, checked after extraction, never the
        // archive bytes a cache hit or download verifies.
        let archive_pin = self.authored.as_ref().filter(|_| !self.bundled);
        let (bytes, mut from_cache) = self
            .part_bytes(transport, None, &self.artifact.sha256, archive_pin)
            .await?;
        // Cache hits transfer nothing.
        let mut downloaded_bytes = if from_cache {
            0
        } else {
            bytes.as_bytes().len() as u64
        };
        let (tree, native) = match &self.native {
            None => {
                let tree = extract_registry(
                    &bytes,
                    self.manager,
                    &self.tool.version,
                    &self.artifact.executables,
                    self.manager == Manager::Npm,
                )?;
                if let Some(pin) = self.authored.as_ref().filter(|_| self.bundled) {
                    let (sha256, sha512) = bundle_digests(&tree)?;
                    crate::registry_resolution::check_pin(pin, None, &sha256, &sha512)?;
                }
                (tree, None)
            }
            Some((name, native)) => {
                let tree = extract_wrapper(&bytes, &self.tool.version)?;
                let binary = native.executables.get("pnpm").ok_or(Error::InvalidLock)?;
                let (native_bytes, cached) = self
                    .part_bytes(transport, Some(name), &native.sha256, None)
                    .await?;
                if !cached {
                    downloaded_bytes += native_bytes.as_bytes().len() as u64;
                }
                from_cache &= cached;
                let native = extract_native(&native_bytes, name, &self.tool.version, binary)?;
                (tree, Some((native, binary.clone())))
            }
        };
        if tree.root_path().join(&self.launch_directory).try_exists()? {
            return Err(Error::InvalidPackage);
        }
        Ok(Some(PreparedRegistry {
            tree,
            native,
            downloaded_bytes,
            tool: self.tool.clone(),
            launch_directory: self.launch_directory.clone(),
            entrypoints: self.artifact.executables.clone(),
            from_cache,
        }))
    }

    /// Verified bytes for one locked part: a verified shared-cache hit, or the
    /// registry (`name` selects native pnpm's platform package), cached best
    /// effort. Offline transports never reach the registry.
    async fn part_bytes(
        &self,
        transport: &RegistryTransport,
        name: Option<&str>,
        sha256: &str,
        pin: Option<&CorepackIntegrity>,
    ) -> Result<(VerifiedArtifact, bool), Error> {
        if let Some(bytes) = transport.cached(self.platform, sha256, pin) {
            return Ok((bytes, true));
        }
        let version = &self.tool.version;
        let (_, bytes) = match name {
            None => {
                transport
                    .verified_release(self.manager, version, pin, Some(sha256))
                    .await?
            }
            Some(name) => {
                transport
                    .verified_package(self.manager, name, version, pin, Some(sha256))
                    .await?
            }
        };
        // Best effort: a cache write failure never fails verified setup.
        if let Some(cache) = &transport.cache {
            let _ = cache.put(self.platform, &bytes);
        }
        Ok((bytes, false))
    }
}

pub struct PreparedRegistry {
    tree: ExtractedArtifact,
    /// The verified platform package and its binary path, for native pnpm.
    native: Option<(ExtractedArtifact, String)>,
    downloaded_bytes: u64,
    tool: Tool,
    launch_directory: String,
    entrypoints: std::collections::BTreeMap<String, String>,
    from_cache: bool,
}
impl PreparedRegistry {
    /// Verified package bytes transferred for this preparation.
    pub fn downloaded_bytes(&self) -> u64 {
        self.downloaded_bytes
    }

    /// Whether verified bytes came from the shared download cache.
    pub fn from_cache(&self) -> bool {
        self.from_cache
    }

    /// Copy verified resources; launch this generation's managed Node, or the
    /// native pnpm binary directly (`pnpx`/`pnx` as `pnpm dlx`, like pnpm's
    /// own Unix alias scripts).
    pub fn stage(
        &self,
        tool: &Tool,
        destination: &Path,
    ) -> Result<(), turborepo_tool_install::Error> {
        if !cfg!(any(unix, windows)) {
            return Err(turborepo_tool_install::Error::UnsupportedPlatform);
        }
        if tool != &self.tool
            || fs::symlink_metadata(destination)?.file_type().is_symlink()
            || fs::read_dir(destination)?.next().is_some()
        {
            return Err(turborepo_tool_install::Error::InvalidInventory);
        }
        copy_tree(&self.tree.root_path(), destination)?;
        if let Some((native, binary)) = &self.native {
            let target = destination.join(NATIVE_BINARY);
            fs::copy(native.root_path().join(binary), &target)?;
            executable(&target)?;
        }
        // Owner-only like every other store directory, whatever the umask.
        let mut launch = fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut launch, 0o700);
        launch.create(destination.join(&self.launch_directory))?;
        for (name, path) in &self.tool.executables {
            // The bin forwarder calls this file directly, so %~dp0 is always
            // the launch directory. Entry points are fixed validated mappings.
            // Native pnpm never plans on Windows (`RegistryPlan::from_lock`).
            let script = if cfg!(windows) {
                format!(
                    "@\"%~dp0..\\..\\node\\node.exe\" \"%~dp0..\\{}\" %*\r\n",
                    self.entrypoints[name].replace('/', "\\")
                )
            } else {
                let id = &self.tool.id;
                let command = match (&self.native, name.as_str()) {
                    (None, _) => format!(
                        "\"$tools/node/bin/node\" \"$tools/{id}/{}\"",
                        self.entrypoints[name]
                    ),
                    (Some(_), "pnpx" | "pnx") => format!("\"$tools/{id}/{NATIVE_BINARY}\" dlx"),
                    (Some(_), _) => format!("\"$tools/{id}/{NATIVE_BINARY}\""),
                };
                format!(
                    "#!/bin/sh\ncase \"$0\" in */*) ;; *) exit 126;; esac\nhere=${{0%/*}}\ncase \
                     \"${{here##*/}}\" in {}) tools=\"$here/../..\";; *) \
                     tools=\"$here/../tools\";; esac\ntools=$(CDPATH= cd -- \"$tools\" && pwd -P) \
                     || exit 126\nexec {command} \"$@\"\n",
                    self.launch_directory
                )
            };
            fs::write(destination.join(path), script)?;
            executable(&destination.join(path))?;
        }
        Ok(())
    }
}

fn executable(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Locked pnpm facade retaining the existing preparation/staging API.
pub struct PnpmPlan(RegistryPlan);
impl PnpmPlan {
    /// Validate every applicable authored pin before storage or transport, and
    /// bind the effective strong pin into the inventory's reuse identity.
    pub fn from_declaration(
        lock: &Lock,
        platform: Platform,
        node: &NodePlan,
        declaration: &crate::package_manager::Declaration,
    ) -> Result<Self, Error> {
        RegistryPlan::from_declaration(lock, platform, node, declaration, Manager::Pnpm).map(Self)
    }

    pub fn from_lock(
        lock: &Lock,
        platform: Platform,
        node: &NodePlan,
        authored: Option<&CorepackIntegrity>,
    ) -> Result<Self, Error> {
        RegistryPlan::from_lock(lock, platform, node, authored, Manager::Pnpm).map(Self)
    }

    pub fn inventory_tool(&self) -> &Tool {
        self.0.inventory_tool()
    }

    /// Prepare only; reconcile the same complete desired inventory afterward.
    pub async fn prepare_if_needed(
        &self,
        store: &Store,
        desired: &[Tool],
        transport: &RegistryTransport,
    ) -> Result<Option<PreparedRegistry>, Error> {
        self.0.prepare_if_needed(store, desired, transport).await
    }

    /// Select fresh verification without changing the locked identity.
    pub async fn prepare(
        &self,
        store: &Store,
        desired: &[Tool],
        transport: &RegistryTransport,
        preparation: crate::Preparation,
    ) -> Result<Option<PreparedRegistry>, Error> {
        self.0.prepare(store, desired, transport, preparation).await
    }
}

#[cfg(all(test, unix))]
#[path = "pnpm_provision/native_fixture.rs"]
pub(crate) mod native_fixture;
#[cfg(all(test, unix))]
#[path = "pnpm_provision/tests.rs"]
mod pnpm_tests;
