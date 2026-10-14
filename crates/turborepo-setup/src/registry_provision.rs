//! Shared exact registry verification and locked preparation for npm/pnpm.
//! No hooks, probes, dependency installation or activation.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::Path,
    time::Duration,
};

use semver::Version;
use sha2::{Digest, Sha224, Sha256};
use turborepo_archive::{ExtractedArtifact, Layout};
use turborepo_download::{
    ApprovedOrigin, DownloadClient, ExpectedSha256, ExpectedSha512, Limits, VerifiedArtifact,
};
use turborepo_tool_install::{Store, Tool};

use crate::{
    lock::{self, Format, Installation, Lock, Platform},
    node_discovery::UniqueJson,
    node_provision::{NodePlan, copy_tree},
    package_manager::{CorepackIntegrity, Manager},
    registry_metadata::{self, MAX_METADATA_BYTES, PUBLIC_NPM_REGISTRY as REGISTRY, tarball_path},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid locked registry artifact or executable mapping; regenerate turbo.lock")]
    InvalidLock,
    #[error("package manager requires the selected managed Node in the complete desired inventory")]
    NodeBinding,
    #[error("registry package promotion is unqualified for Windows, musl or non-Unix hosts")]
    UnsupportedTarget,
    #[error("package identity or bin entries do not match the selected lock")]
    InvalidPackage,
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

pub struct RegistryTransport {
    client: DownloadClient,
    origin: String,
}
impl RegistryTransport {
    // TODO(TURBO-6219): Apply mirror/registry policy to requests and validation.
    // Never fall back to public npm or forward credentials across origins.
    pub fn official(policy: &crate::source_policy::OfficialSourcePolicy) -> Result<Self, Error> {
        Ok(Self {
            client: DownloadClient::new([ApprovedOrigin::https(policy.registry())?])?,
            origin: policy.registry().into(),
        })
    }

    /// Resolve only a canonical exact npm/pnpm release, verifying registry
    /// SHA-512 and optional authored integrity against bytes before
    /// deriving lock SHA-256. Private bounded scratch is dropped before
    /// return; no repository path or I/O.
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
        let (release, bytes) = self
            .verified_release(manager, exact_version, authored, None)
            .await?;
        let mut natives = BTreeMap::new();
        let package = if native_release(manager, &version) {
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
            drop(extract_registry(
                &bytes,
                manager,
                exact_version,
                &artifact.executables,
                true,
            )?);
            artifact
        };
        Ok(crate::registry_resolution::ResolvedRegistry::new(
            release,
            package,
            natives,
            sha224_hex(&bytes),
        ))
    }

    async fn verified_release(
        &self,
        manager: Manager,
        version: &str,
        authored: Option<&CorepackIntegrity>,
        locked_sha256: Option<&str>,
    ) -> Result<(registry_metadata::RegistryArtifact, VerifiedArtifact), Error> {
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
        // Validate before interpolating any request path or making traffic.
        registry_metadata::validate_selection(manager, version)?;
        let metadata = self
            .client
            .read_metadata(
                &format!("{}/{name}/{version}", self.origin),
                Limits::new(MAX_METADATA_BYTES, Duration::from_secs(30))?,
            )
            .await?;
        let release =
            registry_metadata::parse_package(&metadata, manager, name, version, authored)?;
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
                &format!("{}/{}", self.origin, tarball_path(name, version)),
                ExpectedSha512::from_hex(&release.integrity.digest)?,
                sha256,
                Limits::new(64 * 1024 * 1024, Duration::from_secs(120))?,
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
        })
    }
}

fn sha224_hex(bytes: &VerifiedArtifact) -> String {
    format!("{:x}", Sha224::digest(bytes.as_bytes()))
}

fn package_name(manager: Manager) -> Result<&'static str, Error> {
    match manager {
        Manager::Npm => Ok("npm"),
        Manager::Pnpm => Ok("pnpm"),
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

type Mappings = &'static [(&'static str, &'static str)];
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
    let name = package_name(manager)?;
    let parsed = Version::parse(version).map_err(|_| Error::InvalidLock)?;
    let mappings = standard_mappings(manager, &parsed, platform)?;
    let canonical = |artifact: &lock::Artifact, package: &str| {
        artifact.url == format!("{REGISTRY}/{}", tarball_path(package, version))
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
        || executables.get(name).map(String::as_str) != Some(mappings[0].1)
        || (manager == Manager::Npm && executables.len() != mappings.len())
        || executables
            .iter()
            .any(|(name, path)| !mappings.iter().any(|&(n, p)| n == name && p == path))
    {
        return Err(Error::InvalidLock);
    }
    Ok((package, native))
}

fn extract_registry(
    bytes: &VerifiedArtifact,
    manager: Manager,
    version: &str,
    executables: &BTreeMap<String, String>,
    exact_bins: bool,
) -> Result<ExtractedArtifact, Error> {
    let required: Vec<_> = executables.values().map(String::as_str).collect();
    let (tree, package) = extract_package(bytes, package_name(manager)?, version, &required)?;
    let bin = package.get("bin").and_then(|v| v.as_object());
    if (exact_bins && bin.is_none_or(|bin| bin.len() != executables.len()))
        || executables.iter().any(|(name, path)| {
            bin.and_then(|bin| bin.get(name)).and_then(|v| v.as_str()) != Some(path)
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
    artifact: lock::Artifact,
    /// Native pnpm's platform package name, binary path and locked artifact.
    native: Option<(&'static str, lock::Artifact)>,
    authored: Option<CorepackIntegrity>,
    launch_directory: String,
    manager: Manager,
}
impl RegistryPlan {
    /// Bind Node and authored integrity; retain locked archive SHA-256.
    pub(crate) fn from_lock(
        lock: &Lock,
        platform: Platform,
        node: &NodePlan,
        authored: Option<&CorepackIntegrity>,
        manager: Manager,
    ) -> Result<Self, Error> {
        let name = package_name(manager)?;
        if !cfg!(unix)
            || !matches!(
                platform,
                Platform::MacosX64
                    | Platform::MacosArm64
                    | Platform::LinuxX64Gnu
                    | Platform::LinuxArm64Gnu
            )
        {
            return Err(Error::UnsupportedTarget);
        }
        let selected_node = NodePlan::from_lock(lock, platform).map_err(|_| Error::NodeBinding)?;
        if node.inventory_tool() != selected_node.inventory_tool() {
            return Err(Error::NodeBinding);
        }
        let package = lock.tools().get(name).ok_or(Error::InvalidLock)?;
        if package.adapter != name || !package.options.is_empty() {
            return Err(Error::InvalidLock);
        }
        let Installation::Managed { artifacts } = &package.installation else {
            return Err(Error::InvalidLock);
        };
        let parts = artifacts
            .get(&platform)
            .or_else(|| artifacts.get(&Platform::Any))
            .ok_or(Error::InvalidLock)?;
        let (artifact, native) = locked_parts(manager, &package.version, platform, parts)?;
        let (artifact, native) = (artifact.clone(), native.cloned());
        ExpectedSha256::from_hex(&artifact.sha256)?;
        let node = node.inventory_tool().clone();
        // A changed authored constraint must re-verify, even when archive SHA-256
        // is unchanged. Hash framed identity into a bounded portable component.
        let binding =
            serde_json::to_vec(&(&node, authored.map(|pin| (pin.algorithm, &pin.digest))))
                .map_err(|_| Error::InvalidLock)?;
        let launch_directory = format!("managed-node-{:x}", Sha256::digest(binding));
        let executables = native
            .as_ref()
            .unwrap_or(&artifact)
            .executables
            .keys()
            .map(|name| (name.clone(), format!("{launch_directory}/{name}")))
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
                id: name.into(),
                version: package.version.clone(),
                platform: node.platform.clone(),
                artifact_sha256,
                executables,
            },
            node,
            artifact,
            native,
            authored: authored.cloned(),
            launch_directory,
            manager,
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
        let (_, bytes) = transport
            .verified_release(
                self.manager,
                &self.tool.version,
                self.authored.as_ref(),
                Some(&self.artifact.sha256),
            )
            .await?;
        let (tree, native) = match &self.native {
            None => {
                let executables = &self.artifact.executables;
                let exact = self.manager == Manager::Npm;
                let tree =
                    extract_registry(&bytes, self.manager, &self.tool.version, executables, exact)?;
                (tree, None)
            }
            Some((name, native)) => {
                let tree = extract_wrapper(&bytes, &self.tool.version)?;
                let binary = native.executables.get("pnpm").ok_or(Error::InvalidLock)?;
                let (_, bytes) = transport
                    .verified_package(
                        self.manager,
                        name,
                        &self.tool.version,
                        None,
                        Some(&native.sha256),
                    )
                    .await?;
                let native = extract_native(&bytes, name, &self.tool.version, binary)?;
                (tree, Some((native, binary.clone())))
            }
        };
        if tree.root_path().join(&self.launch_directory).try_exists()? {
            return Err(Error::InvalidPackage);
        }
        Ok(Some(PreparedRegistry {
            tree,
            native,
            tool: self.tool.clone(),
            launch_directory: self.launch_directory.clone(),
            entrypoints: self.artifact.executables.clone(),
        }))
    }
}

pub struct PreparedRegistry {
    tree: ExtractedArtifact,
    /// The verified platform package and its binary path, for native pnpm.
    native: Option<(ExtractedArtifact, String)>,
    tool: Tool,
    launch_directory: String,
    entrypoints: std::collections::BTreeMap<String, String>,
}
impl PreparedRegistry {
    /// Copy verified resources; launch this generation's managed Node, or the
    /// native pnpm binary directly (`pnpx`/`pnx` as `pnpm dlx`, like pnpm's
    /// own Unix alias scripts).
    pub fn stage(
        &self,
        tool: &Tool,
        destination: &Path,
    ) -> Result<(), turborepo_tool_install::Error> {
        if !cfg!(unix) {
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
            let id = &self.tool.id;
            let command = match (&self.native, name.as_str()) {
                (None, _) => format!(
                    "\"$tools/node/bin/node\" \"$tools/{id}/{}\"",
                    self.entrypoints[name]
                ),
                (Some(_), "pnpx" | "pnx") => format!("\"$tools/{id}/{NATIVE_BINARY}\" dlx"),
                (Some(_), _) => format!("\"$tools/{id}/{NATIVE_BINARY}\""),
            };
            let script = format!(
                "#!/bin/sh\ncase \"$0\" in */*) ;; *) exit 126;; esac\nhere=${{0%/*}}\ncase \
                 \"${{here##*/}}\" in {}) tools=\"$here/../..\";; *) tools=\"$here/../tools\";; \
                 esac\ntools=$(CDPATH= cd -- \"$tools\" && pwd -P) || exit 126\nexec {command} \
                 \"$@\"\n",
                self.launch_directory
            );
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
        if declaration.manager != Manager::Pnpm {
            return Err(Error::InvalidLock);
        }
        let plan = Self::from_lock(lock, platform, node, None)?;
        let version = semver::Version::parse(&plan.inventory_tool().version)
            .map_err(|_| Error::InvalidLock)?;
        // Authored pins describe the `pnpm` package Corepack would download.
        let authored = declaration.locked_integrity(&version, &plan.0.artifact.sha256)?;
        Self::from_lock(lock, platform, node, authored.as_ref())
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
