//! The uncertainty floor: the entries that every
//! change selects because they reach an open uncertainty, and the few
//! sites that put them there.
//!
//! [`Engine::affected`] gives each entry that reaches a first-party open
//! uncertainty the `--on-uncertain` verdict, whatever changed.
//! [`Engine::floor`] reports that set for a set of entries, without a change:
//! how many entries are on the floor, which classes of record and which files
//! (sites) they reach, and a greedy order in which to resolve the sites.
//!
//! # Algorithm
//!
//! With `N` nodes and `M` edges in the forward closure of the entries, `E`
//! entries and `S` open sites inside that closure:
//!
//! 1. Collect the sites: first-party nodes with at least one record whose
//!    [`AffectedOptions::disposition`] is [`Disposition::Open`]. `O(records)`.
//! 2. Condense the closure into strongly connected components (iterative
//!    Tarjan), which numbers them in reverse topological order. `O(N + M)`.
//! 3. Give every component a bitset of the sites it reaches: its own sites,
//!    united with the bitsets of its successors, which are complete when it is
//!    visited. `O(M · S / 64)` word operations, less where a successor reaches
//!    no site. The bitsets take `N · S / 8` bytes; above [`PASS_BYTES`] the
//!    sites are processed in several passes of that size.
//! 4. Copy each entry's bitset into an `E × S` bit matrix (`E · S / 8` bytes)
//!    and build its transpose. Counts per site and per class are sums over the
//!    set bits.
//! 5. Greedy hitting set: keep, per entry, the number of unresolved sites it
//!    reaches and, per site, the number of entries it alone keeps on the floor.
//!    A pick scans the sites (`O(S)`) and walks the picked site's column; an
//!    entry's row is scanned once, when it is down to one site. `O(picks · S +
//!    set bits + E · S / 64)`. The weighted order ([`FloorOrder::Weighted`])
//!    rescans the row of every entry of the picked site instead: `O(picks · S +
//!    Σ r² / 2)` over entries that reach `r` sites, if every site is picked.
//!
//! Measured on a graph of 94k nodes and 1.4M edges with 15k entries and 3.8k
//! sites: about 50 ms and 60 MB on one thread, so nothing here runs in
//! parallel.

use std::{
    collections::{BTreeMap, HashMap},
    fmt,
    io::{self, Write},
};

use serde::Serialize;

use crate::{
    change::{AffectedOptions, Disposition},
    engine::Engine,
    glob::{Glob, is_glob},
    set::NodeSet,
    view::{Adjacency, GraphView, NodeId},
    vocab::{EdgeFilter, Effect, Origin, RefFlags, UncertaintyKind},
};

const NONE: u32 = u32::MAX;

/// Memory for the per-component site bitsets of one propagation pass. More
/// sites than fit are handled in several passes.
pub const PASS_BYTES: usize = 256 << 20;

/// The class of an uncertainty record: what it is, what its reference does with
/// the target, and where the unknown value comes from.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct UncertaintyClass {
    /// Category.
    pub kind: UncertaintyKind,
    /// What the reference does with its target.
    pub effect: Effect,
    /// Where the unknown part of the value comes from.
    pub origin: Origin,
}

impl fmt::Display for UncertaintyClass {
    /// `kind/effect/origin`, for example `computed_specifier/read/param`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}/{}", self.kind, self.effect, self.origin)
    }
}

/// First-party uncertainty records by [`Disposition`].
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct DispositionCounts {
    /// Records that trigger `--on-uncertain`.
    pub open: usize,
    /// Records covered by a scope.
    pub scoped: usize,
    /// Unresolved specifiers covered by what their resolution recorded.
    pub probed: usize,
    /// Records ignored by default.
    pub ignored: usize,
    /// Records a directory membership stands for.
    pub membership: usize,
    /// Reads with a computed path that are bounded by names
    /// ([`Disposition::Named`]).
    #[serde(skip_serializing_if = "is_zero")]
    pub named: usize,
    /// Module loads with an unknown target that the declared packages bound
    /// ([`Disposition::Declared`]).
    #[serde(skip_serializing_if = "is_zero")]
    pub declared: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// One class of open records.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FloorClass {
    /// The class.
    #[serde(flatten)]
    pub class: UncertaintyClass,
    /// Open records of the class in the graph.
    pub records: usize,
    /// Files with such a record.
    pub files: usize,
    /// Of those files, the ones at least one entry reaches.
    pub files_reached: usize,
    /// Entries that reach at least one record of the class.
    pub reached: usize,
    /// Entries whose every open record is of this class: resolving the class
    /// frees them.
    pub exclusive: usize,
}

/// One open record of a site.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct FloorRecord {
    /// 1-based line; 0 if unknown.
    pub line: u32,
    /// The record's class.
    #[serde(flatten)]
    pub class: UncertaintyClass,
    /// Human-readable detail.
    pub detail: String,
}

/// A site: a first-party file with open records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FloorSite {
    /// The file.
    pub node: NodeId,
    /// Its open records, by line.
    pub records: Vec<FloorRecord>,
    /// Entries that reach the site.
    pub reached: usize,
    /// Entries that reach no other site: resolving this site alone frees them.
    pub exclusive: usize,
}

/// How the greedy order picks the next site.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FloorOrder {
    /// The unresolved site that frees the most entries: the entries for which
    /// it is the last unresolved site. If no site frees an entry, the site
    /// that the most entries reach. This looks one step ahead only: of two
    /// sites that free many entries together and none alone, neither ranks
    /// high.
    #[default]
    Freed,
    /// Every entry on the floor splits one point evenly between the unresolved
    /// sites it reaches, and the site with the most points is picked. An
    /// entry that only one site holds counts fully for that site, as in
    /// [`FloorOrder::Freed`]; an entry that two sites hold counts half for
    /// each, so groups of sites that free many entries together
    /// rank high too.
    Weighted,
}

/// One step of the greedy order.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct FloorPick {
    /// Index of the picked site in [`FloorReport::sites`].
    pub site: usize,
    /// Entries that leave the floor with this pick: every other site they reach
    /// was picked before.
    pub freed: usize,
    /// Entries still on the floor after this pick.
    pub remaining: usize,
}

/// The result of [`Engine::floor`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FloorReport {
    /// Number of entries evaluated.
    pub entries: usize,
    /// Entries that reach at least one open record.
    pub floor: usize,
    /// First-party records in the whole graph, by disposition.
    pub records: DispositionCounts,
    /// Classes with open records: most entries reaching first, then most
    /// exclusive entries, most records, and class order.
    pub classes: Vec<FloorClass>,
    /// Every site in the graph: most entries reaching first, then by path.
    pub sites: Vec<FloorSite>,
    /// The greedy order ([`FloorOrder`]). Ties go to the site that more entries
    /// reach, then to the smaller path. Ends when the floor is empty or
    /// after the requested number of picks.
    pub greedy: Vec<FloorPick>,
    /// What the assumptions matched, when the report was made with some
    /// ([`Engine::floor_assuming`]). Every other field then describes the graph
    /// as if the assumed records were resolved.
    pub assumed: Option<Assumed>,
    /// Entries evaluated without `TYPE_ONLY` edges
    /// ([`Engine::floor_for_runners`]).
    pub runtime_entries: usize,
}

/// One evaluation: order, picks, memory per pass, assumptions and per-entry
/// edge modes.
struct Run<'a> {
    by: FloorOrder,
    picks: usize,
    pass_bytes: usize,
    assume: &'a FloorAssumptions,
    runtime_entries: Option<&'a NodeSet>,
}

/// A file, or one line of it, whose open records are assumed resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssumedSite {
    /// Path relative to the graph root.
    pub path: String,
    /// Only the records on this line; `None`: every record of the file.
    pub line: Option<u32>,
}

impl AssumedSite {
    /// Parses `path` or `path:line`.
    pub fn parse(s: &str) -> AssumedSite {
        let (path, line) = match s.rsplit_once(':') {
            Some((p, l)) if !p.is_empty() && !l.is_empty() => match l.parse::<u32>() {
                Ok(l) => (p, Some(l)),
                Err(_) => (s, None),
            },
            _ => (s, None),
        };
        AssumedSite {
            path: crate::mem::normalize(path).to_owned(),
            line,
        }
    }
}

impl fmt::Display for AssumedSite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(l) => write!(f, "{}:{l}", self.path),
            None => f.write_str(&self.path),
        }
    }
}

/// A set of classes: `kind[/effect[/origin]]`, where a missing part or `*`
/// matches anything.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ClassPattern {
    /// The kind, or any.
    pub kind: Option<UncertaintyKind>,
    /// The effect, or any.
    pub effect: Option<Effect>,
    /// The origin, or any.
    pub origin: Option<Origin>,
}

impl ClassPattern {
    /// Parses `kind`, `kind/effect` or `kind/effect/origin`; `*` matches
    /// anything.
    pub fn parse(s: &str) -> Result<ClassPattern, String> {
        fn part<T>(
            p: Option<&str>,
            what: &str,
            parse: impl Fn(&str) -> Option<T>,
            names: impl Iterator<Item = &'static str>,
        ) -> Result<Option<T>, String> {
            match p {
                None | Some("*") => Ok(None),
                Some(p) => parse(p).map(Some).ok_or_else(|| {
                    format!(
                        "unknown {what} `{p}`: expected one of {}",
                        names.collect::<Vec<_>>().join(", ")
                    )
                }),
            }
        }
        let mut parts = s.split('/');
        let pattern = ClassPattern {
            kind: part(
                parts.next(),
                "uncertainty kind",
                UncertaintyKind::parse,
                UncertaintyKind::names(),
            )?,
            effect: part(parts.next(), "effect", Effect::parse, Effect::names())?,
            origin: part(parts.next(), "origin", Origin::parse, Origin::names())?,
        };
        if parts.next().is_some() {
            return Err(format!("`{s}`: expected kind[/effect[/origin]]"));
        }
        Ok(pattern)
    }

    /// Whether `class` is in the set.
    pub fn matches(&self, class: &UncertaintyClass) -> bool {
        self.kind.is_none_or(|k| k == class.kind)
            && self.effect.is_none_or(|e| e == class.effect)
            && self.origin.is_none_or(|o| o == class.origin)
    }
}

impl fmt::Display for ClassPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = |s: Option<&'static str>| s.unwrap_or("*");
        write!(
            f,
            "{}/{}/{}",
            name(self.kind.map(UncertaintyKind::as_str)),
            name(self.effect.map(Effect::as_str)),
            name(self.origin.map(Origin::as_str))
        )
    }
}

/// Open records to treat as resolved, to predict the effect of a fix
/// ([`Engine::floor_assuming`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FloorAssumptions {
    /// Files or lines whose open records are resolved.
    pub sites: Vec<AssumedSite>,
    /// Classes whose open records are resolved.
    pub classes: Vec<ClassPattern>,
    /// Nodes that are not traversed: paths, task ids, or globs over them. A
    /// pattern `prefix**` matches every node whose path starts with the
    /// prefix (`task:**`). No entry reaches such a node, or anything
    /// through it, as if every reference to it were gone. An entry that is
    /// itself named is still evaluated. This predicts the effect of
    /// removing a hub.
    pub cut: Vec<String>,
}

impl FloorAssumptions {
    /// Whether nothing is assumed.
    pub fn is_empty(&self) -> bool {
        self.sites.is_empty() && self.classes.is_empty() && self.cut.is_empty()
    }
}

/// What the assumptions of a report matched ([`FloorReport::assumed`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Assumed {
    /// Open records assumed resolved.
    pub records: usize,
    /// Files with such a record.
    pub files: usize,
    /// Nodes that are not traversed ([`FloorAssumptions::cut`]).
    pub cut: usize,
    /// Assumptions that matched nothing: `path[:line]` or `kind/effect/origin`
    /// without an open first-party record, and cut nodes that aren't in the
    /// graph.
    pub unmatched: Vec<String>,
    /// The floor without the assumptions.
    pub floor_before: usize,
    /// Entries that leave the floor with the assumptions: `floor_before` minus
    /// the floor.
    pub freed: usize,
}

impl FloorReport {
    /// Sites that at least one entry reaches.
    pub fn sites_reached(&self) -> usize {
        self.sites.iter().filter(|s| s.reached > 0).count()
    }

    /// The report as JSON (see the README, "`uncertainty`"): `version`,
    /// `entries`, `floor`, `records`, `open_sites`, `open_sites_reached`,
    /// `classes`, `sites` (all of them), `greedy` and, with assumptions,
    /// `assumed`. Paths are relative to the graph root.
    pub fn to_value<G: GraphView>(&self, g: &G) -> serde_json::Value {
        let sites: Vec<serde_json::Value> = self
            .sites
            .iter()
            .map(|s| {
                serde_json::json!({
                    "path": g.path(s.node),
                    "reached": s.reached,
                    "exclusive": s.exclusive,
                    "records": s.records,
                })
            })
            .collect();
        let greedy: Vec<serde_json::Value> = self
            .greedy
            .iter()
            .map(|p| {
                let s = &self.sites[p.site];
                serde_json::json!({
                    "path": g.path(s.node),
                    "freed": p.freed,
                    "remaining": p.remaining,
                    "reached": s.reached,
                })
            })
            .collect();
        let mut v = serde_json::json!({
            "version": 1,
            "entries": self.entries,
            "floor": self.floor,
            "records": self.records,
            "open_sites": self.sites.len(),
            "open_sites_reached": self.sites_reached(),
            "classes": self.classes,
            "sites": sites,
            "greedy": greedy,
            "runtime_only_entries": self.runtime_entries,
        });
        if let Some(a) = &self.assumed {
            v["assumed"] = serde_json::json!(a);
        }
        v
    }

    /// Writes the readable report: totals, the classes, and the sites of the
    /// greedy order with up to three records each.
    pub fn write_text<G: GraphView>(&self, g: &G, out: &mut dyn Write) -> io::Result<()> {
        let pct = if self.entries == 0 {
            0.0
        } else {
            100.0 * self.floor as f64 / self.entries as f64
        };
        writeln!(
            out,
            "uncertainty floor: {} of {} entries ({pct:.1}%) reach open uncertainty and are \
             selected on every change",
            self.floor, self.entries
        )?;
        if let Some(a) = &self.assumed {
            let cut = match a.cut {
                0 => String::new(),
                n => format!(" and {n} node(s) not traversed"),
            };
            writeln!(
                out,
                "what if: {} open record(s) in {} file(s) assumed resolved{cut}; without that the \
                 floor is {} ({} more entries)",
                a.records, a.files, a.floor_before, a.freed
            )?;
            for u in &a.unmatched {
                writeln!(out, "warning: no open record matches the assumption {u}")?;
            }
        }
        let r = &self.records;
        writeln!(
            out,
            "records (first-party): {} open, {} scoped, {} probed, {} ignored by default{}{}",
            r.open,
            r.scoped,
            r.probed,
            r.ignored,
            if r.named > 0 {
                format!(", {} bounded by names", r.named)
            } else {
                String::new()
            },
            if r.declared > 0 {
                format!(", {} load(s) bounded by declared dependencies", r.declared)
            } else {
                String::new()
            }
        )?;
        let reached = self.sites_reached();
        writeln!(
            out,
            "open sites: {} file(s), {} reached by entries",
            self.sites.len(),
            reached
        )?;
        if !self.classes.is_empty() {
            writeln!(out)?;
            writeln!(
                out,
                "open records by class (reached: entries that reach one; exclusive: entries with \
                 no open record of another class)"
            )?;
            writeln!(
                out,
                "{:>8} {:>8} {:>8} {:>10}  kind/effect/origin",
                "records", "files", "reached", "exclusive"
            )?;
            for c in &self.classes {
                writeln!(
                    out,
                    "{:>8} {:>8} {:>8} {:>10}  {}",
                    c.records, c.files, c.reached, c.exclusive, c.class
                )?;
            }
        }
        if !self.greedy.is_empty() {
            writeln!(out)?;
            writeln!(
                out,
                "top sites, in the order to resolve them (freed: entries that leave the floor \
                 with this site; left: the floor after it)"
            )?;
            writeln!(
                out,
                "{:>4} {:>8} {:>8} {:>8}  site",
                "#", "freed", "left", "reached"
            )?;
            for (i, p) in self.greedy.iter().enumerate() {
                let s = &self.sites[p.site];
                writeln!(
                    out,
                    "{:>4} {:>8} {:>8} {:>8}  {}",
                    i + 1,
                    p.freed,
                    p.remaining,
                    s.reached,
                    g.path(s.node)
                )?;
                const SHOWN: usize = 3;
                for rec in s.records.iter().take(SHOWN) {
                    writeln!(
                        out,
                        "{:>33}  line {} {}: {}",
                        "",
                        rec.line,
                        rec.class,
                        truncate(&rec.detail, 80)
                    )?;
                }
                if s.records.len() > SHOWN {
                    writeln!(
                        out,
                        "{:>33}  and {} more record(s)",
                        "",
                        s.records.len() - SHOWN
                    )?;
                }
            }
            if self.greedy.len() < reached {
                writeln!(
                    out,
                    "{} more reached site(s) not shown (--top N; --format json lists every site)",
                    reached - self.greedy.len()
                )?;
            }
        }
        Ok(())
    }
}

/// The first `max` characters of the first line of `s`, with `…` if something
/// was cut.
fn truncate(s: &str, max: usize) -> String {
    let line = s.lines().next().unwrap_or("");
    let mut out: String = line.chars().take(max).collect();
    if out.len() < s.len() {
        out.push('…');
    }
    out
}

/// Calls `f` with the index of every set bit of `words`, in increasing order.
#[inline]
fn for_each_bit(words: &[u64], mut f: impl FnMut(usize)) {
    for (wi, &w) in words.iter().enumerate() {
        let mut w = w;
        while w != 0 {
            f(wi * 64 + w.trailing_zeros() as usize);
            w &= w - 1;
        }
    }
}

/// The first bit set in both `a` and `b`.
fn first_common(a: &[u64], b: &[u64]) -> Option<usize> {
    a.iter().zip(b).enumerate().find_map(|(wi, (x, y))| {
        let w = x & y;
        (w != 0).then(|| wi * 64 + w.trailing_zeros() as usize)
    })
}

/// The nodes reachable from `entries` along accepted edges, without entering a
/// node of `cut` (the entries themselves are always included).
fn closure_without(
    adj: &Adjacency,
    filter: EdgeFilter,
    entries: &NodeSet,
    cut: &NodeSet,
) -> NodeSet {
    let all = filter.is_all();
    let mut out = entries.clone();
    let mut queue: Vec<u32> = entries.iter().map(|n| n.0).collect();
    while let Some(v) = queue.pop() {
        let (targets, attrs) = adj.neighbors(NodeId(v));
        for (&t, &a) in targets.iter().zip(attrs) {
            if (all || filter.accepts(a)) && !cut.contains(NodeId(t)) && out.insert(NodeId(t)) {
                queue.push(t);
            }
        }
    }
    out
}

/// Strongly connected components of the subgraph induced by `closure` (which
/// must be closed under accepted edges that don't enter `cut`), by an iterative
/// Tarjan. Returns the component of every node (`NONE` outside `closure`) and
/// the number of components. Components are numbered in reverse topological
/// order: an accepted edge between two components goes from the larger number
/// to the smaller.
// Invariant: the DFS call stack is non-empty while descending; Tarjan: the root of a component is
// on the stack.
#[allow(clippy::expect_used)]
fn condense(
    adj: &Adjacency,
    filter: EdgeFilter,
    closure: &NodeSet,
    cut: &NodeSet,
) -> (Vec<u32>, usize) {
    let n = adj.node_count() as usize;
    let all = filter.is_all();
    let mut index = vec![NONE; n];
    let mut low = vec![0u32; n];
    let mut comp = vec![NONE; n];
    let mut stack: Vec<u32> = Vec::new();
    // (node, position of its next edge)
    let mut call: Vec<(u32, u32)> = Vec::new();
    let mut next = 0u32;
    let mut ncomp = 0u32;
    for root in closure.iter() {
        if index[root.index()] != NONE {
            continue;
        }
        index[root.index()] = next;
        low[root.index()] = next;
        next += 1;
        stack.push(root.0);
        call.push((root.0, 0));
        while let Some(&(v, start)) = call.last() {
            let (targets, attrs) = adj.neighbors(NodeId(v));
            let mut descend = None;
            let mut pos = start as usize;
            while pos < targets.len() {
                let (w, a) = (targets[pos], attrs[pos]);
                pos += 1;
                if !(all || filter.accepts(a)) || cut.contains(NodeId(w)) {
                    continue;
                }
                if index[w as usize] == NONE {
                    descend = Some(w);
                    break;
                }
                if comp[w as usize] == NONE {
                    // Visited and not yet in a component: on the stack.
                    low[v as usize] = low[v as usize].min(index[w as usize]);
                }
            }
            if let Some(w) = descend {
                call.last_mut().expect("frame").1 = pos as u32;
                index[w as usize] = next;
                low[w as usize] = next;
                next += 1;
                stack.push(w);
                call.push((w, 0));
                continue;
            }
            call.pop();
            if low[v as usize] == index[v as usize] {
                loop {
                    let w = stack.pop().expect("component member");
                    comp[w as usize] = ncomp;
                    if w == v {
                        break;
                    }
                }
                ncomp += 1;
            }
            if let Some(&(p, _)) = call.last() {
                low[p as usize] = low[p as usize].min(low[v as usize]);
            }
        }
    }
    (comp, ncomp as usize)
}

/// The `entries × ncols` bit matrix of the sites each entry reaches along
/// accepted edges: row `i` occupies `ncols.div_ceil(64)` words and has bit
/// `col_of[n]` set for every node `n` in the closure of `entries[i]` with a
/// column. `closure` is the closure of all entries; edges into `cut` are not
/// followed. `pass_bytes` bounds the working memory ([`PASS_BYTES`]).
#[allow(clippy::too_many_arguments)]
fn reach_rows(
    adj: &Adjacency,
    filter: EdgeFilter,
    closure: &NodeSet,
    cut: &NodeSet,
    entries: &[NodeId],
    col_of: &[u32],
    ncols: usize,
    pass_bytes: usize,
) -> Vec<u64> {
    let tw = ncols.div_ceil(64);
    let mut rows = vec![0u64; entries.len() * tw];
    if tw == 0 || entries.is_empty() {
        return rows;
    }
    let (comp, ncomp) = condense(adj, filter, closure, cut);
    // Nodes grouped by component.
    let mut off = vec![0usize; ncomp + 1];
    for n in closure.iter() {
        off[comp[n.index()] as usize + 1] += 1;
    }
    for c in 0..ncomp {
        off[c + 1] += off[c];
    }
    let mut members = vec![0u32; off[ncomp]];
    let mut cursor = off.clone();
    for n in closure.iter() {
        let c = comp[n.index()] as usize;
        members[cursor[c]] = n.0;
        cursor[c] += 1;
    }
    drop(cursor);

    let all = filter.is_all();
    // Words per component in one pass.
    let w = (pass_bytes / 8 / ncomp).clamp(1, tw);
    let mut bits = vec![0u64; ncomp * w];
    let mut nonzero = vec![false; ncomp];
    let mut base = 0;
    while base < tw {
        let pw = w.min(tw - base);
        if base > 0 {
            bits.fill(0);
        }
        for c in 0..ncomp {
            let (done, rest) = bits.split_at_mut(c * w);
            let row = &mut rest[..pw];
            let mut last = NONE;
            for &v in &members[off[c]..off[c + 1]] {
                let col = col_of[v as usize];
                if col != NONE {
                    let word = col as usize / 64;
                    if (base..base + pw).contains(&word) {
                        row[word - base] |= 1 << (col % 64);
                    }
                }
                let (targets, attrs) = adj.neighbors(NodeId(v));
                for (&t, &a) in targets.iter().zip(attrs) {
                    if !(all || filter.accepts(a)) || cut.contains(NodeId(t)) {
                        continue;
                    }
                    let ct = comp[t as usize];
                    if ct as usize == c || ct == last {
                        continue;
                    }
                    debug_assert!((ct as usize) < c, "components in reverse topological order");
                    last = ct;
                    if nonzero[ct as usize] {
                        let src = &done[ct as usize * w..][..pw];
                        for (d, s) in row.iter_mut().zip(src) {
                            *d |= s;
                        }
                    }
                }
            }
            nonzero[c] = row.iter().any(|&x| x != 0);
        }
        for (i, e) in entries.iter().enumerate() {
            let c = comp[e.index()] as usize;
            rows[i * tw + base..][..pw].copy_from_slice(&bits[c * w..][..pw]);
        }
        base += pw;
    }
    rows
}

impl<G: GraphView> Engine<'_, G> {
    /// The uncertainty floor of `entries` under `opts`: which of them reach a
    /// first-party open uncertainty (and so get the `--on-uncertain`
    /// verdict from [`Engine::affected`] on every change), through which
    /// classes of record and which sites, and a greedy
    /// order ([`FloorOrder::Freed`]) of at most `picks` sites to resolve.
    /// Follows the engine's traversal filter.
    /// The policy in `opts` is not used: the report is the same for every
    /// policy.
    ///
    /// Sites are found exactly as in [`Engine::affected`]: external nodes never
    /// count, and a record counts when [`AffectedOptions::triggers`] says
    /// so. Cost: see the [module documentation](crate::floor).
    pub fn floor(&self, entries: &NodeSet, opts: &AffectedOptions, picks: usize) -> FloorReport {
        self.floor_with(entries, opts, FloorOrder::Freed, picks)
    }

    /// [`Engine::floor`] with the greedy order `by`.
    pub fn floor_with(
        &self,
        entries: &NodeSet,
        opts: &AffectedOptions,
        by: FloorOrder,
        picks: usize,
    ) -> FloorReport {
        self.floor_assuming(entries, opts, by, picks, &FloorAssumptions::default())
    }

    /// What-if: [`Engine::floor_with`] as if the open records that `assume`
    /// names were resolved. They are not counted as open, their files are
    /// no sites for them, and [`FloorReport::assumed`] says how many
    /// records matched, which assumptions matched nothing, and what the
    /// floor is without the assumptions. This predicts the effect of
    /// a fix on the floor without building it.
    pub fn floor_assuming(
        &self,
        entries: &NodeSet,
        opts: &AffectedOptions,
        by: FloorOrder,
        picks: usize,
        assume: &FloorAssumptions,
    ) -> FloorReport {
        let run = Run {
            by,
            picks,
            pass_bytes: PASS_BYTES,
            assume,
            runtime_entries: None,
        };
        self.floor_impl(entries, opts, &run)
    }

    /// [`Engine::floor_assuming`] with the per-entry edge modes of
    /// [`Engine::affected_with`] ([`crate::barrel::modes`]): the entries in
    /// `runtime_entries` (tests whose runner transpiles file by file) are
    /// evaluated without `TYPE_ONLY` edges, every other entry with the
    /// engine's traversal filter. With them the floor is what `affected`
    /// selects for those entries on a change that touches nothing.
    pub fn floor_for_runners(
        &self,
        entries: &NodeSet,
        runtime_entries: &NodeSet,
        opts: &AffectedOptions,
        by: FloorOrder,
        picks: usize,
        assume: &FloorAssumptions,
    ) -> FloorReport {
        let run = Run {
            by,
            picks,
            pass_bytes: PASS_BYTES,
            assume,
            runtime_entries: Some(runtime_entries),
        };
        self.floor_impl(entries, opts, &run)
    }

    // Invariant: a row counted once has exactly one common site; `perm` is a
    // permutation of the slots, so each slot is taken once.
    #[allow(clippy::expect_used)]
    fn floor_impl(&self, entries: &NodeSet, opts: &AffectedOptions, run: &Run<'_>) -> FloorReport {
        let &Run {
            by,
            picks,
            pass_bytes,
            assume,
            runtime_entries,
        } = run;
        let g = self.graph();

        // Assumed sites by node: (index of the assumption, line).
        let mut assumed_at: HashMap<u32, Vec<(usize, Option<u32>)>> = HashMap::new();
        for (k, a) in assume.sites.iter().enumerate() {
            if let Some(n) = g.node_by_path(&a.path) {
                assumed_at.entry(n.0).or_default().push((k, a.line));
            }
        }
        let mut site_hit = vec![false; assume.sites.len()];
        let mut class_hit = vec![false; assume.classes.len()];
        let mut cut = NodeSet::new(g.node_count());
        let mut cut_missing = Vec::new();
        for path in &assume.cut {
            // An exact path first: paths may contain glob characters (`[slug]`). Not for
            // `PREFIX**`, which is always the prefix form: the hub of a whole repository is
            // a node named `members:**`, and `members:**` means every hub.
            let before = cut.len();
            let prefix_form = path
                .strip_suffix("**")
                .is_some_and(|p| !p.is_empty() && !is_glob(p));
            if let Some(n) = g.node_by_path(path).filter(|_| !prefix_form) {
                cut.insert(n);
            } else if is_glob(path) {
                let pattern = crate::mem::normalize(path);
                let mut hit = NodeSet::new(g.node_count());
                match pattern.strip_suffix("**") {
                    // `prefix**`: every node whose path starts with the prefix, across `/`
                    // (task ids such as `task:apps/web#build`).
                    Some(prefix) if !is_glob(prefix) => {
                        for i in 0..g.node_count() {
                            if g.path(NodeId(i)).starts_with(prefix) {
                                hit.insert(NodeId(i));
                            }
                        }
                    }
                    _ => {
                        if let Ok(glob) = Glob::new(pattern) {
                            g.glob(&glob, &mut hit);
                        }
                    }
                }
                cut.union_with(&hit);
            } else {
                cut_missing.push(path.clone());
                continue;
            }
            if cut.len() == before && g.node_by_path(path).is_none() {
                cut_missing.push(path.clone());
            }
        }
        let (mut assumed_records, mut assumed_files) = (0, 0);

        // Records resolved per entry (`GraphView::is_entry_resolved`) are not open when
        // every entry of the report is one they are resolved for (the usual
        // case: jest and vitest tests); otherwise they are reported as open,
        // for every entry.
        let per_entry = g
            .resolved_entries()
            .is_some_and(|r| entries.iter().all(|e| r.contains(e)));
        // 1. Sites and record counts.
        let mut records = DispositionCounts::default();
        let mut sites: Vec<FloorSite> = Vec::new();
        for i in 0..g.node_count() {
            let node = NodeId(i);
            if !g.has_uncertainty(node) || g.is_external(node) {
                continue;
            }
            let here = assumed_at.get(&i);
            let mut assumed_here = false;
            let mut open = Vec::new();
            for u in g.uncertainties(node) {
                match self.disposition_of(node, &u, opts) {
                    // Covered by the edges and probes the view holds on each entry.
                    Disposition::Open if per_entry && g.is_entry_resolved(node, &u) => {
                        records.probed += 1;
                    }
                    Disposition::Open => {
                        let class = UncertaintyClass {
                            kind: u.kind,
                            effect: u.effect,
                            origin: u.origin,
                        };
                        let mut resolved = false;
                        for &(k, line) in here.into_iter().flatten() {
                            if line.is_none_or(|l| l == u.line) {
                                site_hit[k] = true;
                                resolved = true;
                            }
                        }
                        for (k, p) in assume.classes.iter().enumerate() {
                            if p.matches(&class) {
                                class_hit[k] = true;
                                resolved = true;
                            }
                        }
                        if resolved {
                            assumed_records += 1;
                            assumed_here = true;
                            continue;
                        }
                        records.open += 1;
                        open.push(FloorRecord {
                            line: u.line,
                            class,
                            detail: u.detail.to_owned(),
                        });
                    }
                    Disposition::Scoped => records.scoped += 1,
                    Disposition::Probed => records.probed += 1,
                    Disposition::Ignored => records.ignored += 1,
                    Disposition::Membership => records.membership += 1,
                    Disposition::Named => records.named += 1,
                    Disposition::Declared => records.declared += 1,
                }
            }
            assumed_files += usize::from(assumed_here);
            if !open.is_empty() {
                open.sort();
                sites.push(FloorSite {
                    node,
                    records: open,
                    reached: 0,
                    exclusive: 0,
                });
            }
        }
        // Path order: columns, and with them every tie, are ordered by path.
        sites.sort_by(|a, b| g.path(a.node).cmp(g.path(b.node)));

        // Classes, and the classes of each site as a bitset.
        let mut class_ids: BTreeMap<UncertaintyClass, usize> = BTreeMap::new();
        for s in &sites {
            for r in &s.records {
                class_ids.entry(r.class).or_insert(0);
            }
        }
        for (i, id) in class_ids.values_mut().enumerate() {
            *id = i;
        }
        let mut classes: Vec<FloorClass> = class_ids
            .keys()
            .map(|&class| FloorClass {
                class,
                records: 0,
                files: 0,
                files_reached: 0,
                reached: 0,
                exclusive: 0,
            })
            .collect();
        let cw = classes.len().div_ceil(64).max(1);
        let mut site_classes = vec![0u64; sites.len() * cw];
        for (si, s) in sites.iter().enumerate() {
            for r in &s.records {
                let c = class_ids[&r.class];
                classes[c].records += 1;
                let word = &mut site_classes[si * cw + c / 64];
                if *word & (1 << (c % 64)) == 0 {
                    *word |= 1 << (c % 64);
                    classes[c].files += 1;
                }
            }
        }

        // 2-4. The sites each entry reaches. Only sites in the entries' closure get a
        // column. Entries are grouped by the edges they follow, as in
        // `affected_with`: the runtime entries without type-only edges, the
        // others with the engine's filter.
        let all = self.options().traversal.for_uncertainty();
        let runtime = all.without_flags(RefFlags::TYPE_ONLY);
        let mut groups: Vec<(EdgeFilter, NodeSet)> = Vec::new();
        let mut runtime_count = 0;
        match runtime_entries.filter(|_| runtime != all) {
            Some(rt) => {
                let mut isolated = entries.clone();
                isolated.intersect_with(rt);
                let mut rest = entries.clone();
                rest.difference_with(rt);
                runtime_count = isolated.len();
                groups.push((all, rest));
                groups.push((runtime, isolated));
            }
            None => groups.push((all, entries.clone())),
        }
        let closures: Vec<NodeSet> = groups
            .iter()
            .map(|(filter, set)| closure_without(self.forward(), *filter, set, &cut))
            .collect();
        let entry_list: Vec<NodeId> = groups.iter().flat_map(|(_, set)| set.iter()).collect();
        let ne = entry_list.len();
        let mut col_of = vec![NONE; g.node_count() as usize];
        // Column → index into `sites`.
        let mut active: Vec<usize> = Vec::new();
        for (si, s) in sites.iter().enumerate() {
            if closures.iter().any(|c| c.contains(s.node)) {
                col_of[s.node.index()] = active.len() as u32;
                active.push(si);
            }
        }
        let ncols = active.len();
        let tw = ncols.div_ceil(64);
        let mut rows = Vec::with_capacity(ne * tw);
        for ((filter, set), closure) in groups.iter().zip(&closures) {
            let list: Vec<NodeId> = set.iter().collect();
            rows.extend(reach_rows(
                self.forward(),
                *filter,
                closure,
                &cut,
                &list,
                &col_of,
                ncols,
                pass_bytes,
            ));
        }
        drop(col_of);
        drop(closures);

        // Per entry: its number of sites and its classes. Per site: its entries (the
        // transposed matrix).
        let ew = ne.div_ceil(64);
        let mut cols = vec![0u64; ncols * ew];
        let mut reached = vec![0usize; ncols];
        let mut remaining = vec![0u32; ne];
        let mut floor = 0;
        let mut mask = vec![0u64; cw];
        for (i, rem) in remaining.iter_mut().enumerate() {
            let mut n = 0;
            mask.fill(0);
            for_each_bit(&rows[i * tw..][..tw], |c| {
                n += 1;
                reached[c] += 1;
                cols[c * ew + i / 64] |= 1 << (i % 64);
                let si = active[c];
                for (m, s) in mask.iter_mut().zip(&site_classes[si * cw..][..cw]) {
                    *m |= s;
                }
            });
            *rem = n;
            if n == 0 {
                continue;
            }
            floor += 1;
            let only = mask.iter().map(|w| w.count_ones()).sum::<u32>() == 1;
            for_each_bit(&mask, |c| {
                classes[c].reached += 1;
                if only {
                    classes[c].exclusive += 1;
                }
            });
        }
        for (c, &si) in active.iter().enumerate() {
            if reached[c] > 0 {
                for_each_bit(&site_classes[si * cw..][..cw], |k| {
                    classes[k].files_reached += 1
                });
            }
        }

        // 5. Greedy hitting set. `sole[c]`: entries whose only unresolved site is `c`.
        let mut unresolved = vec![u64::MAX; tw];
        if !ncols.is_multiple_of(64)
            && let Some(last) = unresolved.last_mut()
        {
            *last = (1u64 << (ncols % 64)) - 1;
        }
        let mut sole = vec![0usize; ncols];
        for (i, _) in remaining.iter().enumerate().filter(|(_, r)| **r == 1) {
            let c = first_common(&rows[i * tw..][..tw], &unresolved).expect("one site");
            sole[c] += 1;
        }
        for (c, &si) in active.iter().enumerate() {
            sites[si].reached = reached[c];
            sites[si].exclusive = sole[c];
        }
        // `score[c]`: what a pick maximizes. Freed: the entries that `c` alone keeps on
        // the floor (`sole`). Weighted: every floored entry splits one point,
        // `ONE`, evenly between the unresolved sites it reaches; the shares are
        // exact fixed-point numbers, so the scores don't depend on the order of
        // the updates.
        const ONE: u64 = 1 << 32;
        let mut score: Vec<u64> = match by {
            FloorOrder::Freed => sole.iter().map(|&n| n as u64).collect(),
            FloorOrder::Weighted => {
                let mut score = vec![0u64; ncols];
                for (i, &r) in remaining.iter().enumerate().filter(|(_, r)| **r > 0) {
                    let share = ONE / u64::from(r);
                    for_each_bit(&rows[i * tw..][..tw], |c| score[c] += share);
                }
                score
            }
        };
        let mut left = floor;
        // (column, freed, remaining floor)
        let mut order: Vec<(usize, usize, usize)> = Vec::new();
        while order.len() < picks && left > 0 {
            // Every entry that reaches an unresolved site is still on the floor, so the
            // site that the most floored entries reach is the one with the largest
            // `reached`. Columns are in path order: the first of equal keys wins.
            let mut best: Option<((u64, usize), usize)> = None;
            for_each_bit(&unresolved, |c| {
                let key = (score[c], reached[c]);
                if reached[c] > 0 && best.is_none_or(|(k, _)| key > k) {
                    best = Some((key, c));
                }
            });
            let Some((_, c)) = best else { break };
            unresolved[c / 64] &= !(1 << (c % 64));
            let mut freed = 0;
            for_each_bit(&cols[c * ew..][..ew], |i| {
                let row = &rows[i * tw..][..tw];
                remaining[i] -= 1;
                let r = remaining[i];
                if r == 0 {
                    freed += 1;
                    return;
                }
                match by {
                    FloorOrder::Freed => {
                        if r == 1 {
                            score[first_common(row, &unresolved).expect("one site")] += 1;
                        }
                    }
                    FloorOrder::Weighted => {
                        // The entry's share grows from 1/(r + 1) to 1/r.
                        let gain = ONE / u64::from(r) - ONE / u64::from(r + 1);
                        for (k, (x, y)) in row.iter().zip(&unresolved).enumerate() {
                            let mut w = x & y;
                            while w != 0 {
                                score[k * 64 + w.trailing_zeros() as usize] += gain;
                                w &= w - 1;
                            }
                        }
                    }
                }
            });
            debug_assert!(by != FloorOrder::Freed || freed as u64 == score[c]);
            left -= freed;
            order.push((c, freed, left));
        }

        // Final order of the sites: most entries first, then path (the sort is stable).
        let mut perm: Vec<usize> = (0..sites.len()).collect();
        perm.sort_by_key(|&i| std::cmp::Reverse(sites[i].reached));
        let mut pos = vec![0usize; sites.len()];
        for (new, &old) in perm.iter().enumerate() {
            pos[old] = new;
        }
        let mut slots: Vec<Option<FloorSite>> = sites.into_iter().map(Some).collect();
        let sites: Vec<FloorSite> = perm
            .iter()
            .map(|&i| slots[i].take().expect("a permutation"))
            .collect();
        let greedy = order
            .into_iter()
            .map(|(c, freed, remaining)| FloorPick {
                site: pos[active[c]],
                freed,
                remaining,
            })
            .collect();
        classes.sort_by(|a, b| {
            (b.reached, b.exclusive, b.records)
                .cmp(&(a.reached, a.exclusive, a.records))
                .then(a.class.cmp(&b.class))
        });
        let assumed = (!assume.is_empty()).then(|| {
            let none = FloorAssumptions::default();
            let before = Run {
                picks: 0,
                assume: &none,
                ..*run
            };
            let floor_before = self.floor_impl(entries, opts, &before).floor;
            let unmatched = (assume.sites.iter().zip(&site_hit))
                .filter(|(_, hit)| !**hit)
                .map(|(a, _)| a.to_string())
                .chain(
                    (assume.classes.iter().zip(&class_hit))
                        .filter(|(_, hit)| !**hit)
                        .map(|(p, _)| p.to_string()),
                )
                .chain(cut_missing.iter().cloned())
                .collect();
            Assumed {
                records: assumed_records,
                files: assumed_files,
                cut: cut.len(),
                unmatched,
                floor_before,
                freed: floor_before - floor,
            }
        });
        FloorReport {
            entries: ne,
            floor,
            records,
            classes,
            sites,
            greedy,
            assumed,
            runtime_entries: runtime_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MemGraph, RefKind};

    /// A pseudo-random graph with cycles: `n` nodes, about `3n` edges, every
    /// third node a site, every fifth an entry.
    fn random(n: u32, seed: u64) -> (MemGraph, NodeSet) {
        let mut x = seed;
        let mut rnd = move |m: u32| {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((x >> 33) % m as u64) as u32
        };
        let mut b = MemGraph::builder();
        let nodes: Vec<NodeId> = (0..n).map(|i| b.file(&format!("f{i:04}.ts"))).collect();
        for _ in 0..3 * n {
            let (from, to) = (rnd(n), rnd(n));
            // Mostly forward edges, so closures stay small enough to differ.
            if to > from || rnd(10) == 0 {
                b.edge(nodes[from as usize], nodes[to as usize], RefKind::Import);
            }
        }
        for (i, &node) in nodes.iter().enumerate() {
            if i % 3 == 0 {
                b.uncertainty(node, UncertaintyKind::ComputedSpecifier, 1, "x");
            }
        }
        let g = b.build().unwrap();
        let entries = NodeSet::from_iter_in(n, nodes.iter().copied().filter(|n| n.0 % 5 == 0));
        (g, entries)
    }

    #[test]
    fn several_passes_give_the_same_report() {
        let (g, entries) = random(600, 7);
        let e = Engine::new(&g);
        let opts = AffectedOptions::default();
        let none = FloorAssumptions::default();
        let run = Run {
            by: FloorOrder::Freed,
            picks: usize::MAX,
            pass_bytes: PASS_BYTES,
            assume: &none,
            runtime_entries: None,
        };
        let one = e.floor_impl(&entries, &opts, &run);
        assert!(one.sites_reached() > 128, "more than two words of sites");
        assert!(one.floor > 0 && one.floor < one.entries);
        // One word per component and pass.
        let run = Run {
            pass_bytes: 0,
            ..run
        };
        let many = e.floor_impl(&entries, &opts, &run);
        assert_eq!(one, many);
    }

    #[test]
    fn matches_a_search_per_entry() {
        for seed in 1..6 {
            let (g, entries) = random(300, seed);
            let e = Engine::new(&g);
            let opts = AffectedOptions::default();
            let r = e.floor(&entries, &opts, usize::MAX);
            let mut reached: BTreeMap<u32, usize> = BTreeMap::new();
            let (mut floor, mut exclusive) = (0, BTreeMap::new());
            for entry in entries.iter() {
                let from = NodeSet::from_iter_in(g.node_count(), [entry]);
                let sites: Vec<u32> = e
                    .deps(&from, None)
                    .iter()
                    .filter(|n| g.has_uncertainty(*n))
                    .map(|n| n.0)
                    .collect();
                floor += usize::from(!sites.is_empty());
                for &s in &sites {
                    *reached.entry(s).or_default() += 1;
                }
                if let [s] = sites[..] {
                    *exclusive.entry(s).or_default() += 1;
                }
            }
            assert_eq!(r.floor, floor, "seed {seed}");
            for s in &r.sites {
                assert_eq!(
                    s.reached,
                    reached.get(&s.node.0).copied().unwrap_or(0),
                    "seed {seed}, {}",
                    g.path(s.node)
                );
                assert_eq!(
                    s.exclusive,
                    exclusive.get(&s.node.0).copied().unwrap_or(0),
                    "seed {seed}, {}",
                    g.path(s.node)
                );
            }
            // Both greedy orders empty the floor, and their counts add up.
            let weighted = e.floor_with(&entries, &opts, FloorOrder::Weighted, usize::MAX);
            assert_eq!((&weighted.sites, &weighted.classes), (&r.sites, &r.classes));
            for r in [&r, &weighted] {
                assert_eq!(r.greedy.last().map(|p| p.remaining), Some(0), "seed {seed}");
                assert_eq!(r.greedy.iter().map(|p| p.freed).sum::<usize>(), floor);
                let mut left = floor;
                for p in &r.greedy {
                    left -= p.freed;
                    assert_eq!(p.remaining, left, "seed {seed}");
                }
                let mut picked: Vec<usize> = r.greedy.iter().map(|p| p.site).collect();
                picked.sort_unstable();
                picked.dedup();
                assert_eq!(picked.len(), r.greedy.len(), "no site twice, seed {seed}");
            }
        }
    }

    #[test]
    fn parses_assumptions() {
        let site = AssumedSite::parse("./src/a.ts:12");
        assert_eq!((site.path.as_str(), site.line), ("src/a.ts", Some(12)));
        assert_eq!(site.to_string(), "src/a.ts:12");
        let file = AssumedSite::parse("task:apps/web#build");
        assert_eq!(
            (file.path.as_str(), file.line),
            ("task:apps/web#build", None)
        );

        let class = |kind, effect, origin| UncertaintyClass {
            kind,
            effect,
            origin,
        };
        let read_param = class(
            UncertaintyKind::ComputedSpecifier,
            Effect::Read,
            Origin::Param,
        );
        let load = class(
            UncertaintyKind::ComputedSpecifier,
            Effect::Load,
            Origin::Unknown,
        );
        let p = ClassPattern::parse("computed_specifier").unwrap();
        assert!(p.matches(&read_param) && p.matches(&load));
        assert_eq!(p.to_string(), "computed_specifier/*/*");
        let p = ClassPattern::parse("computed_specifier/read").unwrap();
        assert!(p.matches(&read_param) && !p.matches(&load));
        let p = ClassPattern::parse("*/*/param").unwrap();
        assert!(p.matches(&read_param) && !p.matches(&load));
        assert_eq!(p.to_string(), "*/*/param");
        assert!(
            ClassPattern::parse("computed/read")
                .unwrap_err()
                .contains("unknown uncertainty kind `computed`")
        );
        assert!(ClassPattern::parse("eval_like/none/unknown/x").is_err());
    }

    #[test]
    fn truncates_details() {
        assert_eq!(truncate("short", 80), "short");
        assert_eq!(truncate("abcdef", 3), "abc…");
        assert_eq!(truncate("one\ntwo", 80), "one…");
    }
}
