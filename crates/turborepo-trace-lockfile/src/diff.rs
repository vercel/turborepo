//! Lockfile diffs and change-impact sets.
//!
//! Two notions of "changed" matter for opaque package nodes:
//!
//! - **Identity changed**: a package in the new lockfile whose content identity
//!   (`name`, `version`, integrity — or resolution when no integrity is
//!   recorded — and patch) did not exist in the old lockfile. Peer suffixes and
//!   install locations are not identity: `foo@1.0.0(react@18)` and
//!   `foo@1.0.0(react@19)` have the same content.
//! - **Closure changed**: a package whose transitive closure contains an
//!   identity-changed package, or a package whose dependency edges, at any
//!   depth, now resolve to different content (a peer suffix change, a
//!   re-pointed nested dependency, an added or removed edge). Its runtime
//!   behavior can differ even though its own files did not change. Always a
//!   superset of the identity-changed set. Closures are compared as a whole
//!   (`Classes`): `foo@1(react@18)` and `foo@1(react@19)` have the same
//!   identity and different closures.
//!
//! An importer has a third case: its direct dependency can be **re-pointed** to
//! another instance of the same package that already existed (pnpm moves a
//! project from `vitest@4(jsdom@27)` to `vitest@4(jsdom@23)` when the peers of
//! other projects change; npm and bun do the same with nested copies). No
//! package changed, but what the importer runs did: [`ImporterDiff::affected`]
//! lists such dependencies too.
//!
//! Everything is computed from the normalized model, so diffs are stable under
//! reordering and formatting-only changes. All output lists are sorted.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
};

use bitflags::bitflags;
use fixedbitset::FixedBitSet;

use crate::{
    model::{DepEdge, DepKind, LockedPackage, Lockfile, NO_PACKAGE, Target},
    util::FxHashMap,
};

bitflags! {
    /// Which fields of a package with the same id differ.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct ChangedFields: u8 {
        /// `version`.
        const VERSION = 1;
        /// `integrity`.
        const INTEGRITY = 1 << 1;
        /// `resolution`.
        const RESOLUTION = 1 << 2;
        /// `patch`.
        const PATCH = 1 << 3;
        /// Outgoing dependency edges.
        const DEPENDENCIES = 1 << 4;
        /// Flags or platform constraints.
        const METADATA = 1 << 5;
        /// `name` (npm/bun install paths can be reused by a different package).
        const NAME = 1 << 6;
    }
}

/// A package present under the same id in both lockfiles, with differing
/// fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageChange {
    /// The shared id.
    pub id: String,
    /// Package name in the new lockfile.
    pub name: String,
    /// Old version.
    pub old_version: String,
    /// New version.
    pub new_version: String,
    /// What differs.
    pub fields: ChangedFields,
}

/// Per package name: the versions that disappeared and appeared (ids aside).
/// Pairs pnpm/yarn remove+add bumps into one readable line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionChange {
    /// Package name.
    pub name: String,
    /// Versions present only in the old lockfile.
    pub removed: Vec<String>,
    /// Versions present only in the new lockfile.
    pub added: Vec<String>,
}

/// One side of a direct dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectDep {
    /// Declared kind.
    pub kind: DepKind,
    /// Resolution target.
    pub target: Target,
    /// Resolved version when the target is a package.
    pub version: Option<String>,
}

/// A direct dependency of an importer whose resolution changed (or that was
/// added/removed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectDepChange {
    /// Dependency name.
    pub name: String,
    /// Old resolution (`None`: added).
    pub old: Option<DirectDep>,
    /// New resolution (`None`: removed).
    pub new: Option<DirectDep>,
}

/// Whether an importer exists on both sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImporterStatus {
    /// Only in the new lockfile.
    Added,
    /// Only in the old lockfile.
    Removed,
    /// In both.
    Present,
}

/// Changes for one importer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImporterDiff {
    /// Importer path.
    pub path: String,
    /// Presence.
    pub status: ImporterStatus,
    /// Direct dependencies whose resolution changed, were added or removed,
    /// sorted by name.
    pub changed: Vec<DirectDepChange>,
    /// Direct dependency names (new side) whose resolved closure is not what it
    /// was: the target is in the closure-changed set, or the dependency now
    /// points at another instance of the same package with another closure
    /// (a peer variant or a nested copy that existed before). Sorted. This
    /// is what a first-party shard key that hashes referenced dependencies
    /// needs, and what decides whether the importer's tools (a test
    /// runner and the environment it loads) run the same code.
    pub affected: Vec<String>,
}

/// The change-impact sets (see the module docs). Ids refer to the new lockfile,
/// except `removed_identities`, which lists old ids whose identity no longer
/// exists.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AffectedPackages {
    /// New packages with a new content identity.
    pub identity_changed: BTreeSet<String>,
    /// New packages whose closure changed (superset of `identity_changed`).
    pub closure_changed: BTreeSet<String>,
    /// Old packages whose content identity is gone from the new lockfile.
    pub removed_identities: BTreeSet<String>,
}

/// The difference between two lockfiles.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LockDiff {
    /// Ids only in the new lockfile.
    pub added: Vec<String>,
    /// Ids only in the old lockfile.
    pub removed: Vec<String>,
    /// Ids in both whose fields differ.
    pub changed: Vec<PackageChange>,
    /// Version-set changes per package name.
    pub version_changes: Vec<VersionChange>,
    /// Importers with direct-dependency changes or affected dependencies, or
    /// that were added or removed, sorted by path.
    pub importers: Vec<ImporterDiff>,
    affected: AffectedPackages,
}

impl LockDiff {
    /// The identity-changed and closure-changed package sets.
    pub fn affected_packages(&self) -> &AffectedPackages {
        &self.affected
    }

    /// Whether nothing changed (packages, importers or impact sets).
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.changed.is_empty()
            && self.importers.is_empty()
            && self.affected.closure_changed.is_empty()
            && self.affected.removed_identities.is_empty()
    }
}

/// Content identity of a package (see module docs).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct IdentityKey<'l> {
    name: &'l str,
    version: &'l str,
    source: Option<&'l str>,
    patch: Option<&'l str>,
}

fn identity(p: &LockedPackage) -> IdentityKey<'_> {
    IdentityKey {
        name: &p.name,
        version: &p.version,
        source: p.integrity.as_deref().or(p.resolution.as_deref()),
        patch: p.patch.as_deref(),
    }
}

/// The closure class of every package of both lockfiles (shared numbering): two
/// packages have the same class exactly when they have the same content
/// identity and their dependency edges, in order, have the same names and lead
/// to packages of the same class (or to the same importer, unresolved reference
/// or missing id). That is, their resolved dependency trees unfold to the same
/// tree of contents. Ids, install locations, peer suffixes and edge
/// kinds are not part of it, so `foo@1(react@18)` in one lockfile and
/// `foo@1(react@18)(x@2)` in the other are the same class when both resolve to
/// the same packages.
///
/// Computed by partition refinement from the identities (cycles are fine:
/// packages on a cycle stay together until something below one of them
/// differs). Keys are interned in full, never hashed down, so a collision can
/// never hide a change.
struct Classes {
    old: Vec<u32>,
    new: Vec<u32>,
}

/// One edge for [`classes`]: `(name, target kind, target)`. Kind 0 is a package
/// of the same lockfile (by index); the other kinds carry an interned string.
type ClassEdge = (u32, u8, u32);

/// The edges of every package of `lock`, with names and non-package targets
/// numbered in `strings` (shared by both lockfiles).
fn class_edges<'l>(
    lock: &'l Lockfile,
    strings: &mut FxHashMap<&'l str, u32>,
) -> Vec<Vec<ClassEdge>> {
    let mut intern = |s: &'l str| {
        let next = strings.len() as u32;
        *strings.entry(s).or_insert(next)
    };
    lock.packages()
        .iter()
        .enumerate()
        .map(|(i, p)| {
            p.dependencies
                .iter()
                .zip(lock.edge_targets(i))
                .map(|(e, &t)| {
                    let name = intern(e.name.as_str());
                    match &e.target {
                        _ if t != NO_PACKAGE => (name, 0, t),
                        Target::Package(id) => (name, 1, intern(id.as_str())),
                        Target::Importer(p) => (name, 2, intern(p.as_str())),
                        Target::Unresolved(s) => (name, 3, intern(s.as_str())),
                    }
                })
                .collect()
        })
        .collect()
}

fn classes<'l>(old: &'l Lockfile, new: &'l Lockfile, ids: &Identities) -> Classes {
    let mut strings: FxHashMap<&'l str, u32> = FxHashMap::default();
    let sides = [
        class_edges(old, &mut strings),
        class_edges(new, &mut strings),
    ];
    let mut cur = [ids.old.clone(), ids.new.clone()];
    let mut count = {
        let mut seen: Vec<u32> = cur.iter().flatten().copied().collect();
        seen.sort_unstable();
        seen.dedup();
        seen.len()
    };
    let total = cur[0].len() + cur[1].len();
    let mut table: FxHashMap<Vec<u32>, u32> = FxHashMap::default();
    let mut key: Vec<u32> = Vec::new();
    // Each round splits classes by the classes of the edge targets; it stops when a
    // round splits nothing. At most one round per package (each round but the
    // last adds a class).
    for _ in 0..=total {
        table.clear();
        let mut next = [
            Vec::with_capacity(cur[0].len()),
            Vec::with_capacity(cur[1].len()),
        ];
        for side in 0..2 {
            for (i, edges) in sides[side].iter().enumerate() {
                key.clear();
                key.push(cur[side][i]);
                for &(name, kind, target) in edges {
                    let target = if kind == 0 {
                        cur[side][target as usize]
                    } else {
                        target
                    };
                    key.extend([name, u32::from(kind), target]);
                }
                let id = table.len() as u32;
                let class = match table.get(key.as_slice()) {
                    Some(&c) => c,
                    None => {
                        table.insert(key.clone(), id);
                        id
                    }
                };
                next[side].push(class);
            }
        }
        let stable = table.len() == count;
        count = table.len();
        cur = next;
        if stable {
            break;
        }
    }
    let [old, new] = cur;
    Classes { old, new }
}

/// Interned content identities of both lockfiles' packages (shared numbering).
struct Identities {
    old: Vec<u32>,
    new: Vec<u32>,
    in_old: Vec<bool>,
    in_new: Vec<bool>,
}

fn identities(old: &Lockfile, new: &Lockfile) -> Identities {
    fn intern<'l>(
        table: &mut FxHashMap<IdentityKey<'l>, u32>,
        packages: &'l [LockedPackage],
    ) -> Vec<u32> {
        packages
            .iter()
            .map(|p| {
                let next = table.len() as u32;
                *table.entry(identity(p)).or_insert(next)
            })
            .collect()
    }
    let mut table: FxHashMap<IdentityKey<'_>, u32> = FxHashMap::default();
    table.reserve(old.packages().len() + 64);
    let old_ids = intern(&mut table, old.packages());
    let new_ids = intern(&mut table, new.packages());
    let n = table.len();
    let mut in_old = vec![false; n];
    let mut in_new = vec![false; n];
    for &i in &old_ids {
        in_old[i as usize] = true;
    }
    for &i in &new_ids {
        in_new[i as usize] = true;
    }
    Identities {
        old: old_ids,
        new: new_ids,
        in_old,
        in_new,
    }
}

/// Computes the diff from `old` to `new`.
pub fn diff(old: &Lockfile, new: &Lockfile) -> LockDiff {
    let mut out = LockDiff::default();
    let ids = identities(old, new);
    // For each new package, the old package with the same id.
    let mut same_id: Vec<Option<usize>> = vec![None; new.packages().len()];

    // Id-level added / removed / changed (both package lists are sorted by id).
    let (mut i, mut j) = (0, 0);
    let (op, np) = (old.packages(), new.packages());
    while i < op.len() || j < np.len() {
        match merge_order(op.get(i).map(|p| &p.id), np.get(j).map(|p| &p.id)) {
            Ordering::Less => {
                out.removed.push(op[i].id.clone());
                i += 1;
            }
            Ordering::Greater => {
                out.added.push(np[j].id.clone());
                j += 1;
            }
            Ordering::Equal => {
                same_id[j] = Some(i);
                let (a, b) = (&op[i], &np[j]);
                let mut fields = ChangedFields::empty();
                fields.set(ChangedFields::NAME, a.name != b.name);
                fields.set(ChangedFields::VERSION, a.version != b.version);
                fields.set(ChangedFields::INTEGRITY, a.integrity != b.integrity);
                fields.set(ChangedFields::RESOLUTION, a.resolution != b.resolution);
                fields.set(ChangedFields::PATCH, a.patch != b.patch);
                fields.set(
                    ChangedFields::DEPENDENCIES,
                    !same_edges(&a.dependencies, &b.dependencies),
                );
                fields.set(
                    ChangedFields::METADATA,
                    a.flags != b.flags || a.platform != b.platform,
                );
                if !fields.is_empty() {
                    out.changed.push(PackageChange {
                        id: b.id.clone(),
                        name: b.name.clone(),
                        old_version: a.version.clone(),
                        new_version: b.version.clone(),
                        fields,
                    });
                }
                i += 1;
                j += 1;
            }
        }
    }

    // Version sets per name.
    let mut versions: BTreeMap<&str, (BTreeSet<&str>, BTreeSet<&str>)> = BTreeMap::new();
    for p in op {
        versions.entry(&p.name).or_default().0.insert(&p.version);
    }
    for p in np {
        versions.entry(&p.name).or_default().1.insert(&p.version);
    }
    for (name, (a, b)) in versions {
        if a != b {
            out.version_changes.push(VersionChange {
                name: name.to_string(),
                removed: a.difference(&b).map(|s| s.to_string()).collect(),
                added: b.difference(&a).map(|s| s.to_string()).collect(),
            });
        }
    }

    let classes = classes(old, new, &ids);
    let (affected, reached) = affected(old, new, &ids, &classes, &same_id);
    out.affected = affected;
    out.importers = importer_diffs(old, new, &ids, &classes, &reached);
    out
}

/// Merge-join step over two sorted sequences (a missing side sorts last).
fn merge_order(a: Option<&String>, b: Option<&String>) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => a.cmp(b),
        (Some(_), None) => Ordering::Less,
        (None, _) => Ordering::Greater,
    }
}

/// Edge equality ignoring specifiers (ranges are manifest data, not
/// resolution).
fn same_edges(a: &[DepEdge], b: &[DepEdge]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| x.name == y.name && x.kind == y.kind && x.target == y.target)
}

fn affected(
    old: &Lockfile,
    new: &Lockfile,
    ids: &Identities,
    classes: &Classes,
    same_id: &[Option<usize>],
) -> (AffectedPackages, FixedBitSet) {
    // The closure classes of the old lockfile (npm/bun duplicates at several
    // install paths and pnpm peer variants are separate packages), consulted
    // for new packages whose id did not exist with this identity.
    let mut old_classes = FixedBitSet::with_capacity(
        classes
            .old
            .iter()
            .chain(&classes.new)
            .max()
            .map_or(0, |&m| m as usize + 1),
    );
    for &c in &classes.old {
        old_classes.insert(c as usize);
    }
    let mut result = AffectedPackages::default();
    let mut seeds = Vec::new();
    for (j, p) in new.packages().iter().enumerate() {
        let ident = ids.new[j];
        if !ids.in_old[ident as usize] {
            result.identity_changed.insert(p.id.clone());
            seeds.push(j);
            continue;
        }
        // The same id with the same identity must unfold to the same closure as before
        // (its edges may be re-pointed to another variant of the same package);
        // a new id, or an id that held another package, is unchanged when some
        // old copy of this package had this closure.
        let unchanged = match same_id[j].filter(|&i| ids.old[i] == ident) {
            Some(i) => classes.old[i] == classes.new[j],
            None => old_classes.contains(classes.new[j] as usize),
        };
        if !unchanged {
            seeds.push(j);
        }
    }
    for (i, p) in old.packages().iter().enumerate() {
        if !ids.in_new[ids.old[i] as usize] {
            result.removed_identities.insert(p.id.clone());
        }
    }
    let reached = new.reach(&seeds, true);
    result.closure_changed = reached
        .ones()
        .map(|i| new.packages()[i].id.clone())
        .collect();
    (result, reached)
}

fn direct(lock: &Lockfile, e: &DepEdge, target: u32) -> DirectDep {
    let version = (target != NO_PACKAGE).then(|| lock.packages()[target as usize].version.clone());
    DirectDep {
        kind: e.kind,
        target: e.target.clone(),
        version,
    }
}

fn key(e: &DepEdge) -> (&str, DepKind) {
    (e.name.as_str(), e.kind)
}

// Invariant: `b` is None only for an importer that exists on the old side, so
// `a` is Some.
#[allow(clippy::unwrap_used)]
fn importer_diffs(
    old: &Lockfile,
    new: &Lockfile,
    ids: &Identities,
    classes: &Classes,
    reached: &FixedBitSet,
) -> Vec<ImporterDiff> {
    let (oi, ni) = (old.importers(), new.importers());
    let mut out = Vec::new();
    // Merge-join on path (both sorted).
    let (mut i, mut j) = (0, 0);
    while i < oi.len() || j < ni.len() {
        let (a, b, status) =
            match merge_order(oi.get(i).map(|x| &x.path), ni.get(j).map(|x| &x.path)) {
                Ordering::Less => {
                    i += 1;
                    (Some(i - 1), None, ImporterStatus::Removed)
                }
                Ordering::Greater => {
                    j += 1;
                    (None, Some(j - 1), ImporterStatus::Added)
                }
                Ordering::Equal => {
                    i += 1;
                    j += 1;
                    (Some(i - 1), Some(j - 1), ImporterStatus::Present)
                }
            };
        let path = b.map_or_else(|| oi[a.unwrap()].path.clone(), |b| ni[b].path.clone());
        let (a_edges, a_targets) = a.map_or((&[][..], &[][..]), |x| {
            (&oi[x].dependencies[..], old.importer_edge_targets(x))
        });
        let (b_edges, b_targets) = b.map_or((&[][..], &[][..]), |x| {
            (&ni[x].dependencies[..], new.importer_edge_targets(x))
        });
        // Edges are sorted by (name, kind): merge-join on that key. A dependency moving
        // between kinds is reported as removed under one kind and added under
        // the other.
        let mut changed = Vec::new();
        // New-side edges that lead to the same package as before through another
        // closure.
        let mut repointed = FixedBitSet::with_capacity(b_edges.len());
        let (mut x, mut y) = (0, 0);
        while x < a_edges.len() || y < b_edges.len() {
            let ord = match (a_edges.get(x), b_edges.get(y)) {
                (Some(ea), Some(eb)) => key(ea).cmp(&key(eb)),
                (Some(_), None) => Ordering::Less,
                (None, _) => Ordering::Greater,
            };
            let (pa, pb) = match ord {
                Ordering::Less => (Some(x), None),
                Ordering::Greater => (None, Some(y)),
                Ordering::Equal => (Some(x), Some(y)),
            };
            x += usize::from(pa.is_some());
            y += usize::from(pb.is_some());
            // Same content identity counts as the same resolution even if the id moved (npm
            // hoisting, pnpm peer suffix); those show up in `affected` if the closure
            // changed.
            let same = match (pa, pb) {
                (Some(pa), Some(pb)) => {
                    let (ta, tb) = (a_targets[pa], b_targets[pb]);
                    if ta != NO_PACKAGE && tb != NO_PACKAGE {
                        let same = ids.old[ta as usize] == ids.new[tb as usize];
                        if same && classes.old[ta as usize] != classes.new[tb as usize] {
                            repointed.insert(pb);
                        }
                        same
                    } else {
                        a_edges[pa].target == b_edges[pb].target
                    }
                }
                _ => false,
            };
            if !same {
                let name = match (pa, pb) {
                    (Some(k), _) => a_edges[k].name.clone(),
                    (None, Some(k)) => b_edges[k].name.clone(),
                    (None, None) => unreachable!("merge-join yields at least one side"),
                };
                changed.push(DirectDepChange {
                    name,
                    old: pa.map(|k| direct(old, &a_edges[k], a_targets[k])),
                    new: pb.map(|k| direct(new, &b_edges[k], b_targets[k])),
                });
            }
        }
        let mut affected: Vec<String> = b_edges
            .iter()
            .zip(b_targets)
            .enumerate()
            .filter(|&(k, (_, &t))| {
                repointed.contains(k) || (t != NO_PACKAGE && reached.contains(t as usize))
            })
            .map(|(_, (e, _))| e.name.clone())
            .collect();
        affected.dedup();
        if status != ImporterStatus::Present || !changed.is_empty() || !affected.is_empty() {
            out.push(ImporterDiff {
                path,
                status,
                changed,
                affected,
            });
        }
    }
    out
}
