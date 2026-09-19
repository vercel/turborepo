//! JavaScript package managers: npm, pnpm and yarn from the npm registry
//! (the same tarballs Corepack installs), bun from its GitHub releases.

use std::collections::BTreeMap;

use node_semver::{Range, Version};
use serde::Deserialize;
use turbopath::AbsoluteSystemPath;

use crate::{
    Error, InstalledTool,
    archive::ArchiveKind,
    http::Checksum,
    install::InstallContext,
    shim,
    version::{VersionSpec, max_satisfying},
};

/// The npm registry serves a much smaller document when asked for this.
const ABBREVIATED_PACKUMENT: &str = "application/vnd.npm.install-v1+json";

/// Abbreviated packument: we only need the version list.
#[derive(Debug, Deserialize)]
struct Packument {
    versions: BTreeMap<String, serde_json::Value>,
}

/// `GET <registry>/<package>/<version>`.
#[derive(Debug, Deserialize)]
struct VersionMetadata {
    dist: Dist,
    #[serde(default)]
    bin: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct Dist {
    tarball: String,
    #[serde(default)]
    integrity: Option<String>,
}

/// Registry package that publishes `name` at `version`. Yarn 1 and Yarn
/// Berry live in different packages.
fn registry_package(name: &str, major: Option<u64>) -> String {
    match (name, major) {
        ("yarn", Some(major)) if major >= 2 => "@yarnpkg/cli-dist".to_string(),
        _ => name.to_string(),
    }
}

fn encoded_package(package: &str) -> String {
    package.replace('/', "%2F")
}

pub(crate) async fn resolve(
    ctx: &InstallContext<'_>,
    name: &str,
    version: &VersionSpec,
    source: &str,
) -> Result<String, Error> {
    match version {
        VersionSpec::Exact(exact) => Ok(exact.clone()),
        VersionSpec::Range(range) => {
            let parsed = Range::parse(range).map_err(|err| Error::InvalidVersion {
                tool: name.into(),
                version: range.clone(),
                declared_in: source.into(),
                reason: err.to_string(),
            })?;
            let major = parsed.min_version().map(|version| version.major);
            let package = registry_package(name, major);
            let url = format!("{}/{}", ctx.sources.npm_registry, encoded_package(&package));
            let packument: Packument = ctx
                .http
                .get_json_with_accept(&url, Some(ABBREVIATED_PACKUMENT))
                .await?;
            max_satisfying(packument.versions.keys().map(String::as_str), &parsed)
                .map(|version| version.to_string())
                .ok_or_else(|| Error::NoMatchingVersion {
                    tool: name.into(),
                    requested: range.clone(),
                    declared_in: source.into(),
                })
        }
        VersionSpec::Alias(alias) => Err(Error::InvalidVersion {
            tool: name.into(),
            version: alias.clone(),
            declared_in: source.into(),
            reason: "package manager versions must be semver".into(),
        }),
    }
}

pub(crate) async fn install(
    ctx: &InstallContext<'_>,
    name: &str,
    version: &str,
    declared_checksum: Option<&Checksum>,
    source: &str,
) -> Result<InstalledTool, Error> {
    if name == "bun" {
        return install_bun(ctx, version, source).await;
    }
    let major = Version::parse(version).ok().map(|v| v.major);
    let package = registry_package(name, major);
    let url = format!(
        "{}/{}/{version}",
        ctx.sources.npm_registry,
        encoded_package(&package)
    );
    let metadata: VersionMetadata = ctx.http.get_json(&url).await?;
    let registry_checksum = metadata
        .dist
        .integrity
        .as_deref()
        .map(Checksum::from_sri)
        .transpose()?
        .flatten();
    let checksum = declared_checksum.cloned().or(registry_checksum);

    let dest = ctx.tools.tool_dir(name, version);
    ctx.download_and_extract(
        &metadata.dist.tarball,
        checksum.as_ref(),
        ArchiveKind::TarGz,
        &dest,
        1,
    )
    .await?;

    // Prefer the extracted package.json's `bin` map; the registry document
    // carries the same data, so fall back to it if the file is unreadable.
    let bin_map = read_bin_map(&dest)
        .or_else(|| bin_map_from_value(&package, &metadata.bin))
        .unwrap_or_default();
    let bin_dir = ctx.tools.bin_dir();
    let mut bins = Vec::new();
    for (bin_name, relative) in bin_map {
        let components: Vec<&str> = relative
            .trim_start_matches("./")
            .split('/')
            .filter(|part| !part.is_empty())
            .collect();
        let script = dest.join_components(&components);
        if !script.exists() {
            continue;
        }
        shim::node_script(&bin_dir, &bin_name, &script)?;
        bins.push(bin_name);
    }
    if bins.is_empty() {
        return Err(Error::Response {
            url,
            reason: format!("{package}@{version} publishes no `bin` entries"),
        });
    }

    ctx.installed_tool(version, source, &dest, bins, BTreeMap::new())
}

fn read_bin_map(package_dir: &AbsoluteSystemPath) -> Option<BTreeMap<String, String>> {
    let contents = package_dir
        .join_component("package.json")
        .read_to_string()
        .ok()?;
    let value: serde_json::Value = serde_json::from_str(&contents).ok()?;
    let name = value.get("name").and_then(|name| name.as_str())?;
    bin_map_from_value(name, value.get("bin")?)
}

/// `bin` is either a path (named after the unscoped package) or a map.
fn bin_map_from_value(package: &str, bin: &serde_json::Value) -> Option<BTreeMap<String, String>> {
    match bin {
        serde_json::Value::String(path) => {
            let name = package.rsplit('/').next().unwrap_or(package);
            Some([(name.to_string(), path.clone())].into())
        }
        serde_json::Value::Object(map) => Some(
            map.iter()
                .filter_map(|(name, path)| {
                    path.as_str().map(|path| (name.clone(), path.to_string()))
                })
                .collect(),
        ),
        _ => None,
    }
}

async fn install_bun(
    ctx: &InstallContext<'_>,
    version: &str,
    source: &str,
) -> Result<InstalledTool, Error> {
    let file = format!("bun-{}.zip", ctx.platform.bun_suffix());
    let base = format!("{}/bun-v{version}", ctx.sources.bun_releases);
    let shasums = ctx.http.get_text(&format!("{base}/SHASUMS256.txt")).await?;
    let checksum =
        Checksum::from_shasums(&shasums, &file).ok_or_else(|| Error::ChecksumMissing {
            url: format!("{base}/SHASUMS256.txt"),
            file: file.clone(),
        })??;

    let dest = ctx.tools.tool_dir("bun", version);
    ctx.download_and_extract(
        &format!("{base}/{file}"),
        Some(&checksum),
        ArchiveKind::Zip,
        &dest,
        1,
    )
    .await?;

    let binary = dest.join_component(&ctx.platform.exe("bun"));
    // bun behaves as `bun x` when invoked through a `bunx` name.
    let bins = ctx.link_bins(&[
        ("bun".to_string(), binary.clone()),
        ("bunx".to_string(), binary),
    ])?;
    ctx.installed_tool(version, source, &dest, bins, BTreeMap::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yarn_versions_map_to_the_right_package() {
        assert_eq!(registry_package("yarn", Some(1)), "yarn");
        assert_eq!(registry_package("yarn", Some(4)), "@yarnpkg/cli-dist");
        assert_eq!(registry_package("pnpm", Some(9)), "pnpm");
        assert_eq!(encoded_package("@yarnpkg/cli-dist"), "@yarnpkg%2Fcli-dist");
    }

    #[test]
    fn bin_maps() {
        let single = serde_json::json!("bin/pnpm.cjs");
        assert_eq!(
            bin_map_from_value("pnpm", &single).unwrap(),
            [("pnpm".to_string(), "bin/pnpm.cjs".to_string())].into()
        );
        let scoped = serde_json::json!("bin/yarn.js");
        assert_eq!(
            bin_map_from_value("@yarnpkg/cli-dist", &scoped)
                .unwrap()
                .keys()
                .next()
                .unwrap(),
            "cli-dist"
        );
        let map = serde_json::json!({"npm": "bin/npm-cli.js", "npx": "bin/npx-cli.js"});
        assert_eq!(bin_map_from_value("npm", &map).unwrap().len(), 2);
    }
}
