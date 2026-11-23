use std::collections::{BTreeMap, BTreeSet};

use turborepo_archive::Layout;
use turborepo_download::{ApprovedOrigin, DownloadClient};
use turborepo_setup::{
    artifact_provision::{archive_limits, download_limits},
    lock::{
        self, Artifact, Platform, Tool,
        additional::{CatalogRequest, lock_format},
    },
    source_policy::ToolDownloadPolicy,
};
use turborepo_turbo_json::ArtifactFormat;
use url::Url;

use super::{Error, Plan, PlannedArtifact, Unavailable};

pub struct CatalogTransport {
    client: DownloadClient,
    loopback: Option<String>,
}

impl CatalogTransport {
    /// Requests go exactly to the origins the plans name. Release hosts
    /// redirect to a CDN, so these downloads may follow HTTPS redirects to any
    /// origin, as locked artifact downloads do; the lock records only the
    /// planned URL and the digest of what arrived. Construct only after the
    /// setup source preflight passes.
    pub fn new<'a>(
        _policy: &ToolDownloadPolicy,
        plans: impl IntoIterator<Item = &'a Plan>,
    ) -> Result<Self, Error> {
        let origins: BTreeSet<_> = plans
            .into_iter()
            .flat_map(|plan| &plan.releases)
            .flat_map(|release| release.platforms.values())
            .filter_map(|planned| Url::parse(&planned.as_ref().ok()?.url).ok())
            .map(|url| url.origin().ascii_serialization())
            .collect();
        let approved = origins
            .iter()
            .map(|origin| ApprovedOrigin::https(origin))
            .collect::<Result<Vec<_>, _>>()
            .map_err(Error::Transport)?;
        let client = DownloadClient::new(approved).map_err(Error::Transport)?;
        Ok(Self::with_client(client, None))
    }

    fn with_client(client: DownloadClient, loopback: Option<String>) -> Self {
        Self {
            client: client.with_https_redirects(),
            loopback,
        }
    }

    /// Explicit fixture-only HTTP opt-in; literal loopback IPs only.
    #[cfg(any(test, feature = "test-support"))]
    pub fn loopback_http_for_tests(origin: &str) -> Result<Self, Error> {
        let client = DownloadClient::loopback_http_for_tests(origin).map_err(Error::Transport)?;
        let loopback = Some(origin.trim_end_matches('/').into());
        Ok(Self::with_client(client, loopback))
    }

    async fn pin(
        &self,
        planned: &PlannedArtifact,
        platforms: &[Platform],
    ) -> Result<Vec<Result<String, Unavailable>>, turborepo_download::Error> {
        let all = |pinned: Result<String, Unavailable>| Ok(vec![pinned; platforms.len()]);
        let windows = platforms.iter().any(|platform| platform.windows());
        let Ok(url) = Url::parse(&planned.url) else {
            return all(Err(Unavailable::Recipe(
                "its download URL is invalid".into(),
            )));
        };
        let request = match &self.loopback {
            Some(origin) => format!("{origin}{}", url.path()),
            None => planned.url.clone(),
        };
        let (bytes, sha256) = match self
            .client
            .download_unpinned(&request, download_limits()?)
            .await
        {
            Ok(unpinned) => unpinned.pin(),
            Err(turborepo_download::Error::HttpStatus(404)) => {
                return all(Err(Unavailable::Missing));
            }
            Err(turborepo_download::Error::TooLarge) => return all(Err(Unavailable::TooLarge)),
            Err(error) => return Err(error),
        };
        let format = match planned.format {
            ArtifactFormat::Binary => {
                let pinned =
                    platforms
                        .iter()
                        .map(|platform| match program(*platform, bytes.as_bytes()) {
                            true => Ok(sha256.clone()),
                            false => Err(Unavailable::NotAProgram),
                        });
                return Ok(pinned.collect());
            }
            ArtifactFormat::Tar => turborepo_archive::Format::Tar,
            ArtifactFormat::TarGz => turborepo_archive::Format::TarGz,
            ArtifactFormat::Zip => turborepo_archive::Format::Zip,
        };
        let unpack = |error: turborepo_archive::Error| Unavailable::Archive(error.to_string());
        let required: Vec<_> = planned.executables.values().map(String::as_str).collect();
        let layout = Layout {
            root: "",
            required_files: &required,
        };
        let unpacked = archive_limits()
            .and_then(|limits| turborepo_archive::extract(&bytes, format, limits, layout));
        let tree = match unpacked {
            Ok(tree) => tree,
            // This host cannot unpack a Unix archive, so it cannot check it.
            // A Windows archive with symlinks cannot be installed anywhere.
            Err(turborepo_archive::Error::UnsupportedSymlinkPlatform) if !windows => {
                return all(Ok(sha256));
            }
            Err(error) => return all(Err(unpack(error))),
        };
        // Installation refuses an export its owner cannot execute. Windows
        // has no such bit, and neither does a Windows host's staging.
        #[cfg(unix)]
        if !windows {
            use std::os::unix::fs::PermissionsExt;
            for path in &required {
                let file = path
                    .split('/')
                    .fold(tree.root_path(), |file, part| file.join(part));
                match std::fs::metadata(file) {
                    Ok(metadata) if metadata.permissions().mode() & 0o100 != 0 => {}
                    Ok(_) => return all(Err(Unavailable::NotExecutable((*path).to_owned()))),
                    Err(error) => return all(Err(unpack(error.into()))),
                }
            }
        }
        #[cfg(not(unix))]
        let _ = (windows, tree);
        all(Ok(sha256))
    }
}

fn program(platform: Platform, bytes: &[u8]) -> bool {
    // 64-bit Mach-O, and universal binaries with 32-bit or 64-bit offsets.
    const MACH_O: [&[u8]; 3] = [
        b"\xcf\xfa\xed\xfe",
        b"\xca\xfe\xba\xbe",
        b"\xca\xfe\xba\xbf",
    ];
    let script = bytes.starts_with(b"#!");
    match platform {
        Platform::Any => false,
        Platform::MacosX64 | Platform::MacosArm64 => {
            script || MACH_O.iter().any(|magic| bytes.starts_with(magic))
        }
        Platform::LinuxX64Gnu
        | Platform::LinuxArm64Gnu
        | Platform::LinuxX64Musl
        | Platform::LinuxArm64Musl => script || bytes.starts_with(b"\x7fELF"),
        Platform::WindowsX64 | Platform::WindowsArm64 => bytes.starts_with(b"MZ"),
    }
}

#[derive(Debug)]
pub struct Resolved {
    /// The complete lock entry: one pinned artifact per installable platform.
    pub tool: Tool,
    /// The catalog entry that gave the directions, as `aqua:<package ID>`.
    pub source: String,
    /// The name the release has upstream, such as `v1.8.2` for `1.8.2`.
    pub release: String,
    pub skipped: BTreeMap<Platform, Unavailable>,
}

/// Resolves `request` as `plan` describes it. `host` must be among the
/// platforms that resolve, because the tool is installed next.
pub async fn resolve(
    request: &CatalogRequest,
    plan: &Plan,
    host: Platform,
    transport: &CatalogTransport,
) -> Result<Resolved, Error> {
    let mut missing = BTreeSet::new();
    let mut asked = false;
    let mut rejected = None;
    for release in &plan.releases {
        let mut downloads: Vec<(&PlannedArtifact, bool, Vec<Platform>)> = Vec::new();
        for (platform, planned) in &release.platforms {
            let Ok(planned) = planned else { continue };
            let windows = platform.windows();
            match downloads
                .iter_mut()
                .find(|(other, target, _)| *other == planned && *target == windows)
            {
                Some((.., platforms)) => platforms.push(*platform),
                None => downloads.push((planned, windows, vec![*platform])),
            }
        }
        let known = &missing;
        let pinned =
            futures::future::join_all(downloads.iter().map(|(planned, _, platforms)| async move {
                if known.contains(&planned.url) {
                    return Ok(vec![Err(Unavailable::Missing); platforms.len()]);
                }
                transport.pin(planned, platforms).await
            }))
            .await;
        let mut artifacts = BTreeMap::new();
        let mut skipped: BTreeMap<_, _> = release
            .platforms
            .iter()
            .filter_map(|(platform, planned)| Some((*platform, planned.as_ref().err()?.clone())))
            .collect();
        let mut answered = false;
        for ((planned, _, platforms), pinned) in downloads.iter().zip(pinned) {
            let pinned = pinned.map_err(|source| Error::Download {
                tool: request.id.clone(),
                version: request.version.clone(),
                platform: spelling(platforms[0]),
                source,
            })?;
            match pinned.first() {
                Some(Err(Unavailable::Missing)) => {
                    asked = true;
                    missing.insert(planned.url.clone());
                }
                Some(Err(Unavailable::Recipe(_))) => {}
                _ => (asked, answered) = (true, true),
            }
            for (platform, pinned) in platforms.iter().zip(pinned) {
                let pinned = pinned.and_then(|sha256| {
                    let artifact = Artifact {
                        url: planned.url.clone(),
                        sha256: Some(sha256),
                        sha512: None,
                        format: lock_format(planned.format),
                        alternates: BTreeMap::new(),
                        root_prefix: None,
                        destination: None,
                        executables: planned.executables.clone(),
                    };
                    match lock::validate_artifact(&artifact) {
                        Ok(()) => Ok(artifact),
                        Err(error) => Err(Unavailable::Recipe(error.to_string())),
                    }
                });
                match pinned {
                    Ok(artifact) => {
                        artifacts.insert(*platform, artifact);
                    }
                    Err(reason) => {
                        skipped.insert(*platform, reason);
                    }
                }
            }
        }
        // A release name is the release only if something under it can be
        // locked. A host that answers a missing asset with a page must not
        // stop the names after it.
        if artifacts.is_empty() {
            if answered {
                rejected.get_or_insert(skipped);
            }
            continue;
        }
        if !artifacts.contains_key(&host) {
            return Err(Error::Host {
                tool: request.id.clone(),
                version: request.version.clone(),
                platform: spelling(host),
                reason: skipped.remove(&host).unwrap_or(Unavailable::Platform),
            });
        }
        return Ok(Resolved {
            tool: request.resolved(&plan.source, artifacts),
            source: plan.source.clone(),
            release: release.name.clone(),
            skipped,
        });
    }
    if let Some(skipped) = rejected {
        return Err(no_platform(request, skipped));
    }
    if asked {
        return Err(Error::NoRelease {
            tool: request.id.clone(),
            version: request.version.clone(),
            package: plan.source.clone(),
            tried: plan.releases.iter().map(|r| r.name.clone()).collect(),
        });
    }
    let reasons = plan.releases.last().into_iter().flat_map(|release| {
        release
            .platforms
            .iter()
            .filter_map(|(platform, planned)| Some((*platform, planned.as_ref().err()?.clone())))
    });
    Err(no_platform(request, reasons.collect()))
}

fn no_platform(request: &CatalogRequest, skipped: BTreeMap<Platform, Unavailable>) -> Error {
    Error::NoPlatform {
        tool: request.id.clone(),
        version: request.version.clone(),
        platforms: skipped
            .into_iter()
            .map(|(platform, reason)| (spelling(platform), reason))
            .collect(),
    }
}

/// The platform as `turbo.lock` and `turbo.json` spell it.
pub fn spelling(platform: Platform) -> String {
    serde_json::to_value(platform)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{platform:?}"))
}

#[cfg(test)]
mod tests;
