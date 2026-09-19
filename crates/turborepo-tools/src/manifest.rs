//! `.turbo/tools/manifest.json`: the record of what `turbo setup` installed.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use turbopath::AbsoluteSystemPath;

use crate::Error;

/// Schema version written to the manifest so future turbo releases can
/// migrate or discard incompatible layouts.
pub const MANIFEST_SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    #[serde(default = "default_schema")]
    pub schema: u32,
    /// Installed tools keyed by tool name (`node`, `pnpm`, `rust`, `go`, …).
    #[serde(default)]
    pub tools: BTreeMap<String, InstalledTool>,
}

fn default_schema() -> u32 {
    MANIFEST_SCHEMA
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            schema: MANIFEST_SCHEMA,
            tools: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledTool {
    /// The resolved version that was installed.
    pub version: String,
    /// Where the declaration came from, e.g. `package.json#packageManager`.
    pub source: String,
    /// Install location, `/`-separated and relative to the tools directory.
    pub path: String,
    /// Names of the shims this tool owns inside `bin/`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bins: Vec<String>,
    /// Environment variables the tool needs at runtime. Values are
    /// `/`-separated paths relative to the tools directory.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

impl Manifest {
    pub fn read(path: &AbsoluteSystemPath) -> Result<Self, Error> {
        let Some(contents) = path
            .read_existing_to_string()
            .map_err(|source| Error::io(path.as_str(), source))?
        else {
            return Ok(Self::default());
        };
        let manifest: Manifest = serde_json::from_str(&contents).map_err(|err| Error::Parse {
            path: path.to_string(),
            reason: err.to_string(),
        })?;
        Ok(manifest)
    }

    pub fn write(&self, path: &AbsoluteSystemPath) -> Result<(), Error> {
        let mut contents = serde_json::to_string_pretty(self)?;
        contents.push('\n');
        path.create_with_contents(contents)
            .map_err(|source| Error::io(path.as_str(), source))
    }

    /// All runtime env entries across installed tools, in tool-name order.
    pub fn env(&self) -> impl Iterator<Item = (&str, &str)> {
        self.tools.values().flat_map(|tool| {
            tool.env
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str()))
        })
    }

    pub fn get(&self, tool: &str) -> Option<&InstalledTool> {
        self.tools.get(tool)
    }
}

#[cfg(test)]
mod tests {
    use turbopath::AbsoluteSystemPathBuf;

    use super::*;

    #[test]
    fn missing_manifest_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        let manifest = Manifest::read(&root.join_component("manifest.json")).unwrap();
        assert!(manifest.tools.is_empty());
    }

    #[test]
    fn round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        let path = root.join_component("manifest.json");
        let mut manifest = Manifest::default();
        assert_eq!(manifest.schema, MANIFEST_SCHEMA);
        manifest.tools.insert(
            "node".into(),
            InstalledTool {
                version: "22.1.0".into(),
                source: ".nvmrc".into(),
                path: "node/22.1.0".into(),
                bins: vec!["node".into(), "npm".into()],
                env: BTreeMap::new(),
            },
        );
        manifest.write(&path).unwrap();
        let read = Manifest::read(&path).unwrap();
        assert_eq!(read, manifest);
        assert_eq!(read.env().count(), 0);
    }
}
