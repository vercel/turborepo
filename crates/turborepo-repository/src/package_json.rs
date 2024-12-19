use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use anyhow::Result;
use biome_deserialize::{json::deserialize_from_json_str, Text};
use biome_deserialize_macros::Deserializable;
use biome_diagnostics::DiagnosticExt;
use biome_json_parser::JsonParserOptions;
use miette::Diagnostic;
use serde::Serialize;
use turbopath::{AbsoluteSystemPath, RelativeUnixPathBuf};
use turborepo_errors::{ParseDiagnostic, Spanned, WithMetadata};
use turborepo_unescape::UnescapedString;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageJson {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_manager: Option<Spanned<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependencies: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dev_dependencies: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optional_dependencies: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_dependencies: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub scripts: BTreeMap<String, Spanned<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolutions: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pnpm: Option<PnpmConfig>,
    // Unstructured fields kept for round trip capabilities
    #[serde(flatten)]
    pub other: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PnpmConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patched_dependencies: Option<BTreeMap<String, RelativeUnixPathBuf>>,
    // Unstructured config options kept for round trip capabilities
    #[serde(flatten)]
    pub other: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserializable)]
pub struct RawPackageJson {
    pub name: Option<UnescapedString>,
    pub version: Option<UnescapedString>,
    pub package_manager: Option<Spanned<UnescapedString>>,
    pub dependencies: Option<BTreeMap<String, UnescapedString>>,
    pub dev_dependencies: Option<BTreeMap<String, UnescapedString>>,
    pub optional_dependencies: Option<BTreeMap<String, UnescapedString>>,
    pub peer_dependencies: Option<BTreeMap<String, UnescapedString>>,
    pub scripts: BTreeMap<String, Spanned<UnescapedString>>,
    pub resolutions: Option<BTreeMap<String, UnescapedString>>,
    pub pnpm: Option<RawPnpmConfig>,
    // Unstructured fields kept for round trip capabilities
    #[deserializable(rest)]
    pub other: BTreeMap<Text, serde_json::Value>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Deserializable)]
pub struct RawPnpmConfig {
    pub patched_dependencies: Option<BTreeMap<String, RelativeUnixPathBuf>>,
    // Unstructured config options kept for round trip capabilities
    #[deserializable(rest)]
    pub other: BTreeMap<Text, serde_json::Value>,
}

#[derive(Debug, thiserror::Error, Diagnostic)]
pub enum Error {
    #[error("unable to read package.json: {0}")]
    Io(#[from] std::io::Error),
    #[error("unable to parse package.json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unable to parse package.json")]
    #[diagnostic(code(package_json_parse_error))]
    Parse(#[related] Vec<ParseDiagnostic>),
}

impl WithMetadata for RawPackageJson {
    fn add_text(&mut self, text: Arc<str>) {
        if let Some(ref mut package_manager) = self.package_manager {
            package_manager.add_text(text.clone());
        }
        self.scripts
            .iter_mut()
            .for_each(|(_, v)| v.add_text(text.clone()));
    }

    fn add_path(&mut self, path: Arc<str>) {
        if let Some(ref mut package_manager) = self.package_manager {
            package_manager.add_path(path.clone());
        }
        self.scripts
            .iter_mut()
            .for_each(|(_, v)| v.add_path(path.clone()));
    }
}

impl From<RawPackageJson> for PackageJson {
    fn from(raw: RawPackageJson) -> Self {
        Self {
            name: raw.name.map(|s| s.into()),
            version: raw.version.map(|s| s.into()),
            package_manager: raw.package_manager.map(|s| s.map(|s| s.into())),
            dependencies: raw
                .dependencies
                .map(|m| m.into_iter().map(|(k, v)| (k, v.into())).collect()),
            dev_dependencies: raw
                .dev_dependencies
                .map(|m| m.into_iter().map(|(k, v)| (k, v.into())).collect()),
            optional_dependencies: raw
                .optional_dependencies
                .map(|m| m.into_iter().map(|(k, v)| (k, v.into())).collect()),
            peer_dependencies: raw
                .peer_dependencies
                .map(|m| m.into_iter().map(|(k, v)| (k, v.into())).collect()),
            scripts: raw
                .scripts
                .into_iter()
                .map(|(k, v)| (k, v.map(|v| v.into())))
                .collect(),
            resolutions: raw
                .resolutions
                .map(|m| m.into_iter().map(|(k, v)| (k, v.into())).collect()),
            pnpm: raw.pnpm.map(|p| p.into()),
            other: raw
                .other
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        }
    }
}

impl From<RawPnpmConfig> for PnpmConfig {
    fn from(raw: RawPnpmConfig) -> Self {
        Self {
            patched_dependencies: raw.patched_dependencies,
            other: raw
                .other
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        }
    }
}

impl PackageJson {
    pub fn load(path: &AbsoluteSystemPath) -> Result<PackageJson, Error> {
        tracing::trace!("loading package.json from {}", path);
        let contents = path.read_to_string()?;
        Self::load_from_str(&contents, path.as_str())
    }

    pub fn load_from_str(contents: &str, path: &str) -> Result<PackageJson, Error> {
        let (result, errors): (Option<RawPackageJson>, _) =
            deserialize_from_json_str(contents, JsonParserOptions::default(), path).consume();
        if !errors.is_empty() {
            return Err(Error::Parse(
                errors
                    .into_iter()
                    .map(|d| {
                        d.with_file_source_code(contents)
                            .with_file_path(path)
                            .into()
                    })
                    .collect(),
            ));
        }

        // We expect a result if there are no errors
        let mut package_json = result.expect("no parse errors produced but no result");

        package_json.add_path(path.into());
        package_json.add_text(contents.into());

        Ok(package_json.into())
    }

    // Utility method for easy construction of package.json during testing
    pub fn from_value(value: serde_json::Value) -> Result<PackageJson, Error> {
        let contents = serde_json::to_string(&value)?;
        let package_json: PackageJson = Self::load_from_str(&contents, "package.json")?;
        Ok(package_json)
    }

    pub fn all_dependencies(&self) -> impl Iterator<Item = (&String, &String)> + '_ {
        self.dev_dependencies
            .iter()
            .flatten()
            .chain(self.optional_dependencies.iter().flatten())
            .chain(self.dependencies.iter().flatten())
    }

    /// Returns the command for script_name if it is non-empty
    pub fn command(&self, script_name: &str) -> Option<&str> {
        self.scripts
            .get(script_name)
            .filter(|command| !command.is_empty())
            .map(|command| command.as_str())
    }

    pub fn engines(&self) -> Option<HashMap<&str, &str>> {
        let engines = self.other.get("engines")?.as_object()?;
        Some(
            engines
                .iter()
                .filter_map(|(key, value)| {
                    let value = value.as_str()?;
                    Some((key.as_str(), value))
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod test {
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use test_case::test_case;

    use super::*;

    #[test_case(json!({"name": "foo", "random-field": true}) ; "additional fields kept during round trip")]
    #[test_case(json!({"name": "foo", "resolutions": {"foo": "1.0.0"}}) ; "berry resolutions")]
    #[test_case(json!({"name": "foo", "pnpm": {"patchedDependencies": {"some-pkg": "./patchfile"}, "another-field": 1}}) ; "pnpm")]
    #[test_case(json!({"name": "foo", "pnpm": {"another-field": 1}}) ; "pnpm without patches")]
    #[test_case(json!({"version": "1.2", "foo": "bar" }) ; "version")]
    #[test_case(json!({"packageManager": "npm@9", "foo": "bar"}) ; "package manager")]
    #[test_case(json!({"dependencies": { "turbo": "latest" }, "foo": "bar"}) ; "dependencies")]
    #[test_case(json!({"devDependencies": { "turbo": "latest" }, "foo": "bar"}) ; "dev dependencies")]
    #[test_case(json!({"optionalDependencies": { "turbo": "latest" }, "foo": "bar"}) ; "optional dependencies")]
    #[test_case(json!({"peerDependencies": { "turbo": "latest" }, "foo": "bar"}) ; "peer dependencies")]
    #[test_case(json!({"scripts": { "build": "turbo build" }, "foo": "bar"}) ; "scripts")]
    #[test_case(json!({"resolutions": { "turbo": "latest" }, "foo": "bar"}) ; "resolutions")]
    fn test_roundtrip(json: serde_json::Value) {
        let package_json: PackageJson = PackageJson::from_value(json.clone()).unwrap();
        let actual = serde_json::to_value(package_json).unwrap();
        assert_eq!(actual, json);
    }
}
