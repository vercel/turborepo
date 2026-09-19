//! uv from its GitHub releases, and Python interpreters through uv into a
//! repository-scoped `UV_PYTHON_INSTALL_DIR`.

use std::collections::BTreeMap;

use node_semver::Range;
use serde::Deserialize;
use turbopath::AbsoluteSystemPathBuf;

use crate::{
    Error, InstalledTool, Manifest,
    archive::ArchiveKind,
    install::InstallContext,
    version::{VersionSpec, max_satisfying},
};

pub(crate) const UV: &str = "uv";
pub(crate) const PYTHON: &str = "python";
pub(crate) const UV_PYTHON_INSTALL_DIR_ENV: &str = "UV_PYTHON_INSTALL_DIR";

/// `https://pypi.org/pypi/uv/json`: we only need the release keys.
#[derive(Debug, Deserialize)]
struct PypiProject {
    releases: BTreeMap<String, Vec<PypiFile>>,
}

#[derive(Debug, Deserialize)]
struct PypiFile {
    #[serde(default)]
    yanked: bool,
}

pub(crate) async fn resolve_uv(
    ctx: &InstallContext<'_>,
    version: &VersionSpec,
    source: &str,
) -> Result<String, Error> {
    match version {
        VersionSpec::Exact(exact) => Ok(exact.clone()),
        VersionSpec::Range(range) => {
            let parsed = Range::parse(range).map_err(|err| Error::InvalidVersion {
                tool: UV.into(),
                version: range.clone(),
                declared_in: source.into(),
                reason: err.to_string(),
            })?;
            let url = format!("{}/uv/json", ctx.sources.pypi);
            let project: PypiProject = ctx.http.get_json(&url).await?;
            let candidates = project
                .releases
                .iter()
                .filter(|(_, files)| files.is_empty() || files.iter().any(|file| !file.yanked))
                .map(|(version, _)| version.as_str());
            max_satisfying(candidates, &parsed)
                .map(|version| version.to_string())
                .ok_or_else(|| Error::NoMatchingVersion {
                    tool: UV.into(),
                    requested: range.clone(),
                    declared_in: source.into(),
                })
        }
        VersionSpec::Alias(alias) => Err(Error::InvalidVersion {
            tool: UV.into(),
            version: alias.clone(),
            declared_in: source.into(),
            reason: "uv versions must be semver".into(),
        }),
    }
}

pub(crate) async fn install_uv(
    ctx: &InstallContext<'_>,
    version: &str,
    source: &str,
) -> Result<InstalledTool, Error> {
    let triple = ctx.platform.rust_triple();
    // Unix tarballs wrap their contents in `uv-<triple>/`; Windows zips don't.
    let (kind, strip) = if ctx.platform.is_windows() {
        (ArchiveKind::Zip, 0)
    } else {
        (ArchiveKind::TarGz, 1)
    };
    let file = format!("uv-{triple}.{}", kind.extension());
    let url = format!("{}/{version}/{file}", ctx.sources.uv_releases);
    let checksum = ctx.sha256_sidecar(&format!("{url}.sha256"), &file).await?;

    let dest = ctx.tools.tool_dir(UV, version);
    ctx.download_and_extract(&url, Some(&checksum), kind, &dest, strip)
        .await?;

    // Point uv at the repository-scoped interpreter directory from the shim
    // itself, so `uv run` in a plain shell finds the same Python that tasks
    // under turbo do (and `uv python install` lands in the repository).
    let python_dir = python_install_dir(ctx);
    let bins = ctx.link_bins_with_env(
        &[
            (
                "uv".to_string(),
                dest.join_component(&ctx.platform.exe("uv")),
            ),
            (
                "uvx".to_string(),
                dest.join_component(&ctx.platform.exe("uvx")),
            ),
        ],
        &[(UV_PYTHON_INSTALL_DIR_ENV, python_dir.as_str())],
    )?;
    ctx.installed_tool(version, source, &dest, bins, BTreeMap::new())
}

/// Installs a Python interpreter with uv. Prefers the uv `turbo setup`
/// manages; falls back to one already on `PATH`.
pub(crate) async fn install_python(
    ctx: &InstallContext<'_>,
    request: &str,
    source: &str,
    manifest: &Manifest,
) -> Result<InstalledTool, Error> {
    let uv = managed_uv(ctx, manifest)
        .or_else(|| {
            which::which("uv")
                .ok()
                .and_then(|path| AbsoluteSystemPathBuf::try_from(path).ok())
        })
        .ok_or_else(|| Error::Unsupported {
            tool: PYTHON.into(),
            reason: "installing Python requires uv. Declare a uv version in pyproject.toml \
                     (`[tool.uv] required-version`) so turbo setup can install it, or put `uv` on \
                     PATH."
                .into(),
        })?;

    let install_dir = python_install_dir(ctx);
    install_dir
        .create_dir_all()
        .map_err(|source| Error::io(install_dir.as_str(), source))?;
    let bin_dir = ctx.tools.bin_dir();
    bin_dir
        .create_dir_all()
        .map_err(|source| Error::io(bin_dir.as_str(), source))?;
    ctx.run(
        uv.as_std_path(),
        &["python", "install", request],
        &[
            (UV_PYTHON_INSTALL_DIR_ENV, install_dir.as_str()),
            ("UV_PYTHON_BIN_DIR", bin_dir.as_str()),
        ],
        ctx.repo_root,
    )
    .await?;

    let env: BTreeMap<String, String> = [(
        UV_PYTHON_INSTALL_DIR_ENV.to_string(),
        ctx.tools.relative_of(&install_dir)?,
    )]
    .into();
    ctx.installed_tool(request, source, &install_dir, Vec::new(), env)
}

/// `.turbo/tools/python`, shared by the uv shims and the Python installer.
fn python_install_dir(ctx: &InstallContext<'_>) -> AbsoluteSystemPathBuf {
    ctx.tools.root().join_component(PYTHON)
}

fn managed_uv(ctx: &InstallContext<'_>, manifest: &Manifest) -> Option<AbsoluteSystemPathBuf> {
    let installed = manifest.get(UV)?;
    let binary = ctx
        .tools
        .resolve_relative(&installed.path)
        .join_component(&ctx.platform.exe("uv"));
    binary.exists().then_some(binary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pypi_releases() {
        let project: PypiProject = serde_json::from_str(
            r#"{"releases": {"0.5.0": [{"yanked": false}], "0.5.1": [{"yanked": true}], "0.6.0": []}}"#,
        )
        .unwrap();
        assert_eq!(project.releases.len(), 3);
        assert!(project.releases["0.5.1"][0].yanked);
    }
}
