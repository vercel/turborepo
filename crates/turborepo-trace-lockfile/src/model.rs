//! The format-neutral lockfile model and its query API.

use std::{collections::BTreeMap, fmt, sync::OnceLock};

use bitflags::bitflags;
use fixedbitset::FixedBitSet;

use crate::util::FxHashMap;

/// Which package manager wrote a lockfile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LockfileFormat {
    /// `pnpm-lock.yaml`.
    Pnpm,
    /// `package-lock.json` / `npm-shrinkwrap.json`.
    Npm,
    /// Yarn classic (v1) `yarn.lock`.
    YarnClassic,
    /// Yarn berry (v2+) `yarn.lock`.
    YarnBerry,
    /// Bun text lockfile `bun.lock`.
    Bun,
    /// Go modules: `go.mod` plus `go.sum` (see [`crate::gosum`]).
    Go,
    /// Cargo `Cargo.lock` (see [`crate::cargo`]).
    Cargo,
}

impl fmt::Display for LockfileFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Pnpm => "pnpm",
            Self::Npm => "npm",
            Self::YarnClassic => "yarn-classic",
            Self::YarnBerry => "yarn-berry",
            Self::Bun => "bun",
            Self::Go => "go",
            Self::Cargo => "cargo",
        })
    }
}

/// How a dependency is declared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DepKind {
    /// `dependencies`.
    Prod,
    /// `devDependencies` (importers only).
    Dev,
    /// `optionalDependencies`, or a dependency marked optional in metadata.
    Optional,
    /// `peerDependencies`.
    Peer,
}

impl DepKind {
    /// Whether this kind is part of the runtime closure (everything except
    /// `Dev`).
    pub fn is_runtime(self) -> bool {
        self != Self::Dev
    }
}

/// What a dependency edge points to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Target {
    /// A locked package, by [`LockedPackage::id`].
    Package(String),
    /// A workspace package (`link:`/`workspace:`), by normalized
    /// [`Importer::path`].
    Importer(String),
    /// Not resolved by the lockfile (for example an unmet optional peer). Holds
    /// the declared range.
    Unresolved(String),
}

impl Target {
    /// The package id, if this edge resolves to a locked package.
    pub fn package_id(&self) -> Option<&str> {
        match self {
            Self::Package(id) => Some(id),
            _ => None,
        }
    }
}

/// One dependency edge: `name` (the name it is required by) resolves to
/// `target`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DepEdge {
    /// The dependency name as written in the dependent's manifest (an alias
    /// name for aliases).
    pub name: String,
    /// How the dependency is declared.
    pub kind: DepKind,
    /// What it resolves to.
    pub target: Target,
    /// The declared range or specifier, where the lockfile records it.
    pub specifier: Option<String>,
}

bitflags! {
    /// Install-time flags recorded for a locked package.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
    pub struct PackageFlags: u8 {
        /// Only reachable through optional dependencies.
        const OPTIONAL = 1;
        /// Only reachable through dev dependencies.
        const DEV = 1 << 1;
        /// Only reachable through peer dependencies (npm `peer: true`).
        const PEER = 1 << 2;
        /// Shipped inside another package's tarball (npm `inBundle`).
        const BUNDLED = 1 << 3;
    }
}

/// Platform constraints of a package. Empty lists mean "any".
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct Platform {
    /// `os` values (may include `!negations`).
    pub os: Vec<String>,
    /// `cpu` values.
    pub cpu: Vec<String>,
    /// `libc` values.
    pub libc: Vec<String>,
}

impl Platform {
    /// Whether no constraint is recorded.
    pub fn is_empty(&self) -> bool {
        self.os.is_empty() && self.cpu.is_empty() && self.libc.is_empty()
    }
}

/// One resolved package instance in the lockfile.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LockedPackage {
    /// Unique key within the lockfile. Format specific:
    /// pnpm: snapshot key including peer and patch suffixes
    /// (`react-dom@18.3.1(react@18.3.1)`, leading `/` of v6 stripped); npm:
    /// install path (`node_modules/a/node_modules/b`); yarn classic:
    /// `name@version`; yarn berry: the resolution locator (`ms@npm:2.1.3`);
    /// bun: install path (`send/ms`).
    pub id: String,
    /// Package name (the real name, not an alias).
    pub name: String,
    /// Resolved version (empty if the lockfile records none).
    pub version: String,
    /// Content hash (`sha512-…`, yarn berry checksum), if recorded.
    pub integrity: Option<String>,
    /// Where it comes from when not implied by name and version: tarball URL,
    /// git URL, directory.
    pub resolution: Option<String>,
    /// Patch hash for patched dependencies.
    pub patch: Option<String>,
    /// Outgoing edges, sorted by `(name, kind)`.
    pub dependencies: Vec<DepEdge>,
    /// Install flags.
    pub flags: PackageFlags,
    /// Platform constraints.
    pub platform: Platform,
}

impl LockedPackage {
    /// A new package with only identity fields set.
    pub fn new(id: impl Into<String>, name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            version: version.into(),
            integrity: None,
            resolution: None,
            patch: None,
            dependencies: Vec::new(),
            flags: PackageFlags::empty(),
            platform: Platform::default(),
        }
    }

    /// The edge for dependency `name` (first by kind order if declared twice).
    pub fn dependency(&self, name: &str) -> Option<&DepEdge> {
        find_edge(&self.dependencies, name)
    }
}

/// A workspace package (pnpm importer, npm workspace, yarn/bun workspace) and
/// its direct dependencies.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Importer {
    /// Path relative to the lockfile directory, `/`-separated; `.` for the
    /// root.
    pub path: String,
    /// Package name, where the lockfile records it.
    pub name: Option<String>,
    /// Direct dependencies of every kind, sorted by `(name, kind)`.
    pub dependencies: Vec<DepEdge>,
}

impl Importer {
    /// A new importer with no dependencies.
    pub fn new(path: &str) -> Self {
        Self {
            path: normalize_importer_path(path),
            name: None,
            dependencies: Vec::new(),
        }
    }

    /// The edge for dependency `name` (prod before dev before optional before
    /// peer).
    pub fn dependency(&self, name: &str) -> Option<&DepEdge> {
        find_edge(&self.dependencies, name)
    }

    /// Direct dependencies of one kind.
    pub fn dependencies_of_kind(&self, kind: DepKind) -> impl Iterator<Item = &DepEdge> {
        self.dependencies.iter().filter(move |d| d.kind == kind)
    }
}

fn find_edge<'e>(edges: &'e [DepEdge], name: &str) -> Option<&'e DepEdge> {
    let start = edges.partition_point(|e| e.name.as_str() < name);
    edges.get(start).filter(|e| e.name == name)
}

/// One catalog entry (pnpm catalogs).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CatalogEntry {
    /// The catalog's declared range.
    pub specifier: String,
    /// The version it resolved to.
    pub version: String,
}

/// Lockfile-level metadata that is recorded but does not feed resolution.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LockfileMeta {
    /// Overrides / resolutions, selector → replacement (record only).
    pub overrides: BTreeMap<String, String>,
    /// pnpm catalogs: catalog name → dependency name → entry.
    pub catalogs: BTreeMap<String, BTreeMap<String, CatalogEntry>>,
    /// Patched dependencies: selector (`name@version` or `name`) → patch hash.
    pub patched_dependencies: BTreeMap<String, String>,
}

/// A parsed lockfile in the neutral model. Packages are sorted by id, importers
/// by path.
#[derive(Clone)]
pub struct Lockfile {
    format: LockfileFormat,
    version: String,
    packages: Vec<LockedPackage>,
    importers: Vec<Importer>,
    meta: LockfileMeta,
    /// Yarn descriptors (`name@range`) → package id, for resolving manifest
    /// ranges.
    descriptors: BTreeMap<String, String>,
    graph: Graph,
}

impl PartialEq for Lockfile {
    fn eq(&self, other: &Self) -> bool {
        self.format == other.format
            && self.version == other.version
            && self.packages == other.packages
            && self.importers == other.importers
            && self.meta == other.meta
            && self.descriptors == other.descriptors
    }
}
impl Eq for Lockfile {}

impl fmt::Debug for Lockfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lockfile")
            .field("format", &self.format)
            .field("version", &self.version)
            .field("packages", &self.packages)
            .field("importers", &self.importers)
            .field("meta", &self.meta)
            .finish_non_exhaustive()
    }
}

/// CSR adjacency over package indices (package → package edges only), plus a
/// lazily built reverse adjacency.
#[derive(Clone, Default)]
struct Graph {
    offsets: Vec<u32>,
    targets: Vec<u32>,
    /// Per package, the package index of each `DepEdge` (in order), or
    /// [`NO_PACKAGE`].
    edge_offsets: Vec<u32>,
    edge_targets: Vec<u32>,
    /// Per importer, the package index of each `DepEdge` (in order), or
    /// [`NO_PACKAGE`].
    importer_edge_offsets: Vec<u32>,
    importer_edge_targets: Vec<u32>,
    reverse: OnceLock<(Vec<u32>, Vec<u32>)>,
}

/// Marker in the resolved edge tables for edges that do not point at a locked
/// package.
pub(crate) const NO_PACKAGE: u32 = u32::MAX;

/// What a dependency name resolves to, from [`Lockfile::resolve`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolved<'l> {
    /// A locked package.
    Package(&'l LockedPackage),
    /// A workspace package.
    Importer(&'l Importer),
    /// Declared but not resolved; holds the declared range.
    Unresolved(&'l str),
}

impl Lockfile {
    /// Builds a lockfile from parts, normalizing order (packages by id,
    /// importers by path, edges by name and kind), merging duplicate ids
    /// and building the query index.
    pub fn from_parts(
        format: LockfileFormat,
        version: impl Into<String>,
        mut packages: Vec<LockedPackage>,
        mut importers: Vec<Importer>,
        meta: LockfileMeta,
    ) -> Self {
        for p in &mut packages {
            normalize_edges(&mut p.dependencies);
            p.platform.os.sort();
            p.platform.cpu.sort();
            p.platform.libc.sort();
        }
        packages.sort_by(|a, b| a.id.cmp(&b.id).then_with(|| cmp_package(a, b)));
        packages.dedup_by(|later, first| {
            if later.id != first.id {
                return false;
            }
            // Same id seen twice (yarn classic merges): union edges and flags.
            first.dependencies.append(&mut later.dependencies);
            normalize_edges(&mut first.dependencies);
            first.flags &= later.flags;
            true
        });
        for i in &mut importers {
            i.path = normalize_importer_path(&i.path);
            normalize_edges(&mut i.dependencies);
        }
        importers.sort_by(|a, b| a.path.cmp(&b.path));
        importers.dedup_by(|later, first| {
            if later.path != first.path {
                return false;
            }
            first.dependencies.append(&mut later.dependencies);
            normalize_edges(&mut first.dependencies);
            if first.name.is_none() {
                first.name = later.name.take();
            }
            true
        });
        let mut lock = Self {
            format,
            version: version.into(),
            packages,
            importers,
            meta,
            descriptors: BTreeMap::new(),
            graph: Graph::default(),
        };
        lock.rebuild_graph();
        lock
    }

    pub(crate) fn set_descriptors(&mut self, descriptors: BTreeMap<String, String>) {
        self.descriptors = descriptors;
    }

    fn rebuild_graph(&mut self) {
        // Hash lookups: binary search over long, prefix-sharing ids (pnpm peer
        // suffixes) costs a dozen string compares per edge.
        let mut by_id: FxHashMap<&str, u32> = FxHashMap::default();
        by_id.reserve(self.packages.len());
        for (i, p) in self.packages.iter().enumerate() {
            by_id.insert(&p.id, i as u32);
        }
        let resolve = |e: &DepEdge| -> u32 {
            match &e.target {
                Target::Package(id) => by_id.get(id.as_str()).copied().unwrap_or(NO_PACKAGE),
                _ => NO_PACKAGE,
            }
        };
        let mut edge_offsets = Vec::with_capacity(self.packages.len() + 1);
        let mut edge_targets = Vec::new();
        edge_offsets.push(0);
        for p in &self.packages {
            edge_targets.extend(p.dependencies.iter().map(resolve));
            edge_offsets.push(edge_targets.len() as u32);
        }
        let mut importer_edge_offsets = Vec::with_capacity(self.importers.len() + 1);
        let mut importer_edge_targets = Vec::new();
        importer_edge_offsets.push(0);
        for i in &self.importers {
            importer_edge_targets.extend(i.dependencies.iter().map(resolve));
            importer_edge_offsets.push(importer_edge_targets.len() as u32);
        }
        let mut offsets = Vec::with_capacity(self.packages.len() + 1);
        let mut targets = Vec::new();
        offsets.push(0);
        for pi in 0..self.packages.len() {
            let (a, b) = (edge_offsets[pi] as usize, edge_offsets[pi + 1] as usize);
            targets.extend(
                edge_targets[a..b]
                    .iter()
                    .copied()
                    .filter(|&t| t != NO_PACKAGE),
            );
            let start = offsets[offsets.len() - 1] as usize;
            targets[start..].sort_unstable();
            let mut w = start;
            for r in start..targets.len() {
                if w == start || targets[w - 1] != targets[r] {
                    targets[w] = targets[r];
                    w += 1;
                }
            }
            targets.truncate(w);
            offsets.push(targets.len() as u32);
        }
        self.graph = Graph {
            offsets,
            targets,
            edge_offsets,
            edge_targets,
            importer_edge_offsets,
            importer_edge_targets,
            reverse: OnceLock::new(),
        };
    }

    /// Package index of each edge of package `index` ([`NO_PACKAGE`] if not a
    /// package edge).
    pub(crate) fn edge_targets(&self, index: usize) -> &[u32] {
        let g = &self.graph;
        &g.edge_targets[g.edge_offsets[index] as usize..g.edge_offsets[index + 1] as usize]
    }

    /// Package index of each edge of importer `index` ([`NO_PACKAGE`] if not a
    /// package edge).
    pub(crate) fn importer_edge_targets(&self, index: usize) -> &[u32] {
        let g = &self.graph;
        &g.importer_edge_targets
            [g.importer_edge_offsets[index] as usize..g.importer_edge_offsets[index + 1] as usize]
    }

    /// The package manager that wrote this lockfile.
    pub fn format(&self) -> LockfileFormat {
        self.format
    }

    /// The lockfile's own format version (`9.0`, `3`, `1`, `8`, …).
    pub fn lockfile_version(&self) -> &str {
        &self.version
    }

    /// All packages, sorted by id.
    pub fn packages(&self) -> &[LockedPackage] {
        &self.packages
    }

    /// All importers (workspace packages), sorted by path.
    pub fn importers(&self) -> &[Importer] {
        &self.importers
    }

    /// Overrides, catalogs and patched dependencies.
    pub fn meta(&self) -> &LockfileMeta {
        &self.meta
    }

    pub(crate) fn index_of(&self, id: &str) -> Option<usize> {
        self.packages
            .binary_search_by(|p| p.id.as_str().cmp(id))
            .ok()
    }

    /// The package with this id.
    pub fn package(&self, id: &str) -> Option<&LockedPackage> {
        self.index_of(id).map(|i| &self.packages[i])
    }

    /// All packages with this name (every version and peer variant), in id
    /// order.
    pub fn packages_named<'s>(&'s self, name: &'s str) -> impl Iterator<Item = &'s LockedPackage> {
        self.packages.iter().filter(move |p| p.name == name)
    }

    /// The importer at this path (`.`/`""`/`./x` forms accepted).
    pub fn importer(&self, path: &str) -> Option<&Importer> {
        let path = normalize_importer_path(path);
        self.importers
            .binary_search_by(|i| i.path.as_str().cmp(&path))
            .ok()
            .map(|i| &self.importers[i])
    }

    fn resolve_edge<'l>(&'l self, edge: &'l DepEdge) -> Resolved<'l> {
        match &edge.target {
            Target::Package(id) => match self.package(id) {
                Some(p) => Resolved::Package(p),
                None => Resolved::Unresolved(id),
            },
            Target::Importer(path) => match self.importer(path) {
                Some(i) => Resolved::Importer(i),
                None => Resolved::Unresolved(path),
            },
            Target::Unresolved(spec) => Resolved::Unresolved(spec),
        }
    }

    /// What dependency `name` of the importer at `importer_path` resolves to.
    pub fn resolve(&self, importer_path: &str, name: &str) -> Option<Resolved<'_>> {
        let edge = self.importer(importer_path)?.dependency(name)?;
        Some(self.resolve_edge(edge))
    }

    /// What dependency `name` of package `id` resolves to.
    pub fn resolve_from_package(&self, id: &str, name: &str) -> Option<Resolved<'_>> {
        let edge = self.package(id)?.dependency(name)?;
        Some(self.resolve_edge(edge))
    }

    /// Resolves a yarn descriptor (`name` + manifest range) to a package. Only
    /// yarn lockfiles record descriptors; other formats return `None`.
    pub fn resolve_descriptor(&self, name: &str, range: &str) -> Option<&LockedPackage> {
        let id = self
            .descriptors
            .get(&format!("{name}@{range}"))
            .or_else(|| self.descriptors.get(&format!("{name}@npm:{range}")))?;
        self.package(id)
    }

    /// The transitive closure of package `id` over package edges of every kind,
    /// including the package itself, sorted by id. Empty if `id` is
    /// unknown.
    pub fn closure(&self, id: &str) -> Vec<&LockedPackage> {
        self.closure_of([id])
    }

    /// The union of the closures of several packages, sorted by id.
    pub fn closure_of<'s>(&self, ids: impl IntoIterator<Item = &'s str>) -> Vec<&LockedPackage> {
        let seeds: Vec<usize> = ids.into_iter().filter_map(|id| self.index_of(id)).collect();
        self.collect(self.reach(&seeds, false))
    }

    /// Every package whose closure contains `id` (reverse closure), including
    /// `id`, sorted by id.
    pub fn dependents(&self, id: &str) -> Vec<&LockedPackage> {
        let seeds: Vec<usize> = self.index_of(id).into_iter().collect();
        self.collect(self.reach(&seeds, true))
    }

    /// The packages an importer needs: its direct dependencies' closures,
    /// following workspace links into other importers' runtime
    /// dependencies. Dev dependencies are included for the importer itself
    /// only when `include_dev` is set (never for linked workspace packages).
    pub fn importer_closure(&self, path: &str, include_dev: bool) -> Vec<&LockedPackage> {
        let mut seen_importers = std::collections::BTreeSet::new();
        let mut stack = vec![(normalize_importer_path(path), include_dev)];
        let mut seeds = Vec::new();
        while let Some((path, dev)) = stack.pop() {
            if !seen_importers.insert(path.clone()) {
                continue;
            }
            let Some(imp) = self.importer(&path) else {
                continue;
            };
            for e in &imp.dependencies {
                if e.kind == DepKind::Dev && !dev {
                    continue;
                }
                match &e.target {
                    Target::Package(id) => seeds.extend(self.index_of(id)),
                    Target::Importer(p) => stack.push((p.clone(), false)),
                    Target::Unresolved(_) => {}
                }
            }
        }
        self.collect(self.reach(&seeds, false))
    }

    fn collect(&self, set: FixedBitSet) -> Vec<&LockedPackage> {
        set.ones().map(|i| &self.packages[i]).collect()
    }

    pub(crate) fn reach(&self, seeds: &[usize], reverse: bool) -> FixedBitSet {
        let (offsets, targets) = if reverse {
            let (o, t) = self.reverse_graph();
            (o.as_slice(), t.as_slice())
        } else {
            (self.graph.offsets.as_slice(), self.graph.targets.as_slice())
        };
        let mut seen = FixedBitSet::with_capacity(self.packages.len());
        let mut stack: Vec<usize> = Vec::new();
        for &s in seeds {
            if !seen.put(s) {
                stack.push(s);
            }
        }
        while let Some(n) = stack.pop() {
            for &t in &targets[offsets[n] as usize..offsets[n + 1] as usize] {
                if !seen.put(t as usize) {
                    stack.push(t as usize);
                }
            }
        }
        seen
    }

    fn reverse_graph(&self) -> &(Vec<u32>, Vec<u32>) {
        self.graph.reverse.get_or_init(|| {
            let n = self.packages.len();
            let mut counts = vec![0u32; n + 1];
            for &t in &self.graph.targets {
                counts[t as usize + 1] += 1;
            }
            for i in 0..n {
                counts[i + 1] += counts[i];
            }
            let mut fill = counts.clone();
            let mut rev = vec![0u32; self.graph.targets.len()];
            for src in 0..n {
                let (a, b) = (
                    self.graph.offsets[src] as usize,
                    self.graph.offsets[src + 1] as usize,
                );
                for &t in &self.graph.targets[a..b] {
                    rev[fill[t as usize] as usize] = src as u32;
                    fill[t as usize] += 1;
                }
            }
            (counts, rev)
        })
    }

    /// Adds importers from workspace manifests, resolving their ranges through
    /// the lockfile's descriptors. Meant for yarn classic, whose lockfile
    /// records no importers. Each entry is `(path relative to the lockfile,
    /// package.json contents)`. Paths that already have an importer are
    /// left unchanged. Workspace siblings are matched by manifest `name`.
    pub fn add_manifest_importers(
        &mut self,
        manifests: &[(&str, &str)],
    ) -> Result<(), crate::Error> {
        #[derive(serde::Deserialize, Default)]
        #[serde(default)]
        struct Manifest {
            name: Option<String>,
            dependencies: BTreeMap<String, String>,
            #[serde(rename = "devDependencies")]
            dev_dependencies: BTreeMap<String, String>,
            #[serde(rename = "optionalDependencies")]
            optional_dependencies: BTreeMap<String, String>,
            #[serde(rename = "peerDependencies")]
            peer_dependencies: BTreeMap<String, String>,
        }
        let mut parsed = Vec::new();
        for (path, json) in manifests {
            let m: Manifest = serde_json::from_str(json).map_err(|e| crate::Error::Json {
                format: self.format,
                source: e,
            })?;
            parsed.push((normalize_importer_path(path), m));
        }
        let workspace_by_name: BTreeMap<String, String> = parsed
            .iter()
            .filter_map(|(p, m)| Some((m.name.clone()?, p.clone())))
            .collect();
        let mut importers = std::mem::take(&mut self.importers);
        for (path, m) in parsed {
            if importers.iter().any(|i| i.path == path) {
                continue;
            }
            let mut imp = Importer {
                path,
                name: m.name.clone(),
                dependencies: Vec::new(),
            };
            let kinds = [
                (DepKind::Prod, &m.dependencies),
                (DepKind::Dev, &m.dev_dependencies),
                (DepKind::Optional, &m.optional_dependencies),
                (DepKind::Peer, &m.peer_dependencies),
            ];
            for (kind, deps) in kinds {
                for (name, range) in deps {
                    let target = if let Some(p) = self.resolve_descriptor(name, range) {
                        Target::Package(p.id.clone())
                    } else if let Some(ws) = workspace_by_name.get(name) {
                        Target::Importer(ws.clone())
                    } else {
                        Target::Unresolved(range.clone())
                    };
                    imp.dependencies.push(DepEdge {
                        name: name.clone(),
                        kind,
                        target,
                        specifier: Some(range.clone()),
                    });
                }
            }
            normalize_edges(&mut imp.dependencies);
            importers.push(imp);
        }
        importers.sort_by(|a, b| a.path.cmp(&b.path));
        self.importers = importers;
        // The resolved importer-edge tables index into `importers`.
        self.rebuild_graph();
        Ok(())
    }
}

fn cmp_package(a: &LockedPackage, b: &LockedPackage) -> std::cmp::Ordering {
    (&a.version, &a.integrity, &a.resolution, &a.patch).cmp(&(
        &b.version,
        &b.integrity,
        &b.resolution,
        &b.patch,
    ))
}

pub(crate) fn normalize_edges(edges: &mut Vec<DepEdge>) {
    edges.sort();
    edges.dedup_by(|a, b| a.name == b.name && a.kind == b.kind && a.target == b.target);
}

/// Normalizes an importer path: `""`, `"."` and `"./"` become `"."`; `./a/b/`
/// becomes `a/b`; backslashes become slashes; `.` and `..` components are
/// folded.
pub fn normalize_importer_path(path: &str) -> String {
    // Fast path: already normal (the common case for lockfile keys).
    let b = path.as_bytes();
    let already_normal = !b.is_empty()
        && path != "."
        && !b.contains(&b'\\')
        && b[0] != b'/'
        && b[b.len() - 1] != b'/'
        && path
            .split('/')
            .all(|c| !c.is_empty() && c != "." && c != "..");
    if already_normal || path == "." {
        return path.to_string();
    }
    let path = path.replace('\\', "/");
    let mut out: Vec<&str> = Vec::new();
    for comp in path.split('/') {
        match comp {
            "" | "." => {}
            ".." => {
                if matches!(out.last(), Some(c) if *c != "..") {
                    out.pop();
                } else {
                    out.push("..");
                }
            }
            c => out.push(c),
        }
    }
    if out.is_empty() {
        ".".to_string()
    } else {
        out.join("/")
    }
}

/// Joins a relative path onto an importer path and normalizes it.
pub(crate) fn join_importer_path(base: &str, rel: &str) -> String {
    if base == "." || base.is_empty() {
        normalize_importer_path(rel)
    } else {
        normalize_importer_path(&format!("{base}/{rel}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn importer_paths_normalize() {
        assert_eq!(normalize_importer_path(""), ".");
        assert_eq!(normalize_importer_path("./"), ".");
        assert_eq!(normalize_importer_path("./a/b/"), "a/b");
        assert_eq!(normalize_importer_path("a\\b"), "a/b");
        assert_eq!(
            join_importer_path("services/svc-a", "../../packages/x"),
            "packages/x"
        );
        assert_eq!(join_importer_path(".", "../outside"), "../outside");
    }
}
