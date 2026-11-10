//! Binding holes to the arguments of their callers across files.
//!
//! [`bind`] evaluates the sinks of a set of files ([`super::facts::Sink`]) with
//! every hole replaced by what its callers pass:
//!
//! - **Who calls a function.** A function reached through its name is called by
//!   the direct calls in its own file (already part of every value), by `new
//!   C(...)` in its file for a constructor, and by the users of its exports.
//!   The users of an export are found from the graph: every module-loading edge
//!   into the file ([`World::importers`]), through re-export barrels. An
//!   importer whose facts list a call of the binding contributes the call's
//!   arguments, evaluated in the importer (where the importer's own holes are
//!   bound the same way, transitively).
//! - **Unknown callers have a home.** Whoever invokes a function got hold of it
//!   somewhere: in a file that lets the binding escape (passes it on, stores
//!   it, re-exports it in a way that cannot be followed), or in a file that
//!   loads the module in a way the facts do not describe (`require`,
//!   `import()`, a test runner's setup list, a file without facts). That file
//!   ran, so it is in the closure of every entry that can cause the invocation.
//!   The binding is evaluated there with an unknown argument: the result (an
//!   open record, or every value of a lookup table) lands on that file, not on
//!   the callee.
//! - **A value has a supplier.** Every alternative of a bound value carries the
//!   file and line where its deepest hole was filled. The dependency is
//!   reported there ([`Dep::file`]): an entry that makes the callee read that
//!   value has loaded the file that passed it.
//! - **Calls and constants of other files.** The result of calling an imported
//!   function is its return summary ([`super::facts::FnFact::returns`]) with
//!   the call's arguments in place of its holes; an imported constant is its
//!   exported value. The file that calls or reads them supplies the result.
//! - **Closed sinks.** The callee's own record ([`Bound::closed`]) is no longer
//!   needed when no hole of the sink is open *at home*: the function does not
//!   escape inside its own file and is not reached in a way the facts cannot
//!   describe (a method, a callback of an unknown callee). Everything else has
//!   been moved to a supplier.
//!
//! - **Loaders the graph does not show.** A module that is only loaded through
//!   a bound site has no edge from the file that loads it. The file that loads
//!   it is a user of every export all the same (it holds the module object), so
//!   binding runs again with those loaders among the importers until nothing
//!   new turns up.
//! - **Objects that change.** A property read through a hole is the caller's
//!   value only when nothing changed the object in between. A write in the
//!   reading file is part of its facts (an unknown next to the read); a callee
//!   the object is handed to is asked for its facts ([`X::Changed`]): the names
//!   its file assigns and what it hands the object on to. The exports of a
//!   module that is followed can be changed by any other file that holds them
//!   and does more than read: that file supplies an unknown.
//!
//! The evaluation over-approximates: alternatives are joined, combinations are
//! capped (a capped or unmodelled operation is an unknown at the supplier),
//! recursion through holes is cut with an unknown, and a step budget turns the
//! rest into unknowns. Unknowns are never dropped: each becomes an open record
//! ([`DepValue::Open`]).
//!
//! # What keeps a site open
//!
//! Every place where the evaluation gives something up yields a value, never
//! nothing:
//!
//! | Given up | Value | Effect |
//! |---|---|---|
//! | more alternatives or combinations than the caps, the step budget | [`gave_up`] (home, no file) | every site that depends on it stays open; the budget also keeps the site being bound open |
//! | recursion through a hole, nesting deeper than the bound | unknown, home at the hole's file | the site stays open there, or an open record on that file |
//! | a file without facts, an importer that is not a static import, an escaping use | `callers not known` at that file | an open record on that file |
//! | an export that cannot be found, a call that is not followed, an unmodelled operation | unknown at the file that holds the expression | open record there (the site itself when it is the site's file) |
//! | a module outside the repository or one that cannot be resolved | unknown at the supplier of the specifier | open record there |
//! | rounds of loaders that do not settle | no site and no table is closed | records and edges stay; dependencies are still added |

use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
};

use turborepo_trace_core::{
    Origin, RefKind,
    api::BaseRule,
    eval::{PatPart, Value},
};

pub use super::facts::REQUIRE;
use super::facts::{ARRAY_ELEMENT, AS_ARRAY, FileFacts, RECEIVER, Read, Sink, Source, X};
use crate::extract::{
    evaluator::{self, Anchor, Func, JsVal, Scope},
    origin::{self, Tag},
};

/// The directory that stands for the repository root in bound values: a path
/// below it is repository-relative.
pub const ROOT: &str = "/__turbotrace_root__";

/// Evaluation steps per [`bind`] call.
const MAX_STEPS: u32 = 4_000_000;
/// Evaluation steps per site file.
const FILE_STEPS: u32 = 100_000;

/// `TURBO_TRACE_DEBUG_BIND_TRACE`, a debug aid of [`Binder::sink`].
fn trace_part() -> Option<&'static str> {
    static PART: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    PART.get_or_init(|| {
        turborepo_trace_core::debug::var("BIND_TRACE")
            .ok()
            .filter(|v| !v.is_empty())
    })
    .as_deref()
}

/// The step budget of one site file: [`FILE_STEPS`], or
/// `TURBO_TRACE_DEBUG_BIND_STEPS` (a measurement aid: what would close with a
/// larger budget, and what it costs). The total of a [`bind`] call grows with
/// it. Part of the key of a cached result.
pub fn file_steps() -> u32 {
    static STEPS: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *STEPS.get_or_init(|| {
        turborepo_trace_core::debug::var("BIND_STEPS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(FILE_STEPS)
    })
}
/// Alternatives kept per value. A value has one alternative per supplier, so a
/// helper with thousands of callers has thousands; what bounds the work is the
/// step budget.
const MAX_ALTS: usize = 50_000;
/// Combinations of one operation.
const MAX_COMBOS: usize = 20_000;
/// Alternatives of the second-largest operand of one operation. One operand may
/// have an alternative per caller (thousands); two such operands are a cross
/// product of values that mostly never meet, and each combination costs a
/// string.
const MAX_CROSS: usize = 16;

/// Known text longer than this is not a path (`PATH_MAX`).
const MAX_TEXT: usize = 4096;

/// The length of the known text of a value.
fn text_len(v: &Val) -> usize {
    let value = |v: &Value| match v {
        Value::Str(s) => s.len(),
        Value::Pattern(parts) => parts
            .iter()
            .map(|p| match p {
                PatPart::Lit(s) => s.len(),
                PatPart::Wild { .. } => 0,
            })
            .sum(),
        _ => 0,
    };
    match v {
        Val::Js(JsVal::Str(v) | JsVal::Path(_, v) | JsVal::Url(_, v)) => value(v),
        _ => 0,
    }
}

/// Whether an operation over operands with these numbers of alternatives is
/// given up.
fn too_many(lens: impl Iterator<Item = usize>) -> bool {
    let (mut product, mut first, mut second) = (1usize, 0usize, 0usize);
    for n in lens {
        product = product.saturating_mul(n.max(1));
        if n > first {
            second = first;
            first = n;
        } else if n > second {
            second = n;
        }
    }
    product > MAX_COMBOS || second > MAX_CROSS
}
/// Re-export hops followed.
const MAX_HOPS: u32 = 12;
/// Nested hole bindings.
const MAX_DEPTH: u32 = 24;
/// Nested calls followed through return summaries.
const MAX_FRAMES: usize = 4;

/// A module-loading edge into a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Importer {
    /// The file that loads the module.
    pub file: u32,
    /// The byte offset of the declaration, for static `import` / `export …
    /// from` declarations; `None` for any other way of loading it.
    pub at: Option<u32>,
}

/// What binding needs from the graph and the files.
pub trait World {
    /// The repository-relative path of a file (`/`-separated).
    fn path(&self, file: u32) -> String;
    /// The facts of a file; `None` when it cannot be analyzed.
    fn facts(&self, file: u32) -> Option<Arc<FileFacts>>;
    /// The files the import or re-export declaration at byte `at` of `file`
    /// resolves to.
    fn targets(&self, file: u32, at: u32) -> Vec<u32>;
    /// Every module-loading edge into `file`.
    fn importers(&self, file: u32) -> Vec<Importer>;
    /// A hint that the facts of these files are about to be requested.
    fn prefetch(&self, _files: &[u32]) {}
    /// A hint that the importers of these files are about to be requested (a
    /// world that finds importers by looking at every edge can do so once
    /// for all of them).
    fn expect_importers(&self, _files: &[u32]) {}
    /// The files that `file` resolves the module specifier `spec` to (as one of
    /// its own references does). Default: none known.
    fn resolve(&self, _file: u32, _spec: &str) -> Vec<u32> {
        Vec::new()
    }
    /// The modules a load of the repository path can reach: the path itself,
    /// with a module extension, or its index file (every candidate that
    /// exists: which one a runner takes depends on its extension order).
    /// Default: not known.
    fn module_at(&self, _path: &str) -> Vec<u32> {
        Vec::new()
    }
    /// Whether the file is source code that runs in the process of what loads
    /// it (not a task, a manifest or a data file). Default: yes.
    fn is_code(&self, _file: u32) -> bool {
        true
    }
    /// The working directories tests run in (repository-relative). Default:
    /// none known.
    fn cwds(&self) -> Vec<String> {
        Vec::new()
    }
    /// The file at a repository-relative path, for replaying a recorded run in
    /// this world ([`super::trace`]). Default: not known, so nothing is
    /// replayed.
    fn file(&self, _path: &str) -> Option<u32> {
        None
    }
    /// What the facts of `file` are a function of, as a number: two worlds that
    /// give the same number for a file give the same facts for it (a digest
    /// of the file's contents and name). `None`: this world cannot say, and
    /// [`World::facts_digest`] decides.
    fn facts_source(&self, _file: u32) -> Option<u128> {
        None
    }
    /// A digest of the facts of `file`: [`super::trace::facts_digest`] of them.
    fn facts_digest(&self, file: u32) -> u64 {
        super::trace::facts_digest(self.facts(file).as_deref())
    }
}

/// Where a value comes from.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct Prov {
    file: u32,
    line: u32,
    /// Hole bindings crossed.
    depth: u32,
    /// The value stands for callers that cannot be enumerated, at the callee
    /// itself.
    home: bool,
}

#[derive(Clone, Debug)]
enum Val {
    Js(JsVal),
    Object(Rc<Obj>),
    Array(Rc<Arr>),
    /// A function: file and id.
    Func(u32, u32),
}

/// Equality of the contents. A container is shared on its way through the
/// binder, not copied, so the same container is met again and again when
/// alternatives are joined: it equals itself without a look inside (nothing in
/// a value is unequal to itself).
impl PartialEq for Val {
    fn eq(&self, other: &Val) -> bool {
        match (self, other) {
            (Val::Js(a), Val::Js(b)) => a == b,
            (Val::Object(a), Val::Object(b)) => Rc::ptr_eq(a, b) || a == b,
            (Val::Array(a), Val::Array(b)) => Rc::ptr_eq(a, b) || a == b,
            (Val::Func(f, id), Val::Func(g, other)) => f == g && id == other,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Obj {
    props: Vec<(String, Vals)>,
    open: Option<Vals>,
    /// The value of a known property is all the property can hold: an object
    /// literal of the facts, whose property values already include the open
    /// value where it applies (not for a property written after the last
    /// spread). Otherwise (the exports of a module) the open value is one
    /// more alternative of every property.
    complete: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct Arr {
    elems: Vec<Vals>,
    rest: Option<Vals>,
}

#[derive(Clone, Debug)]
struct Alt {
    v: Val,
    p: Prov,
}

/// Two alternatives are the same when value and supplier are. The supplier is
/// compared first: alternatives of one value mostly differ in who supplies
/// them, and that takes four numbers, not the text of a value.
impl PartialEq for Alt {
    fn eq(&self, other: &Alt) -> bool {
        self.p == other.p && self.v == other.v
    }
}

type Vals = Vec<Alt>;

/// How a file uses an export of another file.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Use {
    /// Calls it: the index of the call in the file's facts.
    Call(u32, usize),
    /// Lets it escape, or loads the module in a way the facts do not describe.
    Opaque(u32),
}

/// A bound dependency.
#[derive(Clone, Debug, PartialEq)]
pub struct Dep {
    /// The file that supplies the value: the dependency belongs to it.
    pub file: u32,
    /// The line where it does (0: not known).
    pub line: u32,
    /// What the site does with the value.
    pub kind: RefKind,
    /// The file of the site.
    pub sink_file: u32,
    /// The line of the site.
    pub sink_line: u32,
    /// The span of the site's references.
    pub sink_site: [u32; 2],
    /// The value.
    pub value: DepValue,
}

/// The value of a bound dependency.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum DepValue {
    /// A path from the repository root (normalized, no leading slash).
    Path(String),
    /// Something below this directory (repository-relative): a path with
    /// unknown parts after a known directory.
    Below(String),
    /// A path relative to the process working directory.
    Cwd(String),
    /// A module specifier as the site's file would resolve it (`./x` is
    /// relative to the site's file, anything else a package or an alias).
    Spec(String),
    /// A module relative to the working directory, already evaluated for every
    /// working directory tests run in: a record of the supplier that does
    /// not count for those tests.
    PerTestCwd(String),
    /// Not known: an open record for the supplier.
    Open {
        /// Where the unknown part comes from.
        origin: Origin,
        /// What is unknown, for the record's detail.
        detail: String,
    },
}

/// A sink whose own record or edges are no longer needed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Closed {
    /// The file of the site.
    pub file: u32,
    /// The span of the site's uncertainty record.
    pub at: [u32; 2],
    /// The span of the site's references.
    pub site: [u32; 2],
    /// The site had a parameter-origin record (drop it). Otherwise the site's
    /// references with a specifier in `specs` are replaced by the bound
    /// dependencies.
    pub open: bool,
    /// For resolved sites: the specifiers the bound dependencies replace.
    pub specs: Vec<String>,
}

/// A lookup table whose values only leave it through bound keys: references to
/// its string values inside the table's literal are replaced by the bound
/// dependencies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClosedTable {
    /// The file.
    pub file: u32,
    /// The span of the table's object literal.
    pub at: [u32; 2],
    /// Its string values.
    pub specs: Vec<String>,
}

/// The result of [`bind`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bound {
    /// Dependencies, on the files that supply the values.
    pub deps: Vec<Dep>,
    /// Sinks that are closed.
    pub closed: Vec<Closed>,
    /// Tables that are closed.
    pub tables: Vec<ClosedTable>,
    /// Files whose facts were used.
    pub files: usize,
    /// The step budget ran out (some values degraded to unknowns).
    pub exhausted: bool,
}

/// Which sinks to bind.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Select {
    /// Module loads (`require`, `import()`, workers, child processes).
    pub loads: bool,
    /// File and directory reads.
    pub reads: bool,
}

impl Select {
    fn takes(&self, k: RefKind) -> bool {
        match k {
            RefKind::Require
            | RefKind::DynamicImport
            | RefKind::Import
            | RefKind::Worker
            | RefKind::ChildProcess => self.loads,
            _ => self.reads,
        }
    }
}

/// One alternative of a value bound across files.
#[derive(Clone, Debug, PartialEq)]
pub struct Supplied {
    /// The value; paths below [`ROOT`] are repository paths.
    pub value: JsVal,
    /// The file that supplies it.
    pub file: u32,
    /// The line where it does (0: not known).
    pub line: u32,
    /// It stands for callers that could not be enumerated.
    pub open: bool,
}

/// The values of the expression `x` of `file` with every hole bound to what its
/// callers pass, for consumers with their own kind of sink (a URL passed to a
/// fetch wrapper, the name passed to an environment helper): `x` is typically
/// an argument of a [`super::facts::Call`]. Containers and functions are
/// reported as unknown.
pub fn values(world: &dyn World, file: u32, x: &X) -> Vec<Supplied> {
    let here = Prov {
        file,
        line: 0,
        depth: 0,
        home: false,
    };
    let (out, settled) = settle(world, file_steps(), |b| {
        let vals = b.eval(file, x, here, 0);
        let spent = b.steps == 0;
        vals.into_iter()
            .map(|a| Supplied {
                value: match a.v {
                    Val::Js(j) => j,
                    _ => unknown(Tag::Unknown, "not a string"),
                },
                file: if a.p.file == NOWHERE { file } else { a.p.file },
                line: a.p.line,
                open: a.p.home || spent,
            })
            .collect::<Vec<_>>()
    });
    if settled {
        out
    } else {
        out.into_iter()
            .map(|s| Supplied { open: true, ..s })
            .collect()
    }
}

/// Binds the sinks of `roots` (see the module documentation).
pub fn bind(world: &dyn World, roots: &[u32], select: Select) -> Bound {
    // Files whose budget ran out in an earlier round: their sites stay open
    // whatever a later round finds, so they are not bound again (what they load
    // is already known).
    let mut spent: HashSet<u32> = HashSet::new();
    let total = MAX_STEPS.max(file_steps().saturating_mul(4));
    let (mut out, settled) = settle(world, total, |b| {
        // Every file gets its own share of the budget, so that one expensive file does
        // not leave the sites of the others open.
        let mut left = total;
        let mut exhausted = false;
        for &f in roots {
            if spent.contains(&f) {
                exhausted = true;
                continue;
            }
            let share = left.min(file_steps());
            b.steps = share;
            let t0 = std::time::Instant::now();
            b.file(f, select);
            if turborepo_trace_core::debug::var_os("BIND_ROUNDS").is_some()
                && (t0.elapsed().as_millis() >= 20 || b.steps == 0)
            {
                eprintln!(
                    "bind file {}: {:.1} ms, {} step(s){}",
                    world.path(f),
                    t0.elapsed().as_secs_f64() * 1e3,
                    share - b.steps,
                    if b.steps == 0 { ", exhausted" } else { "" }
                );
                if let Some(prof) = &mut b.prof {
                    let mut v: Vec<_> = prof.drain().collect();
                    v.sort_by_key(|(_, (_, n))| std::cmp::Reverse(*n));
                    eprintln!("  evaluations/alternatives so far: {v:?}");
                }
            }
            if b.steps == 0 {
                exhausted = true;
                spent.insert(f);
            }
            left -= share - b.steps;
        }
        b.out.files = b.facts.values().filter(|f| f.is_some()).count();
        b.out.exhausted = exhausted;
        std::mem::take(&mut b.out)
    });
    if !settled {
        // Loaders kept turning up: the callers of some function are not all known.
        out.closed.clear();
        out.tables.clear();
        out.exhausted = true;
    }
    out
}

/// Rounds of binding until the loaders of bound modules are stable.
const MAX_ROUNDS: usize = 4;

/// Runs `run` with a fresh binder, again with the loaders the last run found
/// (files that load a module through a bound site: the graph has no edge for
/// them), until a run finds no loader that could change what it computed.
/// `false`: it did not settle.
fn settle<T>(
    world: &dyn World,
    steps: u32,
    mut run: impl FnMut(&mut Binder<'_>) -> T,
) -> (T, bool) {
    let mut loaders: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut round = 0;
    loop {
        round += 1;
        let mut b = Binder {
            w: world,
            facts: HashMap::new(),
            holes: HashMap::new(),
            busy: Vec::new(),
            frames: Vec::new(),
            modules: HashMap::new(),
            module_busy: Vec::new(),
            users: HashMap::new(),
            users_busy: Vec::new(),
            users_hit: usize::MAX,
            loaders: std::mem::take(&mut loaders),
            loads: HashSet::new(),
            asked_users: HashSet::new(),
            asked_exports: HashMap::new(),
            changes: HashMap::new(),
            changes_busy: Vec::new(),
            cwd_modules: HashMap::new(),
            steps,
            out: Bound::default(),
            prof: turborepo_trace_core::debug::var_os("BIND_ROUNDS").map(|_| HashMap::new()),
        };
        let t0 = std::time::Instant::now();
        let out = run(&mut b);
        if turborepo_trace_core::debug::var_os("BIND_ROUNDS").is_some() {
            eprintln!(
                "bind round {round}: {:.1} ms, {} load(s), {} hole(s), {} module(s), {} users \
                 asked, steps left {}",
                t0.elapsed().as_secs_f64() * 1e3,
                b.loads.len(),
                b.holes.len(),
                b.modules.len(),
                b.asked_users.len(),
                b.steps
            );
        }
        let mut grew = false;
        let mut loads: Vec<(u32, u32)> = b.loads.iter().copied().collect();
        loads.sort_unstable();
        loaders = b.loaders;
        for (loader, module) in loads {
            if loader == module
                || loaders.get(&module).is_some_and(|l| l.contains(&loader))
                || world
                    .importers(module)
                    .iter()
                    .any(|i| i.file == loader && i.at.is_none())
            {
                continue;
            }
            loaders.entry(module).or_default().push(loader);
            // Only a module whose users or other holders were asked for can change the
            // result.
            let matters = b.asked_users.contains(&module)
                || b.asked_exports
                    .get(&module)
                    .is_some_and(|ls| ls.iter().any(|l| *l != loader));
            if matters && !grew && turborepo_trace_core::debug::var_os("BIND_ROUNDS").is_some() {
                eprintln!(
                    "bind round {round}: new loader {} of {}",
                    world.path(loader),
                    world.path(module)
                );
            }
            grew |= matters;
        }
        if !grew {
            return (out, true);
        }
        if round >= MAX_ROUNDS {
            return (out, false);
        }
    }
}

struct Binder<'w> {
    w: &'w dyn World,
    facts: HashMap<u32, Option<Arc<FileFacts>>>,
    /// Bound holes: `(file, function, parameter, reads)`.
    holes: HashMap<(u32, u32, u32, Vec<Step>), Rc<Vals>>,
    busy: Vec<(u32, u32, u32)>,
    /// Calls being evaluated through their return summary (innermost last).
    frames: Vec<Frame>,
    /// What `require` of a file returns to a loader: `(module, loader)`.
    modules: HashMap<(u32, u32), Rc<Vals>>,
    module_busy: Vec<u32>,
    /// Users of an export: `(file, name)`.
    users: HashMap<(u32, String), Rc<Vec<Use>>>,
    /// Exports whose users are being collected (outermost first).
    users_busy: Vec<(u32, String)>,
    /// The outermost position in `users_busy` that a lookup ran into again (a
    /// cycle of re-exports) since the current collection started.
    users_hit: usize,
    /// Files that load a module through a bound site, by module (from earlier
    /// rounds).
    loaders: HashMap<u32, Vec<u32>>,
    /// `(loader, module)`: modules bound sites load, found in this round.
    loads: HashSet<(u32, u32)>,
    /// Modules whose users were asked for.
    asked_users: HashSet<u32>,
    /// Modules whose exports were followed, with the loaders they were followed
    /// for.
    asked_exports: HashMap<u32, Vec<u32>>,
    /// Whether a function can change the object it is handed: `(file, function,
    /// position, property)`.
    changes: HashMap<(u32, u32, u32, Option<String>), bool>,
    changes_busy: Vec<(u32, u32, u32, Option<String>)>,
    /// [`Binder::cwd_modules`] by specifier.
    cwd_modules: HashMap<String, Rc<[u32]>>,
    steps: u32,
    out: Bound,
    /// With `TURBO_TRACE_DEBUG_BIND_ROUNDS`: evaluations and alternatives
    /// produced per kind of expression.
    prof: Option<HashMap<&'static str, (u64, u64)>>,
}

/// The arguments of a call whose callee's return summary is being evaluated.
struct Frame {
    file: u32,
    func: u32,
    /// The file of the call and where its values come from.
    caller: u32,
    here: Prov,
    args: Vec<Arg>,
    rest: Option<Arg>,
    /// Arguments evaluated so far: `(position, reads)`.
    seen: HashMap<(u32, Vec<Step>), Rc<Vals>>,
}

/// An argument of a call being evaluated through the callee's return summary.
#[derive(Clone)]
enum Arg {
    /// Not evaluated until the summary reads it, and then only as far as it is
    /// read (a function handed a whole generated source text may only look
    /// at an option next to it).
    Lazy(Rc<X>),
    /// A value (the array a rest parameter stands for).
    Vals(Rc<Vals>),
}

fn unknown(tag: Tag, text: &str) -> JsVal {
    JsVal::Unknown(origin::mark(tag, "", text))
}

/// A piece of a known string (a character, its length): unknown.
fn fragment(p: Prov) -> Alt {
    Alt {
        v: Val::Js(JsVal::Unknown(origin::mark(
            Tag::Unknown,
            "fragment",
            "a piece of a known string",
        ))),
        p,
    }
}

/// Module loads only.
const LOADS: Select = Select {
    loads: true,
    reads: false,
};

/// The payload of the unknown that stands for "a module relative to the working
/// directory, evaluated for the tests' working directories".
const PER_TEST_CWD: &str = "cwd-per-test";

/// `rel` below the repository directory `dir`; `None` when it leaves the
/// repository.
fn join_repo(dir: &str, rel: &str) -> Option<String> {
    if rel.starts_with('/') {
        return None;
    }
    let mut parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

fn scope<'a>(consts: &'a HashMap<&'static str, (JsVal, bool)>) -> Scope<'a, 'static> {
    Scope {
        src: b"",
        consts,
        apis: &[],
        loaders: &[],
        file_name: None,
    }
}

/// The "file" of a value that stands for alternatives the evaluation had to
/// give up on.
const NOWHERE: u32 = u32::MAX;

/// Alternatives were given up (too many values or combinations, the step
/// budget): what was dropped is not known, and neither is who supplied it. Such
/// a value keeps every site that depends on it open at the site itself.
fn gave_up() -> Alt {
    Alt {
        v: Val::Js(unknown(Tag::Unknown, "too many alternatives")),
        p: Prov {
            file: NOWHERE,
            line: 0,
            depth: u32::MAX,
            home: true,
        },
    }
}

fn push(out: &mut Vals, a: Alt) {
    // Duplicates come from the same supplier, next to each other: long lists are
    // only checked near their end (a duplicate that stays is harmless).
    let near = out
        .len()
        .saturating_sub(if out.len() > 256 { 32 } else { 256 });
    if out[near..].contains(&a) {
        return;
    }
    if out.len() < MAX_ALTS {
        out.push(a);
    } else if let Some(last) = out.last_mut() {
        // Never drop an alternative silently.
        *last = gave_up();
    }
}

/// One read on the way from a hole to a value: `.name`, any property, any
/// element.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Step {
    Field(String),
    Any,
    Elem,
}

/// `x` as reads of a value: the value and the reads, outermost last.
fn read_path(x: &X) -> (&X, Vec<Step>) {
    let mut steps = Vec::new();
    let mut x = x;
    loop {
        match x {
            X::Field(b, n) => {
                steps.push(Step::Field(n.clone()));
                x = b;
            }
            X::AnyField(b) => {
                steps.push(Step::Any);
                x = b;
            }
            X::Elem(b) => {
                steps.push(Step::Elem);
                x = b;
            }
            _ => {
                steps.reverse();
                return (x, steps);
            }
        }
    }
}

/// `x` as reads of a hole: the function, the parameter and the path.
#[allow(dead_code)]
fn hole_path(x: &X) -> Option<(u32, u32, Vec<Step>)> {
    let mut steps = Vec::new();
    let mut x = x;
    loop {
        match x {
            X::Param { func, index } => {
                steps.reverse();
                return Some((*func, *index, steps));
            }
            X::Field(b, n) => {
                steps.push(Step::Field(n.clone()));
                x = b;
            }
            X::AnyField(b) => {
                steps.push(Step::Any);
                x = b;
            }
            X::Elem(b) => {
                steps.push(Step::Elem);
                x = b;
            }
            _ => return None,
        }
    }
}

impl Binder<'_> {
    fn facts(&mut self, file: u32) -> Option<Arc<FileFacts>> {
        if let Some(f) = self.facts.get(&file) {
            return f.clone();
        }
        let f = self.w.facts(file);
        self.facts.insert(file, f.clone());
        f
    }

    fn tick(&mut self) -> bool {
        if self.steps == 0 {
            return false;
        }
        self.steps -= 1;
        true
    }

    // ---- sinks ----

    fn file(&mut self, file: u32, select: Select) {
        let Some(facts) = self.facts(file) else {
            return;
        };
        let mut closed_sinks: Vec<[u32; 2]> = Vec::new();
        for s in &facts.sinks {
            if !select.takes(s.kind) {
                continue;
            }
            if self.sink(file, s) {
                closed_sinks.push(s.at);
            }
        }
        if closed_sinks.is_empty() {
            return;
        }
        // Tables whose every key is bound: a value can only be selected by a supplier.
        for t in &facts.tables {
            if !t.keyed_only {
                continue;
            }
            // And whose values only reach sites that are closed (a site that does not
            // depend on holes keeps its own references).
            let all_closed = t
                .sinks
                .iter()
                .all(|at| closed_sinks.contains(at) || !facts.sinks.iter().any(|k| k.at == *at));
            if !all_closed {
                continue;
            }
            let here = Prov {
                file,
                line: 0,
                depth: 0,
                home: false,
            };
            let mut ok = true;
            for k in &t.keys {
                let vals = self.eval(file, k, here, 0);
                ok &= !vals
                    .iter()
                    .any(|a| a.p.home && (a.p.file == file || a.p.file == NOWHERE));
            }
            if ok && self.steps > 0 {
                self.out.tables.push(ClosedTable {
                    file,
                    at: t.at,
                    specs: t.values.iter().map(|(_, v)| v.clone()).collect(),
                });
            }
        }
    }

    /// Binds one sink; `true` when it is closed.
    fn sink(&mut self, file: u32, s: &Sink) -> bool {
        // An unknown of the file itself among the alternatives keeps the site's record
        // whatever the callers pass: every entry that reaches the file is selected on
        // every change, so what the callers pass adds nothing and is not evaluated.
        let own_unknown = |x: &X| matches!(x, X::Const(JsVal::Unknown(_)));
        let stays = match &s.value {
            X::Alts(xs) => xs.iter().any(own_unknown),
            x => own_unknown(x),
        };
        if stays {
            return false;
        }
        let here = Prov {
            file,
            line: s.line,
            depth: 0,
            home: false,
        };
        let mut vals = self.eval(file, &s.value, here, 0);
        if self.only_strings(file, s, here) {
            // A container or a function never reaches the site: the test around it
            // lets strings through and nothing else. An unknown stays (it can be one).
            vals.retain(|a| matches!(a.v, Val::Js(_)));
        }
        let mut home = false;
        let mut deps = Vec::new();
        let mut seen: std::collections::HashSet<(u32, u32, DepValue)> =
            std::collections::HashSet::new();
        // `TURBO_TRACE_DEBUG_BIND_TRACE=<part of a path>`: the unknown alternatives of
        // every sink that a matching file supplies (or that stay at home in a
        // matching site file), with the whole reason text: how the unknown came
        // about.
        if let Some(part) = trace_part() {
            for a in &vals {
                let at_home = a.p.home && (a.p.file == file || a.p.file == NOWHERE);
                let who = if a.p.file == NOWHERE {
                    "nowhere".to_owned()
                } else {
                    self.w.path(a.p.file)
                };
                if !(who.contains(part) || (at_home && self.w.path(file).contains(part))) {
                    continue;
                }
                let what = match &a.v {
                    Val::Js(JsVal::Unknown(r)) => format!("unknown {r}"),
                    // A constant that is no string (a number, a container of the
                    // extractor's domain): its kind and the start of its text.
                    Val::Js(j)
                        if matches!(
                            lower(j, s.rule),
                            Some(DepValue::Open { ref detail, .. }) if detail == "not a string"
                        ) =>
                    {
                        let text = format!("{j:?}");
                        format!(
                            "a constant that is not a string: {}",
                            text.chars().take(60).collect::<String>()
                        )
                    }
                    Val::Js(_) => continue,
                    Val::Object(_) => "an object".to_owned(),
                    Val::Array(_) => "an array".to_owned(),
                    Val::Func(..) => "a function".to_owned(),
                };
                eprintln!(
                    "trace {}:{} <- {who}:{} depth {}{}: {what}",
                    self.w.path(file),
                    s.line,
                    a.p.line,
                    a.p.depth,
                    if a.p.home { " HOME" } else { "" },
                );
            }
        }
        for a in &vals {
            // Callers that cannot be enumerated, or an unknown of the file itself (a call
            // that is not followed, an operation that is not modelled): the site's own
            // record stays. A piece of a known string (a character, its length) is an
            // unknown like any other: `.` is a module specifier.
            let own =
                a.p.file == file && a.p.depth == 0 && matches!(&a.v, Val::Js(JsVal::Unknown(_)));
            if (a.p.home && (a.p.file == file || a.p.file == NOWHERE)) || own {
                home = true;
                continue;
            }
            let load = LOADS.takes(s.kind);
            let not_a_string = || DepValue::Open {
                origin: Origin::Unknown,
                detail: "not a string".to_owned(),
            };
            // The dependencies of this alternative, each with its supplier.
            let mut values: Vec<(DepValue, Prov)> = Vec::new();
            match &a.v {
                Val::Js(j) => values.extend(lower(j, s.rule).map(|v| (v, a.p))),
                // `require` of anything but a string throws. `import()`, a worker and a
                // child process take the string form of their argument: `[object
                // Object]` for a plain object, the source text for a function.
                Val::Object(_) | Val::Func(..) | Val::Array(_)
                    if load && s.kind == RefKind::Require => {}
                Val::Func(..) if load => {}
                Val::Object(o) if load => {
                    // An object that brings its own string form can name a module.
                    let own_form = o.open.is_some()
                        || o.props
                            .iter()
                            .any(|(k, _)| matches!(k.as_str(), "toString" | "valueOf" | "href"));
                    if own_form {
                        values.push((not_a_string(), a.p));
                    }
                }
                Val::Array(arr) if load => {
                    // The string form of an array is its elements joined by commas: a
                    // path only when there is one element. A worker or a child process
                    // can take a list of arguments: any element.
                    let single = matches!(s.kind, RefKind::DynamicImport | RefKind::Import);
                    let cands: Vec<&Vals> = if !single {
                        arr.elems.iter().chain(arr.rest.iter()).collect()
                    } else {
                        match (arr.elems.as_slice(), &arr.rest) {
                            ([e], _) => vec![e],
                            ([], Some(r)) => vec![r],
                            _ => Vec::new(),
                        }
                    };
                    for e in cands.into_iter().flatten() {
                        if e.p.home && (e.p.file == file || e.p.file == NOWHERE) {
                            home = true;
                            continue;
                        }
                        let p = if e.p.depth > a.p.depth { e.p } else { a.p };
                        match &e.v {
                            Val::Js(j) => values.extend(lower(j, s.rule).map(|v| (v, p))),
                            Val::Func(..) => {}
                            // `[{…}]` is `[object Object]` unless the object brings its
                            // own string form.
                            Val::Object(o)
                                if o.open.is_none()
                                    && !o.props.iter().any(|(k, _)| {
                                        matches!(k.as_str(), "toString" | "valueOf" | "href")
                                    }) => {}
                            _ => values.push((not_a_string(), p)),
                        }
                    }
                }
                _ => values.push((not_a_string(), a.p)),
            }
            if let Some(part) = trace_part()
                && values.iter().any(|(v, p)| {
                    matches!(v, DepValue::Open { detail, .. } if detail == "not a string")
                        && p.file != NOWHERE
                        && self.w.path(p.file).contains(part)
                })
            {
                // Which container stands for "not a string": its keys or lengths.
                let what = match &a.v {
                    Val::Object(o) => format!(
                        "object, keys {:?}, open {}",
                        o.props.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
                        o.open.is_some()
                    ),
                    Val::Array(arr) => format!(
                        "array of {} (+rest {}): element kinds {:?}",
                        arr.elems.len(),
                        arr.rest.is_some(),
                        arr.elems
                            .iter()
                            .chain(arr.rest.iter())
                            .flatten()
                            .map(|e| match &e.v {
                                Val::Js(JsVal::Unknown(r)) => format!("unknown {r}"),
                                Val::Js(_) => "constant".to_owned(),
                                Val::Object(o) => format!("object open {}", o.open.is_some()),
                                Val::Array(_) => "array".to_owned(),
                                Val::Func(..) => "function".to_owned(),
                            })
                            .take(6)
                            .collect::<Vec<_>>()
                    ),
                    Val::Js(_) => "constant".to_owned(),
                    Val::Func(..) => "function".to_owned(),
                };
                eprintln!(
                    "trace not-a-string {}:{}: {what}",
                    self.w.path(file),
                    s.line
                );
            }
            for (value, p) in values {
                if load {
                    // The site's file holds the module it loads: it is a user of the
                    // module's exports that the graph has no edge for.
                    for t in self.load_targets(file, &value) {
                        self.loads.insert((file, t));
                    }
                }
                let d = Dep {
                    file: p.file,
                    line: p.line,
                    kind: s.kind,
                    sink_file: file,
                    sink_line: s.line,
                    sink_site: s.site,
                    value,
                };
                if seen.insert((d.file, d.line, d.value.clone())) {
                    deps.push(d);
                }
            }
        }
        if self.steps == 0 {
            // Out of budget: the values are incomplete. Keep the callee's record.
            return false;
        }
        self.out.deps.extend(deps);
        if home {
            return false;
        }
        let mut specs = Vec::new();
        if !s.open {
            table_values(&s.value, &mut specs);
        }
        self.out.closed.push(Closed {
            file,
            at: s.at,
            site: s.site,
            open: s.open,
            specs,
        });
        true
    }

    /// Whether the site `s` of `file` only runs where its argument is a string:
    /// a `typeof` test of the argument guards it ([`Sink::string_only`]),
    /// or a call does whose every callee is a test for a string of the
    /// argument's position ([`super::facts::FnFact::tests_string`]). A
    /// callee that is not a function with facts, or one of several that is
    /// no such test, decides nothing.
    fn only_strings(&mut self, file: u32, s: &Sink, here: Prov) -> bool {
        if s.string_only {
            return true;
        }
        for (callee, pos) in &s.string_tests {
            let callees = self.eval(file, callee, here, 0);
            let mut all = !callees.is_empty();
            for c in &callees {
                let test = match c.v {
                    Val::Func(tf, id) => self
                        .facts(tf)
                        .and_then(|f| f.func(id).map(|f| f.tests_string == Some(*pos)))
                        .unwrap_or(false),
                    _ => false,
                };
                if !test {
                    all = false;
                    break;
                }
            }
            if all {
                return true;
            }
        }
        false
    }

    /// The modules a bound module load of `file` with value `v` can reach.
    fn load_targets(&mut self, file: u32, v: &DepValue) -> Vec<u32> {
        match v {
            DepValue::Path(p) => self.w.module_at(p),
            DepValue::Spec(s) => {
                let mut ts = self.w.resolve(file, s);
                if ts.is_empty() && (s.starts_with("./") || s.starts_with("../")) {
                    let path = self.w.path(file);
                    if let Some(p) = join_repo(dir_of(&path), s) {
                        ts = self.w.module_at(&p);
                    }
                }
                ts
            }
            _ => Vec::new(),
        }
    }

    // ---- evaluation ----

    fn js_alts(&self, file: u32, j: &JsVal, p: Prov, out: &mut Vals) {
        for leaf in evaluator::split_alts(j.clone()) {
            push(
                out,
                Alt {
                    v: Val::Js(self.reanchor(file, leaf)),
                    p,
                },
            );
        }
    }

    /// `j` with paths anchored at the directory of `file` made absolute below
    /// [`ROOT`].
    fn reanchor(&self, file: u32, j: JsVal) -> JsVal {
        match j {
            JsVal::Path(Anchor::FileDir, v) | JsVal::Url(Anchor::FileDir, v) => {
                let consts = HashMap::new();
                let sc = scope(&consts);
                let path = self.w.path(file);
                let dir = dir_of(&path);
                let base = if dir.is_empty() {
                    ROOT.to_owned()
                } else {
                    format!("{ROOT}/{dir}")
                };
                let args = [JsVal::Str(Value::str(base)), JsVal::Str(v)];
                evaluator::apply(Func::PathJoin, &args, &sc)
                    .unwrap_or_else(|| unknown(Tag::Unknown, "path"))
            }
            JsVal::File | JsVal::FileUrl => {
                JsVal::Str(Value::str(format!("{ROOT}/{}", self.w.path(file))))
            }
            other => other,
        }
    }

    fn eval(&mut self, file: u32, x: &X, here: Prov, depth: u32) -> Vals {
        let mut out = Vals::new();
        if !self.tick() || depth > 64 {
            out.push(gave_up());
            return out;
        }
        let d = depth + 1;
        // Reads are pushed down to where the value is built ([`Binder::eval_read`]).
        if matches!(x, X::Field(..) | X::AnyField(_) | X::Elem(_)) {
            let (base, path) = read_path(x);
            let vals = self.eval_read(file, base, &path, here, d);
            self.steps = self.steps.saturating_sub(vals.len() as u32);
            return vals;
        }
        match x {
            X::Const(j) => self.js_alts(file, j, here, &mut out),
            X::Void => {}
            X::Param { func, index } => {
                let vals = self.hole(file, *func, *index, &[], here);
                for a in vals.iter() {
                    push(&mut out, a.clone());
                }
            }
            X::Field(b, name) => {
                for a in self.eval(file, b, here, d) {
                    self.member(&a, Some(name), &mut out);
                }
            }
            X::AnyField(b) => {
                for a in self.eval(file, b, here, d) {
                    self.member(&a, None, &mut out);
                }
            }
            X::Elem(b) => {
                for a in self.eval(file, b, here, d) {
                    self.elem(&a, &mut out);
                }
            }
            X::Concat(a, b) => {
                let (l, r) = (self.eval(file, a, here, d), self.eval(file, b, here, d));
                if too_many([l.len(), r.len()].into_iter()) {
                    out.push(gave_up());
                    return out;
                }
                // A text longer than any path (generated source that a function is handed
                // next to the path it reads) is not built: an unknown where its parts come
                // from, which costs nothing however many alternatives the parts have.
                let longest = |vs: &Vals| vs.iter().map(|a| text_len(&a.v)).max().unwrap_or(0);
                if longest(&l) + longest(&r) > MAX_TEXT {
                    let floor = [&l, &r]
                        .iter()
                        .map(|vs| vs.iter().map(|a| a.p.depth).min().unwrap_or(0))
                        .max()
                        .unwrap_or(0);
                    let long = || Val::Js(unknown(Tag::Unknown, "a text longer than a path"));
                    if floor <= here.depth {
                        push(&mut out, Alt { v: long(), p: here });
                    }
                    for a in l.iter().chain(&r) {
                        if a.p.depth >= floor && (a.p.depth > here.depth || a.p.home) {
                            push(&mut out, Alt { v: long(), p: a.p });
                        }
                    }
                    return out;
                }
                let reason = origin::mark(Tag::Unknown, "", "concatenation");
                self.steps = self.steps.saturating_sub((l.len() * r.len()) as u32);
                for x in &l {
                    for y in &r {
                        let (xj, yj) = (operand(&x.v, true), operand(&y.v, false));
                        let j = evaluator::concat(xj, yj, &reason);
                        push(
                            &mut out,
                            Alt {
                                v: Val::Js(j),
                                p: deepest([x.p, y.p], here),
                            },
                        );
                    }
                }
            }
            X::Apply(f, xs) => {
                let args: Vec<Vals> = xs.iter().map(|a| self.eval(file, a, here, d)).collect();
                self.apply(*f, &args, here, &mut out);
            }
            X::Opaque(xs) => {
                // Some function of the operands: unknown, with their origins.
                let mut reasons: Vec<String> = Vec::new();
                // The suppliers of each operand.
                let mut ops: Vec<Vec<Prov>> = Vec::new();
                let mut never = false;
                for a in xs {
                    let vals = self.eval(file, a, here, d);
                    // An operand without a value (a hole nobody fills): the operation
                    // never runs.
                    never |= vals.is_empty();
                    let mut ps: Vec<Prov> = Vec::new();
                    for v in vals {
                        if !ps.contains(&v.p) {
                            ps.push(v.p);
                        }
                        if let Val::Js(JsVal::Unknown(r)) = &v.v {
                            let r = r.to_string();
                            if !reasons.contains(&r) && reasons.len() < 8 {
                                reasons.push(r);
                            }
                        }
                    }
                    ops.push(ps);
                }
                if never {
                    return out;
                }
                if trace_part().is_some() && reasons.iter().any(|r| r.contains("@import[]")) {
                    let shown = match self.facts(file) {
                        Some(f) => super::show::x(x, &f),
                        None => String::new(),
                    };
                    eprintln!(
                        "trace opaque in {} (depth {}): {}",
                        self.w.path(file),
                        here.depth,
                        shown.chars().take(700).collect::<String>()
                    );
                }
                let j = if reasons.is_empty() {
                    unknown(Tag::Call, "derived value")
                } else {
                    JsVal::Unknown(origin::derive(&reasons.join(" "), "derived value"))
                };
                // The supplier of a combination of operands is its deepest one, so every
                // supplier that is the deepest of some combination supplies the unknown:
                // the ones at least as deep as the shallowest alternative of every
                // operand. (One unknown for all of them would leave the others without.)
                let floor = ops
                    .iter()
                    .map(|ps| ps.iter().map(|p| p.depth).min().unwrap_or(0))
                    .max()
                    .unwrap_or(0);
                if floor <= here.depth {
                    push(
                        &mut out,
                        Alt {
                            v: Val::Js(j.clone()),
                            p: here,
                        },
                    );
                }
                for p in ops.iter().flatten() {
                    if p.depth >= floor && (p.depth > here.depth || p.home) {
                        push(
                            &mut out,
                            Alt {
                                v: Val::Js(j.clone()),
                                p: *p,
                            },
                        );
                    }
                }
            }
            X::Index { table, key } => {
                let tables = self.eval(file, table, here, d);
                for k in self.eval(file, key, here, d) {
                    let name = match &k.v {
                        Val::Js(JsVal::Str(Value::Str(s))) => Some(s.as_str()),
                        _ => None,
                    };
                    for t in &tables {
                        let mut sel = Vals::new();
                        self.member(t, name, &mut sel);
                        // The key decides: its supplier owns the selection.
                        for mut a in sel {
                            if k.p.depth >= a.p.depth {
                                a.p = k.p;
                            }
                            push(&mut out, a);
                        }
                    }
                }
                // A key that is `undefined` or `null` reads the properties of those names
                // (the engine's rule); such keys are not values here, so a table that has
                // them always yields them.
                for t in &tables {
                    if let Val::Object(o) = &t.v {
                        for (name, v) in &o.props {
                            if name == "undefined" || name == "null" {
                                v.iter().for_each(|x| push(&mut out, x.clone()));
                            }
                        }
                    }
                }
            }
            X::Object { props, open } => {
                let props = props
                    .iter()
                    .map(|(k, v)| (k.clone(), self.eval(file, v, here, d)))
                    .collect();
                let open = open.as_ref().map(|o| self.eval(file, o, here, d));
                push(
                    &mut out,
                    Alt {
                        v: Val::Object(Rc::new(Obj {
                            props,
                            open,
                            complete: true,
                        })),
                        p: here,
                    },
                );
            }
            X::Array { elems, rest } => {
                let elems = elems.iter().map(|e| self.eval(file, e, here, d)).collect();
                let rest = rest.as_ref().map(|r| self.eval(file, r, here, d));
                push(
                    &mut out,
                    Alt {
                        v: Val::Array(Rc::new(Arr { elems, rest })),
                        p: here,
                    },
                );
            }
            X::Func(id) => push(
                &mut out,
                Alt {
                    v: Val::Func(file, *id),
                    p: here,
                },
            ),
            X::Import { module, name } => {
                let vals = self.imported(file, module, name, here, d);
                vals.into_iter().for_each(|a| push(&mut out, a));
            }
            X::Call { callee, args, .. } if matches!(&**callee, X::Import { module, .. } if module == REQUIRE) => {
                if let Some(a) = args.first() {
                    self.required(file, a, here, d, &mut out);
                }
            }
            X::Call { callee, args, rest } => {
                self.call(file, callee, args, rest.as_deref(), &[], here, d, &mut out);
            }
            X::Alts(xs) => {
                for a in xs {
                    for v in self.eval(file, a, here, d) {
                        push(&mut out, v);
                    }
                }
            }
            X::Reach { base, steps } => {
                // Everything the reads reach, starting from each value of the base.
                // Containers are visited once (by identity: a value is shared, not
                // copied, on its way here).
                let mut work = self.eval(file, base, here, d);
                let mut seen: HashSet<usize> = HashSet::new();
                while let Some(a) = work.pop() {
                    if !self.tick() {
                        out.push(gave_up());
                        break;
                    }
                    let fresh = match &a.v {
                        Val::Object(o) => seen.insert(Rc::as_ptr(o) as usize),
                        Val::Array(r) => seen.insert(Rc::as_ptr(r) as usize),
                        // An unknown stays one under any read.
                        Val::Js(JsVal::Unknown(_)) => {
                            push(&mut out, a);
                            continue;
                        }
                        _ => true,
                    };
                    if !fresh {
                        continue;
                    }
                    // What the reads make of a string (a piece of it) is unknown and
                    // not read further.
                    if matches!(&a.v, Val::Js(_)) {
                        for s in steps {
                            match s {
                                Read::Field(n) => self.member(&a, Some(n), &mut out),
                                Read::Any => self.member(&a, None, &mut out),
                                Read::Elem => self.elem(&a, &mut out),
                            }
                        }
                        push(&mut out, a);
                        continue;
                    }
                    // A function has the properties its file assigns to something by
                    // name (`f.handler = …`), no elements, and is no array.
                    if let Val::Func(ff, _) = &a.v {
                        let writes = self.facts(*ff).map(|f| f.writes.clone());
                        let assigned = steps.iter().any(|s| match s {
                            Read::Field(n) if n == AS_ARRAY || n == ARRAY_ELEMENT => false,
                            Read::Field(n) => writes.as_ref().is_none_or(|w| w.hits(Some(n))),
                            Read::Any => true,
                            Read::Elem => false,
                        });
                        if assigned {
                            push(
                                &mut out,
                                Alt {
                                    v: Val::Js(unknown(Tag::Unknown, "property of a function")),
                                    p: a.p,
                                },
                            );
                        }
                        push(&mut out, a);
                        continue;
                    }
                    let mut next = Vals::new();
                    for s in steps {
                        match s {
                            Read::Field(n) => self.member(&a, Some(n), &mut next),
                            Read::Any => self.member(&a, None, &mut next),
                            Read::Elem => self.elem(&a, &mut next),
                        }
                    }
                    // Visited once (by identity): no need to look for a duplicate.
                    if out.len() < MAX_ALTS {
                        out.push(a);
                    } else {
                        push(&mut out, a);
                    }
                    work.extend(next);
                }
            }
            X::Changed { by, prop } => {
                // The object is changed where a callee that changes it is the one that
                // is called: the unknown belongs to whoever supplies that callee (the
                // reading file itself for a function it imports).
                for (callee, pos) in by {
                    for p in self.changers(file, callee, *pos, prop.as_deref(), here, d) {
                        push(
                            &mut out,
                            Alt {
                                v: Val::Js(unknown(Tag::Unknown, "an object a callee can change")),
                                p,
                            },
                        );
                    }
                }
            }
        }
        // The budget also covers what was produced (alternatives are copied around).
        self.steps = self.steps.saturating_sub(out.len() as u32);
        if let Some(prof) = &mut self.prof {
            let name = match x {
                X::Const(_) => "const",
                X::Void => "void",
                X::Param { .. } => "param",
                X::Field(..) | X::Elem(_) | X::AnyField(_) => "read",
                X::Concat(..) => "concat",
                X::Apply(..) => "apply",
                X::Opaque(_) => "opaque",
                X::Object { .. } => "object",
                X::Array { .. } => "array",
                X::Index { .. } => "index",
                X::Func(_) => "func",
                X::Import { .. } => "import",
                X::Call { .. } => "call",
                X::Alts(_) => "alts",
                X::Changed { .. } => "changed",
                X::Reach { .. } => "reach",
            };
            let e = prof.entry(name).or_default();
            e.0 += 1;
            e.1 += out.len() as u64;
        }
        out
    }

    /// The value of `x` after the reads `path`, without building what the reads
    /// do not look at:
    ///
    /// - reads of a hole are applied to each caller's argument before the
    ///   callers are joined (a helper with thousands of callers whose option
    ///   objects all differ still has few distinct values of one option);
    /// - a property of an object literal is that property (the others are not
    ///   evaluated), an element of an array literal its elements;
    /// - a read of a call result is that read of the callee's return summary (a
    ///   function that returns `{ file, source }` is not asked for the source
    ///   when the file is read).
    fn eval_read(&mut self, file: u32, x: &X, path: &[Step], here: Prov, depth: u32) -> Vals {
        let Some((step, rest)) = path.split_first() else {
            return self.eval(file, x, here, depth);
        };
        if !self.tick() || depth > 64 {
            return vec![gave_up()];
        }
        let d = depth + 1;
        let mut out = Vals::new();
        match (x, step) {
            (X::Param { func, index }, _) => {
                return (*self.hole(file, *func, *index, path, here)).clone();
            }
            (X::Alts(xs), _) => {
                for a in xs {
                    for v in self.eval_read(file, a, path, here, d) {
                        push(&mut out, v);
                    }
                }
            }
            (X::Object { props, open }, Step::Field(_) | Step::Any) if !matches!(step, Step::Field(n) if n == AS_ARRAY || n == ARRAY_ELEMENT) =>
            {
                let picked: Vec<&X> = match step {
                    // The last definition of a name is the one that counts.
                    Step::Field(n) => props
                        .iter()
                        .rev()
                        .find(|(k, _)| k == n)
                        .map(|(_, v)| v)
                        .into_iter()
                        .collect(),
                    _ => props.iter().map(|(_, v)| v).collect(),
                };
                for v in picked.into_iter().chain(open.as_deref()) {
                    for a in self.eval_read(file, v, rest, here, d) {
                        push(&mut out, a);
                    }
                }
            }
            (X::Array { elems, rest: more }, Step::Elem | Step::Any) => {
                for v in elems.iter().chain(more.as_deref()) {
                    for a in self.eval_read(file, v, rest, here, d) {
                        push(&mut out, a);
                    }
                }
            }
            (
                X::Call {
                    callee,
                    args,
                    rest: spread,
                },
                _,
            ) if !matches!(&**callee, X::Import { module, .. } if module == REQUIRE) => {
                self.call(
                    file,
                    callee,
                    args,
                    spread.as_deref(),
                    path,
                    here,
                    d,
                    &mut out,
                );
            }
            _ => {
                let vals = self.eval(file, x, here, d);
                return self.project(vals, path);
            }
        }
        out
    }

    /// The result of calling `callee` with `args` (and the spread `rest`) in
    /// `file`, after the reads `path`: the callee's return summary with the
    /// arguments in place of its holes.
    #[allow(clippy::too_many_arguments)]
    fn call(
        &mut self,
        file: u32,
        callee: &X,
        args: &[X],
        rest: Option<&X>,
        path: &[Step],
        here: Prov,
        d: u32,
        out: &mut Vals,
    ) {
        for c in self.eval(file, callee, here, d) {
            self.call_value(file, &c, args, rest, path, here, d, out);
        }
    }

    /// The result of calling the value `c` (one alternative of a callee) with
    /// `args` in `file` ([`Binder::call`]).
    #[allow(clippy::too_many_arguments)]
    fn call_value(
        &mut self,
        file: u32,
        c: &Alt,
        args: &[X],
        rest: Option<&X>,
        path: &[Step],
        here: Prov,
        d: u32,
        out: &mut Vals,
    ) {
        // An unknown stays one under any read.
        let opaque = |out: &mut Vals| {
            push(
                out,
                Alt {
                    v: Val::Js(unknown(Tag::Call, "call result")),
                    p: here,
                },
            )
        };
        {
            let Val::Func(tf, id) = c.v else {
                opaque(out);
                return;
            };
            let summary = self
                .facts(tf)
                .and_then(|f| f.func(id).map(|f| (f.returns.clone(), f.rest)));
            let Some((Some(returns), rest_param)) = summary else {
                opaque(out);
                return;
            };
            if self.frames.len() >= MAX_FRAMES
                || self.frames.iter().any(|f| f.file == tf && f.func == id)
            {
                // Recursion, or too deep.
                opaque(out);
                return;
            }
            let mut argv: Vec<Arg> = args.iter().map(|a| Arg::Lazy(Rc::new(a.clone()))).collect();
            let mut restv = rest.map(|r| Arg::Lazy(Rc::new(r.clone())));
            if let Some(i) = rest_param {
                // `...args`: the array of the remaining arguments.
                let i = (i as usize).min(args.len());
                let elems: Vec<Vals> = args[i..]
                    .iter()
                    .map(|a| self.eval(file, a, here, d))
                    .collect();
                let more = rest.map(|r| self.eval(file, r, here, d));
                let arr = Val::Array(Rc::new(Arr { elems, rest: more }));
                argv.truncate(i);
                while argv.len() < i {
                    argv.push(Arg::Vals(Rc::new(Vals::new())));
                }
                argv.push(Arg::Vals(Rc::new(vec![Alt { v: arr, p: here }])));
                restv = None;
            }
            self.frames.push(Frame {
                file: tf,
                func: id,
                caller: file,
                here,
                args: argv,
                rest: restv,
                seen: HashMap::new(),
            });
            // What the callee computes belongs to the file that calls it.
            let vals = self.eval_read(tf, &returns, path, here, d);
            self.frames.pop();
            vals.into_iter().for_each(|a| push(out, a));
        }
    }

    /// Any element of `a`.
    fn elem(&self, a: &Alt, out: &mut Vals) {
        match &a.v {
            Val::Array(arr) => {
                for e in arr.elems.iter().chain(arr.rest.iter()) {
                    e.iter().for_each(|x| push(out, x.clone()));
                }
            }
            // A character of a known string.
            Val::Js(JsVal::Str(Value::Str(_))) => push(out, fragment(a.p)),
            Val::Js(j) => push(out, derived(j, "element", a.p)),
            // A plain object or a function is not iterable (iterating it throws); an
            // object with properties the facts do not list (a `Map`, a spread of
            // something unknown) can hold anything.
            Val::Object(o) if o.open.is_none() => {}
            Val::Func(..) => {}
            Val::Object(_) => push(
                out,
                Alt {
                    v: Val::Js(unknown(Tag::Unknown, "element")),
                    p: a.p,
                },
            ),
        }
    }

    /// `vals` after the reads `path`.
    fn project(&self, mut vals: Vals, path: &[Step]) -> Vals {
        for step in path {
            let mut next = Vals::new();
            for a in &vals {
                match step {
                    Step::Field(n) => self.member(a, Some(n), &mut next),
                    Step::Any => self.member(a, None, &mut next),
                    Step::Elem => self.elem(a, &mut next),
                }
            }
            vals = next;
        }
        vals
    }

    /// The modules `require(arg)` / `import(arg)` in `file` loads, as values:
    /// for each specifier the argument can be, what that module exports.
    fn required(&mut self, file: u32, arg: &X, here: Prov, depth: u32, out: &mut Vals) {
        for a in self.eval(file, arg, here, depth) {
            let derived_unknown = |what: &str| Alt {
                v: Val::Js(match &a.v {
                    Val::Js(JsVal::Unknown(r)) => JsVal::Unknown(origin::derive(r, what)),
                    _ => unknown(Tag::Call, what),
                }),
                p: a.p,
            };
            let Val::Js(j) = &a.v else {
                push(out, derived_unknown("module"));
                continue;
            };
            let targets: Vec<u32> = match lower(j, BaseRule::Module) {
                // Outside the repository, below the temporary directory, the empty
                // string: what such a module exports is not known.
                None => {
                    push(out, derived_unknown("module outside the repository"));
                    continue;
                }
                Some(DepValue::Spec(s)) => {
                    let mut ts = self.w.resolve(file, &s);
                    if ts.is_empty() && (s.starts_with("./") || s.starts_with("../")) {
                        let path = self.w.path(file);
                        if let Some(p) = join_repo(dir_of(&path), &s) {
                            ts.extend(self.w.module_at(&p));
                        }
                    }
                    ts
                }
                Some(DepValue::Path(p)) => self.w.module_at(&p),
                Some(DepValue::Cwd(s)) => {
                    // Every working directory a test runs in. Entries without a known
                    // working directory keep a record (it does not count for tests).
                    push(
                        out,
                        Alt {
                            v: Val::Js(JsVal::Unknown(origin::mark(
                                Tag::Unknown,
                                PER_TEST_CWD,
                                &s,
                            ))),
                            p: a.p,
                        },
                    );
                    let ts = self.cwd_modules(&s);
                    for &t in ts.iter() {
                        let m = self.module(t, file, a.p.depth);
                        m.iter().for_each(|x| push(out, x.clone()));
                    }
                    continue;
                }
                Some(_) => {
                    push(out, derived_unknown("module"));
                    continue;
                }
            };
            if targets.is_empty() {
                push(out, derived_unknown("module that cannot be resolved here"));
                continue;
            }
            for t in targets {
                let m = self.module(t, file, a.p.depth);
                m.iter().for_each(|x| push(out, x.clone()));
            }
        }
    }

    /// The modules that a load of `spec` relative to the working directory
    /// reaches in the working directories tests run in. The same for every
    /// site and caller, and a lookup per working directory: computed once
    /// per specifier.
    fn cwd_modules(&mut self, spec: &str) -> Rc<[u32]> {
        if let Some(ts) = self.cwd_modules.get(spec) {
            return ts.clone();
        }
        let mut ts: Vec<u32> = Vec::new();
        for c in self.w.cwds() {
            if let Some(p) = join_repo(&c, spec) {
                for t in self.w.module_at(&p) {
                    if !ts.contains(&t) {
                        ts.push(t);
                    }
                }
            }
        }
        let ts: Rc<[u32]> = ts.into();
        self.cwd_modules.insert(spec.to_owned(), ts.clone());
        ts
    }

    /// What `require` of the file `t` returns to the file `loader`: the value
    /// of `module.exports`, or an object of the module's exports. Its
    /// constants are the module's own: the file that holds a literal
    /// supplies it. Every other file that holds the module's exports and
    /// does more than read them supplies an unknown: it can have changed
    /// them before `loader` looks.
    fn module(&mut self, t: u32, loader: u32, depth: u32) -> Rc<Vals> {
        // The loader holds the module object: a user of its exports without an edge.
        self.loads.insert((loader, t));
        let asked = self.asked_exports.entry(t).or_default();
        if !asked.contains(&loader) {
            asked.push(loader);
        }
        if let Some(m) = self.modules.get(&(t, loader)) {
            return m.clone();
        }
        let p = Prov {
            file: t,
            line: 0,
            depth: depth.saturating_add(1),
            home: false,
        };
        let opaque = |what: &str| {
            Rc::new(vec![Alt {
                v: Val::Js(unknown(Tag::Import, what)),
                p,
            }])
        };
        let Some(facts) = self.facts(t) else {
            return opaque("a module without facts");
        };
        if self.module_busy.contains(&t) || self.module_busy.len() >= 8 {
            return opaque("a module that loads itself");
        }
        self.module_busy.push(t);
        let mut out = Vals::new();
        let mut props: Vec<(String, Vals)> = Vec::new();
        for e in &facts.exports {
            let vals = self.eval(t, &e.value, p, 0);
            if e.name == "module.exports" {
                vals.into_iter().for_each(|a| push(&mut out, a));
            } else if let Some(slot) = props.iter_mut().find(|(k, _)| *k == e.name) {
                vals.into_iter().for_each(|a| push(&mut slot.1, a));
            } else {
                props.push((e.name.clone(), vals));
            }
        }
        self.module_busy.pop();
        let whole = !out.is_empty();
        // Exports the facts do not list: re-exports, and exports written in a way that
        // is not followed.
        let open = (!facts.reexports.is_empty() || facts.exports_open).then(|| {
            vec![Alt {
                v: Val::Js(unknown(Tag::Import, "an export that is not followed")),
                p,
            }]
        });
        if !props.is_empty() || open.is_some() || !whole {
            push(
                &mut out,
                Alt {
                    v: Val::Object(Rc::new(Obj {
                        props,
                        open,
                        complete: false,
                    })),
                    p,
                },
            );
        }
        for g in self.holders(t, loader) {
            push(
                &mut out,
                Alt {
                    v: Val::Js(unknown(Tag::Import, "exports that this file can change")),
                    p: Prov {
                        file: g,
                        line: 0,
                        depth: depth.saturating_add(1),
                        home: false,
                    },
                },
            );
        }
        let out = Rc::new(out);
        if self.steps > 0 {
            self.modules.insert((t, loader), out.clone());
        }
        out
    }

    /// The files other than `loader` that hold the exports of the module `t`
    /// and can change them: an importer that does more than read and call
    /// them, re-exports them, has no facts, or loads the module in a way
    /// the facts do not describe.
    fn holders(&mut self, t: u32, loader: u32) -> Vec<u32> {
        let mut out: Vec<u32> = Vec::new();
        let mut imps = self.w.importers(t);
        if let Some(ls) = self.loaders.get(&t) {
            imps.extend(ls.iter().map(|l| Importer { file: *l, at: None }));
        }
        let files: Vec<u32> = imps.iter().map(|i| i.file).collect();
        self.w.prefetch(&files);
        for imp in imps {
            let g = imp.file;
            if g == loader || g == t || out.contains(&g) || !self.w.is_code(g) {
                continue;
            }
            let reads_only = match (imp.at, self.facts(g)) {
                (Some(at), Some(gf)) => {
                    let mut described = false;
                    let mut writes = false;
                    for i in gf.imports.iter().filter(|i| i.at == at) {
                        described = true;
                        writes |= i.writes;
                    }
                    // A re-export hands the exports to files this check does not see.
                    writes |= gf.reexports.iter().any(|r| r.at == at);
                    described && !writes
                }
                _ => false,
            };
            if !reads_only {
                out.push(g);
            }
        }
        out
    }

    /// Whether one of the values of `callee` (an expression of `file`) can
    /// change the object it is handed at `pos`, as far as a read of `prop`
    /// is concerned. A callee that is not a function with facts can.
    fn callee_changes(
        &mut self,
        file: u32,
        callee: &X,
        pos: u32,
        prop: Option<&str>,
        here: Prov,
        depth: u32,
    ) -> bool {
        !self
            .changers(file, callee, pos, prop, here, depth)
            .is_empty()
    }

    /// The values of the callee `callee` of `file`, each with whoever supplies
    /// it.
    ///
    /// The result of a call as the callee (`xs.map(merge(spec))`): the function
    /// that is called decides what the callee is, so what cannot be said
    /// about the result (a function without a return summary, a result that
    /// is not a function) belongs to whoever supplied that function, like
    /// the function itself.
    fn callee_values(&mut self, file: u32, callee: &X, here: Prov, depth: u32) -> Vals {
        match callee {
            X::Alts(xs) => {
                let mut out = Vals::new();
                for x in xs {
                    for v in self.callee_values(file, x, here, depth) {
                        push(&mut out, v);
                    }
                }
                out
            }
            X::Call {
                callee: inner,
                args,
                rest,
            } if !matches!(&**inner, X::Import { module, .. } if module == REQUIRE) => {
                let mut out = Vals::new();
                for f in self.callee_values(file, inner, here, depth) {
                    let owner = if f.p.depth > here.depth || f.p.file == NOWHERE {
                        f.p
                    } else {
                        here
                    };
                    let mut results = Vals::new();
                    self.call_value(
                        file,
                        &f,
                        args,
                        rest.as_deref(),
                        &[],
                        here,
                        depth + 1,
                        &mut results,
                    );
                    for mut r in results {
                        if r.p.depth <= here.depth && r.p.file != NOWHERE {
                            r.p = owner;
                        }
                        push(&mut out, r);
                    }
                }
                out
            }
            _ => self.eval(file, callee, here, depth),
        }
    }

    /// The suppliers of the values of `callee` that can change the object they
    /// are handed at `pos` (see [`Binder::callee_changes`]).
    fn changers(
        &mut self,
        file: u32,
        callee: &X,
        pos: u32,
        prop: Option<&str>,
        here: Prov,
        depth: u32,
    ) -> Vec<Prov> {
        let mut out: Vec<Prov> = Vec::new();
        let callees = self.callee_values(file, callee, here, depth);
        for c in callees {
            let changes = match c.v {
                Val::Func(tf, id) => self.changes(tf, id, pos, prop),
                _ => {
                    if self.prof.is_some() {
                        eprintln!(
                            "bind changes: a callee in {} is not a function with facts (supplied \
                             by {})",
                            self.w.path(file),
                            if c.p.file == NOWHERE {
                                "nowhere".to_owned()
                            } else {
                                self.w.path(c.p.file)
                            }
                        );
                    }
                    true
                }
            };
            // A callee the reading file knows by itself (an import) changes the object
            // wherever the read happens; one a caller supplies only where that caller is.
            let p = if c.p.depth > here.depth || c.p.file == NOWHERE {
                c.p
            } else {
                here
            };
            if changes && !out.contains(&p) {
                out.push(p);
            }
        }
        out
    }

    /// Whether the function `id` of `file` can change the object it gets as
    /// parameter `pos` ([`RECEIVER`]: as `this`), as far as a read of
    /// `prop` is concerned: its file assigns such a property somewhere, or
    /// it hands the object to a callee that can.
    fn changes(&mut self, file: u32, id: u32, pos: u32, prop: Option<&str>) -> bool {
        let key = (file, id, pos, prop.map(str::to_owned));
        if let Some(b) = self.changes.get(&key) {
            return *b;
        }
        // A cycle of calls: it changes the object only if one of its members does.
        if self.changes_busy.contains(&key) {
            return false;
        }
        if self.changes_busy.len() >= 16 || !self.tick() {
            return true;
        }
        let debug = self.prof.is_some();
        let Some(facts) = self.facts(file) else {
            if debug {
                eprintln!("bind changes: {} has no facts", self.w.path(file));
            }
            return true;
        };
        let Some(f) = facts.func(id) else {
            if debug {
                eprintln!(
                    "bind changes: {}: function at {id} is not described",
                    self.w.path(file)
                );
            }
            return true;
        };
        let mut r = f.uses_arguments
            || facts.writes.hits(prop)
            // Writes through `this` are not among the file's named writes.
            || (pos == RECEIVER && f.uses_this);
        if r && debug {
            eprintln!(
                "bind changes: {} {} #{pos} read {:?}: arguments {} file writes {} this {}",
                self.w.path(file),
                f.name,
                prop.map(|p| p.trim_start_matches('\0')),
                f.uses_arguments,
                facts.writes.hits(prop),
                pos == RECEIVER && f.uses_this
            );
        }
        if !r && pos != RECEIVER {
            let index = match f.rest {
                Some(rest) if pos >= rest => rest,
                _ => pos,
            };
            self.changes_busy.push(key.clone());
            let here = Prov {
                file,
                line: 0,
                depth: 0,
                home: false,
            };
            'handoffs: for h in f.handoffs.iter().filter(|h| h.param == index) {
                if h.unknown {
                    r = true;
                    break;
                }
                for (callee, p2) in &h.to {
                    if self.callee_changes(file, callee, *p2, prop, here, 0) {
                        r = true;
                        break 'handoffs;
                    }
                }
            }
            self.changes_busy.pop();
        }
        if self.steps > 0 && self.changes_busy.is_empty() {
            self.changes.insert(key, r);
        }
        r
    }

    /// `a.name` (`None`: any property).
    fn member(&self, a: &Alt, name: Option<&str>, out: &mut Vals) {
        // As the receiver of an array method: an array is itself; a string, a function
        // and an object without such a method have none (the call throws).
        if name == Some(AS_ARRAY) || name == Some(ARRAY_ELEMENT) {
            match &a.v {
                Val::Array(_) if name == Some(ARRAY_ELEMENT) => self.elem(a, out),
                Val::Array(_) => push(out, a.clone()),
                Val::Js(j @ JsVal::Unknown(_)) => push(out, derived(j, "elements", a.p)),
                Val::Object(o)
                    if o.open.is_some()
                        || o.props.iter().any(|(k, _)| {
                            matches!(
                                k.as_str(),
                                "forEach"
                                    | "map"
                                    | "filter"
                                    | "find"
                                    | "findLast"
                                    | "findIndex"
                                    | "findLastIndex"
                                    | "some"
                                    | "every"
                                    | "flatMap"
                            )
                        }) =>
                {
                    push(
                        out,
                        Alt {
                            v: Val::Js(unknown(
                                Tag::Unknown,
                                "what a method of the object hands out",
                            )),
                            p: a.p,
                        },
                    )
                }
                _ => {}
            }
            return;
        }
        match &a.v {
            Val::Object(o) => {
                let mut known = false;
                match name {
                    Some(n) => {
                        if let Some((_, v)) = o.props.iter().rev().find(|(k, _)| k == n) {
                            v.iter().for_each(|x| push(out, x.clone()));
                            known = o.complete;
                        }
                    }
                    None => {
                        for (_, v) in &o.props {
                            v.iter().for_each(|x| push(out, x.clone()));
                        }
                    }
                }
                if let (Some(open), false) = (&o.open, known) {
                    open.iter().for_each(|x| push(out, x.clone()));
                }
            }
            Val::Array(arr) => match name.map(str::parse::<usize>) {
                Some(Ok(i)) => {
                    for e in arr.elems.get(i).into_iter().chain(arr.rest.iter()) {
                        e.iter().for_each(|x| push(out, x.clone()));
                    }
                }
                Some(Err(_)) => {}
                None => {
                    for e in arr.elems.iter().chain(arr.rest.iter()) {
                        e.iter().for_each(|x| push(out, x.clone()));
                    }
                }
            },
            Val::Js(j @ JsVal::Unknown(_)) => push(out, derived(j, "property", a.p)),
            // A property of a known string: its length or a character (a piece of it);
            // any other name is a method or nothing.
            Val::Js(JsVal::Str(Value::Str(_))) => {
                if name.is_none_or(|n| n == "length" || n.parse::<usize>().is_ok()) {
                    push(out, fragment(a.p));
                }
            }
            // A property of another string-like value or of a function: unknown at the
            // supplier.
            Val::Js(_) | Val::Func(..) => push(
                out,
                Alt {
                    v: Val::Js(unknown(Tag::Unknown, "property")),
                    p: a.p,
                },
            ),
        }
    }

    fn apply(&mut self, f: Func, args: &[Vals], here: Prov, out: &mut Vals) {
        if too_many(args.iter().map(Vec::len)) {
            push(out, gave_up());
            return;
        }
        if args.iter().any(Vec::is_empty) {
            // An argument that is never a string: the call throws.
            return;
        }
        let consts = HashMap::new();
        let sc = scope(&consts);
        let mut idx = vec![0usize; args.len()];
        'combos: loop {
            if !self.tick() {
                push(out, gave_up());
                return;
            }
            let combo: Vec<JsVal> = args
                .iter()
                .zip(&idx)
                .map(|(a, &i)| operand(&a[i].v, true))
                .collect();
            let p = deepest(args.iter().zip(&idx).map(|(a, &i)| a[i].p), here);
            let j = match evaluator::apply(f, &combo, &sc) {
                Some(j) => j,
                None => match combo.iter().find(|j| matches!(j, JsVal::Unknown(_))) {
                    Some(JsVal::Unknown(r)) => JsVal::Unknown(origin::derive(r, "path")),
                    _ => unknown(Tag::Unknown, "path"),
                },
            };
            for leaf in evaluator::split_alts(j) {
                push(
                    out,
                    Alt {
                        v: Val::Js(leaf),
                        p,
                    },
                );
            }
            let mut k = 0;
            loop {
                if k == idx.len() {
                    break 'combos;
                }
                idx[k] += 1;
                if idx[k] < args[k].len() {
                    break;
                }
                idx[k] = 0;
                k += 1;
            }
        }
    }

    // ---- exports ----

    /// The value of the export path `name` of the module `spec` as imported by
    /// `file`.
    fn imported(&mut self, file: u32, spec: &str, name: &str, here: Prov, depth: u32) -> Vals {
        let mut out = Vals::new();
        let unresolved = |out: &mut Vals| {
            push(
                out,
                Alt {
                    v: Val::Js(JsVal::Unknown(origin::mark(
                        Tag::Import,
                        &format!("{spec}#{name}"),
                        name,
                    ))),
                    p: here,
                },
            )
        };
        let Some(facts) = self.facts(file) else {
            unresolved(&mut out);
            return out;
        };
        let mut targets: Vec<u32> = Vec::new();
        for i in facts.imports.iter().filter(|i| i.spec == spec) {
            for t in self.w.targets(file, i.at) {
                if !targets.contains(&t) {
                    targets.push(t);
                }
            }
        }
        if targets.is_empty() {
            unresolved(&mut out);
            return out;
        }
        for t in targets {
            let Some(defs) = self.export_values(t, name, 0) else {
                unresolved(&mut out);
                continue;
            };
            for (f, x) in defs {
                let p = Prov {
                    file: f,
                    line: 0,
                    depth: here.depth,
                    home: false,
                };
                for a in self.eval(f, &x, p, depth + 1) {
                    // A constant of another module: the importer supplies it.
                    let a = match a.v {
                        Val::Js(_) if a.p.depth <= here.depth => Alt { v: a.v, p: here },
                        _ => a,
                    };
                    push(&mut out, a);
                }
            }
        }
        out
    }

    /// The files and expressions that define the export `name` of `file`,
    /// through re-exports: every assignment of a CommonJS export, the
    /// explicit re-export if there is one (it wins over `export *`), else
    /// every `export *` that has the name. `None`: not found, or not
    /// followed to the end.
    fn export_values(&mut self, file: u32, name: &str, hops: u32) -> Option<Vec<(u32, X)>> {
        if hops > MAX_HOPS || !self.tick() {
            return None;
        }
        let facts = self.facts(file)?;
        let own: Vec<(u32, X)> = facts
            .exports
            .iter()
            .filter(|e| e.name == name)
            .map(|e| (file, e.value.clone()))
            .collect();
        if !own.is_empty() {
            return Some(own);
        }
        let mut out: Vec<(u32, X)> = Vec::new();
        for explicit in [true, false] {
            for r in &facts.reexports {
                let inner = match r.names.iter().find(|(_, e)| e == name) {
                    Some((i, _)) if i != "*" => explicit.then_some(i.as_str()),
                    // The namespace of the module under this name: not a value here.
                    Some(_) if explicit => return None,
                    Some(_) => None,
                    None if r.star && name != "default" && !explicit => Some(name),
                    None => None,
                };
                let Some(inner) = inner else { continue };
                for t in self.w.targets(file, r.at) {
                    match self.export_values(t, inner, hops + 1) {
                        Some(v) => out.extend(v),
                        // A named re-export of something that is not found there.
                        None if explicit => return None,
                        None => {}
                    }
                }
            }
            if !out.is_empty() {
                return Some(out);
            }
        }
        None
    }

    /// The names `file` exports the function `func` under.
    fn export_names(facts: &FileFacts, func: u32) -> Vec<String> {
        fn has(x: &X, func: u32) -> bool {
            match x {
                X::Func(f) => *f == func,
                X::Alts(xs) => xs.iter().any(|x| has(x, func)),
                _ => false,
            }
        }
        facts
            .exports
            .iter()
            .filter(|e| has(&e.value, func))
            .map(|e| e.name.clone())
            .collect()
    }

    /// How the files of the graph use the export `name` of `file`.
    fn users(&mut self, file: u32, name: &str, hops: u32) -> Rc<Vec<Use>> {
        let key = (file, name.to_owned());
        self.asked_users.insert(file);
        if let Some(u) = self.users.get(&key) {
            return u.clone();
        }
        // A cycle of re-exports ends here: what is collected between the first visit of
        // this export and now lacks its users (and is not kept, see below).
        if let Some(i) = self.users_busy.iter().position(|k| *k == key) {
            self.users_hit = self.users_hit.min(i);
            return Rc::new(Vec::new());
        }
        let my = self.users_busy.len();
        self.users_busy.push(key.clone());
        let hit_before = std::mem::replace(&mut self.users_hit, usize::MAX);
        // A set: barrels that re-export each other reach the same user along many
        // paths.
        let mut out: Vec<Use> = Vec::new();
        let mut seen: std::collections::HashSet<Use> = std::collections::HashSet::new();
        let mut add = |out: &mut Vec<Use>, u: Use| {
            if seen.insert(u.clone()) {
                out.push(u);
            }
        };
        let mut importers = self.w.importers(file);
        // Files that load the module through a bound site hold all of its exports.
        if let Some(ls) = self.loaders.get(&file) {
            importers.extend(ls.iter().map(|l| Importer { file: *l, at: None }));
        }
        let files: Vec<u32> = importers.iter().map(|i| i.file).collect();
        self.w.prefetch(&files);
        for imp in importers {
            let g = imp.file;
            let (Some(at), Some(gf)) = (imp.at, self.facts(g)) else {
                add(&mut out, Use::Opaque(g));
                continue;
            };
            let mut described = false;
            for i in gf.imports.iter().filter(|i| i.at == at) {
                described = true;
                let escaped = i.escapes.iter().any(|p| {
                    p == "*"
                        || p == name
                        || p.strip_prefix(name).is_some_and(|r| r.starts_with('.'))
                        || name
                            .strip_prefix(p.as_str())
                            .is_some_and(|r| r.starts_with('.'))
                });
                if escaped {
                    add(&mut out, Use::Opaque(g));
                }
                for (ci, c) in gf.calls.iter().enumerate() {
                    match calls_import(&c.callee, &i.spec, name) {
                        Some(true) => add(&mut out, Use::Call(g, ci)),
                        // `f.call(...)`, `f.bind(...)`, a method of the export.
                        Some(false) => add(&mut out, Use::Opaque(g)),
                        None => {}
                    }
                }
            }
            for r in gf.reexports.iter().filter(|r| r.at == at) {
                described = true;
                let mut outer: Vec<&str> = r
                    .names
                    .iter()
                    .filter(|(i, _)| i == name)
                    .map(|(_, e)| e.as_str())
                    .collect();
                if r.star && name != "default" {
                    outer.push(name);
                }
                if r.names.iter().any(|(i, _)| i == "*") || hops >= MAX_HOPS {
                    // The namespace as an export, or a chain too long to follow.
                    add(&mut out, Use::Opaque(g));
                    continue;
                }
                for o in outer {
                    let o = o.to_owned();
                    let more = self.users(g, &o, hops + 1);
                    for u in more.iter() {
                        add(&mut out, u.clone());
                    }
                }
            }
            if !described {
                add(&mut out, Use::Opaque(g));
            }
        }
        let out = Rc::new(out);
        self.users_busy.pop();
        // Complete unless a lookup inside ran into an export further out that is still
        // being collected.
        let hit = self.users_hit;
        self.users_hit = hit_before.min(hit);
        if hit >= my {
            self.users.insert(key, out.clone());
        }
        out
    }

    // ---- holes ----

    /// What the callers pass as parameter `index` of the function `func` of
    /// `file`.
    fn hole(&mut self, file: u32, func: u32, index: u32, path: &[Step], here: Prov) -> Rc<Vals> {
        let key = (file, func, index);
        // A call being evaluated through its return summary: its own arguments.
        if let Some(pos) = self
            .frames
            .iter()
            .rposition(|f| f.file == file && f.func == func)
        {
            let fr = &self.frames[pos];
            let seen = (index, path.to_vec());
            if let Some(v) = fr.seen.get(&seen) {
                return v.clone();
            }
            let arg = match (fr.args.get(index as usize), &fr.rest) {
                (Some(a), _) => Some(a.clone()),
                (None, Some(r)) => Some(r.clone()),
                // A missing argument is `undefined`: no value.
                (None, None) => None,
            };
            let (caller, at) = (fr.caller, fr.here);
            let vals = match arg {
                // In the caller, where the caller's own holes are the frames below.
                Some(Arg::Lazy(x)) => self.eval_read(caller, &x, path, at, 0),
                Some(Arg::Vals(v)) => self.project((*v).clone(), path),
                None => Vals::new(),
            };
            let vals = Rc::new(vals);
            if self.steps > 0
                && let Some(fr) = self.frames.get_mut(pos)
                && fr.file == file
                && fr.func == func
            {
                fr.seen.insert(seen, vals.clone());
            }
            return vals;
        }
        let memo = (file, func, index, path.to_vec());
        if let Some(v) = self.holes.get(&memo) {
            return v.clone();
        }
        let home = |line: u32| Alt {
            v: Val::Js(JsVal::Unknown(origin::mark(
                Tag::Param,
                &format!("#{index}"),
                "parameter",
            ))),
            p: Prov {
                file,
                line,
                depth: here.depth,
                home: true,
            },
        };
        if self.busy.contains(&key) || self.busy.len() as u32 >= MAX_DEPTH || !self.tick() {
            // Recursion through holes, or too deep: unknown at the callee.
            return Rc::new(vec![home(0)]);
        }
        let Some(facts) = self.facts(file) else {
            return Rc::new(vec![home(0)]);
        };
        let Some(f) = facts.func(func) else {
            return Rc::new(vec![home(0)]);
        };
        self.busy.push(key);
        // The callers are other activations: the frames of calls being evaluated
        // through their return summary do not apply to them.
        let frames = std::mem::take(&mut self.frames);
        let mut out = Vals::new();
        let depth = here.depth + 1;
        match &f.source {
            Source::Named { escapes } | Source::Ctor { escapes } => {
                if *escapes {
                    push(&mut out, home(f.line));
                }
                // Calls in the file that the engine does not join: `new C(...)`.
                for c in &facts.calls {
                    if matches!(&c.callee, X::Func(id) if *id == func) {
                        let p = Prov {
                            file,
                            line: c.line,
                            depth,
                            home: false,
                        };
                        let mut vals = Vals::new();
                        self.argument(file, &c.args, c.rest.as_ref(), index, f.rest, p, &mut vals);
                        for a in self.project(vals, path) {
                            push(&mut out, a);
                        }
                    }
                }
                for name in Self::export_names(&facts, func) {
                    let users = self.users(file, &name, 0);
                    for u in users.iter() {
                        match u {
                            Use::Call(g, ci) => {
                                let Some(gf) = self.facts(*g) else { continue };
                                let Some(c) = gf.calls.get(*ci) else { continue };
                                let p = Prov {
                                    file: *g,
                                    line: c.line,
                                    depth,
                                    home: false,
                                };
                                let mut vals = Vals::new();
                                self.argument(
                                    *g,
                                    &c.args,
                                    c.rest.as_ref(),
                                    index,
                                    f.rest,
                                    p,
                                    &mut vals,
                                );
                                for a in self.project(vals, path) {
                                    push(&mut out, a);
                                }
                            }
                            Use::Opaque(g) => push(
                                &mut out,
                                Alt {
                                    v: Val::Js(JsVal::Unknown(origin::mark(
                                        Tag::Param,
                                        &format!("{}#{index}", f.name),
                                        "callers not known",
                                    ))),
                                    p: Prov {
                                        file: *g,
                                        line: 0,
                                        depth,
                                        home: false,
                                    },
                                },
                            ),
                        }
                    }
                }
            }
            // A callback's parameters are bound by the callee of the call it is passed
            // to, a method's by whoever holds the object: not followed.
            Source::Callback { .. } | Source::Unseen => push(&mut out, home(f.line)),
        }
        self.busy.pop();
        self.frames = frames;
        let out = Rc::new(out);
        // A value computed while a caller's hole was still open may be incomplete.
        if self.steps > 0 && (self.busy.is_empty() || !out.iter().any(|a| a.p.home)) {
            self.holes.insert(memo, out.clone());
        }
        out
    }

    /// The value of parameter `index` at a call with `args` in `file`.
    #[allow(clippy::too_many_arguments)]
    fn argument(
        &mut self,
        file: u32,
        args: &[X],
        rest: Option<&X>,
        index: u32,
        rest_param: Option<u32>,
        p: Prov,
        out: &mut Vals,
    ) {
        let i = index as usize;
        if rest_param == Some(index) {
            // `...args`: the array of the remaining arguments.
            let elems = args
                .iter()
                .skip(i)
                .map(|a| self.eval(file, a, p, 0))
                .collect();
            let rest = rest.map(|r| self.eval(file, r, p, 0));
            push(
                out,
                Alt {
                    v: Val::Array(Rc::new(Arr { elems, rest })),
                    p,
                },
            );
            return;
        }
        if let Some(a) = args.get(i) {
            for v in self.eval(file, a, p, 0) {
                push(out, v);
            }
        } else if let Some(r) = rest {
            for v in self.eval(file, r, p, 0) {
                push(out, v);
            }
        }
        // A missing argument is `undefined`: no value.
    }
}

/// Whether the callee `x` is the export path `name` of the module imported as
/// `spec` (`Some(true)`), or a member of it (`Some(false)`).
fn calls_import(x: &X, spec: &str, name: &str) -> Option<bool> {
    match x {
        X::Import { module, name: n } if module == spec => {
            if n == name {
                Some(true)
            } else if n.strip_prefix(name).is_some_and(|r| r.starts_with('.')) {
                Some(false)
            } else {
                None
            }
        }
        X::Alts(xs) => xs
            .iter()
            .filter_map(|x| calls_import(x, spec, name))
            .reduce(|a, b| a && b),
        _ => None,
    }
}

/// The deepest provenance: the supplier of a combined value.
fn deepest(ps: impl IntoIterator<Item = Prov>, here: Prov) -> Prov {
    let mut best = here;
    for p in ps {
        if p.depth > best.depth || (p.depth == best.depth && p.home && !best.home) {
            best = p;
        }
    }
    best
}

/// An unknown derived from the unknown `j`.
fn derived(j: &JsVal, what: &str, p: Prov) -> Alt {
    let v = match j {
        JsVal::Unknown(r) => JsVal::Unknown(origin::derive(r, what)),
        _ => unknown(Tag::Unknown, what),
    };
    Alt { v: Val::Js(v), p }
}

/// A value as an operand of `+` or a path function.
fn operand(v: &Val, left: bool) -> JsVal {
    let reason = || origin::mark(Tag::Unknown, "", "not a string");
    match v {
        Val::Js(j) if left => evaluator::stringy(j.clone(), reason),
        Val::Js(j) => evaluator::stringy_right(j.clone(), reason),
        _ => JsVal::Unknown(reason()),
    }
}

/// The string values of the tables `x` reads by key.
fn table_values(x: &X, out: &mut Vec<String>) {
    match x {
        X::Index { table, .. } => strings(table, out),
        X::Alts(xs) | X::Apply(_, xs) | X::Opaque(xs) => {
            xs.iter().for_each(|x| table_values(x, out))
        }
        X::Concat(a, b) => {
            table_values(a, out);
            table_values(b, out);
        }
        X::Field(b, _) | X::Elem(b) | X::AnyField(b) => table_values(b, out),
        _ => {}
    }
}

fn strings(x: &X, out: &mut Vec<String>) {
    match x {
        X::Const(JsVal::Str(Value::Str(s))) => {
            if !out.contains(s) {
                out.push(s.clone());
            }
        }
        X::Const(JsVal::Alts(vs)) => {
            for v in vs {
                strings(&X::Const(v.clone()), out);
            }
        }
        X::Object { props, .. } => props.iter().for_each(|(_, v)| strings(v, out)),
        X::Alts(xs) => xs.iter().for_each(|x| strings(x, out)),
        _ => {}
    }
}

/// A bound value as a dependency of a site with base rule `rule`. `None`: never
/// a file (the empty string) or outside the repository.
fn lower(j: &JsVal, rule: BaseRule) -> Option<DepValue> {
    let open = |r: &str| {
        let (origin, payload) = origin::combine([r]);
        let text = origin::strip(r);
        Some(DepValue::Open {
            origin,
            detail: if payload.is_empty() {
                text
            } else {
                format!("{text}; {payload}")
            },
        })
    };
    let reasons = |v: &Value| {
        let mut rs = Vec::new();
        evaluator::wild_reasons(v, &mut rs);
        rs.join(" ")
    };
    match j {
        JsVal::Unknown(r) if r.starts_with("@unknown[cwd-per-test]") => {
            Some(DepValue::PerTestCwd(origin::strip(r)))
        }
        JsVal::Unknown(r) => open(r),
        JsVal::Path(Anchor::Tmp, _) | JsVal::Url(Anchor::Tmp, _) => None,
        JsVal::Path(Anchor::Cwd, v) | JsVal::Url(Anchor::Cwd, v) => {
            match evaluator::normalize_value(v.clone(), true) {
                Value::Str(s) => Some(DepValue::Cwd(s)),
                v => open(&reasons(&v)),
            }
        }
        JsVal::Str(v) => {
            let v = match v {
                Value::Str(s) if s.is_empty() => return None,
                v => v.clone(),
            };
            let abs = turborepo_trace_core::eval::path::starts_absolute(&v);
            if abs {
                let v = evaluator::normalize_value(v, false);
                return match &v {
                    Value::Str(s) => match s.strip_prefix(ROOT) {
                        Some("") => Some(DepValue::Path(String::new())),
                        Some(rest) if rest.starts_with('/') => {
                            Some(DepValue::Path(rest[1..].to_owned()))
                        }
                        // Another absolute path: outside the repository.
                        _ => None,
                    },
                    Value::Pattern(parts) => match parts.first() {
                        Some(PatPart::Lit(head)) => match head.strip_prefix(ROOT) {
                            Some(rest) if rest.is_empty() || rest.starts_with('/') => {
                                // The known directory before the first unknown part.
                                let rest = rest.trim_start_matches('/');
                                let dir = match rest.rfind('/') {
                                    Some(i) => &rest[..i],
                                    None => "",
                                };
                                Some(DepValue::Below(dir.to_owned()))
                            }
                            _ => None,
                        },
                        _ => open(&reasons(&v)),
                    },
                    v => open(&reasons(v)),
                };
            }
            match (&v, rule) {
                (Value::Str(s), BaseRule::Module) => Some(DepValue::Spec(s.clone())),
                (Value::Str(s), _) => Some(DepValue::Cwd(s.clone())),
                (v, _) => open(&reasons(v)),
            }
        }
        _ => Some(DepValue::Open {
            origin: Origin::Unknown,
            detail: "not a string".to_owned(),
        }),
    }
}
