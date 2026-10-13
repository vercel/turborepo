//! CPython request → portable `turbo.lock` selection through the locked uv.
//! The managed uv's own download list decides which python-build-standalone
//! build serves each target, so selections are ones that uv manages; each
//! target binds the build's published SHA-256 from the Astral mirror. Listing
//! runs the verified uv offline with configuration disabled and covers every
//! platform and architecture, so the lock never depends on the host. Nothing
//! is installed and no system Python is consulted.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

use semver::Version;
use serde_json::Value;
use turborepo_download::{ApprovedOrigin, DownloadClient, Limits};

use crate::{
    lock::{self, Artifact, Document, Format, Installation, Lock, Platform, Tool},
    node_discovery::UniqueJson,
    node_metadata::Checksums,
    python_discovery::PythonRequirement,
    uv::{ORIGIN, exact_release},
};

pub const MAX_LISTING_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_CHECKSUM_BYTES: usize = 2 * 1024 * 1024;
const MAX_CHECKSUM_ENTRIES: usize = 16_384;
const MAX_ENTRIES: usize = 65_536;
pub(crate) const RELEASES: &str = "/github/python-build-standalone/releases/download/";
const GITHUB: &str = "https://github.com/astral-sh/python-build-standalone/releases/download/";

/// Lock platforms with uv's `(os, arch, libc)` CPython download spellings.
/// Every other listed architecture or CPU variant (`x86`, `x86_64_v3`,
/// `armv7`, ...) is never recorded.
const TARGETS: [(Platform, &str, &str, &str); 8] = [
    (Platform::MacosX64, "macos", "x86_64", "none"),
    (Platform::MacosArm64, "macos", "aarch64", "none"),
    (Platform::LinuxX64Gnu, "linux", "x86_64", "gnu"),
    (Platform::LinuxArm64Gnu, "linux", "aarch64", "gnu"),
    (Platform::LinuxX64Musl, "linux", "x86_64", "musl"),
    (Platform::LinuxArm64Musl, "linux", "aarch64", "musl"),
    (Platform::WindowsX64, "windows", "x86_64", "none"),
    (Platform::WindowsArm64, "windows", "aarch64", "none"),
];

/// uv's managed-installation key, which is also its install directory name.
fn key(version: &Version, platform: Platform) -> Option<String> {
    let (_, os, arch, libc) = TARGETS.iter().find(|target| target.0 == platform)?;
    Some(format!("cpython-{version}-{os}-{arch}-{libc}"))
}

/// The only artifact shape setup records or installs for a CPython build.
pub(crate) fn official_artifact(
    version: &Version,
    platform: Platform,
    path: &str,
    sha256: String,
) -> Option<Artifact> {
    let windows = matches!(platform, Platform::WindowsX64 | Platform::WindowsArm64);
    let executables = if windows {
        BTreeMap::from([("python".into(), "python.exe".into())])
    } else {
        ["python", "python3"]
            .map(|name| (name.to_owned(), "bin/python3".to_owned()))
            .into()
    };
    official_path(path).then_some(())?;
    Some(Artifact {
        url: format!("{ORIGIN}{RELEASES}{path}"),
        sha256,
        format: Format::TarGz,
        alternates: BTreeMap::new(),
        root_prefix: Some("python".into()),
        // uv discovers managed interpreters by their key directory.
        destination: Some(key(version, platform)?),
        executables,
    })
}

/// Network-free checks for a preserved selection: the release still satisfies
/// the request and every target is an official build at uv's key directory.
pub fn validate(tool: &Tool, requirement: &PythonRequirement) -> Result<(), Error> {
    let invalid = || lock::Error::Invalid("locked Python contradicts its request or layout");
    let version = exact_release(&tool.version).ok_or_else(invalid)?;
    let Installation::Managed { artifacts } = &tool.installation else {
        return Err(invalid().into());
    };
    if tool.adapter != "python" || !tool.options.is_empty() || !requirement.matches(&version) {
        return Err(invalid().into());
    }
    for (platform, parts) in artifacts {
        match parts.get("distribution") {
            Some(artifact) if parts.len() == 1 && is_official(&version, *platform, artifact) => {}
            _ => return Err(invalid().into()),
        }
    }
    Ok(())
}

pub(crate) fn is_official(version: &Version, platform: Platform, artifact: &Artifact) -> bool {
    artifact
        .url
        .strip_prefix(&format!("{ORIGIN}{RELEASES}"))
        .and_then(|path| official_artifact(version, platform, path, artifact.sha256.clone()))
        .is_some_and(|official| &official == artifact)
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid or ambiguous uv Python download listing")]
    Listing,
    #[error("the managed uv could not list Python downloads; run turbo setup to repair uv")]
    ListingFailed,
    #[error(
        "the locked uv lists Python downloads for one architecture only, so turbo.lock would not \
         cover every platform; require uv 0.7.22 or newer"
    )]
    HostOnlyListing,
    #[error("invalid python-build-standalone SHA256SUMS")]
    Checksums,
    #[error("no CPython build known to the locked uv satisfies {file} = {request:?}")]
    NoMatchingRelease { file: &'static str, request: String },
    #[error("python-build-standalone publishes no checksum for {0}")]
    MissingChecksum(String),
    #[error(transparent)]
    Lock(#[from] lock::Error),
    #[error(transparent)]
    Download(#[from] turborepo_download::Error),
    #[error("uv Python listing I/O failed")]
    Io(#[from] io::Error),
}

/// Default-variant CPython final releases by version and lock platform, each
/// mapped to `<tag>/<percent-encoded filename>` below the official releases.
#[derive(Clone, Debug)]
pub struct Listing(BTreeMap<Version, BTreeMap<Platform, String>>);

impl Listing {
    /// Parse `uv python list --only-downloads --all-versions --all-platforms
    /// --all-arches --output-format json`, which is the same on every host.
    /// Other implementations, variants, prereleases and unsupported targets
    /// are ignored; inconsistent CPython entries fail.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_LISTING_BYTES {
            return Err(Error::Listing);
        }
        let UniqueJson(value) = serde_json::from_slice(bytes).map_err(|_| Error::Listing)?;
        let entries = value.as_array().ok_or(Error::Listing)?;
        if entries.len() > MAX_ENTRIES {
            return Err(Error::Listing);
        }
        let mut listing: BTreeMap<_, BTreeMap<_, _>> = BTreeMap::new();
        for entry in entries {
            let field = |name| entry.get(name).and_then(Value::as_str);
            if field("implementation") != Some("cpython") || field("variant") != Some("default") {
                continue;
            }
            let Some(version) = field("version").and_then(exact_release) else {
                continue;
            };
            let (Some(os), Some(arch), Some(libc)) = (field("os"), field("arch"), field("libc"))
            else {
                return Err(Error::Listing);
            };
            let Some(&(platform, ..)) = TARGETS
                .iter()
                .find(|target| (target.1, target.2, target.3) == (os, arch, libc))
            else {
                continue;
            };
            let key = key(&version, platform).ok_or(Error::Listing)?;
            let path = field("url")
                .and_then(|url| {
                    url.strip_prefix(&format!("{ORIGIN}{RELEASES}"))
                        .or_else(|| url.strip_prefix(GITHUB))
                })
                .ok_or(Error::Listing)?;
            if !official_path(path) {
                // uv lists some older builds only as full `.tar.zst` archives,
                // which setup cannot install; skip them, reject anything else.
                if path.ends_with(".tar.zst") {
                    continue;
                }
                return Err(Error::Listing);
            }
            if field("key") != Some(key.as_str()) {
                return Err(Error::Listing);
            }
            if listing
                .entry(version)
                .or_default()
                .insert(platform, path.to_owned())
                .is_some()
            {
                return Err(Error::Listing);
            }
        }
        // uv before 0.7.22 ignores `--all-arches` with `--only-downloads` and
        // lists only builds the host can run, which never include both Linux
        // architectures. Fail rather than lock Python for one architecture.
        let listed = |platform: Platform| listing.values().any(|map| map.contains_key(&platform));
        if !(listed(Platform::LinuxX64Gnu) && listed(Platform::LinuxArm64Gnu)) {
            return Err(Error::HostOnlyListing);
        }
        Ok(Self(listing))
    }

    /// Highest exact release satisfying the request on any target.
    pub fn select(&self, requirement: &PythonRequirement) -> Result<Version, Error> {
        self.0
            .keys()
            .rev()
            .find(|version| requirement.matches(version))
            .cloned()
            .ok_or_else(|| Error::NoMatchingRelease {
                file: requirement.file(),
                request: requirement.request().to_owned(),
            })
    }

    fn tags(&self, version: &Version) -> BTreeSet<&str> {
        self.0
            .get(version)
            .into_iter()
            .flat_map(BTreeMap::values)
            .filter_map(|path| path.split('/').next())
            .collect()
    }
}

/// `<numeric tag>/cpython-...-install_only[_stripped].tar.gz`; `%2B` is the
/// only escape (python-build-standalone's `+` build separator).
fn official_path(path: &str) -> bool {
    let Some((tag, filename)) = path.split_once('/') else {
        return false;
    };
    !tag.is_empty()
        && tag.len() <= 32
        && tag.bytes().all(|b| b.is_ascii_digit())
        && filename.len() <= 255
        && filename.starts_with("cpython-")
        && (filename.ends_with("-install_only_stripped.tar.gz")
            || filename.ends_with("-install_only.tar.gz"))
        && filename
            .replace("%2B", "+")
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
}

/// Bind every listed target of `version` to its published checksum. A listed
/// build without a checksum fails instead of being omitted or trusted.
pub fn resolve(
    requirement: &PythonRequirement,
    listing: &Listing,
    version: &Version,
    checksums: &BTreeMap<String, Checksums>,
) -> Result<Tool, Error> {
    if !requirement.matches(version) {
        return Err(lock::Error::Invalid("selected Python contradicts its request").into());
    }
    let mut artifacts = BTreeMap::new();
    for (platform, path) in listing.0.get(version).into_iter().flatten() {
        let (tag, filename) = path.split_once('/').ok_or(Error::Listing)?;
        let published = filename.replace("%2B", "+");
        let sha256 = checksums
            .get(tag)
            .and_then(|sums| sums.sha256(&published))
            .ok_or_else(|| Error::MissingChecksum(published.clone()))?;
        let artifact =
            official_artifact(version, *platform, path, sha256.to_owned()).ok_or(Error::Listing)?;
        artifacts.insert(
            *platform,
            BTreeMap::from([("distribution".into(), artifact)]),
        );
    }
    let tool = Tool {
        adapter: "python".into(),
        version: version.to_string(),
        declarations: vec![requirement.declaration()],
        options: BTreeMap::new(),
        installation: Installation::Managed { artifacts },
    };
    Lock::new(Document {
        schema_version: lock::SCHEMA_VERSION,
        tools: BTreeMap::from([("python".into(), tool.clone())]),
    })?;
    Ok(tool)
}

/// Run the locked uv's offline download listing with user and repository
/// configuration disabled and private scratch state.
pub fn list(uv: &Path) -> Result<Listing, Error> {
    let scratch = tempfile::tempdir()?;
    let output = Command::new(uv)
        .args([
            "python",
            "list",
            "--only-downloads",
            "--all-versions",
            "--all-platforms",
            "--all-arches",
            "--output-format",
            "json",
            "--offline",
            "--no-config",
        ])
        .env_clear()
        .env("HOME", scratch.path())
        .env("UV_CACHE_DIR", scratch.path().join("cache"))
        .env("UV_PYTHON_INSTALL_DIR", scratch.path().join("python"))
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Err(Error::ListingFailed);
    }
    Listing::parse(&output.stdout)
}

pub struct PythonTransport {
    pub(crate) client: DownloadClient,
    pub(crate) origin: String,
}

impl PythonTransport {
    pub fn official(_policy: &crate::source_policy::OfficialSourcePolicy) -> Result<Self, Error> {
        Ok(Self {
            client: DownloadClient::new([ApprovedOrigin::https(ORIGIN)?])?,
            origin: ORIGIN.into(),
        })
    }

    /// Select from the uv listing, then read only the selected builds'
    /// `SHA256SUMS`. No artifact download.
    pub async fn resolve(
        &self,
        requirement: &PythonRequirement,
        listing: &Listing,
    ) -> Result<Tool, Error> {
        let version = listing.select(requirement)?;
        let mut checksums = BTreeMap::new();
        for tag in listing.tags(&version) {
            let bytes = self
                .client
                .read_metadata(
                    &format!("{}{RELEASES}{tag}/SHA256SUMS", self.origin),
                    Limits::new(MAX_CHECKSUM_BYTES, Duration::from_secs(30))?,
                )
                .await?;
            let sums = Checksums::parse_bounded(&bytes, MAX_CHECKSUM_BYTES, MAX_CHECKSUM_ENTRIES)
                .map_err(|_| Error::Checksums)?;
            checksums.insert(tag.to_owned(), sums);
        }
        resolve(requirement, listing, &version, &checksums)
    }

    /// Explicit fixture-only HTTP opt-in; literal loopback IPs only.
    #[cfg(any(test, feature = "test-support"))]
    pub fn loopback_http_for_tests(origin: &str) -> Result<Self, Error> {
        Ok(Self {
            client: DownloadClient::loopback_http_for_tests(origin)?,
            origin: origin.trim_end_matches('/').into(),
        })
    }
}

#[cfg(all(test, unix))]
mod tests;
