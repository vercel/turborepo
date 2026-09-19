//! Go from go.dev/dl (or a mirror).

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::{
    Error, InstalledTool, archive::ArchiveKind, install::InstallContext, version::VersionSpec,
};

pub(crate) const TOOL: &str = "go";

/// One entry of `https://go.dev/dl/?mode=json&include=all`.
#[derive(Debug, Deserialize)]
struct GoRelease {
    /// e.g. `go1.22.5`
    version: String,
    #[serde(default)]
    stable: bool,
}

pub(crate) async fn resolve(
    ctx: &InstallContext<'_>,
    version: &VersionSpec,
    source: &str,
) -> Result<String, Error> {
    match version {
        VersionSpec::Exact(exact) => Ok(exact.trim_start_matches("go").to_string()),
        VersionSpec::Range(language_version) => {
            let url = format!("{}/?mode=json&include=all", ctx.sources.go_dist);
            let releases: Vec<GoRelease> = ctx.http.get_json(&url).await?;
            newest_patch(&releases, language_version).ok_or_else(|| Error::NoMatchingVersion {
                tool: TOOL.into(),
                requested: language_version.clone(),
                declared_in: source.into(),
            })
        }
        VersionSpec::Alias(alias) => Err(Error::InvalidVersion {
            tool: TOOL.into(),
            version: alias.clone(),
            declared_in: source.into(),
            reason: "expected a Go release such as 1.22 or 1.22.5".into(),
        }),
    }
}

/// Newest stable `go<language_version>.N` release.
fn newest_patch(releases: &[GoRelease], language_version: &str) -> Option<String> {
    let prefix = format!("go{language_version}.");
    releases
        .iter()
        .filter(|release| release.stable)
        .filter_map(|release| {
            let patch = release.version.strip_prefix(&prefix)?;
            let patch: u64 = patch.parse().ok()?;
            Some((patch, release.version.trim_start_matches("go").to_string()))
        })
        .max_by_key(|(patch, _)| *patch)
        .map(|(_, version)| version)
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
        "go{version}.{}.{}",
        ctx.platform.go_suffix(),
        kind.extension()
    );
    let url = format!("{}/{file}", ctx.sources.go_dist);
    let checksum = ctx.sha256_sidecar(&format!("{url}.sha256"), &file).await?;

    let dest = ctx.tools.tool_dir(TOOL, version);
    // Archives wrap everything in a top-level `go/` directory.
    ctx.download_and_extract(&url, Some(&checksum), kind, &dest, 1)
        .await?;

    let bins = ctx.link_bins(&[
        (
            "go".to_string(),
            dest.join_components(&["bin", &ctx.platform.exe("go")]),
        ),
        (
            "gofmt".to_string(),
            dest.join_components(&["bin", &ctx.platform.exe("gofmt")]),
        ),
    ])?;
    ctx.installed_tool(version, source, &dest, bins, BTreeMap::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_newest_stable_patch() {
        let releases: Vec<GoRelease> = serde_json::from_str(
            r#"[
                {"version": "go1.23rc1", "stable": false},
                {"version": "go1.22.5", "stable": true},
                {"version": "go1.22.10", "stable": true},
                {"version": "go1.22.11", "stable": false},
                {"version": "go1.21.0", "stable": true}
            ]"#,
        )
        .unwrap();
        assert_eq!(newest_patch(&releases, "1.22").unwrap(), "1.22.10");
        assert_eq!(newest_patch(&releases, "1.21").unwrap(), "1.21.0");
        assert!(newest_patch(&releases, "1.20").is_none());
    }
}
