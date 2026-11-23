use std::collections::BTreeMap;

use aqua_registry::{AquaPackage, AquaPackageType};
use turborepo_setup::lock::{
    self, Platform,
    additional::{CatalogRequest, TARGETS, lock_platform},
};
use turborepo_turbo_json::{ArtifactFormat, ArtifactPlatform, derived_artifact_violation};

use super::{Catalog, Error};

struct Target {
    platform: ArtifactPlatform,
    os: &'static str,
    arch: &'static str,
    libc: Option<&'static str>,
}

impl Target {
    fn of(platform: ArtifactPlatform) -> Self {
        use ArtifactPlatform::*;
        let (os, arch, libc) = match platform {
            MacosX64 => ("darwin", "amd64", None),
            MacosArm64 => ("darwin", "arm64", None),
            LinuxX64Gnu => ("linux", "amd64", Some("gnu")),
            LinuxArm64Gnu => ("linux", "arm64", Some("gnu")),
            LinuxX64Musl => ("linux", "amd64", Some("musl")),
            LinuxArm64Musl => ("linux", "arm64", Some("musl")),
            WindowsX64 => ("windows", "amd64", None),
            WindowsArm64 => ("windows", "arm64", None),
        };
        Self {
            platform,
            os,
            arch,
            libc,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Unavailable {
    #[error("the catalog has no download for this platform")]
    Platform,
    #[error("the catalog has no download for this version")]
    NoAsset,
    #[error("the catalog says: {0}")]
    Message(String),
    #[error("the catalog installs it with {0}, which turbo setup does not do")]
    Kind(String),
    #[error("its {0} download is a format that turbo setup cannot install yet")]
    Format(String),
    #[error("the catalog entry cannot be used: {0}")]
    Recipe(String),
    #[error("the release has no download for this platform")]
    Missing,
    #[error("its download is larger than turbo setup installs")]
    TooLarge,
    #[error("its download cannot be unpacked: {0}")]
    Archive(String),
    #[error("{0:?} in its download is not marked executable")]
    NotExecutable(String),
    #[error("its download is not an executable for this platform")]
    NotAProgram,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedArtifact {
    pub url: String,
    pub format: ArtifactFormat,
    /// Executable names mapped to paths below the archive's top level, or to
    /// the one path a binary is installed at.
    pub executables: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleasePlan {
    pub name: String,
    pub platforms: BTreeMap<Platform, Result<PlannedArtifact, Unavailable>>,
}

/// Every release name a version may be published under, most likely first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The catalog entry as `turbo.lock` records it: `aqua:<package ID>`.
    pub source: String,
    pub releases: Vec<ReleasePlan>,
}

impl Catalog {
    /// The downloads the catalog describes for `request`. No I/O beyond
    /// reading the bundled recipes.
    pub fn plan(&self, request: &CatalogRequest) -> Result<Plan, Error> {
        let (source, recipe) = self.entry(request)?;
        Ok(Plan::of(recipe, source, &request.version))
    }
}

impl Plan {
    pub(crate) fn of(recipe: &AquaPackage, source: String, version: &str) -> Self {
        let releases = release_names(recipe, version)
            .into_iter()
            .map(|name| ReleasePlan {
                platforms: TARGETS
                    .into_iter()
                    .map(|platform| {
                        let target = Target::of(platform);
                        let planned = artifact(recipe, &name, version, &target);
                        (lock_platform(platform), planned)
                    })
                    .collect(),
                name,
            })
            .collect();
        Self { source, releases }
    }
}

/// The names a release of `version` may have, most specific first: under each
/// version prefix the recipe sets (for itself, then for version ranges, which
/// is where a recipe that changed its release names records them), under each
/// of those prefixes followed by a `v` (a prefix may stop before it, as
/// `kustomize/` does for `kustomize/v5.8.3`), with a `v`, and bare.
fn release_names(recipe: &AquaPackage, version: &str) -> Vec<String> {
    let prefixes: Vec<_> = std::iter::once(recipe)
        .chain(&recipe.version_overrides)
        .filter_map(|recipe| recipe.version_prefix.as_deref())
        .collect();
    let exact = prefixes.iter().map(|prefix| format!("{prefix}{version}"));
    let with_v = (prefixes.iter().filter(|prefix| !prefix.ends_with('v')))
        .map(|prefix| format!("{prefix}v{version}"));
    let mut names = Vec::new();
    for name in exact
        .chain(with_v)
        .chain([format!("v{version}"), version.to_owned()])
    {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

fn artifact(
    recipe: &AquaPackage,
    release: &str,
    version: &str,
    target: &Target,
) -> Result<PlannedArtifact, Unavailable> {
    let Target {
        platform,
        os,
        arch,
        libc,
    } = *target;
    let unusable = |error: &dyn std::fmt::Display| Unavailable::Recipe(error.to_string());
    let package = recipe
        .clone()
        .with_version_libc(&[release, version], os, arch, libc);
    if let Some(message) = &package.error_message {
        return Err(Unavailable::Message(message.trim().to_owned()));
    }
    if package.no_asset == Some(true) {
        return Err(Unavailable::NoAsset);
    }
    if !supported(&package.supported_envs, os, arch) {
        return Err(Unavailable::Platform);
    }
    let url = match package.package_type() {
        AquaPackageType::GithubRelease => {
            let asset = package.asset(release, os, arch).map_err(|e| unusable(&e))?;
            if package.repo_owner.is_empty() || package.repo_name.is_empty() || asset.is_empty() {
                return Err(unusable(&"it names no GitHub release asset"));
            }
            format!(
                "https://github.com/{}/{}/releases/download/{release}/{asset}",
                package.repo_owner, package.repo_name
            )
        }
        AquaPackageType::Http => package.url(release, os, arch).map_err(|e| unusable(&e))?,
        kind => return Err(Unavailable::Kind(kind.to_string())),
    };
    let format = match package
        .format(release, os, arch)
        .map_err(|e| unusable(&e))?
    {
        "raw" => ArtifactFormat::Binary,
        "zip" => ArtifactFormat::Zip,
        "tar.gz" | "tgz" => ArtifactFormat::TarGz,
        "tar" => ArtifactFormat::Tar,
        other => return Err(Unavailable::Format(other.to_owned())),
    };
    let windows = os == "windows";
    // Without `files`, a package installs one command named after itself:
    // after the recipe, not the ID that was declared, so an alias installs
    // the same command as the canonical ID.
    let name = package.name.as_deref();
    let name = name.and_then(|name| name.rsplit('/').next());
    let default = [aqua_registry::AquaFile {
        name: name.unwrap_or(&package.repo_name).to_owned(),
        ..Default::default()
    }];
    let files = match package.files.as_slice() {
        [] => &default[..],
        files => files,
    };
    let mut executables = BTreeMap::new();
    for file in files {
        let path = match format {
            ArtifactFormat::Binary if windows && package.windows_ext() == ".exe" => {
                match file.name.to_ascii_lowercase().ends_with(".exe") {
                    true => file.name.clone(),
                    false => format!("{}.exe", file.name),
                }
            }
            ArtifactFormat::Binary => {
                package.complete_windows_ext_to_file_src(&file.name, release, os)
            }
            _ => {
                let src = file
                    .src(&package, release, os, arch)
                    .map_err(|e| unusable(&e))?;
                let src = src.unwrap_or_else(|| file.name.clone());
                package.complete_windows_ext_to_file_src(&src, release, os)
            }
        };
        let name = match file.name.len().checked_sub(4) {
            Some(stem)
                if windows
                    && file.name.is_char_boundary(stem)
                    && file.name[stem..].eq_ignore_ascii_case(".exe") =>
            {
                &file.name[..stem]
            }
            _ => file.name.as_str(),
        };
        if executables.insert(name.to_owned(), path).is_some() {
            return Err(unusable(&format!("it installs {name:?} twice")));
        }
    }
    if let Some(reason) = derived_artifact_violation(platform, &url, format, &executables) {
        return Err(Unavailable::Recipe(reason));
    }
    lock::validate_source(&url).map_err(|e| unusable(&e))?;
    Ok(PlannedArtifact {
        url,
        format,
        executables,
    })
}

/// aqua's `supported_envs`: each entry is an OS, an architecture, `os/arch`,
/// or `all`. An empty list supports every platform.
fn supported(envs: &[String], os: &str, arch: &str) -> bool {
    envs.is_empty()
        || envs.iter().any(|env| {
            env == "all"
                || env == os
                || env == arch
                || env
                    .split_once('/')
                    .is_some_and(|(env_os, env_arch)| env_os == os && env_arch == arch)
        })
}

#[cfg(test)]
pub(super) mod tests;
