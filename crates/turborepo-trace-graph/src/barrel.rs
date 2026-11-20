//! Export-level tracing through barrel files in `affected`.
//!
//! Graph edges are file to file, so a test that imports `{ x }` from a barrel
//! (`export * from './a'; export * from './b'`) depends on every re-exported
//! file. This module lets `affected` skip a modified file `f` for an entry when
//! changing `f` cannot change the entry's behaviour:
//!
//! 1. **Candidates** come from the caller ([`Candidate`]): modified JS files
//!    whose top level is free of side effects at the base *and* at the head
//!    revision, that load the same modules in the same order at both, export
//!    the same names at run time and import the same names (where modules are
//!    linked before they run, a name that is asked for and is not there is an
//!    error for every file that loads the module), parse without errors, and
//!    whose export names are fully known (their union over both versions is
//!    [`Candidate::exports`]). The CLI checks this with
//!    `turborepo_trace_lang_js::extract::exports` and a full parser.
//! 2. **Deferral.** A candidate's node is taken out of the seeds only when
//!    nothing else seeds it (probes, memberships, config inputs, packages,
//!    scopes).
//! 3. **Name tracking.** From `f`, a reverse walk follows the names that may
//!    carry changed content: `f`'s exports. Over a re-export edge `B → f` the
//!    names are mapped to `B`'s export names (`export *`: every name but
//!    `default`, `export {a as b}`: `a` → `b`, `export * as ns`: `ns`) and the
//!    walk continues at `B`. Any other edge `s → n` makes `s` *relevant* when
//!    it uses one of the tracked names (or everything: namespace imports with
//!    dynamic use, plain `require`, `import()`, mocks, framework and non-import
//!    edges, edges the module shape doesn't explain, files without a shape).
//!    `export *` names are not filtered by the other sources' exports, so a
//!    name that several sources provide (an ESM ambiguity, a CommonJS "first
//!    wins") reaches all of them: a collision involving `f` keeps `f` relevant.
//!
//!    A relevant node that is not an entry does not always end the walk:
//!
//!    - **Definition-level tracking** ([`ModuleSource::taint`]). The source
//!      says which of the importer's exports are defined (transitively, through
//!      its other top-level declarations) by code that reads the tracked names
//!      as values, and whether a statement that runs when the importer is
//!      loaded reads them. If no load-time statement does, `f`'s change can
//!      only be observed through those exports, and the walk continues from the
//!      importer with them (possibly none). The importer may itself be a
//!      deferred candidate; any other changed file ends the walk.
//!    - Without such facts, a node that is not itself changed and is *pure* at
//!      the head ([`Shape::pure_exports`]: side-effect-free top level, fully
//!      known exports) passes the change on through all of its exports: its top
//!      level never evaluates an imported binding (the side-effect check
//!      rejects that).
//!    - An entry is never a barrel of this walk: one that forwards a tracked
//!      name (`export { GET } from './handlers'` in a route file) is relevant,
//!      because what loads an entry may read what it exports, and a modified
//!      candidate that is an entry is affected.
//! 4. **Entries.** Entries in the reverse closure of a relevant node are
//!    affected (witness: `f`, the barrels, the relevant node, then the usual
//!    chain). Entries that reach `f` only through barrels without using its
//!    names are skipped, but only if they are jest or vitest tests that don't
//!    type-check (no ts-jest with diagnostics, no vitest typecheck, known
//!    transforms only); every other entry that reaches `f` stays affected. An
//!    entry that is itself a candidate is affected: a modified test always
//!    runs.
//!
//! A candidate's tracked names are all its exports, or with
//! [`Candidate::narrowed`] only the exports whose definition differs between
//! the base and the head revision (the caller compares them; a file whose top
//! level runs code can then be a candidate too, as long as what it does at load
//! time is the same at both revisions). That is **changed exports**:
//! the rule, its conditions construct by construct and its assumptions are
//! stated in `turborepo_trace_lang_js::symbols`. What the walk asks of an
//! importer ([`ModuleSource::shape`], [`ModuleSource::taint`],
//! [`ModuleSource::reads`]) is a function of the importer's text;
//! the CLI answers from the relation of the file, computed when the walk first
//! asks for it. Each end of a walk is recorded with how the file reads the
//! change ([`Reader`]), for `--explain`.
//!
//! Executing `f` through a barrel is harmless because `f` has no top-level side
//! effects and its imports are the same modules at both versions; reading `f`'s
//! exports is the only way its change can matter, and that is what the name
//! walk tracks.
//!
//! Two more rules for test entries whose runner only transpiles live in the
//! submodules:
//!
//! - [`erase`]: **erased-equal changes**. A modified file whose runtime
//!   JavaScript is the same at the base and head revisions
//!   ([`Barrels::erased`]: only types, comments or formatting changed) is not a
//!   change for entries that load it as a module through a per-file transpiler.
//! - [`modes`]: **type-only edges per entry**. [`Engine::affected_with`]
//!   evaluates those entries without `TYPE_ONLY` edges and every other entry
//!   with all edges ([`Barrels::runtime_entries`]).
//! - [`mocks`]: **mock-aware loading**. A change that reaches such an entry
//!   only through modules it replaces with a mock does not select it, nor does
//!   one that reaches a test only through a manual mock that no file it loads
//!   registers ([`Barrels::mocks`]).

use std::{
    borrow::Cow,
    collections::{BTreeSet, HashMap},
    sync::Arc,
};

use crate::{
    change::{Cause, Change, ChangeKind, ChangeSet, EntryResult, Verdict},
    engine::Engine,
    set::NodeSet,
    view::{Edge, GraphView, NodeId},
    vocab::EdgeFilter,
};

pub mod erase;
pub mod mocks;
pub mod modes;

pub use modes::Runner;

/// Names a file uses from one import: `None` for everything.
pub type UsedNames = Option<Vec<String>>;

/// One import or require of a module (see
/// `turborepo_trace_lang_js::extract::exports::Use`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Use {
    /// Start of the graph edge's span (the `import` / `require` keyword).
    pub start: u32,
    /// The specifier as written.
    pub spec: String,
    /// The names used.
    pub names: UsedNames,
}

/// What a re-export forwards.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReExportKind {
    /// `export * from`: every name except `default`.
    Star,
    /// `export * as ns from`.
    Namespace(String),
    /// `export {a as b} from`: `(name in the source, exported name)`.
    Named(Vec<(String, String)>),
}

/// One `export ... from`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReExport {
    /// Start of the graph edge's span (the `export` keyword).
    pub start: u32,
    /// The specifier.
    pub spec: String,
    /// What is forwarded.
    pub kind: ReExportKind,
}

/// The module shape of a file at the head revision, as far as name tracking
/// needs it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shape {
    /// False: nothing below can be relied on (every edge uses everything).
    pub understood: bool,
    /// Every edge of the file uses everything.
    pub uses_all: bool,
    /// Imports and requires.
    pub uses: Vec<Use>,
    /// Re-exports.
    pub reexports: Vec<ReExport>,
    /// The file's export names when its top level is free of side effects, it
    /// is an ES module and its exports are fully known; `None` otherwise. A
    /// changed module it imports can then only be observed through these
    /// exports.
    pub pure_exports: Option<Vec<String>>,
}

/// How one edge of a file reads the module it leads to
/// ([`ModuleSource::reading`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reading<'s> {
    /// Uses everything, or something not understood; the start of the import or
    /// require when the module shape has one for the edge.
    All(Option<u32>),
    /// Uses these names: the start of the import, the names.
    Names(u32, Vec<Cow<'s, str>>),
    /// Re-exports it.
    ReExport(Forwarded<'s>),
}

/// What a re-export forwards ([`ReExportKind`], borrowed where the source can
/// lend).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Forwarded<'s> {
    /// `export * from`: every name except `default`.
    Star,
    /// `export * as ns from`.
    Namespace(Cow<'s, str>),
    /// `export {a as b} from`: `(name in the source, exported name)`.
    Named(Vec<(Cow<'s, str>, Cow<'s, str>)>),
}

/// Module shapes and file texts at the head revision.
pub trait ModuleSource: Sync {
    /// The shape of a graph file (`None`: not a JS module or unreadable; its
    /// edges use everything).
    fn shape(&self, path: &str) -> Option<Arc<Shape>>;
    /// The text of a file (framework configs, for the type-check detection).
    fn text(&self, path: &str) -> Option<Arc<str>>;
    /// How the edge `e` of the file `path` reads the module it leads to. The
    /// walk asks this once per edge into a tracked file; the default
    /// answers from [`ModuleSource::shape`] ([`reading_of`]), and a source
    /// that can find one import without building the file's shape answers
    /// directly.
    fn reading<'s>(&'s self, path: &str, e: &Edge<'_>) -> Reading<'s> {
        reading_of(self.shape(path).as_deref(), e)
    }
    /// The walk is about to ask [`ModuleSource::reading`] for edges of each of
    /// these files (the importers of the tracked files that wait for their
    /// turn). A source that works a file out the first time it is asked
    /// about it can do that for all of them at once; the answers are the
    /// same with and without the call.
    fn prepare(&self, _files: &[&str]) {}
    /// [`Shape::pure_exports`] of the file `path`.
    fn pure_exports(&self, path: &str) -> Option<Vec<String>> {
        self.shape(path).and_then(|s| s.pure_exports.clone())
    }
    /// Per module the answers so far relied on: the files (root-relative) that
    /// register a test-runner mock of it, and the files that relied on it.
    /// A load-time call of such a file was taken to only build a value
    /// because of what the function's module defines, and a test that mocks
    /// that module sees the call and its arguments. Asked after the
    /// walks; a waiting entry that loads one of the mocking files and one of
    /// the relying files is affected.
    fn mockers(&self) -> Vec<(Vec<String>, Vec<String>)> {
        Vec::new()
    }
    /// Definition-level facts of the head version of `path`: what it derives
    /// from the given imports of modules whose names carry a change.
    /// `None`: unknown (the walk treats the file as using the change
    /// everywhere).
    fn taint(&self, _path: &str, _imports: &[TaintedImport]) -> Option<Taint> {
        None
    }
    /// Whether some statement of the head version of `path` reads, as a value,
    /// what the given imports carry (`None`: unknown). Asked for entries: a
    /// test that only names a changed export in types, or reads other
    /// members of a changed table, cannot observe the change.
    fn reads(&self, _path: &str, _imports: &[TaintedImport]) -> Option<bool> {
        None
    }
}

/// A modified file that may be skipped (see the module documentation).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Candidate {
    /// Root-relative path.
    pub path: String,
    /// The export names that may carry the change: all of them at the base and
    /// at the head revision, types included (union), or with
    /// [`Candidate::narrowed`] only the changed ones.
    pub exports: Vec<String>,
    /// `exports` lists only the names whose definition differs between the base
    /// and the head revision; what the file does at load time is the same
    /// at both.
    pub narrowed: bool,
    /// For changed exports that are an enum or an object table and differ only
    /// in some members: `(export, changed members)`. An importer that reads
    /// other members of such an export does not read the change
    /// ([`TaintedImport::members`]).
    pub members: Vec<(String, Vec<String>)>,
    /// Of `exports`, the names whose binding changed: they exist at one
    /// revision only, or are declared in another way (a function became a
    /// constant). A file that imports such a name by name reads the change
    /// when it is loaded: linking it can fail, and on an import cycle the
    /// binding can be uninitialized ([`TaintedImport::rebound`]).
    pub rebound: Vec<String>,
    /// The exports are declared in another order: only a reader of the whole
    /// module (`import *` used as an object, `require`, `export *`) can see
    /// that.
    pub reordered: bool,
}

/// The tracked name that stands for "the order of the module's exports changed"
/// ([`Candidate::reordered`]). No import by name reads it.
const ORDER: &str = "*";

/// Prefix of the tracked names that stand for "the binding of this export
/// changed" ([`Candidate::rebound`]); they travel with the name through `export
/// *`.
const REBOUND: char = '\u{1}';

/// One import declaration (or `require`) of a tracked module by a file
/// ([`ModuleSource::taint`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaintedImport {
    /// Start of the import: [`Use::start`].
    pub start: u32,
    /// The names it reads that carry a change (`None`: any name).
    pub names: Option<Vec<String>>,
    /// Of those names, the ones of which only some members changed: `(name,
    /// members)`.
    pub members: Vec<(String, Vec<String>)>,
    /// Of the names of the module, the ones whose binding changed
    /// ([`Candidate::rebound`]).
    pub rebound: Vec<String>,
}

/// What a file derives from tainted imports ([`ModuleSource::taint`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Taint {
    /// A statement that runs when the file is loaded reads a tainted value:
    /// every file that loads it may observe the change.
    pub evaluation: bool,
    /// The file's own export names whose definition reads a tainted value.
    pub exports: Vec<String>,
}

/// Input for barrel pruning and its sibling rules (see the module
/// documentation).
pub struct Barrels<'a> {
    /// Barrel candidates.
    pub candidates: Vec<Candidate>,
    /// Erased-equal files: root-relative paths of modified files whose runtime
    /// JavaScript is the same at the base and head revisions ([`erase`]).
    pub erased: Vec<String>,
    /// Evaluate test entries whose runner transpiles file by file without
    /// `TYPE_ONLY` edges ([`modes`]).
    pub runtime_entries: bool,
    /// Don't select test entries through modules they replace with a mock
    /// ([`mocks`]).
    pub mocks: bool,
    /// Shapes and texts.
    pub source: &'a dyn ModuleSource,
    /// Entries the caller knows to be compiled file by file with the types
    /// erased, without being tests of jest or vitest: the route files of a
    /// Next.js app (SWC) and Playwright specs (Babel). They count as
    /// [`Runner::Isolated`].
    pub isolated: Option<&'a NodeSet>,
}

impl<'a> Barrels<'a> {
    /// Barrel candidates only.
    pub fn new(candidates: Vec<Candidate>, source: &'a dyn ModuleSource) -> Self {
        Barrels {
            candidates,
            erased: Vec::new(),
            runtime_entries: false,
            mocks: false,
            source,
            isolated: None,
        }
    }

    /// How the runner of `entry` treats TypeScript: [`Barrels::isolated`], else
    /// [`entry_runner`].
    pub fn runner<G: GraphView>(&self, g: &G, entry: NodeId) -> Runner {
        if self.isolated.is_some_and(|s| s.contains(entry)) {
            Runner::Isolated
        } else {
            entry_runner(g, self.source, entry)
        }
    }
}

/// A candidate whose change was skipped for some entries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    /// The modified file.
    pub node: NodeId,
    /// Files that re-export it directly (the barrels it was not imported
    /// through).
    pub barrels: Vec<NodeId>,
    /// Files whose use of its names makes their dependents affected.
    pub relevant: usize,
    /// Entries that reach it but were not affected by it.
    pub entries: usize,
    /// Its changed export names, when only those were tracked
    /// ([`Candidate::narrowed`]).
    pub changed: Option<Vec<String>>,
    /// Files on the walk that passed the change on through some of their
    /// exports only ([`ModuleSource::taint`]).
    pub passed: usize,
}

/// How a file ends the walk from a changed export ([`Reader`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum How {
    /// An entry that uses the name.
    Entry,
    /// A statement that runs when the file is loaded reads the name.
    LoadTime,
    /// The file imports the name and nothing is known about what it does with
    /// it (no definition-level facts, and its top level is not free of side
    /// effects).
    Unknown,
    /// The file reaches the changed file by something that is not an import of
    /// names: a file read, `import()`, a mock, a framework edge.
    Edge,
    /// The file re-exports by name a binding that was added, removed or
    /// redeclared.
    Relinked,
    /// The file imports the name and is itself changed.
    Changed,
    /// The file registers a test-runner mock of a module whose function a
    /// load-time call was taken to only build a value with: the mock sees
    /// the call.
    Mocks,
}

/// A file whose use of a changed export selects the entries that load it: the
/// end of a name walk ([`PruneReport::readers`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reader {
    /// The file.
    pub node: NodeId,
    /// The changed file the walk started at.
    pub changed: NodeId,
    /// The module the file takes the name from (the changed file, or a file
    /// that passes its change on).
    pub from: NodeId,
    /// The name it reads there, as [`display_name`] shows it.
    pub name: String,
    /// How it reads it.
    pub how: How,
}

/// A tracked name in words: the export's name, or what the marker stands for.
pub fn display_name(name: &str) -> String {
    if name == ORDER {
        "the order of the exports".to_owned()
    } else if let Some(n) = name.strip_prefix(REBOUND) {
        format!("{n} (added, removed or redeclared)")
    } else if name.is_empty() {
        "the module".to_owned()
    } else {
        name.to_owned()
    }
}

/// What barrel pruning did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PruneReport {
    /// Candidates deferred from the seeds (barrel candidates and erased-equal
    /// files).
    pub deferred: usize,
    /// Per deferred barrel candidate.
    pub skipped: Vec<Skipped>,
    /// Per deferred erased-equal file: `relevant` counts the files that read
    /// one of them other than as a module of a transpiling runner,
    /// `barrels` is empty.
    pub erased: Vec<Skipped>,
    /// Entries that reach a deferred candidate and were not affected by any
    /// (they may still be affected by uncertainty).
    pub unaffected: Vec<NodeId>,
    /// Entries evaluated without type-only edges (`None`: every entry used the
    /// engine's traversal filter).
    pub runtime_entries: Option<NodeSet>,
    /// Entries a change reaches only through type-only edges, which their
    /// runner does not load: not affected by it (they may still be affected
    /// by uncertainty).
    pub type_only: Vec<NodeId>,
    /// The seed nodes of the change set (for explanations of
    /// [`PruneReport::type_only`]).
    pub seeds: Vec<NodeId>,
    /// Entries a change reaches only through modules they replace with a mock
    /// ([`mocks`]): not affected by it (they may still be affected by
    /// uncertainty).
    pub mocked: Vec<mocks::MockSkip>,
    /// What mock-aware loading did, by reason ([`mocks::MockStats`]).
    pub mock_stats: mocks::MockStats,
    /// Tests a change reaches only through manual mocks that no file they load
    /// registers ([`mocks::lazy`]): not affected by it.
    pub unregistered: Vec<mocks::Unregistered>,
    /// The files at which the name walks ended: each selects the entries that
    /// load it (for explanations; one per file, from the first candidate
    /// that reached it).
    pub readers: Vec<Reader>,
}

/// Why a change did not select an entry (see [`PruneReport::reasons`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    /// A re-exported side-effect-free file whose names the entry does not use:
    /// the file and the barrel on the shortest path from the entry.
    Barrel(NodeId, Option<NodeId>),
    /// A file of which only some exports changed, none of them used on a path
    /// to the entry: the file and the changed names.
    Unused(NodeId, Vec<String>),
    /// A file whose runtime JavaScript did not change.
    Erased(NodeId),
    /// A changed node reached only through type-only edges: the node and the
    /// first type-only hop `(importer, imported)` on the shortest path from
    /// the entry.
    TypeOnly(NodeId, Option<(NodeId, NodeId)>),
    /// A change reached only through a module the entry replaces with a mock:
    /// the module, the file whose load of it returns the mock, and where
    /// the mock is declared (`<file>:<line>`).
    Mocked(NodeId, NodeId, String),
    /// A change reached only through a manual mock that is not loaded: the
    /// mock, the module it stands in for, a file that registers it and the
    /// number of other files that do. The test loads none of them.
    Unregistered(NodeId, NodeId, NodeId, usize),
}

impl PruneReport {
    /// Why the changes `entry` reaches did not select it: the skipped
    /// candidates it reaches (barrel candidates with the barrel on the
    /// shortest path from the entry, erased-equal files) and, for an entry
    /// in [`PruneReport::type_only`], the nearest changed node with
    /// the type-only hop that leads to it.
    pub fn reasons<G: GraphView>(&self, engine: &Engine<'_, G>, entry: NodeId) -> Vec<Reason> {
        let g = engine.graph();
        let mut from = NodeSet::new(g.node_count());
        from.insert(entry);
        let mut out = Vec::new();
        for (list, erased) in [(&self.skipped, false), (&self.erased, true)] {
            for s in list {
                let mut to = NodeSet::new(g.node_count());
                to.insert(s.node);
                if let Some(path) = engine.somepath(&from, &to) {
                    out.push(if erased {
                        Reason::Erased(s.node)
                    } else if let Some(names) = &s.changed {
                        Reason::Unused(s.node, names.clone())
                    } else {
                        Reason::Barrel(s.node, (path.len() >= 2).then(|| path[path.len() - 2]))
                    });
                }
            }
        }
        for m in self.mocked.iter().filter(|m| m.entry == entry) {
            out.push(Reason::Mocked(m.module, m.importer, m.at.clone()));
        }
        for u in self.unregistered.iter().filter(|u| u.entry == entry) {
            out.push(Reason::Unregistered(u.mock, u.module, u.by, u.others));
        }
        if self.type_only.contains(&entry) {
            let to = NodeSet::from_iter_in(g.node_count(), self.seeds.iter().copied());
            if let Some(path) = engine.somepath(&from, &to) {
                let hop = path
                    .windows(2)
                    .find(|w| modes::type_only_hop(g, w[0], w[1]))
                    .map(|w| (w[0], w[1]));
                out.push(Reason::TypeOnly(path[path.len() - 1], hop));
            }
        }
        out
    }

    /// Adds `"edges": "runtime"` to the JSON results
    /// ([`crate::output::affected_entry_value`]) of entries evaluated
    /// without type-only edges, and `"edges": "all"` to the others. Nothing
    /// when every entry used the engine's filter.
    pub fn annotate<G: GraphView>(&self, g: &G, results: &mut [serde_json::Value]) {
        let Some(rt) = &self.runtime_entries else {
            return;
        };
        for r in results {
            let mode = r
                .get("entry")
                .and_then(|e| e.as_str())
                .and_then(|p| g.node_by_path(p))
                .map(|n| if rt.contains(n) { "runtime" } else { "all" });
            if let (Some(mode), Some(o)) = (mode, r.as_object_mut()) {
                o.insert("edges".to_owned(), mode.into());
            }
        }
    }

    /// Writes one line per [`Reason`] of `entry`:
    /// `  <file> skipped: not imported through <barrel>, side-effect free`,
    /// `  <file> skipped: only <names> changed in it, and nothing this entry
    /// loads uses them`,
    /// `  <file> skipped: only types, comments or formatting changed (same
    /// JavaScript as at the base revision)`, `  <file> not followed: <a>
    /// imports <b> only for types, which the test runner does not load`, `
    /// not selected through <module>: replaced by the mock at <file>:
    /// <line>` or `  not selected through <mock>: the mock of <module> for
    /// tests that load <file>, which the test does not`.
    pub fn write_reasons<G: GraphView>(
        &self,
        engine: &Engine<'_, G>,
        entry: NodeId,
        out: &mut dyn std::io::Write,
    ) -> std::io::Result<()> {
        let g = engine.graph();
        for r in self.reasons(engine, entry) {
            match r {
                Reason::Barrel(f, Some(b)) if b != entry => writeln!(
                    out,
                    "  {} skipped: not imported through {}, side-effect free",
                    g.path(f),
                    g.path(b)
                )?,
                Reason::Barrel(f, _) => writeln!(
                    out,
                    "  {} skipped: none of its exports is used, side-effect free",
                    g.path(f)
                )?,
                Reason::Unused(f, names) if names.is_empty() => writeln!(
                    out,
                    "  {} skipped: none of its exports changed, and it does the same when it is \
                     loaded",
                    g.path(f)
                )?,
                Reason::Unused(f, names) => writeln!(
                    out,
                    "  {} skipped: only {} changed in it, and nothing this entry loads uses {}",
                    g.path(f),
                    list(&names),
                    if names.len() == 1 { "it" } else { "them" }
                )?,
                Reason::Erased(f) => writeln!(
                    out,
                    "  {} skipped: only types, comments or formatting changed (same JavaScript as \
                     at the base revision)",
                    g.path(f)
                )?,
                Reason::TypeOnly(f, Some((a, b))) => writeln!(
                    out,
                    "  {} not followed: {} imports {} only for types, which the test runner does \
                     not load",
                    g.path(f),
                    g.path(a),
                    g.path(b)
                )?,
                Reason::Mocked(m, importer, at) if importer == entry => writeln!(
                    out,
                    "  not selected through {}: replaced by the mock at {at}",
                    g.path(m)
                )?,
                Reason::Mocked(m, importer, at) => writeln!(
                    out,
                    "  not selected through {}: replaced by the mock at {at} (loaded by {})",
                    g.path(m),
                    g.path(importer)
                )?,
                Reason::Unregistered(mock, module, by, others) => writeln!(
                    out,
                    "  not selected through {}: the mock of {} for tests that load {}{}, which \
                     the test does not",
                    g.path(mock),
                    g.path(module),
                    g.path(by),
                    match others {
                        0 => String::new(),
                        n => format!(" (or one of {n} other file(s) that register it)"),
                    }
                )?,
                Reason::TypeOnly(f, None) => writeln!(
                    out,
                    "  {} not followed: reached only through type-only imports, which the test \
                     runner does not load",
                    g.path(f)
                )?,
            }
        }
        Ok(())
    }
}

impl PruneReport {
    /// The [`Reader`] an affected entry was selected through: the first file of
    /// its witness chain that ended a name walk.
    pub fn reader_of(&self, witness: &[NodeId]) -> Option<&Reader> {
        let first = *witness.first()?;
        witness.iter().find_map(|n| {
            self.readers
                .iter()
                .find(|r| r.node == *n && r.changed == first)
        })
    }

    /// Writes what changed in the file an affected entry was selected through,
    /// and how the change gets to the entry: `  <file>: only <names>
    /// changed` and one line about the file that reads one of them
    /// (`detail`: what the caller knows about that file, such as
    /// the load-time statement). Nothing when the entry was not selected
    /// through a file tracked by its exports.
    pub fn write_reader<G: GraphView>(
        &self,
        g: &G,
        witness: &[NodeId],
        detail: &dyn Fn(&Reader) -> Option<String>,
        out: &mut dyn std::io::Write,
    ) -> std::io::Result<()> {
        let Some(r) = self.reader_of(witness) else {
            return Ok(());
        };
        if let Some(names) = self
            .skipped
            .iter()
            .find(|s| s.node == r.changed)
            .and_then(|s| s.changed.as_ref())
        {
            if names.is_empty() {
                writeln!(
                    out,
                    "  {}: none of its exports changed (the order of its exports, or a name that \
                     was added or removed)",
                    g.path(r.changed)
                )?;
            } else {
                writeln!(out, "  {}: only {} changed", g.path(r.changed), list(names))?;
            }
        }
        let (file, name, from) = (g.path(r.node), display_name(&r.name), g.path(r.from));
        let detail = detail(r).map(|d| format!(" ({d})")).unwrap_or_default();
        match r.how {
            How::Entry => writeln!(out, "  {file} uses {name} of {from}{detail}"),
            How::LoadTime => writeln!(
                out,
                "  {file} reads {name} of {from} when it is loaded{detail}: every entry that \
                 loads it is affected"
            ),
            How::Unknown => writeln!(
                out,
                "  {file} imports {name} of {from}, and what it does with it is not \
                 known{detail}: it counts as using it when it is loaded"
            ),
            How::Edge => writeln!(
                out,
                "  {file} reaches {from} by something other than an import of names (a file read, \
                 import(), a mock, a framework edge){detail}: it counts as using everything"
            ),
            How::Relinked => writeln!(
                out,
                "  {file} re-exports {name} of {from}: linking it can fail"
            ),
            How::Changed => writeln!(
                out,
                "  {file} imports {name} of {from} and is itself changed{detail}"
            ),
            How::Mocks => writeln!(
                out,
                "  {file} mocks a module whose function {from} calls while it is loaded{detail}: \
                 the call was taken to only build a value, and the mock sees it"
            ),
        }
    }
}

/// Up to four names, then how many more.
fn list(names: &[String]) -> String {
    let mut out = names
        .iter()
        .take(4)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    if names.len() > 4 {
        out.push_str(&format!(" and {} more", names.len() - 4));
    }
    out
}

/// The change set without the candidates' own modifications, and the
/// candidates' nodes (with candidate indexes; erased-equal files are numbered
/// after the barrel candidates). Used by [`Engine::affected_with`].
pub(crate) fn split_changes<G: GraphView>(
    g: &G,
    changes: &ChangeSet,
    b: &Barrels<'_>,
) -> (ChangeSet, Vec<(NodeId, usize)>) {
    let mut by_path: HashMap<&str, usize> = HashMap::new();
    for (i, c) in b.candidates.iter().enumerate() {
        by_path.insert(c.path.as_str(), i);
    }
    // An erased-equal file wins over a barrel candidate of the same path.
    for (i, p) in b.erased.iter().enumerate() {
        by_path.insert(p.as_str(), b.candidates.len() + i);
    }
    let mut rest = changes.clone();
    let mut cands = Vec::new();
    rest.files.retain(|c: &Change| {
        // An added file is a candidate only when the caller lists it as one.
        let Some(&i) = matches!(c.kind, ChangeKind::Modified | ChangeKind::Added)
            .then(|| by_path.get(c.path.as_str()))
            .flatten()
        else {
            return true;
        };
        match g.node_by_path(&c.path) {
            Some(n) => {
                cands.push((n, i));
                false
            }
            None => true,
        }
    });
    cands.sort_unstable();
    cands.dedup_by_key(|c| c.0);
    (rest, cands)
}

/// Removes from `seeds` the candidates that nothing in `rest` (the seeds of the
/// change set without the candidates' modifications) seeds; returns them.
pub(crate) fn defer(
    seeds: &mut Vec<(NodeId, Cause)>,
    rest: &[(NodeId, Cause)],
    cands: Vec<(NodeId, usize)>,
) -> Vec<(NodeId, usize)> {
    let other: std::collections::HashSet<NodeId> = rest.iter().map(|s| s.0).collect();
    let deferred: Vec<(NodeId, usize)> = cands
        .into_iter()
        .filter(|c| !other.contains(&c.0))
        .collect();
    let set: std::collections::HashSet<NodeId> = deferred.iter().map(|c| c.0).collect();
    seeds.retain(|s| !set.contains(&s.0));
    deferred
}

/// The import specifier of a framework edge for an import that a module mapper
/// or alias rewrites (`jest:moduleNameMapper <spec> -> <request>`,
/// `vitest:resolve.alias ...`).
pub fn aliased(spec: &str) -> Option<&str> {
    let (label, rest) = spec.split_once(' ')?;
    matches!(
        label,
        "jest:moduleNameMapper" | "vitest:resolve.alias" | "jest:resolve.alias"
    )
    .then(|| rest.split(" -> ").next().unwrap_or(rest))
}

fn names_of(names: &[String]) -> Vec<Cow<'static, str>> {
    names.iter().map(|n| Cow::Owned(n.clone())).collect()
}

fn forwarded(kind: &ReExportKind) -> Forwarded<'static> {
    match kind {
        ReExportKind::Star => Forwarded::Star,
        ReExportKind::Namespace(ns) => Forwarded::Namespace(Cow::Owned(ns.clone())),
        ReExportKind::Named(pairs) => Forwarded::Named(
            pairs
                .iter()
                .map(|(a, b)| (Cow::Owned(a.clone()), Cow::Owned(b.clone())))
                .collect(),
        ),
    }
}

/// How the edge `e` of a file with the module shape `shape` reads the module it
/// leads to (the default of [`ModuleSource::reading`]).
pub fn reading_of(shape: Option<&Shape>, e: &Edge<'_>) -> Reading<'static> {
    // The edge a module mapper adds next to an import's own edge (it has no span)
    // reads what the file's import of that specifier reads, when there is
    // exactly one.
    if let (Some(shape), Some(orig)) = (shape, e.specifier.and_then(aliased)) {
        if !shape.understood || shape.uses_all {
            return Reading::All(None);
        }
        let mut uses = shape.uses.iter().filter(|u| u.spec == orig);
        let mut reexports = shape.reexports.iter().filter(|r| r.spec == orig);
        return match (uses.next(), uses.next(), reexports.next(), reexports.next()) {
            (
                Some(Use {
                    names: Some(n),
                    start,
                    ..
                }),
                None,
                None,
                _,
            ) => Reading::Names(*start, names_of(n)),
            (
                Some(Use {
                    names: None, start, ..
                }),
                None,
                None,
                _,
            ) => Reading::All(Some(*start)),
            (None, _, Some(r), None) => Reading::ReExport(forwarded(&r.kind)),
            _ => Reading::All(None),
        };
    }
    let (Some(shape), Some(span), Some(spec)) = (shape, e.span, e.specifier) else {
        return Reading::All(None);
    };
    if !shape.understood || shape.uses_all {
        return Reading::All(None);
    }
    if let Some(r) = shape
        .reexports
        .iter()
        .find(|r| r.start == span.start && r.spec == spec)
    {
        return Reading::ReExport(forwarded(&r.kind));
    }
    match shape
        .uses
        .iter()
        .find(|u| u.start == span.start && u.spec == spec)
    {
        Some(Use {
            names: Some(n),
            start,
            ..
        }) => Reading::Names(*start, names_of(n)),
        Some(Use { start, .. }) => Reading::All(Some(*start)),
        None => Reading::All(None),
    }
}

/// `(name of the re-exported module, name of B)` for the `names` B exposes.
fn forward(kind: &Forwarded<'_>, names: &BTreeSet<String>) -> Vec<(String, String)> {
    match kind {
        // The markers of [`ORDER`] and [`REBOUND`] travel with the names.
        Forwarded::Star => names
            .iter()
            .filter(|n| n.strip_prefix(REBOUND).unwrap_or(n) != "default")
            .map(|n| (n.clone(), n.clone()))
            .collect(),
        Forwarded::Namespace(ns) => names
            .iter()
            .next()
            .map(|n| (n.clone(), ns.to_string()))
            .into_iter()
            .collect(),
        Forwarded::Named(pairs) => pairs
            .iter()
            .filter(|(from, _)| names.contains(from.as_ref()))
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect(),
    }
}

/// Transforms that only transpile (no type checking). `ts-jest` is decided
/// separately.
const TRANSPILE_ONLY: &[&str] = &[
    "babel-jest",
    "@swc/jest",
    "esbuild-jest",
    "jest-esbuild",
    "@sucrase/jest-plugin",
    "@swc-node/jest",
    "jest-transform-stub",
    "jest-css-modules-transform",
    "ts-jest",
];

/// The transformer `next/jest` installs: SWC, which compiles each file on its
/// own.
const NEXT_SWC_TRANSFORMER: &str = "next/dist/build/swc/jest-transformer";

/// Compilers a transformer of the repository may wrap: they compile one file at
/// a time and cannot report type errors. Their plugins and presets are matched
/// by prefix.
const TRANSPILERS: &[&str] = &["@babel/core", "@swc/core", "esbuild", "sucrase"];
const TRANSPILER_PREFIXES: &[&str] = &["@babel/", "@swc/", "babel-plugin-", "babel-preset-"];

/// Node builtins that cannot start a type check. `child_process`,
/// `worker_threads` and `vm` are left out: they can run a compiler the import
/// graph does not show.
const PLAIN_BUILTINS: &[&str] = &[
    "assert",
    "buffer",
    "crypto",
    "events",
    "fs",
    "module",
    "os",
    "path",
    "perf_hooks",
    "process",
    "stream",
    "string_decoder",
    "url",
    "util",
    "zlib",
];

/// How many files of a first-party transformer are inspected.
const MAX_TRANSFORMER_FILES: usize = 64;

/// What loading the package request `spec` means for a transformer:
/// `Some(true)` a transpiler, `Some(false)` something that cannot type-check
/// either (a plain builtin), `None` anything else.
fn transformer_import(spec: &str) -> Option<bool> {
    let name = spec.strip_prefix("node:").unwrap_or(spec);
    let pkg = package_of(name);
    if PLAIN_BUILTINS.contains(&pkg) {
        return Some(false);
    }
    if spec == NEXT_SWC_TRANSFORMER
        || TRANSPILE_ONLY.contains(&pkg) && pkg != "ts-jest"
        || TRANSPILERS.contains(&pkg)
        || TRANSPILER_PREFIXES.iter().any(|p| pkg.starts_with(p))
    {
        return Some(true);
    }
    // Version lookups (`require('next/package.json').version`) load no code.
    spec.ends_with("/package.json").then_some(false)
}

/// Whether the first-party transformer module `t` can only transpile: it and
/// the first-party files it loads have no uncertainty, load a known transpiler,
/// and load nothing else but plain builtins. A type check needs the TypeScript
/// compiler (or a tool that embeds it) in the transformer's process, and the
/// transformer would have to load it. Anything the list does not know keeps the
/// conservative answer.
fn only_transpiles<G: GraphView>(g: &G, t: NodeId) -> bool {
    let mut seen = vec![t];
    let mut transpiler = false;
    let mut i = 0;
    while i < seen.len() {
        let n = seen[i];
        i += 1;
        if seen.len() > MAX_TRANSFORMER_FILES || g.uncertainties(n).next().is_some() {
            return false;
        }
        for e in g.out_edges(n) {
            if !matches!(
                e.kind(),
                crate::vocab::RefKind::Import
                    | crate::vocab::RefKind::Require
                    | crate::vocab::RefKind::DynamicImport
                    | crate::vocab::RefKind::ReExport
            ) {
                continue;
            }
            let spec = e.specifier.unwrap_or("");
            if spec.starts_with("./") || spec.starts_with("../") || spec.starts_with('/') {
                match e.to {
                    Some(to) if !g.is_external(to) => {
                        if !seen.contains(&to) {
                            seen.push(to);
                        }
                    }
                    _ => return false,
                }
                continue;
            }
            match transformer_import(spec) {
                Some(is_transpiler) => transpiler |= is_transpiler,
                None => return false,
            }
        }
    }
    transpiler
}

/// The package name of a module request (`@scope/name/sub` → `@scope/name`).
fn package_of(request: &str) -> &str {
    let mut it = request.match_indices('/');
    let cut = if request.starts_with('@') {
        it.nth(1)
    } else {
        it.next()
    };
    cut.map_or(request, |(i, _)| &request[..i])
}

/// ts-jest without type checking: `isolatedModules: true` or `diagnostics:
/// false`.
fn diagnostics_off(text: &str) -> bool {
    let squashed: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '"' && *c != '\'')
        .collect();
    squashed.contains("isolatedModules:true") || squashed.contains("diagnostics:false")
}

/// vitest type tests (`*.test-d.ts`), which `vitest --typecheck` runs through
/// `tsc`.
fn is_type_test(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.contains(".test-d.") || name.contains(".spec-d.")
}

/// ts-jest transpiling each file on its own (`ts.transpileModule`):
/// `isolatedModules: true`.
fn isolated_modules(text: &str) -> bool {
    let squashed: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '"' && *c != '\'')
        .collect();
    squashed.contains("isolatedModules:true")
}

/// How the runner of `entry` treats TypeScript (see [`Runner`]). Skipping a
/// re-exported file needs a runner that only transpiles; the rules of [`erase`]
/// and [`modes`] need one that transpiles file by file. A file that is a test
/// of both jest and vitest (matched by a config of each) gets the weaker of the
/// two answers.
pub fn entry_runner<G: GraphView>(g: &G, source: &dyn ModuleSource, entry: NodeId) -> Runner {
    // Config files per runner: jest, vitest (`None`: not a test of that runner).
    let mut configs: [Option<Vec<NodeId>>; 2] = [None, None];
    let mut ts_jest = false;
    for e in g.out_edges(entry) {
        let spec = e.specifier.unwrap_or("");
        if spec.contains("ts-jest")
            || e.to
                .is_some_and(|t| g.package_info(g.package(t)).name == "ts-jest")
        {
            ts_jest = true;
        }
        let Some((fw, rest)) = spec.split_once(':') else {
            continue;
        };
        let own = match fw {
            "jest" => &mut configs[0],
            "vitest" => &mut configs[1],
            _ => continue,
        };
        let own = own.get_or_insert_with(Vec::new);
        let (role, request) = rest.split_once(' ').unwrap_or((rest, ""));
        match role {
            "config" => own.extend(e.to),
            // A transform that may type-check: anything but a known transpiler package,
            // next's SWC transformer, or a file of the repository that only wraps such
            // compilers ([`only_transpiles`]).
            "transform"
                if !(TRANSPILE_ONLY.contains(&package_of(request))
                    || request == NEXT_SWC_TRANSFORMER
                    || e.to
                        .is_some_and(|t| !g.is_external(t) && only_transpiles(g, t))) =>
            {
                return Runner::Checks;
            }
            _ => {}
        }
    }
    let text = |n: NodeId| source.text(g.path(n));
    let [jest, vitest] = configs;
    let jest = jest.map(|configs| {
        if !ts_jest
            || configs
                .iter()
                .any(|&c| text(c).is_some_and(|t| isolated_modules(&t)))
        {
            Runner::Isolated
        } else if configs
            .iter()
            .any(|&c| text(c).is_some_and(|t| diagnostics_off(&t)))
        {
            Runner::Transpiles
        } else {
            Runner::Checks
        }
    });
    let vitest = vitest.map(|configs| {
        if !is_type_test(g.path(entry))
            && configs
                .iter()
                .all(|&c| text(c).is_some_and(|t| !t.contains("typecheck")))
        {
            Runner::Isolated
        } else {
            Runner::Checks
        }
    });
    match (jest, vitest) {
        (Some(a), Some(b)) => a.min(b),
        (Some(a), None) | (None, Some(a)) => a,
        (None, None) => Runner::Checks,
    }
}

/// Reverse BFS from `seeds` over the edges `filter` accepts; returns parents
/// (`u32::MAX` for none and for seeds, which are marked in `seen`) and the
/// visit order.
pub(crate) fn bfs<G: GraphView>(
    engine: &Engine<'_, G>,
    filter: EdgeFilter,
    seeds: impl IntoIterator<Item = NodeId>,
) -> (Vec<u32>, Vec<NodeId>) {
    let g = engine.graph();
    let rev = engine.reverse();
    let mut parent = vec![u32::MAX; g.node_count() as usize];
    let mut seen = NodeSet::new(g.node_count());
    let mut order = Vec::new();
    for s in seeds {
        if seen.insert(s) {
            order.push(s);
        }
    }
    let mut i = 0;
    while i < order.len() {
        let n = order[i];
        i += 1;
        let (srcs, attrs) = rev.neighbors(n);
        for (&s, &a) in srcs.iter().zip(attrs) {
            let s = NodeId(s);
            if filter.accepts(a) && seen.insert(s) {
                parent[s.index()] = n.0;
                order.push(s);
            }
        }
    }
    (parent, order)
}

/// The chain from a BFS root to `n` (root first).
pub(crate) fn chain(parent: &[u32], mut n: NodeId) -> Vec<NodeId> {
    let mut v = vec![n];
    while parent[n.index()] != u32::MAX {
        n = NodeId(parent[n.index()]);
        v.push(n);
    }
    v.reverse();
    v
}

/// What [`track`] found for one deferred candidate.
struct Tracked {
    /// Relevant nodes with their path from `f` (`f` first, the relevant node
    /// last), the tracked name each reads and how.
    relevant: Vec<(NodeId, Vec<NodeId>, String, How)>,
    /// The direct re-exporters of `f`.
    barrels: Vec<NodeId>,
    /// Files that passed the change on through some of their exports only.
    passed: usize,
}

/// The name walk from one deferred candidate `f`. `deferred`: the deferred
/// barrel candidates (changed files that may still pass another file's change
/// on).
#[allow(clippy::too_many_arguments)]
fn track<G: GraphView>(
    engine: &Engine<'_, G>,
    b: &Barrels<'_>,
    filter: EdgeFilter,
    f: NodeId,
    exports: &[String],
    members: &[(String, Vec<String>)],
    rebound: &[String],
    reordered: bool,
    entries: &NodeSet,
    changed: &NodeSet,
    deferred: &NodeSet,
    needed: &NodeSet,
) -> Tracked {
    let g = engine.graph();
    let rev = engine.reverse();
    let mut state: HashMap<NodeId, BTreeSet<String>> = HashMap::new();
    // (node, name) -> (node, name) it carries, towards `f` (for witnesses).
    let mut via: HashMap<(NodeId, String), (NodeId, String)> = HashMap::new();
    let mut tracked: BTreeSet<String> = exports.iter().cloned().collect();
    tracked.extend(rebound.iter().map(|n| format!("{REBOUND}{n}")));
    if reordered {
        tracked.insert(ORDER.to_owned());
    }
    state.insert(f, tracked);
    let mut work = vec![f];
    // (relevant node, tracked node it reads, a tracked name it reads, how)
    let mut relevant: Vec<(NodeId, NodeId, String, How)> = Vec::new();
    let mut is_relevant = NodeSet::new(g.node_count());
    let mut barrels = Vec::new();
    let mut passed = NodeSet::new(g.node_count());
    // The tracked files whose importers were announced ([`ModuleSource::prepare`]).
    let mut announced = NodeSet::new(g.node_count());
    while let Some(n) = work.pop() {
        let names = state.get(&n).cloned().unwrap_or_default();
        // The names of `n` whose binding changed (they travel with a marker).
        let rebound_here: Vec<&str> = names
            .iter()
            .filter_map(|n| n.strip_prefix(REBOUND))
            .collect();
        // The importers that are asked below and for the files that wait behind `n`,
        // announced together, each file's once.
        let mut asked: Vec<&str> = Vec::new();
        for &m in std::iter::once(&n).chain(&work) {
            if !announced.insert(m) {
                continue;
            }
            let (srcs, attrs) = rev.neighbors(m);
            asked.extend(
                srcs.iter()
                    .zip(attrs)
                    .filter(|&(&s, &a)| {
                        filter.accepts(a)
                            && !is_relevant.contains(NodeId(s))
                            && needed.contains(NodeId(s))
                    })
                    .map(|(&s, _)| g.path(NodeId(s))),
            );
        }
        if asked.len() > 1 {
            b.source.prepare(&asked);
        }
        let (srcs, attrs) = rev.neighbors(n);
        let mut visited: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for (&s, &a) in srcs.iter().zip(attrs) {
            if !filter.accepts(a) || !visited.insert(s) {
                continue;
            }
            let s = NodeId(s);
            // A file that no waiting entry loads decides nothing, and neither does anything
            // that imports it.
            if is_relevant.contains(s) || !needed.contains(s) {
                continue;
            }
            let path = g.path(s);
            // A tracked name of `n` that `s` reads (for witnesses).
            let mut hit: Option<String> = None;
            let mut any_edge = false;
            // (name of `n`, name of `s`)
            let mut grown: Vec<(String, String)> = Vec::new();
            let mut reexports = false;
            // The imports of `n` that read tracked names, for [`ModuleSource::taint`];
            // `opaque`: a read that is not an import the module shape knows.
            let mut imports: Vec<TaintedImport> = Vec::new();
            let mut opaque = false;
            // A re-export by name of a binding that was added, removed or redeclared:
            // linking `s` can fail.
            let mut relinked: Option<String> = None;
            let first = || names.iter().next().cloned().unwrap_or_default();
            let is_rebound = |name: &str| rebound_here.contains(&name);
            for e in g.out_edges(s) {
                if e.to != Some(n) || !filter.accepts(e.attr) {
                    continue;
                }
                any_edge = true;
                match b.source.reading(path, &e) {
                    // An import of the whole module: with no changed export there is nothing
                    // for it to read.
                    Reading::All(Some(_)) if names.is_empty() => {}
                    Reading::All(Some(start)) => {
                        hit.get_or_insert_with(first);
                        imports.push(TaintedImport {
                            start,
                            names: None,
                            members: Vec::new(),
                            rebound: names
                                .iter()
                                .filter_map(|n| n.strip_prefix(REBOUND))
                                .map(str::to_owned)
                                .collect(),
                        });
                    }
                    // Not an import the module shape knows (a file read, a membership, a
                    // framework edge, `import()`): it may see the file itself, whatever its
                    // exports are.
                    Reading::All(None) => {
                        hit.get_or_insert_with(first);
                        opaque = true;
                    }
                    Reading::Names(start, used) => {
                        let read: Vec<String> = used
                            .iter()
                            .filter(|u| names.contains(u.as_ref()))
                            .map(|u| u.to_string())
                            .collect();
                        if let Some(u) = read.first() {
                            hit.get_or_insert_with(|| u.clone());
                            // Member-level changes are known for the candidate itself.
                            let members = if n == f {
                                members
                                    .iter()
                                    .filter(|(name, _)| read.contains(name))
                                    .cloned()
                                    .collect()
                            } else {
                                Vec::new()
                            };
                            let rebound = read.iter().filter(|u| is_rebound(u)).cloned().collect();
                            imports.push(TaintedImport {
                                start,
                                names: Some(read),
                                members,
                                rebound,
                            });
                        }
                    }
                    Reading::ReExport(kind) => {
                        reexports = true;
                        if let Forwarded::Named(pairs) = &kind
                            && let Some((from, _)) = pairs.iter().find(|(from, _)| is_rebound(from))
                        {
                            relinked.get_or_insert_with(|| format!("{REBOUND}{from}"));
                        }
                        grown.extend(forward(&kind, &names));
                    }
                }
            }
            if let Some(name) = relinked {
                is_relevant.insert(s);
                relevant.push((s, n, name, How::Relinked));
                continue;
            }
            if !any_edge {
                is_relevant.insert(s);
                relevant.push((s, n, first(), How::Edge));
                continue;
            }
            if let Some(name) = hit {
                // The exports through which `s` passes the change on, when that is the only
                // way it can: by its definition-level facts, else all of them when it is pure.
                let through: Result<Vec<String>, How> = if entries.contains(s) {
                    // An entry runs everything it reads. It passes nothing on, and is not
                    // selected, only when no statement of it reads what changed: it names
                    // the change in types, or reads other members of a changed table.
                    if opaque {
                        Err(How::Edge)
                    } else if !changed.contains(s) && b.source.reads(path, &imports) == Some(false)
                    {
                        Ok(Vec::new())
                    } else {
                        Err(How::Entry)
                    }
                } else {
                    let own_change = changed.contains(s) && !deferred.contains(s);
                    let facts = (!opaque && !own_change)
                        .then(|| b.source.taint(path, &imports))
                        .flatten();
                    match facts {
                        Some(t) if t.evaluation => Err(How::LoadTime),
                        Some(t) => {
                            passed.insert(s);
                            Ok(t.exports)
                        }
                        // Without facts a pure file passes the change on through all its
                        // exports, unless a binding it imports changed: then loading the
                        // file itself may fail.
                        None => (!changed.contains(s)
                            && imports.iter().all(|i| i.rebound.is_empty()))
                        .then(|| b.source.pure_exports(path))
                        .flatten()
                        .ok_or(if opaque {
                            How::Edge
                        } else if changed.contains(s) {
                            How::Changed
                        } else {
                            How::Unknown
                        }),
                    }
                };
                match through {
                    Ok(exports) => {
                        grown.extend(exports.into_iter().map(|p| (name.clone(), p)));
                    }
                    Err(how) => {
                        is_relevant.insert(s);
                        relevant.push((s, n, name, how));
                        continue;
                    }
                }
            }
            // An entry is not a barrel on the way to an importer: what loads it may read
            // what it exports (a framework calls the handlers of a route file, a runner the
            // default export of its config, a plugin makes a test of every export of a
            // file). An entry that forwards a tracked name uses it.
            if let Some((name, _)) = grown.first().filter(|_| entries.contains(s)) {
                is_relevant.insert(s);
                relevant.push((s, n, name.clone(), How::Entry));
                continue;
            }
            if n == f && reexports && !barrels.contains(&s) {
                barrels.push(s);
            }
            if grown.is_empty() {
                continue;
            }
            let entry = state.entry(s).or_default();
            let mut new = false;
            for (from, to) in grown {
                if entry.insert(to.clone()) {
                    new = true;
                    via.insert((s, to), (n, from));
                }
            }
            if new {
                work.push(s);
            }
        }
    }
    let path_to = |n: NodeId, name: String| {
        let mut v = vec![n];
        let mut cur = (n, name);
        while cur.0 != f && v.len() <= via.len() + 1 {
            match via.get(&cur) {
                Some(p) => {
                    v.push(p.0);
                    cur = p.clone();
                }
                None => break,
            }
        }
        v.reverse();
        v
    };
    let relevant = relevant
        .into_iter()
        .map(|(r, n, name, how)| {
            let mut p = path_to(n, name.clone());
            p.push(r);
            (r, p, name, how)
        })
        .collect();
    Tracked {
        relevant,
        barrels,
        passed: passed.len(),
    }
}

/// Decides the entries the deferred candidates affect; see the module
/// documentation. `filter` is the traversal filter of this evaluation pass
/// ([`modes`]).
#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve<G: GraphView>(
    engine: &Engine<'_, G>,
    b: &Barrels<'_>,
    changes: &ChangeSet,
    deferred: &[(NodeId, usize)],
    entries: &NodeSet,
    filter: EdgeFilter,
    is_affected: &mut NodeSet,
    results: &mut Vec<EntryResult>,
) -> PruneReport {
    let g = engine.graph();
    // `TURBO_TRACE_DEBUG_BARREL_TIMING=1` prints where this step
    // spends its time (stderr), like `TURBO_TRACE_DEBUG_MOCKS_TIMING`.
    let timing =
        turborepo_trace_core::debug::var_os("BARREL_TIMING").map(|_| std::time::Instant::now());
    let lap = |what: &str| {
        if let Some(t) = timing {
            eprintln!(
                "barrel resolve: {what} at {:.1} ms",
                t.elapsed().as_secs_f64() * 1e3
            );
        }
    };
    let mut report = PruneReport {
        deferred: deferred.len(),
        ..PruneReport::default()
    };
    let cause = |n: NodeId| Cause::Modified {
        path: g.path(n).to_owned(),
    };
    // Erased-equal files are numbered after the barrel candidates.
    let barrel_d: Vec<(NodeId, usize)> = deferred
        .iter()
        .copied()
        .filter(|d| d.1 < b.candidates.len())
        .collect();
    let erased_d: Vec<NodeId> = deferred
        .iter()
        .filter(|d| d.1 >= b.candidates.len())
        .map(|d| d.0)
        .collect();
    let is_erased = NodeSet::from_iter_in(g.node_count(), erased_d.iter().copied());
    // Entries that reach a candidate at all: the ones whose runner allows skipping
    // it wait for the walks below, the others are affected. An erased-equal
    // file needs a runner that transpiles file by file, and a modified entry
    // always runs.
    let mut pending: Vec<NodeId> = Vec::new();
    let mut owner: HashMap<NodeId, NodeId> = HashMap::new();
    let mut runners: HashMap<NodeId, Runner> = HashMap::new();
    let barrel_nodes: Vec<NodeId> = barrel_d.iter().map(|d| d.0).collect();
    let is_candidate = NodeSet::from_iter_in(g.node_count(), barrel_nodes.iter().copied());
    // The files from which a barrel candidate is reached.
    let mut reach = NodeSet::new(g.node_count());
    for (cands, erased) in [(&erased_d, true), (&barrel_nodes, false)] {
        if cands.is_empty() {
            continue;
        }
        let (parent, order) = bfs(engine, filter, cands.iter().copied());
        if !erased {
            for &n in &order {
                reach.insert(n);
            }
        }
        for &n in &order {
            if !entries.contains(n) || is_affected.contains(n) {
                continue;
            }
            let runner = *runners.entry(n).or_insert_with(|| b.runner(g, n));
            // A modified entry always runs: nothing in the graph reads the exports of a
            // route file, and the name walk only looks at importers.
            let skippable = if erased {
                runner == Runner::Isolated && !is_erased.contains(n)
            } else {
                runner != Runner::Checks && !barrel_nodes.contains(&n)
            };
            if skippable {
                if let std::collections::hash_map::Entry::Vacant(v) = owner.entry(n) {
                    v.insert(chain(&parent, n)[0]);
                    pending.push(n);
                }
            } else {
                let witness = chain(&parent, n);
                is_affected.insert(n);
                results.push(EntryResult {
                    entry: n,
                    verdict: Verdict::Affected,
                    cause: Some(cause(witness[0])),
                    witness,
                    uncertain: None,
                });
            }
        }
    }
    lap(&format!(
        "entries that reach a candidate ({} pending, {} runner(s) decided)",
        pending.len(),
        runners.len()
    ));
    // The walks only have to look at what a waiting entry loads on its way to a
    // candidate: the files between the two (every file on such a way is one
    // from which the candidate is reached).
    let mut needed = NodeSet::new(g.node_count());
    if !barrel_d.is_empty() {
        let fwd = engine.forward();
        let mut stack: Vec<NodeId> = Vec::new();
        for &e in &pending {
            if reach.contains(e) && needed.insert(e) {
                stack.push(e);
            }
        }
        while let Some(n) = stack.pop() {
            let (targets, attrs) = fwd.neighbors(n);
            for (&t, &a) in targets.iter().zip(attrs) {
                let t = NodeId(t);
                if filter.accepts(a) && reach.contains(t) && needed.insert(t) {
                    stack.push(t);
                }
            }
        }
    }
    // Name walks.
    let mut changed = NodeSet::new(g.node_count());
    for c in &changes.files {
        if let Some(n) = g.node_by_path(&c.path) {
            changed.insert(n);
        }
    }
    let mut roots: Vec<NodeId> = Vec::new();
    let mut prefix: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for &(f, i) in &barrel_d {
        let c = &b.candidates[i];
        // The walks decide the entries that wait: with none waiting there is nothing to
        // find out.
        let Tracked {
            relevant,
            barrels,
            passed,
        } = if pending.is_empty() {
            Tracked {
                relevant: Vec::new(),
                barrels: Vec::new(),
                passed: 0,
            }
        } else {
            track(
                engine,
                b,
                filter,
                f,
                &c.exports,
                &c.members,
                &c.rebound,
                c.reordered,
                entries,
                &changed,
                &is_candidate,
                &needed,
            )
        };
        report.skipped.push(Skipped {
            node: f,
            barrels,
            relevant: relevant.len(),
            entries: 0,
            // For --explain: `Resource.Rules` for a table of which one member changed.
            changed: c.narrowed.then(|| {
                c.exports
                    .iter()
                    .map(|e| match c.members.iter().find(|(name, _)| name == e) {
                        Some((_, m)) if !m.is_empty() && m.len() <= 3 => {
                            format!("{e}.{}", m.join("/"))
                        }
                        Some((_, m)) if !m.is_empty() => format!("{e}.{} and others", m[0]),
                        _ => e.clone(),
                    })
                    .collect()
            }),
            passed,
        });
        for (r, path, name, how) in relevant {
            if let std::collections::hash_map::Entry::Vacant(v) = prefix.entry(r) {
                report.readers.push(Reader {
                    node: r,
                    changed: f,
                    from: path.len().checked_sub(2).map_or(f, |i| path[i]),
                    name,
                    how,
                });
                v.insert(path);
                roots.push(r);
            }
        }
    }
    // A mock of a module that the answers relied on sees the calls that were taken
    // to only build a value: the entries that load a file with such a mock and
    // a file that relied on the module are affected.
    if let (false, Some(&(f, _))) = (pending.is_empty(), barrel_d.first()) {
        let nodes = |paths: &[String]| -> Vec<NodeId> {
            paths.iter().filter_map(|p| g.node_by_path(p)).collect()
        };
        for (mockers, relying) in b.source.mockers() {
            let (mockers, relying) = (nodes(&mockers), nodes(&relying));
            let (parent, order) = bfs(engine, filter, mockers.iter().copied());
            let loads_mocker = NodeSet::from_iter_in(g.node_count(), order.iter().copied());
            let (_, order) = bfs(engine, filter, relying.iter().copied());
            let loads_relying = NodeSet::from_iter_in(g.node_count(), order.iter().copied());
            for &e in &pending {
                if is_affected.contains(e)
                    || !loads_mocker.contains(e)
                    || !loads_relying.contains(e)
                {
                    continue;
                }
                let tail = chain(&parent, e);
                let m = tail[0];
                let mut witness = vec![f];
                witness.extend(tail);
                is_affected.insert(e);
                results.push(EntryResult {
                    entry: e,
                    verdict: Verdict::Affected,
                    cause: Some(cause(f)),
                    witness,
                    uncertain: None,
                });
                if !report.readers.iter().any(|r| r.node == m && r.changed == f) {
                    report.readers.push(Reader {
                        node: m,
                        changed: f,
                        from: relying.first().copied().unwrap_or(f),
                        name: String::new(),
                        how: How::Mocks,
                    });
                }
            }
        }
    }
    lap(&format!(
        "name walks of {} candidate(s) ({} relevant root(s))",
        barrel_d.len(),
        roots.len()
    ));
    // Readers of the erased-equal files that don't go through the transpiler.
    if !erased_d.is_empty() {
        let readers = if pending.is_empty() {
            Vec::new()
        } else {
            erase::readers(engine, filter, &erased_d, &pending)
        };
        for &f in &erased_d {
            report.erased.push(Skipped {
                node: f,
                barrels: Vec::new(),
                relevant: readers.len(),
                entries: 0,
                changed: None,
                passed: 0,
            });
        }
        for (r, path) in readers {
            if let std::collections::hash_map::Entry::Vacant(v) = prefix.entry(r) {
                v.insert(path);
                roots.push(r);
            }
        }
    }
    lap(&format!(
        "readers of {} erased-equal file(s)",
        erased_d.len()
    ));
    if !pending.is_empty() && !roots.is_empty() {
        let (parent, order) = bfs(engine, filter, roots.iter().copied());
        let mut reached = NodeSet::new(g.node_count());
        for &n in &order {
            reached.insert(n);
        }
        for &e in &pending {
            // (An entry that waits for an erased-equal file may be a modified candidate.)
            if !reached.contains(e) || is_affected.contains(e) {
                continue;
            }
            let tail = chain(&parent, e);
            let mut witness = prefix.get(&tail[0]).cloned().unwrap_or_default();
            witness.pop(); // the relevant node starts `tail`
            witness.extend(tail);
            is_affected.insert(e);
            results.push(EntryResult {
                entry: e,
                verdict: Verdict::Affected,
                cause: Some(cause(witness[0])),
                witness,
                uncertain: None,
            });
        }
    }
    // Skipped entries, attributed to the candidate their shortest chain starts at.
    let mut per: HashMap<NodeId, usize> = HashMap::new();
    for &e in &pending {
        if !is_affected.contains(e) {
            report.unaffected.push(e);
            if let Some(&f) = owner.get(&e) {
                *per.entry(f).or_default() += 1;
            }
        }
    }
    for s in report.skipped.iter_mut().chain(&mut report.erased) {
        s.entries = per.get(&s.node).copied().unwrap_or(0);
    }
    report
        .unaffected
        .sort_unstable_by(|a, b| g.path(*a).cmp(g.path(*b)));
    lap("walk from the relevant files to the entries, report");
    report
}

#[cfg(test)]
mod prune_tests;
#[cfg(test)]
mod taint_tests;
#[cfg(test)]
mod tests;
