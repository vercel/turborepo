//! Version requests and semver resolution helpers.

use std::fmt;

use node_semver::{Range, Version};

/// What a declaration asked for, before resolution against a release index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionSpec {
    /// A fully pinned version, installed verbatim (`22.1.0`, `1.22.3`,
    /// `nightly-2026-07-03`).
    Exact(String),
    /// A semver range resolved to the newest matching release (`^9.0.0`,
    /// `>=18`, `22`, `1.22` for Go).
    Range(String),
    /// A named alias resolved by the tool's own index (`lts/*`, `lts/jod`,
    /// `latest` for Node.js).
    Alias(String),
}

impl VersionSpec {
    /// Classifies a raw request: exact semver versions stay exact, anything
    /// else that parses as a range becomes one.
    pub fn from_semverish(raw: &str) -> Result<Self, String> {
        let raw = raw.trim();
        let stripped = raw.strip_prefix('v').unwrap_or(raw);
        if Version::parse(stripped).is_ok() {
            return Ok(Self::Exact(stripped.to_string()));
        }
        Range::parse(raw)
            .map(|_| Self::Range(raw.to_string()))
            .map_err(|err| err.to_string())
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Exact(s) | Self::Range(s) | Self::Alias(s) => s,
        }
    }
}

impl fmt::Display for VersionSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Newest version in `candidates` that satisfies `range`. Prereleases are
/// only considered when the range itself names one, matching npm semantics.
pub fn max_satisfying<'a>(
    candidates: impl IntoIterator<Item = &'a str>,
    range: &Range,
) -> Option<Version> {
    candidates
        .into_iter()
        .filter_map(|raw| Version::parse(raw.strip_prefix('v').unwrap_or(raw)).ok())
        .filter(|version| range.satisfies(version))
        .max()
}

/// Translates a PEP 440 specifier set (`>=0.5.0,<0.6`, `==0.5.1`, `~=0.5.1`)
/// into a node-semver range string. uv's `required-version` uses PEP 440.
pub fn pep440_to_semver(spec: &str) -> String {
    spec.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            if let Some(rest) = part.strip_prefix("==") {
                format!("={}", rest.trim())
            } else if let Some(rest) = part.strip_prefix("~=") {
                format!("~{}", rest.trim())
            } else if let Some(rest) = part.strip_prefix("!=") {
                // node-semver has no inequality; the closest expression is
                // "anything but exactly this", which we express as two sides.
                let rest = rest.trim();
                format!("<{rest} || >{rest}")
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_requests() {
        assert_eq!(
            VersionSpec::from_semverish("v22.1.0").unwrap(),
            VersionSpec::Exact("22.1.0".into())
        );
        assert_eq!(
            VersionSpec::from_semverish("^9").unwrap(),
            VersionSpec::Range("^9".into())
        );
        assert_eq!(
            VersionSpec::from_semverish("22").unwrap(),
            VersionSpec::Range("22".into())
        );
        assert!(VersionSpec::from_semverish("lts/*").is_err());
    }

    #[test]
    fn picks_newest_matching() {
        let range = Range::parse(">=18 <21").unwrap();
        let versions = [
            "v17.9.0",
            "v18.20.0",
            "v20.11.1",
            "v21.0.0",
            "v20.12.0-rc.1",
        ];
        assert_eq!(
            max_satisfying(versions, &range).unwrap(),
            Version::parse("20.11.1").unwrap()
        );
    }

    #[test]
    fn converts_pep440() {
        assert_eq!(pep440_to_semver(">=0.5.0, <0.6"), ">=0.5.0 <0.6");
        assert_eq!(pep440_to_semver("==0.5.1"), "=0.5.1");
        assert_eq!(pep440_to_semver("~=0.5.1"), "~0.5.1");
        assert!(Range::parse(pep440_to_semver(">=0.5.0,<0.6")).is_ok());
    }
}
