//! Node.js from nodejs.org (or a mirror).

use std::collections::BTreeMap;

use node_semver::Range;
use serde::Deserialize;

use crate::{
    Error, InstalledTool,
    archive::ArchiveKind,
    http::Checksum,
    install::InstallContext,
    shim,
    version::{VersionSpec, max_satisfying},
};

pub(crate) const TOOL: &str = "node";

/// One entry of `https://nodejs.org/dist/index.json`.
#[derive(Debug, Deserialize)]
struct NodeRelease {
    version: String,
    /// `false` or the LTS codename.
    #[serde(default)]
    lts: serde_json::Value,
}

impl NodeRelease {
    fn lts_name(&self) -> Option<&str> {
        self.lts.as_str()
    }
}

pub(crate) async fn resolve(
    ctx: &InstallContext<'_>,
    version: &VersionSpec,
    source: &str,
) -> Result<String, Error> {
    match version {
        VersionSpec::Exact(exact) => Ok(exact.trim_start_matches('v').to_string()),
        VersionSpec::Range(range) => {
            let parsed = Range::parse(range).map_err(|err| Error::InvalidVersion {
                tool: TOOL.into(),
                version: range.clone(),
                declared_in: source.into(),
                reason: err.to_string(),
            })?;
            let releases = index(ctx).await?;
            max_satisfying(
                releases.iter().map(|release| release.version.as_str()),
                &parsed,
            )
            .map(|version| version.to_string())
            .ok_or_else(|| no_match(range, source))
        }
        VersionSpec::Alias(alias) => {
            let releases = index(ctx).await?;
            let candidates: Vec<&NodeRelease> = match alias.as_str() {
                "lts/*" => releases
                    .iter()
                    .filter(|release| release.lts_name().is_some())
                    .collect(),
                other if other.starts_with("lts/") => {
                    let codename = &other["lts/".len()..];
                    releases
                        .iter()
                        .filter(|release| {
                            release
                                .lts_name()
                                .is_some_and(|name| name.eq_ignore_ascii_case(codename))
                        })
                        .collect()
                }
                _ => releases.iter().collect(),
            };
            candidates
                .iter()
                .filter_map(|release| {
                    node_semver::Version::parse(release.version.trim_start_matches('v')).ok()
                })
                .filter(|version| version.pre_release.is_empty())
                .max()
                .map(|version| version.to_string())
                .ok_or_else(|| no_match(alias, source))
        }
    }
}

fn no_match(requested: &str, source: &str) -> Error {
    Error::NoMatchingVersion {
        tool: TOOL.into(),
        requested: requested.to_string(),
        declared_in: source.to_string(),
    }
}

async fn index(ctx: &InstallContext<'_>) -> Result<Vec<NodeRelease>, Error> {
    let url = format!("{}/index.json", ctx.sources.node_dist);
    ctx.http.get_json(&url).await
}

pub(crate) async fn install(
    ctx: &InstallContext<'_>,
    version: &str,
    source: &str,
) -> Result<InstalledTool, Error> {
    let kind = if ctx.platform.is_windows() {
        ArchiveKind::Zip
    } else {
        ArchiveKind::TarGz
    };
    let file = format!(
        "node-v{version}-{}.{}",
        ctx.platform.node_suffix(),
        kind.extension()
    );
    let base = format!("{}/v{version}", ctx.sources.node_dist);
    let shasums = ctx.http.get_text(&format!("{base}/SHASUMS256.txt")).await?;
    let checksum =
        Checksum::from_shasums(&shasums, &file).ok_or_else(|| Error::ChecksumMissing {
            url: format!("{base}/SHASUMS256.txt"),
            file: file.clone(),
        })??;

    let dest = ctx.tools.tool_dir(TOOL, version);
    ctx.download_and_extract(&format!("{base}/{file}"), Some(&checksum), kind, &dest, 1)
        .await?;

    let bin_dir = ctx.tools.bin_dir();
    let mut bins = Vec::new();
    let (node_binary, npm_root) = if ctx.platform.is_windows() {
        (
            dest.join_component("node.exe"),
            dest.join_component("node_modules"),
        )
    } else {
        (
            dest.join_components(&["bin", "node"]),
            dest.join_components(&["lib", "node_modules"]),
        )
    };
    shim::link_binary(&bin_dir, "node", &node_binary)?;
    bins.push("node".to_string());
    for (name, script) in [
        ("npm", &["npm", "bin", "npm-cli.js"][..]),
        ("npx", &["npm", "bin", "npx-cli.js"][..]),
        ("corepack", &["corepack", "dist", "corepack.js"][..]),
    ] {
        let script = npm_root.join_components(script);
        if script.exists() {
            shim::node_script(&bin_dir, name, &script)?;
            bins.push(name.to_string());
        }
    }

    ctx.installed_tool(version, source, &dest, bins, BTreeMap::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_index_entries() {
        let releases: Vec<NodeRelease> = serde_json::from_str(
            r#"[{"version":"v22.1.0","lts":false},{"version":"v20.11.1","lts":"Iron"}]"#,
        )
        .unwrap();
        assert_eq!(releases[0].lts_name(), None);
        assert_eq!(releases[1].lts_name(), Some("Iron"));
    }
}
