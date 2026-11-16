//! Port of pnpm v12.20.0 `next_format/read.rs`, `read/choose.rs` and
//! `read/scope.rs`: matching every reference to one snapshot, every peer
//! binding to its provider, and rebuilding each snapshot's peer-suffixed key.
//! Function names and tie-breaks follow pnpm so the two can be diffed.

mod keys;
mod unbound;

use std::collections::HashMap;

pub(super) use unbound::UnboundPeers;

use super::{
    super::super::NextFormatError,
    implied_version::Implied,
    key,
    wire::{Bindings, Document, NextRef, Section, SnapshotEntry},
};

pub(super) type Sid = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Referrer<'a> {
    Importer(&'a str),
    Snapshot(Sid),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Provider {
    Snapshot(Sid),
    Link(String),
    Missing(String),
}

pub(super) struct ResolvedDep<'a> {
    pub specifier: &'a str,
    pub version: String,
    pub with: Option<Bindings<'a>>,
}

pub(super) struct Importer<'a> {
    pub id: &'a str,
    pub sections: Vec<(Section, Vec<(&'a str, ResolvedDep<'a>)>)>,
}

impl<'a> Importer<'a> {
    pub fn resolved_dependencies(&self) -> impl Iterator<Item = (&'a str, &ResolvedDep<'a>)> {
        self.sections
            .iter()
            .flat_map(|(_, deps)| deps.iter().map(|(alias, dep)| (*alias, dep)))
    }
}

fn resolve_importers<'a>(doc: &Document<'a>) -> Result<Vec<Importer<'a>>, NextFormatError> {
    let implied = Implied::of(doc.settings.catalogs.as_ref(), &doc.packages, &doc.projects);
    doc.projects
        .iter()
        .map(|project| {
            let sections = project
                .sections
                .iter()
                .map(|(section, deps)| {
                    let deps = deps
                        .iter()
                        .map(|(alias, dep)| {
                            let version = match dep.version {
                                Some(version) => version.to_string(),
                                None => implied
                                    .version(project.id, alias, dep.specifier)
                                    .ok_or_else(|| NextFormatError::UnderivableVersion {
                                        importer: project.id.to_string(),
                                        name: alias.to_string(),
                                        specifier: dep.specifier.to_string(),
                                    })?,
                            };
                            Ok((
                                *alias,
                                ResolvedDep {
                                    specifier: dep.specifier,
                                    version,
                                    with: dep.with.clone(),
                                },
                            ))
                        })
                        .collect::<Result<Vec<_>, NextFormatError>>()?;
                    Ok((*section, deps))
                })
                .collect::<Result<Vec<_>, NextFormatError>>()?;
            Ok(Importer {
                id: project.id,
                sections,
            })
        })
        .collect()
}

pub(super) struct Graph<'g, 'a> {
    pub doc: &'g Document<'a>,
    pub keys: Vec<&'a str>,
    pub names: Vec<&'a str>,
    pub slots: Vec<(usize, usize)>,
    pub by_package: HashMap<&'a str, (Sid, usize)>,
    pub importers: Vec<Importer<'a>>,
    importer_index: HashMap<&'a str, usize>,
    pub children: Vec<Vec<(&'a str, Sid)>>,
    pub importer_children: HashMap<&'a str, Vec<(&'a str, Sid)>>,
    pub parent: Vec<Option<Referrer<'a>>>,
    pub providers: Vec<Vec<(&'a str, Provider)>>,
}

impl<'g, 'a> Graph<'g, 'a> {
    pub fn new(doc: &'g Document<'a>) -> Result<Self, NextFormatError> {
        let importers = resolve_importers(doc)?;
        let mut graph = Graph::index(doc, importers);
        graph.children = (0..graph.keys.len())
            .map(|sid| graph.choose_children(sid))
            .collect();
        graph.importer_children = graph
            .importers
            .iter()
            .map(|importer| (importer.id, graph.choose_importer_children(importer)))
            .collect();
        graph.parent = graph.parents();
        graph.providers = (0..graph.keys.len())
            .map(|sid| {
                graph
                    .entry(sid)
                    .bindings()
                    .map(|(name, reference)| (name, graph.provider(sid, name, reference)))
                    .collect()
            })
            .collect();
        Ok(graph)
    }

    fn index(doc: &'g Document<'a>, importers: Vec<Importer<'a>>) -> Self {
        let mut keys = Vec::new();
        let mut names = Vec::new();
        let mut slots = Vec::new();
        let mut by_package = HashMap::with_capacity(doc.packages.len());
        for (index, package) in doc.packages.iter().enumerate() {
            by_package.insert(package.key, (keys.len(), package.snapshots.len()));
            for element in 0..package.snapshots.len() {
                keys.push(package.key);
                names.push(package.name);
                slots.push((index, element));
            }
        }
        let importer_index = importers
            .iter()
            .enumerate()
            .map(|(index, importer)| (importer.id, index))
            .collect();
        Graph {
            doc,
            keys,
            names,
            slots,
            by_package,
            importers,
            importer_index,
            children: Vec::new(),
            importer_children: HashMap::new(),
            parent: Vec::new(),
            providers: Vec::new(),
        }
    }

    pub fn entry(&self, sid: Sid) -> &'g SnapshotEntry<'a> {
        let (package, element) = self.slots[sid];
        &self.doc.packages[package].snapshots[element]
    }

    pub fn importer(&self, id: &str) -> &Importer<'a> {
        &self.importers[self.importer_index[id]]
    }

    fn choose_children(&self, sid: Sid) -> Vec<(&'a str, Sid)> {
        let entry = self.entry(sid);
        let mut children: Vec<(&'a str, Sid)> = entry
            .dependencies
            .iter()
            .chain(entry.optional_dependencies.iter())
            .flatten()
            .filter_map(|(alias, reference)| {
                let key = reference.resolve(alias)?;
                Some((
                    *alias,
                    self.choose(&key, reference.peers(), Referrer::Snapshot(sid))?,
                ))
            })
            .collect();
        children.sort_unstable_by(|left, right| key::compare_names(left.0, right.0));
        children
    }

    fn choose_importer_children(&self, importer: &Importer<'a>) -> Vec<(&'a str, Sid)> {
        let mut children: Vec<(&'a str, Sid)> = importer
            .resolved_dependencies()
            .filter_map(|(alias, dep)| {
                let key = key::resolve(alias, &dep.version)?;
                Some((
                    alias,
                    self.choose(&key, dep.with.as_ref(), Referrer::Importer(importer.id))?,
                ))
            })
            .collect();
        children.sort_unstable_by(|left, right| key::compare_names(left.0, right.0));
        children
    }

    fn choose(
        &self,
        key: &str,
        peers: Option<&Bindings<'a>>,
        referrer: Referrer<'a>,
    ) -> Option<Sid> {
        let &(start, count) = self.by_package.get(key::package_key(key))?;
        if count == 1 {
            return Some(start);
        }
        if let Some(peers) = peers
            && let Some(sid) = self.snapshot_with_bindings(start, count, peers)
        {
            return Some(sid);
        }
        Some(self.snapshot_in_scope(start, count, referrer))
    }

    pub fn snapshot_with_bindings(
        &self,
        start: Sid,
        count: usize,
        peers: &Bindings<'a>,
    ) -> Option<Sid> {
        (start..start + count)
            .filter(|&sid| {
                peers.iter().all(|(name, wanted)| {
                    self.entry(sid).bindings().any(|(bound, reference)| {
                        bound == *name && super::wire::names_binding(wanted, reference)
                    })
                })
            })
            .min_by_key(|&sid| self.entry(sid).bindings().count())
    }

    fn snapshot_in_scope(&self, start: Sid, count: usize, referrer: Referrer<'a>) -> Sid {
        (start..start + count)
            .filter_map(|sid| Some((sid, self.agreement(sid, referrer)?)))
            .min_by_key(|&(sid, agreement)| {
                (std::cmp::Reverse(agreement.agreeing), agreement.absent, sid)
            })
            .map_or(start, |(sid, _)| sid)
    }

    fn agreement(&self, sid: Sid, referrer: Referrer<'a>) -> Option<Agreement> {
        let mut agreement = Agreement::default();
        for (name, reference) in self.entry(sid).bindings() {
            match self.binding_in_scope(referrer, name, reference) {
                Binding::Agrees => agreement.agreeing += 1,
                Binding::Absent => agreement.absent += 1,
                Binding::Disagrees => return None,
            }
        }
        Some(agreement)
    }

    fn binding_in_scope(
        &self,
        referrer: Referrer<'a>,
        name: &str,
        reference: &NextRef<'a>,
    ) -> Binding {
        let agrees = |same: bool| {
            if same {
                Binding::Agrees
            } else {
                Binding::Disagrees
            }
        };
        match referrer {
            Referrer::Snapshot(sid) => match self.entry(sid).reference_in_scope(name) {
                Some(scoped) => agrees(same_target(name, scoped, reference)),
                None if self.is_itself(sid, name, reference) => Binding::Agrees,
                None => Binding::Absent,
            },
            Referrer::Importer(id) => self
                .importer(id)
                .resolved_dependencies()
                .find(|(alias, _)| *alias == name)
                .map_or(Binding::Absent, |(_, dep)| {
                    agrees(importer_target_is(name, &dep.version, reference))
                }),
        }
    }

    fn is_itself(&self, sid: Sid, name: &str, reference: &NextRef<'a>) -> bool {
        self.names[sid] == name
            && reference
                .resolve(name)
                .is_some_and(|key| key::package_key(&key) == self.keys[sid])
    }

    fn parents(&self) -> Vec<Option<Referrer<'a>>> {
        let mut walk = Walk {
            graph: self,
            parent: vec![None; self.keys.len()],
            visited: vec![false; self.keys.len()],
            queue: Vec::new(),
        };
        let mut ids: Vec<&'a str> = self.importer_children.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            for &(_, child) in &self.importer_children[id] {
                walk.reach(child, Referrer::Importer(id));
            }
            walk.drain();
        }
        for sid in self.unreached_in_key_order(&walk.parent) {
            walk.queue.push(sid);
            walk.drain();
        }
        walk.parent
    }

    fn unreached_in_key_order(&self, parent: &[Option<Referrer<'a>>]) -> Vec<Sid> {
        let mut unreached: Vec<Sid> = (0..self.keys.len())
            .filter(|&sid| parent[sid].is_none())
            .collect();
        unreached.sort_by_key(|&sid| (self.keys[sid], sid - self.by_package[self.keys[sid]].0));
        unreached
    }

    fn provider(&self, sid: Sid, name: &str, reference: &NextRef<'a>) -> Provider {
        if let Some(path) = reference.as_link_target() {
            return Provider::Link(path.to_string());
        }
        let Some(key) = reference.resolve(name) else {
            return Provider::Missing(reference.version.to_string());
        };
        let package = key::package_key(&key);
        let Some(&(start, count)) = self.by_package.get(package) else {
            return Provider::Missing(package.to_string());
        };
        if count == 1 {
            return Provider::Snapshot(start);
        }
        if let Some(peers) = reference.peers()
            && let Some(found) = self.snapshot_with_bindings(start, count, peers)
        {
            return Provider::Snapshot(found);
        }
        match self.linked_above(sid, name, start, count) {
            Some(found) => Provider::Snapshot(found),
            None => {
                Provider::Snapshot(self.snapshot_in_scope(start, count, Referrer::Snapshot(sid)))
            }
        }
    }

    fn linked_above(&self, sid: Sid, name: &str, start: Sid, count: usize) -> Option<Sid> {
        let is_candidate = |found: Sid| (start..start + count).contains(&found);
        let mut referrer = self.parent[sid];
        let mut steps = 0;
        while let Some(current) = referrer {
            let linked = match current {
                Referrer::Snapshot(parent) => child_named(&self.children[parent], name)
                    .or_else(|| (self.names[parent] == name).then_some(parent)),
                Referrer::Importer(id) => child_named(&self.importer_children[id], name),
            };
            if let Some(found) = linked.filter(|&found| is_candidate(found)) {
                return Some(found);
            }
            referrer = match current {
                Referrer::Snapshot(parent) => self.parent[parent],
                Referrer::Importer(_) => None,
            };
            steps += 1;
            if steps > self.keys.len() {
                return None;
            }
        }
        None
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct Agreement {
    agreeing: usize,
    absent: usize,
}

enum Binding {
    Agrees,
    Disagrees,
    Absent,
}

fn same_target(name: &str, left: &NextRef<'_>, right: &NextRef<'_>) -> bool {
    match (left.as_link_target(), right.as_link_target()) {
        (Some(left), Some(right)) => left == right,
        (None, None) => {
            let target = |reference: &NextRef<'_>| {
                reference
                    .resolve(name)
                    .map(|key| key::package_key(&key).to_string())
            };
            target(left) == target(right)
        }
        _ => false,
    }
}

fn importer_target_is(name: &str, version: &str, reference: &NextRef<'_>) -> bool {
    match (key::as_link_target(version), reference.as_link_target()) {
        (Some(left), Some(right)) => left == right,
        (None, None) => {
            let own = key::resolve(name, version).map(|key| key::package_key(&key).to_string());
            own.is_some()
                && own
                    == reference
                        .resolve(name)
                        .map(|key| key::package_key(&key).to_string())
        }
        _ => false,
    }
}

pub(super) fn child_named(children: &[(&str, Sid)], name: &str) -> Option<Sid> {
    children
        .binary_search_by(|(alias, _)| key::compare_names(alias, name))
        .ok()
        .map(|index| children[index].1)
}

struct Walk<'w, 'g, 'a> {
    graph: &'w Graph<'g, 'a>,
    parent: Vec<Option<Referrer<'a>>>,
    visited: Vec<bool>,
    queue: Vec<Sid>,
}

impl<'a> Walk<'_, '_, 'a> {
    fn reach(&mut self, sid: Sid, referrer: Referrer<'a>) {
        self.parent[sid].get_or_insert(referrer);
        self.queue.push(sid);
    }

    fn drain(&mut self) {
        let graph = self.graph;
        while let Some(sid) = self.queue.pop() {
            if std::mem::replace(&mut self.visited[sid], true) {
                continue;
            }
            for &(_, child) in &graph.children[sid] {
                if !self.visited[child] {
                    self.reach(child, Referrer::Snapshot(sid));
                }
            }
        }
    }
}
