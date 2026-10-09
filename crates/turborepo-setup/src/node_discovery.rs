//! Root-only; injected metadata, no binaries/network/lock schema.
//! Precedence: runtime > .nvmrc > .node-version > engines.node; sources AND,
//! runtime nodes OR. First matching alternative gives provenance.
//! Only .nvmrc aliases: node (stable), lts/*, lts/name (stable LTS, ASCII
//! names). Runtime/engines.node/.node-version: strict npm exact/range, no
//! aliases. Exact identity includes build; ranges/engines use VersionRequest
//! semantics. Highest semver/build wins, order independent; no
//! default/comments/shell.

use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, Read},
    path::Path,
};

use semver::Version;
use serde::{
    Deserialize,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::Value;
use thiserror::Error;

use crate::version_request::{MAX_REQUEST_BYTES, VersionRequest};

pub const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
pub const MAX_RUNTIME_ENTRIES: usize = 64;
pub const MAX_RELEASES: usize = 16_384;

#[derive(Debug, Error)]
pub enum NodeDiscoveryError {
    #[error("cannot read {0}: {1}")]
    Read(&'static str, #[source] io::Error),
    #[error("{file} exceeds the {limit}-byte discovery limit")]
    TooLarge { file: &'static str, limit: usize },
    #[error("invalid {0}")]
    InvalidSource(String),
    #[error("invalid Node release metadata: {0}")]
    InvalidMetadata(String),
    #[error("no Node declarations in root sources")]
    NoSelection,
    #[error("conflicting or unavailable Node releases; all sources: {sources}")]
    NoMatchingRelease { sources: String },
}

/// Lock-ready provenance without depending on a lockfile schema. Files are
/// root-relative Unix names; fields include array indices. Requests trim ASCII
/// whitespace, canonicalize exact versions/LTS names, and retain range syntax
/// without simplifying authored constraints or prerelease branches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSource {
    pub file: &'static str,
    pub field: Option<String>,
    /// None only for an authored name-only runtime (not a fabricated wildcard).
    pub request: Option<String>,
}

impl NodeSource {
    fn location(&self) -> String {
        self.field
            .as_ref()
            .map_or_else(|| self.file.into(), |f| format!("{}#{f}", self.file))
    }
}

/// Injected, validated release metadata. Versions must be canonical exact
/// semver (no `v` prefix); LTS codenames are normalized to lowercase ASCII.
#[derive(Debug, Clone)]
pub struct NodeRelease {
    version: Version,
    lts: Option<String>,
}

impl NodeRelease {
    pub fn new(version: &str, lts: Option<&str>) -> Result<Self, NodeDiscoveryError> {
        let parsed = VersionRequest::parse(version).map_err(|e| metadata_error(e.to_string()))?;
        let version = parsed
            .exact_version()
            .filter(|v| v.to_string() == version)
            .cloned()
            .ok_or_else(|| metadata_error("expected canonical exact version"))?;
        let lts = lts
            .map(|name| normalize_name(name).ok_or_else(|| metadata_error("invalid LTS codename")))
            .transpose()?;
        Ok(Self { version, lts })
    }
}

#[derive(Debug, Clone)]
enum Request {
    Unconstrained,
    Version(VersionRequest),
    Node,
    Lts(Option<String>),
}

#[derive(Debug, Clone)]
struct Declaration {
    source: NodeSource,
    request: Request,
    exact_identity: bool,
}

impl Declaration {
    fn matches(&self, release: &NodeRelease) -> bool {
        let version = &release.version;
        match &self.request {
            Request::Unconstrained => true,
            Request::Version(r) => {
                r.matches_with_exact(version, |a, b| !self.exact_identity || a == b)
            }
            Request::Node => version.pre.is_empty(),
            Request::Lts(None) => version.pre.is_empty() && release.lts.is_some(),
            Request::Lts(Some(name)) => {
                version.pre.is_empty() && release.lts.as_ref() == Some(name)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct NodeRequirements {
    declarations: Vec<Declaration>,
    // Provenance only: policies must never become unconstrained OR branches.
    policy_sources: Vec<NodeSource>,
    runtime_end: usize, // Leading runtime alternatives are OR; the remainder AND.
}

#[derive(Debug, Clone)]
pub struct ResolvedNode {
    pub version: Version,
    pub selection_source: NodeSource,
    pub sources: Vec<NodeSource>,
}

impl NodeRequirements {
    /// Missing files are optional; invalid JSON, field shapes, UTF-8, requests,
    /// duplicate JSON keys, and read errors are fatal even with another source.
    /// Reads stop at limit + 1 bytes; request limits apply BEFORE trimming.
    pub fn read(root: &Path) -> Result<Self, NodeDiscoveryError> {
        Self::read_with(|file, limit| read_optional(root, file, limit))
    }

    /// Parse an injected root snapshot with the same limits and semantics as
    /// native discovery, without reading files. Missing sources are optional.
    pub fn from_sources(
        package_json: Option<&str>,
        nvmrc: Option<&str>,
        node_version: Option<&str>,
    ) -> Result<Self, NodeDiscoveryError> {
        Self::read_with(|file, limit| {
            let text = match file {
                "package.json" => package_json,
                ".nvmrc" => nvmrc,
                ".node-version" => node_version,
                _ => unreachable!("fixed native Node source"),
            };
            if text.is_some_and(|text| text.len() > limit) {
                return Err(NodeDiscoveryError::TooLarge { file, limit });
            }
            Ok(text.map(str::to_owned))
        })
    }

    pub(crate) fn read_with(
        mut read: impl FnMut(&'static str, usize) -> Result<Option<String>, NodeDiscoveryError>,
    ) -> Result<Self, NodeDiscoveryError> {
        let mut result = Self {
            declarations: Vec::new(),
            policy_sources: Vec::new(),
            runtime_end: 0,
        };
        let manifest = read("package.json", MAX_MANIFEST_BYTES)?
            .map(|text| {
                serde_json::from_str::<UniqueJson>(&text)
                    .map(|v| v.0)
                    .map_err(|e| invalid("package.json", e.to_string()))
            })
            .transpose()?;
        if let Some(manifest) = &manifest {
            manifest
                .as_object()
                .ok_or_else(|| invalid("package.json", "expected object"))?;
            if let Some(dev) = manifest.get("devEngines") {
                let dev = dev
                    .as_object()
                    .ok_or_else(|| invalid("package.json#devEngines", "expected object"))?;
                if let Some(runtime) = dev.get("runtime") {
                    let field = "devEngines.runtime";
                    if let Some(entries) = runtime.as_array() {
                        if entries.len() > MAX_RUNTIME_ENTRIES {
                            return Err(invalid(
                                "package.json#devEngines.runtime",
                                "too many runtime entries",
                            ));
                        }
                        for (index, entry) in entries.iter().enumerate() {
                            result.runtime(entry, &format!("{field}[{index}]"))?;
                        }
                    } else {
                        result.runtime(runtime, field)?;
                    }
                }
            }
        }
        result.runtime_end = result.declarations.len();
        for file in [".nvmrc", ".node-version"] {
            if let Some(request) = read(file, MAX_REQUEST_BYTES)? {
                result.push(file, None, Some(&request), true)?;
            }
        }
        if let Some(engines) = manifest.as_ref().and_then(|m| m.get("engines")) {
            let engines = engines
                .as_object()
                .ok_or_else(|| invalid("package.json#engines", "expected object"))?;
            if let Some(node) = engines.get("node") {
                let request = node
                    .as_str()
                    .ok_or_else(|| invalid("package.json#engines.node", "expected string"))?;
                result.push(
                    "package.json",
                    Some("engines.node".into()),
                    Some(request),
                    false,
                )?;
            }
        }
        Ok(result)
    }

    pub fn sources(&self) -> impl Iterator<Item = &NodeSource> {
        self.declarations
            .iter()
            .map(|d| &d.source)
            .chain(&self.policy_sources)
    }

    /// Check a committed pin without refreshing release metadata. LTS channel
    /// identity remains the resolver's responsibility; reject prereleases for
    /// those channels and enforce all statically checkable native constraints.
    pub fn matches_locked_version(&self, version: &Version) -> bool {
        let release = NodeRelease {
            version: version.clone(),
            lts: None,
        };
        let matches = |d: &Declaration| match &d.request {
            Request::Lts(_) => version.pre.is_empty(),
            _ => d.matches(&release),
        };
        let (alternatives, constraints) = self.declarations.split_at(self.runtime_end);
        crate::version_request::is_valid_release(version)
            && !self.declarations.is_empty()
            && (alternatives.is_empty() || alternatives.iter().any(matches))
            && constraints.iter().all(matches)
    }

    /// Injected metadata only. Validate all identities, even ineligible
    /// releases. Conflicts/unavailable releases never return a partial
    /// result.
    pub fn resolve(&self, releases: &[NodeRelease]) -> Result<ResolvedNode, NodeDiscoveryError> {
        if releases.len() > MAX_RELEASES {
            return Err(metadata_error("too many releases"));
        }
        let mut identities = BTreeMap::new();
        for release in releases {
            if let Some(old) = identities.insert(&release.version, &release.lts)
                && old != &release.lts
            {
                return Err(metadata_error(format!(
                    "contradictory LTS metadata for {}: {old:?} vs {:?}",
                    release.version, release.lts
                )));
            }
        }
        self.declarations
            .first()
            .ok_or(NodeDiscoveryError::NoSelection)?;
        let sources = || {
            self.sources()
                .map(|s| format!("{} = {:?}", s.location(), s.request))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let (alternatives, constraints) = self.declarations.split_at(self.runtime_end);
        let release = releases
            .iter()
            .filter(|r| {
                (alternatives.is_empty() || alternatives.iter().any(|d| d.matches(r)))
                    && constraints.iter().all(|d| d.matches(r))
            })
            .max_by(|a, b| a.version.cmp(&b.version))
            .ok_or_else(|| NodeDiscoveryError::NoMatchingRelease { sources: sources() })?;
        let selection = self
            .declarations
            .iter()
            .find(|d| d.matches(release))
            .ok_or(NodeDiscoveryError::NoSelection)?;
        Ok(ResolvedNode {
            version: release.version.clone(),
            selection_source: selection.source.clone(),
            sources: self.sources().cloned().collect(),
        })
    }

    fn runtime(&mut self, entry: &Value, field: &str) -> Result<(), NodeDiscoveryError> {
        let location = format!("package.json#{field}");
        let object = entry
            .as_object()
            .ok_or_else(|| invalid(&location, "expected runtime object"))?;
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .and_then(normalize_name)
            .ok_or_else(|| invalid(&location, "expected runtime name"))?;
        if name == "node" {
            if let Some(policy) = object.get("onFail") {
                let location = format!("{location}.onFail");
                match policy.as_str() {
                    Some("error") => self.policy_sources.push(NodeSource {
                        file: "package.json",
                        field: Some(format!("{field}.onFail")),
                        request: Some("error".into()),
                    }),
                    Some("warn" | "ignore") => {
                        return Err(invalid(
                            &location,
                            "advisory Node runtime onFail policies are unsupported by setup",
                        ));
                    }
                    _ => return Err(invalid(&location, "expected error, warn, or ignore")),
                }
            }
            let request = object
                .get("version")
                .map(|v| {
                    v.as_str()
                        .ok_or_else(|| invalid(&location, "expected Node version string"))
                })
                .transpose()?;
            let field = request.map_or_else(|| field.into(), |_| format!("{field}.version"));
            self.push("package.json", Some(field), request, true)?;
        }
        Ok(())
    }

    fn push(
        &mut self,
        file: &'static str,
        field: Option<String>,
        input: Option<&str>,
        exact_identity: bool,
    ) -> Result<(), NodeDiscoveryError> {
        let mut source = NodeSource {
            file,
            field,
            request: None,
        };
        let request = if let Some(input) = input {
            let location = source.location();
            if input.len() > MAX_REQUEST_BYTES || !input.is_ascii() {
                return Err(invalid(
                    &location,
                    "request exceeds byte limit or contains non-ASCII syntax",
                ));
            }
            let input = input.trim();
            source.request = Some(input.into());
            if file == ".nvmrc" && input == "node" {
                Request::Node
            } else if file == ".nvmrc" && input.starts_with("lts/") {
                let name = &input[4..];
                let name = if name == "*" {
                    None
                } else {
                    Some(
                        normalize_name(name)
                            .ok_or_else(|| invalid(&location, "unsupported LTS alias"))?,
                    )
                };
                source.request = Some(format!("lts/{}", name.as_deref().unwrap_or("*")));
                Request::Lts(name)
            } else {
                let parsed =
                    VersionRequest::parse(input).map_err(|e| invalid(&location, e.to_string()))?;
                if let Some(version) = parsed.exact_version() {
                    source.request = Some(version.to_string());
                }
                Request::Version(parsed)
            }
        } else {
            Request::Unconstrained
        };
        self.declarations.push(Declaration {
            source,
            request,
            exact_identity,
        });
        Ok(())
    }
}

fn normalize_name(name: &str) -> Option<String> {
    (name.len() <= 64
        && name.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
    .then(|| name.to_ascii_lowercase())
}

fn metadata_error(reason: impl Into<String>) -> NodeDiscoveryError {
    NodeDiscoveryError::InvalidMetadata(reason.into())
}

fn invalid(location: &str, reason: impl Into<String>) -> NodeDiscoveryError {
    NodeDiscoveryError::InvalidSource(format!("{location}: {}", reason.into()))
}

fn read_optional(
    root: &Path,
    file: &'static str,
    limit: usize,
) -> Result<Option<String>, NodeDiscoveryError> {
    let handle = match File::open(root.join(file)) {
        Ok(handle) => handle,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(NodeDiscoveryError::Read(file, source)),
    };
    let mut text = String::new();
    handle
        .take(limit as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|source| NodeDiscoveryError::Read(file, source))?;
    if text.len() > limit {
        return Err(NodeDiscoveryError::TooLarge { file, limit });
    }
    Ok(Some(text))
}

// serde_json::Value silently overwrites duplicate keys. Reject them recursively
// so duplicate runtime/version/engines fields cannot erase authored
// constraints. serde_json's default recursion limit remains enabled.
pub(crate) struct UniqueJson(pub(crate) Value);
impl<'de> Deserialize<'de> for UniqueJson {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct JsonVisitor;
        macro_rules! scalar {
            ($method:ident, $ty:ty) => {
                fn $method<E: de::Error>(self, v: $ty) -> Result<Self::Value, E> {
                    Ok(UniqueJson(v.into()))
                }
            };
        }
        impl<'de> Visitor<'de> for JsonVisitor {
            type Value = UniqueJson;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("JSON without duplicate keys")
            }
            scalar!(visit_bool, bool);
            scalar!(visit_i64, i64);
            scalar!(visit_u64, u64);
            scalar!(visit_f64, f64);
            scalar!(visit_str, &str);
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(UniqueJson(value)) = seq.next_element()? {
                    values.push(value);
                }
                Ok(UniqueJson(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some((key, UniqueJson(value))) = map.next_entry::<String, UniqueJson>()? {
                    if values.contains_key(&key) {
                        return Err(de::Error::custom(format!("duplicate key {key:?}")));
                    }
                    values.insert(key, value);
                }
                Ok(UniqueJson(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(JsonVisitor)
    }
}

#[cfg(test)]
mod tests;
