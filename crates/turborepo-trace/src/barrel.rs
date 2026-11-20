//! `affected`: inputs for export-level tracing through barrel files and for
//! changed exports (`turborepo_trace_graph::barrel`).
//!
//! - [`DiskSource`]: what the walk asks about the files it meets. The module
//!   shape of a file and the exports that read each of its imports are the
//!   per-file relation of `turborepo_trace_lang_js::symbols::relation`
//!   ([`ExportFacts`]), computed from the working tree the first time the walk
//!   asks for the file ([`Relations`]): the walk only meets the files between a
//!   waiting entry and a changed file, so a query reads, lexes and parses those
//!   and no others, and a build pays nothing for the rule. Texts (framework
//!   configs) and the definitions behind a load-time call that may only build a
//!   value are read from the working tree as well, on demand.
//! - [`candidates`]: modified JS files that may be skipped. Both the base
//!   revision (`git show`) and the working tree must be ES modules with fully
//!   known exports, load the same modules in the same order, link as before
//!   ([`links`], [`unlinked`]: where modules are linked, a name that is asked
//!   for and is not there breaks every file that loads the module), and parse
//!   without syntax or early errors (oxc parser and semantic checks: a syntax
//!   error in a file breaks every test that loads it, even through a barrel).
//!   Then either
//!   - the definition-level comparison of `turborepo_trace_lang_js::symbols`
//!     shows that the file does the same at load time at both revisions: the
//!     candidate carries only the exports whose definition changed
//!     ([`Candidate::narrowed`]), or
//!   - both versions are side-effect free at the top level: the candidate
//!     carries all exports.
//!
//! `TURBO_TRACE_DEBUG_EXPORTS=0` switches changed exports off (the A/B switch
//! of the rule): a modified file is then a candidate only when it is
//! side-effect free, with all its exports, and an importer passes a change on
//! only when it is side-effect free too. `TURBO_TRACE_DEBUG_EXPORT_DIFF=0` (the
//! comparison of a modified file), `TURBO_TRACE_DEBUG_SYMBOLS=0`
//! (what an importer reads) and `TURBO_TRACE_DEBUG_PURE_CALLS=0` (load-time
//! calls that only build a value) switch its parts off one by one, for
//! measurements.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

// Where the time of export-level tracking goes (`TURBO_TRACE_DEBUG_TIMING=1`).
use turborepo_trace_core::timing::Span;
use turborepo_trace_graph::{
    Change, ChangeKind, GraphView, NodeId,
    barrel::{
        Candidate, Forwarded, ModuleSource, ReExport, ReExportKind, Reading, Shape, Taint,
        TaintedImport, Use,
    },
};
use turborepo_trace_lang_js::{
    Dialect,
    extract::exports::{self as ex, Names, scan_shape},
    symbols::{
        self, Alone, Callee, Oracle, Prop, Step,
        relation::{Fact, Forward, Used},
    },
};

use crate::lockdiff::Git;
static T_SHAPE: Span = Span::new("exports: shape (decoded from the file's relation)");
static T_FACT_MISS: Span = Span::new("exports: definitions behind a call, read from the tree");
static T_READING: Span = Span::new("exports: reading of one edge (the import's line)");
static T_TAINT: Span = Span::new("exports: taint query");
static T_READS: Span = Span::new("exports: reads query of entries");
static T_CANDS: Span = Span::new("exports: candidate check per modified file (base text + diff)");

/// File extensions scanned as JS modules.
const JS_EXTS: &[&str] = &["js", "mjs", "cjs", "jsx", "ts", "mts", "cts", "tsx"];

fn is_js(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, ext)| JS_EXTS.contains(&ext))
}

/// The relations of changed exports of a graph's first-party JS files
/// (`turborepo_trace_lang_js::symbols::relation`), each computed from the file
/// in the working tree the first time it is asked for and kept for the query.
///
/// A relation is a function of the file's name and text alone, so it could be
/// traced with the file and stored. It is not: computing it costs a second lex
/// and a parse of every file at every build (about a fifth of the trace phase
/// when it was measured) and a tenth of the packed graph, and a query asks for
/// the relations of the files between a changed file and the tests that are
/// still undecided: none to a few hundred for most changes, one or two thousand
/// when a module that much of an application imports changes. That last query
/// is the one that pays, a third of a millisecond per file spread over the
/// cores ([`ModuleSource::prepare`]).
///
/// The files are the ones the JS extractor reads outside `node_modules`; the
/// file is read as it is in the working tree, which is what the graph of a
/// query describes. A file that cannot be read has no relation (its imports use
/// everything).
pub struct Relations<'g, G> {
    g: &'g G,
    root: PathBuf,
    cells: std::sync::OnceLock<Vec<std::sync::OnceLock<Option<Box<str>>>>>,
}

impl<'g, G: GraphView + Sync> Relations<'g, G> {
    /// The relations of the files of `g`, whose paths are relative to `root`.
    pub fn new(g: &'g G, root: &Path) -> Self {
        Relations {
            g,
            root: root.to_owned(),
            cells: std::sync::OnceLock::new(),
        }
    }

    /// The path of `n` in the working tree when the file has a relation: a JS
    /// file outside `node_modules`, as where the extractor reads
    /// environment facts and mocks.
    fn file(&self, n: NodeId) -> Option<PathBuf> {
        if self.g.lang(n) != Some("js") {
            return None;
        }
        // Decided on the path in the graph: the root may itself be below a directory
        // called `node_modules`.
        let rel = self.g.path(n);
        (!rel.split('/').any(|c| c == "node_modules")).then(|| self.root.join(rel))
    }

    /// The relation of `n` (a string that starts with `relation::TAG`).
    pub fn get(&self, n: NodeId) -> Option<&str> {
        let cells = self.cells.get_or_init(|| {
            (0..self.g.node_count())
                .map(|_| std::sync::OnceLock::new())
                .collect()
        });
        cells
            .get(n.0 as usize)?
            .get_or_init(|| {
                let path = self.file(n)?;
                let src = std::fs::read(&path).ok()?;
                symbols::relation::fact(&path, &src).map(String::into_boxed_str)
            })
            .as_deref()
    }

    /// Whether the relation of `n` is worked out already.
    pub fn ready(&self, n: NodeId) -> bool {
        self.cells
            .get()
            .and_then(|c| c.get(n.0 as usize))
            .is_some_and(|c| c.get().is_some())
    }

    /// Whether some file hands out a function that reads `this`
    /// (`relation::Fact::reads_this`). The one question about every file of the
    /// graph. `likely`: files to look at first (the ones with test-runner
    /// mock calls: such a file says yes when it has the word, without a
    /// parse). Then every file is looked at, in parallel, until one says
    /// yes; only a file with the word in it is lexed or parsed. A
    /// repository without any such file pays for a look at each of its files,
    /// once per query that asks.
    pub fn any_this(&self, likely: &[NodeId]) -> bool {
        use rayon::prelude::*;
        let says_yes = |n: NodeId| {
            let Some(path) = self.file(n) else {
                return false;
            };
            std::fs::read(&path).is_ok_and(|src| symbols::relation::has_this(&src))
                && self
                    .get(n)
                    .and_then(Fact::parse)
                    .is_some_and(|f| f.reads_this())
        };
        likely.iter().take(LIKELY_THIS).any(|&n| says_yes(n))
            || (0..self.g.node_count())
                .into_par_iter()
                .any(|i| says_yes(NodeId(i)))
    }
}

/// The per-file relations of the graph
/// (`turborepo_trace_lang_js::symbols::relation`), and the edges they are keyed
/// by.
pub struct ExportFacts<'a> {
    /// The node of a root-relative path.
    pub node: NodeOf<'a>,
    /// The relation fact of a node (a string that starts with `relation::TAG`).
    pub fact: FactOf<'a>,
    /// Whether [`ExportFacts::fact`] has its answer for a node at hand (it
    /// would not have to work it out).
    pub ready: Box<dyn Fn(NodeId) -> bool + Sync + 'a>,
    /// Calls the function with the span start and the specifier of every edge
    /// of a node that is written as an import, a `require` or a re-export.
    pub edges: EdgesOf<'a>,
    /// Whether some file of the graph hands out a function that reads `this`
    /// (`relation::Fact::reads_this`); asked at most once.
    pub any_this: Box<dyn Fn() -> bool + Sync + 'a>,
    /// The files with test-runner mock calls: the path of each and the module
    /// names it hands to them (`None`: a name that is not a literal). The
    /// argument: what a request has to contain to matter; files whose calls
    /// all name other things may be left out.
    pub mocks: MocksOf<'a>,
}

/// See [`ExportFacts::node`].
pub type NodeOf<'a> = Box<dyn Fn(&str) -> Option<NodeId> + Sync + 'a>;

/// See [`ExportFacts::fact`].
pub type FactOf<'a> = Box<dyn Fn(NodeId) -> Option<&'a str> + Sync + 'a>;

/// See [`ExportFacts::mocks`].
pub type MocksOf<'a> = Box<dyn Fn(&[String]) -> Vec<(String, Vec<Option<String>>)> + Sync + 'a>;

/// See [`ExportFacts::edges`].
pub type EdgesOf<'a> = Box<dyn Fn(NodeId, &mut dyn FnMut(u32, &str)) + Sync + 'a>;

/// What the walk asks about files: shapes and relations ([`Relations`]) and
/// texts, all from the working tree.
pub struct DiskSource<'a> {
    root: PathBuf,
    /// The file an import of a file resolves to: `(importer, specifier)` to a
    /// root-relative path.
    resolve: Option<Resolver<'a>>,
    /// The relations of the graph (`None`: no file has a shape).
    facts: Option<ExportFacts<'a>>,
    /// The changed files: what they define is not taken on trust (see [`At`]).
    changed: std::collections::HashSet<String>,
    shapes: Mutex<HashMap<String, Option<Arc<Shape>>>>,
    texts: Mutex<HashMap<String, Option<Arc<str>>>>,
    /// Definition-level facts of the files behind load-time calls
    /// (`symbols::Module`), read from the working tree, or why a file has
    /// none.
    modules: Mutex<HashMap<String, Result<Arc<symbols::Module>, &'static str>>>,
    /// Answer `ModuleSource::taint` (`TURBO_TRACE_DEBUG_SYMBOLS`).
    symbols: bool,
    /// Recognize load-time chains that only build a value
    /// (`TURBO_TRACE_DEBUG_PURE_CALLS`).
    pure_calls: bool,
    /// Take the word of [`known_builder`] for the builders of third-party
    /// packages (not under `--strict`: it is an assumption about code that
    /// is not read).
    packages: bool,
    /// Compare every answer of the relation with the one computed from the
    /// file's text (`TURBO_TRACE_DEBUG_EXPORTS_CHECK=1`; a development
    /// check, slow).
    check: bool,
    /// [`ExportFacts::any_this`], once asked.
    any_this: std::sync::OnceLock<bool>,
    stats: Mutex<SymbolStats>,
}

/// The fewest files [`ModuleSource::prepare`] hands to other threads.
const PREPARE_MIN: usize = 2;

/// How many of the likely files [`Relations::any_this`] looks at, one after the
/// other, before it looks at every file.
const LIKELY_THIS: usize = 32;

/// See [`DiskSource::with_resolver`].
pub type Resolver<'a> = Box<dyn Fn(&str, &str) -> Option<String> + Sync + 'a>;

/// Builders of React that only make a value from their arguments (as named
/// imports, or as members of the default or namespace import).
const REACT_BUILDERS: &[&str] = &["memo", "forwardRef", "lazy", "createContext"];

/// Members of a zod schema chain that only build a schema, whatever their
/// arguments are (checked against zod 3.23 and 4.1): they neither run the
/// schema, nor call a function they are handed, nor write to a registry with a
/// visible key, nor reject their arguments while the schema is built. Not
/// listed, among others: `parse` and its variants, `meta` and `register` (a
/// registry), `config`; and the builders that validate while building:
/// `discriminatedUnion` (zod 3 throws on a duplicate or missing discriminator),
/// `extend`, `safeExtend`, `merge`, `pick`, `omit`, `partial`, `required` (zod
/// 4 throws for a schema with refinements or for an unknown key), `extract`,
/// `exclude`, `keyof`, `tuple`, `templateLiteral`, `function`.
const ZOD_BUILDS: &[&str] = &[
    // Schemas.
    "string",
    "number",
    "bigint",
    "boolean",
    "date",
    "symbol",
    "undefined",
    "null",
    "void",
    "any",
    "unknown",
    "never",
    "nan",
    "literal",
    "enum",
    "nativeEnum",
    "object",
    "strictObject",
    "looseObject",
    "array",
    "record",
    "map",
    "set",
    "union",
    "intersection",
    "lazy",
    "promise",
    "custom",
    "instanceof",
    "preprocess",
    "coerce",
    // Methods.
    "optional",
    "nullable",
    "nullish",
    "or",
    "and",
    "default",
    "catch",
    "describe",
    "brand",
    "readonly",
    "refine",
    "superRefine",
    "transform",
    "pipe",
    "min",
    "max",
    "length",
    "email",
    "url",
    "uuid",
    "cuid",
    "regex",
    "startsWith",
    "endsWith",
    "includes",
    "trim",
    "toLowerCase",
    "toUpperCase",
    "datetime",
    "int",
    "positive",
    "negative",
    "nonnegative",
    "nonpositive",
    "finite",
    "safe",
    "gt",
    "gte",
    "lt",
    "lte",
    "multipleOf",
    "nonempty",
    "strict",
    "passthrough",
    "strip",
    "catchall",
];

/// Whether `flag(declaration)` of `flags/next` only builds the flag for this
/// declaration (checked against flags 4.4): the function validates its argument
/// and throws without a `decide` function, calls `adapter` when that is a
/// function, and calls the adapter's `origin`. So the declaration must name its
/// `decide` function literally, or take its adapter from a call of
/// `vercelAdapter()` of `@flags-sdk/vercel` (which returns the package's one
/// adapter object).
///
/// The declaration may spread a parameter of the function the call stands in (a
/// declarator `(d) => flag({ ...d, adapter: adapter() })`), as long as `decide`
/// or `adapter` is written after the spread: what is written last wins.
/// `vercel` says whether a chain yields the adapter object of
/// `@flags-sdk/vercel`.
fn flag_declaration(
    arg: &[(String, Prop<'_>)],
    vercel: &dyn Fn(Callee<'_>, &[Step]) -> bool,
) -> bool {
    // The properties after the last spread are the ones that count.
    let own = match arg.iter().rposition(|(_, p)| matches!(p, Prop::Spread)) {
        Some(at) => &arg[at + 1..],
        None => arg,
    };
    let spread = own.len() < arg.len();
    let get = |key: &str| {
        let mut found = own.iter().filter(|(name, _)| name == key);
        // A key given twice: the last one wins, not worth modelling.
        (found.next().map(|(_, p)| p), found.next().is_some())
    };
    let ((decide, twice_decide), (adapter, twice_adapter)) = (get("decide"), get("adapter"));
    if twice_decide || twice_adapter {
        return false;
    }
    match (decide, adapter) {
        // With a spread before it, an `adapter` of the spread value could still be there.
        (Some(Prop::Function), None) => !spread,
        (None | Some(Prop::Function), Some(Prop::Chain(c, steps))) => vercel(*c, steps),
        _ => false,
    }
}

/// Whether a chain on an import of `@flags-sdk/vercel` yields the package's
/// adapter object: `vercelAdapter()`, or the call of what
/// `createVercelAdapter(...)` returned.
fn vercel_adapter(c: Callee<'_>, steps: &[Step]) -> bool {
    c.spec == "@flags-sdk/vercel"
        && matches!(
            (c.name, steps),
            ("vercelAdapter", [Step::Call]) | ("createVercelAdapter", [Step::Call, Step::Call])
        )
}

/// Whether a chain on an import of a package only builds a value: the builders
/// known by package and name (`turborepo_trace_lang_js::symbols`,
/// "Calls that only build a value"). Each entry is a statement about a package:
/// its builder returns a new value and neither calls its arguments nor
/// registers anything.
///
/// A builder is listed only when, at the package versions checked, a change of
/// its arguments cannot be observed by a file that does not read the value it
/// returns: the builder does not throw for any argument of the declared types,
/// runs none of them, and stores the result nowhere else.
///
/// - `react` (18.3, 19): `memo`, `forwardRef`, `lazy`, `createContext` make an
///   object that holds their argument; the development build validates the
///   argument and may warn on the console, nothing more. Not `cache`: React
///   18's stable build does not export it, so a first use fails the whole file.
/// - `flags/next`, `flags/sveltekit` (flags 4.4): `flag(declaration)` for the
///   declarations [`flag_declaration`] accepts (`arg`: the declaration's
///   properties).
/// - `@flags-sdk/vercel` (1.6): `vercelAdapter()` returns the package's default
///   adapter (made once). Not `createVercelAdapter`: it creates a client, which
///   validates its argument and registers itself.
/// - `zod` (3.23, 4.1): a chain on `z` whose members are all in [`ZOD_BUILDS`].
fn known_builder(
    c: Callee<'_>,
    steps: &[Step],
    arg: Option<&[(String, Prop<'_>)]>,
    vercel: &dyn Fn(Callee<'_>, &[Step]) -> bool,
) -> bool {
    let call = |names: &[&str]| matches!(steps, [Step::Call]) && names.contains(&c.name);
    let member = |names: &[&str]| {
        matches!(c.name, "default" | "*")
            && matches!(steps, [Step::Member(m), Step::Call] if names.contains(&m.as_str()))
    };
    match c.spec {
        "react" => call(REACT_BUILDERS) || member(REACT_BUILDERS),
        "flags/next" | "flags/sveltekit" => {
            call(&["flag"]) && arg.is_some_and(|a| flag_declaration(a, vercel))
        }
        "@flags-sdk/vercel" => matches!((c.name, steps), ("vercelAdapter", [Step::Call])),
        spec if spec == "zod" || spec.starts_with("zod/") => {
            matches!(c.name, "z" | "default" | "*")
                && matches!(steps.first(), Some(Step::Member(_)))
                && steps
                    .iter()
                    .all(|s| !matches!(s, Step::Member(m) if !ZOD_BUILDS.contains(&m.as_str())))
        }
        _ => false,
    }
}

/// What a [`DiskSource`] knows about the imports of the file `path`
/// (`symbols`'s [`Oracle`]): the builders of [`known_builder`], and the
/// definitions in first-party files that did not change. A changed file's
/// definitions are not used: the question is always whether a statement does
/// the same at the base and at the head revision, and for an unchanged file the
/// head answer holds at both.
struct At<'s, 'a> {
    source: &'s DiskSource<'a>,
    /// The file whose statements are being judged (the questions may lead
    /// through other files: `path`).
    root: &'s str,
    path: &'s str,
    /// Hops between files left.
    fuel: u8,
}

impl At<'_, '_> {
    /// Whether the chain `steps` on the import yields the adapter object of
    /// `@flags-sdk/vercel`: directly, or through a constant of an unchanged
    /// first-party file that holds what `createVercelAdapter(...)`
    /// returned.
    fn vercel_adapter(&self, c: Callee<'_>, steps: &[Step]) -> bool {
        if self.source.packages && vercel_adapter(c, steps) {
            self.source.assumed(c.spec, self.root);
            return true;
        }
        let Some((target, m)) = self.target(c) else {
            return false;
        };
        let Some((spec, name, all)) = m.export_chain(c.name, steps) else {
            return false;
        };
        At {
            source: self.source,
            root: self.root,
            path: &target,
            fuel: self.fuel - 1,
        }
        .vercel_adapter(
            Callee {
                spec: &spec,
                name: &name,
            },
            &all,
        )
    }

    /// The unchanged first-party file the import resolves to, with its facts.
    fn target(&self, c: Callee<'_>) -> Option<(String, Arc<symbols::Module>)> {
        if self.fuel == 0 || !self.source.pure_calls {
            return None;
        }
        let target = self.source.resolve.as_ref()?(self.path, c.spec)?;
        if self.source.changed.contains(&target) {
            return None;
        }
        let m = self.source.module(&target).ok()?;
        // The answer is about the function this file defines: a test that mocks the
        // file sees the call instead ([`DiskSource::mockers`]).
        self.source.relied(&target, self.root);
        Some((target, m))
    }
}

impl Oracle for At<'_, '_> {
    fn builds_with(
        &self,
        c: Callee<'_>,
        steps: &[Step],
        literal_args: bool,
        arg: &[(String, Prop<'_>)],
    ) -> bool {
        self.source.pure_calls
            && (self
                .source
                .package_builder(self.root, c, steps, Some(arg), &|c, steps| {
                    self.vercel_adapter(c, steps)
                })
                || self.builds(c, steps, literal_args))
    }

    fn continues(&self, c: Callee<'_>, made: &[Step], steps: &[Step]) -> bool {
        if !self.source.pure_calls {
            return false;
        }
        // The factory ran where the constant was declared; calling what it returned
        // hands out the adapter.
        if self.source.packages
            && c.spec == "@flags-sdk/vercel"
            && c.name == "createVercelAdapter"
            && made == [Step::Call]
            && steps == [Step::Call]
        {
            self.source.assumed(c.spec, self.root);
            return true;
        }
        let all: Vec<Step> = made.iter().chain(steps).cloned().collect();
        self.builds(c, &all, false)
    }

    fn builds(&self, c: Callee<'_>, steps: &[Step], literal_args: bool) -> bool {
        if !self.source.pure_calls {
            return false;
        }
        if self
            .source
            .package_builder(self.root, c, steps, None, &|_, _| false)
        {
            return true;
        }
        let Some((target, m)) = self.target(c) else {
            return false;
        };
        m.builds(
            c.name,
            steps,
            literal_args,
            &At {
                source: self.source,
                root: self.root,
                path: &target,
                fuel: self.fuel - 1,
            },
        )
    }

    fn prim(&self, c: Callee<'_>) -> bool {
        self.target(c).is_some_and(|(_, m)| m.prim(c.name))
    }

    fn esm(&self, c: Callee<'_>) -> bool {
        // An ES module of the repository, as the graph has it (a changed file too: when
        // it was something else at the base, it is a changed file as a whole, and what
        // reads it loads it).
        self.source
            .resolve
            .as_ref()
            .and_then(|resolve| resolve(self.path, c.spec))
            .and_then(|target| self.source.fact(&target))
            .is_some_and(|(_, fact)| fact.es_module())
    }
}

/// What the definition-level facts did during one `affected` run.
#[derive(Clone, Debug, Default)]
pub struct SymbolStats {
    /// Importers that passed a change on through their exports only.
    pub passed: usize,
    /// Load-time statements that read a change but only build a value (closure
    /// factories, higher-order components, known builders).
    pub pure_calls: usize,
    /// Importers that read a change at load time, by reason: the count and one
    /// example.
    pub evaluated: BTreeMap<String, (usize, String)>,
    /// Importers without facts, by reason: the count and one example.
    pub unknown: BTreeMap<String, (usize, String)>,
    /// The importers without facts, with the reason.
    pub unknown_files: BTreeMap<String, String>,
    /// The third-party packages whose builders were taken to only build a value
    /// ([`known_builder`]): an assumption, not made under `--strict`.
    pub assumed_builders: BTreeSet<String>,
    /// The packages that were taken to export a name a modified file newly
    /// imports from them: an assumption, not made under `--strict`.
    pub assumed_names: BTreeSet<String>,
    /// Per first-party file whose definitions were consulted for a load-time
    /// call or an imported constant, and per package whose builders were
    /// taken on trust (`pkg:<specifier>`): the files whose load-time
    /// statements relied on it ([`At`]).
    pub relied: BTreeMap<String, BTreeSet<String>>,
    /// The files with a mock call that names such a file or package, with the
    /// request and what it names ([`DiskSource::mocking`]).
    pub mockers: BTreeMap<String, String>,
    /// The importers that read a change at load time, with the reason (the walk
    /// ends at them: every entry that loads one is affected).
    pub load_time_readers: BTreeMap<String, String>,
    /// `TURBO_TRACE_DEBUG_EXPORTS_CHECK=1`: answers compared with the file's
    /// text, the ones the text does not cover (unsound), and the ones that
    /// say more than the text.
    pub checked: usize,
    /// See `checked`.
    pub check_failed: usize,
    /// See `checked`.
    pub check_differs: usize,
}

impl<'a> DiskSource<'a> {
    /// Files below `root`.
    pub fn new(root: &Path) -> Self {
        DiskSource {
            root: root.to_path_buf(),
            resolve: None,
            facts: None,
            changed: std::collections::HashSet::new(),
            pure_calls: enabled("PURE_CALLS"),
            packages: true,
            shapes: Mutex::default(),
            texts: Mutex::default(),
            modules: Mutex::default(),
            symbols: enabled("SYMBOLS"),
            check: turborepo_trace_core::debug::var_os("EXPORTS_CHECK").is_some_and(|v| v == "1"),
            any_this: std::sync::OnceLock::new(),
            stats: Mutex::default(),
        }
    }

    /// Lets the source look at the definitions behind an import (is a load-time
    /// call of it only building a value? is it a primitive constant?):
    /// `resolve` maps an import of a file to the file it resolves to, and
    /// `changed` lists the changed files, whose definitions are not used
    /// for that.
    pub fn with_resolver(
        mut self,
        resolve: Resolver<'a>,
        changed: impl IntoIterator<Item = String>,
    ) -> Self {
        self.resolve = Some(resolve);
        self.changed = changed.into_iter().collect();
        self
    }

    /// `--strict`: no assumption about third-party packages. A load-time call
    /// into a package is load-time code.
    pub fn strict(mut self, on: bool) -> Self {
        self.packages = !on;
        self
    }

    /// [`known_builder`], when the table may be used; a yes is recorded as an
    /// assumption.
    fn package_builder(
        &self,
        root: &str,
        c: Callee<'_>,
        steps: &[Step],
        arg: Option<&[(String, Prop<'_>)]>,
        vercel: &dyn Fn(Callee<'_>, &[Step]) -> bool,
    ) -> bool {
        let yes = self.packages && known_builder(c, steps, arg, vercel);
        if yes {
            self.assumed(c.spec, root);
        }
        yes
    }

    /// Records that a builder of the package behind `spec` was taken to only
    /// build a value in a load-time statement of `root`.
    fn assumed(&self, spec: &str, root: &str) {
        if let Ok(mut s) = self.stats.lock() {
            if !s.assumed_builders.contains(spec) {
                s.assumed_builders.insert(spec.to_owned());
            }
            let by = s.relied.entry(format!("pkg:{spec}")).or_default();
            if !by.contains(root) {
                by.insert(root.to_owned());
            }
        }
    }

    /// Records that the definitions of the file `target` were consulted for a
    /// load-time statement of `root`.
    fn relied(&self, target: &str, root: &str) {
        if let Ok(mut s) = self.stats.lock() {
            let by = s.relied.entry(target.to_owned()).or_default();
            if !by.contains(root) {
                by.insert(root.to_owned());
            }
        }
    }

    /// Takes shapes and relations from `facts`.
    pub fn with_facts(mut self, facts: ExportFacts<'a>) -> Self {
        self.facts = Some(facts);
        self
    }

    /// Whether the module that `spec` names in the file `from` provides the
    /// export `name` at run time, as far as that can be read: an ES module
    /// of the repository that exports the name itself, re-exports it by
    /// name, or stars a module that provides it. A package is taken to
    /// provide what is asked of it, except under `--strict`; a module
    /// that is not found, or one whose exports cannot be read (CommonJS), is
    /// not.
    fn provides(&self, from: &str, spec: &str, name: &str, fuel: u8) -> bool {
        let Some(target) = self.resolve.as_ref().and_then(|r| r(from, spec)) else {
            let relative = spec.starts_with('.') || spec.starts_with('/');
            let assumed = self.packages && !relative;
            if assumed && let Ok(mut s) = self.stats.lock() {
                s.assumed_names.insert(spec.to_owned());
            }
            return assumed;
        };
        let Ok(src) = std::fs::read_to_string(self.root.join(&target)) else {
            return false;
        };
        let shape = scan_shape(src.as_bytes(), Dialect::from_path(Path::new(&target)));
        if !shape.understood || !shape.module_syntax {
            return false;
        }
        if clean_exports(&target, &src).is_some_and(|names| names.iter().any(|n| n == name)) {
            return true;
        }
        // `export *` hands on every name but `default`.
        fuel > 0
            && name != "default"
            && shape.reexports.iter().any(|r| {
                !r.type_only
                    && r.kind == ex::ReExportKind::Star
                    && self.provides(&target, r.spec, name, fuel - 1)
            })
    }

    /// Whether some file of the graph hands out a function that reads `this`.
    fn any_this(&self) -> bool {
        *self
            .any_this
            .get_or_init(|| self.facts.as_ref().is_none_or(|f| (f.any_this)()))
    }

    /// Per module whose definitions were consulted and per package whose
    /// builders were taken on trust: the files that register a test-runner
    /// mock of it, and the files that relied on it. Such a mock sees the
    /// calls that those files make while they load, which were taken to
    /// only build a value.
    ///
    /// A mock call names its module by a request; whether a request names a
    /// consulted file is decided by name, on the side of naming too much:
    /// the request's last segment is the file's name without its extension,
    /// or the directory of an index file. A request that is not a literal
    /// names every module.
    fn mocking(&self) -> Vec<(Vec<String>, Vec<String>)> {
        let relied = match self.stats.lock() {
            Ok(s) => s.relied.clone(),
            Err(_) => return Vec::new(),
        };
        if relied.is_empty() {
            return Vec::new();
        }
        let Some(facts) = &self.facts else {
            return Vec::new();
        };
        // What a request has to end with (a file) or be (a package) to name each
        // builder source: the file's name without its extension, or the
        // directory of an index file.
        let needle = |what: &str| -> String {
            match what.strip_prefix("pkg:") {
                Some(p) => p.to_owned(),
                None => {
                    let (dir, file) = what.rsplit_once('/').unwrap_or(("", what));
                    let stem = file.split('.').next().unwrap_or(file);
                    if stem == "index" {
                        dir.rsplit('/').next().unwrap_or(dir).to_owned()
                    } else {
                        stem.to_owned()
                    }
                }
            }
        };
        let needles: Vec<String> = relied.keys().map(|w| needle(w)).collect();
        let names = |request: &str, what: &str, needle: &str| {
            if what.starts_with("pkg:") {
                needle == request
                    || needle
                        .strip_prefix(request)
                        .is_some_and(|r| r.starts_with('/'))
                    || request
                        .strip_prefix(needle)
                        .is_some_and(|r| r.starts_with('/'))
            } else {
                let last = request.rsplit('/').next().unwrap_or(request);
                needle == last || needle == last.split('.').next().unwrap_or(last)
            }
        };
        let calls = (facts.mocks)(&needles);
        let mut found: Vec<(Vec<String>, Vec<String>)> = Vec::new();
        let mut why: BTreeMap<String, String> = BTreeMap::new();
        for ((what, by), needle) in relied.iter().zip(&needles) {
            let mut mockers: Vec<String> = Vec::new();
            for (path, requests) in &calls {
                let hit = requests
                    .iter()
                    .find(|r| r.as_deref().is_none_or(|r| names(r, what, needle)));
                if let Some(request) = hit {
                    mockers.push(path.clone());
                    why.entry(path.clone()).or_insert_with(|| {
                        format!(
                            "{} for {}",
                            request.as_deref().unwrap_or("a computed module name"),
                            what.strip_prefix("pkg:").unwrap_or(what)
                        )
                    });
                }
            }
            if !mockers.is_empty() {
                found.push((mockers, by.iter().cloned().collect()));
            }
        }
        if let Ok(mut s) = self.stats.lock() {
            s.mockers.extend(why);
        }
        found
    }

    /// The relation of a graph file.
    fn fact(&self, path: &str) -> Option<(NodeId, Fact<'a>)> {
        let facts = self.facts.as_ref()?;
        let n = (facts.node)(path)?;
        Some((n, Fact::parse((facts.fact)(n)?)?))
    }

    /// What this source knows about the imports of `path`.
    fn at<'s>(&'s self, path: &'s str) -> At<'s, 'a> {
        At {
            source: self,
            root: path,
            path,
            fuel: 6,
        }
    }

    /// What `ModuleSource::taint` answered so far.
    pub fn symbol_stats(&self) -> SymbolStats {
        self.stats.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// The definition-level facts of a file of the working tree: asked for the
    /// files behind load-time calls ([`At`]) and by the development check.
    fn module(&self, path: &str) -> Result<Arc<symbols::Module>, &'static str> {
        if let Some(m) = self.modules.lock().ok().and_then(|m| m.get(path).cloned()) {
            return m;
        }
        let m = if !is_js(path) {
            Err("not a JS or TS file")
        } else {
            let _t = T_FACT_MISS.enter();
            match std::fs::read_to_string(self.root.join(path)) {
                Ok(src) => symbols::Module::read(path, &src, false).map(Arc::new),
                Err(_) => Err("unreadable"),
            }
        };
        if let Ok(mut cache) = self.modules.lock() {
            cache.insert(path.to_owned(), m.clone());
        }
        m
    }

    /// The questions of `ModuleSource::taint` in the terms of the relation.
    fn asked(
        imports: &[TaintedImport],
    ) -> (Vec<symbols::Asked<'_>>, BTreeMap<String, Vec<String>>) {
        let asked = imports
            .iter()
            .map(|i| symbols::Asked {
                start: i.start,
                names: i.names.as_deref(),
                rebound: &i.rebound,
            })
            .collect();
        let members = imports
            .iter()
            .flat_map(|i| i.members.iter().cloned())
            .collect();
        (asked, members)
    }

    /// The development check: the relation's answer must cover the one computed
    /// from the file's text as it is in the working tree.
    fn check_taint(
        &self,
        path: &str,
        asked: &[symbols::Asked<'_>],
        members: &BTreeMap<String, Vec<String>>,
        oracle: &dyn Oracle,
        got: Option<&symbols::Taint>,
    ) {
        let want = self
            .module(path)
            .ok()
            .and_then(|m| m.taint_members(asked, members, oracle));
        let covers = match (&want, got) {
            (None, None) => true,
            (Some(w), Some(g)) => {
                w.exports.iter().all(|e| g.exports.contains(e))
                    && (!w.any || g.any)
                    && (w.evaluation.is_none() || g.evaluation.is_some())
            }
            // The relation may have no answer where the text has one, never the reverse.
            (Some(_), None) => true,
            (None, Some(_)) => false,
        };
        let same = match (&want, got) {
            (None, None) => true,
            (Some(w), Some(g)) => {
                w.exports == g.exports
                    && w.any == g.any
                    && w.evaluation.is_some() == g.evaluation.is_some()
            }
            _ => false,
        };
        if !covers {
            eprintln!(
                "exports check: UNSOUND {path}: text {want:?}, relation {got:?} for {asked:?} \
                 {members:?}"
            );
        } else if !same {
            eprintln!(
                "exports check: differs {path}: text {want:?}, relation {got:?} for {asked:?} \
                 {members:?}"
            );
        }
        if let Ok(mut s) = self.stats.lock() {
            s.checked += 1;
            s.check_failed += usize::from(!covers);
            s.check_differs += usize::from(covers && !same);
        }
    }

    /// Number of shapes scanned so far.
    pub fn scanned(&self) -> usize {
        self.shapes.lock().map(|m| m.len()).unwrap_or(0)
    }
}

impl<'a> ModuleSource for DiskSource<'a> {
    fn prepare(&self, files: &[&str]) {
        use rayon::prelude::*;
        // A relation takes a read, a lex and a parse of the file: a changed name with
        // many importers asks for all of theirs, one after the other, unless
        // they are worked out here on every core. One file alone is left to the
        // walk.
        let Some(facts) = self.facts.as_ref().filter(|_| files.len() >= PREPARE_MIN) else {
            return;
        };
        let mut todo: Vec<NodeId> = files
            .iter()
            .filter_map(|path| (facts.node)(path))
            .filter(|&n| !(facts.ready)(n))
            .collect();
        todo.sort_unstable();
        todo.dedup();
        if todo.len() >= PREPARE_MIN {
            todo.par_iter().for_each(|&n| {
                (facts.fact)(n);
            });
        }
    }

    fn shape(&self, path: &str) -> Option<Arc<Shape>> {
        if let Some(s) = self.shapes.lock().ok()?.get(path) {
            return s.clone();
        }
        let _t = T_SHAPE.enter();
        let s = self.fact(path).map(|(n, fact)| {
            // The relation keys an import by where it starts; its specifier is the one of
            // the edge that starts there. (The walk matches an edge by both.)
            let mut specs: Vec<(u32, String)> = Vec::new();
            if let Some(f) = &self.facts {
                (f.edges)(n, &mut |start, spec| {
                    if !specs.iter().any(|x| x.0 == start && x.1 == spec) {
                        specs.push((start, spec.to_owned()));
                    }
                });
            }
            Arc::new(shape_of(&fact, &specs))
        });
        self.shapes.lock().ok()?.insert(path.to_owned(), s.clone());
        s
    }

    fn reading<'s>(&'s self, path: &str, e: &turborepo_trace_graph::Edge<'_>) -> Reading<'s> {
        use turborepo_trace_graph::RefKind;
        let Some(spec) = e.specifier else {
            return Reading::All(None);
        };
        let _t = T_READING.enter();
        let alias = turborepo_trace_graph::barrel::aliased(spec);
        // The relation keys an import by where it starts; the edge is that import when
        // it is written as one.
        if alias.is_none()
            && (e.span.is_none()
                || !matches!(
                    e.kind(),
                    RefKind::Import | RefKind::Require | RefKind::ReExport
                ))
        {
            return Reading::All(None);
        }
        let Some((n, fact)) = self.fact(path) else {
            return Reading::All(None);
        };
        if !fact.understood() || fact.uses_all() {
            return Reading::All(None);
        }
        let lend = |n: &'a str| std::borrow::Cow::Borrowed(n);
        // What the import or re-export that starts at `start` reads.
        let at = |start: u32| -> Reading<'a> {
            // An import declaration without a namespace binding hands no namespace over:
            // only an answer with a line of its own can (a namespace import, a `require`).
            let (used, own_line) = match fact.at_line(start) {
                Some(u) => u,
                None => return Reading::All(None),
            };
            match used {
                Used::All => Reading::All(Some(start)),
                // `ns.f()` hands the namespace to `f` as `this`, and so does
                // `require('./m').f()`. A function that reads `this` then reads any export
                // of that namespace, also of a barrel it is re-exported through: when the
                // graph has such a function, an import that can hand its namespace over
                // reads everything.
                Used::Names(_) if own_line && fact.namespace_as_this(start) && self.any_this() => {
                    Reading::All(Some(start))
                }
                Used::Names(names) => Reading::Names(start, names.into_iter().map(lend).collect()),
                Used::Forward(Forward::Star) => Reading::ReExport(Forwarded::Star),
                Used::Forward(Forward::Namespace(ns)) => {
                    Reading::ReExport(Forwarded::Namespace(lend(ns)))
                }
                Used::Forward(Forward::Named(pairs)) => Reading::ReExport(Forwarded::Named(
                    pairs.into_iter().map(|(a, b)| (lend(a), lend(b))).collect(),
                )),
            }
        };
        match (alias, e.span) {
            // The edge a module mapper adds next to an import's own edge (it has no span)
            // reads what the file's import of that specifier reads, when there is exactly
            // one: one import and no re-export, or one re-export and no import.
            (Some(orig), _) => {
                let mut starts: Vec<u32> = Vec::new();
                if let Some(f) = &self.facts {
                    (f.edges)(n, &mut |start, spec| {
                        if spec == orig && !starts.contains(&start) {
                            starts.push(start);
                        }
                    });
                }
                let mut found: Vec<Reading<'a>> = starts
                    .into_iter()
                    .map(at)
                    .filter(|r| !matches!(r, Reading::All(None)))
                    .collect();
                match (found.pop(), found.is_empty()) {
                    (Some(only), true) => only,
                    _ => Reading::All(None),
                }
            }
            (None, Some(span)) => at(span.start),
            (None, None) => Reading::All(None),
        }
    }

    fn mockers(&self) -> Vec<(Vec<String>, Vec<String>)> {
        self.mocking()
    }

    fn pure_exports(&self, path: &str) -> Option<Vec<String>> {
        let (_, fact) = self.fact(path)?;
        (fact.understood())
            .then(|| fact.pure_exports())
            .flatten()
            .map(|v| v.into_iter().map(str::to_owned).collect())
    }

    fn taint(&self, path: &str, imports: &[TaintedImport]) -> Option<Taint> {
        if !self.symbols {
            return None;
        }
        let _t = T_TAINT.enter();
        let count = |map: &mut BTreeMap<String, (usize, String)>, why: &str| {
            let e = map
                .entry(why.to_owned())
                .or_insert_with(|| (0, path.to_owned()));
            e.0 += 1;
        };
        let unknown = |why: &str| {
            if let Ok(mut s) = self.stats.lock() {
                count(&mut s.unknown, why);
                s.unknown_files.insert(path.to_owned(), why.to_owned());
            }
        };
        let (asked, members) = Self::asked(imports);
        let Some((_, fact)) = self.fact(path) else {
            unknown("no relation of the file");
            return None;
        };
        if let Some(why) = fact.no_rows() {
            unknown(why);
            return None;
        }
        let t = fact.taint(&asked, &members, &self.at(path));
        if self.check {
            self.check_taint(path, &asked, &members, &self.at(path), t.as_ref());
        }
        let Some(t) = t else {
            unknown("not an import declaration");
            return None;
        };
        if let Ok(mut s) = self.stats.lock() {
            s.pure_calls += t.pure_calls;
            match &t.evaluation {
                Some(why) => {
                    if s.load_time_readers
                        .insert(path.to_owned(), why.clone())
                        .is_none()
                    {
                        count(&mut s.evaluated, why);
                    }
                }
                None => s.passed += 1,
            }
        }
        Some(Taint {
            evaluation: t.evaluation.is_some(),
            exports: t.exports,
        })
    }

    fn reads(&self, path: &str, imports: &[TaintedImport]) -> Option<bool> {
        if !self.symbols {
            return None;
        }
        let _t = T_READS.enter();
        let (_, fact) = self.fact(path)?;
        let (asked, members) = Self::asked(imports);
        let t = fact.taint(&asked, &members, &Alone);
        if self.check {
            self.check_taint(path, &asked, &members, &Alone, t.as_ref());
        }
        Some(t?.any)
    }

    fn text(&self, path: &str) -> Option<Arc<str>> {
        if let Some(t) = self.texts.lock().ok()?.get(path) {
            return t.clone();
        }
        let t = std::fs::read_to_string(self.root.join(path))
            .ok()
            .map(Arc::<str>::from);
        self.texts.lock().ok()?.insert(path.to_owned(), t.clone());
        t
    }
}

/// The query-side shape of a file from its relation; `specs`: the starts and
/// specifiers of the file's import, `require` and re-export edges. An import
/// has one entry per specifier of an edge that starts where it does (one, but
/// for the alternatives of a computed `require`, which the relation does not
/// list), and none without such an edge: it is never asked about.
fn shape_of(fact: &Fact<'_>, specs: &[(u32, String)]) -> Shape {
    let at = |start: u32| {
        specs
            .iter()
            .filter(move |s| s.0 == start)
            .map(|s| s.1.clone())
    };
    let names = |v: &[&str]| v.iter().map(|n| (*n).to_owned()).collect::<Vec<String>>();
    let mut uses = Vec::new();
    for (start, used) in fact.uses() {
        for spec in at(start) {
            uses.push(Use {
                start,
                spec,
                names: used.as_deref().map(names),
            });
        }
    }
    let mut reexports = Vec::new();
    for (start, kind) in fact.reexports() {
        for spec in at(start) {
            reexports.push(ReExport {
                start,
                spec,
                kind: match &kind {
                    Forward::Star => ReExportKind::Star,
                    Forward::Namespace(n) => ReExportKind::Namespace((*n).to_owned()),
                    Forward::Named(p) => ReExportKind::Named(
                        p.iter()
                            .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
                            .collect(),
                    ),
                },
            });
        }
    }
    Shape {
        understood: fact.understood(),
        uses_all: fact.uses_all(),
        pure_exports: fact.pure_exports().map(|v| names(&v)),
        uses,
        reexports,
    }
}

/// Whether `src` parses as `path` without syntax or early errors.
#[cfg(test)]
fn parses_cleanly(path: &str, src: &str) -> bool {
    clean_exports(path, src).is_some()
}

/// The names `src` exports at run time (types left out; `default` for the
/// default export; sorted) when it parses as `path` without syntax or early
/// errors, else `None`.
fn clean_exports(path: &str, src: &str) -> Option<Vec<String>> {
    let Ok(st) = oxc_span::SourceType::from_path(path) else {
        return None;
    };
    let alloc = oxc_allocator::Allocator::default();
    let ret = oxc_parser::Parser::new(&alloc, src, st).parse();
    if ret.fatal_error || !ret.diagnostics.is_empty() || ret.is_flow_language {
        return None;
    }
    // oxc skips the duplicate-export check for TypeScript (`export type A` next to
    // `export const A` is fine there); transpilers reject duplicate value
    // exports. Any duplicate non-type export name (TS overloads included,
    // conservatively) disqualifies.
    let r = &ret.module_record;
    let mut seen = std::collections::HashSet::new();
    for e in r
        .local_export_entries
        .iter()
        .chain(r.indirect_export_entries.iter())
        .chain(r.star_export_entries.iter())
    {
        if e.is_type {
            continue;
        }
        let name = if e.export_name.is_default() {
            "default"
        } else if let Some(sp) = e.export_name.span() {
            src.get(sp.start as usize..sp.end as usize)
                .unwrap_or("")
                .trim_matches(|c| c == '"' || c == '\'')
        } else {
            continue;
        };
        if !seen.insert(name) {
            return None;
        }
    }
    let sem = oxc_semantic::SemanticBuilder::new()
        .with_check_syntax_error(true)
        .build(&ret.program);
    if !sem.diagnostics.is_empty() || symbols::import_redeclared(sem.semantic.scoping()) {
        return None;
    }
    let mut names: Vec<String> = seen.into_iter().map(str::to_owned).collect();
    names.sort_unstable();
    Some(names)
}

/// Why a version of a modified file is not skippable, or its shape facts
/// (`side_effect_free` says whether its top level runs code) and what linking
/// asks of it and of the modules it imports ([`links`]).
fn check_version<'s>(path: &str, src: &'s str) -> Result<(ex::Shape<'s>, Links), &'static str> {
    let s = scan_shape(src.as_bytes(), Dialect::from_path(Path::new(path)));
    if !s.understood {
        return Err("not understood");
    }
    if !s.module_syntax {
        return Err("not an ES module");
    }
    if s.exports_open {
        return Err("exports not fully known");
    }
    let Some(exports) = clean_exports(path, src) else {
        return Err("parse errors");
    };
    let links = links(&s, exports);
    Ok((s, links))
}

/// The outcome of [`candidates`].
#[derive(Default)]
pub struct CandidateCheck {
    /// Skippable modified files.
    pub candidates: Vec<Candidate>,
    /// Modified JS graph files that are not, with the reason.
    pub rejected: Vec<(String, String)>,
    /// For rejected files whose load-time code differs: what differs.
    pub load_time: BTreeMap<String, String>,
    /// Erased-equal modified files (`crate::erase`): same runtime JavaScript at
    /// the base revision and in the working tree.
    pub erased: Vec<String>,
    /// Modified JS graph files that are not erased-equal, with the reason.
    pub not_erased: Vec<(String, String)>,
}

/// Why a modified file is not a candidate, and for a file whose load-time code
/// differs between the revisions, what differs.
struct Rejected {
    reason: String,
    load_time: Option<String>,
}

impl From<String> for Rejected {
    fn from(reason: String) -> Self {
        Rejected {
            reason,
            load_time: None,
        }
    }
}

/// What [`check_file`] found for one modified file.
enum Checked {
    /// Erased-equal.
    Erased,
    /// Not erased-equal (the reason, when that rule is on), then the barrel
    /// check.
    Other(Option<String>, Option<Result<Candidate, Rejected>>),
}

/// Checks one modified file: erased-equality of the base revision and the
/// working tree (`erase`), then whether it is a barrel candidate (`barrels`).
fn check_file(
    root: &Path,
    path: &str,
    git: &Git,
    erase: bool,
    barrels: bool,
    source: Option<&DiskSource<'_>>,
    cyclic: bool,
) -> Checked {
    let _t = T_CANDS.enter();
    let Ok(head) = std::fs::read_to_string(root.join(path)) else {
        let why = "unreadable".to_owned();
        return Checked::Other(
            erase.then(|| why.clone()),
            barrels.then_some(Err(why.into())),
        );
    };
    let mut base: Option<Option<String>> = None;
    let mut why_not = None;
    if erase {
        let b = base.insert(git.show(path));
        match b {
            Some(b) => match crate::erase::erased_equal(path, b, &head) {
                Ok(()) => return Checked::Erased,
                Err(e) => why_not = Some(e),
            },
            None => why_not = Some("not at the base revision".to_owned()),
        }
    }
    let candidate = barrels.then(|| barrel_candidate(path, &head, base, git, source, cyclic));
    Checked::Other(why_not, candidate)
}

/// [`links`]: the names a module exports at run time, and what it asks of others.
type Links = (Vec<String>, Vec<String>);

/// What linking asks of a module and of the modules it imports: the names it
/// exports at run time (`exports`: types are erased before anything is linked),
/// the names it imports by name from each module, and what it re-exports. Where
/// modules are linked before they run (ES modules in Node and in a browser, a
/// bundler that checks that exports are there), a name that is asked for and is
/// not there is an error for every file that loads the module, whatever it
/// uses: `export { b } from './b'` in a barrel breaks all importers of the
/// barrel when `./b` stops exporting `b`. A file is only skipped when
/// none of that changed. A namespace import asks for no name, a type-only
/// import or re-export is erased, and a `require` is no link.
fn links(s: &ex::Shape<'_>, exports: Vec<String>) -> Links {
    let mut asked: Vec<String> = Vec::new();
    for u in &s.uses {
        if let (Names::Some(names), false, false) = (&u.names, u.namespace, u.type_only) {
            asked.extend(names.iter().map(|n| format!("{} {n}", u.spec)));
        }
    }
    for r in s.reexports.iter().filter(|r| !r.type_only) {
        asked.push(format!("{} {:?}", r.spec, r.kind));
    }
    asked.sort_unstable();
    (exports, asked)
}

/// The names a module asks of other modules by name, as `(specifier, name)`:
/// named and default imports that are not type-only, and named re-exports. A
/// namespace import and `export *` ask for no name, a `require` is no link.
fn asked(s: &ex::Shape<'_>) -> BTreeSet<(String, String)> {
    let mut out = BTreeSet::new();
    for u in &s.uses {
        if let (Names::Some(names), false, false) = (&u.names, u.namespace, u.type_only) {
            out.extend(names.iter().map(|n| (u.spec.to_owned(), (*n).to_owned())));
        }
    }
    for r in s.reexports.iter().filter(|r| !r.type_only) {
        if let ex::ReExportKind::Named(pairs) = &r.kind {
            out.extend(
                pairs
                    .iter()
                    .map(|(from, _)| (r.spec.to_owned(), (*from).to_owned())),
            );
        }
    }
    out
}

/// Why the head version of the modified file `path` may fail to link where
/// modules are linked before they run: a name it newly asks of another module
/// (a named import, a named re-export, the same name from another module) that
/// the module does not provide. `None`: every new name is there.
///
/// What the file itself no longer exports needs no check here: such a name is
/// rebound, and the walk ends at every import and re-export that names it. An
/// export that is new breaks nothing.
fn unlinked(
    path: &str,
    head: &ex::Shape<'_>,
    base: &ex::Shape<'_>,
    source: Option<&DiskSource<'_>>,
) -> Option<String> {
    let before = asked(base);
    for (spec, name) in asked(head) {
        if before.contains(&(spec.clone(), name.clone())) {
            continue;
        }
        if !source.is_some_and(|s| s.provides(path, &spec, &name, 4)) {
            return Some(format!(
                "it newly asks for {name} of {spec:?}, which may not be there when modules are \
                 linked"
            ));
        }
    }
    None
}

/// The barrel check of one modified file: the working tree first (no `git`
/// process for files that fail there, unless `base` was read already), then the
/// base revision.
fn barrel_candidate(
    path: &str,
    head: &str,
    base: Option<Option<String>>,
    git: &Git,
    source: Option<&DiskSource<'_>>,
    cyclic: bool,
) -> Result<Candidate, Rejected> {
    let (h, h_links) = check_version(path, head).map_err(|e| format!("head: {e}"))?;
    let base = base
        .unwrap_or_else(|| git.show(path))
        .ok_or_else(|| "not at the base revision".to_owned())?;
    let (b, b_links) = check_version(path, &base).map_err(|e| format!("base: {e}"))?;
    if h.executes != b.executes {
        return Err("module loads differ between base and head"
            .to_owned()
            .into());
    }
    // Only the exports whose definition changed, when the file does the same at
    // load time at both revisions (`symbols::changed`).
    // Why a version has no definition-level facts, reported
    // with the load-time changes (`pruning.load_time_changes` of `--format json`).
    let mut no_facts: Option<String> = None;
    let diff = enabled("EXPORT_DIFF")
        .then(|| {
            // The same oracle for both revisions: imports resolve as in the working tree,
            // and only unchanged files are consulted.
            let at = source.map(|s| s.at(path));
            let oracle: &dyn Oracle = match &at {
                Some(at) => at,
                None => &Alone,
            };
            match symbols::changed(path, &base, head, oracle, oracle, cyclic) {
                Ok(c) => Some(c),
                Err(why) => {
                    no_facts = Some(format!("no facts: {why}"));
                    None
                }
            }
        })
        .flatten();
    let all: BTreeSet<String> = h
        .exports
        .iter()
        .chain(&b.exports)
        .map(|s| (*s).to_owned())
        .collect();
    let pure = h.side_effect_free && b.side_effect_free;
    let load_time = no_facts;
    // A file whose definitions cannot be compared is not narrowed, and it is not
    // left to barrel pruning's own test either: that test does not see
    // everything a module does when it is loaded (an import that a transpiler
    // drops, a decorator on a parameter).
    if load_time.is_some() {
        return Err(Rejected {
            reason: "its definitions cannot be compared".to_owned(),
            load_time,
        });
    }
    if let Some(c) = diff {
        if !c.evaluation {
            // Linking: an export that is gone is a rebound name, which the walk ends at for
            // every import and re-export that names it; a name the head newly asks of
            // another module has to be there.
            if let Some(why) = unlinked(path, &h, &b, source) {
                return Err(Rejected {
                    reason: why,
                    load_time: None,
                });
            }
            // Every export of a side-effect-free file changed: the plain barrel candidate.
            let narrowed = !pure || !all.iter().eq(c.exports.iter());
            return Ok(Candidate {
                path: path.to_owned(),
                exports: c.exports,
                narrowed,
                members: c.members.into_iter().collect(),
                rebound: c.rebound,
                reordered: c.reordered,
            });
        }
        // The comparison is the authority where it has an answer: what the file does
        // when it is loaded differs. The check below would let some of that
        // through (an import whose last use as a value went away is no longer
        // loaded; a decorator on a parameter runs), so the file is not a
        // candidate whatever that check says.
        let reason = if !h.side_effect_free {
            "head: top-level side effects"
        } else if !b.side_effect_free {
            "base: top-level side effects"
        } else {
            "what the file does when it is loaded differs between base and head"
        };
        return Err(Rejected {
            reason: reason.to_owned(),
            load_time: c.load_time,
        });
    }
    if !h.side_effect_free {
        return Err(Rejected {
            reason: "head: top-level side effects".to_owned(),
            load_time,
        });
    }
    if !b.side_effect_free {
        return Err(Rejected {
            reason: "base: top-level side effects".to_owned(),
            load_time,
        });
    }
    // Without the comparison nothing says which names changed: the file must export
    // and ask for the same names at both revisions ([`links`]).
    if h_links != b_links {
        return Err(Rejected {
            reason: "exported or imported names differ between base and head".to_owned(),
            load_time,
        });
    }
    // Without definition-level facts the names the file re-exports are not
    // compared: a name that a re-export gains, loses or takes from another
    // module changes for its importers without being one of the file's own
    // exports.
    let same_reexports = h.reexports.len() == b.reexports.len()
        && h.reexports
            .iter()
            .zip(&b.reexports)
            .all(|(x, y)| x.spec == y.spec && x.kind == y.kind && x.type_only == y.type_only);
    if !same_reexports {
        return Err(Rejected {
            reason: "the re-exports differ between base and head".to_owned(),
            load_time,
        });
    }
    // Every export counts as changed. Without definition-level facts the names that
    // exist at one revision only are known to be rebound; on an import cycle
    // any name may be.
    let exports: Vec<String> = all.into_iter().collect();
    let rebound = if cyclic {
        exports.clone()
    } else {
        exports
            .iter()
            .filter(|n| h.exports.contains(&n.as_str()) != b.exports.contains(&n.as_str()))
            .cloned()
            .collect()
    };
    Ok(Candidate {
        path: path.to_owned(),
        rebound,
        exports,
        narrowed: false,
        members: Vec::new(),
        reordered: true,
    })
}

/// Checks the modified JS files of `changes` at the base revision and in the
/// working tree (in parallel): erased-equal files (`erase`) and barrel
/// candidates (`barrels`).
pub fn candidates<G: GraphView>(
    g: &G,
    root: &Path,
    changes: &[Change],
    git: &Git,
    erase: bool,
    barrels: bool,
    source: Option<&DiskSource<'_>>,
) -> CandidateCheck {
    let paths: Vec<&str> = changes
        .iter()
        .filter(|c| c.kind == ChangeKind::Modified && is_js(&c.path))
        .filter(|c| g.node_by_path(&c.path).is_some())
        .map(|c| c.path.as_str())
        .collect();
    // Modules on an import cycle are compared more strictly (`crate::cycles`).
    let nodes: Vec<turborepo_trace_graph::NodeId> =
        paths.iter().filter_map(|p| g.node_by_path(p)).collect();
    let cyclic = if barrels && nodes.len() == paths.len() {
        crate::cycles::on_cycle(g, &nodes)
    } else {
        vec![true; paths.len()]
    };
    // For measurements: how many modified files are on an import cycle
    // (`TURBO_TRACE_DEBUG_CYCLE_STATS=1`, stderr).
    if turborepo_trace_core::debug::var_os("CYCLE_STATS").is_some() {
        eprintln!(
            "cycles: {} of {} modified JS file(s) are on an import cycle",
            cyclic.iter().filter(|c| **c).count(),
            cyclic.len()
        );
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results: Mutex<Vec<(usize, Checked)>> = Mutex::default();
    let workers = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .min(8)
        .min(paths.len());
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(&p) = paths.get(i) else { break };
                    let r = check_file(root, p, git, erase, barrels, source, cyclic[i]);
                    if let Ok(mut v) = results.lock() {
                        v.push((i, r));
                    }
                }
            });
        }
    });
    let mut results = results.into_inner().unwrap_or_default();
    results.sort_unstable_by_key(|r| r.0);
    let mut out = CandidateCheck::default();
    for (i, r) in results {
        let path = paths[i].to_owned();
        match r {
            Checked::Erased => out.erased.push(path),
            Checked::Other(why_not, candidate) => {
                if let Some(why) = why_not {
                    out.not_erased.push((path.clone(), why));
                }
                match candidate {
                    Some(Ok(c)) => out.candidates.push(c),
                    Some(Err(why)) => {
                        if let Some(what) = why.load_time {
                            out.load_time.insert(path.clone(), what);
                        }
                        out.rejected.push((path, why.reason));
                    }
                    None => {}
                }
            }
        }
    }
    out
}

/// See [`GraphFacts::mocks`].
pub type MockCalls<'a> = dyn Fn(&[String]) -> Vec<(NodeId, Vec<Option<String>>)> + Sync + 'a;

/// What [`evaluate`] reads from the store's graph: the per-file facts that were
/// traced with each file. With it, changed exports is on: the relations of the
/// files come from the working tree ([`Relations`]).
#[derive(Clone, Copy)]
pub struct GraphFacts<'a> {
    /// The files that have test-runner mock calls ([`Relations::any_this`]
    /// looks at them first).
    pub mock_files: &'a (dyn Fn() -> Vec<NodeId> + Sync),
    /// The files with test-runner mock calls and the module names they hand to
    /// them ([`ExportFacts::mocks`]).
    pub mocks: &'a MockCalls<'a>,
}

/// `affected` with barrel pruning, plus what the pruning did.
pub struct Outcome {
    /// The report.
    pub report: turborepo_trace_graph::AffectedReport,
    /// Pruning details.
    pub pruned: turborepo_trace_graph::barrel::PruneReport,
    /// Candidates checked.
    pub candidates: Vec<Candidate>,
    /// Modified JS files that are not candidates, with the reason.
    pub rejected: Vec<(String, String)>,
    /// For rejected files whose load-time code differs between the revisions:
    /// what differs.
    pub load_time: BTreeMap<String, String>,
    /// Erased-equal modified files.
    pub erased: Vec<String>,
    /// Modified JS files that are not erased-equal, with the reason.
    pub not_erased: Vec<(String, String)>,
    /// Module shapes scanned.
    pub scanned: usize,
    /// What the definition-level facts of importers did.
    pub symbols: SymbolStats,
    /// Time spent on candidate checks and pruning.
    pub elapsed: std::time::Duration,
}

/// Whether the rule switched by the debug variable `TURBO_TRACE_DEBUG_<name>`
/// is on (`0` turns it off: A/B measurements, escape hatch).
fn enabled(name: &str) -> bool {
    let on = |name: &str| turborepo_trace_core::debug::var_os(name).is_none_or(|v| v != "0");
    // `TURBO_TRACE_DEBUG_EXPORTS=0` switches every part of changed exports off.
    let part = matches!(name, "EXPORT_DIFF" | "SYMBOLS" | "PURE_CALLS");
    on(name) && (!part || on("EXPORTS"))
}

/// Evaluates `affected` with the pruning rules of
/// `turborepo_trace_graph::barrel`:
///
/// - with a base revision, modified files that are side-effect free and
///   re-exported through barrels are skipped for entries that don't use their
///   exports (`TURBO_TRACE_DEBUG_BARRELS=0` turns it off);
/// - with a base revision, modified files whose runtime JavaScript did not
///   change are skipped for tests whose runner transpiles file by file
///   (`TURBO_TRACE_DEBUG_ERASE=0`);
/// - those tests are evaluated without type-only edges
///   (`TURBO_TRACE_DEBUG_TYPE_EDGES=0`);
/// - and are not selected through modules they replace with a mock
///   (`TURBO_TRACE_DEBUG_MOCKS=0`).
#[allow(clippy::too_many_arguments)]
pub fn evaluate<'a, G: GraphView + Sync>(
    engine: &'a turborepo_trace_graph::Engine<'_, G>,
    root: &Path,
    changes: &turborepo_trace_graph::ChangeSet,
    entries: &turborepo_trace_graph::NodeSet,
    opts: &turborepo_trace_graph::AffectedOptions,
    git: Option<&Git>,
    isolated: Option<&turborepo_trace_graph::NodeSet>,
    facts: Option<GraphFacts<'a>>,
) -> Outcome {
    let t0 = std::time::Instant::now();
    let barrels_on = enabled("BARRELS");
    let erase_on = enabled("ERASE");
    let runtime_entries = enabled("TYPE_EDGES");
    let mocks = enabled("MOCKS");
    let g = engine.graph();
    let relations = Relations::new(g, root);
    let disk = DiskSource::new(root).strict(opts.strict).with_resolver(
        Box::new(move |from, spec| {
            let n = g.node_by_path(from)?;
            let to = g
                .out_edges(n)
                .find(|e| e.specifier == Some(spec) && e.to.is_some())?
                .to?;
            (!g.is_external(to)).then(|| g.path(to).to_owned())
        }),
        changes.files.iter().map(|c| c.path.clone()),
    );
    let disk = match facts {
        Some(facts) => disk.with_facts(ExportFacts {
            node: Box::new(move |path| g.node_by_path(path)),
            fact: Box::new(|n| relations.get(n)),
            ready: Box::new(|n| relations.ready(n)),
            any_this: Box::new(|| relations.any_this(&(facts.mock_files)())),
            mocks: Box::new(move |needles| {
                (facts.mocks)(needles)
                    .into_iter()
                    .map(|(n, requests)| (g.path(n).to_owned(), requests))
                    .collect()
            }),
            edges: Box::new(move |n, f| {
                use turborepo_trace_graph::RefKind;
                for e in g.out_edges(n) {
                    if let (Some(span), Some(spec)) = (e.span, e.specifier)
                        && matches!(
                            e.kind(),
                            RefKind::Import | RefKind::Require | RefKind::ReExport
                        )
                    {
                        f(span.start, spec);
                    }
                }
            }),
        }),
        None => disk,
    };
    let check = git
        .filter(|_| barrels_on || erase_on)
        .map(|git| {
            candidates(
                engine.graph(),
                root,
                &changes.files,
                git,
                erase_on,
                barrels_on,
                Some(&disk),
            )
        })
        .unwrap_or_default();
    turborepo_trace_core::timing::mark(
        "affected: barrel and erase candidates (base versions, parse)",
    );
    let input = turborepo_trace_graph::barrel::Barrels {
        candidates: check.candidates.clone(),
        erased: check.erased.clone(),
        runtime_entries,
        mocks,
        source: &disk,
        isolated,
    };
    let used = runtime_entries || mocks || !check.candidates.is_empty() || !check.erased.is_empty();
    let (report, pruned) = engine.affected_with(changes, entries, opts, used.then_some(&input));
    if disk.check {
        let s = disk.symbol_stats();
        eprintln!(
            "exports checked: {} answer(s) of the relation against the files' text, {} not \
             covered by it, {} that say more",
            s.checked, s.check_failed, s.check_differs
        );
    }
    Outcome {
        report,
        pruned,
        candidates: check.candidates,
        rejected: check.rejected,
        load_time: check.load_time,
        erased: check.erased,
        not_erased: check.not_erased,
        scanned: disk.scanned(),
        symbols: disk.symbol_stats(),
        elapsed: t0.elapsed(),
    }
}

/// What `--explain` says about changed exports beyond the walk's own record
/// (`turborepo_trace_graph::barrel::Reader`).
#[derive(Default)]
pub struct ExportNotes {
    /// Per file that ended a walk: the load-time statement that reads the
    /// change, or why the file has no definition-level facts.
    pub readers: BTreeMap<String, String>,
    /// Per modified JS file that is tracked as a whole: why its change is not
    /// narrowed to exports.
    pub whole: BTreeMap<String, String>,
    /// The third-party packages whose builders were taken to only build a
    /// value.
    pub assumed_builders: Vec<String>,
    /// The packages that were taken to export the names newly imported from
    /// them.
    pub assumed_names: Vec<String>,
}

impl ExportNotes {
    /// Writes what the rule assumed for an entry that a file tracked by its
    /// exports did not select: one sentence, for `--explain`. Nothing when
    /// no such file was skipped for the entry.
    pub fn write_assumption<G: GraphView>(
        &self,
        engine: &turborepo_trace_graph::Engine<'_, G>,
        pruned: &turborepo_trace_graph::barrel::PruneReport,
        entry: NodeId,
        out: &mut dyn std::io::Write,
    ) -> std::io::Result<()> {
        if !pruned
            .reasons(engine, entry)
            .iter()
            .any(|r| matches!(r, turborepo_trace_graph::barrel::Reason::Unused(..)))
        {
            return Ok(());
        }
        let builders = if self.assumed_builders.is_empty() {
            String::new()
        } else {
            format!(
                ", and that the builders of {} only build a value from their arguments when a \
                 module calls them while it loads (--strict does not assume that)",
                self.assumed_builders.join(", ")
            )
        };
        let names = if self.assumed_names.is_empty() {
            String::new()
        } else {
            format!(
                "; that {} export the names a changed file newly imports from them (--strict does \
                 not assume that)",
                self.assumed_names.join(", ")
            )
        };
        writeln!(
            out,
            "  assumed: both revisions of a changed file transpile (syntax and early errors are \
             checked){builders}{names}"
        )
    }

    /// Writes the lines of `--explain` for an entry that is affected through
    /// `witness`: which exports of the changed file changed and which file
    /// reads one of them, or why the changed file counts as a whole.
    pub fn write<G: GraphView>(
        &self,
        g: &G,
        pruned: &turborepo_trace_graph::barrel::PruneReport,
        witness: &[NodeId],
        out: &mut dyn std::io::Write,
    ) -> std::io::Result<()> {
        pruned.write_reader(
            g,
            witness,
            &|r| self.readers.get(g.path(r.node)).cloned(),
            out,
        )?;
        if let Some(first) = witness.first()
            && let Some(why) = self.whole.get(g.path(*first))
        {
            writeln!(
                out,
                "  {} counts as changed as a whole: {why}",
                g.path(*first)
            )?;
        }
        Ok(())
    }
}

impl Outcome {
    /// What `--explain` says about changed exports.
    pub fn export_notes(&self) -> ExportNotes {
        let mut notes = ExportNotes {
            assumed_builders: self.symbols.assumed_builders.iter().cloned().collect(),
            assumed_names: self.symbols.assumed_names.iter().cloned().collect(),
            ..ExportNotes::default()
        };
        for (path, why) in &self.symbols.unknown_files {
            notes.readers.insert(path.clone(), why.clone());
        }
        for (path, why) in &self.symbols.load_time_readers {
            notes.readers.insert(path.clone(), why.clone());
        }
        for (path, why) in &self.symbols.mockers {
            notes
                .readers
                .entry(path.clone())
                .or_insert_with(|| why.clone());
        }
        for (path, reason) in &self.rejected {
            let why = match self.load_time.get(path) {
                Some(what) if what.starts_with("no facts: ") => format!(
                    "{reason}, and its definitions cannot be compared ({})",
                    what.trim_start_matches("no facts: ")
                ),
                Some(what) => format!(
                    "what it does when it is loaded differs between the base and the head ({what})"
                ),
                None => reason.clone(),
            };
            notes.whole.insert(path.clone(), why);
        }
        notes
    }

    /// The JSON `pruning` object of `affected --format json`: erased-equal
    /// files and the per-entry edge modes.
    pub fn pruning_json<G: GraphView>(&self, g: &G) -> serde_json::Value {
        let p = &self.pruned;
        serde_json::json!({
            "erased": self.erased,
            "not_erased": self
                .not_erased
                .iter()
                .map(|(p, why)| serde_json::json!({ "path": p, "reason": why }))
                .collect::<Vec<_>>(),
            "erased_skipped": p
                .erased
                .iter()
                .map(|s| serde_json::json!({
                    "path": g.path(s.node),
                    "other_readers": s.relevant,
                    "entries_skipped": s.entries,
                }))
                .collect::<Vec<_>>(),
            "runtime_only_entries": p.runtime_entries.as_ref().map_or(0, |s| s.len()),
            "type_only_skipped": p.type_only.iter().map(|&n| g.path(n)).collect::<Vec<_>>(),
            "mock_skipped": p
                .mocked
                .iter()
                .map(|m| serde_json::json!({
                    "entry": g.path(m.entry),
                    "module": g.path(m.module),
                    "loaded_by": g.path(m.importer),
                    "mock": m.at,
                }))
                .collect::<Vec<_>>(),
            "mock_unregistered": p
                .unregistered
                .iter()
                .map(|u| serde_json::json!({
                    "entry": g.path(u.entry),
                    "mock": g.path(u.mock),
                    "module": g.path(u.module),
                    "registered_by": g.path(u.by),
                }))
                .collect::<Vec<_>>(),
            "load_time_changes": self
                .load_time
                .iter()
                .map(|(path, what)| serde_json::json!({ "path": path, "what": what }))
                .collect::<Vec<_>>(),
            "changed_exports": p
                .skipped
                .iter()
                .filter_map(|s| s.changed.as_ref().map(|names| serde_json::json!({
                    "path": g.path(s.node),
                    "names": names,
                    "passed_on_by": s.passed,
                    "relevant_importers": s.relevant,
                    "entries_skipped": s.entries,
                })))
                .collect::<Vec<_>>(),
            "importers": {
                "passed_on": self.symbols.passed,
                "pure_load_time_calls": self.symbols.pure_calls,
                "read_at_load_time": reasons(&self.symbols.evaluated),
                "load_time_readers": self
                    .symbols
                    .load_time_readers
                    .iter()
                    .map(|(path, why)| serde_json::json!({ "path": path, "reason": why }))
                    .collect::<Vec<_>>(),
                "without_facts": reasons(&self.symbols.unknown),
            },
            "builders": {
                "relied_on": self
                    .symbols
                    .relied
                    .iter()
                    .map(|(what, by)| serde_json::json!({
                        "builder": what.strip_prefix("pkg:").unwrap_or(what),
                        "package": what.starts_with("pkg:"),
                        "files": by.len(),
                    }))
                    .collect::<Vec<_>>(),
                "mocked_in": self
                    .symbols
                    .mockers
                    .iter()
                    .map(|(path, why)| serde_json::json!({ "path": path, "mock": why }))
                    .collect::<Vec<_>>(),
            },
        })
    }

    /// The stderr lines of `--stats` for erased-equal files, edge modes and
    /// export-level tracking.
    pub fn stats_line(&self) -> String {
        let p = &self.pruned;
        let narrowed = self.candidates.iter().filter(|c| c.narrowed).count();
        let sum = |m: &BTreeMap<String, (usize, String)>| m.values().map(|v| v.0).sum::<usize>();
        format!(
            "pruning: {} erased-equal file(s) ({} modified JS file(s) not), {} entr(ies) skipped; \
             {} entr(ies) without type-only edges, {} of them reached only through type-only \
             edges; {} entr(ies) reached only through modules they replace with a mock ({} \
             replace a module: kept by a hazard of the test or its project {}, change elsewhere \
             {}, a hazard in a file it loads {}, real loads {}, runner {}, loaded before the \
             mocks {}, another import path {}); {} entr(ies) reached only through manual mocks \
             that no file they load registers\nexports: {} of {} candidate(s) tracked by their \
             changed exports only ({} changed name(s)); importers: {} passed a change on through \
             their exports ({} load-time call(s) that only build a value), {} read it at load \
             time, {} without facts",
            self.erased.len(),
            self.not_erased.len(),
            p.erased.iter().map(|s| s.entries).sum::<usize>(),
            p.runtime_entries.as_ref().map_or(0, |s| s.len()),
            p.type_only.len(),
            p.mocked.len(),
            p.mock_stats.entries,
            p.mock_stats.own_hazard,
            p.mock_stats.unrelated,
            p.mock_stats.hazard,
            p.mock_stats.real,
            p.mock_stats.runner,
            p.mock_stats.before,
            p.mock_stats.other_path,
            p.unregistered.len(),
            narrowed,
            self.candidates.len(),
            self.candidates
                .iter()
                .filter(|c| c.narrowed)
                .map(|c| c.exports.len())
                .sum::<usize>(),
            self.symbols.passed,
            self.symbols.pure_calls,
            sum(&self.symbols.evaluated),
            sum(&self.symbols.unknown),
        )
    }
}

/// Reasons with their counts and examples, most frequent first.
fn reasons(m: &BTreeMap<String, (usize, String)>) -> Vec<serde_json::Value> {
    let mut v: Vec<(&String, &(usize, String))> = m.iter().collect();
    v.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(b.0)));
    v.into_iter()
        .map(|(why, (n, example))| serde_json::json!({ "reason": why, "files": n, "example": example }))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_parses() {
        assert!(parses_cleanly("a.ts", "export const a = 1;\n"));
        assert!(!parses_cleanly("a.ts", "export const a = ;\n"));
        // Early errors inside function bodies (the side-effect check skips bodies).
        assert!(!parses_cleanly(
            "a.ts",
            "export function f() { let x = 1; let x = 2; }\n"
        ));
        assert!(!parses_cleanly(
            "a.ts",
            "export const a = 1;\nexport { a };\n"
        ));
        assert!(!parses_cleanly(
            "a.ts",
            "export function f() { return 1 +; }\n"
        ));
        assert!(parses_cleanly(
            "a.tsx",
            "export const C = () => <div>{1}</div>;\n"
        ));
    }

    #[test]
    fn version_checks() {
        assert!(check_version("a.ts", "export const a = 1;\n").is_ok());
        assert!(
            !check_version("a.ts", "export const a = f();\n")
                .expect("shape")
                .0
                .side_effect_free
        );
        assert_eq!(
            check_version("a.ts", "const a = 1;\n").err(),
            Some("not an ES module")
        );
        assert_eq!(
            check_version("a.ts", "export * from './b';\n").err(),
            Some("exports not fully known")
        );
        assert_eq!(
            check_version("a.ts", "export function f() { let x; let x; }\n").err(),
            Some("parse errors")
        );
    }

    #[test]
    fn what_linking_asks_for() {
        let same = |a: &str, b: &str| {
            let of = |src: &str| check_version("a.ts", src).expect("skippable").1;
            of(a) == of(b)
        };
        // A body, a namespace member, a type-only import and a `require` ask for
        // nothing new.
        assert!(same(
            "import { x } from './x';\nexport function f() { return 1; }\n",
            "import { x } from './x';\nexport function f() { return x; }\n"
        ));
        assert!(same(
            "import * as ns from './x';\nexport const f = () => ns.a;\n",
            "import * as ns from './x';\nexport const f = () => ns.b;\n"
        ));
        assert!(same(
            "import type { T } from './t';\nexport const a = 1;\n",
            "import type { T, U } from './t';\nexport const a = 2;\n"
        ));
        // An exported type is no name at run time.
        assert!(same(
            "export const a = 1;\nexport type T = number;\n",
            "export const a = 1;\nexport interface U { x: number }\n"
        ));
        // An export that is gone or new, a named import, a named re-export.
        assert!(!same("export const b = 2;\n", "export const c = 2;\n"));
        assert!(!same(
            "export const b = 2;\n",
            "export const b = 2;\nexport const c = 3;\n"
        ));
        assert!(!same(
            "import { x } from './x';\nexport const f = () => x;\n",
            "import { x, y } from './x';\nexport const f = () => x;\n"
        ));
        assert!(!same(
            "export { a } from './a';\n",
            "export { a, b } from './a';\n"
        ));
        assert!(!same(
            "export { a } from './a';\n",
            "export { a as b } from './a';\n"
        ));
    }
}
