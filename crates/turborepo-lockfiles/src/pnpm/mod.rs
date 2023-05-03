use serde::{Deserialize, Serialize};

type Map<K, V> = std::collections::BTreeMap<K, V>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("yaml: {0}")]
    Yaml(#[from] serde_yaml::Error),
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PnpmLockfileData {
    lockfile_version: LockfileVersion,
    never_built_dependencies: Option<Vec<String>>,
    only_built_dependencies: Option<Vec<String>>,
    overrides: Option<Map<String, String>>,
    package_extensions_checksum: Option<String>,
    patched_dependencies: Option<Map<String, PatchFile>>,
    importers: Map<String, ProjectSnapshot>,
    packages: Option<Map<String, PackageSnapshot>>,
    time: Option<Map<String, String>>,
}

#[derive(Debug, PartialEq, Eq)]
struct LockfileVersion {
    version: String,
    format: VersionFormat,
}

#[derive(Debug, PartialEq, Eq)]
enum VersionFormat {
    String,
    Float,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PatchFile {
    path: String,
    hash: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSnapshot {
    #[serde(flatten)]
    dependencies: DependencyInfo,
    dependencies_meta: Option<Map<String, DependenciesMeta>>,
    publish_directory: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", untagged)]
pub enum DependencyInfo {
    PreV6 {
        specifiers: Option<Map<String, String>>,
        dependencies: Option<Map<String, String>>,
        optional_dependencies: Option<Map<String, String>>,
        dev_dependencies: Option<Map<String, String>>,
    },
    V6 {
        dependencies: Option<Map<String, Dependency>>,
        optional_dependencies: Option<Map<String, Dependency>>,
        dev_dependencies: Option<Map<String, Dependency>>,
    },
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Dependency {
    specifier: String,
    version: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PackageSnapshot {
    // can we make this flow?/is it necessary?
    resolution: PackageResolution,
    id: Option<String>,

    name: Option<String>,
    version: Option<String>,

    dependencies: Option<Map<String, String>>,
    optional_dependencies: Option<Map<String, String>>,
    patched: Option<bool>,

    #[serde(flatten)]
    other: Map<String, serde_yaml::Value>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DependenciesMeta {
    injected: Option<bool>,
    node: Option<String>,
    patch: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackageResolution {
    #[serde(rename = "type")]
    type_field: Option<String>,
    integrity: Option<String>,
    tarball: Option<String>,
    dir: Option<String>,
    repo: Option<String>,
    commit: Option<String>,
}

impl PnpmLockfileData {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        let this = serde_yaml::from_slice(bytes)?;
        Ok(this)
    }
}

impl From<f32> for LockfileVersion {
    fn from(value: f32) -> Self {
        Self {
            version: value.to_string(),
            format: VersionFormat::Float,
        }
    }
}

impl From<String> for LockfileVersion {
    fn from(value: String) -> Self {
        Self {
            version: value,
            format: VersionFormat::String,
        }
    }
}

impl<'de> Deserialize<'de> for LockfileVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum StringOrNum {
            Str(String),
            Num(f32),
        }

        Ok(match StringOrNum::deserialize(deserializer)? {
            StringOrNum::Num(x) => LockfileVersion::from(x),
            StringOrNum::Str(s) => LockfileVersion::from(s),
        })
    }
}

impl Serialize for LockfileVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self.format {
            VersionFormat::String => serializer.serialize_str(&self.version),
            VersionFormat::Float => serializer.serialize_f32(
                self.version
                    .parse()
                    .expect("Expected lockfile version to be valid f32"),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_roundtrip() {
        for fixture in &[
            &include_bytes!("../../fixtures/pnpm6-workspace.yaml")[..],
            &include_bytes!("../../fixtures/pnpm7-workspace.yaml")[..],
            &include_bytes!("../../fixtures/pnpm8.yaml")[..],
        ] {
            let lockfile = PnpmLockfileData::from_bytes(fixture).unwrap();
            let serialized_lockfile = serde_yaml::to_string(&lockfile).unwrap();
            let lockfile_from_serialized =
                serde_yaml::from_slice(serialized_lockfile.as_bytes()).unwrap();
            assert_eq!(lockfile, lockfile_from_serialized);
        }
    }
}
