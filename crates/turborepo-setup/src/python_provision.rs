//! Locked CPython adapter: a verified python-build-standalone archive staged at
//! uv's key directory inside the generation's `python` tool, which is where the
//! managed uv launchers point `UV_PYTHON_INSTALL_DIR`. No resolution, no uv
//! installer (it writes absolute links that a relocatable generation rejects)
//! and no system Python. The only subprocess is the verified interpreter
//! compiling its own standard library while staging.
//!
//! Windows x64 builds keep `python.exe` beside its DLLs and `Lib`, which it
//! finds from its own image path, so a hard link in the managed bin could not
//! start. Windows exports a batch launcher that runs the interpreter in
//! place instead, from a dot directory that uv's install-directory scan skips.

use std::{
    collections::BTreeMap,
    fs, io,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

use sha2::{Digest, Sha256};
use turborepo_archive::{Allowances, ExtractedArtifact, Layout};
use turborepo_download::ExpectedSha256;
use turborepo_tool_install::{Store, Tool};

use crate::{
    lock::{self, Installation, Lock, Platform},
    node_provision::copy_tree,
    python_resolution::{PythonTransport, is_official},
    uv::{ORIGIN, exact_release},
};

/// The official builds rename `python/install/...` headers in place, leaving
/// the old name's tail after the NUL, and their Linux terminfo has case-only
/// siblings that are different terminals (`E/Eterm`, `e/eterm`).
const OFFICIAL_ARCHIVE: Allowances = Allowances {
    tar_name_residue: true,
    case_sensitive_dir: Some("share/terminfo"),
};

/// Windows launcher beside uv's key directory: `%~dp0` is its own directory,
/// and the interpreter keeps its image path inside the build.
const WINDOWS_LAUNCHER: &str = "@\"%~dp0..\\@KEY@\\@ENTRYPOINT@\" %*\r\n";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid locked Python artifact or mappings; regenerate turbo.lock")]
    InvalidLock,
    #[error(
        "no locked Python build for this target; update turbo.lock on a supported target or \
         configure verify-system"
    )]
    MissingTarget,
    #[error(
        "managed Python promotion is not qualified for Windows arm64 or musl; use macOS, GNU \
         Linux or Windows x64, or configure verify-system"
    )]
    UnsupportedTarget,
    #[error("the complete desired inventory must include this Python plan unchanged")]
    InventoryMismatch,
    #[error(transparent)]
    Download(#[from] turborepo_download::Error),
    #[error(transparent)]
    Archive(#[from] turborepo_archive::Error),
    #[error(transparent)]
    Install(#[from] turborepo_tool_install::Error),
    #[error("Python staging I/O failed")]
    Io(#[from] io::Error),
}

pub struct PythonPlan {
    tool: Tool,
    artifact: lock::Artifact,
    key: String,
    /// The versioned interpreter; `bin/python3` is a relative link to it.
    /// Windows: `python.exe` at the build's top level.
    interpreter: String,
    /// The standard library, including site-packages. Only this directory is
    /// importable; other `lib` trees (Tix data in 3.8) are not Python.
    /// Windows: `Lib`.
    stdlib: String,
    /// The Windows launchers' directory; Unix exports link into the build.
    launch_directory: Option<String>,
}

impl PythonPlan {
    /// Validate only the selected platform against the official layout.
    pub fn from_lock(lock: &Lock, platform: Platform) -> Result<Self, Error> {
        let windows = match platform {
            Platform::MacosX64
            | Platform::MacosArm64
            | Platform::LinuxX64Gnu
            | Platform::LinuxArm64Gnu => false,
            Platform::WindowsX64 => true,
            _ => return Err(Error::UnsupportedTarget),
        };
        let locked = lock.tools().get("python").ok_or(Error::InvalidLock)?;
        let version = exact_release(&locked.version).ok_or(Error::InvalidLock)?;
        let Installation::Managed { artifacts } = &locked.installation else {
            return Err(Error::InvalidLock);
        };
        if locked.adapter != "python" || !locked.options.is_empty() {
            return Err(Error::InvalidLock);
        }
        let parts = artifacts.get(&platform).ok_or(Error::MissingTarget)?;
        let artifact = match parts.get("distribution") {
            Some(artifact) if parts.len() == 1 => artifact.clone(),
            _ => return Err(Error::InvalidLock),
        };
        if !is_official(&version, platform, &artifact) {
            return Err(Error::InvalidLock);
        }
        ExpectedSha256::from_hex(&artifact.sha256)?;
        let key = artifact.destination.clone().ok_or(Error::InvalidLock)?;
        let spelling = serde_json::to_value(platform)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .ok_or(Error::InvalidLock)?;
        let minor = format!("{}.{}", version.major, version.minor);
        let (executables, interpreter, stdlib, launch_directory) = if windows {
            // Launcher semantics are part of reuse identity, as for uv.
            let launch = format!(
                ".launch-{}",
                &hex::encode(Sha256::digest(WINDOWS_LAUNCHER))[..16]
            );
            let executables = artifact
                .executables
                .keys()
                .map(|name| (name.clone(), format!("{launch}/{name}.cmd")))
                .collect();
            (executables, "python.exe".into(), "Lib".into(), Some(launch))
        } else {
            let executables = artifact
                .executables
                .iter()
                .map(|(name, path)| (name.clone(), format!("{key}/{path}")))
                .collect();
            let (interpreter, stdlib) =
                (format!("bin/python{minor}"), format!("lib/python{minor}"));
            (executables, interpreter, stdlib, None)
        };
        Ok(Self {
            tool: Tool {
                id: "python".into(),
                version: locked.version.clone(),
                platform: spelling,
                artifact_sha256: artifact.sha256.clone(),
                executables,
            },
            artifact,
            key,
            interpreter,
            stdlib,
            launch_directory,
        })
    }

    pub fn inventory_tool(&self) -> &Tool {
        &self.tool
    }

    /// uv's download key for this build, which is also its directory name.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The caller supplies ALL desired tools and reconciles that same set,
    /// holding this Store lock throughout preparation and reconciliation.
    pub async fn prepare(
        &self,
        store: &Store,
        desired: &[Tool],
        transport: &PythonTransport,
        preparation: crate::Preparation,
    ) -> Result<Option<PreparedPython>, Error> {
        if !host_matches(self.launch_directory.is_some()) {
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
        let path = self
            .artifact
            .url
            .strip_prefix(ORIGIN)
            .ok_or(Error::InvalidLock)?;
        let bytes = transport
            .client
            .download_verified(
                &format!("{}{path}", transport.origin),
                ExpectedSha256::from_hex(&self.artifact.sha256)?,
                turborepo_download::Limits::new(256 * 1024 * 1024, Duration::from_secs(300))?,
            )
            .await?;
        let tree = turborepo_archive::extract_with(
            &bytes,
            turborepo_archive::Format::TarGz,
            turborepo_archive::Limits::new(1024 * 1024 * 1024, 200_000, 4096, 64)?,
            Layout {
                root: "python",
                required_files: &[&self.interpreter],
            },
            OFFICIAL_ARCHIVE,
        )?;
        Ok(Some(PreparedPython {
            tree,
            tool: self.tool.clone(),
            key: self.key.clone(),
            interpreter: self.interpreter.clone(),
            stdlib: self.stdlib.clone(),
            launch_directory: self.launch_directory.clone(),
            entrypoints: self.artifact.executables.clone(),
        }))
    }
}

pub struct PreparedPython {
    tree: ExtractedArtifact,
    tool: Tool,
    key: String,
    interpreter: String,
    stdlib: String,
    launch_directory: Option<String>,
    entrypoints: BTreeMap<String, String>,
}

impl PreparedPython {
    /// Callback for Store::reconcile: copy the FULL verified interpreter tree
    /// into uv's key directory, preserving relative links and modes. Windows
    /// adds the batch launchers that the Store's forwarders run.
    pub fn stage(
        &self,
        tool: &Tool,
        destination: &Path,
    ) -> Result<(), turborepo_tool_install::Error> {
        if !host_matches(self.launch_directory.is_some()) {
            return Err(turborepo_tool_install::Error::UnsupportedPlatform);
        }
        if tool != &self.tool
            || fs::symlink_metadata(destination)?.file_type().is_symlink()
            || fs::read_dir(destination)?.next().is_some()
        {
            return Err(turborepo_tool_install::Error::InvalidInventory);
        }
        let installation = destination.join(&self.key);
        fs::create_dir(&installation)?;
        copy_tree(&self.tree.root_path(), &installation)?;
        self.compile_bytecode(&installation)?;
        if let Some(launch) = &self.launch_directory {
            let launchers = destination.join(launch);
            fs::create_dir(&launchers)?;
            for name in self.tool.executables.keys() {
                let script = WINDOWS_LAUNCHER
                    .replace("@KEY@", &self.key)
                    .replace("@ENTRYPOINT@", &self.entrypoints[name].replace('/', "\\"));
                fs::write(launchers.join(format!("{name}.cmd")), script)?;
            }
        }
        Ok(())
    }

    /// The Unix builds ship almost no bytecode, so the first import of the
    /// stdlib or pip would write `__pycache__` into the hashed tree and damage
    /// it. Compile it before the Store hashes the tree, with the verified
    /// interpreter isolated from the environment and the repository.
    /// Unchecked-hash bytecode never consults source mtimes, which neither the
    /// archive nor the copy preserves.
    fn compile_bytecode(&self, installation: &Path) -> io::Result<()> {
        let status = compile_command(installation, &self.interpreter, &self.stdlib).status()?;
        if !status.success() {
            return Err(io::Error::other(
                "the verified Python could not compile its standard library",
            ));
        }
        Ok(())
    }
}

fn compile_command(installation: &Path, interpreter: &str, stdlib: &str) -> Command {
    let mut command = Command::new(installation.join(interpreter));
    command
        .args(["-I", "-B", "-m", "compileall", "-q", "-f", "-j", "0"])
        .args(["--invalidation-mode", "unchecked-hash", stdlib])
        .current_dir(installation)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Windows processes need SYSTEMROOT: without it Winsock cannot start,
    // and compileall's worker processes import `socket`.
    if cfg!(windows)
        && let Some(root) = std::env::var_os("SYSTEMROOT")
    {
        command.env("SYSTEMROOT", root);
    }
    command
}

/// Unix builds run in place and Windows builds add launchers, so each
/// installs only on its own host family.
fn host_matches(windows: bool) -> bool {
    if cfg!(windows) {
        windows
    } else {
        cfg!(unix) && !windows
    }
}

#[cfg(all(test, unix))]
mod tests;
#[cfg(all(test, windows))]
mod windows_tests;
