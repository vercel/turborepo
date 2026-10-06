//! Validated prelaunch tool resolutions, not dependency graphs or installation
//! state. Parsing an adapter ID does not grant execution support.
//! Adapter-specific exact release identities and semantic options are validated
//! again by that adapter.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

use crate::node_discovery::UniqueJson;

pub const SCHEMA_VERSION: u32 = 0;
pub const MAX_LOCK_BYTES: usize = 1024 * 1024;
const MAX_TOOLS: usize = 64;
const MAX_PARTS: usize = 64;
const MAX_DECLARATIONS: usize = 64;
const MAX_EXECUTABLES: usize = 64;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    #[error("turbo.lock exceeds the one MiB limit")]
    TooLarge,
    #[error("invalid turbo.lock JSON (duplicate keys and unknown fields are forbidden)")]
    Json,
    #[error(
        "unsupported turbo.lock schema version {0}; upgrade turbo or regenerate this prelaunch \
         lock"
    )]
    Schema(u32),
    #[error("invalid turbo.lock: {0}")]
    Invalid(&'static str),
}

/// Wire types are intentionally separate from the validated, read-only wrapper.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Document {
    pub schema_version: u32,
    pub tools: BTreeMap<String, Tool>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Tool {
    pub adapter: String,
    /// Exact upstream identity, not an authored range or floating channel.
    pub version: String,
    pub declarations: Vec<Declaration>,
    /// Adapter-owned semantic options only, never transport policy or
    /// credentials. Keys and values are bounded tokens; adapters define
    /// their meaning.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub options: BTreeMap<String, Vec<String>>,
    pub installation: Installation,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Declaration {
    pub file: String,
    /// Opaque adapter notation: Node uses dotted fields; managers use JSON
    /// pointers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Installation {
    /// A platform can require several pinned parts (e.g. Rust components).
    /// Assembly is in part-name order into each part's relative destination.
    /// Installers must reject resource collisions before promotion.
    Managed {
        artifacts: BTreeMap<Platform, BTreeMap<String, Artifact>>,
    },
    /// Explicit external-tool requirement. No fabricated artifact digest.
    VerifySystem { executables: Vec<String> },
}

/// This is artifact selection, not runtime libc detection or cache
/// compatibility. `any` is a platform-independent payload, never a
/// platform-independent task hash.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Platform {
    #[serde(rename = "any")]
    Any,
    #[serde(rename = "macos-x64")]
    MacosX64,
    #[serde(rename = "macos-arm64")]
    MacosArm64,
    #[serde(rename = "linux-x64-gnu")]
    LinuxX64Gnu,
    #[serde(rename = "linux-arm64-gnu")]
    LinuxArm64Gnu,
    #[serde(rename = "linux-x64-musl")]
    LinuxX64Musl,
    #[serde(rename = "linux-arm64-musl")]
    LinuxArm64Musl,
    #[serde(rename = "windows-x64")]
    WindowsX64,
    #[serde(rename = "windows-arm64")]
    WindowsArm64,
}
impl Platform {
    const NATIVE: [Self; 8] = [
        Self::MacosX64,
        Self::MacosArm64,
        Self::LinuxX64Gnu,
        Self::LinuxArm64Gnu,
        Self::LinuxX64Musl,
        Self::LinuxArm64Musl,
        Self::WindowsX64,
        Self::WindowsArm64,
    ];
    fn windows(self) -> bool {
        matches!(self, Self::WindowsX64 | Self::WindowsArm64)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Format {
    Binary,
    Tar,
    TarGz,
    Zip,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Artifact {
    /// Transport only, deliberately excluded from task-cache identity.
    pub url: String,
    pub sha256: String,
    pub format: Format,
    /// None means archive root. Binary payloads have no prefix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_prefix: Option<String>,
    /// None means installation root; otherwise a relative assembly directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination: Option<String>,
    /// Logical executable name -> path relative to this part's extracted root.
    /// A binary has exactly one payload path. Components may expose no
    /// binaries.
    pub executables: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lock(Document);
impl Lock {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_LOCK_BYTES {
            return Err(Error::TooLarge);
        }
        // The recursive duplicate-key visitor keeps serde_json's depth limit.
        let UniqueJson(value) = serde_json::from_slice(bytes).map_err(|_| Error::Json)?;
        let document = serde_json::from_value(value).map_err(|_| Error::Json)?;
        Self::new(document)
    }

    pub fn new(document: Document) -> Result<Self, Error> {
        if document.schema_version != SCHEMA_VERSION {
            return Err(Error::Schema(document.schema_version));
        }
        if document.tools.len() > MAX_TOOLS {
            return Err(Error::Invalid("too many tools"));
        }
        for (id, tool) in &document.tools {
            if !identifier(id) || !identifier(&tool.adapter) {
                return Err(Error::Invalid("invalid tool or adapter identifier"));
            }
            if tool.version.is_empty()
                || tool.version.len() > 128
                || !tool.version.bytes().all(|b| b.is_ascii_graphic())
            {
                return Err(Error::Invalid("expected exact release identity"));
            }
            // Current builtins share npm exact identity. Other adapters own their
            // normalization (Rust nightlies and Go prereleases are not npm ranges).
            if matches!(tool.adapter.as_str(), "node" | "npm" | "pnpm") {
                let version = crate::VersionRequest::parse(&tool.version)
                    .map_err(|_| Error::Invalid("expected canonical exact builtin version"))?;
                if version
                    .exact_version()
                    .is_none_or(|v| v.to_string() != tool.version)
                {
                    return Err(Error::Invalid("expected canonical exact builtin version"));
                }
            }
            declarations(&tool.declarations)?;
            if tool.options.len() > 32 {
                return Err(Error::Invalid("too many semantic options"));
            }
            for (key, values) in &tool.options {
                if !identifier(key)
                    || values.is_empty()
                    || values.len() > MAX_PARTS
                    || values.iter().any(|v| !token(v, 128))
                    || values.windows(2).any(|w| w[0] >= w[1])
                {
                    return Err(Error::Invalid(
                        "semantic options must be bounded sorted unique tokens",
                    ));
                }
            }
            if let Installation::VerifySystem { executables } = &tool.installation
                && (executables.is_empty()
                    || executables.len() > MAX_EXECUTABLES
                    || executables
                        .iter()
                        .any(|name| !portable_path(name) || name.contains('/')))
            {
                return Err(Error::Invalid("invalid verified-system executable names"));
            }
            if let Installation::Managed { artifacts } = &tool.installation {
                if artifacts.is_empty()
                    || (artifacts.contains_key(&Platform::Any) && artifacts.len() != 1)
                {
                    return Err(Error::Invalid(
                        "managed artifacts require native platforms or only any",
                    ));
                }
                for parts in artifacts.values() {
                    if parts.is_empty() || parts.len() > MAX_PARTS {
                        return Err(Error::Invalid("expected a bounded nonempty artifact set"));
                    }
                    let mut executable_count = 0;
                    for (part, artifact) in parts {
                        if !identifier(part) {
                            return Err(Error::Invalid("invalid artifact part name"));
                        }
                        validate_artifact(artifact)?;
                        executable_count += artifact.executables.len();
                    }
                    if executable_count == 0 || executable_count > MAX_EXECUTABLES {
                        return Err(Error::Invalid(
                            "installation must expose a bounded nonempty executable set",
                        ));
                    }
                }
            }
        }
        for platform in Platform::NATIVE {
            let mut names = BTreeSet::new();
            for tool in document.tools.values() {
                let executable_names: Vec<_> = match &tool.installation {
                    Installation::VerifySystem { executables } => executables.iter().collect(),
                    Installation::Managed { artifacts } => artifacts
                        .get(&platform)
                        .or_else(|| artifacts.get(&Platform::Any))
                        .into_iter()
                        .flat_map(|parts| parts.values())
                        .flat_map(|artifact| artifact.executables.keys())
                        .collect(),
                };
                for name in executable_names {
                    let folded = name.to_ascii_lowercase();
                    let alias = if platform.windows() {
                        [".exe", ".cmd", ".bat", ".com"]
                            .iter()
                            .find_map(|suffix| folded.strip_suffix(suffix))
                            .unwrap_or(&folded)
                    } else {
                        &folded
                    };
                    if !names.insert(alias.to_owned()) {
                        return Err(Error::Invalid("co-active executable names collide"));
                    }
                }
            }
        }
        // Constructors obey the same raw document bound as parsing.
        if serde_json::to_vec(&document)
            .map_err(|_| Error::Json)?
            .len()
            > MAX_LOCK_BYTES
        {
            return Err(Error::TooLarge);
        }
        Ok(Self(document))
    }

    pub fn document(&self) -> &Document {
        &self.0
    }
    pub fn tools(&self) -> &BTreeMap<String, Tool> {
        &self.0.tools
    }
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_".contains(&b))
}
fn token(value: &str, limit: usize) -> bool {
    !value.is_empty()
        && value.len() <= limit
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_+".contains(&b))
}
fn text(value: &str, limit: usize) -> bool {
    !value.is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}

fn declarations(values: &[Declaration]) -> Result<(), Error> {
    if values.is_empty() || values.len() > MAX_DECLARATIONS {
        return Err(Error::Invalid("expected bounded nonempty declarations"));
    }
    let mut sources = BTreeSet::new();
    for value in values {
        if !portable_path(&value.file)
            || value.field.as_ref().is_some_and(|v| !text(v, 1024))
            || value.request.as_ref().is_some_and(|v| !text(v, 4096))
            || !sources.insert((&value.file, &value.field))
        {
            return Err(Error::Invalid("invalid or repeated declaration location"));
        }
    }
    Ok(())
}
fn validate_artifact(value: &Artifact) -> Result<(), Error> {
    // Never echo source metadata in diagnostics. Query parameters can carry
    // secrets.
    let authority = value
        .url
        .strip_prefix("https://")
        .and_then(|rest| rest.split('/').next())
        .unwrap_or_default();
    if value.url.len() > 4096 || authority.is_empty() || authority.contains('@') {
        return Err(Error::Invalid("invalid or credential-bearing artifact URL"));
    }
    let url = Url::parse(&value.url).map_err(|_| Error::Invalid("invalid artifact URL"))?;
    if !value.url.starts_with("https://")
        || value.url.chars().any(|c| c.is_control() || c == '\\')
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::Invalid(
            "artifact URL must be credential-free HTTPS without query or fragment",
        ));
    }
    if value.sha256.len() != 64
        || !value
            .sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::Invalid(
            "artifact requires a lowercase SHA-256 digest",
        ));
    }
    if value
        .root_prefix
        .as_ref()
        .is_some_and(|v| !portable_path(v))
        || value
            .destination
            .as_ref()
            .is_some_and(|v| !portable_path(v))
    {
        return Err(Error::Invalid("unsafe artifact layout"));
    }
    if value.format == Format::Binary
        && (value.root_prefix.is_some() || value.executables.len() != 1)
    {
        return Err(Error::Invalid(
            "binary requires one payload path and no archive prefix",
        ));
    }
    if value.executables.len() > MAX_EXECUTABLES {
        return Err(Error::Invalid("too many executables"));
    }
    for (name, path) in &value.executables {
        if !portable_path(name) || name.contains('/') || !portable_path(path) {
            return Err(Error::Invalid("unsafe executable name or path"));
        }
        if value
            .destination
            .as_ref()
            .is_some_and(|dest| !portable_path(&format!("{dest}/{path}")))
        {
            return Err(Error::Invalid("assembled executable path exceeds limits"));
        }
    }
    Ok(())
}

// Conservative root-relative ASCII paths, portable across every supported OS.
fn portable_path(value: &str) -> bool {
    if value.is_empty() || value.len() > 1024 || value.split('/').count() > 32 {
        return false;
    }
    value.split('/').all(|part| {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.len() > 255
            || part.ends_with('.')
            || !part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-+@".contains(&b))
        {
            return false;
        }
        let base = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        !matches!(
            base.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) && !["COM", "LPT"].iter().any(|prefix| {
            base.strip_prefix(prefix)
                .is_some_and(|tail| tail.len() == 1 && matches!(tail.as_bytes()[0], b'1'..=b'9'))
        })
    })
}

#[cfg(test)]
mod tests;
