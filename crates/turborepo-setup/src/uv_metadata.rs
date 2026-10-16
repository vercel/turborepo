//! Pure bounded parsing of Astral's uv version index (NDJSON) and a release's
//! `sha256.sum`. The caller must fetch both from the approved origin; parsing
//! does not authenticate a publisher or download artifacts.

use std::collections::BTreeMap;

use semver::Version;
use serde_json::Value;

use crate::{node_discovery::UniqueJson, node_metadata};

pub const MAX_INDEX_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_CHECKSUM_BYTES: usize = 64 * 1024;
const MAX_RELEASES: usize = 16_384;
const MAX_ARTIFACTS: usize = 128;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("uv release metadata exceeds its byte or entry limit")]
    Limit,
    #[error("invalid or ambiguous uv release index")]
    Index,
    #[error("invalid or ambiguous uv sha256.sum")]
    Checksums,
}

/// Exact releases mapped to the index's default-variant target digests. Index
/// digests only cross-check the release's own checksum file.
#[derive(Clone, Debug)]
pub struct ReleaseIndex(BTreeMap<Version, BTreeMap<String, String>>);

impl ReleaseIndex {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_INDEX_BYTES {
            return Err(Error::Limit);
        }
        let text = std::str::from_utf8(bytes).map_err(|_| Error::Index)?;
        let mut releases = BTreeMap::new();
        for line in text.lines().filter(|line| !line.is_empty()) {
            if releases.len() == MAX_RELEASES {
                return Err(Error::Limit);
            }
            let UniqueJson(entry) = serde_json::from_str(line).map_err(|_| Error::Index)?;
            let version = entry
                .get("version")
                .and_then(Value::as_str)
                .and_then(crate::uv::exact_release)
                .ok_or(Error::Index)?;
            let artifacts = entry
                .get("artifacts")
                .and_then(Value::as_array)
                .ok_or(Error::Index)?;
            if artifacts.len() > MAX_ARTIFACTS {
                return Err(Error::Limit);
            }
            let mut digests = BTreeMap::new();
            for artifact in artifacts {
                let field = |name| artifact.get(name).and_then(Value::as_str);
                let (Some(target), Some(variant), Some(sha256)) =
                    (field("platform"), field("variant"), field("sha256"))
                else {
                    return Err(Error::Index);
                };
                if target.is_empty()
                    || target.len() > 64
                    || !target
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
                    || sha256.len() != 64
                    || !sha256
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                {
                    return Err(Error::Index);
                }
                // Unknown upstream fields and non-default variants do not
                // influence the selected official archive.
                if variant == "default"
                    && digests
                        .insert(target.to_owned(), sha256.to_owned())
                        .is_some()
                {
                    return Err(Error::Index);
                }
            }
            if releases.insert(version, digests).is_some() {
                return Err(Error::Index);
            }
        }
        if releases.is_empty() {
            return Err(Error::Index);
        }
        Ok(Self(releases))
    }

    /// Ascending exact releases, independent of upstream line order.
    pub fn versions(&self) -> impl DoubleEndedIterator<Item = &Version> {
        self.0.keys()
    }

    pub fn sha256(&self, version: &Version, target: &str) -> Option<&str> {
        self.0.get(version)?.get(target).map(String::as_str)
    }
}

/// A release's sha256sum-format manifest (`<digest> *<filename>`).
pub fn checksums(bytes: &[u8]) -> Result<node_metadata::Checksums, Error> {
    if bytes.len() > MAX_CHECKSUM_BYTES {
        return Err(Error::Limit);
    }
    node_metadata::Checksums::parse(bytes).map_err(|error| match error {
        node_metadata::Error::Limit => Error::Limit,
        _ => Error::Checksums,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn line(version: &str, artifacts: Value) -> String {
        json!({"version":version,"date":"2026-10-09T19:49:55Z","artifacts":artifacts}).to_string()
    }
    fn artifact(target: &str, variant: &str) -> Value {
        json!({"platform":target,"variant":variant,"url":"https://example.invalid/x",
            "archive_format":"tar.gz","sha256":"a".repeat(64)})
    }

    #[test]
    fn index_orders_releases_and_ignores_unknown_fields_and_variants() {
        let index = format!(
            "{}\n{}\n",
            line(
                "0.13.0",
                json!([
                    artifact("x86_64-unknown-linux-gnu", "default"),
                    artifact("x86_64-unknown-linux-gnu", "debug")
                ])
            ),
            line("0.9.1", json!([]))
        );
        let index = ReleaseIndex::parse(index.as_bytes()).unwrap();
        let versions: Vec<_> = index.versions().map(ToString::to_string).collect();
        assert_eq!(versions, ["0.9.1", "0.13.0"]);
        let latest = Version::new(0, 13, 0);
        assert_eq!(
            index.sha256(&latest, "x86_64-unknown-linux-gnu"),
            Some("a".repeat(64).as_str())
        );
        assert_eq!(index.sha256(&latest, "aarch64-apple-darwin"), None);
    }

    #[test]
    fn ambiguous_or_noncanonical_index_entries_fail_closed() {
        let duplicate_target = json!([
            artifact("x86_64-unknown-linux-gnu", "default"),
            artifact("x86_64-unknown-linux-gnu", "default")
        ]);
        let mut bad_digest = artifact("x86_64-unknown-linux-gnu", "default");
        bad_digest["sha256"] = json!("A".repeat(64));
        for index in [
            String::new(),
            format!("{}\n{}", line("0.5.0", json!([])), line("0.5.0", json!([]))),
            line("0.5", json!([])),
            line("0.5.0-rc.1", json!([])),
            line("0.5.0", duplicate_target),
            line("0.5.0", json!([bad_digest])),
            line("0.5.0", json!([artifact("../escape", "default")])),
            r#"{"version":"0.5.0","version":"0.6.0","artifacts":[]}"#.into(),
            "not json".into(),
        ] {
            assert_eq!(
                ReleaseIndex::parse(index.as_bytes()).unwrap_err(),
                Error::Index,
                "{index}"
            );
        }
        assert_eq!(
            ReleaseIndex::parse(&vec![b' '; MAX_INDEX_BYTES + 1]).unwrap_err(),
            Error::Limit
        );
    }

    #[test]
    fn checksum_manifest_reuses_sha256sum_parsing() {
        let sums =
            checksums(format!("{} *uv-x86_64-apple-darwin.tar.gz\n", "b".repeat(64)).as_bytes())
                .unwrap();
        assert_eq!(
            sums.sha256("uv-x86_64-apple-darwin.tar.gz"),
            Some("b".repeat(64).as_str())
        );
        assert_eq!(checksums(b"oops").unwrap_err(), Error::Checksums);
        assert_eq!(
            checksums(&vec![b'a'; MAX_CHECKSUM_BYTES + 1]).unwrap_err(),
            Error::Limit
        );
    }
}
