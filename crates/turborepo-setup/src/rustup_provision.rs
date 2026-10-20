//! Locked rustup adapter. Verified official installer bytes are staged as a
//! repository-scoped multicall `rustup` (`rustup.exe` on Windows); the
//! installer itself never runs, so no shell profile, PATH, registry key,
//! `~/.rustup` or `~/.cargo` can change. No Rust toolchain is installed here.

use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use turborepo_download::{ApprovedOrigin, DownloadClient, ExpectedSha256, Limits};
use turborepo_tool_install::{Store, Tool};

use crate::{
    lock::{self, Artifact, Declaration, Format, Installation, Lock, Platform},
    rustup::{self, ORIGIN},
};

const PART: &str = "rustup-init";
const MAX_INSTALLER_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid locked rustup artifact or mappings; regenerate turbo.lock")]
    InvalidLock,
    #[error("no locked rustup artifact for this target; update turbo.lock on a supported target")]
    MissingTarget,
    #[error(
        "managed rustup promotion is not qualified for this host, Windows arm64 or musl; use a \
         supported macOS, GNU Linux or Windows x64 target"
    )]
    UnsupportedTarget,
    #[error("the complete desired inventory must include this rustup plan unchanged")]
    InventoryMismatch,
    #[error(transparent)]
    Metadata(#[from] rustup::Error),
    #[error(transparent)]
    Lock(#[from] lock::Error),
    #[error(transparent)]
    Download(#[from] turborepo_download::Error),
    #[error(transparent)]
    Install(#[from] turborepo_tool_install::Error),
    #[error("rustup staging I/O failed")]
    Io(#[from] io::Error),
}

pub struct RustupTransport {
    client: DownloadClient,
    // Mirror and test transports change only the origin; lock provenance
    // remains official. The private rustup downloads from the same origin.
    origin: String,
    /// Fixture-only stand-in runner and its leading arguments.
    #[cfg(any(test, feature = "test-support"))]
    runner: Option<(PathBuf, Vec<std::ffi::OsString>)>,
}

impl RustupTransport {
    fn new(client: DownloadClient, origin: String) -> Self {
        Self {
            client,
            origin,
            #[cfg(any(test, feature = "test-support"))]
            runner: None,
        }
    }

    pub fn official(_policy: &crate::source_policy::ToolDownloadPolicy) -> Result<Self, Error> {
        let client = DownloadClient::new([ApprovedOrigin::https(ORIGIN)?])?;
        Ok(Self::new(client, ORIGIN.into()))
    }

    /// Send the official rustup and `dist` paths to a validated mirror base,
    /// which also becomes the private rustup's `RUSTUP_DIST_SERVER`. No
    /// redirect leaves its origin.
    pub fn mirrored(
        _policy: &crate::source_policy::ToolDownloadPolicy,
        mirror: &crate::mirror::Mirror,
    ) -> Result<Self, Error> {
        Ok(Self::new(mirror.client()?, mirror.base().into()))
    }

    /// Explicit fixture-only HTTP opt-in; literal loopback IPs only, no env
    /// override.
    #[cfg(any(test, feature = "test-support"))]
    pub fn loopback_http_for_tests(origin: &str) -> Result<Self, Error> {
        let client = DownloadClient::loopback_http_for_tests(origin)?;
        Ok(Self::new(client, origin.trim_end_matches('/').into()))
    }

    /// Fixture-only: run every private rustup as `<program> <args> <staged
    /// rustup> <rustup arguments>`, so a stand-in can check the staged binary
    /// and play rustup where the fixture image cannot.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_runner_for_tests(
        mut self,
        program: impl Into<PathBuf>,
        args: impl IntoIterator<Item = impl Into<std::ffi::OsString>>,
    ) -> Self {
        self.runner = Some((program.into(), args.into_iter().map(Into::into).collect()));
        self
    }

    pub(crate) async fn read(&self, path: &str, limit: usize) -> Result<Vec<u8>, Error> {
        Ok(self
            .client
            .read_metadata(
                &format!("{}{path}", self.origin),
                Limits::new(limit, Duration::from_secs(30))?,
            )
            .await?)
    }

    /// Current stable rustup with every native platform's published installer
    /// digest, read from the official origin over HTTPS like Node's SHASUMS.
    /// No artifact download, installation or execution.
    pub async fn resolve(&self, declarations: Vec<Declaration>) -> Result<lock::Tool, Error> {
        let release = self
            .read("/rustup/release-stable.toml", rustup::MAX_RELEASE_BYTES)
            .await?;
        let version = rustup::parse_release(&release)?;
        let mut artifacts = BTreeMap::new();
        for platform in rustup::PLATFORMS {
            let url = rustup::installer_url(&version, platform).ok_or(Error::InvalidLock)?;
            let path = url.strip_prefix(ORIGIN).ok_or(Error::InvalidLock)?;
            let checksum = self
                .read(&format!("{path}.sha256"), rustup::MAX_CHECKSUM_BYTES)
                .await?;
            let sha256 = rustup::parse_checksum(&checksum, rustup::installer_file(platform))?;
            artifacts.insert(
                platform,
                BTreeMap::from([(
                    PART.into(),
                    Artifact {
                        url,
                        sha256,
                        format: Format::Binary,
                        alternates: BTreeMap::new(),
                        root_prefix: None,
                        destination: None,
                        executables: BTreeMap::from([(
                            "rustup".into(),
                            rustup::binary_path(platform).into(),
                        )]),
                    },
                )]),
            );
        }
        let tool = lock::Tool {
            adapter: "rustup".into(),
            version: version.to_string(),
            declarations,
            options: BTreeMap::new(),
            installation: Installation::Managed { artifacts },
        };
        let candidate = Lock::new(lock::Document {
            schema_version: lock::SCHEMA_VERSION,
            tools: BTreeMap::from([("rustup".into(), tool.clone())]),
        })?;
        for platform in rustup::PLATFORMS {
            RustupPlan::from_lock(&candidate, platform)?;
        }
        Ok(tool)
    }
}

pub struct RustupPlan {
    tool: Tool,
    artifact: Artifact,
}

impl RustupPlan {
    /// Validate only the selected platform's official installer and mapping,
    /// without modifying the lock.
    pub fn from_lock(lock: &Lock, platform: Platform) -> Result<Self, Error> {
        let (_, spelling) = rustup::target(platform).ok_or(Error::UnsupportedTarget)?;
        let rustup = lock.tools().get("rustup").ok_or(Error::InvalidLock)?;
        let version = rustup::canonical_release(&rustup.version).ok_or(Error::InvalidLock)?;
        let Installation::Managed { artifacts } = &rustup.installation else {
            return Err(Error::InvalidLock);
        };
        if rustup.adapter != "rustup" || !rustup.options.is_empty() {
            return Err(Error::InvalidLock);
        }
        let parts = artifacts.get(&platform).ok_or(Error::MissingTarget)?;
        let artifact = parts.get(PART).ok_or(Error::InvalidLock)?;
        let executables = BTreeMap::from([(
            "rustup".to_owned(),
            rustup::binary_path(platform).to_owned(),
        )]);
        if parts.len() != 1
            || Some(&artifact.url) != rustup::installer_url(&version, platform).as_ref()
            || artifact.format != Format::Binary
            || artifact.root_prefix.is_some()
            || artifact.destination.is_some()
            || artifact.executables != executables
        {
            return Err(Error::InvalidLock);
        }
        Ok(Self {
            tool: Tool {
                id: "rustup".into(),
                version: version.to_string(),
                platform: spelling.into(),
                artifact_sha256: artifact.sha256.clone(),
                executables,
            },
            artifact: artifact.clone(),
        })
    }

    pub fn inventory_tool(&self) -> &Tool {
        &self.tool
    }

    /// The caller supplies ALL desired tools and reconciles that same set while
    /// holding this Store lock. A healthy identical tool needs no download.
    pub async fn prepare(
        &self,
        store: &Store,
        desired: &[Tool],
        transport: &RustupTransport,
        preparation: crate::Preparation,
    ) -> Result<Option<PreparedRustup>, Error> {
        if !rustup::promotable(&self.tool.platform) {
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
                Limits::new(MAX_INSTALLER_BYTES, Duration::from_secs(120))?,
            )
            .await?;
        Ok(Some(PreparedRustup {
            bytes: bytes.into_bytes(),
            tool: self.tool.clone(),
        }))
    }
}

impl RustupPlan {
    /// The verified multicall binary toolchain installs run: freshly prepared
    /// bytes staged into the empty `scratch`, or the healthy selected tree.
    pub fn executable(
        &self,
        store: &Store,
        desired: &[Tool],
        prepared: Option<&PreparedRustup>,
        scratch: &Path,
    ) -> Result<PathBuf, Error> {
        let tree = match prepared {
            Some(prepared) => {
                prepared.stage(&self.tool, scratch)?;
                scratch.to_owned()
            }
            None => store
                .reusable_tree(&self.tool, desired)?
                .ok_or(Error::InventoryMismatch)?,
        };
        // Split components: Windows images must not carry a lock path's `/`
        // (a `cmd.exe` image there cannot even run its own commands).
        let path = self.tool.executables["rustup"].split('/');
        Ok(path.fold(tree, |tree, part| tree.join(part)))
    }
}

/// Verified installer bytes, not yet staged or executed.
pub struct PreparedRustup {
    bytes: Vec<u8>,
    tool: Tool,
}

impl PreparedRustup {
    /// Verified installer bytes transferred for this preparation.
    pub fn downloaded_bytes(&self) -> u64 {
        self.bytes.len() as u64
    }

    /// Callback for Store::reconcile: write the one owner-executable multicall
    /// binary. Toolchains, proxies and homes are never created here.
    pub fn stage(
        &self,
        tool: &Tool,
        destination: &Path,
    ) -> Result<(), turborepo_tool_install::Error> {
        let invalid = turborepo_tool_install::Error::InvalidInventory;
        if tool != &self.tool
            || fs::symlink_metadata(destination)?.file_type().is_symlink()
            || fs::read_dir(destination)?.next().is_some()
        {
            return Err(invalid);
        }
        let path = destination.join(self.tool.executables.get("rustup").ok_or(invalid)?);
        fs::create_dir(
            path.parent()
                .ok_or(turborepo_tool_install::Error::UnsafePath)?,
        )?;
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o755);
        // Store flushes the complete staged tree before publication.
        io::Write::write_all(&mut options.open(path)?, &self.bytes)?;
        Ok(())
    }
}

/// Private `RUSTUP_HOME` and `CARGO_HOME` passed explicitly to every managed
/// rustup process, never the user's `~/.rustup` or `~/.cargo`.
pub struct PrivateHomes {
    rustup: PathBuf,
    cargo: PathBuf,
}

impl PrivateHomes {
    /// Create both homes as new owner-only directories inside `parent`.
    pub fn create(parent: &Path) -> io::Result<Self> {
        let homes = Self {
            rustup: parent.join("rustup"),
            cargo: parent.join("cargo"),
        };
        for path in [&homes.rustup, &homes.cargo] {
            let mut builder = fs::DirBuilder::new();
            builder.recursive(false);
            #[cfg(unix)]
            std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
            builder.create(path)?;
        }
        Ok(homes)
    }

    pub fn rustup_home(&self) -> &Path {
        &self.rustup
    }

    pub fn cargo_home(&self) -> &Path {
        &self.cargo
    }

    /// A rustup invocation confined to these homes and the transport's
    /// distribution, official or mirrored: caller home, toolchain and mirror
    /// overrides are replaced, rustup never auto-installs or self-updates, and
    /// PATH is left untouched. rustup checks every component against the
    /// channel manifest; callers bind its `.tar.gz` entries to `turbo.lock`.
    pub fn command(&self, rustup: &Path, transport: &RustupTransport) -> Command {
        let origin = &transport.origin;
        // Under a canonical `\\?\` store, rustup's image and homes get the
        // ordinary spelling unless only the verbatim one is exact.
        #[cfg(windows)]
        let (rustup, homes) = (
            dunce::simplified(rustup),
            [&self.rustup, &self.cargo].map(|home| dunce::simplified(home)),
        );
        #[cfg(not(windows))]
        let homes = [&self.rustup, &self.cargo].map(PathBuf::as_path);
        #[cfg(any(test, feature = "test-support"))]
        let mut command = match &transport.runner {
            Some((program, args)) => {
                let mut command = Command::new(program);
                command.args(args).arg(rustup);
                command
            }
            None => Command::new(rustup),
        };
        #[cfg(not(any(test, feature = "test-support")))]
        let mut command = Command::new(rustup);
        command
            .env("RUSTUP_HOME", homes[0])
            .env("CARGO_HOME", homes[1])
            .env("RUSTUP_DIST_SERVER", origin)
            .env("RUSTUP_UPDATE_ROOT", format!("{origin}/rustup"))
            .env("RUSTUP_AUTO_INSTALL", "0")
            .env_remove("RUSTUP_TOOLCHAIN")
            .env_remove("RUSTUP_DIST_ROOT");
        command
    }
}

#[cfg(all(test, unix))]
mod tests;

#[cfg(all(test, windows))]
mod windows_tests;
