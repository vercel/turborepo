use std::collections::BTreeMap;

use serde::Deserialize;

use super::{BerryPackage, DependencyMeta, LockfileData, Metadata};

const METADATA_KEY: &str = "__metadata";

/// Union type of yarn.lock metadata entry and package entries.
/// Only as a workaround for serde_yaml behavior around parsing numbers as
/// strings.
// In the ideal world this would be an enum, but serde_yaml currently has behavior
// where using `#[serde(untagged)]` or `#[serde(flatten)]` affects how it handles
// YAML numbers being parsed as Strings.
// If these macros are present, then it will refuse to parse 1 or 1.0 as a String
// and will instead only parse them as an int/float respectively.
// If these macros aren't present, then it will happily parse 1 or 1.0 as
// "1" and "1.0".
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Entry {
    version: String,
    language_name: Option<String>,
    dependencies: Option<BTreeMap<String, String>>,
    peer_dependencies: Option<BTreeMap<String, String>>,
    dependencies_meta: Option<BTreeMap<String, DependencyMeta>>,
    peer_dependencies_meta: Option<BTreeMap<String, DependencyMeta>>,
    bin: Option<BTreeMap<String, String>>,
    link_type: Option<String>,
    resolution: Option<String>,
    checksum: Option<String>,
    conditions: Option<String>,
    cache_key: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("missing resolution for entry {0}")]
    MissingResolution(String),
    #[error("multiple entry {0} has fields that should only appear in metadata")]
    InvalidMetadataFields(String),
    #[error("lockfile missing {METADATA_KEY} entry")]
    MissingMetadata,
}

impl TryFrom<BTreeMap<String, Entry>> for LockfileData {
    type Error = Error;

    fn try_from(mut value: BTreeMap<String, Entry>) -> Result<Self, Self::Error> {
        let Entry {
            version, cache_key, ..
        } = value.remove(METADATA_KEY).ok_or(Error::MissingMetadata)?;
        let metadata = Metadata { version, cache_key };
        let mut packages = BTreeMap::new();
        for (key, entry) in value {
            let Entry {
                version,
                language_name,
                dependencies,
                peer_dependencies,
                dependencies_meta,
                peer_dependencies_meta,
                bin,
                link_type,
                resolution,
                checksum,
                conditions,
                cache_key,
            } = entry;
            if cache_key.is_some() {
                return Err(Error::InvalidMetadataFields(key));
            }
            let resolution = resolution.ok_or_else(|| Error::MissingResolution(key.clone()))?;
            packages.insert(
                key,
                BerryPackage {
                    version,
                    language_name,
                    dependencies,
                    peer_dependencies,
                    dependencies_meta,
                    peer_dependencies_meta,
                    bin,
                    link_type,
                    resolution,
                    checksum,
                    conditions,
                },
            );
        }

        Ok(LockfileData { metadata, packages })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_requires_metadata() {
        let data = BTreeMap::new();
        assert!(LockfileData::try_from(data).is_err());
    }

    #[test]
    fn test_rejects_cache_key_in_packages() {
        let mut data = BTreeMap::new();
        data.insert(
            METADATA_KEY.to_string(),
            Entry {
                version: "1".into(),
                cache_key: Some("8".into()),
                ..Default::default()
            },
        );
        data.insert(
            "foo".to_string(),
            Entry {
                version: "1".into(),
                resolution: Some("resolved".into()),
                cache_key: Some("8".into()),
                ..Default::default()
            },
        );
        assert!(LockfileData::try_from(data).is_err());
    }

    #[test]
    fn test_requires_resolution() {
        let mut data = BTreeMap::new();
        data.insert(
            METADATA_KEY.to_string(),
            Entry {
                version: "1".into(),
                cache_key: Some("8".into()),
                ..Default::default()
            },
        );
        data.insert(
            "foo".to_string(),
            Entry {
                version: "1".into(),
                resolution: None,
                ..Default::default()
            },
        );
        assert!(LockfileData::try_from(data).is_err());
    }
}
