//! Locked Rust toolchain adapter. The repository's verified private rustup
//! installs the exact locked toolchain into fresh private homes; rustup's
//! installed component set and manifest digests must then match the lock
//! before the home is staged. No dependency fetch or task execution.
//!
//! Unix exports link the toolchain binaries. A Windows export is a hard link
//! whose image path is the managed bin, where `rustc.exe` would find neither
//! its `rustc_driver` DLL nor its sysroot, so every Windows export is instead
//! rustup's own proxy: a copy of the verified rustup binary inside this tree,
//! as `rustup-init` lays out `%CARGO_HOME%\bin`. Proxies select the toolchain
//! from the `RUSTUP_HOME` and `RUSTUP_TOOLCHAIN` that activation sets.

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
    lock::{self, Artifact, Format, Installation, Lock, Platform},
    rust_discovery::{Channel, Toolchain},
    rust_metadata::{MAX_MANIFEST_BYTES, Manifest},
    rust_resolution::{executables, toolchain_name},
    rustup,
    rustup_provision::{self, PrivateHomes, RustupPlan, RustupTransport},
};

const DIST: &str = "https://static.rust-lang.org/dist/";
/// The Windows proxy binary, relative to the tree that serves as RUSTUP_HOME.
const PROXY: &str = "proxy/rustup.exe";
const MAX_PROXY_BYTES: u64 = 64 * 1024 * 1024;

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
        "managed Rust promotion is not qualified for this host, Windows arm64 or musl; use a \
         supported macOS, GNU Linux or Windows x64 target or configure verify-system"
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
    /// Windows only: the locked rustup installer digest every proxy copies.
    proxy_sha256: Option<String>,
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
            if artifact.format != Format::TarGz
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
        let mut digest = serde_json::to_vec(parts).map_err(|_| Error::InvalidLock)?;
        // A Windows tree also holds the proxy, so its identity follows the
        // locked rustup installer as well.
        let proxy_sha256 = if rustup::windows(platform) {
            let installer = RustupPlan::from_lock(lock, platform).map_err(|e| match e {
                rustup_provision::Error::MissingTarget => Error::MissingTarget,
                _ => Error::InvalidLock,
            })?;
            let sha256 = installer.inventory_tool().artifact_sha256.clone();
            digest.extend(sha256.as_bytes());
            Some(sha256)
        } else {
            None
        };
        let proxied = proxy_sha256.is_some();
        let export = |path: &str| {
            if proxied {
                PROXY.to_owned()
            } else {
                format!("{toolchain}/{path}")
            }
        };
        Ok(Self {
            tool: Tool {
                id: "rust".into(),
                version: rust.version.clone(),
                platform: spelling.into(),
                artifact_sha256: hex::encode(Sha256::digest(digest)),
                executables: parts
                    .values()
                    .flat_map(|artifact| &artifact.executables)
                    .map(|(name, path)| (name.clone(), export(path)))
                    .collect(),
            },
            host,
            profile,
            options: rust.options.clone(),
            parts: parts.clone(),
            proxy_sha256,
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
    /// the Store (same filesystem), downloading from the transport's origin,
    /// then verify. Healthy identical trees are reused without running rustup.
    /// Hold the Store guard through publication.
    pub fn prepare(
        &self,
        store: &Store,
        desired: &[Tool],
        rustup: &Path,
        transport: &RustupTransport,
        preparation: crate::Preparation,
    ) -> Result<Option<PreparedRust>, Error> {
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
        // Split components: `/` is not a separator under a `\\?\` root.
        let scratch = tempfile::Builder::new()
            .prefix(".rust-")
            .tempdir_in(store.repository_root()?.join(".turbo").join("tools"))?;
        let homes = PrivateHomes::create(scratch.path())?;
        let mut command = homes.command(rustup, transport);
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
        if let Some(sha256) = &self.proxy_sha256 {
            stage_proxy(rustup, homes.rustup_home(), sha256)?;
        }
        Ok(Some(PreparedRust {
            home: homes.rustup_home().to_owned(),
            _scratch: scratch,
            tool: self.tool.clone(),
        }))
    }

    /// rustup verified every download against its manifest; bind that
    /// manifest and the exact installed component set to the lock.
    fn verify(&self, home: &Path) -> Result<(), Error> {
        // Split components: `/` is not a separator under a `\\?\` root.
        let toolchain = home.join("toolchains").join(self.toolchain());
        let rustlib = toolchain.join("lib").join("rustlib");
        // Proxies run the locked binaries by path, so they must be installed;
        // Unix exports link them and the Store checks them instead.
        if self.proxy_sha256.is_some() {
            let paths = self
                .parts
                .values()
                .flat_map(|part| part.executables.values());
            for path in paths {
                let path = path.split('/').fold(toolchain.clone(), |p, c| p.join(c));
                if !fs::symlink_metadata(path).is_ok_and(|m| m.is_file()) {
                    return Err(Error::Mismatch);
                }
            }
        }
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
            if tarball.url != artifact.url || tarball.sha256 != artifact.sha256 {
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

/// Copy the rustup that installed this toolchain into the home as its
/// Windows proxy, rechecking the copied bytes against the locked installer.
fn stage_proxy(rustup: &Path, home: &Path, sha256: &str) -> Result<(), Error> {
    let mut bytes = Vec::new();
    fs::File::open(rustup)?
        .take(MAX_PROXY_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if hex::encode(Sha256::digest(&bytes)) != sha256 {
        return Err(Error::Mismatch);
    }
    let (directory, file) = PROXY.split_once('/').ok_or(Error::InvalidLock)?;
    let directory = home.join(directory);
    fs::create_dir(&directory)?;
    let mut proxy = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join(file))?;
    // Store flushes the complete staged tree before publication.
    io::Write::write_all(&mut proxy, &bytes)?;
    Ok(())
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

#[cfg(all(test, windows))]
mod windows_tests;
