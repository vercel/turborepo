//! Locked Rust toolchain adapter. The repository's verified private rustup
//! installs the exact locked toolchain into fresh private homes; rustup's
//! installed component set and manifest digests must then match the lock
//! before the home is staged. No dependency fetch or task execution.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::Stdio,
};

use sha2::{Digest, Sha256};
use turborepo_tool_install::{Store, Tool};

use crate::{
    lock::{self, AlternateFormat, Artifact, Format, Installation, Lock, Platform},
    rust_discovery::{Channel, Toolchain},
    rust_metadata::{MAX_MANIFEST_BYTES, Manifest},
    rust_resolution::{executables, toolchain_name},
    rustup,
    rustup_provision::PrivateHomes,
};

const DIST: &str = "https://static.rust-lang.org/dist/";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid locked Rust toolchain; regenerate turbo.lock")]
    InvalidLock,
    #[error(
        "no locked Rust toolchain for this target; update turbo.lock on a supported target or \
         configure verify-system"
    )]
    MissingTarget,
    #[error(
        "managed Rust promotion is not qualified for Windows or musl; use a supported GNU \
         Linux/macOS target or configure verify-system"
    )]
    UnsupportedTarget,
    #[error("the complete desired inventory must include this Rust plan unchanged")]
    InventoryMismatch,
    #[error("private rustup could not install the locked Rust toolchain ({0})")]
    Rustup(std::process::ExitStatus),
    #[error("the installed Rust toolchain does not match turbo.lock; nothing was published")]
    Mismatch,
    #[error(transparent)]
    Install(#[from] turborepo_tool_install::Error),
    #[error("Rust toolchain staging I/O failed")]
    Io(#[from] io::Error),
}

/// Whether a locked selection still satisfies the authored request: the same
/// semantic options and an exact toolchain the channel admits.
pub fn satisfies(toolchain: &Toolchain, tool: &lock::Tool) -> bool {
    let set = |values: &BTreeSet<String>| values.iter().cloned().collect::<Vec<_>>();
    let option = |key: &str| tool.options.get(key).cloned().unwrap_or_default();
    let version = tool.version.as_str();
    let channel = match &toolchain.channel {
        Channel::Stable => rustup::canonical_release(version).is_some(),
        Channel::Release(request) => {
            rustup::canonical_release(version).is_some()
                && (version == request || version.starts_with(&format!("{request}.")))
        }
        Channel::Beta(date) | Channel::Nightly(date) => {
            let name = if matches!(toolchain.channel, Channel::Beta(_)) {
                "beta"
            } else {
                "nightly"
            };
            Channel::parse(version).is_some()
                && date
                    .as_ref()
                    .map_or(version.starts_with(&format!("{name}-")), |date| {
                        version == format!("{name}-{date}")
                    })
        }
    };
    channel
        && option("profile") == [toolchain.profile.name()]
        && option("components") == set(&toolchain.components)
        && option("targets") == set(&toolchain.targets)
        && tool.options.len()
            == 1 + usize::from(!toolchain.components.is_empty())
                + usize::from(!toolchain.targets.is_empty())
}

pub struct RustPlan {
    tool: Tool,
    host: &'static str,
    profile: String,
    options: BTreeMap<String, Vec<String>>,
    parts: BTreeMap<String, Artifact>,
}

impl RustPlan {
    /// Validate only the selected platform's official component parts.
    pub fn from_lock(lock: &Lock, platform: Platform) -> Result<Self, Error> {
        let (host, spelling) = rustup::target(platform).ok_or(Error::UnsupportedTarget)?;
        let rust = lock.tools().get("rust").ok_or(Error::InvalidLock)?;
        let Installation::Managed { artifacts } = &rust.installation else {
            return Err(Error::InvalidLock);
        };
        let profile = match rust.options.get("profile").map(Vec::as_slice) {
            Some([profile]) if profile == "minimal" || profile == "default" => profile.clone(),
            _ => return Err(Error::InvalidLock),
        };
        let exact = Channel::parse(&rust.version).is_some_and(|channel| match channel {
            Channel::Release(version) => rustup::canonical_release(&version).is_some(),
            Channel::Beta(date) | Channel::Nightly(date) => date.is_some(),
            Channel::Stable => false,
        });
        if rust.adapter != "rust"
            || !exact
            || rust
                .options
                .keys()
                .any(|key| !["components", "profile", "targets"].contains(&key.as_str()))
        {
            return Err(Error::InvalidLock);
        }
        let parts = artifacts.get(&platform).ok_or(Error::MissingTarget)?;
        let exe = if rustup::windows(platform) {
            ".exe"
        } else {
            ""
        };
        for (name, artifact) in parts {
            let (package, target) = split(name, host);
            let stem = artifact
                .url
                .strip_prefix(DIST)
                .and_then(|path| path.rsplit('/').next())
                .and_then(|file| file.strip_suffix(".tar.gz"))
                .ok_or(Error::InvalidLock)?;
            let directory = match package {
                "rust-std" | "rust-analysis" => format!("{package}-{target}"),
                _ => package.to_owned(),
            };
            let expected: BTreeMap<_, _> = executables(package)
                .iter()
                .map(|name| ((*name).to_owned(), format!("bin/{name}{exe}")))
                .collect();
            let xz_url = artifact
                .url
                .strip_suffix(".tar.gz")
                .map(|url| url.to_owned() + ".tar.xz");
            if artifact.format != Format::TarGz
                || artifact.alternates.len() != 1
                || artifact
                    .alternates
                    .get(&AlternateFormat::TarXz)
                    .is_none_or(|xz| Some(&xz.url) != xz_url.as_ref())
                || artifact.destination.is_some()
                || artifact.root_prefix.as_deref() != Some(&format!("{stem}/{directory}"))
                || artifact.executables != expected
            {
                return Err(Error::InvalidLock);
            }
        }
        if !parts.contains_key("rustc") || !parts.contains_key("cargo") {
            return Err(Error::InvalidLock);
        }
        let toolchain = format!("toolchains/{}-{host}", rust.version);
        let digest = serde_json::to_vec(parts).map_err(|_| Error::InvalidLock)?;
        Ok(Self {
            tool: Tool {
                id: "rust".into(),
                version: rust.version.clone(),
                platform: spelling.into(),
                artifact_sha256: hex::encode(Sha256::digest(digest)),
                executables: parts
                    .values()
                    .flat_map(|artifact| &artifact.executables)
                    .map(|(name, path)| (name.clone(), format!("{toolchain}/{path}")))
                    .collect(),
            },
            host,
            profile,
            options: rust.options.clone(),
            parts: parts.clone(),
        })
    }

    pub fn inventory_tool(&self) -> &Tool {
        &self.tool
    }

    /// The installed toolchain's rustup name, `<exact>-<host>`, under the
    /// tool tree that serves as `RUSTUP_HOME`.
    pub fn toolchain(&self) -> String {
        format!("{}-{}", self.tool.version, self.host)
    }

    /// Install with the given verified rustup into fresh private homes beside
    /// the Store (same filesystem), then verify. Healthy identical trees are
    /// reused without running rustup. Hold the Store guard through publication.
    pub fn prepare(
        &self,
        store: &Store,
        desired: &[Tool],
        rustup: &Path,
        preparation: crate::Preparation,
    ) -> Result<Option<PreparedRust>, Error> {
        if self.tool.platform.starts_with("windows-")
            || self.tool.platform.ends_with("-musl")
            || !cfg!(unix)
        {
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
        let scratch = tempfile::Builder::new()
            .prefix(".rust-")
            .tempdir_in(store.repository_root()?.join(".turbo/tools"))?;
        let homes = PrivateHomes::create(scratch.path())?;
        let mut command = homes.command(rustup);
        command
            .args([
                "toolchain",
                "install",
                &self.tool.version,
                "--no-self-update",
            ])
            .args(["--profile", &self.profile])
            .stdin(Stdio::null());
        for (flag, key) in [("--component", "components"), ("--target", "targets")] {
            for value in self.options.get(key).into_iter().flatten() {
                command.args([flag, value]);
            }
        }
        let status = command.status()?;
        if !status.success() {
            return Err(Error::Rustup(status));
        }
        self.verify(homes.rustup_home())?;
        Ok(Some(PreparedRust {
            home: homes.rustup_home().to_owned(),
            _scratch: scratch,
            tool: self.tool.clone(),
        }))
    }

    /// rustup verified every download against its manifest; bind that
    /// manifest and the exact installed component set to the lock. rustup
    /// downloads the first published copy of zstd, xz and gzip, so the
    /// installed manifest's xz and gzip pins must both be the locked ones and
    /// it must not publish zstd, which turbo.lock never pins. A mirror's
    /// manifest therefore cannot substitute any downloaded bytes.
    fn verify(&self, home: &Path) -> Result<(), Error> {
        let rustlib = home
            .join(format!("toolchains/{}-{}", self.tool.version, self.host))
            .join("lib/rustlib");
        let read = |name: &str| -> Result<Vec<u8>, Error> {
            let mut bytes = Vec::new();
            fs::File::open(rustlib.join(name))?
                .take(MAX_MANIFEST_BYTES as u64 + 1)
                .read_to_end(&mut bytes)?;
            Ok(bytes)
        };
        let manifest = Manifest::parse(&read("multirust-channel-manifest.toml")?)
            .map_err(|_| Error::Mismatch)?;
        let channel = Channel::parse(&self.tool.version).ok_or(Error::Mismatch)?;
        if toolchain_name(&channel, &manifest).ok().as_ref() != Some(&self.tool.version) {
            return Err(Error::Mismatch);
        }
        let mut expected = BTreeSet::new();
        for (name, artifact) in &self.parts {
            let (package, target) = split(name, self.host);
            let tarball = manifest.tarball(package, target).ok_or(Error::Mismatch)?;
            let xz = artifact
                .alternates
                .get(&AlternateFormat::TarXz)
                .ok_or(Error::Mismatch)?;
            if tarball.zst
                || (tarball.url, tarball.sha256) != (&*artifact.url, &*artifact.sha256)
                || (tarball.xz_url, tarball.xz_sha256) != (&*xz.url, &*xz.sha256)
            {
                return Err(Error::Mismatch);
            }
            let qualified = tarball.stem.ends_with(&format!("-{target}"));
            expected.insert(if qualified {
                format!("{package}-{target}")
            } else {
                package.to_owned()
            });
        }
        let installed = String::from_utf8(read("components")?).map_err(|_| Error::Mismatch)?;
        if installed
            .lines()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>()
            != expected
        {
            return Err(Error::Mismatch);
        }
        Ok(())
    }
}

// Cross-target standard libraries are `rust-std-<target>`; others are host
// parts.
fn split<'a>(name: &'a str, host: &'a str) -> (&'a str, &'a str) {
    match name.strip_prefix("rust-std-") {
        Some(target) => ("rust-std", target),
        None => (name, host),
    }
}

/// A verified private RUSTUP_HOME holding only the locked toolchain.
pub struct PreparedRust {
    home: PathBuf,
    _scratch: tempfile::TempDir,
    tool: Tool,
}

impl PreparedRust {
    /// Callback for Store::reconcile: move the verified home into the empty
    /// staged tree, which becomes this generation's RUSTUP_HOME.
    pub fn stage(
        &self,
        tool: &Tool,
        destination: &Path,
    ) -> Result<(), turborepo_tool_install::Error> {
        if tool != &self.tool
            || fs::symlink_metadata(destination)?.file_type().is_symlink()
            || fs::read_dir(destination)?.next().is_some()
        {
            return Err(turborepo_tool_install::Error::InvalidInventory);
        }
        for entry in fs::read_dir(&self.home)? {
            let entry = entry?;
            fs::rename(entry.path(), destination.join(entry.file_name()))?;
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests;
