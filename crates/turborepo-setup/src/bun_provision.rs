//! Locked Bun from official GitHub release archives, verified against each
//! release's published SHASUMS256.txt. Bun is self-contained: no managed Node,
//! global Bun, PATH change, `bun install`, or lifecycle execution. `bunx` is
//! the same binary selected by its invoked name, as Bun's own installer does.

use std::{collections::BTreeMap, io, path::Path, time::Duration};

use semver::Version;
use turborepo_archive::{ExtractedArtifact, Layout};
use turborepo_download::{ApprovedOrigin, DownloadClient, ExpectedSha256, Limits};
use turborepo_tool_install::{Store, Tool};

use crate::{
    lock::{Artifact, Format, Installation, Lock, Platform},
    node_metadata::{Checksums, MAX_CHECKSUM_BYTES},
    node_provision::copy_tree,
    package_manager::{Declaration, Manager},
};

const RELEASES: &str = "https://github.com";
const PLATFORMS: [Platform; 6] = [
    Platform::MacosX64,
    Platform::MacosArm64,
    Platform::LinuxX64Gnu,
    Platform::LinuxArm64Gnu,
    Platform::WindowsX64,
    Platform::WindowsArm64,
];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid locked Bun artifact or executable mapping; regenerate turbo.lock")]
    InvalidLock,
    #[error("no locked Bun artifact for this target; update turbo.lock on a supported target")]
    MissingTarget,
    #[error("managed Bun promotion is unqualified for Windows, musl or non-Unix hosts")]
    UnsupportedTarget,
    #[error("an exact canonical Bun release version is required")]
    InvalidVersion,
    #[error("invalid Bun SHA-256 manifest or no supported release artifact")]
    Checksums,
    #[error("the complete desired inventory must include this Bun plan unchanged")]
    InventoryMismatch,
    #[error(transparent)]
    Authored(#[from] crate::package_manager::Error),
    #[error(transparent)]
    Download(#[from] turborepo_download::Error),
    #[error(transparent)]
    Archive(#[from] turborepo_archive::Error),
    #[error(transparent)]
    Install(#[from] turborepo_tool_install::Error),
    #[error("Bun staging I/O failed")]
    Io(#[from] io::Error),
}

/// Official archive target. x64 uses the `-baseline` build so one portable
/// lock also runs on x64 CPUs without AVX2. No musl, Android, or FreeBSD.
fn target(platform: Platform) -> Option<&'static str> {
    Some(match platform {
        Platform::MacosX64 => "darwin-x64-baseline",
        Platform::MacosArm64 => "darwin-aarch64",
        Platform::LinuxX64Gnu => "linux-x64-baseline",
        Platform::LinuxArm64Gnu => "linux-aarch64",
        Platform::WindowsX64 => "windows-x64-baseline",
        Platform::WindowsArm64 => "windows-aarch64",
        _ => return None,
    })
}

/// Release directory path for an exact canonical version, before any traffic.
fn release(version: &str) -> Result<String, Error> {
    let parsed = Version::parse(version).map_err(|_| Error::InvalidVersion)?;
    if version.len() > 128
        || parsed.to_string() != version
        || !parsed.build.is_empty()
        || !crate::version_request::is_valid_release(&parsed)
    {
        return Err(Error::InvalidVersion);
    }
    Ok(format!("/oven-sh/bun/releases/download/bun-v{version}"))
}

/// The one artifact shape setup locks and accepts for a platform.
fn official(platform: Platform, version: &str, sha256: &str) -> Result<Artifact, Error> {
    let target = target(platform).ok_or(Error::UnsupportedTarget)?;
    let binary = if target.starts_with("windows-") {
        "bun.exe"
    } else {
        "bun"
    };
    Ok(Artifact {
        alternates: BTreeMap::new(),
        url: format!("{RELEASES}{}/bun-{target}.zip", release(version)?),
        sha256: sha256.into(),
        format: Format::Zip,
        root_prefix: Some(format!("bun-{target}")),
        destination: None,
        executables: BTreeMap::from([
            ("bun".into(), binary.into()),
            ("bunx".into(), binary.into()),
        ]),
    })
}

/// Portable check of EVERY locked variant without host restrictions, for
/// preserved selections; the exact release must still match declarations.
pub(crate) fn validate(tool: &crate::lock::Tool, declaration: &Declaration) -> Result<(), Error> {
    let Installation::Managed { artifacts } = &tool.installation else {
        return Err(Error::InvalidLock);
    };
    if tool.adapter != "bun" || !tool.options.is_empty() {
        return Err(Error::InvalidLock);
    }
    for (platform, parts) in artifacts {
        let artifact = parts.get("distribution").ok_or(Error::InvalidLock)?;
        let expected = official(*platform, &tool.version, &artifact.sha256);
        if parts.len() != 1 || expected.ok().as_ref() != Some(artifact) {
            return Err(Error::InvalidLock);
        }
    }
    let version = Version::parse(&tool.version).map_err(|_| Error::InvalidLock)?;
    declaration.preflight_integrity(&version)?;
    Ok(())
}

pub struct BunTransport {
    client: DownloadClient,
    // Test transport changes only the origin; lock provenance remains official.
    origin: String,
}
impl BunTransport {
    // TODO(TURBO-6373): GitHub answers release downloads with a redirect to
    // its asset CDN; approve those origins once redirect support lands.
    pub fn official(_policy: &crate::source_policy::OfficialSourcePolicy) -> Result<Self, Error> {
        Ok(Self {
            client: DownloadClient::new([ApprovedOrigin::https(RELEASES)?])?,
            origin: RELEASES.into(),
        })
    }

    /// Explicit fixture-only HTTP opt-in; literal loopback IPs only.
    #[cfg(any(test, feature = "test-support"))]
    pub fn loopback_http_for_tests(origin: &str) -> Result<Self, Error> {
        Ok(Self {
            client: DownloadClient::loopback_http_for_tests(origin)?,
            origin: origin.trim_end_matches('/').into(),
        })
    }

    /// Lock every supported target the exact release publishes in its
    /// SHASUMS256.txt. Unpublished targets are omitted, never substituted.
    /// Reads only the checksum manifest: no archive download or execution.
    pub async fn resolve_exact(&self, version: &str) -> Result<Installation, Error> {
        let manifest = self
            .client
            .read_metadata(
                &format!("{}{}/SHASUMS256.txt", self.origin, release(version)?),
                Limits::new(MAX_CHECKSUM_BYTES, Duration::from_secs(30))?,
            )
            .await?;
        let checksums = Checksums::parse(&manifest).map_err(|_| Error::Checksums)?;
        let mut artifacts = BTreeMap::new();
        for platform in PLATFORMS {
            let target = target(platform).ok_or(Error::UnsupportedTarget)?;
            if let Some(sha256) = checksums.sha256(&format!("bun-{target}.zip")) {
                let artifact = official(platform, version, sha256)?;
                artifacts.insert(
                    platform,
                    BTreeMap::from([("distribution".into(), artifact)]),
                );
            }
        }
        if artifacts.is_empty() {
            return Err(Error::Checksums);
        }
        Ok(Installation::Managed { artifacts })
    }
}

pub struct BunPlan {
    tool: Tool,
    artifact: Artifact,
}
impl BunPlan {
    /// Bun has no Corepack integrity (discovery rejects it); the locked exact
    /// release must still satisfy every applicable native declaration.
    pub fn from_declaration(
        lock: &Lock,
        platform: Platform,
        declaration: &Declaration,
    ) -> Result<Self, Error> {
        if declaration.manager != Manager::Bun {
            return Err(Error::InvalidLock);
        }
        let plan = Self::from_lock(lock, platform)?;
        let version = Version::parse(&plan.tool.version).map_err(|_| Error::InvalidLock)?;
        if declaration.preflight_integrity(&version)?.is_some() {
            return Err(Error::InvalidLock);
        }
        Ok(plan)
    }

    /// Validate only the selected platform's exact official artifact shape.
    pub fn from_lock(lock: &Lock, platform: Platform) -> Result<Self, Error> {
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
        let bun = lock.tools().get("bun").ok_or(Error::InvalidLock)?;
        if bun.adapter != "bun" || !bun.options.is_empty() {
            return Err(Error::InvalidLock);
        }
        let Installation::Managed { artifacts } = &bun.installation else {
            return Err(Error::InvalidLock);
        };
        let parts = artifacts.get(&platform).ok_or(Error::MissingTarget)?;
        let artifact = parts.get("distribution").ok_or(Error::InvalidLock)?;
        let expected =
            official(platform, &bun.version, &artifact.sha256).map_err(|_| Error::InvalidLock)?;
        if parts.len() != 1 || artifact != &expected {
            return Err(Error::InvalidLock);
        }
        ExpectedSha256::from_hex(&artifact.sha256)?;
        let (_, spelling) = crate::node::lock_target(platform).ok_or(Error::UnsupportedTarget)?;
        Ok(Self {
            tool: Tool {
                id: "bun".into(),
                version: bun.version.clone(),
                platform: spelling.into(),
                artifact_sha256: artifact.sha256.clone(),
                executables: artifact.executables.clone(),
            },
            artifact: expected,
        })
    }

    pub fn inventory_tool(&self) -> &Tool {
        &self.tool
    }

    pub async fn prepare_if_needed(
        &self,
        store: &Store,
        desired: &[Tool],
        transport: &BunTransport,
    ) -> Result<Option<PreparedBun>, Error> {
        self.prepare(store, desired, transport, crate::Preparation::IfNeeded)
            .await
    }

    /// Reuse only a healthy selected generation; otherwise download, verify
    /// the locked SHA-256, and extract. Never publishes or executes Bun.
    pub async fn prepare(
        &self,
        store: &Store,
        desired: &[Tool],
        transport: &BunTransport,
        preparation: crate::Preparation,
    ) -> Result<Option<PreparedBun>, Error> {
        if !desired.contains(&self.tool) {
            return Err(Error::InventoryMismatch);
        }
        if store.reusable_tree(&self.tool, desired)?.is_some()
            && preparation == crate::Preparation::IfNeeded
        {
            return Ok(None);
        }
        let path = self
            .artifact
            .url
            .strip_prefix(RELEASES)
            .ok_or(Error::InvalidLock)?;
        let bytes = transport
            .client
            .download_verified(
                &format!("{}{path}", transport.origin),
                ExpectedSha256::from_hex(&self.artifact.sha256)?,
                Limits::new(128 * 1024 * 1024, Duration::from_secs(300))?,
            )
            .await?;
        let tree = turborepo_archive::extract(
            &bytes,
            turborepo_archive::Format::Zip,
            turborepo_archive::Limits::new(512 * 1024 * 1024, 1024, 4096, 16)?,
            Layout {
                root: self
                    .artifact
                    .root_prefix
                    .as_deref()
                    .ok_or(Error::InvalidLock)?,
                required_files: &[&self.tool.executables["bun"]],
            },
        )?;
        Ok(Some(PreparedBun {
            tree,
            tool: self.tool.clone(),
        }))
    }
}

pub struct PreparedBun {
    tree: ExtractedArtifact,
    tool: Tool,
}
impl PreparedBun {
    /// Callback for Store::reconcile: copy the verified release tree as-is.
    pub fn stage(
        &self,
        tool: &Tool,
        destination: &Path,
    ) -> Result<(), turborepo_tool_install::Error> {
        if tool != &self.tool
            || std::fs::symlink_metadata(destination)?
                .file_type()
                .is_symlink()
            || std::fs::read_dir(destination)?.next().is_some()
        {
            return Err(turborepo_tool_install::Error::InvalidInventory);
        }
        copy_tree(&self.tree.root_path(), destination)?;
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests;
