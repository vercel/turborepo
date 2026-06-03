use std::collections::HashMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use super::{BTreeSet, Map, PackageIndex};

/// Represents a platform constraint that can be either inclusive or exclusive.
/// This matches Bun's Negatable type for os/cpu/libc fields.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum Negatable {
    /// No constraint - package works on all platforms
    #[default]
    None,
    /// Single platform constraint
    Single(String),
    /// Multiple platform constraints (all must match)
    Multiple(Vec<String>),
    /// Negated constraints - package works on all platforms except these
    Negated(Vec<String>),
}

impl Serialize for Negatable {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Negatable::None => serializer.serialize_str("none"),
            Negatable::Single(platform) => serializer.serialize_str(platform),
            Negatable::Multiple(platforms) => platforms.serialize(serializer),
            Negatable::Negated(platforms) => {
                let negated_platforms: Vec<String> =
                    platforms.iter().map(|p| format!("!{p}")).collect();
                negated_platforms.serialize(serializer)
            }
        }
    }
}

impl<'de> Deserialize<'de> for Negatable {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value: Value = Value::deserialize(deserializer)?;

        match value {
            Value::String(s) => {
                // Treating "none" as special breaks wasm packages that use cpu="none"
                if let Some(stripped) = s.strip_prefix('!') {
                    Ok(Negatable::Negated(vec![stripped.to_string()]))
                } else {
                    Ok(Negatable::Single(s))
                }
            }
            Value::Array(arr) => {
                let platforms: Result<Vec<String>, _> = arr
                    .into_iter()
                    .map(|v| match v {
                        Value::String(s) => Ok(s),
                        _ => Err(serde::de::Error::invalid_type(
                            serde::de::Unexpected::Other("non-string in platform array"),
                            &"string",
                        )),
                    })
                    .collect();

                let platforms = platforms?;

                let has_negated = platforms.iter().any(|p| p.starts_with('!'));
                let has_non_negated = platforms.iter().any(|p| !p.starts_with('!'));

                if has_negated && has_non_negated {
                    // npm spec requires explicit allows to take precedence over denies
                    let allowed_platforms: Vec<String> = platforms
                        .into_iter()
                        .filter(|p| !p.starts_with('!'))
                        .collect();
                    Ok(Negatable::Multiple(allowed_platforms))
                } else if has_negated {
                    // Strip '!' prefix to extract the blocked platforms
                    let negated_platforms: Vec<String> = platforms
                        .into_iter()
                        .filter_map(|p| p.strip_prefix('!').map(str::to_string))
                        .collect();
                    Ok(Negatable::Negated(negated_platforms))
                } else {
                    Ok(Negatable::Multiple(platforms))
                }
            }
            _ => Err(serde::de::Error::invalid_type(
                serde::de::Unexpected::Other("non-string/array for platform constraint"),
                &"string or array of strings",
            )),
        }
    }
}

impl Negatable {
    /// Returns true if this constraint allows the given platform
    #[allow(dead_code)]
    pub fn allows(&self, platform: &str) -> bool {
        match self {
            Negatable::None => true,
            Negatable::Single(p) => p == platform,
            Negatable::Multiple(platforms) => platforms.contains(&platform.to_string()),
            Negatable::Negated(platforms) => !platforms.contains(&platform.to_string()),
        }
    }

    /// Returns true if this is an empty/none constraint
    pub fn is_none(&self) -> bool {
        matches!(self, Negatable::None)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to strip commas: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("Failed to strip commas: {0}")]
    Format(#[from] biome_formatter::FormatError),
    #[error("Failed to strip commas: {0}")]
    Print(#[from] biome_formatter::PrintError),
    #[error("{ident} had two entries with differing checksums: {sha1}, {sha2}")]
    MismatchedShas {
        ident: String,
        sha1: String,
        sha2: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LockfileVersion {
    V0 = 0,
    V1 = 1,
}

impl LockfileVersion {
    #[allow(dead_code)]
    pub(super) fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::V0),
            1 => Some(Self::V1),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub(super) fn as_i32(self) -> i32 {
        self as i32
    }
}

#[derive(Debug)]
pub struct BunLockfile {
    pub(super) data: BunLockfileData,
    pub(super) key_to_entry: HashMap<String, String>,
    pub(super) index: PackageIndex,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BunLockfileData {
    pub(super) lockfile_version: i32,
    #[serde(default)]
    pub(super) config_version: Option<i32>,
    pub(super) workspaces: Map<String, WorkspaceEntry>,
    #[serde(default)]
    pub(super) trusted_dependencies: Vec<String>,
    #[serde(default)]
    pub(super) overrides: Map<String, String>,
    #[serde(default)]
    pub(super) catalog: Map<String, String>,
    #[serde(default)]
    pub(super) catalogs: Map<String, Map<String, String>>,
    pub(super) packages: Map<String, PackageEntry>,
    #[serde(default)]
    pub(super) patched_dependencies: Map<String, String>,
}

#[derive(Debug, Deserialize, PartialEq, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceEntry {
    pub(super) name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) dependencies: Option<Map<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) dev_dependencies: Option<Map<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) optional_dependencies: Option<Map<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) peer_dependencies: Option<Map<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) optional_peers: Option<Vec<String>>,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct PackageEntry {
    pub(super) ident: String,
    pub(super) registry: Option<String>,
    // Present for all package types except root deps
    pub(super) info: Option<PackageInfo>,
    // Present on registry
    pub(super) checksum: Option<String>,
    pub(super) root: Option<RootInfo>,
}

#[derive(Debug, Deserialize, Default, PartialEq, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PackageInfo {
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub(super) dependencies: Map<String, String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub(super) dev_dependencies: Map<String, String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub(super) optional_dependencies: Map<String, String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub(super) peer_dependencies: Map<String, String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub(super) optional_peers: BTreeSet<String>,
    /// Operating system constraint for this package
    #[serde(default, skip_serializing_if = "Negatable::is_none")]
    pub(super) os: Negatable,
    /// CPU architecture constraint for this package
    #[serde(default, skip_serializing_if = "Negatable::is_none")]
    pub(super) cpu: Negatable,
    // We do not care about the rest here
    // the values here should be generic
    #[serde(flatten)]
    pub(super) other: Map<String, Value>,
}
#[derive(Debug, Deserialize, PartialEq, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RootInfo {
    pub(super) bin: Option<String>,
    pub(super) bin_dir: Option<String>,
}
impl PackageEntry {
    // Extracts version from key
    pub(super) fn version(&self) -> &str {
        self.ident
            .rsplit_once('@')
            .map(|(_, version)| version)
            .unwrap_or(&self.ident)
    }
}

impl PackageInfo {
    pub fn is_empty(&self) -> bool {
        self.dependencies.is_empty()
            && self.dev_dependencies.is_empty()
            && self.optional_dependencies.is_empty()
            && self.peer_dependencies.is_empty()
            && self.optional_peers.is_empty()
            && self.other.is_empty()
    }

    pub fn all_dependencies(&self) -> impl Iterator<Item = (&str, &str)> {
        [
            self.dependencies.iter(),
            self.dev_dependencies.iter(),
            self.optional_dependencies.iter(),
            self.peer_dependencies.iter(),
        ]
        .into_iter()
        .flatten()
        .map(|(k, v)| (k.as_str(), v.as_str()))
    }
}
