//! Locked uv adapter: verified official archive, full resource tree, and
//! repo-scoped `uv`/`uvx` launchers. No resolution, Python installation,
//! dependency sync or subprocesses. Plans compose with the complete desired
//! inventory; preparation never publishes.

use std::{fs, io, path::Path, time::Duration};

use sha2::{Digest, Sha256};
use turborepo_archive::{ExtractedArtifact, Layout};
use turborepo_download::ExpectedSha256;
use turborepo_tool_install::{Store, Tool};

use crate::{
    lock::{self, Installation, Lock, Platform},
    node_provision::copy_tree,
    uv::{self, UvArtifact},
    uv_resolution::UvTransport,
};

/// Launchers pin uv's interpreter directory to the `python` tool tree of the
/// SAME generation, so managed interpreters stay repo-scoped and never land in
/// the user's global uv data. uv must not download interpreters into an
/// immutable generation, and when setup provisions that tree uv uses only it,
/// never a system Python.
const LAUNCHER: &str = "#!/bin/sh
case \"$0\" in */*) ;; *) exit 126;; esac
here=${0%/*}
case \"${here##*/}\" in @LAUNCH@) tools=\"$here/../..\";; *) tools=\"$here/../tools\";; esac
tools=$(CDPATH= cd -- \"$tools\" && pwd -P) || exit 126
UV_PYTHON_INSTALL_DIR=\"$tools/python\"
UV_PYTHON_DOWNLOADS=never
export UV_PYTHON_INSTALL_DIR UV_PYTHON_DOWNLOADS
if [ -d \"$tools/python\" ]; then UV_PYTHON_PREFERENCE=only-managed; export UV_PYTHON_PREFERENCE; \
                        fi
exec \"$tools/uv/@ENTRYPOINT@\" \"$@\"
";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid locked uv artifact or mappings; regenerate turbo.lock")]
    InvalidLock,
    #[error(
        "no locked uv artifact for this target; update turbo.lock on a supported target or \
         configure verify-system"
    )]
    MissingTarget,
    #[error(
        "managed uv promotion is not qualified for Windows or musl; use a supported GNU \
         Linux/macOS target or configure verify-system"
    )]
    UnsupportedTarget,
    #[error("the complete desired inventory must include this uv plan unchanged")]
    InventoryMismatch,
    #[error(transparent)]
    Download(#[from] turborepo_download::Error),
    #[error(transparent)]
    Archive(#[from] turborepo_archive::Error),
    #[error(transparent)]
    Install(#[from] turborepo_tool_install::Error),
    #[error("uv staging I/O failed")]
    Io(#[from] io::Error),
}

pub struct UvPlan {
    tool: Tool,
    artifact: lock::Artifact,
    launch_directory: String,
}

impl UvPlan {
    /// Validate only the selected platform's metadata against the official
    /// layout, without modifying the lock or inferring other executables.
    pub fn from_lock(lock: &Lock, platform: Platform) -> Result<Self, Error> {
        if !matches!(
            platform,
            Platform::MacosX64
                | Platform::MacosArm64
                | Platform::LinuxX64Gnu
                | Platform::LinuxArm64Gnu
        ) {
            return Err(Error::UnsupportedTarget);
        }
        let locked = lock.tools().get("uv").ok_or(Error::InvalidLock)?;
        let version = uv::exact_release(&locked.version).ok_or(Error::InvalidLock)?;
        if locked.adapter != "uv" || !locked.options.is_empty() {
            return Err(Error::InvalidLock);
        }
        let Installation::Managed { artifacts } = &locked.installation else {
            return Err(Error::InvalidLock);
        };
        let parts = artifacts.get(&platform).ok_or(Error::MissingTarget)?;
        let artifact = match parts.get("distribution") {
            Some(artifact) if parts.len() == 1 => artifact.clone(),
            _ => return Err(Error::InvalidLock),
        };
        let official = UvArtifact::for_platform(&version, platform).ok_or(Error::InvalidLock)?;
        if artifact != official.lock_artifact(artifact.sha256.clone()) {
            return Err(Error::InvalidLock);
        }
        ExpectedSha256::from_hex(&artifact.sha256)?;
        // Launcher semantics are part of reuse identity: a changed launcher must
        // restage instead of copying an older generation's tree.
        let launch_directory = format!("launch-{}", &hex::encode(Sha256::digest(LAUNCHER))[..16]);
        let spelling = serde_json::to_value(platform)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .ok_or(Error::InvalidLock)?;
        Ok(Self {
            tool: Tool {
                id: "uv".into(),
                version: locked.version.clone(),
                platform: spelling,
                artifact_sha256: artifact.sha256.clone(),
                executables: artifact
                    .executables
                    .keys()
                    .map(|name| (name.clone(), format!("{launch_directory}/{name}")))
                    .collect(),
            },
            artifact,
            launch_directory,
        })
    }

    pub fn inventory_tool(&self) -> &Tool {
        &self.tool
    }

    /// The caller supplies ALL desired tools and reconciles that same set,
    /// holding this Store lock throughout preparation and reconciliation.
    pub async fn prepare_if_needed(
        &self,
        store: &Store,
        desired: &[Tool],
        transport: &UvTransport,
    ) -> Result<Option<PreparedUv>, Error> {
        self.prepare(store, desired, transport, crate::Preparation::IfNeeded)
            .await
    }

    /// Fresh preparation verifies a new complete artifact even for a healthy
    /// tool. Reuse requires a healthy selected generation with this exact tool.
    pub async fn prepare(
        &self,
        store: &Store,
        desired: &[Tool],
        transport: &UvTransport,
        preparation: crate::Preparation,
    ) -> Result<Option<PreparedUv>, Error> {
        if !cfg!(unix) {
            return Err(Error::UnsupportedTarget);
        }
        if !desired.contains(&self.tool) {
            return Err(Error::InventoryMismatch);
        }
        if store.reusable_tree(&self.tool, desired)?.is_some()
            && preparation == crate::Preparation::IfNeeded
        {
            return Ok(None);
        }
        self.download(transport).await.map(Some)
    }

    /// Download/verify/extract only, with no installation or execution.
    pub async fn download(&self, transport: &UvTransport) -> Result<PreparedUv, Error> {
        let path = self
            .artifact
            .url
            .strip_prefix(uv::ORIGIN)
            .ok_or(Error::InvalidLock)?;
        let bytes = transport
            .client
            .download_verified(
                &format!("{}{path}", transport.origin),
                ExpectedSha256::from_hex(&self.artifact.sha256)?,
                turborepo_download::Limits::new(128 * 1024 * 1024, Duration::from_secs(120))?,
            )
            .await?;
        let required: Vec<_> = self
            .artifact
            .executables
            .values()
            .map(String::as_str)
            .collect();
        let tree = turborepo_archive::extract(
            &bytes,
            turborepo_archive::Format::TarGz,
            turborepo_archive::Limits::new(512 * 1024 * 1024, 64, 4096, 8)?,
            Layout {
                root: self
                    .artifact
                    .root_prefix
                    .as_deref()
                    .ok_or(Error::InvalidLock)?,
                required_files: &required,
            },
        )?;
        if tree.root_path().join(&self.launch_directory).try_exists()? {
            return Err(Error::InvalidLock);
        }
        Ok(PreparedUv {
            tree,
            tool: self.tool.clone(),
            launch_directory: self.launch_directory.clone(),
            entrypoints: self.artifact.executables.clone(),
        })
    }
}

impl PreparedUv {
    /// The verified extracted uv binary, for offline queries during
    /// resolution only. Its private scratch tree is removed on drop.
    pub fn binary(&self) -> std::path::PathBuf {
        self.tree.root_path().join(&self.entrypoints["uv"])
    }
}

pub struct PreparedUv {
    tree: ExtractedArtifact,
    tool: Tool,
    launch_directory: String,
    entrypoints: std::collections::BTreeMap<String, String>,
}

impl PreparedUv {
    /// Callback for Store::reconcile. Copy the FULL verified tree, then add
    /// launchers that export the repo-scoped interpreter location.
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
        let launchers = destination.join(&self.launch_directory);
        fs::create_dir(&launchers)?;
        for (name, path) in &self.tool.executables {
            let script = LAUNCHER
                .replace("@LAUNCH@", &self.launch_directory)
                .replace("@ENTRYPOINT@", &self.entrypoints[name]);
            fs::write(destination.join(path), script)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(destination.join(path), fs::Permissions::from_mode(0o755))?;
            }
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests;
