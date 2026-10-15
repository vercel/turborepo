//! Locked Go adapter: one verified official archive (a `.tar.gz`, or the
//! `.zip` on Windows) becomes the complete `go/` tree exposing `go` and
//! `gofmt` (`go.exe` and `gofmt.exe` on Windows). No resolution, module
//! downloads, or subprocesses. Plans compose with the complete desired
//! inventory; preparation never publishes.

use std::{fs, time::Duration};

use turborepo_archive::{ExtractedArtifact, Layout};
use turborepo_download::ExpectedSha256;
use turborepo_tool_install::{Store, Tool};

use crate::{
    go_discovery::GoVersion,
    go_resolution::{self, ARTIFACT_ORIGIN, GoTransport},
    lock::{self, Installation, Lock, Platform},
};

/// Runtime environment every managed `go` invocation must receive.
/// `GOTOOLCHAIN=local` keeps the go command on this locked toolchain instead
/// of switching to, and downloading, a release named by `go`/`toolchain`
/// lines, `go env -w`, or the caller's environment.
pub const RUNTIME_ENV: [(&str, &str); 1] = [("GOTOOLCHAIN", "local")];
// Exactly `go/test`, Go's own compiler regression suite: unused by the go
// command, and it contains PAX-named non-ASCII fixtures. A Go-only constant;
// no lock, index, or repository input can change it.
const EXCLUDED: &[&str] = &["test"];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid locked Go artifact or mappings; regenerate turbo.lock")]
    InvalidLock,
    #[error("no locked Go archive for this target; update turbo.lock on a supported target")]
    MissingTarget,
    #[error(
        "managed Go promotion is not qualified for Windows arm64 or musl; use a supported GNU \
         Linux, macOS, or Windows x64 target"
    )]
    UnsupportedTarget,
    #[error("the complete desired inventory must include this Go plan unchanged")]
    InventoryMismatch,
    #[error(transparent)]
    Download(#[from] turborepo_download::Error),
    #[error(transparent)]
    Archive(#[from] turborepo_archive::Error),
    #[error(transparent)]
    Install(#[from] turborepo_tool_install::Error),
}

pub struct GoPlan {
    tool: Tool,
    artifact: lock::Artifact,
}

impl GoPlan {
    /// Validate only the selected platform against the official layout,
    /// without modifying the lock or inferring executables.
    pub fn from_lock(lock: &Lock, platform: Platform) -> Result<Self, Error> {
        let (_, spelling) = crate::node::lock_target(platform).ok_or(Error::UnsupportedTarget)?;
        let go = lock.tools().get("go").ok_or(Error::InvalidLock)?;
        let version = GoVersion::parse(&go.version)
            .filter(|v| v.is_toolchain() && v.to_string() == go.version)
            .ok_or(Error::InvalidLock)?;
        let Installation::Managed { artifacts } = &go.installation else {
            return Err(Error::InvalidLock);
        };
        if go.adapter != "go" || !go.options.is_empty() {
            return Err(Error::InvalidLock);
        }
        let parts = artifacts.get(&platform).ok_or(Error::MissingTarget)?;
        let (name, format) =
            go_resolution::archive(version, platform).ok_or(Error::UnsupportedTarget)?;
        let artifact = match parts.get("distribution") {
            Some(artifact) if parts.len() == 1 => artifact.clone(),
            _ => return Err(Error::InvalidLock),
        };
        if artifact.url != format!("{ARTIFACT_ORIGIN}/go/{name}")
            || artifact.format != format
            || artifact.root_prefix.as_deref() != Some("go")
            || artifact.destination.is_some()
            || artifact.executables != go_resolution::executables(format)
        {
            return Err(Error::InvalidLock);
        }
        Ok(Self {
            tool: Tool {
                id: "go".into(),
                version: go.version.clone(),
                platform: spelling.into(),
                artifact_sha256: artifact.sha256.clone(),
                executables: artifact.executables.clone(),
            },
            artifact,
        })
    }

    pub fn inventory_tool(&self) -> &Tool {
        &self.tool
    }

    /// The caller supplies ALL desired tools and reconciles that same set while
    /// holding this Store lock. A healthy identical Go tree needs no download
    /// unless preparation is fresh.
    pub async fn prepare(
        &self,
        store: &Store,
        desired: &[Tool],
        transport: &GoTransport,
        preparation: crate::Preparation,
    ) -> Result<Option<PreparedGo>, Error> {
        // Unix trees promote on Unix and Windows x64 trees only on native
        // Windows; Windows arm64 stays unqualified, as for Node.
        if !crate::node_provision::promotable(&self.tool) {
            return Err(Error::UnsupportedTarget);
        }
        if !desired.contains(&self.tool) {
            return Err(Error::InventoryMismatch);
        }
        // Fresh skips reuse, never validation of the complete desired cohort.
        Store::validate_desired(desired)?;
        if preparation == crate::Preparation::IfNeeded
            && store.reusable_tree(&self.tool, desired)?.is_some()
        {
            return Ok(None);
        }
        self.download(transport).await.map(Some)
    }

    /// Download, verify, and extract only; no installation or probing.
    pub async fn download(&self, transport: &GoTransport) -> Result<PreparedGo, Error> {
        let path = self
            .artifact
            .url
            .strip_prefix(ARTIFACT_ORIGIN)
            .ok_or(Error::InvalidLock)?;
        let bytes = transport
            .client
            .download_verified(
                &format!("{}{path}", transport.artifacts),
                ExpectedSha256::from_hex(&self.artifact.sha256)?,
                turborepo_download::Limits::new(256 * 1024 * 1024, Duration::from_secs(300))?,
            )
            .await?;
        // `from_lock` bound the format to the platform's official archive.
        let format = match self.artifact.format {
            lock::Format::TarGz => turborepo_archive::Format::TarGz,
            lock::Format::Zip => turborepo_archive::Format::Zip,
            _ => return Err(Error::InvalidLock),
        };
        // Both formats share the `go/test` exclusion: the Windows zip carries
        // the same UTF-8 named fixtures, flagged rather than PAX-named.
        let required: Vec<_> = self.tool.executables.values().map(String::as_str).collect();
        let tree = turborepo_archive::extract_excluding(
            &bytes,
            format,
            turborepo_archive::Limits::new(1024 * 1024 * 1024, 65_536, 4096, 64)?,
            Layout {
                root: "go",
                required_files: &required,
            },
            EXCLUDED,
        )?;
        Ok(PreparedGo {
            tree,
            tool: self.tool.clone(),
        })
    }
}

pub struct PreparedGo {
    tree: ExtractedArtifact,
    tool: Tool,
}

impl PreparedGo {
    /// Callback for Store reconciliation: copy the verified tree, preserving
    /// executable modes; never run hooks.
    pub fn stage(
        &self,
        tool: &Tool,
        destination: &std::path::Path,
    ) -> Result<(), turborepo_tool_install::Error> {
        if tool != &self.tool
            || fs::read_dir(destination)?.next().is_some()
            || fs::symlink_metadata(destination)?.file_type().is_symlink()
        {
            return Err(turborepo_tool_install::Error::InvalidInventory);
        }
        crate::node_provision::copy_tree(&self.tree.root_path(), destination)?;
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests;
#[cfg(test)]
mod zip_tests;
