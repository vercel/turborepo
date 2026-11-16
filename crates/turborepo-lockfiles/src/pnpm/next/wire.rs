//! The v13 main document as one order-preserving tree, and read-only views of
//! the parts pnpm's reader interprets. The tree is what `encode` writes and
//! what `prune` filters, so fields no view models pass through untouched.

use std::collections::BTreeMap;

use serde_yaml_ng::{Mapping, Value};

use super::{super::super::NextFormatError, key};

pub(super) const DOCUMENT_START: &str = "---\n";
pub(super) const DOCUMENT_SEPARATOR: &str = "\n---\n";

/// The text of a lockfile split into its env document and main document,
/// after stripping a BOM and normalizing CRLF the way pnpm does.
pub(super) struct Split {
    pub env: Option<String>,
    pub main: String,
}

pub(super) fn split(bytes: &[u8]) -> Result<Split, NextFormatError> {
    let text =
        std::str::from_utf8(bytes).map_err(|error| NextFormatError::Syntax(error.to_string()))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let text = text.replace("\r\n", "\n");
    let Some(rest) = text.strip_prefix(DOCUMENT_START) else {
        return Ok(Split {
            env: None,
            main: text,
        });
    };
    match rest.find(DOCUMENT_SEPARATOR) {
        Some(at) => Ok(Split {
            env: Some(rest[..at].to_string()),
            main: rest[at + DOCUMENT_SEPARATOR.len()..].to_string(),
        }),
        None => Ok(Split {
            env: None,
            main: rest.to_string(),
        }),
    }
}

/// Whether the main document declares `lockfileVersion` 13.x. Reads only
/// the root mapping's `lockfileVersion`, so it also works when the rest of the
/// document would not parse.
pub(in crate::pnpm::data) fn declares_next_format(bytes: &[u8]) -> bool {
    let Ok(split) = split(bytes) else {
        return false;
    };
    let main = split.main.trim_start();
    let version = if main.starts_with('{') {
        json_lockfile_version(main)
    } else {
        main.lines().find_map(yaml_version_value)
    };
    version.is_some_and(|version| {
        version
            .split('.')
            .next()
            .and_then(|major| major.parse::<u16>().ok())
            == Some(13)
    })
}

/// Port of pnpm `json_lockfile_version`: the root `lockfileVersion` string of
/// a JSON document, found without parsing it.
fn json_lockfile_version(document: &str) -> Option<&str> {
    let mut depth = 0usize;
    let mut expecting_key = false;
    let mut rest = document;
    loop {
        let index = rest.find(['{', '}', '[', ']', ',', ':', '"'])?;
        let (found, tail) = (rest.as_bytes()[index], &rest[index + 1..]);
        rest = tail;
        match found {
            b'"' => {
                let (text, after) = string_end(tail)?;
                if depth == 1 && expecting_key && text == "lockfileVersion" {
                    let value = after
                        .trim_start()
                        .strip_prefix(':')?
                        .trim_start()
                        .strip_prefix('"')?;
                    return string_end(value).map(|(text, _)| text);
                }
                rest = after;
                expecting_key = false;
            }
            b'{' | b'[' => {
                depth += 1;
                expecting_key = depth == 1 && found == b'{';
            }
            b'}' | b']' => depth = depth.checked_sub(1)?,
            b',' => expecting_key = depth == 1,
            _ => expecting_key = false,
        }
    }
}

fn string_end(text: &str) -> Option<(&str, &str)> {
    let mut from = 0;
    loop {
        let index = from + text.get(from..)?.find(['"', '\\'])?;
        if text.as_bytes()[index] == b'"' {
            return Some((&text[..index], &text[index + 1..]));
        }
        from = index + 2;
    }
}

fn yaml_version_value(line: &str) -> Option<&str> {
    let value = [
        "lockfileVersion:",
        "'lockfileVersion':",
        r#""lockfileVersion":"#,
    ]
    .iter()
    .find_map(|key| line.strip_prefix(key))?;
    let value = value
        .char_indices()
        .find(|&(at, character)| character == '#' && value[..at].ends_with([' ', '\t']))
        .map_or(value, |(at, _)| &value[..at]);
    Some(value.trim().trim_matches(['\'', '"']))
}

/// The main document as a tree. pnpm writes JSON with tabs and trailing
/// commas, which is YAML flow syntax; older 12.x builds wrote block YAML.
pub(super) fn parse_main(main: &str) -> Result<Value, NextFormatError> {
    let value: Value = serde_yaml_ng::from_str(main)
        .map_err(|error| NextFormatError::Syntax(error.to_string()))?;
    if !value.is_mapping() {
        return Err(syntax("the main document is not a map"));
    }
    Ok(value)
}

pub(super) fn syntax(detail: impl Into<String>) -> NextFormatError {
    NextFormatError::Syntax(detail.into())
}

pub(super) fn get<'a>(map: &'a Mapping, key: &str) -> Option<&'a Value> {
    map.get(key)
}

fn as_str<'a>(value: &'a Value, what: &str) -> Result<&'a str, NextFormatError> {
    value
        .as_str()
        .ok_or_else(|| syntax(format!("{what} is not a string")))
}

fn as_map<'a>(value: &'a Value, what: &str) -> Result<&'a Mapping, NextFormatError> {
    value
        .as_mapping()
        .ok_or_else(|| syntax(format!("{what} is not a map")))
}

fn opt_str<'a>(
    map: &'a Mapping,
    key: &str,
    what: &str,
) -> Result<Option<&'a str>, NextFormatError> {
    get(map, key).map(|value| as_str(value, what)).transpose()
}

fn opt_bool(map: &Mapping, key: &str) -> bool {
    get(map, key).and_then(Value::as_bool).unwrap_or(false)
}

/// Iterate a map's entries as `(&str, &Value)`.
pub(super) fn entries<'a>(
    map: &'a Mapping,
    what: &str,
) -> Result<Vec<(&'a str, &'a Value)>, NextFormatError> {
    map.iter()
        .map(|(key, value)| Ok((as_str(key, what)?, value)))
        .collect()
}

/// `with`: the bindings that name a snapshot.
pub(super) type Bindings<'a> = BTreeMap<&'a str, NextRef<'a>>;

/// A reference: `"1.0.0"`, `"name@1.0.0"` (alias), `"link:..."`, or
/// `{version, with: {peer: reference}}`. Never carries peer or patch groups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NextRef<'a> {
    pub version: &'a str,
    pub with: Option<Bindings<'a>>,
}

impl<'a> NextRef<'a> {
    fn parse(value: &'a Value, what: &str) -> Result<Self, NextFormatError> {
        match value {
            Value::String(version) => Ok(NextRef {
                version,
                with: None,
            }),
            Value::Mapping(map) => Ok(NextRef {
                version: as_str(
                    get(map, "version").ok_or_else(|| syntax(format!("{what} has no version")))?,
                    what,
                )?,
                with: Some(parse_bindings(
                    get(map, "with").ok_or_else(|| syntax(format!("{what} has no with")))?,
                    what,
                )?),
            }),
            _ => Err(syntax(format!("{what} is neither a version nor a map"))),
        }
    }

    pub fn peers(&self) -> Option<&Bindings<'a>> {
        self.with.as_ref()
    }

    /// The package key the reference names, without groups.
    pub fn resolve(&self, alias: &str) -> Option<String> {
        key::resolve(alias, self.version)
    }

    pub fn as_link_target(&self) -> Option<&'a str> {
        key::as_link_target(self.version)
    }
}

fn parse_bindings<'a>(value: &'a Value, what: &str) -> Result<Bindings<'a>, NextFormatError> {
    entries(as_map(value, what)?, what)?
        .into_iter()
        .map(|(name, reference)| Ok((name, NextRef::parse(reference, what)?)))
        .collect()
}

fn parse_refs<'a>(
    map: &'a Mapping,
    key: &str,
    what: &str,
) -> Result<Option<BTreeMap<&'a str, NextRef<'a>>>, NextFormatError> {
    get(map, key)
        .map(|value| parse_bindings(value, what))
        .transpose()
}

/// Port of pnpm `names_binding`: whether the binding `binding` satisfies the
/// `with` entry `wanted`.
pub(super) fn names_binding(wanted: &NextRef<'_>, binding: &NextRef<'_>) -> bool {
    wanted.version == key::without_groups(binding.version)
        && wanted.peers().is_none_or(|wanted| match binding.peers() {
            Some(actual) => actual == wanted,
            None => wanted.is_empty(),
        })
}

/// One peer snapshot of a package: its own fields over the entry's shared
/// ones. `own` and `below` are the two kinds of binding pnpm's in-memory
/// snapshot keeps apart.
#[derive(Debug, Clone, Default)]
pub(super) struct SnapshotEntry<'a> {
    pub id: Option<&'a str>,
    pub dependencies: Option<BTreeMap<&'a str, NextRef<'a>>>,
    pub optional_dependencies: Option<BTreeMap<&'a str, NextRef<'a>>>,
    /// The package's own bound peers.
    pub peer_dependencies: Option<BTreeMap<&'a str, NextRef<'a>>>,
    /// Peers bound for a dependency below (`below: true`).
    pub transitive_peer_dependencies: Option<BTreeMap<&'a str, NextRef<'a>>>,
    pub unbound_peer_dependencies: Option<Vec<&'a str>>,
    pub optional: bool,
}

impl<'a> SnapshotEntry<'a> {
    /// Every peer binding: the package's own, then those from below.
    pub fn bindings(&self) -> impl Iterator<Item = (&'a str, &NextRef<'a>)> {
        self.peer_dependencies
            .iter()
            .chain(self.transitive_peer_dependencies.iter())
            .flatten()
            .map(|(name, reference)| (*name, reference))
    }

    pub fn reference_in_scope(&self, name: &str) -> Option<&NextRef<'a>> {
        [
            &self.dependencies,
            &self.optional_dependencies,
            &self.peer_dependencies,
            &self.transitive_peer_dependencies,
        ]
        .into_iter()
        .find_map(|map| map.as_ref()?.get(name))
    }

    /// Port of pnpm `NextSnapshotEntry::over`.
    fn over(mut self, shared: &SnapshotEntry<'a>) -> SnapshotEntry<'a> {
        for (map, shared_map) in [
            (&mut self.dependencies, &shared.dependencies),
            (
                &mut self.optional_dependencies,
                &shared.optional_dependencies,
            ),
            (&mut self.peer_dependencies, &shared.peer_dependencies),
            (
                &mut self.transitive_peer_dependencies,
                &shared.transitive_peer_dependencies,
            ),
        ] {
            if let Some(common) = shared_map {
                map.get_or_insert_default().extend(
                    common
                        .iter()
                        .map(|(name, reference)| (*name, reference.clone())),
                );
            }
        }
        self.id = self.id.or(shared.id);
        self.unbound_peer_dependencies = self
            .unbound_peer_dependencies
            .or_else(|| shared.unbound_peer_dependencies.clone());
        self.optional |= shared.optional;
        self
    }

    /// The graph fields of a map: a `packages` entry or one element of its
    /// `snapshots`.
    fn parse(map: &'a Mapping, what: &str) -> Result<Self, NextFormatError> {
        let mut own = None;
        let mut below = None;
        if let Some(peers) = get(map, "peerDependencies") {
            for (name, entry) in entries(as_map(peers, what)?, what)? {
                let Some(binding) = PeerEntry::parse(entry, what)?.binding else {
                    continue;
                };
                let target: &mut Option<BTreeMap<_, _>> =
                    if binding.below { &mut below } else { &mut own };
                target
                    .get_or_insert_default()
                    .insert(name, binding.reference);
            }
        }
        let unbound = get(map, "unboundPeerDependencies")
            .map(|value| {
                value
                    .as_sequence()
                    .ok_or_else(|| syntax(format!("{what} unboundPeerDependencies is not a list")))?
                    .iter()
                    .map(|name| as_str(name, what))
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?;
        Ok(SnapshotEntry {
            id: opt_str(map, "id", what)?,
            dependencies: parse_refs(map, "dependencies", what)?,
            optional_dependencies: parse_refs(map, "optionalDependencies", what)?,
            peer_dependencies: own,
            transitive_peer_dependencies: below,
            unbound_peer_dependencies: unbound,
            optional: opt_bool(map, "optional"),
        })
    }
}

/// One entry of `peerDependencies`: the declared range, `optional` from
/// `peerDependenciesMeta`, and the binding when there is one.
pub(super) struct PeerEntry<'a> {
    pub specifier: Option<&'a str>,
    pub optional: Option<bool>,
    pub binding: Option<PeerBinding<'a>>,
}

pub(super) struct PeerBinding<'a> {
    pub reference: NextRef<'a>,
    pub below: bool,
}

impl<'a> PeerEntry<'a> {
    pub fn parse(value: &'a Value, what: &str) -> Result<Self, NextFormatError> {
        let map = match value {
            Value::String(specifier) => {
                return Ok(PeerEntry {
                    specifier: Some(specifier),
                    optional: None,
                    binding: None,
                });
            }
            Value::Mapping(map) => map,
            _ => {
                return Err(syntax(format!(
                    "{what} peer entry is neither a range nor a map"
                )));
            }
        };
        let binding = opt_str(map, "version", what)?
            .map(|version| {
                Ok::<_, NextFormatError>(PeerBinding {
                    reference: NextRef {
                        version,
                        with: get(map, "with")
                            .map(|with| parse_bindings(with, what))
                            .transpose()?,
                    },
                    below: opt_bool(map, "below"),
                })
            })
            .transpose()?;
        Ok(PeerEntry {
            specifier: opt_str(map, "specifier", what)?,
            optional: get(map, "optional").and_then(Value::as_bool),
            binding,
        })
    }
}

/// A `packages` entry: the package's metadata, and every peer snapshot with
/// the shared graph fields applied.
pub(super) struct Package<'a> {
    pub key: &'a str,
    pub name: &'a str,
    pub map: &'a Mapping,
    /// Declared peer ranges, as v9 `peerDependencies`.
    pub peer_ranges: BTreeMap<&'a str, &'a str>,
    /// `peerDependenciesMeta.<name>.optional`.
    pub peer_optional: BTreeMap<&'a str, bool>,
    pub snapshots: Vec<SnapshotEntry<'a>>,
}

impl<'a> Package<'a> {
    fn parse(key: &'a str, value: &'a Value) -> Result<Self, NextFormatError> {
        let what = &format!("packages[{key}]");
        let map = as_map(value, what)?;
        let (name, _) =
            key::split(key).ok_or_else(|| syntax(format!("{what} is not a package key")))?;
        let mut peer_ranges = BTreeMap::new();
        let mut peer_optional = BTreeMap::new();
        if let Some(peers) = get(map, "peerDependencies") {
            for (peer, entry) in entries(as_map(peers, what)?, what)? {
                let entry = PeerEntry::parse(entry, what)?;
                if let Some(optional) = entry.optional {
                    peer_optional.insert(peer, optional);
                }
                if let Some(specifier) = entry.specifier {
                    peer_ranges.insert(peer, specifier);
                }
            }
        }
        let shared = SnapshotEntry::parse(map, what)?;
        let snapshots = match get(map, "snapshots").and_then(Value::as_sequence) {
            Some(elements) if !elements.is_empty() => elements
                .iter()
                .enumerate()
                .map(|(index, element)| {
                    let what = &format!("{what}.snapshots[{index}]");
                    Ok(SnapshotEntry::parse(as_map(element, what)?, what)?.over(&shared))
                })
                .collect::<Result<Vec<_>, NextFormatError>>()?,
            _ => vec![shared],
        };
        Ok(Package {
            key,
            name,
            map,
            peer_ranges,
            peer_optional,
            snapshots,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Section {
    Dependencies,
    DevDependencies,
    OptionalDependencies,
}

impl Section {
    pub const ALL: [Section; 3] = [
        Section::Dependencies,
        Section::DevDependencies,
        Section::OptionalDependencies,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Section::Dependencies => "dependencies",
            Section::DevDependencies => "devDependencies",
            Section::OptionalDependencies => "optionalDependencies",
        }
    }
}

/// A project dependency as written: the specifier alone, with `with`, or
/// with an explicit `version`.
pub(super) struct ProjectDep<'a> {
    pub specifier: &'a str,
    pub version: Option<&'a str>,
    pub with: Option<Bindings<'a>>,
}

pub(super) struct Project<'a> {
    pub id: &'a str,
    pub name: Option<&'a str>,
    pub map: &'a Mapping,
    pub sections: Vec<(Section, Vec<(&'a str, ProjectDep<'a>)>)>,
}

impl<'a> Project<'a> {
    fn parse(id: &'a str, value: &'a Value) -> Result<Self, NextFormatError> {
        let what = &format!("projects[{id}]");
        let map = as_map(value, what)?;
        let mut sections = Vec::new();
        for section in Section::ALL {
            let Some(deps) = get(map, section.key()) else {
                continue;
            };
            let deps = entries(as_map(deps, what)?, what)?
                .into_iter()
                .map(|(alias, dep)| {
                    let dep = match dep {
                        Value::String(specifier) => ProjectDep {
                            specifier,
                            version: None,
                            with: None,
                        },
                        Value::Mapping(dep) => ProjectDep {
                            specifier: as_str(
                                get(dep, "specifier").ok_or_else(|| {
                                    syntax(format!("{what}.{alias} has no specifier"))
                                })?,
                                what,
                            )?,
                            version: opt_str(dep, "version", what)?,
                            with: get(dep, "with")
                                .map(|with| parse_bindings(with, what))
                                .transpose()?,
                        },
                        _ => return Err(syntax(format!("{what}.{alias} is not a dependency"))),
                    };
                    Ok((alias, dep))
                })
                .collect::<Result<Vec<_>, NextFormatError>>()?;
            sections.push((section, deps));
        }
        Ok(Project {
            id,
            name: opt_str(map, "name", what)?,
            map,
            sections,
        })
    }
}

pub(super) struct CatalogEntry {
    pub specifier: String,
    pub version: String,
}

/// The `settings` the reader interprets. Everything else in `settings` maps
/// to turbo's `LockfileSettings`.
pub(super) struct Settings<'a> {
    pub map: Option<&'a Mapping>,
    pub catalogs: Option<BTreeMap<String, BTreeMap<String, CatalogEntry>>>,
    pub patched_dependencies: Option<BTreeMap<&'a str, &'a str>>,
    pub dedupe_peers: bool,
    pub peers_suffix_max_length: usize,
}

const DEFAULT_PEERS_SUFFIX_MAX_LENGTH: usize = 1000;

impl<'a> Settings<'a> {
    fn parse(value: Option<&'a Value>) -> Result<Self, NextFormatError> {
        let what = "settings";
        let map = value.map(|value| as_map(value, what)).transpose()?;
        let catalogs = map
            .and_then(|map| get(map, "catalogs"))
            .map(|catalogs| {
                entries(as_map(catalogs, what)?, what)?
                    .into_iter()
                    .map(|(catalog, entries_value)| {
                        let entries = entries(as_map(entries_value, what)?, what)?
                            .into_iter()
                            .map(|(name, entry)| {
                                Ok((name.to_string(), catalog_entry(catalog, name, entry)?))
                            })
                            .collect::<Result<BTreeMap<_, _>, NextFormatError>>()?;
                        Ok((catalog.to_string(), entries))
                    })
                    .collect::<Result<BTreeMap<_, _>, NextFormatError>>()
            })
            .transpose()?;
        let patched_dependencies = map
            .and_then(|map| get(map, "patchedDependencies"))
            .map(|patches| {
                entries(as_map(patches, what)?, what)?
                    .into_iter()
                    .map(|(key, patch)| {
                        let hash = match patch {
                            Value::Mapping(patch) => get(patch, "hash")
                                .ok_or_else(|| syntax(format!("patch {key} has no hash")))?,
                            hash => hash,
                        };
                        Ok((key, as_str(hash, what)?))
                    })
                    .collect::<Result<BTreeMap<_, _>, NextFormatError>>()
            })
            .transpose()?;
        let peers_suffix_max_length = map
            .and_then(|map| get(map, "peersSuffixMaxLength"))
            .and_then(Value::as_u64)
            .map_or(DEFAULT_PEERS_SUFFIX_MAX_LENGTH, |length| {
                usize::try_from(length).unwrap_or(usize::MAX)
            });
        Ok(Settings {
            map,
            catalogs,
            patched_dependencies,
            dedupe_peers: map
                .and_then(|map| get(map, "dedupePeers"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
            peers_suffix_max_length,
        })
    }
}

/// Port of pnpm `catalogs::resolve`: a bare specifier implies its version.
fn catalog_entry(
    catalog: &str,
    name: &str,
    entry: &Value,
) -> Result<CatalogEntry, NextFormatError> {
    let what = "settings.catalogs";
    match entry {
        Value::Mapping(entry) => Ok(CatalogEntry {
            specifier: as_str(
                get(entry, "specifier").ok_or_else(|| syntax("catalog entry has no specifier"))?,
                what,
            )?
            .to_string(),
            version: as_str(
                get(entry, "version").ok_or_else(|| syntax("catalog entry has no version"))?,
                what,
            )?
            .to_string(),
        }),
        Value::String(specifier) => {
            let version =
                super::implied_version::catalog_implied_version(specifier).ok_or_else(|| {
                    NextFormatError::UnderivableCatalogVersion {
                        catalog: catalog.to_string(),
                        name: name.to_string(),
                        specifier: specifier.clone(),
                    }
                })?;
            Ok(CatalogEntry {
                specifier: specifier.clone(),
                version: version.to_string(),
            })
        }
        _ => Err(syntax("catalog entry is neither a specifier nor a map")),
    }
}

/// The views the reader works from, borrowed from the tree.
pub(super) struct Document<'a> {
    pub root: &'a Mapping,
    pub settings: Settings<'a>,
    pub projects: Vec<Project<'a>>,
    pub packages: Vec<Package<'a>>,
}

impl<'a> Document<'a> {
    pub fn parse(tree: &'a Value) -> Result<Self, NextFormatError> {
        let root = as_map(tree, "the main document")?;
        let projects = get(root, "projects")
            .map(|projects| {
                entries(as_map(projects, "projects")?, "projects")?
                    .into_iter()
                    .map(|(id, project)| Project::parse(id, project))
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        let packages = get(root, "packages")
            .map(|packages| {
                entries(as_map(packages, "packages")?, "packages")?
                    .into_iter()
                    .map(|(key, package)| Package::parse(key, package))
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        Ok(Document {
            root,
            settings: Settings::parse(get(root, "settings"))?,
            projects,
            packages,
        })
    }
}
