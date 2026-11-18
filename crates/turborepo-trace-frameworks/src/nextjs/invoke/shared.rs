//! The relation in its shared form: invocations are stored once per source
//! file, and a route refers to sets of files instead of repeating what its
//! layouts and shared modules invoke.
//!
//! ```text
//! files:  [{ file, invokes: [...], strings: [...] }]      one entry per file with sites
//! sets:   [[file index, ...]]                              shared by the routes that reach them
//! routes: { key: { sets: [set index, ...], unknown, open, bound: [route index, ...] } }
//! ```
//!
//! A route's invocations are those of the files of its sets, deduplicated by
//! kind and target ([`SharedRelation::invocations`],
//! [`SharedRelation::expand`]). A route has at most two sets: what its shared
//! segment files reach (layouts, templates, boundaries: the same set
//! for every route below them), and what only its own page or handler file
//! reaches.
//!
//! Closures are not walked per route. The import graph is condensed into
//! strongly connected components once, and each component gets the bit set of
//! the files with sites it reaches, from its successors' sets; a route's set is
//! the union over its segment files.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use turborepo_trace_graph::{GraphView, NodeId};

use super::{
    model::{
        Invocation, InvokeKind, InvokeRelation, InvokeStats, Procedure, Site, Target, UrlString,
    },
    relation::{
        MiddlewareFacts, Reached, RelationOptions, TrpcIndex, follows, outside_input, percentile,
        prepare, rel_path, route_key, unknown_class, unknown_source,
    },
};
use crate::nextjs::{NextApp, routes::RouteKind};

/// A URL-like string that matches handler routes: part of the bound of unknown
/// targets.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StringRoutes {
    /// 1-based line.
    pub line: u32,
    /// The pattern.
    pub pattern: String,
    /// Indexes into [`SharedRelation::route_keys`].
    pub routes: Vec<u32>,
}

/// What one source file invokes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileInvokes {
    /// The file, relative to the graph root.
    pub file: String,
    /// Its invocations, deduplicated by kind and target (`file` is left empty;
    /// `sites` counts the sites in this file).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub invokes: Vec<Invocation>,
    /// Its URL-like strings that match a handler route.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub strings: Vec<StringRoutes>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// What a route refers to.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteSets {
    /// Indexes into [`SharedRelation::sets`].
    pub sets: Vec<u32>,
    /// Some dependency target in the closure is unknown.
    #[serde(default, skip_serializing_if = "is_false")]
    pub unknown: bool,
    /// An unknown target that is not bounded: the route may invoke every
    /// handler.
    #[serde(default, skip_serializing_if = "is_false")]
    pub open: bool,
    /// The bound of the unknown targets: handler routes (indexes into
    /// [`SharedRelation::route_keys`]) that URL-like strings of the closure
    /// match and no site already targets. Empty unless the relation is
    /// bounded and the route has an unknown.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bound: Vec<u32>,
}

/// The relation, shared form.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedRelation {
    /// Format version (2; the expanded form is 1).
    pub version: u32,
    /// The app root, relative to the graph root.
    pub app: String,
    /// Every route key, sorted.
    pub route_keys: Vec<String>,
    /// Files with sites or handler-matching strings, sorted by path.
    pub files: Vec<FileInvokes>,
    /// Sets of file indexes (sorted).
    pub sets: Vec<Vec<u32>>,
    /// Per route.
    pub routes: BTreeMap<String, RouteSets>,
    /// The middleware (it runs before every matched route).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub middleware: Option<RouteSets>,
    /// The app's tRPC procedures.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub procedures: Vec<Procedure>,
    /// Route keys of the tRPC HTTP entries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trpc_handlers: Vec<String>,
    /// Counts.
    pub stats: InvokeStats,
}

/// One target of a route with what the expanded form says about it.
pub struct Entry<'a> {
    /// The first witness, from the file table (`file` empty).
    pub first: &'a Invocation,
    /// The witness file.
    pub file: &'a str,
    /// Sites with this kind and target in the closure.
    pub sites: u32,
    /// Whether every site has the witness's method.
    pub one_method: bool,
    /// Whether the target is wide in every file ([`Invocation::wide`]).
    pub wide: bool,
}

impl SharedRelation {
    /// The files of a route, sorted and without duplicates.
    pub fn route_files(&self, r: &RouteSets) -> Vec<u32> {
        let mut files: Vec<u32> = r
            .sets
            .iter()
            .flat_map(|&s| self.sets[s as usize].iter().copied())
            .collect();
        files.sort_unstable();
        files.dedup();
        files
    }

    /// The targets of a route, deduplicated by kind and target, in the order of
    /// the expanded form (kind, then target). Nothing is copied.
    pub fn entries<'a>(&'a self, r: &RouteSets) -> Vec<Entry<'a>> {
        let mut acc: BTreeMap<(InvokeKind, &'a Target), Entry<'a>> = BTreeMap::new();
        for fi in self.route_files(r) {
            let f = &self.files[fi as usize];
            for inv in &f.invokes {
                let e = acc.entry((inv.kind, &inv.target)).or_insert(Entry {
                    first: inv,
                    file: &f.file,
                    sites: 0,
                    one_method: true,
                    wide: inv.wide,
                });
                // The witness is a site that targets the route by precedence, if any.
                if e.wide && !inv.wide {
                    e.first = inv;
                    e.file = &f.file;
                    e.wide = false;
                }
                e.sites += inv.sites;
                if e.first.method.is_none() || e.first.method != inv.method {
                    e.one_method = false;
                }
            }
        }
        acc.into_values().collect()
    }

    /// For each route of the bound: the first URL-like string of the route's
    /// files that matches it, as `(route index, file, line, pattern)`.
    pub fn bound_witnesses<'a>(&'a self, r: &RouteSets) -> Vec<(u32, &'a str, u32, &'a str)> {
        if r.bound.is_empty() {
            return Vec::new();
        }
        let mut found: BTreeMap<u32, (&'a str, u32, &'a str)> = BTreeMap::new();
        for fi in self.route_files(r) {
            let f = &self.files[fi as usize];
            for s in &f.strings {
                for k in &s.routes {
                    if r.bound.binary_search(k).is_ok() {
                        found.entry(*k).or_insert((&f.file, s.line, &s.pattern));
                    }
                }
            }
            if found.len() == r.bound.len() {
                break;
            }
        }
        found
            .into_iter()
            .map(|(k, (f, l, p))| (k, f, l, p))
            .collect()
    }

    /// The invocations of a route as the expanded form lists them: site targets
    /// deduplicated by kind and target, then the bound.
    pub fn invocations(&self, r: &RouteSets) -> Vec<Invocation> {
        let mut out: Vec<Invocation> = self
            .entries(r)
            .into_iter()
            .map(|e| Invocation {
                kind: e.first.kind,
                target: e.first.target.clone(),
                method: if e.one_method {
                    e.first.method.clone()
                } else {
                    None
                },
                url: e.first.url.clone(),
                file: e.file.to_string(),
                line: e.first.line,
                via: e.first.via.clone(),
                sites: e.sites,
                wide: e.wide,
            })
            .collect();
        for (k, file, line, pattern) in self.bound_witnesses(r) {
            out.push(Invocation {
                kind: InvokeKind::Fetch,
                target: Target::Route(self.route_keys[k as usize].clone()),
                method: None,
                url: Some(pattern.to_string()),
                file: file.to_string(),
                line,
                via: vec!["url-like string".to_string()],
                sites: 0,
                wide: false,
            });
        }
        out
    }

    /// The expanded form: every route with its own list. Large for apps whose
    /// routes share much code (the lists repeat it); for tests, small apps
    /// and consumers of version 1.
    pub fn expand(&self) -> InvokeRelation {
        InvokeRelation {
            version: 1,
            app: self.app.clone(),
            routes: self
                .routes
                .iter()
                .map(|(k, r)| (k.clone(), self.invocations(r)))
                .collect(),
            middleware: self
                .middleware
                .as_ref()
                .map(|r| self.invocations(r))
                .unwrap_or_default(),
            procedures: self.procedures.clone(),
            trpc_handlers: self.trpc_handlers.clone(),
            stats: self.stats.clone(),
        }
    }
}

/// A graph node with sites: its path, its sites and its URL-like strings (by
/// index).
type NodeFiles = (String, Vec<usize>, Vec<usize>);

/// Bit sets stored back to back.
struct Bits {
    words: usize,
    data: Vec<u64>,
}

impl Bits {
    fn new(words: usize) -> Bits {
        Bits {
            words: words.max(1),
            data: Vec::new(),
        }
    }
    fn push(&mut self) -> usize {
        let i = self.data.len() / self.words;
        self.data.resize(self.data.len() + self.words, 0);
        i
    }
    fn get(&self, i: usize) -> &[u64] {
        &self.data[i * self.words..(i + 1) * self.words]
    }
    fn set(&mut self, i: usize, bit: usize) {
        self.data[i * self.words + bit / 64] |= 1 << (bit % 64);
    }
    /// `self[i] |= self[j]` for `j < i`.
    fn or_from(&mut self, i: usize, j: usize) {
        debug_assert!(j < i);
        let (a, b) = self.data.split_at_mut(i * self.words);
        let src = &a[j * self.words..(j + 1) * self.words];
        for (d, s) in b[..self.words].iter_mut().zip(src) {
            *d |= *s;
        }
    }
}

fn or_into(dst: &mut [u64], src: &[u64]) {
    for (d, s) in dst.iter_mut().zip(src) {
        *d |= *s;
    }
}

fn ones(bits: &[u64]) -> impl Iterator<Item = u32> + '_ {
    bits.iter().enumerate().flat_map(|(w, &x)| {
        let mut x = x;
        std::iter::from_fn(move || {
            if x == 0 {
                return None;
            }
            let b = x.trailing_zeros();
            x &= x - 1;
            Some(w as u32 * 64 + b)
        })
    })
}

fn count(bits: &[u64]) -> usize {
    bits.iter().map(|x| x.count_ones() as usize).sum()
}

/// What a set of files adds up to.
#[derive(Clone)]
struct Agg {
    /// Route targets of dependency sites.
    dep: Vec<u64>,
    /// Handler routes matched by URL-like strings.
    strs: Vec<u64>,
    unknown: bool,
    unbounded: bool,
}

impl Agg {
    fn new(rw: usize) -> Agg {
        Agg {
            dep: vec![0; rw],
            strs: vec![0; rw],
            unknown: false,
            unbounded: false,
        }
    }
    fn add(&mut self, a: &Agg) {
        or_into(&mut self.dep, &a.dep);
        or_into(&mut self.strs, &a.strs);
        self.unknown |= a.unknown;
        self.unbounded |= a.unbounded;
    }
    /// Handler routes matched by strings that no dependency site targets.
    fn extra(&self) -> Vec<u64> {
        self.strs
            .iter()
            .zip(&self.dep)
            .map(|(s, d)| s & !d)
            .collect()
    }
}

/// Sets of files, shared by content.
struct Interner {
    sets: Vec<Vec<u32>>,
    aggs: Vec<Agg>,
    ids: HashMap<Vec<u64>, u32>,
    rw: usize,
}

impl Interner {
    fn intern(&mut self, bits: Vec<u64>, file_agg: &[Agg]) -> Option<u32> {
        if bits.iter().all(|x| *x == 0) {
            return None;
        }
        if let Some(&i) = self.ids.get(&bits) {
            return Some(i);
        }
        let list: Vec<u32> = ones(&bits).collect();
        let mut agg = Agg::new(self.rw);
        for &fi in &list {
            agg.add(&file_agg[fi as usize]);
        }
        let i = self.sets.len() as u32;
        self.sets.push(list);
        self.aggs.push(agg);
        self.ids.insert(bits, i);
        Some(i)
    }

    /// The route that refers to `ids`, and what its sets add up to.
    fn route(&self, ids: &[Option<u32>], bound: bool) -> (RouteSets, Agg) {
        let mut agg = Agg::new(self.rw);
        let mut r = RouteSets::default();
        for id in ids.iter().flatten() {
            r.sets.push(*id);
            agg.add(&self.aggs[*id as usize]);
        }
        r.unknown = agg.unknown;
        r.open = agg.unbounded || (agg.unknown && !bound);
        if agg.unknown && bound {
            r.bound = ones(&agg.extra()).collect();
        }
        (r, agg)
    }
}

/// For every node reachable from `seeds` over module edges: the files with
/// sites its closure reaches, as a bit set per strongly connected component.
/// Returns `(component per node, sets)`.
fn reach_sets<G: GraphView>(
    g: &G,
    seeds: &[u32],
    file_of: &[u32],
    files: usize,
) -> (Vec<u32>, Bits) {
    let n = g.node_count() as usize;
    let mut adj: Vec<Option<Vec<u32>>> = vec![None; n];
    let succ = |x: u32| -> Vec<u32> {
        g.out_edges(NodeId(x))
            .filter_map(|e| {
                e.to.filter(|t| {
                    follows(e.kind(), e.flags()) && !g.is_external(*t) && (t.0 as usize) < n
                })
                .map(|t| t.0)
            })
            .collect()
    };
    let mut index = vec![u32::MAX; n];
    let mut low = vec![0u32; n];
    let mut comp = vec![u32::MAX; n];
    let mut on = vec![false; n];
    let mut stack: Vec<u32> = Vec::new();
    let mut next = 0u32;
    let mut bits = Bits::new(files.div_ceil(64));
    // Tarjan, iteratively: a component is complete after every component it
    // reaches.
    let mut frames: Vec<(u32, usize)> = Vec::new();
    for &seed in seeds {
        if (seed as usize) >= n || index[seed as usize] != u32::MAX {
            continue;
        }
        index[seed as usize] = next;
        low[seed as usize] = next;
        next += 1;
        stack.push(seed);
        on[seed as usize] = true;
        adj[seed as usize] = Some(succ(seed));
        frames.push((seed, 0));
        while let Some(&(v, pos)) = frames.last() {
            let vs = v as usize;
            let w = adj[vs].as_ref().and_then(|a| a.get(pos).copied());
            match w {
                Some(w) => {
                    if let Some(top) = frames.last_mut() {
                        top.1 += 1;
                    }
                    let ws = w as usize;
                    if index[ws] == u32::MAX {
                        index[ws] = next;
                        low[ws] = next;
                        next += 1;
                        stack.push(w);
                        on[ws] = true;
                        adj[ws] = Some(succ(w));
                        frames.push((w, 0));
                    } else if on[ws] {
                        low[vs] = low[vs].min(index[ws]);
                    }
                }
                None => {
                    frames.pop();
                    if let Some(&(p, _)) = frames.last() {
                        low[p as usize] = low[p as usize].min(low[vs]);
                    }
                    if low[vs] == index[vs] {
                        let c = bits.push();
                        let mut members = Vec::new();
                        while let Some(w) = stack.pop() {
                            on[w as usize] = false;
                            comp[w as usize] = c as u32;
                            members.push(w);
                            if w == v {
                                break;
                            }
                        }
                        for &m in &members {
                            let f = file_of[m as usize];
                            if f != u32::MAX {
                                bits.set(c, f as usize);
                            }
                            for &s in adj[m as usize].as_deref().unwrap_or(&[]) {
                                let cs = comp[s as usize];
                                if cs != c as u32 && cs != u32::MAX {
                                    bits.or_from(c, cs as usize);
                                }
                            }
                            // The successors are no longer needed.
                            adj[m as usize] = None;
                        }
                    }
                }
            }
        }
    }
    (comp, bits)
}

/// Builds the relation in its shared form: every route's sets of files over its
/// module closure in `g` (runtime import edges from the route's segment files).
pub fn shared_relation<'a, G: GraphView>(
    app: &NextApp,
    g: &G,
    sites: &'a [Site],
    strings: &'a [UrlString],
    trpc: &TrpcIndex,
    opts: &RelationOptions,
) -> SharedRelation {
    let root = &opts.graph_root;
    let timing = turborepo_trace_core::debug::var_os("INVOKE_TIMING").is_some();
    let t0 = std::time::Instant::now();
    let app_rel = rel_path(root, &app.root);
    let in_graph = |f: &str| -> String {
        if app_rel.is_empty() {
            f.to_string()
        } else {
            format!("{app_rel}/{f}")
        }
    };
    // The segment files of every route, and the middleware: where closures start.
    let seg_nodes = |fs: &[String]| -> Vec<u32> {
        fs.iter()
            .filter_map(|f| g.node_by_path(&in_graph(f)).map(|n| n.0))
            .collect()
    };
    let closures: Vec<Vec<String>> = app
        .table
        .routes
        .iter()
        .map(|r| app.table.segment_closure(r).files)
        .collect();
    let mut seeds: Vec<u32> = closures.iter().flat_map(|c| seg_nodes(c)).collect();
    let mw: Vec<u32> = app
        .middleware
        .files
        .iter()
        .filter_map(|f| g.node_by_path(&rel_path(root, f)).map(|n| n.0))
        .collect();
    seeds.extend(mw.iter().copied());
    seeds.sort_unstable();
    seeds.dedup();

    // Reachability first: only the sites and strings some route (or the middleware)
    // reaches are mapped to routes; a repository has many more (other apps,
    // tests, scripts).
    let reached = {
        let mut ids: HashMap<&'a std::path::Path, u32> = HashMap::new();
        let mut file_of: Vec<u32> = vec![u32::MAX; g.node_count() as usize];
        let mut next = 0u32;
        let mut id_of = |p: &'a std::path::Path| -> u32 {
            if let Some(&i) = ids.get(p) {
                return i;
            }
            let i = match g.node_by_path(&rel_path(root, p)) {
                Some(n) if (n.0 as usize) < file_of.len() => {
                    let i = next;
                    next += 1;
                    file_of[n.0 as usize] = i;
                    i
                }
                _ => u32::MAX - 1,
            };
            ids.insert(p, i);
            i
        };
        let site_ids: Vec<u32> = sites.iter().map(|s| id_of(&s.file)).collect();
        let string_ids: Vec<u32> = strings.iter().map(|s| id_of(&s.file)).collect();
        let count = next as usize;
        let (comp, reach) = reach_sets(g, &seeds, &file_of, count);
        let mut all = vec![0u64; count.div_ceil(64).max(1)];
        for &x in &seeds {
            let c = comp[x as usize];
            if c != u32::MAX {
                or_into(&mut all, reach.get(c as usize));
            }
        }
        let on = |i: u32| (i as usize) < count && all[i as usize / 64] & (1 << (i % 64)) != 0;
        // The middleware's own redirects and rewrites: the sites its closure reaches.
        let mut of_mw = vec![0u64; count.div_ceil(64).max(1)];
        for &x in &mw {
            let c = comp[x as usize];
            if c != u32::MAX {
                or_into(&mut of_mw, reach.get(c as usize));
            }
        }
        let in_mw = |i: u32| (i as usize) < count && of_mw[i as usize / 64] & (1 << (i % 64)) != 0;
        let mw_sites: Vec<&Site> = sites
            .iter()
            .zip(&site_ids)
            .filter(|(_, i)| in_mw(**i))
            .map(|(s, _)| s)
            .collect();
        Reached {
            sites: site_ids.iter().map(|&i| on(i)).collect(),
            strings: string_ids.iter().map(|&i| on(i)).collect(),
            middleware: MiddlewareFacts::of(&mw_sites),
        }
    };
    let prep = prepare(app, sites, strings, trpc, opts, &reached);
    let t1 = std::time::Instant::now();
    let mapped = &prep.mapped;

    // Routes by index.
    let mut route_keys: Vec<String> = app.table.routes.iter().map(route_key).collect();
    route_keys.sort();
    route_keys.dedup();
    let key_idx: HashMap<&str, u32> = route_keys
        .iter()
        .enumerate()
        .map(|(i, k)| (k.as_str(), i as u32))
        .collect();
    let rw = route_keys.len().div_ceil(64).max(1);
    let mut handler_bits = vec![0u64; rw];
    for r in &app.table.routes {
        if r.kind == RouteKind::Handler
            && let Some(&i) = key_idx.get(route_key(r).as_str())
        {
            handler_bits[i as usize / 64] |= 1 << (i % 64);
        }
    }

    // Files with sites or handler-matching strings: graph nodes only, sorted by
    // path.
    let mut per_node: HashMap<u32, NodeFiles> = HashMap::new();
    let mut node_of_file: HashMap<&std::path::Path, Option<u32>> = HashMap::new();
    let node = |p: &'_ std::path::Path| -> Option<(u32, String)> {
        let rel = rel_path(root, p);
        g.node_by_path(&rel).map(|n| (n.0, rel))
    };
    let mut site_file: Vec<u32> = vec![u32::MAX; sites.len()];
    for (i, s) in sites.iter().enumerate() {
        if !reached.sites[i] {
            continue;
        }
        let n = match node_of_file.get(s.file.as_path()) {
            Some(n) => *n,
            None => {
                let n = node(&s.file);
                if let Some((id, rel)) = &n {
                    per_node
                        .entry(*id)
                        .or_insert_with(|| (rel.clone(), Vec::new(), Vec::new()));
                }
                let id = n.map(|x| x.0);
                node_of_file.insert(s.file.as_path(), id);
                id
            }
        };
        if let Some(id) = n
            && let Some(e) = per_node.get_mut(&id)
        {
            e.1.push(i);
        }
    }
    for (i, s) in strings.iter().enumerate() {
        if prep.string_routes[i].is_empty() {
            continue;
        }
        let n = match node_of_file.get(s.file.as_path()) {
            Some(n) => *n,
            None => {
                let n = node(&s.file);
                let id = n.as_ref().map(|x| x.0);
                node_of_file.insert(s.file.as_path(), id);
                if let Some((id, rel)) = n {
                    per_node
                        .entry(id)
                        .or_insert_with(|| (rel, Vec::new(), Vec::new()));
                }
                id
            }
        };
        if let Some(id) = n {
            per_node
                .entry(id)
                .or_insert_with(|| (rel_path(root, &s.file), Vec::new(), Vec::new()))
                .2
                .push(i);
        }
    }
    let mut order: Vec<(u32, NodeFiles)> = per_node.into_iter().collect();
    order.sort_by(|a, b| a.1.0.cmp(&b.1.0));
    let n = g.node_count() as usize;
    let mut file_of: Vec<u32> = vec![u32::MAX; n];
    let mut files: Vec<FileInvokes> = Vec::with_capacity(order.len());
    let mut file_agg: Vec<Agg> = Vec::with_capacity(order.len());
    for (fi, (node, (rel, site_idxs, string_idxs))) in order.into_iter().enumerate() {
        if (node as usize) < n {
            file_of[node as usize] = fi as u32;
        }
        let mut agg = Agg::new(rw);
        let mut acc: BTreeMap<(InvokeKind, &Target), Invocation> = BTreeMap::new();
        for &i in &site_idxs {
            site_file[i] = fi as u32;
            let s = &sites[i];
            let fresh = |t: &Target, url: &Option<String>, wide: bool| Invocation {
                kind: s.kind,
                target: t.clone(),
                method: s.method.clone(),
                url: url.clone(),
                file: String::new(),
                line: s.line,
                via: s.via.clone(),
                sites: 0,
                wide,
            };
            // Wide targets: entries of their own, until a site targets the route by
            // precedence.
            for (t, url) in &mapped[i].wide {
                let e = acc
                    .entry((s.kind, t))
                    .or_insert_with(|| fresh(t, url, true));
                e.sites += 1;
                if e.method != s.method {
                    e.method = None;
                }
            }
            for (t, url) in &mapped[i].targets {
                let e = acc
                    .entry((s.kind, t))
                    .or_insert_with(|| fresh(t, url, false));
                if e.wide {
                    // The witness is this site.
                    let sites = e.sites;
                    *e = fresh(t, url, false);
                    e.sites = sites;
                }
                e.sites += 1;
                if e.method != s.method {
                    e.method = None;
                }
                if !s.kind.is_dependency() {
                    continue;
                }
                match t {
                    Target::Route(k) => {
                        if let Some(&ri) = key_idx.get(k.as_str()) {
                            agg.dep[ri as usize / 64] |= 1 << (ri % 64);
                        }
                    }
                    Target::Unknown(why) => {
                        agg.unknown = true;
                        if s.kind != InvokeKind::Redirect && outside_input(why) {
                            agg.unbounded = true;
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut strs = Vec::with_capacity(string_idxs.len());
        for &i in &string_idxs {
            let mut routes: Vec<u32> = prep.string_routes[i]
                .iter()
                .filter_map(|k| key_idx.get(k.as_str()).copied())
                .collect();
            routes.sort_unstable();
            routes.dedup();
            for &ri in &routes {
                agg.strs[ri as usize / 64] |= 1 << (ri % 64);
            }
            strs.push(StringRoutes {
                line: strings[i].line,
                pattern: strings[i].pattern.to_string(),
                routes,
            });
        }
        files.push(FileInvokes {
            file: rel,
            invokes: acc.into_values().collect(),
            strings: strs,
        });
        file_agg.push(agg);
    }
    let fw = files.len().div_ceil(64).max(1);

    // Closures: one pass over the import graph.
    let t2 = std::time::Instant::now();
    let (comp, reach) = reach_sets(g, &seeds, &file_of, files.len());
    let t3 = std::time::Instant::now();
    let reach_of = |nodes: &[u32]| -> Vec<u64> {
        let mut bits = vec![0u64; fw];
        for &x in nodes {
            let c = comp[x as usize];
            if c != u32::MAX {
                or_into(&mut bits, reach.get(c as usize));
            }
        }
        bits
    };

    // Sets, shared by content, with what they add up to.
    let mut interner = Interner {
        sets: Vec::new(),
        aggs: Vec::new(),
        ids: HashMap::new(),
        rw,
    };

    let mut stats = InvokeStats {
        handlers: count(&handler_bits),
        bounded: opts.bound_unknown,
        ..InvokeStats::default()
    };
    let mut reachable = vec![0u64; fw];
    let mut page_sizes: Vec<usize> = Vec::new();
    let mut bound_added: Vec<usize> = Vec::new();
    let mut bound_total: Vec<usize> = Vec::new();
    let mut routes: BTreeMap<String, RouteSets> = BTreeMap::new();
    for (ri, r) in app.table.routes.iter().enumerate() {
        // What the shared segment files reach (the same for every route below them),
        // and what only the route's own file reaches.
        let own: Vec<String> = closures[ri]
            .iter()
            .filter(|f| **f == r.file)
            .cloned()
            .collect();
        let shared: Vec<String> = closures[ri]
            .iter()
            .filter(|f| **f != r.file)
            .cloned()
            .collect();
        let base = reach_of(&seg_nodes(&shared));
        let mut rest = reach_of(&seg_nodes(&own));
        or_into(&mut reachable, &base);
        or_into(&mut reachable, &rest);
        for (x, b) in rest.iter_mut().zip(&base) {
            *x &= !*b;
        }
        let ids = [
            interner.intern(base, &file_agg),
            interner.intern(rest, &file_agg),
        ];
        let (rs, agg) = interner.route(&ids, opts.bound_unknown);
        stats.routes += 1;
        stats.routes_with_unknown += usize::from(agg.unknown);
        if r.kind == RouteKind::Page {
            stats.pages += 1;
            stats.pages_with_unknown += usize::from(agg.unknown);
            stats.pages_unbounded += usize::from(agg.unbounded);
            let extra = agg.extra();
            if agg.unknown {
                let have_handlers: usize = agg
                    .dep
                    .iter()
                    .zip(&handler_bits)
                    .map(|(d, h)| (d & h).count_ones() as usize)
                    .sum();
                bound_added.push(count(&extra));
                bound_total.push(count(&extra) + have_handlers);
            }
            let mut all = agg.dep.clone();
            if agg.unknown && opts.bound_unknown {
                or_into(&mut all, &extra);
            }
            page_sizes.push(count(&all));
        }
        routes.insert(route_key(r), rs);
    }
    let middleware = if mw.is_empty() {
        None
    } else {
        let bits = reach_of(&mw);
        or_into(&mut reachable, &bits);
        let ids = [interner.intern(bits, &file_agg)];
        Some(interner.route(&ids, opts.bound_unknown).0)
    };

    // Counts over the sites some route reaches.
    let handler_keys: std::collections::HashSet<&str> = ones(&handler_bits)
        .map(|i| route_keys[i as usize].as_str())
        .collect();
    let mut wide: Vec<(String, String, usize)> = Vec::new();
    for (i, s) in sites.iter().enumerate() {
        let fi = site_file[i];
        if fi == u32::MAX || reachable[fi as usize / 64] & (1 << (fi % 64)) == 0 {
            continue;
        }
        let ts = &mapped[i].targets;
        if s.kind.is_dependency() {
            let n = ts
                .iter()
                .filter(|(t, _)| matches!(t, Target::Route(_)))
                .count();
            if n >= 8 {
                let mut urls: Vec<&str> = ts
                    .iter()
                    .filter(|(t, _)| matches!(t, Target::Route(_)))
                    .filter_map(|(_, u)| u.as_deref())
                    .collect();
                urls.sort_unstable();
                urls.dedup();
                wide.push((
                    format!("{}:{}", rel_path(root, &s.file), s.line),
                    urls.join(" "),
                    n,
                ));
            }
        }
        *stats.sites.entry(s.kind.as_str().to_string()).or_default() += 1;
        if !s.kind.is_dependency() {
            continue;
        }
        stats.dependency_sites += 1;
        let has = |f: &dyn Fn(&Target) -> bool| ts.iter().any(|(t, _)| f(t));
        stats.to_route += usize::from(has(&|t| matches!(t, Target::Route(_))));
        stats.to_handler += usize::from(has(
            &|t| matches!(t, Target::Route(k) if handler_keys.contains(k.as_str())),
        ));
        stats.to_action += usize::from(has(&|t| matches!(t, Target::Action(_))));
        stats.to_procedure += usize::from(has(&|t| matches!(t, Target::Procedure { .. })));
        stats.external += usize::from(has(&|t| matches!(t, Target::External { .. })));
        let mut origins: Vec<&str> = ts
            .iter()
            .filter_map(|(t, _)| match t {
                Target::External { origin, .. } => Some(origin.as_str()),
                _ => None,
            })
            .collect();
        origins.sort_unstable();
        origins.dedup();
        for o in origins {
            *stats.external_origins.entry(o.to_string()).or_default() += 1;
        }
        stats.unbounded_sites += usize::from(
            s.kind != InvokeKind::Redirect
                && has(&|t| matches!(t, Target::Unknown(r) if outside_input(r))),
        );
        let mut classes: Vec<String> = ts
            .iter()
            .filter_map(|(t, _)| match t {
                Target::Unknown(r) => Some(unknown_class(r)),
                _ => None,
            })
            .collect();
        classes.sort();
        classes.dedup();
        if !classes.is_empty() {
            stats.unknown += 1;
        }
        for c in classes {
            *stats.unknown_classes.entry(c).or_default() += 1;
        }
        let mut sources: Vec<&str> = ts
            .iter()
            .filter_map(|(t, _)| match t {
                Target::Unknown(r) => Some(unknown_source(r)),
                _ => None,
            })
            .collect();
        sources.sort_unstable();
        sources.dedup();
        for c in sources {
            *stats.unknown_sources.entry(c.to_string()).or_default() += 1;
        }
    }
    wide.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
    wide.truncate(25);
    stats.widest_sites = wide;
    let dist = |v: &mut Vec<usize>| {
        v.sort_unstable();
        [
            percentile(v, 0.0),
            percentile(v, 0.25),
            percentile(v, 0.5),
            percentile(v, 0.75),
            percentile(v, 0.9),
            percentile(v, 1.0),
        ]
    };
    if timing {
        eprintln!(
            "invoke timing: relation: map targets {} ms, file table {} ms, closures {} ms, routes \
             and counts {} ms",
            (t1 - t0).as_millis(),
            (t2 - t1).as_millis(),
            (t3 - t2).as_millis(),
            t3.elapsed().as_millis()
        );
    }
    stats.page_invoked_routes = dist(&mut page_sizes);
    stats.bound_added = dist(&mut bound_added);
    stats.bound_total = dist(&mut bound_total);

    SharedRelation {
        version: 2,
        app: app_rel,
        route_keys,
        files,
        sets: interner.sets,
        routes,
        middleware,
        procedures: trpc.procedures.clone(),
        trpc_handlers: trpc.handlers.clone(),
        stats,
    }
}
