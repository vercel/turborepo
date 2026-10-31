//! Locked uv adapter: verified official archive and full resource tree. No
//! resolution, Python installation, dependency sync or subprocesses. Plans
//! compose with the complete desired inventory; preparation never publishes.
//!
//! Unix exports `uv`/`uvx` as Store launchers that set the interpreter
//! variables. Windows x64 exports the official `uv.exe` and `uvx.exe` as the
//! Store's hard links, so programs that start `uv` without a shell still find
//! it. Both are self-contained, and `uvx.exe` starts the `uv.exe` exported
//! beside it. The managed activation that `turbo run`, `turbo exec` and
//! setup's own `uv sync` apply supplies the variables there.

use std::{collections::BTreeMap, fs, io, path::Path, time::Duration};

use turborepo_archive::{ExtractedArtifact, Layout};
use turborepo_download::{ExpectedSha256, VerifiedArtifact};
use turborepo_tool_install::{
    Export, Launch, ManagedVar, SetupSession, Staging, Tool, TreeRef, VarValue,
};

use crate::{
    lock::{self, Installation, Lock, Platform},
    node_provision::copy_tree,
    uv::{self, UvArtifact},
    uv_resolution::UvTransport,
};

/// uv's interpreter settings. They pin uv's interpreter directory to the
/// selected `python` tool's tree, so managed interpreters never land in the
/// user's global uv data. uv must not download interpreters into an
/// immutable tree. With a locked
/// interpreter (`python`, its uv download key), uv uses only that tree and
/// requests it by key rather than path: a path request makes `uv pip` install
/// into the managed interpreter itself instead of the project's virtual
/// environment.
pub(crate) fn interpreter_vars(python: Option<String>) -> BTreeMap<ManagedVar, VarValue> {
    let mut vars = BTreeMap::from([
        (
            ManagedVar::UvPythonDownloads,
            VarValue::Word("never".into()),
        ),
        (
            ManagedVar::UvPythonInstallDir,
            VarValue::Tree(TreeRef::new("python", "")),
        ),
    ]);
    if let Some(key) = python {
        vars.insert(
            ManagedVar::UvPythonPreference,
            VarValue::Word("only-managed".into()),
        );
        vars.insert(ManagedVar::UvPython, VarValue::Word(key));
    }
    vars
}

/// Bump when this adapter stages different bytes for the same staging inputs.
const STAGING_REVISION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid locked uv artifact or mappings; regenerate turbo.lock")]
    InvalidLock,
    #[error("no locked uv artifact for this target; update turbo.lock on a supported target")]
    MissingTarget,
    #[error(
        "managed uv promotion is not qualified for Windows arm64 or musl; use macOS, GNU Linux or \
         Windows x64"
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
    windows: bool,
}

impl UvPlan {
    /// Validate only the selected platform's metadata against the official
    /// layout, without modifying the lock or inferring other executables.
    pub fn from_lock(lock: &Lock, platform: Platform) -> Result<Self, Error> {
        let windows = match platform {
            Platform::MacosX64
            | Platform::MacosArm64
            | Platform::LinuxX64Gnu
            | Platform::LinuxArm64Gnu => false,
            Platform::WindowsX64 => true,
            _ => return Err(Error::UnsupportedTarget),
        };
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
        let executables = if windows {
            // `uv.exe` and `uvx.exe` at the zip's top level.
            Export::files(&artifact.executables)
        } else {
            artifact
                .executables
                .iter()
                .map(|(name, path)| {
                    let program = TreeRef::new("uv", path.clone());
                    let launch = Launch {
                        program,
                        args: Vec::new(),
                    };
                    (name.clone(), Export::Launch(launch))
                })
                .collect()
        };
        let spelling = serde_json::to_value(platform)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .ok_or(Error::InvalidLock)?;
        let python = lock
            .tools()
            .get("python")
            .map(|python| {
                crate::python_resolution::installed_key(&python.version, &spelling)
                    .ok_or(Error::InvalidLock)
            })
            .transpose()?;
        Ok(Self {
            tool: Tool {
                id: "uv".into(),
                version: locked.version.clone(),
                platform: spelling,
                artifact_sha256: artifact.sha256.clone(),
                executables,
                vars: interpreter_vars(python),
                staging: Staging::new(
                    "uv",
                    STAGING_REVISION,
                    &crate::staging::Archive {
                        format: artifact.format,
                        root: artifact.root_prefix.as_deref().unwrap_or_default(),
                    },
                )?,
            },
            artifact,
            windows,
        })
    }

    pub fn inventory_tool(&self) -> &Tool {
        &self.tool
    }

    /// The caller supplies ALL desired tools and ensures that same set
    /// in the same setup session.
    pub async fn prepare_if_needed(
        &self,
        store: &SetupSession<'_>,
        desired: &[Tool],
        transport: &UvTransport,
    ) -> Result<Option<PreparedUv>, Error> {
        self.prepare(store, desired, transport, crate::Preparation::IfNeeded)
            .await
    }

    /// Fresh preparation verifies a new complete artifact even for a healthy
    /// tool. Reuse requires a sealed tree for this exact tool.
    pub async fn prepare(
        &self,
        store: &SetupSession<'_>,
        desired: &[Tool],
        transport: &UvTransport,
        preparation: crate::Preparation,
    ) -> Result<Option<PreparedUv>, Error> {
        if !host_matches(self.windows) {
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
        let bytes = self.fetch(transport).await?;
        self.extract(&bytes)
    }

    /// Like `download`, but keeps the verified archive for one later
    /// preparation in the same setup run, so resolving Python and then
    /// provisioning the same uv downloads it once.
    pub async fn download_retained(&self, transport: &UvTransport) -> Result<PreparedUv, Error> {
        let bytes = self.fetch(transport).await?;
        let prepared = self.extract(&bytes)?;
        if let Ok(mut retained) = transport.retained.lock() {
            *retained = Some(bytes.into_bytes());
        }
        Ok(prepared)
    }

    async fn fetch(&self, transport: &UvTransport) -> Result<VerifiedArtifact, Error> {
        let expected = || ExpectedSha256::from_hex(&self.artifact.sha256);
        let retained = transport.retained.lock().ok().and_then(|mut r| r.take());
        // Bytes kept for another selection fail verification and are dropped.
        if let Some(bytes) = retained
            && let Ok(verified) = VerifiedArtifact::verify_bytes(bytes, expected()?)
        {
            return Ok(verified);
        }
        let path = self
            .artifact
            .url
            .strip_prefix(uv::ORIGIN)
            .ok_or(Error::InvalidLock)?;
        Ok(transport
            .client
            .download_verified(
                &format!("{}{path}", transport.origin),
                expected()?,
                turborepo_download::Limits::new(128 * 1024 * 1024, Duration::from_secs(120))?,
            )
            .await?)
    }

    fn extract(&self, bytes: &VerifiedArtifact) -> Result<PreparedUv, Error> {
        let required: Vec<_> = self
            .artifact
            .executables
            .values()
            .map(String::as_str)
            .collect();
        // The plan matched the official layout: Unix tarballs nest under
        // `uv-<target>/`, while Windows zips are flat.
        let (format, root) = match (self.artifact.format, &self.artifact.root_prefix) {
            (lock::Format::TarGz, Some(root)) => (turborepo_archive::Format::TarGz, root.as_str()),
            (lock::Format::Zip, None) => (turborepo_archive::Format::Zip, ""),
            _ => return Err(Error::InvalidLock),
        };
        let tree = turborepo_archive::extract(
            bytes,
            format,
            turborepo_archive::Limits::new(512 * 1024 * 1024, 64, 4096, 8)?,
            Layout {
                root,
                required_files: &required,
            },
        )?;
        Ok(PreparedUv {
            tree,
            tool: self.tool.clone(),
            // uv has no shared download cache. Archive bytes retained from
            // resolution were still transferred during this setup run.
            downloaded_bytes: bytes.as_bytes().len() as u64,
            windows: self.windows,
            entrypoints: self.artifact.executables.clone(),
        })
    }
}

impl PreparedUv {
    /// Verified archive bytes transferred for this preparation.
    pub fn downloaded_bytes(&self) -> u64 {
        self.downloaded_bytes
    }

    /// The verified extracted uv binary, for offline queries during
    /// resolution only. Its private scratch tree is removed on drop.
    pub fn binary(&self) -> std::path::PathBuf {
        self.tree.root_path().join(&self.entrypoints["uv"])
    }
}

pub struct PreparedUv {
    tree: ExtractedArtifact,
    tool: Tool,
    downloaded_bytes: u64,
    windows: bool,
    entrypoints: BTreeMap<String, String>,
}

impl PreparedUv {
    /// Staging callback for `SetupSession::ensure`: copy the FULL verified
    /// tree.
    pub fn stage(
        &self,
        tool: &Tool,
        destination: &Path,
    ) -> Result<(), turborepo_tool_install::Error> {
        if !host_matches(self.windows) {
            return Err(turborepo_tool_install::Error::UnsupportedPlatform);
        }
        if tool != &self.tool
            || fs::symlink_metadata(destination)?.file_type().is_symlink()
            || fs::read_dir(destination)?.next().is_some()
        {
            return Err(turborepo_tool_install::Error::InvalidInventory);
        }
        Ok(copy_tree(&self.tree.root_path(), destination)?)
    }
}

/// Unix plans export launchers and Windows plans export `.exe` files, so each
/// installs only on its own host family.
fn host_matches(windows: bool) -> bool {
    cfg!(any(unix, windows)) && windows == cfg!(windows)
}

#[cfg(all(test, unix))]
mod tests;
#[cfg(all(test, windows))]
mod windows_tests;
