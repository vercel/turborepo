use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde_yaml_ng::{Mapping, Value};

use super::{
    super::PnpmLockfile,
    implied_version::Implied,
    read::{Provider, Sid},
    restore::Resolved,
    wire::{self, Bindings, Document},
};
use crate::pnpm::NextFormatError;

const MAX_PIN_ROUNDS: usize = 4;

pub(super) fn prune(tree: &Value, kept: &PnpmLockfile) -> Result<Value, NextFormatError> {
    let doc = Document::parse(tree)?;
    let original = Resolved::of(&doc)?;
    let sid_of: HashMap<&str, Sid> = original
        .keys
        .iter()
        .enumerate()
        .map(|(sid, key)| (key.as_str(), sid))
        .collect();
    let mut retained = vec![false; original.keys.len()];
    for key in kept.snapshots.iter().flat_map(|snapshots| snapshots.keys()) {
        let sid = sid_of
            .get(key.as_str())
            .ok_or_else(|| NextFormatError::UnknownRetainedKey { key: key.clone() })?;
        retained[*sid] = true;
    }

    let mut pruned = filter(tree, &original, &retained, kept);
    write_unimplied_versions(&mut pruned, &original)?;
    for _ in 0..MAX_PIN_ROUNDS {
        let pins = {
            let filtered_doc = Document::parse(&pruned)?;
            let filtered = Resolved::of(&filtered_doc)?;
            let origin = origins(&original, &filtered, &retained);
            pins_for_drift(&original, &filtered, &origin)?
        };
        if pins.is_empty() {
            return Ok(pruned);
        }
        for pin in &pins {
            pin.apply(&mut pruned)?;
        }
    }
    Err(not_representable(
        "pinning the peer bindings that filtering changed did not settle",
    ))
}

fn not_representable(detail: impl Into<String>) -> NextFormatError {
    NextFormatError::NotRepresentable {
        detail: detail.into(),
    }
}

fn filter(
    tree: &Value,
    original: &Resolved<'_, '_>,
    retained: &[bool],
    kept: &PnpmLockfile,
) -> Value {
    let graph = &original.graph;
    let Value::Mapping(root) = tree else {
        return tree.clone();
    };
    let mut out = Mapping::new();
    for (field, value) in root {
        let value = match (field.as_str(), value) {
            (Some("time"), _) => continue,
            (Some("projects"), Value::Mapping(projects)) => {
                Value::Mapping(filter_projects(projects, kept))
            }
            (Some("packages"), Value::Mapping(packages)) => {
                let mut filtered = Mapping::new();
                for (index, (key, package)) in packages.iter().enumerate() {
                    let package_key = graph.doc.packages[index].key;
                    let (start, count) = graph.by_package[package_key];
                    let elements: Vec<usize> = (0..count)
                        .filter(|element| retained[start + element])
                        .collect();
                    if elements.is_empty() {
                        continue;
                    }
                    filtered.insert(key.clone(), filter_package(package, &elements));
                }
                Value::Mapping(filtered)
            }
            (Some("settings"), Value::Mapping(settings)) => {
                Value::Mapping(filter_settings(settings, kept))
            }
            _ => value.clone(),
        };
        out.insert(field.clone(), value);
    }
    Value::Mapping(out)
}

fn filter_projects(projects: &Mapping, kept: &PnpmLockfile) -> Mapping {
    let mut out: Vec<(Value, Value)> = projects
        .iter()
        .filter(|(id, _)| {
            id.as_str()
                .is_some_and(|id| kept.importers.contains_key(id))
        })
        .map(|(id, project)| (id.clone(), project.clone()))
        .collect();
    for id in kept.importers.keys() {
        if projects.contains_key(id.as_str()) {
            continue;
        }
        let at = out
            .iter()
            .position(|(existing, _)| {
                existing
                    .as_str()
                    .is_some_and(|existing| existing > id.as_str())
            })
            .unwrap_or(out.len());
        out.insert(
            at,
            (Value::from(id.as_str()), Value::Mapping(Mapping::new())),
        );
    }
    out.into_iter().collect()
}

fn filter_package(package: &Value, elements: &[usize]) -> Value {
    let Value::Mapping(map) = package else {
        return package.clone();
    };
    let mut map = map.clone();
    if let Some(Value::Sequence(snapshots)) = map.get_mut("snapshots") {
        *snapshots = elements
            .iter()
            .filter_map(|&element| snapshots.get(element).cloned())
            .collect();
    }
    Value::Mapping(map)
}

fn filter_settings(settings: &Mapping, kept: &PnpmLockfile) -> Mapping {
    let mut out = Mapping::new();
    for (field, value) in settings {
        let value = match (field.as_str(), value) {
            (Some("catalogs"), Value::Mapping(catalogs)) => {
                let filtered: Mapping = catalogs
                    .iter()
                    .filter_map(|(catalog, entries)| {
                        let kept_entries = kept.catalogs.as_ref()?.get(catalog.as_str()?)?;
                        let Value::Mapping(entries) = entries else {
                            return None;
                        };
                        let entries: Mapping = entries
                            .iter()
                            .filter(|(name, _)| {
                                name.as_str()
                                    .is_some_and(|name| kept_entries.contains_key(name))
                            })
                            .map(|(name, entry)| (name.clone(), entry.clone()))
                            .collect();
                        (!entries.is_empty()).then(|| (catalog.clone(), Value::Mapping(entries)))
                    })
                    .collect();
                if filtered.is_empty() {
                    continue;
                }
                Value::Mapping(filtered)
            }
            (Some("patchedDependencies"), Value::Mapping(patches)) => Value::Mapping(
                patches
                    .iter()
                    .filter(|(key, _)| {
                        key.as_str().is_some_and(|key| {
                            kept.patched_dependencies
                                .as_ref()
                                .is_some_and(|patches| patches.contains_key(key))
                        })
                    })
                    .map(|(key, patch)| (key.clone(), patch.clone()))
                    .collect(),
            ),
            _ => value.clone(),
        };
        out.insert(field.clone(), value);
    }
    out
}

fn write_unimplied_versions(
    pruned: &mut Value,
    original: &Resolved<'_, '_>,
) -> Result<(), NextFormatError> {
    let explicit: Vec<(String, &'static str, String, String)> = {
        let doc = Document::parse(pruned)?;
        let implied = Implied::of(doc.settings.catalogs.as_ref(), &doc.packages, &doc.projects);
        let mut explicit = Vec::new();
        for project in &doc.projects {
            let Some(before) = original
                .graph
                .importers
                .iter()
                .find(|importer| importer.id == project.id)
            else {
                continue;
            };
            for (section, deps) in &project.sections {
                for (alias, dep) in deps.iter().filter(|(_, dep)| dep.version.is_none()) {
                    let version = before
                        .resolved_dependencies()
                        .find(|(name, _)| name == alias)
                        .map(|(_, dep)| dep.version.clone())
                        .ok_or_else(|| wire::syntax(format!("{} lost {alias}", project.id)))?;
                    if implied.version(project.id, alias, dep.specifier).as_ref() != Some(&version)
                    {
                        explicit.push((
                            project.id.to_string(),
                            section.key(),
                            alias.to_string(),
                            version,
                        ));
                    }
                }
            }
        }
        explicit
    };
    for (id, section, alias, version) in explicit {
        let dep = pruned
            .get_mut("projects")
            .and_then(|projects| projects.get_mut(id.as_str()))
            .and_then(|project| project.get_mut(section))
            .and_then(|deps| deps.get_mut(alias.as_str()))
            .ok_or_else(|| wire::syntax(format!("{id} lost {alias}")))?;
        let mut written = Mapping::new();
        match std::mem::take(dep) {
            Value::Mapping(map) => {
                for (field, value) in map {
                    if field.as_str() == Some("with") {
                        written.insert("version".into(), version.as_str().into());
                    }
                    written.insert(field, value);
                }
                if !written.contains_key("version") {
                    written.insert("version".into(), version.as_str().into());
                }
            }
            specifier => {
                written.insert("specifier".into(), specifier);
                written.insert("version".into(), version.as_str().into());
            }
        }
        *dep = Value::Mapping(written);
    }
    Ok(())
}

fn origins(
    original: &Resolved<'_, '_>,
    filtered: &Resolved<'_, '_>,
    retained: &[bool],
) -> Vec<Sid> {
    let mut origin = vec![0; filtered.keys.len()];
    for (&package, &(start, count)) in &filtered.graph.by_package {
        let (original_start, original_count) = original.graph.by_package[package];
        let kept: Vec<Sid> = (original_start..original_start + original_count)
            .filter(|&sid| retained[sid])
            .collect();
        origin[start..start + count].copy_from_slice(&kept[..count]);
    }
    origin
}

fn pins_for_drift(
    original: &Resolved<'_, '_>,
    filtered: &Resolved<'_, '_>,
    origin: &[Sid],
) -> Result<Vec<Pin>, NextFormatError> {
    let inverse: HashMap<Sid, Sid> = origin
        .iter()
        .enumerate()
        .map(|(filtered_sid, &original_sid)| (original_sid, filtered_sid))
        .collect();
    let original_key = |sid: Sid| original.keys[sid].as_str();
    let mut pins = Vec::new();
    let mut providers = Providers::AllKept;

    for (sid, &was) in origin.iter().enumerate() {
        if original.base_keys[was] != filtered.base_keys[sid] {
            return Err(not_representable(format!(
                "{} would be read back as {}",
                original.base_keys[was], filtered.base_keys[sid]
            )));
        }
        let children: Vec<(&str, Sid)> = filtered.graph.children[sid]
            .iter()
            .map(|&(alias, child)| (alias, origin[child]))
            .collect();
        if children != original.graph.children[was] {
            return Err(not_representable(format!(
                "the dependencies of {} would be read back differently",
                original_key(was)
            )));
        }
        for (name, now) in &filtered.graph.providers[sid] {
            let before = original.graph.providers[was]
                .iter()
                .find(|(bound, _)| bound == name)
                .map(|(_, provider)| provider);
            match (before, now) {
                (Some(Provider::Snapshot(before)), Provider::Snapshot(now))
                    if origin[*now] == *before => {}
                (Some(Provider::Snapshot(before)), Provider::Snapshot(_)) => {
                    let Some(&want) = inverse.get(before) else {
                        return Err(not_representable(format!(
                            "{} binds {name} to {}, which the pruned lockfile does not keep",
                            original_key(was),
                            original_key(*before)
                        )));
                    };
                    pins.push(Pin::of(filtered, sid, name, want)?);
                }
                (Some(Provider::Snapshot(before)), Provider::Missing(_))
                    if !inverse.contains_key(before) =>
                {
                    providers = Providers::SomeDropped;
                }
                (Some(before), now) if before == now => {}
                _ => {
                    return Err(not_representable(format!(
                        "{} would bind {name} to another provider",
                        original_key(was)
                    )));
                }
            }
        }
    }

    for importer in &filtered.graph.importers {
        let children: Vec<(&str, Sid)> = filtered.graph.importer_children[importer.id]
            .iter()
            .map(|&(alias, child)| (alias, origin[child]))
            .collect();
        let before = original
            .graph
            .importer_children
            .get(importer.id)
            .map_or(&[][..], Vec::as_slice);
        if children != before {
            return Err(not_representable(format!(
                "the dependencies of {} would be read back differently",
                importer.id
            )));
        }
    }

    if pins.is_empty() {
        check_keys(original, filtered, origin, providers)?;
    }
    Ok(pins)
}

/// Whether every provider the original bound is still in the pruned
/// lockfile. A provider that only `transitivePeerDependencies` names can be
/// pruned without changing what is installed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Providers {
    AllKept,
    SomeDropped,
}

fn check_keys(
    original: &Resolved<'_, '_>,
    filtered: &Resolved<'_, '_>,
    origin: &[Sid],
    providers: Providers,
) -> Result<(), NextFormatError> {
    let changed: Vec<(Sid, Sid)> = origin
        .iter()
        .enumerate()
        .filter(|&(sid, &was)| original.keys[was] != filtered.keys[sid])
        .map(|(sid, &was)| (sid, was))
        .collect();
    let Some(&(sid, was)) = changed.first() else {
        return Ok(());
    };
    let cycles_changed = original.graph.cyclic_peer_names() != filtered.graph.cyclic_peer_names();
    if cycles_changed && providers == Providers::AllKept {
        tracing::debug!(
            "pruned pnpm v13 lockfile renames {} snapshots after a peer cycle was removed",
            changed.len()
        );
        return Ok(());
    }
    Err(not_representable(format!(
        "{} would be read back as {}",
        original.keys[was], filtered.keys[sid]
    )))
}

/// `with` is written as the provider writes its bindings because pnpm
/// compares them literally.
struct Pin {
    package: String,
    target: PinTarget,
    name: String,
    with: Value,
}

impl Pin {
    fn of(
        filtered: &Resolved<'_, '_>,
        sid: Sid,
        name: &str,
        want: Sid,
    ) -> Result<Self, NextFormatError> {
        let graph = &filtered.graph;
        let (start, count) = graph.by_package[graph.keys[want]];
        let bound: Vec<BTreeMap<&str, &wire::NextRef<'_>>> = (start..start + count)
            .map(|candidate| graph.entry(candidate).bindings().collect())
            .collect();
        let names: BTreeSet<&str> = bound
            .iter()
            .flat_map(|bindings| bindings.keys().copied())
            .collect();
        let wanted = &bound[want - start];
        let with: Bindings<'_> = names
            .into_iter()
            .filter(|name| {
                bound
                    .iter()
                    .any(|bindings| bindings.get(name) != wanted.get(name))
            })
            .filter_map(|name| Some((name, (*wanted.get(name)?).clone())))
            .collect();
        if graph.snapshot_with_bindings(start, count, &with) != Some(want) {
            return Err(not_representable(format!(
                "{} binds {name} to {}, which its bindings do not tell apart from the other \
                 snapshots of {}",
                filtered.keys[sid], filtered.keys[want], graph.keys[want]
            )));
        }
        let (package, element) = graph.slots[sid];
        let package = &graph.doc.packages[package];
        let has_list = package
            .map
            .get("snapshots")
            .and_then(Value::as_sequence)
            .is_some_and(|list| !list.is_empty());
        Ok(Pin {
            package: package.key.to_string(),
            target: if has_list {
                PinTarget::Element(element)
            } else {
                PinTarget::Entry
            },
            name: name.to_string(),
            with: wire::bindings_value(&with),
        })
    }

    fn apply(&self, tree: &mut Value) -> Result<(), NextFormatError> {
        let package = tree
            .get_mut("packages")
            .and_then(|packages| packages.get_mut(self.package.as_str()))
            .and_then(Value::as_mapping_mut)
            .ok_or_else(|| wire::syntax(format!("packages[{}] is missing", self.package)))?;
        let PinTarget::Element(element) = self.target else {
            return set_with(package, &self.name, &self.with);
        };
        if !binds(element_of(package, element)?, &self.name) {
            dehoist(package, &self.name)?;
        }
        set_with(element_of(package, element)?, &self.name, &self.with)
    }
}

enum PinTarget {
    /// The package has one snapshot, written on the package entry.
    Entry,
    /// An element of the package's `snapshots` list.
    Element(usize),
}

fn element_of(package: &mut Mapping, element: usize) -> Result<&mut Mapping, NextFormatError> {
    package
        .get_mut("snapshots")
        .and_then(|list| list.get_mut(element))
        .and_then(Value::as_mapping_mut)
        .ok_or_else(|| wire::syntax("a pinned snapshot is missing"))
}

fn binds(map: &Mapping, name: &str) -> bool {
    map.get("peerDependencies")
        .and_then(|peers| peers.get(name))
        .and_then(|entry| entry.get("version"))
        .is_some()
}

/// The order pnpm writes a peer entry's fields in.
const PEER_FIELDS: [&str; 5] = ["specifier", "version", "with", "optional", "below"];

fn set_with(map: &mut Mapping, name: &str, with: &Value) -> Result<(), NextFormatError> {
    let entry = map
        .get_mut("peerDependencies")
        .and_then(|peers| peers.get_mut(name))
        .and_then(Value::as_mapping_mut)
        .filter(|entry| entry.contains_key("version"))
        .ok_or_else(|| wire::syntax(format!("the binding of {name} to pin is missing")))?;
    entry.insert("with".into(), with.clone());
    *entry = ordered(std::mem::take(entry), &PEER_FIELDS);
    Ok(())
}

fn dehoist(package: &mut Mapping, name: &str) -> Result<(), NextFormatError> {
    let peers = package
        .get_mut("peerDependencies")
        .and_then(Value::as_mapping_mut)
        .ok_or_else(|| wire::syntax(format!("the shared binding of {name} is missing")))?;
    let Some(Value::Mapping(mut shared)) = peers.shift_remove(name) else {
        return Err(wire::syntax(format!(
            "the shared binding of {name} is missing"
        )));
    };
    let mut binding = Mapping::new();
    for field in ["version", "with", "below"] {
        if let Some(value) = shared.shift_remove(field) {
            binding.insert(field.into(), value);
        }
    }
    let declared = match (shared.get("specifier"), shared.len()) {
        (_, 0) => None,
        (Some(specifier), 1) => Some(specifier.clone()),
        _ => Some(Value::Mapping(shared)),
    };
    if let Some(declared) = declared {
        insert_sorted(peers, name, declared);
    }
    if peers.is_empty() {
        package.shift_remove("peerDependencies");
    }
    let Some(Value::Sequence(elements)) = package.get_mut("snapshots") else {
        return Err(wire::syntax(
            "a package with a shared binding to pin has no snapshots",
        ));
    };
    for element in elements.iter_mut().filter_map(Value::as_mapping_mut) {
        if !element.contains_key("peerDependencies") {
            element.insert("peerDependencies".into(), Value::Mapping(Mapping::new()));
            *element = ordered(std::mem::take(element), &PACKAGE_FIELDS);
        }
        if let Some(Value::Mapping(element_peers)) = element.get_mut("peerDependencies") {
            insert_sorted(element_peers, name, Value::Mapping(binding.clone()));
        }
    }
    Ok(())
}

/// pnpm's `NEXT_PACKAGE_KEYS`: the order of a package entry's and a
/// snapshot's fields.
const PACKAGE_FIELDS: [&str; 17] = [
    "resolution",
    "integrity",
    "version",
    "engines",
    "cpu",
    "os",
    "libc",
    "hasBin",
    "prepare",
    "bundledDependencies",
    "peerDependencies",
    "specifiers",
    "id",
    "optional",
    "dependencies",
    "optionalDependencies",
    "snapshots",
];

fn ordered(map: Mapping, priority: &[&str]) -> Mapping {
    let rank = |key: &Value| {
        key.as_str()
            .and_then(|key| priority.iter().position(|field| *field == key))
            .unwrap_or(priority.len())
    };
    let mut entries: Vec<(Value, Value)> = map.into_iter().collect();
    entries.sort_by_key(|(key, _)| rank(key));
    entries.into_iter().collect()
}

fn insert_sorted(map: &mut Mapping, name: &str, value: Value) {
    let mut entries: Vec<(Value, Value)> = std::mem::take(map).into_iter().collect();
    let at = entries
        .iter()
        .position(|(key, _)| key.as_str().is_some_and(|key| key > name))
        .unwrap_or(entries.len());
    entries.insert(at, (Value::from(name), value));
    *map = entries.into_iter().collect();
}
