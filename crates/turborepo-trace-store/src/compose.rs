//! Composition: the shards of a manifest as one [`GraphView`].
//!
//! Loading reads every shard listed in `graph.json` into an aligned buffer and
//! accesses it in place. Then, with no re-resolution:
//!
//! 1. One package per shard. Nodes: every file of a traced shard, one node per
//!    opaque package (path `node_modules/<name>@<version>`, or
//!    `<ecosystem>:<name>@<version>` for Python, Go and Rust packages), and one
//!    leaf node per edge target that is not a traced file (assets, manifests,
//!    untraced files of other packages), owned by the package whose shard names
//!    it.
//! 2. Node ids are dense and follow path order, so ids, outputs and exports are
//!    deterministic and `node_by_path` is a binary search.
//! 3. Edges are linked: `File(i)` through the shard's file table; `Cross {
//!    name, version, subpath }` through a hash map from `(name, version)` (or
//!    the workspace package name) to the target shard, then its file table or
//!    the opaque package node.
//! 4. The forward adjacency is built once in CSR form. Edge details (specifier,
//!    line, span) and uncertainties are read straight from the archived shards.
//!
//! Probes and memberships are expanded to repo-relative paths lazily, on first
//! use, into interned tables (the query engine builds its probe trie and
//! membership index from them).
//!
//! Build tasks ([`crate::tasks`]): a task pseudo-file's node is named by its
//! task id (`task:apps/web#build`), a membership hub's by
//! `members:<dir>/<pattern>`. After linking, two kinds of edges are added that
//! no single shard can know:
//!
//! - task → task, for each dependency id that names a task node;
//! - node → task, for generated-file linking: every node an output of the task
//!   covers, and every node that probes, reads (config input) or lists
//!   (membership) a covered path. Such nodes lose their `generated_file`
//!   uncertainties, and the `unresolved_specifier` uncertainties of relative
//!   specifiers that point into a covered output (a gitignored generated file
//!   that is not there yet), or of bare imports of an output package (Prisma's
//!   default client `@prisma/client`, not installed): the link replaces them. A
//!   committed file is not linked to a task that nothing in the repository runs
//!   (a generator run by hand, its output checked in): the tests read the
//!   committed bytes (`compose/tracked.rs`).
//!
//! An output package (`TaskSpec::output_package`) arrives as one output per
//! `<dir>/node_modules/<name>` location, marked with the package name. Besides
//! what any output links, it links the package's opaque node
//! (`node_modules/<name>@<version>`), and only package outputs replace
//! unresolved bare imports.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::OnceLock,
    time::Instant,
};

use rayon::prelude::*;
use tables::{Interned, Inverted, Pair, TABLES_MAGIC, Tables};
use turborepo_trace_core::timing::Span as TimeSpan;
use turborepo_trace_graph::{
    Adjacency, Edge, EdgeAttr, GraphView, Membership, NodeId, PackageId, PackageInfo,
    PackageVersion, Span, Uncertainty,
};

use crate::{
    Error,
    convert::query_unc_kind,
    manifest::Manifest,
    normalize::expand_probe,
    paths::{join_repo, join_simple},
    shard::{self, ArchivedTargetRec, LoadedShard, kind},
};

pub mod bind;
mod cache;
pub(crate) mod cwd;
pub use cwd::cwd_edge_site;
mod derived;
pub use derived::Kept;
pub mod env;
mod facts;
mod gogen;
pub mod hints;
mod keyobs;
mod listed;
mod mocks;
mod reuse;
mod tables;
mod tracked;

const NONE: u32 = u32::MAX;
/// Loose shards (not in the pack) a load tolerates before repacking (or 5% of
/// the shards).
const PACK_SLACK: usize = 64;
/// Log bytes a load tolerates before compacting it into the pack (or a quarter
/// of the pack): appended shards replace earlier versions, which a load still
/// reads.
const LOG_SLACK: u64 = 32 << 20;

/// Per shard, per probe record: the string-table ids of the repo-relative paths
/// the record expands to ([`Graph::build_tables`]).
type ProbeIds = Vec<Vec<Vec<u32>>>;

/// An output that produced links: repo-relative path, pattern, excludes, and
/// the npm package name for an output package's location.
type GenOutput = (String, Option<String>, Vec<String>, Option<String>);

/// Set on a target in [`Graph::extra_to`]: a generated-file link made only by
/// type-level references of the linked node ([`Graph::probed_by_types_only`]).
/// The edge is `type_only`: entries that are evaluated without type-only edges
/// (transpiled tests, `--runtime-only`) don't depend on the producer through
/// it.
pub(crate) const TYPE_ONLY_LINK: u32 = 1 << 31;
/// `Node::file` of an opaque package node.
const OPAQUE: u32 = u32::MAX - 1;

#[derive(Clone, Debug)]
struct Pkg {
    name: String,
    version: Option<String>,
    root: String,
    external: bool,
}

#[derive(Copy, Clone, Debug)]
struct Node {
    pkg: u32,
    shard: u32,
    file: u32,
    lang: u8,
}

/// Probe records of one shard handled per parallel job when the tables are
/// built.
const PAR_RECORDS: usize = 256;

/// Load timings in microseconds: read+check shards, build nodes, link edges.
#[derive(Copy, Clone, Debug, Default)]
pub struct LoadStats {
    /// Reading and checking shard files.
    pub read_us: u64,
    /// Collecting and sorting nodes.
    pub nodes_us: u64,
    /// Linking edges and building the CSR adjacency, task linking included.
    pub link_us: u64,
    /// Of `link_us`: task and generated-file linking ([`Graph`] module docs).
    pub tasks_us: u64,
    /// Of `tasks_us`: expanding probe and membership tables (or reading them
    /// from the tables cache) and inverting them.
    pub tables_us: u64,
    /// The composed parts are the previous snapshot's: the shards that changed
    /// are equal in everything composition reads (an edit that keeps a
    /// file's references).
    pub reused: bool,
    /// Shards loaded.
    pub shards: usize,
    /// Bytes of shard archives.
    pub bytes: usize,
}

/// A composed whole-repo graph.
pub struct Graph {
    root: String,
    manifest: Manifest,
    shards: Vec<LoadedShard>,
    packages: Vec<Pkg>,
    paths: String,
    path_end: Vec<u32>,
    nodes: Vec<Node>,
    langs: Vec<String>,
    edge_off: Vec<u32>,
    edge_to: Vec<u32>,
    fwd: Adjacency,
    /// Task edges added after linking (see the module docs), CSR by node.
    extra_off: Vec<u32>,
    extra_to: Vec<u32>,
    /// Membership hub nodes with the index of their expanded member list in
    /// `hub_targets` (sorted by node); see [`crate::shard::HubRec`].
    hub_nodes: Vec<(u32, u32)>,
    /// Expanded hub members (node ids in path order), one list per distinct
    /// hub.
    hub_targets: Vec<Vec<u32>>,
    /// Nodes linked to the producer of a generated file they use, with the
    /// outputs (indexes into `gen_outputs`) they were linked through
    /// (sorted by node).
    generated: Vec<(u32, Vec<u32>)>,
    /// Outputs that produced links: repo-relative path, pattern, excludes,
    /// output package.
    gen_outputs: Vec<GenOutput>,
    tables: OnceLock<Tables>,
    inverted: OnceLock<Inverted>,
    /// The graph directory the graph was opened from.
    graph_dir: Option<std::path::PathBuf>,
    /// `tables` in the graph directory's [`crate::cache_dir`]: [`Tables`] of
    /// this manifest, reused across runs.
    tables_cache: Option<std::path::PathBuf>,
    /// Entries that are not tests of a graph framework, with the directories
    /// they run in ([`Graph::set_entry_cwds`]).
    entry_cwds: Vec<(u32, String)>,
    /// [`Graph::set_cwd_root_always`].
    cwd_root_always: bool,
    /// [`Graph::cwd_assumed`]: `(entry, sentence)`.
    cwd_notes: std::sync::Mutex<Vec<(u32, String)>>,
    /// [`Graph::cwd_dropped`].
    cwd_dropped: std::sync::Mutex<Vec<cwd::CwdDropped>>,
    /// The snapshot whose composed parts this graph took over across an edit
    /// ([`reuse`]), with the files that changed since ([`Graph::carried`]).
    carried: Option<derived::Carried>,
    stats: LoadStats,
}

impl std::fmt::Debug for Graph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Graph")
            .field("root", &self.root)
            .field("nodes", &self.nodes.len())
            .field("edges", &self.edge_to.len())
            .field("packages", &self.packages.len())
            .finish_non_exhaustive()
    }
}

/// How an edge target links.
enum Link {
    Node(u32),
    Path(String, u32),
    None,
}

impl Graph {
    /// Opens the graph stored in `out` (the `.turbotrace` directory).
    pub fn open(out: &Path) -> Result<Graph, Error> {
        let manifest = Manifest::read(out).ok_or_else(|| Error::NotBuilt(out.to_path_buf()))?;
        Graph::from_manifest(out, manifest)
    }

    /// Composes the shards of `manifest` from `<out>/store`.
    pub fn from_manifest(out: &Path, manifest: Manifest) -> Result<Graph, Error> {
        let t0 = Instant::now();
        let store = out.join("store");
        let pack = shard::Pack::open(&store);
        let shards: Vec<LoadedShard> = manifest
            .shards
            .par_iter()
            .map(|e| shard::load_from(&store, pack.as_ref(), &e.key).map_err(Error::Shard))
            .collect::<Result<_, _>>()?;
        // Compact when the log has grown (it keeps replaced versions of shards) or many
        // shards are loose files (older stores): one pack of exactly this snapshot's
        // shards.
        let loose = manifest
            .shards
            .iter()
            .filter(|e| !pack.as_ref().is_some_and(|p| p.contains(&e.key)))
            .count();
        let [main_bytes, log_bytes] = pack.as_ref().map_or([0, 0], |p| p.sizes());
        // The previous snapshot's composed parts, when this one differs from it only in
        // what composition does not read. Decided here: the compaction below
        // drops the shards that were replaced.
        let mut reused = reuse::across_edit(out, &manifest, &shards, pack.as_ref());
        drop(pack);
        if loose > PACK_SLACK.max(manifest.shards.len() / 20)
            || log_bytes > LOG_SLACK.max(main_bytes / 4)
        {
            let list: Vec<(&str, &LoadedShard)> = manifest
                .shards
                .iter()
                .map(|e| e.key.as_str())
                .zip(&shards)
                .collect();
            let _ = shard::Pack::compact(&store, &list);
        }
        turborepo_trace_core::timing::mark("load: read");
        let mut stats = LoadStats {
            read_us: t0.elapsed().as_micros() as u64,
            shards: shards.len(),
            bytes: shards.iter().map(LoadedShard::len).sum(),
            ..Default::default()
        };

        // Packages.
        let t1 = Instant::now();
        // Roots come from the manifest: an opaque shard does not store its installed
        // location.
        let packages: Vec<Pkg> = shards
            .iter()
            .zip(&manifest.shards)
            .map(|(s, e)| {
                let p = &s.get().package;
                Pkg {
                    name: p.name.to_string(),
                    version: p.version.as_ref().map(|v| v.to_string()),
                    root: e.root.clone(),
                    external: p.kind != kind::FIRST_PARTY,
                }
            })
            .collect();
        let root_pkg = shards
            .iter()
            .position(|s| {
                s.get().package.kind == kind::FIRST_PARTY && s.get().package.root.is_empty()
            })
            .unwrap_or(0) as u32;
        // An unchanged manifest: the composed parts from the previous run.
        let ckey = cache::key(&manifest);
        let cpath = crate::cache_dir(out).join("composed");
        let use_cache = turborepo_trace_core::debug::var_os("NO_COMPOSED_CACHE").is_none();
        if use_cache {
            let shard_files: Vec<usize> = shards.iter().map(|s| s.get().files.len()).collect();
            let (cached, previous, carried) = match reused.take() {
                Some(t) => (Some(t.composed), Some(t.old), Some(t.changed)),
                None => (
                    std::fs::read(&cpath)
                        .ok()
                        .and_then(|b| cache::Composed::decode(&b, ckey, &shard_files)),
                    None,
                    None,
                ),
            };
            if let Some(c) = cached {
                stats.nodes_us = t1.elapsed().as_micros() as u64;
                stats.reused = previous.is_some();
                let carried =
                    carried
                        .zip(previous.as_ref())
                        .map(|(changed, old)| derived::Carried {
                            key: cache::key(old),
                            changed,
                        });
                let g = Graph {
                    root: manifest.root.clone(),
                    manifest,
                    shards,
                    packages,
                    paths: c.paths,
                    path_end: c.path_end,
                    nodes: c.nodes,
                    langs: c.langs,
                    edge_off: c.edge_off,
                    edge_to: c.edge_to,
                    fwd: c.fwd,
                    extra_off: c.extra_off,
                    extra_to: c.extra_to,
                    hub_nodes: c.hub_nodes,
                    hub_targets: c.hub_targets,
                    generated: c.generated,
                    gen_outputs: c.gen_outputs,
                    tables: OnceLock::new(),
                    inverted: OnceLock::new(),
                    graph_dir: Some(out.to_path_buf()),
                    tables_cache: Some(crate::cache_dir(out).join("tables")),
                    entry_cwds: Vec::new(),
                    cwd_root_always: cwd::root_always_by_switch(),
                    cwd_notes: Default::default(),
                    cwd_dropped: Default::default(),
                    carried,
                    stats,
                };
                match previous {
                    // Both caches again, under this manifest's keys.
                    Some(old) => {
                        let _ = crate::write_atomic(&cpath, &cache::Composed::encode(&g, ckey));
                        reuse::stamp(out, &g.manifest);
                        tables::rekey(
                            &crate::cache_dir(out).join("tables"),
                            Graph::tables_key_of(&old, g.nodes.len()),
                            g.tables_key(),
                        );
                        turborepo_trace_core::timing::mark(
                            "load: composed (reused across an edit)",
                        );
                    }
                    None => turborepo_trace_core::timing::mark("load: composed (cached)"),
                }
                return Ok(g);
            }
        }
        let mut ws_by_name: HashMap<&str, u32> = HashMap::new();
        let mut tp_by_id: HashMap<(&str, &str), u32> = HashMap::new();
        for (i, s) in shards.iter().enumerate() {
            let p = &s.get().package;
            match p.version.as_ref() {
                Some(v) if p.kind != kind::FIRST_PARTY => {
                    tp_by_id.insert((p.name.as_str(), v.as_str()), i as u32);
                }
                _ => {
                    ws_by_name.insert(p.name.as_str(), i as u32);
                }
            }
        }

        // Membership hubs: one member list per distinct hub, expanded from the walk.
        turborepo_trace_core::timing::mark("load: packages");
        let hubs = expand_hubs(out, &manifest, &shards)?;
        turborepo_trace_core::timing::mark("load: hubs");
        // Nodes: files and opaque packages, then leaves.
        let mut langs: Vec<String> = Vec::new();
        for s in &shards {
            for f in s.get().files.iter() {
                if let Some(l) = f.lang.as_ref()
                    && !langs.iter().any(|x| x == l.as_str())
                {
                    langs.push(l.to_string());
                }
            }
        }
        let lang_ix = |l: Option<&str>| -> u8 {
            l.and_then(|l| langs.iter().position(|x| x == l))
                .map_or(u8::MAX, |i| i as u8)
        };
        let opaque = |si: u32| shards[si as usize].get().package.kind == kind::OPAQUE;
        let link_path = |si: usize, t: &ArchivedTargetRec| -> Link {
            let a = shards[si].get();
            match t {
                ArchivedTargetRec::None | ArchivedTargetRec::File(_) => Link::None,
                ArchivedTargetRec::Intra(rel) if rel.starts_with('#') => Link::Path(
                    crate::tasks::pseudo_node_path(rel.as_str()).to_owned(),
                    si as u32,
                ),
                ArchivedTargetRec::Intra(rel) => {
                    match join_repo(a.package.root.as_str(), rel.as_str()) {
                        Some(p) => Link::Path(p, si as u32),
                        None => Link::Path(
                            crate::paths::abs(Path::new(a.package.root.as_str()), rel.as_str())
                                .to_string_lossy()
                                .into_owned(),
                            si as u32,
                        ),
                    }
                }
                ArchivedTargetRec::Cross(c) => {
                    let target = match c.version.as_ref() {
                        None => ws_by_name.get(c.name.as_str()),
                        Some(v) => tp_by_id.get(&(c.name.as_str(), v.as_str())),
                    };
                    match target {
                        Some(&ti) if opaque(ti) => Link::Node(ti),
                        Some(&ti) if c.subpath.starts_with('#') => Link::Path(
                            crate::tasks::pseudo_node_path(c.subpath.as_str()).to_owned(),
                            ti,
                        ),
                        Some(&ti) => Link::Path(
                            join_simple(
                                shards[ti as usize].get().package.root.as_str(),
                                c.subpath.as_str(),
                            ),
                            ti,
                        ),
                        None => Link::None,
                    }
                }
                ArchivedTargetRec::Outside(p) => Link::Path(p.to_string(), root_pkg),
            }
        };
        // Per shard, in parallel: its file and package nodes, and its leaf candidates.
        // `rank` 0 = file or package node, 1 = leaf; a leaf that is also a file is
        // dropped.
        let per_shard: Vec<Vec<(String, u8, Node)>> = (0..shards.len())
            .into_par_iter()
            .map(|si| {
                let a = shards[si].get();
                let mut out = Vec::new();
                if a.package.kind == kind::OPAQUE {
                    let v = a
                        .package
                        .version
                        .as_ref()
                        .map(|v| v.as_str())
                        .unwrap_or("unknown");
                    // npm packages: `node_modules/<name>@<version>`; language packages
                    // (namespaced `eco:name`): `<eco>:<name>@<version>`.
                    let name = a.package.name.as_str();
                    let path = if name.contains(':') {
                        format!("{name}@{v}")
                    } else {
                        format!("node_modules/{name}@{v}")
                    };
                    out.push((
                        path,
                        0,
                        Node {
                            pkg: si as u32,
                            shard: NONE,
                            file: OPAQUE,
                            lang: u8::MAX,
                        },
                    ));
                    return out;
                }
                let mut leaves: HashMap<String, u32> = HashMap::new();
                for (fi, f) in a.files.iter().enumerate() {
                    out.push((
                        node_path(a.package.root.as_str(), f),
                        0,
                        Node {
                            pkg: si as u32,
                            shard: si as u32,
                            file: fi as u32,
                            lang: lang_ix(f.lang.as_ref().map(|l| l.as_str())),
                        },
                    ));
                    for r in f.refs.iter() {
                        for e in r.edges.iter() {
                            if let Link::Path(p, owner) = link_path(si, &e.target) {
                                leaves.entry(p).or_insert(owner);
                            }
                        }
                    }
                }
                out.extend(leaves.into_iter().map(|(p, owner)| {
                    (
                        p,
                        1,
                        Node {
                            pkg: owner,
                            shard: NONE,
                            file: NONE,
                            lang: u8::MAX,
                        },
                    )
                }));
                out
            })
            .collect();
        turborepo_trace_core::timing::mark("load: node candidates");
        let mut cand: Vec<(String, u8, Node)> = per_shard.into_iter().flatten().collect();
        // Hub members that are not files: leaves owned by the deepest first-party
        // package containing them (as the stored edges' targets were).
        if !hubs.lists.is_empty() {
            let fp_roots: HashMap<&str, u32> = shards
                .iter()
                .enumerate()
                .filter(|(_, s)| s.get().package.kind == kind::FIRST_PARTY)
                .map(|(i, s)| (s.get().package.root.as_str(), i as u32))
                .collect();
            let owner = |p: &str| -> u32 {
                let mut q = p;
                while let Some(i) = q.rfind('/') {
                    q = &q[..i];
                    if let Some(&o) = fp_roots.get(q) {
                        return o;
                    }
                }
                fp_roots.get("").copied().unwrap_or(root_pkg)
            };
            // One candidate per member path, whichever hubs list it (indexes into the
            // sorted walk: sorted indexes are sorted paths).
            let mut members: Vec<u32> = hubs.lists.iter().flatten().copied().collect();
            if hubs.lists.len() > 1 {
                members.par_sort_unstable();
                members.dedup();
            }
            let files = &hubs.files;
            cand.par_extend(members.into_par_iter().map(|i| {
                let m = files[i as usize].as_str();
                (
                    m.to_owned(),
                    1,
                    Node {
                        pkg: owner(m),
                        shard: NONE,
                        file: NONE,
                        lang: u8::MAX,
                    },
                )
            }));
        }
        // The same leaf named by several shards keeps the owner of the first shard.
        cand.par_sort_unstable_by(|a, b| {
            (a.0.as_str(), a.1, a.2.pkg).cmp(&(b.0.as_str(), b.1, b.2.pkg))
        });
        cand.dedup_by(|a, b| a.0 == b.0);
        turborepo_trace_core::timing::mark("load: node sort");
        let mut paths = String::with_capacity(cand.iter().map(|c| c.0.len()).sum());
        let mut path_end = Vec::with_capacity(cand.len());
        let mut nodes = Vec::with_capacity(cand.len());
        let mut file_node: Vec<Vec<u32>> = shards
            .iter()
            .map(|s| vec![NONE; s.get().files.len()])
            .collect();
        let mut pkg_node = vec![NONE; shards.len()];
        for (i, (p, _, n)) in cand.into_iter().enumerate() {
            paths.push_str(&p);
            path_end.push(paths.len() as u32);
            if n.shard != NONE {
                file_node[n.shard as usize][n.file as usize] = i as u32;
            } else if n.file == OPAQUE {
                pkg_node[n.pkg as usize] = i as u32;
            }
            nodes.push(n);
        }
        stats.nodes_us = t1.elapsed().as_micros() as u64;
        turborepo_trace_core::timing::mark("load: nodes");

        // Edges: targets per shard in parallel, then the CSR in node order.
        let t2 = Instant::now();
        let mut g = Graph {
            root: manifest.root.clone(),
            manifest: Manifest::default(),
            shards: Vec::new(),
            packages,
            paths,
            path_end,
            nodes,
            langs,
            edge_off: Vec::new(),
            edge_to: Vec::new(),
            fwd: Adjacency::default(),
            extra_off: Vec::new(),
            extra_to: Vec::new(),
            hub_nodes: Vec::new(),
            hub_targets: Vec::new(),
            generated: Vec::new(),
            gen_outputs: Vec::new(),
            tables: OnceLock::new(),
            inverted: OnceLock::new(),
            graph_dir: Some(out.to_path_buf()),
            tables_cache: Some(crate::cache_dir(out).join("tables")),
            entry_cwds: Vec::new(),
            cwd_root_always: cwd::root_always_by_switch(),
            cwd_notes: Default::default(),
            cwd_dropped: Default::default(),
            carried: None,
            stats,
        };
        if !hubs.lists.is_empty() {
            // Node of each member, looked up once per distinct member.
            let mut node_of: Vec<u32> = vec![NONE; hubs.files.len()];
            {
                let mut members: Vec<u32> = hubs.lists.iter().flatten().copied().collect();
                members.par_sort_unstable();
                members.dedup();
                let found: Vec<(u32, u32)> = members
                    .par_iter()
                    .filter_map(|&i| Some((i, g.lookup(&hubs.files[i as usize])?)))
                    .collect();
                for (i, n) in found {
                    node_of[i as usize] = n;
                }
            }
            let targets: Vec<Vec<u32>> = hubs
                .lists
                .par_iter()
                .map(|l| {
                    l.iter()
                        .map(|&i| node_of[i as usize])
                        .filter(|&n| n != NONE)
                        .collect()
                })
                .collect();
            let mut hub_nodes: Vec<(u32, u32)> = Vec::new();
            for (si, s) in shards.iter().enumerate() {
                for (h, &hi) in s.get().hubs.iter().zip(&hubs.of_shard[si]) {
                    // A hub several shards define has one node (the first shard's).
                    if let Some(&n) = file_node[si].get(h.file.to_native() as usize)
                        && n != NONE
                    {
                        hub_nodes.push((n, hi));
                    }
                }
            }
            hub_nodes.sort_unstable();
            hub_nodes.dedup_by_key(|x| x.0);
            g.hub_nodes = hub_nodes;
            g.hub_targets = targets;
        }
        let gref = &g;
        // Per shard: (edge offset per file, target per edge, attr per edge).
        type ShardEdges = (Vec<u32>, Vec<u32>, Vec<u16>);
        let per_shard: Vec<ShardEdges> = (0..shards.len())
            .into_par_iter()
            .map(|si| {
                let a = shards[si].get();
                let mut off = Vec::with_capacity(a.files.len() + 1);
                let mut to = Vec::new();
                let mut attr = Vec::new();
                let mut memo: HashMap<String, u32> = HashMap::new();
                off.push(0u32);
                for f in a.files.iter() {
                    for r in f.refs.iter() {
                        for e in r.edges.iter() {
                            let t = match &e.target {
                                ArchivedTargetRec::File(fi) => {
                                    let fi = fi.to_native() as usize;
                                    match file_node[si][fi] {
                                        // A pseudo-file several shards define (a membership
                                        // hub shared by their tasks) has one node, from the
                                        // first shard: link to it by name.
                                        NONE => {
                                            let p =
                                                node_path(a.package.root.as_str(), &a.files[fi]);
                                            gref.lookup(&p).unwrap_or(NONE)
                                        }
                                        n => n,
                                    }
                                }
                                t => match link_path(si, t) {
                                    Link::Node(ti) => pkg_node[ti as usize],
                                    Link::Path(p, _) => match memo.get(&p) {
                                        Some(&id) => id,
                                        None => {
                                            let id = gref.lookup(&p).unwrap_or(NONE);
                                            memo.insert(p, id);
                                            id
                                        }
                                    },
                                    Link::None => NONE,
                                },
                            };
                            to.push(t);
                            attr.push(e.attr.to_native());
                        }
                    }
                    off.push(to.len() as u32);
                }
                (off, to, attr)
            })
            .collect();
        let n = g.nodes.len();
        let total: usize = per_shard.iter().map(|s| s.1.len()).sum::<usize>()
            + g.hub_nodes
                .iter()
                .map(|&(_, h)| g.hub_targets[h as usize].len())
                .sum::<usize>();
        let hub_attr = hub_edge_attr();
        let mut hub_k = 0usize;
        let mut edge_off = Vec::with_capacity(n + 1);
        let mut edge_to = Vec::with_capacity(total);
        let mut fwd_off = Vec::with_capacity(n + 1);
        let mut fwd_to = Vec::with_capacity(total);
        let mut fwd_attr = Vec::with_capacity(total);
        edge_off.push(0u32);
        fwd_off.push(0u32);
        for node in &g.nodes {
            if node.shard != NONE {
                let (off, to, attr) = &per_shard[node.shard as usize];
                let (lo, hi) = (
                    off[node.file as usize] as usize,
                    off[node.file as usize + 1] as usize,
                );
                for (t, a) in to[lo..hi].iter().zip(&attr[lo..hi]) {
                    edge_to.push(*t);
                    if *t != NONE {
                        fwd_to.push(*t);
                        fwd_attr.push(EdgeAttr(*a));
                    }
                }
            }
            // Expanded hub edges follow the node's stored edges.
            let ni = edge_off.len() as u32 - 1;
            while hub_k < g.hub_nodes.len() && g.hub_nodes[hub_k].0 < ni {
                hub_k += 1;
            }
            if hub_k < g.hub_nodes.len() && g.hub_nodes[hub_k].0 == ni {
                let ts = &g.hub_targets[g.hub_nodes[hub_k].1 as usize];
                fwd_to.extend_from_slice(ts);
                fwd_attr.extend(std::iter::repeat_n(hub_attr, ts.len()));
            }
            edge_off.push(edge_to.len() as u32);
            fwd_off.push(fwd_to.len() as u32);
        }
        drop(per_shard);
        g.edge_off = edge_off;
        g.edge_to = edge_to;
        g.fwd = Adjacency::from_parts(fwd_off, fwd_to, fwd_attr);
        g.stats.link_us = t2.elapsed().as_micros() as u64;
        g.shards = shards;
        turborepo_trace_core::timing::mark("load: edges");
        g.manifest = manifest;
        let t3 = Instant::now();
        let produced = g.link_tasks(out);
        g.mark_byte_reads(&produced);
        g.stats.tasks_us = t3.elapsed().as_micros() as u64;
        g.stats.link_us += g.stats.tasks_us;
        turborepo_trace_core::timing::mark("load: link tasks");
        if use_cache {
            crate::create_cache_dir(out);
            let _ = crate::write_atomic(&cpath, &cache::Composed::encode(&g, ckey));
            reuse::stamp(out, &g.manifest);
            turborepo_trace_core::timing::mark("load: composed cache write");
        }
        Ok(g)
    }

    /// Adds task dependency edges and generated-file links (see the module
    /// docs). `out`: the graph directory.
    fn link_tasks(&mut self, out: &Path) -> Vec<u32> {
        let mut extra: Vec<(u32, u32)> = Vec::new();
        // Nodes an output covers: files a task (re)generates (`listed`), sorted.
        let mut produced: Vec<u32> = Vec::new();
        // Generated-file links that exist only through type-level references.
        let mut weak: Vec<(u32, u32)> = Vec::new();
        let mut generated: Vec<(u32, u32)> = Vec::new();
        // Links made only by listings that do not name the output (`listed`).
        let mut listed: Vec<(u32, u32)> = Vec::new();
        let mut gen_outputs: Vec<GenOutput> = Vec::new();
        let mut tables_us = 0u64;
        // Task dependency edges, per shard in parallel (every task names its
        // dependencies by path: many lookups).
        let deps: Vec<(Vec<(u32, u32)>, bool)> = self
            .shards
            .par_iter()
            .enumerate()
            .map(|(si, s)| {
                let mut extra = Vec::new();
                let mut any_output = s.get().parts.iter().any(|p| !p.outputs.is_empty());
                for t in s.get().tasks.iter() {
                    let Some(tn) = self.file_node(si, t.file.to_native()) else {
                        continue;
                    };
                    any_output |= !t.outputs.is_empty();
                    for d in t.deps.iter() {
                        if let Some(dn) = self.lookup(d.as_str())
                            && dn != tn.0
                            && self.is_task(NodeId(dn))
                        {
                            extra.push((tn.0, dn));
                        }
                    }
                }
                (extra, any_output)
            })
            .collect();
        let any_output = deps.iter().any(|d| d.1);
        extra.extend(deps.into_iter().flat_map(|d| d.0));
        if any_output {
            // Committed files of tasks that nothing runs are not linked (`tracked`).
            let tracked = tracked::Tracked::new(self, out, &extra);
            let tracked = &tracked;
            let t0 = Instant::now();
            // With the tables built here (the usual case: they have the composed graph's
            // key), the probe paths of every shard record are at hand as string ids.
            let mut probe_ids: Option<ProbeIds> = None;
            let t = self.tables_with(Some(&mut probe_ids));
            let probe_ids = probe_ids.as_ref();
            let inv = self.inverted();
            turborepo_trace_core::timing::mark("load: tables + inverted indexes (for task links)");
            tables_us = t0.elapsed().as_micros() as u64;
            // Outputs in order (their indexes are stable), then linked in parallel.
            let mut work: Vec<(u32, u32)> = Vec::new();
            // A task part defined by several shards (every `tsc -b` that builds the
            // project) is one node: each of its outputs is linked once.
            let mut part_outputs: HashSet<(u32, GenOutput)> = HashSet::new();
            for (si, s) in self.shards.iter().enumerate() {
                let a = s.get();
                let root = a.package.root.as_str();
                let tasks = a
                    .tasks
                    .iter()
                    .map(|t| (t.file.to_native(), &t.outputs, false));
                let parts = a
                    .parts
                    .iter()
                    .map(|p| (p.file.to_native(), &p.outputs, true));
                for (file, outputs, part) in tasks.chain(parts) {
                    let Some(tn) = self.file_node(si, file) else {
                        continue;
                    };
                    for o in outputs.iter() {
                        let Some(p) = join_repo(root, o.path.as_str()) else {
                            continue;
                        };
                        let out: GenOutput = (
                            p,
                            o.pattern.as_ref().map(|x| x.to_string()),
                            o.exclude.iter().map(|x| x.to_string()).collect(),
                            o.package.as_ref().map(|x| x.to_string()),
                        );
                        if part && !part_outputs.insert((tn.0, out.clone())) {
                            continue;
                        }
                        work.push((tn.0, gen_outputs.len() as u32));
                        gen_outputs.push(out);
                    }
                }
            }
            type Pairs = Vec<(u32, u32)>;
            type Links = (Pairs, Pairs, Pairs, Pairs, Vec<u32>);
            let gen_ref = &gen_outputs;
            let linked: Vec<Links> = work
                .par_iter()
                .map(|&(tn, oi)| {
                    let mut extra: Vec<(u32, u32)> = Vec::new();
                    let mut generated: Vec<(u32, u32)> = Vec::new();
                    let mut weak: Vec<(u32, u32)> = Vec::new();
                    let mut listed: Vec<(u32, u32)> = Vec::new();
                    let mut produced: Vec<u32> = Vec::new();
                    let mut may_list_memo: HashMap<
                        (u32, String, Option<String>),
                        bool,
                        xxhash_rust::xxh3::Xxh3Builder,
                    > = HashMap::with_hasher(xxhash_rust::xxh3::Xxh3Builder::new());
                    let (p, pattern, exclude, package) = gen_ref[oi as usize].clone();
                    // A write to a path with an unexpanded variable (`$BUILD_DIR/x`): no
                    // file anyone names; its listing links are listing-only.
                    let unplaced = listed::unplaced(&p);
                    {
                        {
                            // An output package: its opaque nodes (any version).
                            if let Some(name) = &package {
                                let prefix = format!("node_modules/{name}@");
                                for i in self.lower_bound(&prefix)..self.nodes.len() {
                                    if !self.path_of(i).starts_with(prefix.as_str()) {
                                        break;
                                    }
                                    if self.nodes[i].file == OPAQUE {
                                        produced.push(i as u32);
                                        extra.push((i as u32, tn));
                                        generated.push((i as u32, oi));
                                    }
                                }
                            }
                            // `output_covers` with the globs compiled once per output.
                            let inc = pattern.as_deref().map(crate::glob::FastGlob::new);
                            let exc: Vec<crate::glob::FastGlob> = exclude
                                .iter()
                                .map(|e| crate::glob::FastGlob::new(e))
                                .collect();
                            let covers = |path: &str| {
                                let rel = if path == p {
                                    ""
                                } else if p.is_empty() {
                                    path
                                } else if let Some(r) = path
                                    .strip_prefix(p.as_str())
                                    .and_then(|r| r.strip_prefix('/'))
                                {
                                    r
                                } else {
                                    return false;
                                };
                                inc.as_ref().is_none_or(|g| g.matches(rel))
                                    && !exc.iter().any(|g| g.matches(rel))
                            };
                            let under = |path: &str| {
                                path == p
                                    || p.is_empty()
                                    || path
                                        .strip_prefix(p.as_str())
                                        .is_some_and(|r| r.starts_with('/'))
                            };
                            // Nodes covered by the output.
                            // Paths under `p` are a subrange of those starting with `p`, which may
                            // interleave with siblings such as `p.config.js` (`.` sorts before
                            // `/`).
                            let start = self.lower_bound(&p);
                            for i in start..self.nodes.len() {
                                let path = self.path_of(i);
                                if !path.starts_with(p.as_str()) {
                                    break;
                                }
                                if !under(path) {
                                    continue;
                                }
                                if i as u32 != tn
                                    && !self.is_pseudo(i)
                                    && covers(path)
                                    && !tracked.committed(tn, path)
                                {
                                    produced.push(i as u32);
                                    extra.push((i as u32, tn));
                                    generated.push((i as u32, oi));
                                }
                            }
                            // Probes and config inputs of covered paths.
                            let sstart = lower_bound_str(&t.strings, &p);
                            for id in sstart..t.strings.len() {
                                let s = t.strings.get(id as u32);
                                if !s.starts_with(p.as_str()) {
                                    break;
                                }
                                if !under(s) || !covers(s) {
                                    continue;
                                }
                                let (lo, hi) =
                                    (inv.probe_off[id] as usize, inv.probe_off[id + 1] as usize);
                                for &n in &inv.probe_nodes[lo..hi] {
                                    if n != tn && !tracked.passing_probe(self, tn, n, s) {
                                        // A path that only type-level references look for
                                        // (`import type { Api } from './generated/api'`)
                                        // links by a type-only edge.
                                        if self.probed_by_types_only(n, s, id as u32, probe_ids) {
                                            weak.push((n, tn));
                                        } else {
                                            extra.push((n, tn));
                                        }
                                        generated.push((n, oi));
                                    }
                                }
                                let (lo, hi) =
                                    (inv.input_off[id] as usize, inv.input_off[id + 1] as usize);
                                for &n in &inv.input_nodes[lo..hi] {
                                    if n != tn && !tracked.committed(tn, s) {
                                        extra.push((n, tn));
                                        generated.push((n, oi));
                                    }
                                }
                            }
                            // Listings of a directory inside the output (anything listed there is
                            // produced), or containing it when the listing's pattern can match a
                            // file the output produces.
                            let mut dirs: Vec<(u32, Option<String>)> = Vec::new();
                            // A directory inside the output: its listing links only when an
                            // entry it lists can be one the output produces (the output's
                            // pattern and excludes; `None` below: everything there is).
                            let scoped = pattern.is_some() || !exclude.is_empty();
                            let mut inside: Vec<(u32, String)> = Vec::new();
                            for id in sstart..t.strings.len() {
                                let s = t.strings.get(id as u32);
                                if !s.starts_with(p.as_str()) {
                                    break;
                                }
                                if under(s) {
                                    if scoped {
                                        let drel = if s == p {
                                            ""
                                        } else if p.is_empty() {
                                            s
                                        } else {
                                            &s[p.len() + 1..]
                                        };
                                        inside.push((id as u32, drel.to_owned()));
                                    } else {
                                        dirs.push((id as u32, None));
                                    }
                                }
                            }
                            let mut anc = p.as_str();
                            while let Some(i) = anc.rfind('/') {
                                let rel = p[i + 1..].to_string();
                                anc = &anc[..i];
                                if let Some(id) = t.strings.find(anc) {
                                    dirs.push((id, Some(rel)));
                                }
                            }
                            if !p.is_empty()
                                && let Some(id) = t.strings.find("")
                            {
                                dirs.push((id, Some(p.clone())));
                            }
                            for (id, drel) in inside {
                                if !output_may_reach(&drel, pattern.as_deref(), &exclude) {
                                    continue;
                                }
                                let (lo, hi) = (
                                    inv.member_off[id as usize] as usize,
                                    inv.member_off[id as usize + 1] as usize,
                                );
                                for &Pair(n, _) in &inv.member_nodes[lo..hi] {
                                    if n != tn {
                                        extra.push((n, tn));
                                        generated.push((n, oi));
                                    }
                                }
                            }
                            // Members of a directory are grouped by pattern: decide once per
                            // pattern (memoized across outputs: a root `**/__mocks__/**` listing is
                            // recorded on thousands of tests and every output has the root as an
                            // ancestor), then link the whole run.
                            for (id, rel) in dirs {
                                let (lo, hi) = (
                                    inv.member_off[id as usize] as usize,
                                    inv.member_off[id as usize + 1] as usize,
                                );
                                let members = &inv.member_nodes[lo..hi];
                                let mut k = 0;
                                while k < members.len() {
                                    let pat = members[k].1;
                                    let end = k + members[k..].partition_point(|m| m.1 == pat);
                                    let lists = match &rel {
                                        None => true,
                                        Some(rel) => *may_list_memo
                                            .entry((pat, rel.clone(), pattern.clone()))
                                            .or_insert_with(|| {
                                                may_list_output(
                                                    t.strings.get(pat),
                                                    rel,
                                                    pattern.as_deref(),
                                                )
                                            }),
                                    };
                                    if lists {
                                        // A listing that reaches the output through
                                        // wildcards alone links as listing-only.
                                        let names = !unplaced
                                            && rel.as_ref().is_none_or(|rel| {
                                                listed::names_output(
                                                    t.strings.get(pat),
                                                    rel,
                                                    pattern.as_deref(),
                                                )
                                            });
                                        let links = if names { &mut extra } else { &mut listed };
                                        for &Pair(n, _) in &members[k..end] {
                                            if n != tn {
                                                links.push((n, tn));
                                                generated.push((n, oi));
                                            }
                                        }
                                    }
                                    k = end;
                                }
                            }
                        }
                    }
                    (extra, generated, weak, listed, produced)
                })
                .collect();
            for (e, g, w, l, p) in linked {
                extra.extend(e);
                generated.extend(g);
                weak.extend(w);
                listed.extend(l);
                produced.extend(p);
            }
        }
        produced.sort_unstable();
        produced.dedup();
        self.stats.tables_us = tables_us;
        if extra.is_empty() && weak.is_empty() && listed.is_empty() {
            return produced;
        }
        extra.par_sort_unstable();
        extra.dedup();
        // A link that anything else also makes is a full link.
        weak.sort_unstable();
        weak.dedup();
        listed.par_sort_unstable();
        listed.dedup();
        listed.retain(|e| extra.binary_search(e).is_err());
        // A listing link was a full link before it was marked: a pair that a type-level
        // probe makes as well stays one for every entry but tasks (listing-only wins).
        weak.retain(|e| extra.binary_search(e).is_err() && listed.binary_search(e).is_err());
        const MARKS: u32 = TYPE_ONLY_LINK | listed::LISTED_LINK;
        let mut extra: Vec<(u32, u32)> = extra
            .into_iter()
            .chain(weak.into_iter().map(|(a, b)| (a, b | TYPE_ONLY_LINK)))
            .chain(
                listed
                    .into_iter()
                    .map(|(a, b)| (a, b | listed::LISTED_LINK)),
            )
            .collect();
        extra.par_sort_unstable_by_key(|&(a, b)| (a, b & !MARKS));
        generated.par_sort_unstable();
        generated.dedup();
        let mut by_node: Vec<(u32, Vec<u32>)> = Vec::new();
        for (n, o) in generated {
            match by_node.last_mut() {
                Some((m, v)) if *m == n => v.push(o),
                _ => by_node.push((n, vec![o])),
            }
        }
        let n = self.nodes.len();
        let mut off = Vec::with_capacity(n + 1);
        let mut to = Vec::with_capacity(extra.len());
        let mut k = 0;
        off.push(0u32);
        for i in 0..n as u32 {
            while k < extra.len() && extra[k].0 == i {
                to.push(extra[k].1);
                k += 1;
            }
            off.push(to.len() as u32);
        }
        // The forward adjacency with the extra edges appended per node. The
        // listing-only mark lives there alone (`listed`): the stored extra
        // edges keep the type-only mark.
        let listed_attr = listed::listed_attr();
        let attr = EdgeAttr(crate::convert::edge_attr(
            turborepo_trace_core::RefKind::Import,
            turborepo_trace_core::Phase::Runtime,
            turborepo_trace_core::RefFlags::empty(),
        ));
        let type_only = EdgeAttr(crate::convert::edge_attr(
            turborepo_trace_core::RefKind::Import,
            turborepo_trace_core::Phase::Runtime,
            turborepo_trace_core::RefFlags::TYPE_ONLY,
        ));
        let mut fwd_off = Vec::with_capacity(n + 1);
        let mut fwd_to = Vec::new();
        let mut fwd_attr = Vec::new();
        fwd_off.push(0u32);
        for i in 0..n {
            let (ts, ats) = self.fwd.neighbors(NodeId(i as u32));
            fwd_to.extend_from_slice(ts);
            fwd_attr.extend_from_slice(ats);
            for x in &mut to[off[i] as usize..off[i + 1] as usize] {
                fwd_to.push(*x & !MARKS);
                fwd_attr.push(if *x & TYPE_ONLY_LINK != 0 {
                    type_only
                } else if *x & listed::LISTED_LINK != 0 {
                    listed_attr
                } else {
                    attr
                });
                *x &= !listed::LISTED_LINK;
            }
            fwd_off.push(fwd_to.len() as u32);
        }
        self.fwd = Adjacency::from_parts(fwd_off, fwd_to, fwd_attr);
        self.extra_off = off;
        self.extra_to = to;
        self.generated = by_node;
        self.gen_outputs = gen_outputs;
        produced
    }

    /// Whether an uncertainty of node `n` (linked through outputs `outs`) is
    /// replaced by its generated-file link: `generated_file`, or an
    /// unresolved relative specifier whose target lies in one of those
    /// outputs.
    fn replaced_by_link(&self, n: NodeId, outs: &[u32], kind: u8, detail: &str) -> bool {
        if kind == generated_file_index() {
            // Not the record of a read below a temporary directory, which says nothing
            // about how the file itself is made.
            return !detail.ends_with(turborepo_trace_core::output::TEMP_READ_NOTE)
                && !detail.ends_with(turborepo_trace_core::output::MOUNT_READ_NOTE);
        }
        if kind != unresolved_index() {
            return false;
        }
        // A Go import of a package directory that a task generates (`gogen`).
        if let Some(dir) = gogen::missing_package_dir(self.path_of(n.index()), detail, |p| {
            self.node_by_path(p).is_some()
        }) {
            return outs
                .iter()
                .any(|&o| gogen::below(&self.gen_outputs[o as usize].0, &dir));
        }
        // `cannot resolve '<spec>' from <file>: ...`
        let Some(spec) = detail.split('\'').nth(1) else {
            return false;
        };
        if !spec.starts_with("./") && !spec.starts_with("../") {
            return self.bare_spec_linked(outs, spec);
        }
        let from = self.path_of(n.index());
        let dir = from.rsplit_once('/').map_or("", |(d, _)| d);
        let Some(target) = join_repo(dir, spec) else {
            return false;
        };
        const EXTS: &[&str] = &[
            "",
            ".ts",
            ".tsx",
            ".js",
            ".jsx",
            ".mjs",
            ".cjs",
            ".mts",
            ".cts",
            ".d.ts",
            ".json",
            "/index.ts",
            "/index.tsx",
            "/index.js",
        ];
        outs.iter().any(|&o| {
            let (p, pat, ex, _) = &self.gen_outputs[o as usize];
            EXTS.iter()
                .any(|e| output_covers(p, pat.as_deref(), ex, &format!("{target}{e}")))
        })
    }

    /// Whether an unresolved bare specifier (`@prisma/client`,
    /// `@prisma/client/edge`) names an output package of one of the outputs
    /// `outs` a node was linked through.
    fn bare_spec_linked(&self, outs: &[u32], spec: &str) -> bool {
        outs.iter().any(|&o| {
            self.gen_outputs[o as usize]
                .3
                .as_deref()
                .is_some_and(|name| {
                    spec.strip_prefix(name)
                        .is_some_and(|r| r.is_empty() || r.starts_with('/'))
                })
        })
    }

    /// Whether every reference of node `n` that looked for the missing path
    /// `path` is type-level (`import type`, a type-only re-export): then
    /// the file a task produces at that path can only matter to `n` as
    /// types. A reference counts when one of its probes expands to `path`;
    /// the manifests its resolution consulted don't make it a runtime
    /// reference.
    ///
    /// `id` is `path`'s id in the string table. With `probe_ids` (the expansion
    /// of every probe record, kept from building the tables) a reference
    /// looks for `path` when one of its probe records lists `id`. Without,
    /// the records are expanded again and compared as strings: the same
    /// answer, far slower (this runs for every pair of a probed path below
    /// a task output and a node that probes it).
    fn probed_by_types_only(
        &self,
        n: u32,
        path: &str,
        id: u32,
        probe_ids: Option<&ProbeIds>,
    ) -> bool {
        let node = self.nodes[n as usize];
        if node.shard == NONE {
            return false;
        }
        let a = self.shards[node.shard as usize].get();
        let root = a.package.root.as_str();
        let mut scratch = Vec::new();
        let mut any = false;
        for r in a.files[node.file as usize].refs.iter() {
            let looks = match probe_ids {
                Some(ids) => {
                    let recs = &ids[node.shard as usize];
                    r.probes
                        .iter()
                        .any(|pi| recs[pi.to_native() as usize].contains(&id))
                }
                None => r.probes.iter().any(|pi| {
                    scratch.clear();
                    expand_probe(&a.probes[pi.to_native() as usize], &mut scratch);
                    scratch
                        .iter()
                        .any(|rel| join_repo(root, rel).as_deref() == Some(path))
                }),
            };
            if !looks {
                continue;
            }
            let mut refs = r
                .edges
                .iter()
                .map(|e| EdgeAttr(e.attr.to_native()))
                .filter(|a| a.kind() != turborepo_trace_graph::RefKind::Manifest)
                .peekable();
            if refs.peek().is_none()
                || !refs.all(|a| {
                    a.flags()
                        .contains(turborepo_trace_graph::RefFlags::TYPE_ONLY)
                })
            {
                return false;
            }
            any = true;
        }
        any
    }

    /// The first node index whose path is `>= p`.
    fn lower_bound(&self, p: &str) -> usize {
        let (mut lo, mut hi) = (0usize, self.nodes.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.path_of(mid) < p {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }

    fn is_pseudo(&self, i: usize) -> bool {
        let l = self.nodes[i].lang;
        l != u8::MAX && crate::tasks::is_pseudo_lang(Some(self.langs[l as usize].as_str()))
    }

    /// Whether `n` is a build task node (its path is the task id).
    pub fn is_task(&self, n: NodeId) -> bool {
        let l = self.nodes[n.index()].lang;
        l != u8::MAX && self.langs[l as usize] == crate::tasks::TASK_LANG
    }

    /// Every build task node with its record, in node order.
    pub fn tasks(&self) -> Vec<(NodeId, &shard::ArchivedTaskRec)> {
        let mut out = Vec::new();
        for (si, s) in self.shards.iter().enumerate() {
            for t in s.get().tasks.iter() {
                if let Some(n) = self.file_node(si, t.file.to_native()) {
                    out.push((n, t));
                }
            }
        }
        out.sort_by_key(|(n, _)| *n);
        out
    }

    /// The task record of a task node.
    pub fn task(&self, n: NodeId) -> Option<&shard::ArchivedTaskRec> {
        let node = self.nodes[n.index()];
        if node.shard == NONE || !self.is_task(n) {
            return None;
        }
        self.shards[node.shard as usize]
            .get()
            .tasks
            .iter()
            .find(|t| t.file.to_native() == node.file)
    }

    /// The expanded edges of a membership hub node (stored like any hub edge:
    /// file-level, no specifier text, no span).
    fn hub_edges(&self, n: NodeId) -> impl Iterator<Item = Edge<'_>> + '_ {
        let ts: &[u32] = match self.hub_nodes.binary_search_by_key(&n.0, |x| x.0) {
            Ok(i) => &self.hub_targets[self.hub_nodes[i].1 as usize],
            Err(_) => &[],
        };
        let attr = hub_edge_attr();
        ts.iter().map(move |&t| Edge {
            to: Some(NodeId(t)),
            attr,
            line: 0,
            span: None,
            specifier: Some(""),
        })
    }

    /// The task edges of `n` added after linking.
    fn extra_edges(&self, n: NodeId) -> impl Iterator<Item = Edge<'_>> + '_ {
        let range = if self.extra_off.is_empty() {
            0..0
        } else {
            self.extra_off[n.index()] as usize..self.extra_off[n.index() + 1] as usize
        };
        let from_task = self.is_task(n);
        self.extra_to[range].iter().map(move |&t| Edge {
            to: Some(NodeId(t & !TYPE_ONLY_LINK)),
            attr: EdgeAttr(crate::convert::edge_attr(
                turborepo_trace_core::RefKind::Import,
                turborepo_trace_core::Phase::Runtime,
                if t & TYPE_ONLY_LINK != 0 {
                    turborepo_trace_core::RefFlags::TYPE_ONLY
                } else {
                    turborepo_trace_core::RefFlags::empty()
                },
            )),
            line: 0,
            span: None,
            specifier: Some(if from_task {
                "task dependency"
            } else {
                "generated by task"
            }),
        })
    }

    fn lookup(&self, p: &str) -> Option<u32> {
        let mut lo = 0usize;
        let mut hi = self.nodes.len();
        while lo < hi {
            let mid = (lo + hi) / 2;
            match self.path_of(mid).cmp(p) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(mid as u32),
            }
        }
        None
    }

    #[inline]
    fn path_of(&self, i: usize) -> &str {
        let start = if i == 0 {
            0
        } else {
            self.path_end[i - 1] as usize
        };
        &self.paths[start..self.path_end[i] as usize]
    }

    /// The manifest this graph was composed from.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The absolute repository root.
    pub fn root(&self) -> &str {
        &self.root
    }

    /// Number of edges (resolved and unresolved).
    pub fn edge_count(&self) -> usize {
        self.edge_to.len()
            + self.extra_to.len()
            + self
                .hub_nodes
                .iter()
                .map(|&(_, h)| self.hub_targets[h as usize].len())
                .sum::<usize>()
    }

    /// Load timings.
    pub fn load_stats(&self) -> LoadStats {
        self.stats
    }

    /// The location records of the graph ([`crate::shard::LocatedRec`]), each
    /// once and sorted: per project whose runner matches the absolute path
    /// of a file, its config file, its discovery as the config writes it,
    /// and the root the graph holds for it (empty: the place of the
    /// checkout decides nothing for the project).
    pub fn located(&self) -> Vec<(String, String, String)> {
        let mut out: Vec<(String, String, String)> = Vec::new();
        for s in &self.shards {
            for l in s.get().located.iter() {
                out.push((l.config.to_string(), l.spec.to_string(), l.root.to_string()));
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The test files of `framework` (`jest`, `vitest`, `pytest`, `gotest`,
    /// `cargo-test`), or of every graph framework with `None`, in path
    /// order.
    pub fn tests(&self, framework: Option<&str>) -> Vec<NodeId> {
        let mut out = Vec::new();
        for (si, s) in self.shards.iter().enumerate() {
            let a = s.get();
            for t in a.tests.iter() {
                if framework.is_none_or(|f| t.framework.as_str() == f) {
                    let fi = t.file.to_native();
                    if let Some(n) = self.file_node(si, fi) {
                        out.push(n);
                    }
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Frameworks that run `n` as a test.
    pub fn test_frameworks(&self, n: NodeId) -> Vec<&str> {
        let node = self.nodes[n.index()];
        if node.shard == NONE {
            return Vec::new();
        }
        self.shards[node.shard as usize]
            .get()
            .tests
            .iter()
            .filter(|t| t.file.to_native() == node.file)
            .map(|t| t.framework.as_str())
            .collect()
    }

    fn file_node(&self, si: usize, fi: u32) -> Option<NodeId> {
        let a = self.shards[si].get();
        let f = a.files.get(fi as usize)?;
        self.lookup(&node_path(a.package.root.as_str(), f))
            .map(NodeId)
    }

    /// The content hash recorded for a traced file node (xxh3-128), if any.
    pub fn file_hash(&self, n: NodeId) -> Option<u128> {
        let h = self.file(n)?.hash.as_ref()?;
        Some(crate::hash::join([h[0].to_native(), h[1].to_native()]))
    }

    /// Whether `n` is an opaque package node.
    pub fn is_opaque(&self, n: NodeId) -> bool {
        self.nodes[n.index()].file == OPAQUE
    }

    /// The integrity a third-party package's shard records (lockfile hash or
    /// source pin).
    pub fn package_integrity(&self, p: PackageId) -> Option<&str> {
        self.shards
            .get(p.0 as usize)?
            .get()
            .package
            .integrity
            .as_ref()
            .map(|i| i.as_str())
    }

    /// The shard key of package `p`.
    pub fn package_key(&self, p: PackageId) -> Option<&str> {
        self.manifest
            .shards
            .get(p.0 as usize)
            .map(|e| e.key.as_str())
    }

    /// Expands probes and memberships and builds their inverted indexes now
    /// (otherwise done on first use).
    pub fn warm(&self) {
        let _ = self.inverted();
    }

    fn tables(&self) -> &Tables {
        self.tables_with(None)
    }

    /// [`Graph::tables`]. When this call builds them (they were neither built
    /// before nor cached on disk), `probe_ids` receives the expansion of
    /// every shard's probe records.
    ///
    /// The inverted indexes come with the tables: both are mapped from the
    /// cache file, or built here and written to it (in the background: the
    /// query does not wait for 100 MB to reach the page cache; the process
    /// does before it exits).
    fn tables_with(&self, probe_ids: Option<&mut Option<ProbeIds>>) -> &Tables {
        crate::once::init_once(&self.tables, || {
            static READ: TimeSpan = TimeSpan::new("tables: map + check the cache");
            static BUILD: TimeSpan = TimeSpan::new("tables: build");
            static INVERT: TimeSpan = TimeSpan::new("tables: invert");
            static WRITE: TimeSpan = TimeSpan::new("tables: encode the cache");
            let key = self.tables_key();
            let tr = READ.enter();
            if let Some(p) = &self.tables_cache
                && let Some((t, inv)) = tables::open(p, key, self.nodes.len())
            {
                // Published before the tables: whoever sees the tables finds the indexes.
                let _ = self.inverted.set(inv);
                return t;
            }
            drop(tr);
            let tb = BUILD.enter();
            let (t, ids) = self.build_tables();
            drop(tb);
            if let Some(out) = probe_ids {
                *out = Some(ids);
            }
            let ti = INVERT.enter();
            let inv = Inverted::of(&t);
            drop(ti);
            if let Some(p) = &self.tables_cache {
                let tw = WRITE.enter();
                let bytes = tables::encode(&t, &inv, key);
                drop(tw);
                if let Some(bytes) = bytes {
                    let p = p.clone();
                    // The write is one of the graph directory's.
                    let out = self.graph_dir.clone().unwrap_or_else(|| p.clone());
                    crate::spawn_cache_write(
                        &out,
                        move || {
                            let _ = std::fs::create_dir_all(p.parent().unwrap_or(Path::new(".")));
                            let _ = crate::write_atomic(&p, &bytes);
                        },
                        || {},
                    );
                }
            }
            let _ = self.inverted.set(inv);
            t
        })
    }

    /// What the tables are a function of: the shard keys in order.
    fn tables_key(&self) -> u128 {
        Graph::tables_key_of(&self.manifest, self.nodes.len())
    }

    /// [`Graph::tables_key`] of a graph of `nodes` nodes composed from
    /// `manifest`.
    fn tables_key_of(manifest: &Manifest, nodes: usize) -> u128 {
        let mut h = xxhash_rust::xxh3::Xxh3::new();
        h.update(TABLES_MAGIC);
        h.update(&(nodes as u64).to_le_bytes());
        for e in &manifest.shards {
            h.update(e.key.as_bytes());
            h.update(&[0]);
        }
        h.digest128()
    }

    // Invariant: the inverted indexes are set with the tables.
    #[allow(clippy::expect_used)]
    fn inverted(&self) -> &Inverted {
        // Filled with the tables (`tables_with`), before they are published.
        self.tables();
        self.inverted
            .get()
            .expect("the inverted indexes are set with the tables")
    }

    // Invariant: every looked-up string was interned into `all` above.
    #[allow(clippy::expect_used)]
    fn build_tables(&self) -> (Tables, ProbeIds) {
        // 1. Per shard, in parallel: each probe and membership record expanded once
        //    into repo-relative strings (records outside the repository expand to
        //    nothing).
        struct Expanded {
            probes: Vec<Vec<String>>,
            members: Vec<Option<(String, String)>>,
            configs: Vec<Option<String>>,
        }
        let expanded: Vec<Expanded> = self
            .shards
            .par_iter()
            .map(|s| {
                let a = s.get();
                let root = a.package.root.as_str();
                // Also in parallel inside a shard: a repository with a few very large
                // packages has most of its records in a handful of shards.
                let probes = a
                    .probes
                    .as_slice()
                    .par_iter()
                    .with_min_len(PAR_RECORDS)
                    .map_init(Vec::new, |scratch, p| {
                        scratch.clear();
                        expand_probe(p, scratch);
                        scratch
                            .iter()
                            .filter_map(|rel| join_repo(root, rel))
                            .collect()
                    })
                    .collect();
                let members = a
                    .memberships
                    .iter()
                    .map(|m| Some((join_repo(root, m.dir.as_str())?, m.pattern.to_string())))
                    .collect();
                let configs = a
                    .configs
                    .iter()
                    // A manifest's resolution view is the manifest, for queries.
                    .map(|c| join_repo(root, crate::normalize::config_path(c.as_str())))
                    .collect();
                Expanded {
                    probes,
                    members,
                    configs,
                }
            })
            .collect();
        turborepo_trace_core::timing::mark("tables: records expanded");
        // 2. One sorted string table: ids follow string order, so every list sorted by
        //    id is sorted by path.
        let mut all: Vec<&str> = expanded
            .par_iter()
            .flat_map_iter(|e| {
                e.probes
                    .iter()
                    .flatten()
                    .map(String::as_str)
                    .chain(
                        e.members
                            .iter()
                            .flatten()
                            .flat_map(|(d, p)| [d.as_str(), p.as_str()]),
                    )
                    .chain(e.configs.iter().flatten().map(String::as_str))
            })
            .collect();
        let strings = all.len();
        all.par_sort_unstable();
        all.dedup();
        if turborepo_trace_core::timing::enabled() {
            turborepo_trace_core::timing::mark(&format!(
                "tables: {strings} strings sorted, {} distinct",
                all.len()
            ));
        }
        let id = |s: &str| all.binary_search(&s).expect("interned") as u32;
        type RecIds = (Vec<Vec<u32>>, Vec<Option<(u32, u32)>>, Vec<Option<u32>>);
        let rec_ids: Vec<RecIds> = expanded
            .par_iter()
            .map(|e| {
                (
                    e.probes
                        .par_iter()
                        .with_min_len(PAR_RECORDS)
                        .map(|v| v.iter().map(|s| id(s)).collect())
                        .collect(),
                    e.members
                        .iter()
                        .map(|m| m.as_ref().map(|(d, p)| (id(d), id(p))))
                        .collect(),
                    e.configs.iter().map(|c| c.as_deref().map(id)).collect(),
                )
            })
            .collect();
        turborepo_trace_core::timing::mark("tables: record ids");
        // 3. Per node, in parallel: the union over its references.
        type NodeLists = (Vec<u32>, Vec<(u32, u32)>, Vec<u32>);
        let per_node: Vec<NodeLists> = self
            .nodes
            .par_iter()
            .map(|node| {
                let mut ids = Vec::new();
                let mut mids = Vec::new();
                let mut cids = Vec::new();
                if node.shard != NONE {
                    let si = node.shard as usize;
                    let (pr, mr, cr) = &rec_ids[si];
                    for r in self.shards[si].get().files[node.file as usize].refs.iter() {
                        for ci in r.configs.iter() {
                            if let Some(c) = cr[ci.to_native() as usize] {
                                cids.push(c);
                            }
                        }
                        for pi in r.probes.iter() {
                            ids.extend_from_slice(&pr[pi.to_native() as usize]);
                        }
                        for mi in r.memberships.iter() {
                            if let Some(m) = mr[mi.to_native() as usize] {
                                mids.push(m);
                            }
                        }
                    }
                }
                ids.sort_unstable();
                ids.dedup();
                mids.sort_unstable();
                mids.dedup();
                cids.sort_unstable();
                cids.dedup();
                (ids, mids, cids)
            })
            .collect();
        turborepo_trace_core::timing::mark("tables: per-node lists");
        let (mut probe_off, mut probes) = (vec![0u32], Vec::new());
        let (mut member_off, mut members) = (vec![0u32], Vec::new());
        let (mut input_off, mut inputs) = (vec![0u32], Vec::new());
        for (ids, mids, cids) in per_node {
            probes.extend_from_slice(&ids);
            probe_off.push(probes.len() as u32);
            members.extend(mids.iter().map(|&(d, p)| Pair(d, p)));
            member_off.push(members.len() as u32);
            inputs.extend_from_slice(&cids);
            input_off.push(inputs.len() as u32);
        }
        let t = Tables {
            strings: Interned::new(&all),
            probe_off: probe_off.into(),
            probes: probes.into(),
            member_off: member_off.into(),
            members: members.into(),
            input_off: input_off.into(),
            inputs: inputs.into(),
        };
        let ids = rec_ids.into_iter().map(|r| r.0).collect();
        // Over a million small strings: freed off the caller's critical path.
        drop(all);
        rayon::spawn(move || drop(expanded));
        turborepo_trace_core::timing::mark("tables: built");
        (t, ids)
    }

    fn file(&self, n: NodeId) -> Option<&shard::ArchivedFileRec> {
        let node = self.nodes[n.index()];
        (node.shard != NONE)
            .then(|| &self.shards[node.shard as usize].get().files[node.file as usize])
    }
}

/// The node path of a shard file: its repo path, or a pseudo-file's id.
fn node_path(root: &str, f: &shard::ArchivedFileRec) -> String {
    if crate::tasks::is_pseudo_lang(f.lang.as_ref().map(|l| l.as_str())) {
        crate::tasks::pseudo_node_path(f.path.as_str()).to_owned()
    } else {
        join_simple(root, f.path.as_str())
    }
}

/// The first string id `>= p` in a sorted string table.
fn lower_bound_str(t: &Interned, p: &str) -> usize {
    let (mut lo, mut hi) = (0usize, t.len());
    while lo < hi {
        let mid = (lo + hi) / 2;
        if t.get(mid as u32) < p {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo
}

/// Whether a listing with glob `pattern` of a directory that contains an output
/// at `rel` (relative to the listed directory) can match a file the output
/// produces. Tested with representative names (common source extensions, and
/// the output's own pattern), so that unrelated listings (`**/__mocks__/**`
/// next to `dist/`) are not linked.
fn may_list_output(pattern: &str, rel: &str, out_pattern: Option<&str>) -> bool {
    const NAMES: &[&str] = &[
        "x",
        "x.ts",
        "x.tsx",
        "x.js",
        "x.jsx",
        "x.mjs",
        "x.cjs",
        "x.mts",
        "x.cts",
        "x.d.ts",
        "x.json",
        "x.go",
        "x.py",
        "x.pyi",
        "x.rs",
        "x.css",
        "x.scss",
        "x.graphql",
        "x.proto",
        "x.sql",
        "x.yaml",
        "x.html",
        "x.snap",
        "x.node",
        "x.wasm",
        "x.map",
    ];
    let derived = out_pattern.map(|op| {
        op.replace("**/", "")
            .replace("**", "x")
            .replace(['*', '?'], "x")
    });
    let names = || NAMES.iter().copied().chain(derived.as_deref());
    let rel_last = rel.rsplit('/').next().unwrap_or(rel);
    let mut buf = String::new();
    // `matches(pattern, c)` is `matches(alt, c)` for some brace alternative `alt`
    // (each is a fixed point of the expansion), so each alternative is tested
    // on its own, after exact prefilters (necessary conditions; the glob runs
    // only on what passes). The candidates are `rel`, `rel/<name>` and
    // `rel/x/<name>`. Without escapes, a segment with no wildcard matches
    // exactly one whole path component (when every `**` is a whole segment: `a*
    // */b` matches `ab`), so it must be a component of `rel`, `x` or a name; and
    // the last segment's literal tail (after its last wildcard) must end the
    // candidate.
    turborepo_trace_tasks::glob::expand_braces(pattern)
        .iter()
        .any(|alt| {
            let pat = alt.strip_prefix("./").unwrap_or(alt);
            let mut tail = "";
            // Without `**`, every segment matches exactly one component: only candidates
            // with as many components as the pattern has segments can match.
            let mut comps: Option<usize> = None;
            if !pat.contains(['{', '}', '\\']) {
                if !pat.contains("**") && !pat.contains(['[', ']']) {
                    comps = Some(pat.split('/').count());
                }
                let wild = |c: char| matches!(c, '*' | '?' | '[' | ']');
                let aligned = pat.split('/').all(|seg| seg == "**" || !seg.contains("**"));
                for seg in pat.split('/').filter(|_| aligned) {
                    if seg.is_empty() || seg.contains(wild) {
                        continue;
                    }
                    if seg != "x" && !rel.split('/').any(|c| c == seg) && !names().any(|n| n == seg)
                    {
                        return false;
                    }
                }
                let last = pat.rsplit('/').next().unwrap_or(pat);
                if !last.contains("**") {
                    tail = last.rfind(wild).map_or(last, |i| &last[i + 1..]);
                }
            }
            let k = rel.split('/').count();
            let fits = |n: usize| comps.is_none_or(|c| c == n);
            if fits(k) && rel_last.ends_with(tail) && turborepo_trace_tasks::glob::matches(alt, rel)
            {
                return true;
            }
            if !fits(k + 1) && !fits(k + 2) {
                return false;
            }
            names().filter(|n| n.ends_with(tail)).any(|n| {
                if fits(k + 1) {
                    buf.clear();
                    buf.push_str(rel);
                    buf.push('/');
                    buf.push_str(n);
                    if turborepo_trace_tasks::glob::matches(alt, &buf) {
                        return true;
                    }
                }
                if !fits(k + 2) {
                    return false;
                }
                buf.clear();
                buf.push_str(rel);
                buf.push_str("/x/");
                buf.push_str(n);
                turborepo_trace_tasks::glob::matches(alt, &buf)
            })
        })
}

/// For a listing of the directory at `drel` inside an output (relative to the
/// output's base), whether the output can produce anything in that directory:
/// `false` only when that is provably impossible, because every alternative of
/// the output's pattern diverges from `drel` before reaching it, or an exclude
/// of the form `<prefix>/**` (or `**`) covers the whole directory. Anything
/// else links, as before (sound).
fn output_may_reach(drel: &str, out_pattern: Option<&str>, exclude: &[String]) -> bool {
    use turborepo_trace_tasks::glob::{expand_braces, matches};
    let comps: Vec<&str> = if drel.is_empty() {
        Vec::new()
    } else {
        drel.split('/').collect()
    };
    // One glob segment against one path component (no `/` on either side).
    let seg_matches = |seg: &str, comp: &str| -> bool {
        if seg.contains(['[', ']', '\\']) {
            return true;
        }
        matches(seg, comp)
    };
    // Whether `alt` can match a path at or below `drel`.
    let reaches = |alt: &str| -> bool {
        let alt = alt.strip_prefix("./").unwrap_or(alt);
        let segs: Vec<&str> = alt.split('/').collect();
        for (i, comp) in comps.iter().enumerate() {
            let Some(seg) = segs.get(i) else {
                // The pattern ends above the directory: it names the directory or an
                // ancestor, never an entry inside it.
                return false;
            };
            if seg.contains("**") {
                return true;
            }
            if !seg_matches(seg, comp) {
                return false;
            }
        }
        segs.len() > comps.len() || segs.last().is_some_and(|s| s.contains("**"))
    };
    if let Some(op) = out_pattern
        && !expand_braces(op).iter().any(|a| reaches(a))
    {
        return false;
    }
    // Whether exclude alternative `alt` (`<prefix>/**`) excludes every path below
    // `drel`.
    let covers_dir = |alt: &str| -> bool {
        let alt = alt.strip_prefix("./").unwrap_or(alt);
        let Some(prefix) = alt.strip_suffix("**") else {
            return false;
        };
        let prefix = prefix.strip_suffix('/').unwrap_or(prefix);
        if prefix.is_empty() {
            return alt == "**";
        }
        if prefix.contains("**") || prefix.contains(['[', ']', '\\']) {
            return false;
        }
        let psegs: Vec<&str> = prefix.split('/').collect();
        psegs.len() <= comps.len() && psegs.iter().zip(&comps).all(|(s, c)| matches(s, c))
    };
    !exclude
        .iter()
        .any(|e| expand_braces(e).iter().all(|a| covers_dir(a)))
}

/// Whether the output at repo-relative `p` (with an optional glob and excludes
/// below it) covers the repo-relative `path`.
fn output_covers(p: &str, pattern: Option<&str>, exclude: &[String], path: &str) -> bool {
    let rel = if path == p {
        ""
    } else if p.is_empty() {
        path
    } else if let Some(r) = path.strip_prefix(p).and_then(|r| r.strip_prefix('/')) {
        r
    } else {
        return false;
    };
    let inc = pattern.is_none_or(|pat| turborepo_trace_tasks::glob::matches(pat, rel));
    inc && !exclude
        .iter()
        .any(|e| turborepo_trace_tasks::glob::matches(e, rel))
}

/// The stored index of the `unresolved_specifier` uncertainty kind.
fn unresolved_index() -> u8 {
    crate::convert::UNCERTAINTY_KINDS
        .iter()
        .position(|k| *k == turborepo_trace_core::UncertaintyKind::UnresolvedSpecifier)
        .unwrap_or(usize::MAX) as u8
}

/// The stored index of the `generated_file` uncertainty kind.
fn generated_file_index() -> u8 {
    crate::convert::UNCERTAINTY_KINDS
        .iter()
        .position(|k| *k == turborepo_trace_core::UncertaintyKind::GeneratedFile)
        .unwrap_or(usize::MAX) as u8
}

/// Attributes of a membership hub's edges ([`crate::tasks`]: a build-phase fs
/// read).
fn hub_edge_attr() -> EdgeAttr {
    EdgeAttr(crate::convert::edge_attr(
        turborepo_trace_core::RefKind::FsRead,
        turborepo_trace_core::Phase::Build,
        turborepo_trace_core::RefFlags::empty(),
    ))
}

/// Distinct membership hubs of a snapshot with their members.
struct Hubs {
    /// The snapshot's walked files (repo-relative, sorted).
    files: Vec<String>,
    /// Member paths per distinct hub: indexes into `files`, sorted.
    lists: Vec<Vec<u32>>,
    /// Per shard, per [`crate::shard::HubRec`]: the index into `lists`.
    of_shard: Vec<Vec<u32>>,
}

/// Expands every distinct hub of `shards` against the snapshot's walked files.
fn expand_hubs(out: &Path, manifest: &Manifest, shards: &[LoadedShard]) -> Result<Hubs, Error> {
    let mut of_shard: Vec<Vec<u32>> = vec![Vec::new(); shards.len()];
    if shards.iter().all(|s| s.get().hubs.is_empty()) {
        return Ok(Hubs {
            files: Vec::new(),
            lists: Vec::new(),
            of_shard,
        });
    }
    let store = out.join("store");
    let files = manifest
        .files
        .as_deref()
        .and_then(|n| crate::manifest::read_files(&store, n))
        .ok_or_else(|| Error::NotBuilt(out.to_path_buf()))?;
    type Key<'a> = (&'a str, &'a str, Vec<&'a str>, Vec<&'a str>, bool);
    // The package roots, for hubs of a package's own files (`crate::own`).
    let nested = crate::own::Nested::new(
        shards
            .iter()
            .map(|s| s.get().package.root.as_str().to_owned()),
    );
    let mut keys: Vec<Key<'_>> = Vec::new();
    let mut ix: HashMap<Key<'_>, u32> = HashMap::new();
    for (si, s) in shards.iter().enumerate() {
        for h in s.get().hubs.iter() {
            let k: Key<'_> = (
                h.dir.as_str(),
                h.pattern.as_str(),
                h.exclude.iter().map(|e| e.as_str()).collect(),
                h.skip.iter().map(|e| e.as_str()).collect(),
                h.own,
            );
            let i = *ix.entry(k.clone()).or_insert_with(|| {
                keys.push(k);
                keys.len() as u32 - 1
            });
            of_shard[si].push(i);
        }
    }
    let lists: Vec<Vec<u32>> = keys
        .par_iter()
        .map(|(dir, pattern, exclude, skip, own)| {
            let prefix = if dir.is_empty() {
                String::new()
            } else {
                format!("{dir}/")
            };
            // The files below `dir` are one contiguous run of the sorted walk.
            let start = files.partition_point(|f| f.as_str() < prefix.as_str());
            let len = files[start..].partition_point(|f| f.starts_with(prefix.as_str()));
            let pat = crate::glob::FastGlob::new(pattern);
            let exc: Vec<crate::glob::FastGlob> = exclude
                .iter()
                .map(|e| crate::glob::FastGlob::new(e))
                .collect();
            // A repo-wide hub scans every walked file: in parallel chunks, in order.
            (start..start + len)
                .into_par_iter()
                .with_min_len(4096)
                .filter(|&i| {
                    let f = files[i].as_str();
                    let rel = &f[prefix.len()..];
                    pat.matches(rel)
                        && !exc.iter().any(|e| e.matches(rel))
                        && skip.binary_search(&f).is_err()
                        && !(*own && nested.inside(dir, f))
                })
                .map(|i| i as u32)
                .collect()
        })
        .collect();
    Ok(Hubs {
        files,
        lists,
        of_shard,
    })
}

fn span_of(s: &rkyv::Archived<[u32; 2]>) -> Option<Span> {
    let (start, end) = (s[0].to_native(), s[1].to_native());
    (start != 0 || end != 0).then_some(Span { start, end })
}

impl GraphView for Graph {
    fn node_count(&self) -> u32 {
        self.nodes.len() as u32
    }

    fn path(&self, n: NodeId) -> &str {
        self.path_of(n.index())
    }

    fn package(&self, n: NodeId) -> PackageId {
        PackageId(self.nodes[n.index()].pkg)
    }

    fn lang(&self, n: NodeId) -> Option<&str> {
        let l = self.nodes[n.index()].lang;
        (l != u8::MAX).then(|| self.langs[l as usize].as_str())
    }

    fn is_external(&self, n: NodeId) -> bool {
        self.packages[self.nodes[n.index()].pkg as usize].external
    }

    fn package_count(&self) -> u32 {
        self.packages.len() as u32
    }

    fn package_info(&self, p: PackageId) -> PackageInfo<'_> {
        let pkg = &self.packages[p.0 as usize];
        PackageInfo {
            name: &pkg.name,
            version: match &pkg.version {
                Some(v) => PackageVersion::Version(v),
                None => PackageVersion::Workspace,
            },
            root: &pkg.root,
        }
    }

    fn out_edges(&self, n: NodeId) -> impl Iterator<Item = Edge<'_>> + '_ {
        let lo = self.edge_off[n.index()] as usize;
        let hi = self.edge_off[n.index() + 1] as usize;
        let to = &self.edge_to[lo..hi];
        self.file(n)
            .into_iter()
            .flat_map(|f| {
                f.refs
                    .iter()
                    .flat_map(|r| r.edges.iter().map(move |e| (r.line.to_native(), e)))
            })
            .zip(to)
            .map(|((line, e), &to)| Edge {
                to: (to != NONE).then_some(NodeId(to)),
                attr: EdgeAttr(e.attr.to_native()),
                line,
                span: span_of(&e.span),
                specifier: Some(e.spec.as_str()),
            })
            .chain(self.hub_edges(n))
            .chain(self.extra_edges(n))
    }

    fn uncertainties(&self, n: NodeId) -> impl Iterator<Item = Uncertainty<'_>> + '_ {
        let linked = self
            .generated
            .binary_search_by_key(&n.0, |(m, _)| *m)
            .ok()
            .map(|i| &self.generated[i].1);
        let node = self.nodes[n.index()];
        let shard = (node.shard != NONE).then(|| self.shards[node.shard as usize].get());
        shard
            .into_iter()
            .flat_map(move |s| {
                s.files[node.file as usize]
                    .uncertainties
                    .iter()
                    .map(move |u| (u, s.details[u.detail.to_native() as usize].as_str()))
            })
            .filter(move |(u, detail)| match linked {
                None => true,
                Some(outs) => !self.replaced_by_link(n, outs, u.kind, detail),
            })
            .map(|(u, detail)| Uncertainty {
                kind: query_unc_kind(u.kind),
                line: u.line.to_native(),
                span: span_of(&u.span),
                detail,
                scope: u.scope.as_ref().map(|s| Membership {
                    dir: s.dir.as_str(),
                    pattern: s.pattern.as_str(),
                }),
                effect: crate::convert::query_effect(u.effect),
                origin: crate::convert::query_origin(u.origin),
            })
    }

    fn has_uncertainty(&self, n: NodeId) -> bool {
        if self
            .generated
            .binary_search_by_key(&n.0, |(m, _)| *m)
            .is_ok()
        {
            return self.uncertainties(n).next().is_some();
        }
        self.file(n).is_some_and(|f| !f.uncertainties.is_empty())
    }

    fn probes(&self, n: NodeId) -> impl Iterator<Item = &str> + '_ {
        let t = self.tables();
        let (lo, hi) = (
            t.probe_off[n.index()] as usize,
            t.probe_off[n.index() + 1] as usize,
        );
        t.probes[lo..hi].iter().map(move |&i| t.strings.get(i))
    }

    fn memberships(&self, n: NodeId) -> impl Iterator<Item = Membership<'_>> + '_ {
        let t = self.tables();
        let (lo, hi) = (
            t.member_off[n.index()] as usize,
            t.member_off[n.index() + 1] as usize,
        );
        t.members[lo..hi].iter().map(move |&Pair(d, p)| Membership {
            dir: t.strings.get(d),
            pattern: t.strings.get(p),
        })
    }

    fn inputs(&self, n: NodeId) -> impl Iterator<Item = &str> + '_ {
        let t = self.tables();
        let (lo, hi) = (
            t.input_off[n.index()] as usize,
            t.input_off[n.index() + 1] as usize,
        );
        t.inputs[lo..hi].iter().map(move |&i| t.strings.get(i))
    }

    fn node_by_path(&self, path: &str) -> Option<NodeId> {
        self.lookup(path).map(NodeId)
    }

    fn has_lookups(&self) -> bool {
        true
    }

    fn probe_lookup(&self, path: &str, f: &mut dyn FnMut(NodeId, &str)) {
        let t = self.tables();
        let Some(id) = t.strings.find(path) else {
            return;
        };
        let inv = self.inverted();
        let (lo, hi) = (
            inv.probe_off[id as usize] as usize,
            inv.probe_off[id as usize + 1] as usize,
        );
        let p = t.strings.get(id);
        for &n in &inv.probe_nodes[lo..hi] {
            f(NodeId(n), p);
        }
    }

    fn probe_lookup_below(&self, path: &str, f: &mut dyn FnMut(NodeId, &str)) {
        let t = self.tables();
        let ids = t.strings.starting_with(&format!("{path}/"));
        if ids.is_empty() {
            return;
        }
        // (Of the strings below the path, the ones no node probes have no nodes here.)
        let inv = self.inverted();
        for id in ids {
            let (lo, hi) = (
                inv.probe_off[id as usize] as usize,
                inv.probe_off[id as usize + 1] as usize,
            );
            let p = t.strings.get(id);
            for &n in &inv.probe_nodes[lo..hi] {
                f(NodeId(n), p);
            }
        }
    }

    fn membership_lookup(&self, dir: &str, f: &mut dyn FnMut(NodeId, &str, &str)) {
        let t = self.tables();
        let Some(id) = t.strings.find(dir) else {
            return;
        };
        let inv = self.inverted();
        let (lo, hi) = (
            inv.member_off[id as usize] as usize,
            inv.member_off[id as usize + 1] as usize,
        );
        let d = t.strings.get(id);
        for &Pair(n, pat) in &inv.member_nodes[lo..hi] {
            f(NodeId(n), d, t.strings.get(pat));
        }
    }

    fn input_lookup(&self, path: &str, f: &mut dyn FnMut(NodeId)) {
        let t = self.tables();
        let Some(id) = t.strings.find(path) else {
            return;
        };
        let inv = self.inverted();
        let (lo, hi) = (
            inv.input_off[id as usize] as usize,
            inv.input_off[id as usize + 1] as usize,
        );
        for &n in &inv.input_nodes[lo..hi] {
            f(NodeId(n));
        }
    }

    fn forward_adjacency(&self) -> Option<&Adjacency> {
        Some(&self.fwd)
    }
}

#[cfg(test)]
mod tests {
    use super::{may_list_output, output_may_reach};

    #[test]
    fn listings_inside_an_output_respect_its_pattern_and_excludes() {
        let ex = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // No pattern, no excludes: everything inside.
        assert!(output_may_reach("src/__mocks__", None, &[]));
        // The pattern decides which directories can hold outputs.
        assert!(output_may_reach("", Some("dist/**"), &[]));
        assert!(output_may_reach("dist", Some("dist/**"), &[]));
        assert!(output_may_reach("dist/chunks", Some("dist/**"), &[]));
        assert!(!output_may_reach("src", Some("dist/**"), &[]));
        assert!(!output_may_reach(
            "src/__mocks__",
            Some("{dist,build}/**"),
            &[]
        ));
        assert!(output_may_reach("build/x", Some("{dist,build}/**"), &[]));
        assert!(output_may_reach("a/b/gen", Some("**/gen/*.ts"), &[]));
        assert!(output_may_reach("pkg-a/dist", Some("*/dist/*.js"), &[]));
        assert!(!output_may_reach("pkg-a/src", Some("*/dist/*.js"), &[]));
        // A pattern naming the directory itself or an ancestor produces no entry inside
        // it.
        assert!(!output_may_reach("dist/x", Some("dist"), &[]));
        assert!(output_may_reach("x", Some("[ab]/**"), &[]), "classes: link");
        // Excludes of whole subtrees (nested packages excluded from `apps/**`).
        let nested = ex(&["web/**", "docs/**"]);
        assert!(!output_may_reach("web", Some("**"), &nested));
        assert!(!output_may_reach("web/src/__mocks__", Some("**"), &nested));
        assert!(output_may_reach("tools", Some("**"), &nested));
        assert!(output_may_reach("", Some("**"), &nested), "the base itself");
        assert!(!output_may_reach("a/b", None, &ex(&["**"])));
        assert!(!output_may_reach("pkgs/x/src", None, &ex(&["pkgs/*/**"])));
        // Partial excludes never prove a directory empty.
        assert!(output_may_reach("web", None, &ex(&["web/**/*.ts"])));
        assert!(output_may_reach("web", None, &ex(&["**/*.map"])));
    }

    /// The original `may_list_output`: every representative name, no
    /// prefilters.
    fn reference(pattern: &str, rel: &str, out_pattern: Option<&str>) -> bool {
        const NAMES: &[&str] = &[
            "x",
            "x.ts",
            "x.tsx",
            "x.js",
            "x.jsx",
            "x.mjs",
            "x.cjs",
            "x.mts",
            "x.cts",
            "x.d.ts",
            "x.json",
            "x.go",
            "x.py",
            "x.pyi",
            "x.rs",
            "x.css",
            "x.scss",
            "x.graphql",
            "x.proto",
            "x.sql",
            "x.yaml",
            "x.html",
            "x.snap",
            "x.node",
            "x.wasm",
            "x.map",
        ];
        let mut names: Vec<String> = NAMES.iter().map(|n| n.to_string()).collect();
        if let Some(op) = out_pattern {
            names.push(
                op.replace("**/", "")
                    .replace("**", "x")
                    .replace(['*', '?'], "x"),
            );
        }
        if turborepo_trace_tasks::glob::matches(pattern, rel) {
            return true;
        }
        names.iter().any(|n| {
            turborepo_trace_tasks::glob::matches(pattern, &format!("{rel}/{n}"))
                || turborepo_trace_tasks::glob::matches(pattern, &format!("{rel}/x/{n}"))
        })
    }

    #[test]
    fn prefilters_are_exact() {
        let patterns = [
            "**",
            "*",
            "**/*",
            "**/__mocks__/**",
            "__mocks__/**",
            "**/__snapshots__/*.snap",
            "**/*.snap",
            "*.snap",
            "src/**",
            "src/**/*.ts",
            "src/*.{ts,tsx}",
            "dist",
            "dist/**",
            "./dist/**",
            "x",
            "x/**",
            "x.ts",
            "**/x.ts",
            "a**/b",
            "a**/dist",
            "p**/x.ts",
            "**/*.test.ts",
            "*.d.ts",
            "**/generated/**",
            "gen*/**",
            "[gd]ist/**",
            "?ist/*",
            "**/x",
            "node_modules/**",
            "**/node_modules/@prisma/client/**",
            "a/[/]b",
            "**/*.[jt]s",
            "schema.prisma",
            "src/\\*.ts",
            "**/*.map",
            "dist/x/*.js",
            "**/dist",
            "lib/**/x",
            "*/*",
            "**/*/x.json",
            "**/{turbo.json,turbo.jsonc}",
            "**/{dist,lib}/**",
            "{src,dist}/*.{ts,js}",
            "{a**,b}/dist",
            "**/{x,y}",
            "{./dist,lib}/**",
            "{gen",
            "**/{__mocks__,__snapshots__}/*.snap",
            "src/{a,{b,c}}/*",
        ];
        let rels = [
            "dist",
            "src/generated",
            "p0001/dist",
            "packages/p0001/dist",
            "node_modules/@prisma/client",
            "@prisma/client",
            "x",
            "ab",
            "a/dist",
            "gen",
            "generated",
            "src",
            "lib/x",
            "__mocks__",
            "a/__snapshots__",
            "dist/x",
            ".next",
            "build/out.js",
        ];
        let outs = [
            None,
            Some("**"),
            Some("**/*.js"),
            Some("*.d.ts"),
            Some("x/*.snap"),
        ];
        let mut n = 0;
        for p in patterns {
            for r in rels {
                for o in outs {
                    assert_eq!(
                        may_list_output(p, r, o),
                        reference(p, r, o),
                        "pattern {p:?}, rel {r:?}, output pattern {o:?}"
                    );
                    n += 1;
                }
            }
        }
        assert!(n > 3000);
    }

    #[test]
    fn fast_glob_agrees_with_glob() {
        let patterns = [
            "**",
            "*",
            "**/*",
            "dist/**",
            "**/*.js",
            "*.tsbuildinfo",
            "src/generated/**",
            "./dist/**",
            "{dist,lib}/**",
            "**/{a,b}/*.d.ts",
            "a**/b",
            "x/*/y",
            "[ab]*/x",
            "**/x.ts",
            "next-env.d.ts",
            "a/**/b/**/c",
            "*.{js,map}",
            "src/\\*.ts",
            "?",
            "**/__generated__/**",
            "",
            "a/",
            "x/**/",
        ];
        let paths = [
            "",
            "a",
            "b",
            "dist",
            "dist/x.js",
            "dist/a/b.map",
            "src/generated/schema.ts",
            "lib/x.d.ts",
            "q/a/z.d.ts",
            "ab",
            "a/b",
            "x/y/y",
            "x/q/y",
            "a/x",
            "c/x",
            "x.ts",
            "deep/x.ts",
            "next-env.d.ts",
            "a/z/b/q/c",
            "f.js",
            "f.map",
            "src/*.ts",
            "q",
            "w/__generated__/k.ts",
            "x/",
            "x/z/",
        ];
        for pat in patterns {
            let g = crate::glob::FastGlob::new(pat);
            for p in paths {
                assert_eq!(
                    g.matches(p),
                    turborepo_trace_tasks::glob::matches(pat, p),
                    "pattern {pat:?}, path {p:?}"
                );
            }
        }
    }
}
