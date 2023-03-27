mod de;
mod identifiers;
mod ser;

use serde::{Deserialize, Serialize};

// We depend on BTree iteration being sorted
type Map<K, V> = std::collections::BTreeMap<K, V>;

// This is the direct representation of the lockfile as it appears on disk.
// More internal tracking is required for effectively altering the lockfile
#[derive(Debug, Deserialize, Serialize)]
struct LockfileData {
    #[serde(rename = "__metadata")]
    metadata: Metadata,
    #[serde(flatten)]
    packages: Map<String, BerryPackage>,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    version: u64,
    cache_key: String,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
struct BerryPackage {
    version: SemverString,
    language_name: Option<String>,
    dependencies: Option<Map<String, SemverString>>,
    peer_dependencies: Option<Map<String, SemverString>>,
    dependencies_meta: Option<Map<String, DependencyMeta>>,
    peer_dependencies_meta: Option<Map<String, DependencyMeta>>,
    // Structured metadata we need to persist
    bin: Option<Map<String, SemverString>>,
    link_type: Option<String>,
    resolution: Option<String>,
    checksum: Option<String>,
    conditions: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Clone, Copy)]
struct DependencyMeta {
    optional: Option<bool>,
    unplugged: Option<bool>,
}

// Newtype used exclusively for correct deserialization
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Default, Clone)]
struct SemverString(String);

impl From<SemverString> for String {
    fn from(value: SemverString) -> Self {
        value.0
    }
}

impl AsRef<str> for SemverString {
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

#[cfg(test)]
mod test {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_deserialize_lockfile() {
        let lockfile: LockfileData =
            serde_yaml::from_slice(include_bytes!("../../fixtures/berry.lock")).unwrap();
        assert_eq!(lockfile.metadata.version, 6);
        assert_eq!(lockfile.metadata.cache_key, "8c0");
    }

    #[test]
    fn test_roundtrip() {
        let contents = include_str!("../../fixtures/berry.lock");
        let lockfile: LockfileData = serde_yaml::from_str(contents).unwrap();
        let new_contents = lockfile.to_string();
        assert_eq!(contents, new_contents);
    }
}
