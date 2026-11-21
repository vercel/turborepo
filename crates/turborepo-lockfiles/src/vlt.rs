//! vlt lockfile (`vlt-lock.json`) support.
//!
//! The lockfile stores the install graph directly:
//!
//! - `nodes` maps a dependency id to `[flags, name, integrity?, resolved?,
//!   ...]`. Only external packages are listed, the root and the workspaces are
//!   not.
//! - `edges` maps `"<from id> <dependency name>"` to `"<type> <spec> <to id>"`,
//!   where the target is `MISSING` for an optional dependency that was not
//!   installed.
//!
//! Dependency ids already tell packages that resolved differently apart
//! (version, registry, peer context), so an edge names exactly one package
//! and resolution never has to guess from a name and a range.

use std::{
    any::Any,
    borrow::Cow,
    collections::{BTreeMap, HashMap, HashSet},
};

use serde::{
    Deserialize, Serialize,
    de::{Deserializer, MapAccess, SeqAccess, Visitor},
    ser::{SerializeMap, SerializeSeq, Serializer},
};
use thiserror::Error;

use super::{Lockfile, Package, PackageSource};

/// The only `lockfileVersion` vlt reads or writes.
const LOCKFILE_VERSION: u64 = 1;
/// Dependency id of the project root.
const ROOT_IMPORTER: &str = "file~_d";
const WORKSPACE_PREFIX: &str = "workspace~";
const NODES_HEADER: &str = "  \"nodes\": {";
/// Bits of a node's flags, its first field.
const FLAG_OPTIONAL: u64 = 1;
const FLAG_DEV: u64 = 2;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Unsupported vlt lockfile version: {0}")]
    UnsupportedVersion(String),
    #[error("Invalid vlt lockfile: {0}")]
    Invalid(String),
}

/// A JSON value that keeps object keys in document order, so entries this
/// module does not interpret are written back the way vlt wrote them.
#[derive(Debug, Clone, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(s) => Some(s),
            _ => None,
        }
    }

    fn into_object(self) -> Option<Vec<(String, Json)>> {
        match self {
            Json::Object(entries) => Some(entries),
            _ => None,
        }
    }
}

impl<'de> Deserialize<'de> for Json {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct JsonVisitor;

        impl<'de> Visitor<'de> for JsonVisitor {
            type Value = Json;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a JSON value")
            }

            fn visit_unit<E>(self) -> Result<Json, E> {
                Ok(Json::Null)
            }

            fn visit_bool<E>(self, value: bool) -> Result<Json, E> {
                Ok(Json::Bool(value))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Json, E> {
                Ok(Json::Number(value.into()))
            }

            fn visit_u64<E>(self, value: u64) -> Result<Json, E> {
                Ok(Json::Number(value.into()))
            }

            fn visit_f64<E>(self, value: f64) -> Result<Json, E> {
                Ok(serde_json::Number::from_f64(value).map_or(Json::Null, Json::Number))
            }

            fn visit_str<E>(self, value: &str) -> Result<Json, E> {
                Ok(Json::String(value.to_owned()))
            }

            fn visit_string<E>(self, value: String) -> Result<Json, E> {
                Ok(Json::String(value))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Json, A::Error> {
                let mut items = Vec::with_capacity(seq.size_hint().unwrap_or(0));
                while let Some(item) = seq.next_element()? {
                    items.push(item);
                }
                Ok(Json::Array(items))
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Json, A::Error> {
                let mut entries = Vec::with_capacity(map.size_hint().unwrap_or(0));
                while let Some(entry) = map.next_entry()? {
                    entries.push(entry);
                }
                Ok(Json::Object(entries))
            }
        }

        deserializer.deserialize_any(JsonVisitor)
    }
}

impl Serialize for Json {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Json::Null => serializer.serialize_unit(),
            Json::Bool(value) => serializer.serialize_bool(*value),
            Json::Number(value) => value.serialize(serializer),
            Json::String(value) => serializer.serialize_str(value),
            Json::Array(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Json::Object(entries) => {
                let mut map = serializer.serialize_map(Some(entries.len()))?;
                for (key, value) in entries {
                    map.serialize_entry(key, value)?;
                }
                map.end()
            }
        }
    }
}

/// One `"<from id> <name>": "<type> <spec> <to id>"` entry. Ids and names
/// hold no spaces while a spec may (`>=1 <2`), so the key splits on its first
/// space and the target follows the last space of the value.
#[derive(Debug, Clone)]
struct Edge {
    key: String,
    value: String,
    from_len: usize,
    to_start: usize,
}

impl Edge {
    fn new(key: String, value: String) -> Result<Self, Error> {
        let invalid = || Error::Invalid(format!("unexpected edge '{key}': '{value}'"));
        let from_len = key.find(' ').ok_or_else(invalid)?;
        let to_start = value.rfind(' ').ok_or_else(invalid)? + 1;
        Ok(Self {
            key,
            value,
            from_len,
            to_start,
        })
    }

    fn from(&self) -> &str {
        &self.key[..self.from_len]
    }

    fn to(&self) -> &str {
        &self.value[self.to_start..]
    }

    /// The dependency type: `prod`, `dev`, `optional`, `peer` or
    /// `peerOptional`.
    fn kind(&self) -> &str {
        self.value.split(' ').next().unwrap_or_default()
    }
}

#[derive(Debug)]
pub struct VltLockfile {
    lockfile_version: Json,
    options: Json,
    // `nodes` and `edges` stay in document order. vlt sorts them with a
    // locale aware comparison, and keeping the order means a subgraph is
    // written the way vlt would write it without redoing that sort.
    nodes: Vec<(String, Json)>,
    edges: Vec<Edge>,
    node_index: HashMap<String, usize>,
    edge_index: HashMap<String, usize>,
    edges_from: HashMap<String, Vec<usize>>,
}

impl VltLockfile {
    pub fn load(content: &[u8]) -> Result<Self, crate::Error> {
        let mut lockfile_version = Json::Null;
        let mut options = Json::Object(Vec::new());
        let mut nodes = Vec::new();
        let mut edges = Vec::new();

        let document = serde_json::from_slice::<Json>(content)?
            .into_object()
            .ok_or_else(|| Error::Invalid("expected an object".into()))?;
        for (key, value) in document {
            match key.as_str() {
                "lockfileVersion" => lockfile_version = value,
                "options" => options = value,
                "nodes" => {
                    nodes = value
                        .into_object()
                        .ok_or_else(|| Error::Invalid("`nodes` must be an object".into()))?;
                }
                "edges" => {
                    let entries = value
                        .into_object()
                        .ok_or_else(|| Error::Invalid("`edges` must be an object".into()))?;
                    edges = entries
                        .into_iter()
                        .map(|(key, value)| match value {
                            Json::String(value) => Edge::new(key, value),
                            _ => Err(Error::Invalid(format!("edge '{key}' must be a string"))),
                        })
                        .collect::<Result<_, _>>()?;
                }
                _ => {}
            }
        }

        // vlt only reads the version it writes, anything else is a format this
        // module knows nothing about.
        if lockfile_version != Json::Number(LOCKFILE_VERSION.into()) {
            let found = serde_json::to_string(&lockfile_version)?;
            return Err(Error::UnsupportedVersion(found).into());
        }

        Ok(Self::new(lockfile_version, options, nodes, edges))
    }

    fn new(
        lockfile_version: Json,
        options: Json,
        nodes: Vec<(String, Json)>,
        edges: Vec<Edge>,
    ) -> Self {
        let node_index = nodes
            .iter()
            .enumerate()
            .map(|(index, (id, _))| (id.clone(), index))
            .collect();
        let mut edge_index = HashMap::with_capacity(edges.len());
        let mut edges_from: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, edge) in edges.iter().enumerate() {
            edge_index.insert(edge.key.clone(), index);
            edges_from
                .entry(edge.from().to_owned())
                .or_default()
                .push(index);
        }
        Self {
            lockfile_version,
            options,
            nodes,
            edges,
            node_index,
            edge_index,
            edges_from,
        }
    }

    /// The package behind a dependency id, `None` for anything that is not an
    /// external package: `MISSING`, the root or a workspace.
    fn package(&self, id: &str) -> Option<Package> {
        self.node_index.contains_key(id).then(|| Package {
            key: id.to_owned(),
            version: id_version(id),
        })
    }

    fn node_name(&self, id: &str) -> Option<&str> {
        let (_, node) = self.nodes.get(*self.node_index.get(id)?)?;
        match node {
            Json::Array(fields) => fields.get(1)?.as_str(),
            _ => None,
        }
    }

    /// Indices of the nodes reachable from the importers without crossing an
    /// edge of one of these types.
    fn reachable_without(&self, importers: &HashSet<String>, kinds: &[&str]) -> HashSet<usize> {
        let mut reached = HashSet::new();
        let mut queue: Vec<&str> = importers.iter().map(String::as_str).collect();
        while let Some(id) = queue.pop() {
            for index in self.edges_from.get(id).into_iter().flatten() {
                let edge = &self.edges[*index];
                if kinds.contains(&edge.kind()) {
                    continue;
                }
                if let Some(node) = self.node_index.get(edge.to())
                    && reached.insert(*node)
                {
                    queue.push(edge.to());
                }
            }
        }
        reached
    }

    /// Sets the `dev` and `optional` flags of every node the way vlt does: a
    /// node is dev (or optional) only when each path from an importer crosses
    /// a dev (or optional) edge. Removing a workspace can leave a package with
    /// nothing but such paths.
    fn recompute_flags(&mut self, importers: &HashSet<String>) {
        let non_dev = self.reachable_without(importers, &["dev"]);
        let non_optional = self.reachable_without(importers, &["optional", "peerOptional"]);
        for (index, (_, node)) in self.nodes.iter_mut().enumerate() {
            let Json::Array(fields) = node else { continue };
            let Some(Json::Number(flags)) = fields.first_mut() else {
                continue;
            };
            let Some(current) = flags.as_u64() else {
                continue;
            };
            let mut updated = current & !(FLAG_DEV | FLAG_OPTIONAL);
            if !non_dev.contains(&index) {
                updated |= FLAG_DEV;
            }
            if !non_optional.contains(&index) {
                updated |= FLAG_OPTIONAL;
            }
            *flags = updated.into();
        }
    }

    /// `options` without the catalogs. A catalog edit only moves the edges
    /// that reference the entry, which the changed packages already show.
    fn resolution_options(&self) -> Vec<&(String, Json)> {
        match &self.options {
            Json::Object(entries) => entries
                .iter()
                .filter(|(key, _)| key != "catalog" && key != "catalogs")
                .collect(),
            _ => Vec::new(),
        }
    }
}

impl Lockfile for VltLockfile {
    #[tracing::instrument(skip(self, _version))]
    fn resolve_package(
        &self,
        workspace_path: &str,
        name: &str,
        _version: &str,
    ) -> Result<Option<Package>, crate::Error> {
        // `all_dependencies` hands out dependency ids, which already name the
        // package no matter which workspace asks.
        if let Some(package) = self.package(name) {
            return Ok(Some(package));
        }
        // Otherwise this is a dependency declared by a workspace. A workspace
        // without dependencies has no edges, so an unknown one is not an
        // error.
        let key = format!("{} {name}", importer_id(workspace_path));
        Ok(self
            .edge_index
            .get(&key)
            .and_then(|index| self.package(self.edges[*index].to())))
    }

    #[tracing::instrument(skip(self))]
    fn all_dependencies(
        &self,
        key: &str,
    ) -> Result<Option<Cow<'_, BTreeMap<String, String>>>, crate::Error> {
        if !self.node_index.contains_key(key) {
            return Ok(None);
        }
        let dependencies = self
            .edges_from
            .get(key)
            .into_iter()
            .flatten()
            .filter_map(|index| self.package(self.edges[*index].to()))
            .map(|package| (package.key, package.version))
            .collect();
        Ok(Some(Cow::Owned(dependencies)))
    }

    fn transitive_edge_resolver(&self) -> Option<Box<dyn crate::TransitiveEdgeResolver + '_>> {
        Some(Box::new(VltEdgeResolver { lockfile: self }))
    }

    fn subgraph(
        &self,
        workspace_packages: &[String],
        packages: &[String],
    ) -> Result<Box<dyn Lockfile>, crate::Error> {
        for package in packages {
            if !self.node_index.contains_key(package) {
                return Err(crate::Error::MissingPackage(package.clone()));
            }
        }
        let mut importers: HashSet<String> = workspace_packages
            .iter()
            .map(|path| importer_id(path))
            .collect();
        importers.insert(ROOT_IMPORTER.to_owned());
        let mut retained = importers.clone();
        retained.extend(packages.iter().cloned());

        let nodes = self
            .nodes
            .iter()
            .filter(|(id, _)| retained.contains(id))
            .cloned()
            .collect();
        // An edge to a workspace that is left behind goes with it: its
        // declaration is dropped from the pruned package.json as well.
        let edges = self
            .edges
            .iter()
            .filter(|edge| {
                retained.contains(edge.from())
                    && (!edge.to().starts_with(WORKSPACE_PREFIX) || retained.contains(edge.to()))
            })
            .cloned()
            .collect();

        let mut pruned = Self::new(
            self.lockfile_version.clone(),
            self.options.clone(),
            nodes,
            edges,
        );
        pruned.recompute_flags(&importers);
        Ok(Box::new(pruned))
    }

    fn encode(&self) -> Result<Vec<u8>, crate::Error> {
        let document = Json::Object(vec![
            ("lockfileVersion".to_owned(), self.lockfile_version.clone()),
            ("options".to_owned(), self.options.clone()),
            ("nodes".to_owned(), Json::Object(self.nodes.clone())),
            (
                "edges".to_owned(),
                Json::Object(
                    self.edges
                        .iter()
                        .map(|edge| (edge.key.clone(), Json::String(edge.value.clone())))
                        .collect(),
                ),
            ),
        ]);
        Ok(single_line_nodes(serde_json::to_string_pretty(&document)?).into_bytes())
    }

    fn global_change(&self, other: &dyn Lockfile) -> bool {
        let any_other = other as &dyn Any;
        if let Some(other) = any_other.downcast_ref::<Self>() {
            self.lockfile_version != other.lockfile_version
                || self.resolution_options() != other.resolution_options()
        } else {
            true
        }
    }

    fn turbo_version(&self) -> Option<String> {
        let edge = self.edge_index.get(&format!("{ROOT_IMPORTER} turbo"))?;
        let version = self.package(self.edges[*edge].to())?.version;
        semver::Version::parse(&version).ok()?;
        Some(version)
    }

    fn human_name(&self, package: &Package) -> Option<String> {
        let name = self.node_name(&package.key)?;
        Some(format!("{name}@{}", package.version))
    }

    fn package_source(&self, package: &Package) -> PackageSource {
        match package.key.split('~').next() {
            Some("git") => PackageSource::Git,
            Some("file") => PackageSource::File,
            Some("workspace") => PackageSource::Workspace,
            _ => PackageSource::Registry,
        }
    }

    fn format_version(&self) -> Option<String> {
        serde_json::to_string(&self.lockfile_version).ok()
    }
}

/// Proves per-edge workspace independence for the shared closure DP.
///
/// Transitive edges come from `all_dependencies`, which only emits dependency
/// ids found in `nodes`. `resolve_package` returns such an id as is, without
/// looking at the workspace, so every workspace resolves the edge the same
/// way. Anything else is a declared dependency name and depends on who asks.
struct VltEdgeResolver<'a> {
    lockfile: &'a VltLockfile,
}

impl crate::TransitiveEdgeResolver for VltEdgeResolver<'_> {
    fn resolve_edge(
        &self,
        name: &str,
        _version: &str,
    ) -> Result<crate::TransitiveEdgeResolution, crate::Error> {
        Ok(match self.lockfile.package(name) {
            Some(package) => crate::TransitiveEdgeResolution::Global(Some(package)),
            None => crate::TransitiveEdgeResolution::WorkspaceSensitive,
        })
    }
}

/// Dependency id of the root (`""` or `.`) or of the workspace at this path.
fn importer_id(workspace_path: &str) -> String {
    match workspace_path {
        "" | "." => ROOT_IMPORTER.to_owned(),
        path => format!("{WORKSPACE_PREFIX}{}", encode_id_part(path)),
    }
}

/// Escapes one part of a dependency id the way vlt does: `/` becomes `+` and
/// characters that are unsafe in a path or reserved by the id syntax become
/// `_` followed by a letter.
fn encode_id_part(part: &str) -> String {
    let mut out = String::with_capacity(part.len());
    for ch in part.chars() {
        match ch {
            '/' => out.push('+'),
            '_' => out.push_str("__"),
            '+' => out.push_str("_p"),
            '\\' => out.push_str("_b"),
            ':' => out.push_str("_c"),
            '~' => out.push_str("_t"),
            '<' => out.push_str("_l"),
            '>' => out.push_str("_g"),
            '"' => out.push_str("_q"),
            '|' => out.push_str("_i"),
            '?' => out.push_str("_m"),
            '*' => out.push_str("_a"),
            ' ' => out.push_str("_s"),
            ch if (ch as u32) < 0x20 => out.push_str(&format!("_{:02X}", ch as u32)),
            ch => out.push(ch),
        }
    }
    if out.ends_with('.') {
        out.pop();
        out.push_str("_d");
    }
    out
}

/// Reverses [`encode_id_part`].
fn decode_id_part(part: &str) -> String {
    let mut out = String::with_capacity(part.len());
    let mut chars = part.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '+' => out.push('/'),
            '_' => {
                let Some(escape) = chars.next() else {
                    out.push('_');
                    break;
                };
                let literal = match escape {
                    '_' => Some('_'),
                    'p' => Some('+'),
                    'b' => Some('\\'),
                    'c' => Some(':'),
                    't' => Some('~'),
                    'l' => Some('<'),
                    'g' => Some('>'),
                    'q' => Some('"'),
                    'i' => Some('|'),
                    'm' => Some('?'),
                    'a' => Some('*'),
                    'd' => Some('.'),
                    's' => Some(' '),
                    _ => None,
                };
                if let Some(literal) = literal {
                    out.push(literal);
                    continue;
                }
                // `_XX`: a control character written as two hex digits.
                let control = chars
                    .peek()
                    .and_then(|low| u32::from_str_radix(&format!("{escape}{low}"), 16).ok())
                    .and_then(char::from_u32);
                match control {
                    Some(control) => {
                        chars.next();
                        out.push(control);
                    }
                    None => {
                        out.push('_');
                        out.push(escape);
                    }
                }
            }
            ch => out.push(ch),
        }
    }
    out
}

/// The version a dependency id points at: the semver version of a registry
/// package, otherwise what identifies the package for its type (git
/// selector, file path or tarball url).
fn id_version(id: &str) -> String {
    let mut parts = id.split('~');
    match parts.next() {
        // `~<registry>~<name>@<version>`, with an optional peer context after.
        Some("") => {
            let specifier = decode_id_part(parts.nth(1).unwrap_or_default());
            match specifier.rsplit_once('@') {
                Some((_, version)) => version.to_owned(),
                None => specifier,
            }
        }
        // `git~<remote>~<selector>`
        Some("git") => decode_id_part(parts.nth(1).unwrap_or_default()),
        // `file~<path>`, `remote~<url>`, `workspace~<path>`
        _ => decode_id_part(parts.next().unwrap_or_default()),
    }
}

/// vlt pretty prints the lockfile, then puts every node on a single line. The
/// second step is a plain text replacement over everything after the `nodes`
/// key, redone here as is so the output is byte for byte what vlt writes.
fn single_line_nodes(json: String) -> String {
    let json = json + "\n";
    let mut parts = json.split(NODES_HEADER);
    let mut out = parts.next().unwrap_or_default().to_owned();
    for part in parts {
        out.push_str(NODES_HEADER);
        out.push_str(&part.replace("\n      ", "").replace("\n    ]", "]"));
    }
    out
}

#[cfg(test)]
mod test {
    use pretty_assertions::assert_eq;
    use test_case::test_case;

    use super::*;

    // Written by `vlt install` for a project with the workspaces `apps/web`,
    // `packages/a` and `packages/b`.
    const LOCKFILE: &str = r#"{
  "lockfileVersion": 1,
  "options": {
    "registries": {
      "npm": "https://registry.npmjs.org/"
    }
  },
  "nodes": {
    "~npm~@sindresorhus+is@7.2.0": [2,"@sindresorhus/is","sha512-P1Cz1dWaFfR4IR+U13mqqiGsLFf1KbayybWwdd2vfctdV6hDpUkgCY0nKOLLTMSoRd/jJNjtbqzf13K8DCCXQw==","https://registry.npmjs.org/@sindresorhus/is/-/is-7.2.0.tgz"],
    "~npm~is-buffer@1.1.6": [0,"is-buffer","sha512-NcdALwpXkTm5Zvvbk7owOUSvVvBKDgKP5/ewfXEznmQFfs4ZRmanOeKBTjRVjka3QFoN6XJ+9F3USqfHqTaU5w==","https://registry.npmjs.org/is-buffer/-/is-buffer-1.1.6.tgz"],
    "~npm~is-even@1.0.0": [0,"is-even","sha512-LEhnkAdJqic4Dbqn58A0y52IXoHWlsueqQkKfMfdEnIYG8A1sm/GHidKkS6yvXlMoRrkM34csHnXQtOqcb+Jzg==","https://registry.npmjs.org/is-even/-/is-even-1.0.0.tgz"],
    "~npm~is-number@3.0.0": [0,"is-number","sha512-4cboCqIpliH+mAvFNegjZQ4kgKc3ZUhQVr3HvWbSh5q3WH2v82ct+T2Y1hdU5Gdtorx/cLifQjqCbL7bpznLTg==","https://registry.npmjs.org/is-number/-/is-number-3.0.0.tgz"],
    "~npm~is-number@6.0.0": [0,"is-number","sha512-Wu1VHeILBK8KAWJUAiSZQX94GmOE45Rg6/538fKwiloUu21KncEkYGPqob2oSZ5mUT73vLGrHQjKw3KMPwfDzg==","https://registry.npmjs.org/is-number/-/is-number-6.0.0.tgz"],
    "~npm~is-odd@0.1.2": [0,"is-odd","sha512-Ri7C2K7o5IrUU9UEI8losXJCCD/UtsaIrkR5sxIcFg4xQ9cRJXlWA5DQvTE0yDc0krvSNLsRGXN11UPS6KyfBw==","https://registry.npmjs.org/is-odd/-/is-odd-0.1.2.tgz"],
    "~npm~is-odd@3.0.1": [0,"is-odd","sha512-CQpnWPrDwmP1+SMHXZhtLtJv90yiyVfluGsX5iNCVkrhQtU3TQHsUWPG9wkdk9Lgd5yNpAg9jQEo90CBaXgWMA==","https://registry.npmjs.org/is-odd/-/is-odd-3.0.1.tgz"],
    "~npm~kind-of@3.2.2": [0,"kind-of","sha512-NOW9QQXMoZGg/oqnVNoNTTIFEIid1627WCffUBJEdMxYApq7mNE7CpzucIPc+ZQg25Phej7IJSmX3hO+oblOtQ==","https://registry.npmjs.org/kind-of/-/kind-of-3.2.2.tgz"],
    "~npm~left-pad@1.3.0": [0,"left-pad","sha512-XI5MPzVNApjAyhQzphX8BkmKsKUxD4LdyK24iZeQGinBN9yTQT3bFlCBy/aVx2HrNcqQGsdot8ghrjyrvMCoEA==","https://registry.npmjs.org/left-pad/-/left-pad-1.3.0.tgz"],
    "~npm~turbo-darwin-64@2.5.4": [3,"turbo-darwin-64","sha512-ah6YnH2dErojhFooxEzmvsoZQTMImaruZhFPfMKPBq8sb+hALRdvBNLqfc8NWlZq576FkfRZ/MSi4SHvVFT9PQ==","https://registry.npmjs.org/turbo-darwin-64/-/turbo-darwin-64-2.5.4.tgz",null,null,null,{  "os": [    "darwin"  ],  "cpu": [    "x64"  ]}],
    "~npm~turbo-darwin-arm64@2.5.4": [3,"turbo-darwin-arm64","sha512-2+Nx6LAyuXw2MdXb7pxqle3MYignLvS7OwtsP9SgtSBaMlnNlxl9BovzqdYAgkUW3AsYiQMJ/wBRb7d+xemM5A==","https://registry.npmjs.org/turbo-darwin-arm64/-/turbo-darwin-arm64-2.5.4.tgz",null,null,null,{  "os": [    "darwin"  ],  "cpu": [    "arm64"  ]}],
    "~npm~turbo-linux-64@2.5.4": [3,"turbo-linux-64","sha512-5May2kjWbc8w4XxswGAl74GZ5eM4Gr6IiroqdLhXeXyfvWEdm2mFYCSWOzz0/z5cAgqyGidF1jt1qzUR8hTmOA==","https://registry.npmjs.org/turbo-linux-64/-/turbo-linux-64-2.5.4.tgz",null,null,null,{  "os": [    "linux"  ],  "cpu": [    "x64"  ]}],
    "~npm~turbo-linux-arm64@2.5.4": [3,"turbo-linux-arm64","sha512-/2yqFaS3TbfxV3P5yG2JUI79P7OUQKOUvAnx4MV9Bdz6jqHsHwc9WZPpO4QseQm+NvmgY6ICORnoVPODxGUiJg==","https://registry.npmjs.org/turbo-linux-arm64/-/turbo-linux-arm64-2.5.4.tgz",null,null,null,{  "os": [    "linux"  ],  "cpu": [    "arm64"  ]}],
    "~npm~turbo-windows-64@2.5.4": [3,"turbo-windows-64","sha512-EQUO4SmaCDhO6zYohxIjJpOKRN3wlfU7jMAj3CgcyTPvQR/UFLEKAYHqJOnJtymbQmiiM/ihX6c6W6Uq0yC7mA==","https://registry.npmjs.org/turbo-windows-64/-/turbo-windows-64-2.5.4.tgz",null,null,null,{  "os": [    "win32"  ],  "cpu": [    "x64"  ]}],
    "~npm~turbo-windows-arm64@2.5.4": [3,"turbo-windows-arm64","sha512-oQ8RrK1VS8lrxkLriotFq+PiF7iiGgkZtfLKF4DDKsmdbPo0O9R2mQxm7jHLuXraRCuIQDWMIw6dpcr7Iykf4A==","https://registry.npmjs.org/turbo-windows-arm64/-/turbo-windows-arm64-2.5.4.tgz",null,null,null,{  "os": [    "win32"  ],  "cpu": [    "arm64"  ]}],
    "~npm~turbo@2.5.4": [2,"turbo","sha512-kc8ZibdRcuWUG1pbYSBFWqmIjynlD8Lp7IB6U3vIzvOv9VG+6Sp8bzyeBWE3Oi8XV5KsQrznyRTBPvrf99E4mA==","https://registry.npmjs.org/turbo/-/turbo-2.5.4.tgz",null,null,null,null,{  "turbo": "bin/turbo"}]
  },
  "edges": {
    "file~_d turbo": "dev 2.5.4 ~npm~turbo@2.5.4",
    "workspace~apps+web left-pad": "prod ^1.3.0 ~npm~left-pad@1.3.0",
    "workspace~apps+web @fixture/a": "prod workspace:* workspace~packages+a",
    "workspace~packages+a is-odd": "prod ^3.0.1 ~npm~is-odd@3.0.1",
    "workspace~packages+a @fixture/b": "prod workspace:* workspace~packages+b",
    "workspace~packages+b @sindresorhus/is": "dev ^7.0.0 ~npm~@sindresorhus+is@7.2.0",
    "workspace~packages+b is-even": "prod ^1.0.0 ~npm~is-even@1.0.0",
    "~npm~is-even@1.0.0 is-odd": "prod ^0.1.2 ~npm~is-odd@0.1.2",
    "~npm~is-number@3.0.0 kind-of": "prod ^3.0.2 ~npm~kind-of@3.2.2",
    "~npm~is-odd@0.1.2 is-number": "prod ^3.0.0 ~npm~is-number@3.0.0",
    "~npm~is-odd@3.0.1 is-number": "prod ^6.0.0 ~npm~is-number@6.0.0",
    "~npm~kind-of@3.2.2 is-buffer": "prod ^1.1.5 ~npm~is-buffer@1.1.6",
    "~npm~turbo@2.5.4 turbo-darwin-64": "optional 2.5.4 ~npm~turbo-darwin-64@2.5.4",
    "~npm~turbo@2.5.4 turbo-darwin-arm64": "optional 2.5.4 ~npm~turbo-darwin-arm64@2.5.4",
    "~npm~turbo@2.5.4 turbo-linux-64": "optional 2.5.4 ~npm~turbo-linux-64@2.5.4",
    "~npm~turbo@2.5.4 turbo-linux-arm64": "optional 2.5.4 ~npm~turbo-linux-arm64@2.5.4",
    "~npm~turbo@2.5.4 turbo-windows-64": "optional 2.5.4 ~npm~turbo-windows-64@2.5.4",
    "~npm~turbo@2.5.4 turbo-windows-arm64": "optional 2.5.4 ~npm~turbo-windows-arm64@2.5.4"
  }
}
"#;

    fn lockfile() -> VltLockfile {
        VltLockfile::load(LOCKFILE.as_bytes()).unwrap()
    }

    fn with_options(options: &str) -> VltLockfile {
        let contents = format!(
            r#"{{"lockfileVersion": 1, "options": {options}, "nodes": {{}}, "edges": {{}}}}"#
        );
        VltLockfile::load(contents.as_bytes()).unwrap()
    }

    #[test]
    fn test_roundtrip() {
        let encoded = lockfile().encode().unwrap();
        assert_eq!(std::str::from_utf8(&encoded).unwrap(), LOCKFILE);
    }

    #[test_case("", "turbo", Some("~npm~turbo@2.5.4"); "root dependency")]
    #[test_case(".", "turbo", Some("~npm~turbo@2.5.4"); "root as a dot")]
    #[test_case("packages/a", "is-odd", Some("~npm~is-odd@3.0.1"); "workspace dependency")]
    #[test_case("packages/b", "@sindresorhus/is", Some("~npm~@sindresorhus+is@7.2.0"); "scoped dependency")]
    #[test_case("packages/a", "@fixture/b", None; "workspace link")]
    #[test_case("packages/a", "left-pad", None; "declared by another workspace")]
    #[test_case("packages/missing", "is-odd", None; "unknown workspace")]
    #[test_case("apps/web", "~npm~is-odd@0.1.2", Some("~npm~is-odd@0.1.2"); "dependency id")]
    fn test_resolve_package(workspace: &str, name: &str, expected: Option<&str>) {
        let package = lockfile().resolve_package(workspace, name, "*").unwrap();
        assert_eq!(package.as_ref().map(|p| p.key.as_str()), expected);
        if let (Some(package), Some(expected)) = (package, expected) {
            assert_eq!(package.version, id_version(expected));
        }
    }

    #[test]
    fn test_all_dependencies() {
        let lockfile = lockfile();
        let dependencies = lockfile
            .all_dependencies("~npm~is-even@1.0.0")
            .unwrap()
            .unwrap();
        assert_eq!(
            dependencies.into_owned(),
            BTreeMap::from([("~npm~is-odd@0.1.2".to_owned(), "0.1.2".to_owned())])
        );
        // optional dependencies are followed as well
        assert_eq!(
            lockfile
                .all_dependencies("~npm~turbo@2.5.4")
                .unwrap()
                .unwrap()
                .len(),
            6
        );
        assert!(
            lockfile
                .all_dependencies("~npm~left-pad@1.3.0")
                .unwrap()
                .unwrap()
                .is_empty()
        );
        // importers and unknown ids are not packages
        assert!(lockfile.all_dependencies("file~_d").unwrap().is_none());
        assert!(lockfile.all_dependencies("left-pad").unwrap().is_none());
    }

    fn closures() -> HashMap<String, HashSet<Package>> {
        let workspace = |dependencies: &[(&str, &str)]| {
            dependencies
                .iter()
                .map(|(name, spec)| (name.to_string(), spec.to_string()))
                .collect::<BTreeMap<_, _>>()
        };
        crate::all_transitive_closures(
            &lockfile(),
            HashMap::from([
                ("".to_owned(), workspace(&[("turbo", "2.5.4")])),
                (
                    "apps/web".to_owned(),
                    workspace(&[("@fixture/a", "workspace:*"), ("left-pad", "^1.3.0")]),
                ),
                (
                    "packages/a".to_owned(),
                    workspace(&[("@fixture/b", "workspace:*"), ("is-odd", "^3.0.1")]),
                ),
                (
                    "packages/b".to_owned(),
                    workspace(&[("@sindresorhus/is", "^7.0.0"), ("is-even", "^1.0.0")]),
                ),
            ]),
            false,
        )
        .unwrap()
    }

    fn keys(closure: &HashSet<Package>) -> Vec<&str> {
        let mut keys: Vec<_> = closure.iter().map(|package| package.key.as_str()).collect();
        keys.sort_unstable();
        keys
    }

    #[test]
    fn test_transitive_closures() {
        let closures = closures();
        assert_eq!(
            keys(&closures["packages/a"]),
            ["~npm~is-number@6.0.0", "~npm~is-odd@3.0.1"]
        );
        // the other `is-odd`, reached through `is-even`
        assert_eq!(
            keys(&closures["packages/b"]),
            [
                "~npm~@sindresorhus+is@7.2.0",
                "~npm~is-buffer@1.1.6",
                "~npm~is-even@1.0.0",
                "~npm~is-number@3.0.0",
                "~npm~is-odd@0.1.2",
                "~npm~kind-of@3.2.2",
            ]
        );
        assert_eq!(keys(&closures["apps/web"]), ["~npm~left-pad@1.3.0"]);
        assert_eq!(closures[""].len(), 7);
    }

    #[test]
    fn test_subgraph() {
        let closures = closures();
        let packages: Vec<String> = ["", "packages/a", "packages/b"]
            .iter()
            .flat_map(|workspace| &closures[*workspace])
            .map(|package| package.key.clone())
            .collect();
        let pruned = lockfile()
            .subgraph(
                &["packages/a".to_owned(), "packages/b".to_owned()],
                &packages,
            )
            .unwrap();
        // what `vlt install` writes for the same project without `apps/web`
        let expected: String = LOCKFILE
            .split_inclusive('\n')
            .filter(|line| !line.contains("left-pad") && !line.contains("workspace~apps+web"))
            .collect();
        assert_eq!(
            std::str::from_utf8(&pruned.encode().unwrap()).unwrap(),
            expected
        );
    }

    #[test]
    fn test_subgraph_without_workspace_dependency() {
        // `apps/web` without `packages/a`, which `--production` leaves behind
        // when it is a devDependency
        let pruned = lockfile()
            .subgraph(
                &["apps/web".to_owned()],
                &["~npm~left-pad@1.3.0".to_owned()],
            )
            .unwrap();
        let encoded = pruned.encode().unwrap();
        let encoded = std::str::from_utf8(&encoded).unwrap();
        assert!(encoded.contains(r#""workspace~apps+web left-pad""#));
        assert!(!encoded.contains("workspace~packages+a"));
    }

    #[test]
    fn test_subgraph_recomputes_flags() {
        let lockfile = VltLockfile::load(
            br#"{
                "lockfileVersion": 1,
                "nodes": {
                    "~npm~a@1.0.0": [0, "a"],
                    "~npm~b@1.0.0": [4, "b"],
                    "~npm~c@1.0.0": [0, "c"],
                    "~npm~d@1.0.0": [1, "d"]
                },
                "edges": {
                    "workspace~app a": "prod ^1 ~npm~a@1.0.0",
                    "workspace~tool a": "dev ^1 ~npm~a@1.0.0",
                    "workspace~tool d": "optional ^1 ~npm~d@1.0.0",
                    "~npm~a@1.0.0 b": "prod ^1 ~npm~b@1.0.0",
                    "~npm~a@1.0.0 c": "peerOptional ^1 ~npm~c@1.0.0",
                    "~npm~d@1.0.0 c": "prod ^1 ~npm~c@1.0.0"
                }
            }"#,
        )
        .unwrap();
        let flags = |workspaces: &[&str]| {
            let workspaces: Vec<String> = workspaces.iter().map(|w| w.to_string()).collect();
            let packages: Vec<String> = lockfile.nodes.iter().map(|(id, _)| id.clone()).collect();
            let pruned = lockfile.subgraph(&workspaces, &packages).unwrap();
            let pruned = (pruned.as_ref() as &dyn Any)
                .downcast_ref::<VltLockfile>()
                .unwrap();
            pruned
                .nodes
                .iter()
                .map(|(_, node)| match node {
                    Json::Array(fields) => serde_json::to_string(&fields[0]).unwrap(),
                    _ => unreachable!(),
                })
                .collect::<Vec<_>>()
                .join(" ")
        };
        // with `app`, `a` and `b` have a production path; `c` is optional
        // either way and `d` is only an optional dependency
        assert_eq!(flags(&["app", "tool"]), "0 4 1 1");
        // without it they are left with the dev path of `tool`, and the other
        // bit of `b` is kept
        assert_eq!(flags(&["tool"]), "2 6 1 1");
    }

    #[test]
    fn test_subgraph_unknown_package() {
        let result = lockfile().subgraph(&[], &["~npm~nope@1.0.0".to_owned()]);
        assert!(
            matches!(result, Err(crate::Error::MissingPackage(key)) if key == "~npm~nope@1.0.0")
        );
    }

    #[test_case(r#"{"registries": {"npm": "https://registry.npmjs.org/"}}"#, false; "same options")]
    #[test_case(r#"{"registries": {"npm": "https://example.com/"}}"#, true; "registry")]
    #[test_case(r##"{"modifiers": {"#a": "2"}, "registries": {"npm": "https://registry.npmjs.org/"}}"##, true; "modifiers")]
    #[test_case(r#"{"catalog": {"a": "^2"}, "registries": {"npm": "https://registry.npmjs.org/"}}"#, false; "catalog")]
    #[test_case(r#"{"catalogs": {"dev": {"a": "^2"}}, "registries": {"npm": "https://registry.npmjs.org/"}}"#, false; "named catalogs")]
    fn test_global_change(options: &str, expected: bool) {
        assert_eq!(lockfile().global_change(&with_options(options)), expected);
    }

    #[test]
    fn test_global_change_other_format() {
        assert!(lockfile().global_change(&crate::NpmLockfile::default()));
    }

    #[test]
    fn test_turbo_version() {
        assert_eq!(lockfile().turbo_version().as_deref(), Some("2.5.4"));
        assert_eq!(with_options("{}").turbo_version(), None);
    }

    #[test]
    fn test_package_info() {
        let lockfile = lockfile();
        let package = lockfile
            .resolve_package("packages/b", "@sindresorhus/is", "^7.0.0")
            .unwrap()
            .unwrap();
        assert_eq!(
            lockfile.human_name(&package).as_deref(),
            Some("@sindresorhus/is@7.2.0")
        );
        assert_eq!(lockfile.package_source(&package), PackageSource::Registry);
        assert_eq!(lockfile.format_version().as_deref(), Some("1"));
    }

    #[test_case("git~github_ca+b~main", PackageSource::Git; "git")]
    #[test_case("file~vendor+a", PackageSource::File; "file")]
    #[test_case("remote~https_c++example.com+a.tgz", PackageSource::Registry; "remote tarball")]
    fn test_package_source(key: &str, expected: PackageSource) {
        let package = Package {
            key: key.to_owned(),
            version: id_version(key),
        };
        assert_eq!(lockfile().package_source(&package), expected);
    }

    #[test_case(r#"{"lockfileVersion": 2, "nodes": {}, "edges": {}}"#, "Unsupported vlt lockfile version: 2"; "newer version")]
    #[test_case(r#"{"nodes": {}, "edges": {}}"#, "Unsupported vlt lockfile version: null"; "no version")]
    #[test_case(r#"[]"#, "Invalid vlt lockfile: expected an object"; "not an object")]
    #[test_case(r#"{"lockfileVersion": 1, "nodes": []}"#, "Invalid vlt lockfile: `nodes` must be an object"; "nodes")]
    #[test_case(r#"{"lockfileVersion": 1, "edges": []}"#, "Invalid vlt lockfile: `edges` must be an object"; "edges")]
    #[test_case(r#"{"lockfileVersion": 1, "edges": {"a b": 1}}"#, "Invalid vlt lockfile: edge 'a b' must be a string"; "edge value")]
    #[test_case(r#"{"lockfileVersion": 1, "edges": {"a": "prod * b"}}"#, "Invalid vlt lockfile: unexpected edge 'a': 'prod * b'"; "edge key")]
    #[test_case(r#"{"lockfileVersion": 1, "edges": {"a b": "prod"}}"#, "Invalid vlt lockfile: unexpected edge 'a b': 'prod'"; "edge target")]
    fn test_invalid_lockfile(contents: &str, expected: &str) {
        let error = VltLockfile::load(contents.as_bytes()).unwrap_err();
        assert_eq!(error.to_string(), expected);
    }

    #[test]
    fn test_spec_with_spaces() {
        let lockfile = VltLockfile::load(
            br#"{
                "lockfileVersion": 1,
                "nodes": {"~npm~a@1.5.0": [0, "a"]},
                "edges": {"file~_d a": "prod >=1 <2 ~npm~a@1.5.0", "file~_d b": "optional * MISSING"}
            }"#,
        )
        .unwrap();
        let package = lockfile
            .resolve_package("", "a", ">=1 <2")
            .unwrap()
            .unwrap();
        assert_eq!(package.key, "~npm~a@1.5.0");
        assert_eq!(lockfile.resolve_package("", "b", "*").unwrap(), None);
    }

    #[test]
    fn test_empty_lockfile() {
        let encoded = with_options("{}").encode().unwrap();
        assert_eq!(
            std::str::from_utf8(&encoded).unwrap(),
            "{\n  \"lockfileVersion\": 1,\n  \"options\": {},\n  \"nodes\": {},\n  \"edges\": \
             {}\n}\n"
        );
    }

    #[test_case("", "file~_d"; "root")]
    #[test_case(".", "file~_d"; "root as a dot")]
    #[test_case("packages/a", "workspace~packages+a"; "nested")]
    #[test_case("my_pkgs/a+b c", "workspace~my__pkgs+a_pb_sc"; "escaped")]
    #[test_case("packages/a.", "workspace~packages+a_d"; "trailing dot")]
    fn test_importer_id(path: &str, expected: &str) {
        assert_eq!(importer_id(path), expected);
        if !path.is_empty() {
            let (_, encoded) = expected.split_once('~').unwrap();
            assert_eq!(decode_id_part(encoded), path);
        }
    }

    #[test_case("~npm~is-odd@3.0.1", "3.0.1"; "registry")]
    #[test_case("~npm~@sindresorhus+is@7.2.0", "7.2.0"; "scoped")]
    #[test_case("~npm~@affordances+md@0.5.0~peer.1e31dcefde560160", "0.5.0"; "peer context")]
    #[test_case("~jsr~@jsr+std____semver@1.0.8", "1.0.8"; "other registry")]
    #[test_case("~npm~a@1.0.0-beta.1_pbuild.5", "1.0.0-beta.1+build.5"; "build metadata")]
    #[test_case("git~git_pssh_c++host.com+x.git~branch", "branch"; "git selector")]
    #[test_case("git~github_ca+b~", ""; "git default branch")]
    #[test_case("file~vendor+a", "vendor/a"; "file")]
    #[test_case("remote~https_c++example.com+a.tgz", "https://example.com/a.tgz"; "remote tarball")]
    fn test_id_version(id: &str, expected: &str) {
        assert_eq!(id_version(id), expected);
    }

    #[test_case("a_0Ab", "a\nb"; "control character")]
    #[test_case("a_zb", "a_zb"; "unknown escape")]
    #[test_case("a_", "a_"; "trailing underscore")]
    fn test_decode_id_part(encoded: &str, expected: &str) {
        assert_eq!(decode_id_part(encoded), expected);
    }

    #[test]
    fn test_encode_control_character() {
        assert_eq!(encode_id_part("a\nb"), "a_0Ab");
    }
}
