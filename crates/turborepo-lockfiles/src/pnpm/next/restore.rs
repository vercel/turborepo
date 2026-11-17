//! The peer-suffixed model a v13 document describes: pnpm's
//! `read/entries.rs` (snapshot entries and importers), plus the mapping of
//! `packages` metadata and `settings` onto turbo's v9 structs.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_yaml_ng::{Mapping, Value};

use super::{
    super::{
        Dependency, LockfileSettings, Map, PackageSnapshot, PackageSnapshotV7, PatchFile,
        ProjectSnapshot,
    },
    key,
    read::{Graph, Provider, Sid},
    wire::{self, NextRef, Package},
};
use crate::pnpm::NextFormatError;

pub(super) struct Resolved<'g, 'a> {
    pub graph: Graph<'g, 'a>,
    pub base_keys: Vec<String>,
    pub keys: Vec<String>,
}

impl<'g, 'a> Resolved<'g, 'a> {
    pub fn of(doc: &'g wire::Document<'a>) -> Result<Self, NextFormatError> {
        let graph = Graph::new(doc)?;
        let base_keys = graph.base_keys();
        let keys = graph.keys(&base_keys);
        Ok(Resolved {
            graph,
            base_keys,
            keys,
        })
    }
}

pub(super) struct Sections {
    pub importers: BTreeMap<String, ProjectSnapshot>,
    pub packages: Option<BTreeMap<String, PackageSnapshot>>,
    pub snapshots: Option<BTreeMap<String, PackageSnapshotV7>>,
    pub settings: Option<LockfileSettings>,
    pub catalogs: Option<Map<String, Map<String, Dependency>>>,
    pub overrides: Option<Map<String, String>>,
    pub patched_dependencies: Option<Map<String, PatchFile>>,
    pub package_extensions_checksum: Option<String>,
    pub pnpmfile_checksum: Option<String>,
    pub ignored_optional_dependencies: Option<Vec<String>>,
    pub time: Option<Map<String, String>>,
}

/// Settings `next` keeps under `settings` that v9 keeps at the top level.
const LIFTED_SETTINGS: [&str; 6] = [
    "catalogs",
    "overrides",
    "patchedDependencies",
    "packageExtensionsChecksum",
    "pnpmfileChecksum",
    "ignoredOptionalDependencies",
];

pub(super) fn sections(resolved: &Resolved<'_, '_>) -> Result<Sections, NextFormatError> {
    let graph = &resolved.graph;
    let doc = graph.doc;
    let unbound = graph.unbound_peers();
    let mut snapshots = BTreeMap::new();
    for sid in 0..graph.keys.len() {
        let entry = snapshot_entry(resolved, sid, &unbound);
        if snapshots
            .insert(resolved.keys[sid].clone(), entry)
            .is_some()
        {
            return Err(NextFormatError::DuplicateKey {
                key: resolved.keys[sid].clone(),
            });
        }
    }
    let importers = doc
        .projects
        .iter()
        .zip(&graph.importers)
        .map(|(project, importer)| {
            Ok((
                importer.id.to_string(),
                restore_importer(resolved, project, importer)?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, NextFormatError>>()?;
    let packages = doc
        .packages
        .iter()
        .map(|package| Ok((package.key.to_string(), package_snapshot(package)?)))
        .collect::<Result<BTreeMap<_, _>, NextFormatError>>()?;

    let settings_map = doc.settings.map;
    let lifted = |name: &str| settings_map.and_then(|map| map.get(name));
    let strings = |value: &Value| -> Result<Map<String, String>, NextFormatError> {
        deserialize(value.clone(), "settings")
    };
    let base: Mapping = settings_map
        .map(|map| {
            map.iter()
                .filter(|(key, _)| {
                    key.as_str()
                        .is_none_or(|key| !LIFTED_SETTINGS.contains(&key))
                })
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default();
    let settings = (!base.is_empty())
        .then(|| deserialize::<LockfileSettings>(Value::Mapping(base), "settings"))
        .transpose()?;
    let catalogs = doc.settings.catalogs.as_ref().map(|catalogs| {
        catalogs
            .iter()
            .map(|(catalog, entries)| {
                let entries = entries
                    .iter()
                    .map(|(name, entry)| {
                        (
                            name.clone(),
                            Dependency {
                                specifier: entry.specifier.clone(),
                                version: entry.version.clone(),
                            },
                        )
                    })
                    .collect();
                (catalog.clone(), entries)
            })
            .collect()
    });
    let patched_dependencies = doc.settings.patched_dependencies.as_ref().map(|patches| {
        patches
            .iter()
            .map(|(key, hash)| (key.to_string(), PatchFile::Hash(hash.to_string())))
            .collect()
    });
    let string = |name: &str| -> Result<Option<String>, NextFormatError> {
        lifted(name)
            .map(|value| deserialize(value.clone(), name))
            .transpose()
    };
    Ok(Sections {
        importers,
        packages: (!packages.is_empty()).then_some(packages),
        snapshots: (!snapshots.is_empty()).then_some(snapshots),
        settings,
        catalogs,
        overrides: lifted("overrides").map(strings).transpose()?,
        patched_dependencies,
        package_extensions_checksum: string("packageExtensionsChecksum")?,
        pnpmfile_checksum: string("pnpmfileChecksum")?,
        ignored_optional_dependencies: lifted("ignoredOptionalDependencies")
            .map(|value| deserialize(value.clone(), "ignoredOptionalDependencies"))
            .transpose()?,
        time: doc
            .root
            .get("time")
            .map(|time| deserialize(time.clone(), "time"))
            .transpose()?,
    })
}

fn deserialize<T: for<'de> Deserialize<'de>>(
    value: Value,
    what: &str,
) -> Result<T, NextFormatError> {
    T::deserialize(value).map_err(|error| NextFormatError::Syntax(format!("{what}: {error}")))
}

/// Port of pnpm `snapshot_entry`.
fn snapshot_entry(
    resolved: &Resolved<'_, '_>,
    sid: Sid,
    unbound: &super::read::UnboundPeers<'_>,
) -> PackageSnapshotV7 {
    let graph = &resolved.graph;
    let entry = graph.entry(sid);
    let mut dependencies = restored_children(resolved, sid, entry.dependencies.as_ref());
    let mut optional_dependencies =
        restored_children(resolved, sid, entry.optional_dependencies.as_ref());
    let package = &graph.doc.packages[graph.slots[sid].0];
    for (name, reference) in entry.peer_dependencies.iter().flatten() {
        let optional = package.peer_optional.get(name).copied().unwrap_or(false);
        let target = if optional {
            &mut optional_dependencies
        } else {
            &mut dependencies
        };
        target.insert(
            name.to_string(),
            restored_peer(resolved, sid, name, reference),
        );
    }
    let below = graph.unbound_below(sid, unbound);
    let mut transitive: Vec<String> = entry
        .transitive_peer_dependencies
        .iter()
        .flatten()
        .map(|(name, _)| name.to_string())
        .collect();
    match &entry.unbound_peer_dependencies {
        Some(names) => transitive.extend(names.iter().map(|name| name.to_string())),
        None => transitive.extend(below.iter().map(|name| name.to_string())),
    }
    transitive.sort_unstable();
    transitive.dedup();
    PackageSnapshotV7 {
        optional: entry.optional,
        dependencies: (!dependencies.is_empty()).then_some(dependencies),
        optional_dependencies: (!optional_dependencies.is_empty()).then_some(optional_dependencies),
        transitive_peer_dependencies: (!transitive.is_empty()).then_some(transitive),
    }
}

fn restored_children(
    resolved: &Resolved<'_, '_>,
    sid: Sid,
    children: Option<&BTreeMap<&str, NextRef<'_>>>,
) -> Map<String, String> {
    children
        .into_iter()
        .flatten()
        .map(|(alias, reference)| {
            let restored = match super::read::child_named(&resolved.graph.children[sid], alias) {
                Some(child) => key::with_key(reference.version, &resolved.keys[child]),
                None => reference.version.to_string(),
            };
            (alias.to_string(), restored)
        })
        .collect()
}

fn restored_peer(
    resolved: &Resolved<'_, '_>,
    sid: Sid,
    name: &str,
    reference: &NextRef<'_>,
) -> String {
    let provider = resolved.graph.providers[sid]
        .iter()
        .find(|(bound, _)| *bound == name)
        .map(|(_, provider)| provider);
    match provider {
        Some(Provider::Snapshot(provider)) => {
            key::with_key(reference.version, &resolved.keys[*provider])
        }
        _ => reference.version.to_string(),
    }
}

fn restore_importer(
    resolved: &Resolved<'_, '_>,
    project: &wire::Project<'_>,
    importer: &super::read::Importer<'_>,
) -> Result<ProjectSnapshot, NextFormatError> {
    let id = importer.id;
    let chosen = &resolved.graph.importer_children[id];
    let mut out = Mapping::new();
    for (section, deps) in &importer.sections {
        let deps: Mapping = deps
            .iter()
            .map(|(alias, dep)| {
                let version = match super::read::child_named(chosen, alias) {
                    Some(sid) => key::with_key(&dep.version, &resolved.keys[sid]),
                    None => dep.version.clone(),
                };
                let mut entry = Mapping::new();
                entry.insert("specifier".into(), dep.specifier.into());
                entry.insert("version".into(), version.into());
                (Value::from(*alias), Value::Mapping(entry))
            })
            .collect();
        out.insert(section.key().into(), Value::Mapping(deps));
    }
    for field in ["dependenciesMeta", "publishDirectory"] {
        if let Some(value) = project.map.get(field) {
            out.insert(field.into(), value.clone());
        }
    }
    deserialize(Value::Mapping(out), &format!("projects[{id}]"))
}

/// Metadata fields a v13 entry shares with a v9 `packages` entry, by name.
/// v13's `specifiers` has no v9 counterpart: pnpm drops it from v9 output.
const METADATA_FIELDS: [&str; 8] = [
    "version",
    "engines",
    "cpu",
    "os",
    "libc",
    "hasBin",
    "prepare",
    "bundledDependencies",
];

/// A `packages` entry as the v9 metadata entry it restores to: the
/// resolution rebuilt (pnpm `into_metadata`) and `peerDependencies` split back
/// into ranges and `peerDependenciesMeta`. v13 drops `deprecated`.
fn package_snapshot(package: &Package<'_>) -> Result<PackageSnapshot, NextFormatError> {
    let map = package.map;
    let integrity = map.get("integrity").cloned();
    let resolution = match (map.get("resolution"), integrity) {
        (None, Some(integrity)) => {
            let mut resolution = Mapping::new();
            resolution.insert("integrity".into(), integrity);
            resolution
        }
        (None, None) => {
            let directory = key::split(package.key)
                .and_then(|(_, suffix)| suffix.strip_prefix("file:"))
                .ok_or_else(|| NextFormatError::MissingResolution {
                    key: package.key.to_string(),
                })?;
            let mut resolution = Mapping::new();
            resolution.insert("directory".into(), directory.into());
            resolution.insert("type".into(), "directory".into());
            resolution
        }
        (Some(Value::Mapping(resolution)), Some(integrity))
            if resolution.contains_key("tarball") && !resolution.contains_key("type") =>
        {
            let mut resolution = resolution.clone();
            if !resolution.contains_key("integrity") {
                resolution.insert("integrity".into(), integrity);
            }
            resolution
        }
        (Some(Value::Mapping(resolution)), _) => resolution.clone(),
        (Some(_), _) => {
            return Err(NextFormatError::Syntax(format!(
                "packages[{}].resolution is not a map",
                package.key
            )));
        }
    };
    let mut out = Mapping::new();
    out.insert("resolution".into(), Value::Mapping(resolution));
    for field in METADATA_FIELDS {
        if let Some(value) = map.get(field) {
            out.insert(field.into(), value.clone());
        }
    }
    if !package.peer_ranges.is_empty() {
        let ranges: Mapping = package
            .peer_ranges
            .iter()
            .map(|(name, range)| (Value::from(*name), Value::from(*range)))
            .collect();
        out.insert("peerDependencies".into(), Value::Mapping(ranges));
    }
    if !package.peer_optional.is_empty() {
        let meta: Mapping = package
            .peer_optional
            .iter()
            .map(|(name, optional)| {
                let mut meta = Mapping::new();
                meta.insert("optional".into(), Value::Bool(*optional));
                (Value::from(*name), Value::Mapping(meta))
            })
            .collect();
        out.insert("peerDependenciesMeta".into(), Value::Mapping(meta));
    }
    deserialize(Value::Mapping(out), &format!("packages[{}]", package.key))
}
