//! Cross-file binding of JS / TS values in the derived layer of the graph
//! (`turborepo_trace_lang_js::xfile`,
//! `turborepo_trace_store::Graph::bound_patch`).
//!
//! The binder needs the facts of the files involved and the graph's
//! module-loading edges. [`GraphWorld`] provides both: edges from the composed
//! graph, facts extracted on demand from the working tree (the revision the
//! graph was refreshed from). A file whose contents no longer match the graph's
//! content hash has no facts, which the binder treats as a file that may do
//! anything with what it imports.
//!
//! Facts are cached in `<cache dir>/xfile-facts` by content hash and file
//! name: a query after a small change extracts the changed files only.
//!
//! `TURBO_TRACE_DEBUG_BIND_DEBUG=<part of a path>` prints the facts, the module
//! edges and the bound result of the matching site files to stderr (`summary`:
//! one line per site file).
//!
//! `TURBO_TRACE_DEBUG_BIND=1` binds module loads, `TURBO_TRACE_DEBUG_BIND=all`
//! file reads too, and `TURBO_TRACE_DEBUG_BIND=0` nothing.
//!
//! Binding is **on by default** ([`BIND_BY_DEFAULT`]);
//! `TURBO_TRACE_DEBUG_BIND=0` turns it off.
//!
//! The result is a function of the composed graph (which covers the contents of
//! every file) and of the module-loading edges of the derived layers before it,
//! so it is kept with the graph's other derived layers
//! (`turborepo_trace_store::Graph::cached_patch`, `<cache dir>/
//! derived-bound`): a query on an unchanged graph loads the patch instead of
//! binding again. `TURBO_TRACE_DEBUG_DERIVED_CACHE=0` turns that cache off.
//!
//! After an edit the patch of the graph before it is this graph's when the
//! store kept the composed graph across the edit (nothing composition reads
//! changed) and no file whose contents changed is one the run consulted
//! ([`carries_over`]): the patch is taken as it is, and nothing is scanned,
//! replayed or built. Most edits are of that kind.
//!
//! A changed graph is not bound again either when the binder would get the
//! answers it got last time: the questions of the last run are kept with
//! digests of their answers (`<cache dir>/xfile-trace`,
//! `turborepo_trace_lang_js::xfile::trace`) and asked again of the new graph.
//! An edit that changes no fact the binder looked at (most edits: the
//! facts of the files it consulted, their imports and importers, what a
//! specifier resolves to) reuses the recorded result; any other is bound from
//! nothing, with the whole graph as its input.
//! `TURBO_TRACE_DEBUG_DERIVED_CACHE=0` turns this off too.
//!
//! The string scan ([`crate::names`]) asks for the lookup tables that binding
//! closed ([`closed_spans`]): their strings are not names of the file
//! (`TURBO_TRACE_DEBUG_BIND_NAMES=0` keeps them, for measurements).

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};

use turborepo_trace_graph::{
    Adjacency, Effect, GraphView, NodeId, Origin, Patch, RefFlags, RefKind, UncertaintyKind,
};
use turborepo_trace_lang_js::xfile::{
    FileFacts,
    bind::{self, DepValue, Importer, Select, World},
    codec, file_facts, trace,
};
use turborepo_trace_store::{BoundDep, BoundTarget, ClosedSite, ClosedTable, Graph, Kept};

/// The closed tables of the last [`patch`] of this process, by node: the string
/// values of a table that only leave it through bound keys, into module loads
/// that are closed. The string scan ([`crate::names`]) asks for them: such a
/// string is not a name the file can hand to a read.
static CLOSED_TABLES: Mutex<Vec<ClosedTable>> = Mutex::new(Vec::new());

/// The closed tables of the last [`patch`] as regions whose strings are not
/// names ([`crate::names::ClosedSpan`]): a string that only occurs inside the
/// table's literal can reach nothing but the module loads the table is read
/// for, and those are bound to the files that pass the keys.
pub(crate) fn closed_spans() -> Vec<crate::names::ClosedSpan> {
    // `TURBO_TRACE_DEBUG_BIND_NAMES=0` keeps the names (an A/B switch for
    // measurements).
    if turborepo_trace_core::debug::var_os("BIND_NAMES").is_some_and(|v| v == "0") {
        return Vec::new();
    }
    let Ok(tables) = CLOSED_TABLES.lock() else {
        return Vec::new();
    };
    tables
        .iter()
        .map(|t| crate::names::ClosedSpan {
            node: t.node,
            start: t.at[0],
            end: t.at[1],
        })
        .collect()
}

/// Edges that read their target's contents: they cannot call into it.
fn reads_only(k: RefKind) -> bool {
    matches!(
        k,
        RefKind::FsRead
            | RefKind::FsReadDir
            | RefKind::Asset
            | RefKind::Embed
            | RefKind::Manifest
            | RefKind::NativeAddon
            | RefKind::SharedLib
    )
}

/// Facts by content hash and file name, from earlier runs (`<graph
/// dir>/cache/xfile-facts`). An empty entry is a file without facts.
struct FactsCache {
    path: Option<PathBuf>,
    old: HashMap<u128, Vec<u8>>,
    /// Entries extracted in this run.
    new: Mutex<HashMap<u128, Vec<u8>>>,
    /// Entries of `old` used in this run.
    used: Mutex<HashSet<u128>>,
}

const CACHE_MAGIC: &[u8; 8] = b"DTXFACT\n";

impl FactsCache {
    fn open(dir: Option<&Path>) -> FactsCache {
        let path = dir.map(|d| d.join("xfile-facts"));
        let mut old = HashMap::new();
        let head = Self::header();
        if let Some(buf) = path.as_ref().and_then(|p| std::fs::read(p).ok())
            && buf.starts_with(&head)
        {
            let mut at = head.len();
            while let (Some(k), Some(n)) = (buf.get(at..at + 16), buf.get(at + 16..at + 20)) {
                let key = u128::from_le_bytes(k.try_into().unwrap_or_default());
                let n = u32::from_le_bytes(n.try_into().unwrap_or_default()) as usize;
                let Some(body) = buf.get(at + 20..at + 20 + n) else {
                    break;
                };
                old.insert(key, body.to_vec());
                at += 20 + n;
            }
        }
        FactsCache {
            path,
            old,
            new: Mutex::default(),
            used: Mutex::default(),
        }
    }

    /// Magic, format version and the build identity: facts extracted by another
    /// build are not reused.
    fn header() -> Vec<u8> {
        let mut h = CACHE_MAGIC.to_vec();
        h.push(codec::VERSION);
        let id = turborepo_trace_store::build::build_id().as_bytes();
        h.push(id.len().min(255) as u8);
        h.extend_from_slice(&id[..id.len().min(255)]);
        h
    }

    fn key(hash: u128, path: &str) -> u128 {
        let name = path.rsplit('/').next().unwrap_or(path);
        let mut b = hash.to_le_bytes().to_vec();
        b.extend_from_slice(name.as_bytes());
        xxhash_rust::xxh3::xxh3_128(&b)
    }

    /// `Some(facts)` when the entry exists.
    fn get(&self, key: u128) -> Option<Option<Arc<FileFacts>>> {
        let body = self.old.get(&key)?;
        let facts = if body.is_empty() {
            None
        } else {
            Some(Arc::new(codec::decode(body)?))
        };
        if let Ok(mut u) = self.used.lock() {
            u.insert(key);
        }
        Some(facts)
    }

    fn put(&self, key: u128, facts: Option<&FileFacts>) {
        if let Ok(mut n) = self.new.lock() {
            n.insert(key, facts.map(codec::encode).unwrap_or_default());
        }
    }

    /// The digest of the facts of an entry ([`trace::facts_digest`]), from
    /// their encoded form.
    fn digest(&self, key: u128) -> Option<u64> {
        let of = |body: &Vec<u8>| match body.is_empty() {
            true => 0,
            false => trace::encoded_facts_digest(body),
        };
        if let Ok(n) = self.new.lock()
            && let Some(body) = n.get(&key)
        {
            return Some(of(body));
        }
        self.old.get(&key).map(of)
    }

    /// Writes the entries of this run when something was extracted or an old
    /// entry is no longer used.
    fn save(&self) {
        let (Some(path), Ok(new), Ok(used)) = (&self.path, self.new.lock(), self.used.lock())
        else {
            return;
        };
        if new.is_empty() && used.len() == self.old.len() {
            return;
        }
        let mut entries: Vec<(u128, &[u8])> = self
            .old
            .iter()
            .filter(|(k, _)| used.contains(k))
            .map(|(k, v)| (*k, v.as_slice()))
            .collect();
        entries.extend(new.iter().map(|(k, v)| (*k, v.as_slice())));
        entries.sort_unstable_by_key(|e| e.0);
        let mut buf = Self::header();
        buf.reserve(entries.iter().map(|e| 20 + e.1.len()).sum::<usize>());
        for (k, v) in entries {
            buf.extend_from_slice(&k.to_le_bytes());
            buf.extend_from_slice(&(v.len() as u32).to_le_bytes());
            buf.extend_from_slice(v);
        }
        // Best effort: a cache that cannot be written is rebuilt next time.
        let tmp = path.with_extension(format!("tmp{}", std::process::id()));
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if std::fs::write(&tmp, &buf).is_ok() && std::fs::rename(&tmp, path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

struct GraphWorld<'g> {
    g: &'g Graph,
    root: PathBuf,
    /// The reverse of the graph's adjacency, built when the importers of a file
    /// are first asked for.
    rev: OnceLock<Adjacency>,
    facts: Mutex<HashMap<u32, Option<Arc<FileFacts>>>>,
    parsed: AtomicUsize,
    /// The facts cache, read when the facts of a file are first asked for: a
    /// replayed run ([`trace`]) does not need it.
    cache: OnceLock<FactsCache>,
    /// Files without facts for a reason that is not in the graph: the file
    /// could not be read, or its contents are not the ones the graph was
    /// built from. What their facts are a function of cannot be named
    /// ([`World::facts_source`]).
    transient: Mutex<HashSet<u32>>,
    /// Working directories of the tests.
    cwds: Vec<String>,
    /// Module-loading edges that earlier derived layers add (per-test working
    /// directories, hints), by target: their sources hold the module too.
    derived: HashMap<u32, Vec<u32>>,
    /// Answers given so far ([`World::importers`], [`World::module_at`],
    /// [`World::resolve`]). Each is a function of the graph alone, and the
    /// binder asks the same question many times: for the importers of a
    /// helper once per export, per module it follows and per load it finds,
    /// each time a scan of every edge of every importer.
    memo: Memo,
}

/// The edges of a file that make it an importer ([`GraphWorld::loads_of`]): the
/// target, and the offset of a static declaration.
type Loads = Arc<[(u32, Option<u32>)]>;

/// A list of files.
type Files = Arc<[u32]>;

/// The answers of a [`GraphWorld`] that are kept.
#[derive(Default)]
struct Memo {
    importers: Mutex<HashMap<u32, Arc<[Importer]>>>,
    /// [`GraphWorld::loads_of`] by source.
    loads: Mutex<HashMap<u32, Loads>>,
    module_at: Mutex<HashMap<String, Files>>,
    /// By file and specifier.
    resolve: Mutex<HashMap<u32, HashMap<String, Files>>>,
}

/// `compute()`, once per key of `memo`.
fn memoized<K, Q, V>(
    memo: &Mutex<HashMap<K, Arc<[V]>>>,
    key: &Q,
    compute: impl FnOnce() -> Vec<V>,
) -> Vec<V>
where
    K: std::borrow::Borrow<Q> + std::hash::Hash + Eq,
    Q: std::hash::Hash + Eq + ToOwned<Owned = K> + ?Sized,
    V: Clone,
{
    if let Ok(m) = memo.lock()
        && let Some(v) = m.get(key)
    {
        return v.to_vec();
    }
    let v = compute();
    if let Ok(mut m) = memo.lock() {
        m.insert(key.to_owned(), Arc::from(v.as_slice()));
    }
    v
}

impl GraphWorld<'_> {
    fn cache(&self) -> &FactsCache {
        self.cache
            .get_or_init(|| FactsCache::open(self.g.cache_dir()))
    }

    fn rev(&self) -> &Adjacency {
        self.rev.get_or_init(|| match self.g.reverse_adjacency() {
            Some(r) => r.clone(),
            None => match self.g.forward_adjacency() {
                Some(f) => f.transpose(),
                None => Adjacency::from_view(self.g).transpose(),
            },
        })
    }

    /// The key of the facts of a file in the facts cache: what they are a
    /// function of.
    fn facts_key(&self, n: NodeId) -> Option<u128> {
        self.g
            .file_hash(n)
            .map(|h| FactsCache::key(h, self.g.path(n)))
    }

    /// The facts of a source file from its text in the working tree. `None`:
    /// the file cannot be read, or its contents are not the ones the graph
    /// was built from (the facts must describe those).
    fn extract(&self, n: NodeId) -> Option<Option<FileFacts>> {
        let path = self.g.path(n);
        let src = std::fs::read(self.root.join(path)).ok().filter(|src| {
            self.g
                .file_hash(n)
                .is_none_or(|h| xxhash_rust::xxh3::xxh3_128(src) == h)
        });
        let Some(src) = src else {
            if let Ok(mut t) = self.transient.lock() {
                t.insert(n.0);
            }
            return None;
        };
        self.parsed.fetch_add(1, Ordering::Relaxed);
        Some(file_facts(&src, Path::new(path)))
    }

    /// The edges of `source` that make it an importer of their target, in the
    /// order of the file's edges: the target, and the offset of the
    /// declaration for a static `import` / `export … from` of a source
    /// file. A test file imports several of the helpers the binder asks
    /// about, so its edges are read once, not once per helper.
    fn loads_of(&self, source: u32) -> Loads {
        if let Ok(m) = self.memo.loads.lock()
            && let Some(v) = m.get(&source)
        {
            return v.clone();
        }
        let js = self.is_js(NodeId(source));
        let mut out: Vec<(u32, Option<u32>)> = Vec::new();
        for e in self.g.out_edges(NodeId(source)) {
            let Some(to) = e.to else { continue };
            let (kind, flags) = (e.attr.kind(), e.attr.flags());
            // A source file that reads the module's text cannot call into it. A task or
            // another pseudo-file that takes it as an input may run a copy of it (a
            // compiler's output), unless the edge says it only reads it.
            if flags.intersects(RefFlags::TYPE_ONLY | RefFlags::DATA) || (js && reads_only(kind)) {
                continue;
            }
            let at = match (kind, e.span) {
                (RefKind::Import | RefKind::ReExport, Some(sp)) if js => Some(sp.start),
                _ => None,
            };
            out.push((to.0, at));
        }
        let out: Loads = out.into();
        if let Ok(mut m) = self.memo.loads.lock() {
            m.insert(source, out.clone());
        }
        out
    }

    /// [`World::importers`], computed.
    fn importers_of(&self, file: u32) -> Vec<Importer> {
        let mut sources: Vec<u32> = self.rev().neighbors(NodeId(file)).0.to_vec();
        sources.sort_unstable();
        sources.dedup();
        self.importers_from(file, sources)
    }

    /// The importers of `file` among `sources`: the nodes with an edge to it,
    /// in the order of their ids, each once.
    fn importers_from(&self, file: u32, sources: Vec<u32>) -> Vec<Importer> {
        let mut out: Vec<Importer> = Vec::new();
        for s in sources {
            // The edges of one source are next to each other in `out`: a duplicate can only
            // be among them.
            let first = out.len();
            for &(to, at) in self.loads_of(s).iter() {
                let imp = Importer { file: s, at };
                if to == file && !out[first..].contains(&imp) {
                    out.push(imp);
                }
            }
        }
        for s in self.derived.get(&file).into_iter().flatten() {
            let imp = Importer { file: *s, at: None };
            if !out.contains(&imp) {
                out.push(imp);
            }
        }
        out
    }

    /// [`World::resolve`], computed.
    fn resolve_in(&self, file: u32, spec: &str) -> Vec<u32> {
        // What one of the file's own references with that specifier resolves to (also
        // through a test runner's alias: `<runner>:<source> <spec> -> <request>`).
        let aliased = |label: &str| {
            let (left, _) = label.split_once(" -> ")?;
            let (head, s) = left.rsplit_once(' ')?;
            (head.contains(':') && s == spec).then_some(())
        };
        let mut out: Vec<u32> = Vec::new();
        for e in self.g.out_edges(NodeId(file)) {
            let named = e.specifier == Some(spec)
                || (e.span.is_none() && e.specifier.and_then(aliased).is_some());
            if named
                && !reads_only(e.attr.kind())
                && let Some(t) = e.to
                && self.is_js(t)
                && !out.contains(&t.0)
            {
                out.push(t.0);
            }
        }
        out
    }

    /// [`World::module_at`], computed.
    fn modules_at(&self, path: &str) -> Vec<u32> {
        const EXTS: &[&str] = &[
            "", ".ts", ".tsx", ".js", ".mjs", ".cjs", ".jsx", ".mts", ".cts",
        ];
        // Every candidate that exists: which one a runner takes depends on its
        // extension order. An index file is only a candidate when the path
        // itself is no file.
        let mut out: Vec<u32> = Vec::new();
        let mut candidate = String::with_capacity(path.len() + 12);
        for prefix in ["", "/index"] {
            for ext in EXTS {
                if prefix.is_empty() && ext.is_empty() && !path.contains('.') {
                    continue;
                }
                candidate.clear();
                candidate.push_str(path);
                candidate.push_str(prefix);
                candidate.push_str(ext);
                if let Some(n) = self.g.node_by_path(&candidate)
                    && self.is_js(n)
                    && !out.contains(&n.0)
                {
                    out.push(n.0);
                }
            }
            if !out.is_empty() {
                break;
            }
        }
        out
    }

    fn is_js(&self, n: NodeId) -> bool {
        self.g.lang(n) == Some("js") && !self.g.is_external(n)
    }

    fn load(&self, file: u32) -> Option<Arc<FileFacts>> {
        let n = NodeId(file);
        if !self.is_js(n) {
            return None;
        }
        let key = self.facts_key(n);
        let cache = self.cache();
        if let Some(hit) = key.and_then(|k| cache.get(k)) {
            return hit;
        }
        let facts = self.extract(n)?;
        if let Some(k) = key {
            cache.put(k, facts.as_ref());
        }
        facts.map(Arc::new)
    }
}

impl World for GraphWorld<'_> {
    fn path(&self, file: u32) -> String {
        self.g.path(NodeId(file)).to_owned()
    }

    fn facts(&self, file: u32) -> Option<Arc<FileFacts>> {
        if let Ok(m) = self.facts.lock()
            && let Some(f) = m.get(&file)
        {
            return f.clone();
        }
        let f = self.load(file);
        if let Ok(mut m) = self.facts.lock() {
            m.insert(file, f.clone());
        }
        f
    }

    fn targets(&self, file: u32, at: u32) -> Vec<u32> {
        let mut out: Vec<u32> = Vec::new();
        for e in self.g.out_edges(NodeId(file)) {
            if e.span.is_some_and(|s| s.start == at)
                && matches!(e.attr.kind(), RefKind::Import | RefKind::ReExport)
                && !e.attr.flags().contains(RefFlags::TYPE_ONLY)
                && let Some(t) = e.to
                && !out.contains(&t.0)
            {
                out.push(t.0);
            }
        }
        out
    }

    fn importers(&self, file: u32) -> Vec<Importer> {
        memoized(&self.memo.importers, &file, || self.importers_of(file))
    }

    fn expect_importers(&self, files: &[u32]) {
        // With the reverse adjacency at hand each answer is a lookup. Without it, one
        // pass over the edges finds the sources of all these files, which is
        // much less than reversing every edge (a replayed run asks about a few
        // hundred modules).
        if self.rev.get().is_some() {
            return;
        }
        let Some(fwd) = self.g.forward_adjacency() else {
            return;
        };
        let known = |f: &u32| self.memo.importers.lock().is_ok_and(|m| m.contains_key(f));
        let mut sources: HashMap<u32, Vec<u32>> = files
            .iter()
            .filter(|f| !known(f))
            .map(|f| (*f, Vec::new()))
            .collect();
        if sources.is_empty() {
            return;
        }
        let n = fwd.node_count();
        let mut wanted = turborepo_trace_graph::NodeSet::new(n);
        for f in sources.keys().filter(|f| **f < n) {
            wanted.insert(NodeId(*f));
        }
        for from in 0..n {
            for &to in fwd.neighbors(NodeId(from)).0 {
                if wanted.contains(NodeId(to))
                    && let Some(v) = sources.get_mut(&to)
                    && v.last() != Some(&from)
                {
                    v.push(from);
                }
            }
        }
        // The edges of every source, read once each and on several threads (most of the
        // work of a replayed run: thousands of test files import the helpers asked
        // about).
        let mut todo: Vec<u32> = sources.values().flatten().copied().collect();
        todo.sort_unstable();
        todo.dedup();
        let workers = std::thread::available_parallelism()
            .map_or(4, |n| n.get())
            .min(8);
        if todo.len() >= 64 && workers > 1 {
            std::thread::scope(|s| {
                for part in todo.chunks(todo.len().div_ceil(workers)) {
                    s.spawn(move || {
                        for source in part {
                            self.loads_of(*source);
                        }
                    });
                }
            });
        }
        for (file, sources) in sources {
            let imps = self.importers_from(file, sources);
            if let Ok(mut m) = self.memo.importers.lock() {
                m.insert(file, Arc::from(imps.as_slice()));
            }
        }
    }

    fn is_code(&self, file: u32) -> bool {
        self.is_js(NodeId(file))
    }

    fn resolve(&self, file: u32, spec: &str) -> Vec<u32> {
        if let Ok(m) = self.memo.resolve.lock()
            && let Some(v) = m.get(&file).and_then(|by| by.get(spec))
        {
            return v.to_vec();
        }
        let v = self.resolve_in(file, spec);
        if let Ok(mut m) = self.memo.resolve.lock() {
            m.entry(file)
                .or_default()
                .insert(spec.to_owned(), Arc::from(v.as_slice()));
        }
        v
    }

    fn module_at(&self, path: &str) -> Vec<u32> {
        memoized(&self.memo.module_at, path, || self.modules_at(path))
    }

    fn cwds(&self) -> Vec<String> {
        self.cwds.clone()
    }

    fn file(&self, path: &str) -> Option<u32> {
        self.g.node_by_path(path).map(|n| n.0)
    }

    fn facts_source(&self, file: u32) -> Option<u128> {
        let n = NodeId(file);
        // A file that is no source code has no facts whatever it contains; that is
        // cheap to say again ([`World::facts_digest`]).
        if !self.is_js(n) || self.transient.lock().is_ok_and(|t| t.contains(&file)) {
            return None;
        }
        self.facts_key(n)
    }

    fn facts_digest(&self, file: u32) -> u64 {
        let n = NodeId(file);
        if !self.is_js(n) {
            return 0;
        }
        // Facts this run has loaded: the digest of their cache entry.
        let loaded = self.facts.lock().ok().and_then(|m| m.get(&file).cloned());
        if let Some(facts) = loaded {
            return match (&facts, self.cache.get(), self.facts_key(n)) {
                (None, ..) => 0,
                (Some(f), cache, key) => key
                    .and_then(|k| cache?.digest(k))
                    .filter(|d| *d != 0)
                    .unwrap_or_else(|| trace::facts_digest(Some(f))),
            };
        }
        // Not loaded: a recorded run is being replayed, and this file is not the one it
        // saw. Its facts come from its text; the facts cache is not read for it.
        trace::facts_digest(self.extract(n).flatten().as_ref())
    }

    fn prefetch(&self, files: &[u32]) {
        let todo: Vec<u32> = match self.facts.lock() {
            Ok(m) => files
                .iter()
                .copied()
                .filter(|f| !m.contains_key(f))
                .collect(),
            Err(_) => return,
        };
        if todo.len() < 8 {
            return;
        }
        let next = AtomicUsize::new(0);
        let workers = std::thread::available_parallelism()
            .map_or(4, |n| n.get())
            .min(8);
        std::thread::scope(|s| {
            for _ in 0..workers {
                s.spawn(|| {
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(&f) = todo.get(i) else { break };
                        self.facts(f);
                    }
                });
            }
        });
    }
}

/// What binding computes for a graph: the patch, the closed tables (for
/// [`closed_spans`]) and the numbers of its `--stats` line ([`STATS`] of them).
struct Bound {
    patch: Patch,
    tables: Vec<ClosedTable>,
    stats: [u64; STATS],
    /// The files whose contents the result is a function of, when the run can
    /// name them all (sorted node ids): every file of a question, of an
    /// answer or of the result ([`trace::Trace::paths`]). Kept with the
    /// patch for [`carries_over`].
    consulted: Option<Vec<u32>>,
}

/// Number of statistics of a binding run: site files, files with facts, files
/// extracted, dependencies on suppliers, sites closed, records dropped, edges
/// cut, records opened on suppliers, whether the budget ran out, and whether
/// the result is a recorded run's that was replayed ([`trace`]).
const STATS: usize = 10;

/// The recorded run of the last binding in a graph directory (`<graph
/// dir>/cache/xfile-trace`, [`trace::Trace`]), behind a header that names what
/// else the run was a function of.
struct TraceFile {
    path: PathBuf,
    head: Vec<u8>,
}

impl TraceFile {
    /// `None`: the graph has no directory, or the derived caches are switched
    /// off (`TURBO_TRACE_DEBUG_DERIVED_CACHE=0`), or the binder is asked
    /// for debug output, which a replayed run cannot give.
    fn of(g: &Graph, mode: &str) -> Option<TraceFile> {
        let set =
            |name: &str| turborepo_trace_core::debug::var_os(name).is_some_and(|v| !v.is_empty());
        if turborepo_trace_core::debug::var_os("DERIVED_CACHE").is_some_and(|v| v == "0")
            || set("BIND_DEBUG")
            || set("BIND_ROUNDS")
            || set("BIND_TRACE")
        {
            return None;
        }
        // The facts format, the binary, what is bound and the step budget.
        let mut head = b"DTXTRACE".to_vec();
        head.push(codec::VERSION);
        let id = turborepo_trace_store::build::build_id().as_bytes();
        head.push(id.len().min(255) as u8);
        head.extend_from_slice(&id[..id.len().min(255)]);
        head.push(mode.len() as u8);
        head.extend_from_slice(mode.as_bytes());
        head.extend_from_slice(&bind::file_steps().to_le_bytes());
        Some(TraceFile {
            path: g.cache_dir()?.join("xfile-trace"),
            head,
        })
    }

    fn read(&self) -> Option<trace::Trace> {
        let bytes = std::fs::read(&self.path).ok()?;
        trace::Trace::from_bytes(bytes.strip_prefix(self.head.as_slice())?)
    }

    /// Best effort: without the file the next changed graph is bound again.
    fn write(&self, t: &trace::Trace) {
        let mut buf = self.head.clone();
        buf.extend_from_slice(&t.to_bytes());
        let tmp = self
            .path
            .with_extension(format!("tmp{}", std::process::id()));
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if std::fs::write(&tmp, &buf).is_err() || std::fs::rename(&tmp, &self.path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

/// The files of `g` with a site worth binding: a parameter-origin record of a
/// selected effect, or a dynamic module load with several stored targets (a
/// table read by key). In the order of their ids. Every record and edge of the
/// graph is looked at, on several threads.
fn site_files(g: &Graph, select: Select) -> Vec<u32> {
    let is_site = |i: u32| -> bool {
        let n = NodeId(i);
        if g.lang(n) != Some("js") || g.is_external(n) {
            return false;
        }
        let record = g.uncertainties(n).any(|u| {
            u.kind == UncertaintyKind::ComputedSpecifier
                && matches!(u.origin, Origin::Param | Origin::Call | Origin::Imported)
                && match u.effect {
                    Effect::Load => select.loads,
                    Effect::Read => select.reads,
                    Effect::None => false,
                }
        });
        record
            || (select.loads
                && g.out_edges(n).any(|e| {
                    e.attr.flags().contains(RefFlags::CONDITIONAL)
                        && matches!(e.attr.kind(), RefKind::Require | RefKind::DynamicImport)
                }))
    };
    let n = g.node_count();
    let workers = std::thread::available_parallelism()
        .map_or(4, |n| n.get() as u32)
        .min(8);
    if n < 4096 || workers < 2 {
        return (0..n).filter(|i| is_site(*i)).collect();
    }
    let chunk = n.div_ceil(workers);
    let is_site = &is_site;
    std::thread::scope(|s| {
        let parts: Vec<_> = (0..workers)
            .map(|w| {
                let (lo, hi) = (w * chunk, ((w + 1) * chunk).min(n));
                s.spawn(move || (lo..hi).filter(|i| is_site(*i)).collect::<Vec<u32>>())
            })
            .collect();
        let mut out = Vec::new();
        for p in parts {
            match p.join() {
                Ok(part) => out.extend(part),
                Err(e) => std::panic::resume_unwind(e),
            }
        }
        out
    })
}

/// Whether cross-file binding runs when `TURBO_TRACE_DEBUG_BIND` is not set.
/// **On**: it was measured to cost at most 1.1 times the time without it on
/// large repositories. `TURBO_TRACE_DEBUG_BIND=0` turns it off,
/// `TURBO_TRACE_DEBUG_BIND=1` on (`all`: file reads too), whatever the default.
pub(crate) const BIND_BY_DEFAULT: bool = true;

/// What is bound, from `TURBO_TRACE_DEBUG_BIND`: `None` when binding is off,
/// else `loads` (module loads) or `all` (module loads and file reads).
fn mode() -> Option<&'static str> {
    match turborepo_trace_core::debug::var("BIND").ok().as_deref() {
        None | Some("") => BIND_BY_DEFAULT.then_some("loads"),
        Some("0") => None,
        Some("all") => Some("all"),
        Some(_) => Some("loads"),
    }
}

/// The inputs of the bound patch besides the composed graph (which
/// [`Graph::cached_patch`] adds): what is bound, the facts format, and the
/// module-loading edges of the derived layers before it (per-test working
/// directories, hints).
///
/// Every query computes this key, so it must cost next to nothing: the earlier
/// layers' edges enter as a digest of the edges as they are ([`derived_loads`]
/// reads them: which file holds which module follows from an edge's ends, kind
/// and label), not as the holders derived from them, which takes a lookup of
/// every site path. The digest is a sum over the edges, so the order a layer
/// adds them in does not matter.
fn bound_key(mode: &str, earlier: &Patch) -> u128 {
    use xxhash_rust::xxh3::{xxh3_64, xxh3_128};
    let mut labels: Vec<Option<u64>> = vec![None; earlier.string_count()];
    let mut edges = 0u128;
    for (from, to, attr, label) in earlier.edge_ids() {
        if !loads_a_module(attr) {
            continue;
        }
        let l = match labels.get_mut(label as usize) {
            Some(slot) => *slot.get_or_insert_with(|| xxh3_64(earlier.string(label).as_bytes())),
            None => 0,
        };
        let mut b = [0u8; 18];
        b[..4].copy_from_slice(&from.0.to_le_bytes());
        b[4..8].copy_from_slice(&to.0.to_le_bytes());
        b[8..10].copy_from_slice(&attr.0.to_le_bytes());
        b[10..].copy_from_slice(&l.to_le_bytes());
        edges = edges.wrapping_add(xxh3_128(&b));
    }
    let mut h = xxhash_rust::xxh3::Xxh3::new();
    // The layout of what is kept with the patch (the statistics, the closed tables,
    // the consulted files).
    h.update(&[STATS as u8, 2]);
    h.update(&[codec::VERSION]);
    h.update(mode.as_bytes());
    h.update(&[0]);
    h.update(&bind::file_steps().to_le_bytes());
    h.update(&edges.to_le_bytes());
    h.digest128()
}

/// Whether an edge of an earlier derived layer loads its target as a module
/// (its source, or the site its label names, then holds that module): not an
/// edge that only reads the target's contents or its types.
fn loads_a_module(attr: turborepo_trace_graph::EdgeAttr) -> bool {
    !reads_only(attr.kind())
        && !attr
            .flags()
            .intersects(RefFlags::TYPE_ONLY | RefFlags::DATA)
}

/// The closed tables as numbers, for the cache of the derived layer (which
/// keeps a list of numbers next to the patch): per table its node, span and
/// strings, a string as its length and its bytes in words of eight.
fn pack_tables(tables: &[ClosedTable], out: &mut Vec<u64>) {
    out.push(tables.len() as u64);
    for t in tables {
        out.extend([
            u64::from(t.node.0),
            u64::from(t.at[0]),
            u64::from(t.at[1]),
            t.specs.len() as u64,
        ]);
        for s in &t.specs {
            out.push(s.len() as u64);
            for c in s.as_bytes().chunks(8) {
                let mut w = [0u8; 8];
                w[..c.len()].copy_from_slice(c);
                out.push(u64::from_le_bytes(w));
            }
        }
    }
}

/// The consulted files of a run after its tables ([`Bound::consulted`]): a
/// flag, then the number of files and their node ids.
fn pack_consulted(consulted: Option<&[u32]>, out: &mut Vec<u64>) {
    match consulted {
        None => out.push(0),
        Some(files) => {
            out.extend([1, files.len() as u64]);
            out.extend(files.iter().map(|f| u64::from(*f)));
        }
    }
}

/// The tables and the consulted files that [`pack_tables`] and
/// [`pack_consulted`] wrote; `None` for anything else.
fn unpack_kept(m: &[u64]) -> Option<(Vec<ClosedTable>, Option<Vec<u32>>)> {
    let (tables, rest) = unpack_tables(m)?;
    let consulted = match rest.split_first()? {
        (0, []) => None,
        (1, rest) => {
            let (n, ids) = rest.split_first()?;
            if usize::try_from(*n).ok()? != ids.len() {
                return None;
            }
            let ids: Vec<u32> = ids
                .iter()
                .map(|v| u32::try_from(*v).ok())
                .collect::<Option<_>>()?;
            if !ids.is_sorted() {
                return None;
            }
            Some(ids)
        }
        _ => return None,
    };
    Some((tables, consulted))
}

/// Whether the bound layer kept for the snapshot before an edit is this graph's
/// too ([`Graph::carried_patch`]): `meta` are the numbers kept with it,
/// `changed` the files whose contents changed since.
///
/// The layer is a function of what composition reads (the site files, the
/// tests' working directories, every edge and record without its position: the
/// same in both graphs), of the earlier layers' module-loading edges and the
/// switches (in the key, which is the same), and of the contents of the files
/// the run consulted: their facts, and the lines and spans of their references
/// and records, which the answers of the graph and the patch are made of. A run
/// that was recorded names all of them, the files of its result included. When
/// none of them changed, binding this graph would ask the same questions,
/// get the same answers and build the same patch.
fn carries_over(meta: &[u64], changed: &[NodeId]) -> bool {
    let Some((_, rest)) = meta.split_at_checked(STATS) else {
        return false;
    };
    let Some((_, Some(consulted))) = unpack_kept(rest) else {
        return false;
    };
    changed
        .iter()
        .all(|n| consulted.binary_search(&n.0).is_err())
}

/// The tables [`pack_tables`] wrote and what follows them; `None` for anything
/// else.
fn unpack_tables(mut m: &[u64]) -> Option<(Vec<ClosedTable>, &[u64])> {
    let mut next = || -> Option<u64> {
        let (v, rest) = m.split_first()?;
        m = rest;
        Some(*v)
    };
    let small = |v: u64| u32::try_from(v).ok();
    let mut tables = Vec::new();
    for _ in 0..next()? {
        let node = NodeId(small(next()?)?);
        let at = [small(next()?)?, small(next()?)?];
        let mut specs = Vec::new();
        for _ in 0..next()? {
            let len = usize::try_from(next()?).ok()?;
            let mut bytes = Vec::with_capacity(len.min(4096));
            for _ in 0..len.div_ceil(8) {
                bytes.extend_from_slice(&next()?.to_le_bytes());
            }
            bytes.truncate(len);
            specs.push(String::from_utf8(bytes).ok()?);
        }
        tables.push(ClosedTable { node, at, specs });
    }
    Some((tables, m))
}

/// The module-loading edges of `earlier` (the derived layers before binding),
/// by target: the files that hold each module without a stored edge.
///
/// An edge of the per-test working-directory layer runs from a test to what a
/// site loads in that test's working directory. The code that holds the loaded
/// module is the site's (the label names it), not the test's: the test is not a
/// second holder of exports that the site's own facts describe. Any other edge
/// (a hint) is held by its source.
fn derived_loads(g: &Graph, earlier: &Patch) -> HashMap<u32, Vec<u32>> {
    let mut m: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut by_path: Option<HashMap<&str, u32>> = None;
    for (from, to, attr, label) in earlier.labeled_edges() {
        if !loads_a_module(attr) {
            continue;
        }
        let holder = turborepo_trace_store::cwd_edge_site(label)
            .and_then(|site| {
                by_path
                    .get_or_insert_with(|| {
                        (0..g.node_count())
                            .map(|i| (g.path(NodeId(i)), i))
                            .collect()
                    })
                    .get(site)
                    .copied()
            })
            .unwrap_or(from.0);
        let v = m.entry(to.0).or_default();
        if !v.contains(&holder) {
            v.push(holder);
        }
    }
    m
}

/// The patch that binds parameter-dependent module loads (and, with
/// `TURBO_TRACE_DEBUG_BIND=all`, file reads) of `g` to the files that supply
/// their values. `None` when binding is off ([`BIND_BY_DEFAULT`],
/// `TURBO_TRACE_DEBUG_BIND`) or there is nothing to bind. `earlier` is the
/// patch of the derived layers before it: a file that loads a module through
/// one of its edges holds that module too.
///
/// The result is cached with the graph's other derived layers
/// ([`Graph::cached_patch`], `cache/derived-bound`;
/// `TURBO_TRACE_DEBUG_DERIVED_CACHE=0` always computes), except when the debug
/// output is asked for, which needs the facts.
pub(crate) fn patch(g: &Graph, earlier: &Patch, stats: bool) -> Option<Patch> {
    let mode = mode()?;
    let t = Instant::now();
    let key = bound_key(mode, earlier);
    turborepo_trace_core::timing::mark("bound: key");
    let run = || {
        let b = compute(g, derived_loads(g, earlier), mode);
        let mut meta = b.stats.to_vec();
        pack_tables(&b.tables, &mut meta);
        pack_consulted(b.consulted.as_deref(), &mut meta);
        (b.patch, meta)
    };
    let debug = turborepo_trace_core::debug::var_os("BIND_DEBUG").is_some_and(|v| !v.is_empty());
    let (patch, meta, kept) = if debug {
        let (patch, meta) = run();
        (patch, meta, Kept::Computed)
    } else {
        // Stored for this graph; or stored for the graph before an edit that changed no
        // file the run consulted, which makes it this graph's; or bound (replayed) now.
        g.carried_patch("bound", key, carries_over, run)
    };
    if kept == Kept::Carried {
        turborepo_trace_core::timing::mark("bound: kept across the edit");
    }
    // A cache entry that does not hold the tables keeps their strings as names:
    // more is selected, never less.
    let (s, tables) = match meta.split_at_checked(STATS) {
        Some((s, rest)) => (
            s.to_vec(),
            unpack_kept(rest).map(|k| k.0).unwrap_or_default(),
        ),
        None => (vec![0; STATS], Vec::new()),
    };
    if stats && s[0] > 0 {
        eprintln!(
            "bound: {} site file(s), facts of {} file(s) ({} extracted), {} dependenc(ies) on \
             suppliers, {} site(s) closed ({} record(s) dropped, {} edge(s) cut), {} open \
             record(s) on suppliers{}, {:.1} ms{}",
            s[0],
            s[1],
            s[2],
            s[3],
            s[4],
            s[5],
            s[6],
            s[7],
            if s[8] != 0 {
                ", budget exhausted (the sites of the files it ran out on stay open)"
            } else {
                ""
            },
            t.elapsed().as_secs_f64() * 1e3,
            match (kept, s[9] != 0) {
                (Kept::Cached, _) => " (cached)",
                (Kept::Carried, _) => " (kept across the edit)",
                (Kept::Computed, true) => " (a recorded run replayed)",
                (Kept::Computed, false) => "",
            }
        );
    }
    if let Ok(mut t) = CLOSED_TABLES.lock() {
        *t = tables;
    }
    (!patch.is_empty()).then_some(patch)
}

/// Binds the sites of `g` (see [`patch`]).
fn compute(g: &Graph, derived: HashMap<u32, Vec<u32>>, mode: &str) -> Bound {
    let select = Select {
        loads: true,
        reads: mode == "all",
    };
    // (Whether there is a site at all follows from what composition reads: no
    // file's contents were consulted.)
    let nothing = || Bound {
        patch: Patch::new(),
        tables: Vec::new(),
        stats: [0; STATS],
        consulted: Some(Vec::new()),
    };
    // The site files and the tests' working directories each take a pass over the
    // graph: both at the same time.
    let (roots, cwds) = std::thread::scope(|s| {
        let cwds = s.spawn(|| g.test_cwds());
        (site_files(g, select), cwds.join())
    });
    let cwds = match cwds {
        Ok(cwds) => cwds,
        Err(e) => std::panic::resume_unwind(e),
    };
    if roots.is_empty() {
        return nothing();
    }
    turborepo_trace_core::timing::mark("bound: site files, working directories of the tests");
    let world = GraphWorld {
        g,
        root: PathBuf::from(g.root()),
        rev: OnceLock::new(),
        facts: Mutex::default(),
        parsed: AtomicUsize::new(0),
        cache: OnceLock::new(),
        transient: Mutex::default(),
        cwds,
        derived,
        memo: Memo::default(),
    };
    // The run recorded when a graph in this directory was last bound: when it would
    // ask the same questions of this graph and get the same answers, its result
    // is this graph's ([`trace`]), and nothing is bound.
    let kept = TraceFile::of(g, mode);
    let mut replayed = None;
    // The files the run saw, when it was recorded.
    let mut consulted: Option<Vec<u32>> = None;
    if let Some(file) = &kept
        && let Some(mut t) = file.read()
    {
        if let Some(r) = trace::replay(&mut t, &world, &roots) {
            if r.updated {
                file.write(&t);
            }
            consulted = Some(r.files);
            replayed = Some(r.bound);
        }
        turborepo_trace_core::timing::mark(if replayed.is_some() {
            "bound: recorded run replayed"
        } else {
            "bound: recorded run does not apply"
        });
    }
    let reused = replayed.is_some();
    let bound = match replayed {
        Some(bound) => bound,
        None => {
            world.prefetch(&roots);
            turborepo_trace_core::timing::mark("bound: facts of the site files");
            let bound = match &kept {
                Some(file) => {
                    let rec = trace::Recorder::new(&world);
                    let bound = bind::bind(&rec, &roots, select);
                    turborepo_trace_core::timing::mark("bound: bind");
                    if let Some(t) = rec.finish(&roots, &bound) {
                        file.write(&t);
                        consulted = t
                            .paths()
                            .iter()
                            .map(|p| g.node_by_path(p).map(|n| n.0))
                            .collect();
                    }
                    bound
                }
                None => {
                    let bound = bind::bind(&world, &roots, select);
                    turborepo_trace_core::timing::mark("bound: bind");
                    bound
                }
            };
            world.cache().save();
            turborepo_trace_core::timing::mark("bound: facts cache and recorded run written");
            bound
        }
    };
    let deps: Vec<BoundDep> = bound
        .deps
        .iter()
        .map(|d| BoundDep {
            node: NodeId(d.file),
            line: d.line,
            kind: d.kind,
            sink: NodeId(d.sink_file),
            sink_line: d.sink_line,
            target: match &d.value {
                DepValue::Path(p) => BoundTarget::Path(p.clone()),
                DepValue::Below(p) => BoundTarget::Below(p.clone()),
                DepValue::Cwd(p) => BoundTarget::Cwd(p.clone()),
                DepValue::Spec(s) => BoundTarget::Spec(s.clone()),
                DepValue::PerTestCwd(s) => BoundTarget::PerTestCwd(s.clone()),
                DepValue::Open { origin, detail } => BoundTarget::Open {
                    origin: *origin,
                    detail: detail.clone(),
                },
            },
        })
        .collect();
    let closed: Vec<ClosedSite> = bound
        .closed
        .iter()
        .map(|c| ClosedSite {
            node: NodeId(c.file),
            at: c.at,
            site: c.site,
            open: c.open,
            specs: c.specs.clone(),
        })
        .collect();
    let tables: Vec<ClosedTable> = bound
        .tables
        .iter()
        .map(|t| ClosedTable {
            node: NodeId(t.file),
            at: t.at,
            specs: t.specs.clone(),
        })
        .collect();
    // Debug aid: `TURBO_TRACE_DEBUG_BIND_DEBUG=<part of a path>` prints the facts
    // of the matching site files and what was bound for them.
    if let Ok(part) = turborepo_trace_core::debug::var("BIND_DEBUG")
        && !part.is_empty()
    {
        use turborepo_trace_lang_js::xfile::show;
        // `sinks`: every bound site with what was bound for it, grouped by supplier.
        if part == "sinks" {
            for &r in &roots {
                let Some(f) = world.facts(r) else { continue };
                for sink in f.sinks.iter().filter(|x| {
                    select.loads
                        && matches!(
                            x.kind,
                            turborepo_trace_core::RefKind::Require
                                | turborepo_trace_core::RefKind::DynamicImport
                                | turborepo_trace_core::RefKind::Worker
                                | turborepo_trace_core::RefKind::ChildProcess
                        )
                        || select.reads
                }) {
                    let closed = bound.closed.iter().any(|c| c.file == r && c.at == sink.at);
                    let deps: Vec<_> = bound
                        .deps
                        .iter()
                        .filter(|d| d.sink_file == r && d.sink_line == sink.line)
                        .collect();
                    let known = deps
                        .iter()
                        .filter(|d| !matches!(d.value, DepValue::Open { .. }))
                        .count();
                    eprintln!(
                        "sink {}:{} {} {} {}: {} known, {} open; value {}",
                        g.path(NodeId(r)),
                        sink.line,
                        sink.kind.as_str(),
                        if sink.open { "record" } else { "resolved" },
                        if closed { "CLOSED" } else { "stays" },
                        known,
                        deps.len() - known,
                        show::x(&sink.value, &f)
                            .chars()
                            .take(
                                if turborepo_trace_core::debug::var_os("BIND_WIDE").is_some() {
                                    4000
                                } else {
                                    160
                                }
                            )
                            .collect::<String>()
                    );
                    let mut groups: std::collections::BTreeMap<(String, String), usize> =
                        Default::default();
                    for d in &deps {
                        if let DepValue::Open { origin, detail } = &d.value {
                            *groups
                                .entry((
                                    g.path(NodeId(d.file)).to_owned(),
                                    format!("{} {}", origin.as_str(), detail),
                                ))
                                .or_default() += 1;
                        }
                    }
                    // Membership hubs (tasks that take the file as an input) are listed
                    // as one line.
                    let hubs = groups
                        .keys()
                        .filter(|k| k.0.starts_with("members:"))
                        .count();
                    if hubs > 0 {
                        eprintln!("    open at {hubs} membership hub record(s)");
                    }
                    let rest: Vec<_> = groups
                        .iter()
                        .filter(|(k, _)| !k.0.starts_with("members:"))
                        .collect();
                    for ((file, what), n) in rest.iter().take(14) {
                        eprintln!(
                            "    open at {file} x{n}: {}",
                            what.chars().take(140).collect::<String>()
                        );
                    }
                    if rest.len() > 14 {
                        eprintln!("    … and {} more", rest.len() - 14);
                    }
                }
            }
        }
        // `summary`: one line per site file.
        if part == "summary" {
            let mut by: std::collections::BTreeMap<&str, [usize; 4]> = Default::default();
            for d in &bound.deps {
                let e = by.entry(g.path(NodeId(d.sink_file))).or_default();
                match &d.value {
                    DepValue::Open { .. } => e[1] += 1,
                    _ => e[0] += 1,
                }
            }
            for c in &bound.closed {
                by.entry(g.path(NodeId(c.file))).or_default()[2] += 1;
            }
            for &r in &roots {
                by.entry(g.path(NodeId(r))).or_default()[3] += 1;
            }
            for (path, [known, open, closed, _]) in by {
                eprintln!("site {path}: {known} known, {open} open, {closed} closed");
            }
        }
        for &r in roots.iter().filter(|r| g.path(NodeId(**r)).contains(&part)) {
            eprintln!("== facts of {}", g.path(NodeId(r)));
            match world.facts(r) {
                Some(f) => eprintln!("{}", show::facts(&f)),
                None => eprintln!("none"),
            }
            for e in g.out_edges(NodeId(r)) {
                if e.attr.kind() != RefKind::Import {
                    eprintln!(
                        "edge {} {:?} {:?} {:?} -> {}",
                        e.attr.kind().as_str(),
                        e.attr.flags(),
                        e.span.map(|s| (s.start, s.end)),
                        e.specifier,
                        e.to.map_or("-", |t| g.path(t))
                    );
                }
            }
            for c in bound.closed.iter().filter(|c| c.file == r) {
                eprintln!("closed site {:?} at {:?} open {}", c.site, c.at, c.open);
            }
            for t in bound.tables.iter().filter(|t| t.file == r) {
                eprintln!("closed table {:?}", t.at);
            }
        }
        // `TURBO_TRACE_DEBUG_BIND_FACTS=<part of a path>`: the facts of every file the
        // binder consulted whose path matches (suppliers, modules that were
        // followed).
        if let Ok(which) = turborepo_trace_core::debug::var("BIND_FACTS")
            && !which.is_empty()
        {
            let seen: Vec<u32> = world
                .facts
                .lock()
                .map(|m| m.keys().copied().collect())
                .unwrap_or_default();
            let mut seen: Vec<u32> = seen
                .into_iter()
                .filter(|f| g.path(NodeId(*f)).contains(&which))
                .collect();
            seen.sort_unstable();
            for f in seen {
                eprintln!("== facts of {} (consulted)", g.path(NodeId(f)));
                match world.facts(f) {
                    Some(facts) => eprintln!("{}", show::facts(&facts)),
                    None => eprintln!("none"),
                }
            }
        }
        let mut b = bound.clone();
        b.deps
            .retain(|d| g.path(NodeId(d.sink_file)).contains(&part));
        b.closed.retain(|c| g.path(NodeId(c.file)).contains(&part));
        b.tables.retain(|t| g.path(NodeId(t.file)).contains(&part));
        eprintln!(
            "== bound\n{}",
            show::bound(&b, &|f| g.path(NodeId(f)).to_owned())
        );
    }
    let (patch, st) = g.bound_patch(&deps, &closed, &tables);
    turborepo_trace_core::timing::mark("bound: patch");
    let stats = [
        roots.len() as u64,
        bound.files as u64,
        world.parsed.load(Ordering::Relaxed) as u64,
        st.deps as u64,
        closed.len() as u64,
        st.dropped as u64,
        st.cuts as u64,
        st.opened as u64,
        u64::from(bound.exhausted),
        u64::from(reused),
    ];
    Bound {
        patch,
        // Only tables whose literal references the patch replaces: a closed site of the
        // same file.
        tables: tables
            .iter()
            .filter(|t| closed.iter().any(|c| c.node == t.node && !c.open))
            .cloned()
            .collect(),
        stats,
        // A file that had no facts for a reason outside the graph (it could not be read,
        // or its contents were not the graph's) may have them next time: the result is
        // then not a function of contents the graph knows.
        consulted: consulted.filter(|_| world.transient.lock().is_ok_and(|t| t.is_empty())),
    }
}
