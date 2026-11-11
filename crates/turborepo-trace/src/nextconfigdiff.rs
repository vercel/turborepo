//! What a `next/jest` test can observe of its Next.js config: the file, its
//! fields and the modules it loads.
//!
//! `next/jest` loads `next.config.*` in the jest process before it returns the
//! jest config (`next/src/build/jest/jest.ts`), and hands `transpilePackages`,
//! `modularizeImports`, `compiler` and `experimental.swcPlugins` to the
//! transform every test file goes through. So the config is an input of every
//! jest test of the project (`crates/turborepo-trace-frameworks/src/jest.rs`
//! records the file as a content input), and so is everything the config's
//! process loads while the module runs: a module that the config imports can
//! compute one of those fields, break the load, or set a variable of the
//! process the tests run in.
//!
//! One table says what counts. It is the same for an edit of the config and for
//! a change to something the config loads:
//!
//! | Part of the config | Can a `next/jest` test observe it? |
//! |---|---|
//! | `transpilePackages`, `modularizeImports`, `compiler`, `experimental.swcPlugins` | yes: options of the transform |
//! | the value of any other field | yes: loading validates the config (an invalid `basePath` or `i18n` throws, and then every test fails), so values are not compared |
//! | code of the module's top level, of any function it calls there | yes |
//! | the bodies of `rewrites()`, `redirects()` and `headers()` | no: Next.js calls them when it builds its routes (`next build`, `next dev`, the server); loading the config for the test phase does not |
//! | a module the config loads (an import, a `require`, a file it reads) outside those bodies | yes, with everything that module loads in turn |
//! | a load written inside one of those bodies | no: it never runs |
//! | an imported module whose bindings are used only inside those bodies | only through what loading it does: no, when the module and every module it loads at its top level only define things; yes otherwise |
//! | a package the config imports | yes ([`crate::configdiff::config_imports`]): what loading a package does is not known |
//!
//! # An edit of the config: [`effects`]
//!
//! When the base revision and the working tree differ only *inside the bodies*
//! of route functions, the module does the same at load time, the loaded config
//! has the same values, and `next/jest` returns the same jest config. The edit
//! is then not a change of the tests' content input. [`route_rules_only`]
//! decides that with [`crate::jsblank`]:
//!
//! 1. both revisions parse without syntax or early errors (a syntax error in a
//!    body breaks the load, and with it every test);
//! 2. a *route function* is a function that is the value of a property named
//!    `rewrites`, `redirects` or `headers` in an object literal (a method, a
//!    function expression or an arrow function; not a getter). Nothing else in
//!    the module may be able to call one: outside the bodies, no member access
//!    `.rewrites`, no string `'rewrites'` (a computed access) and no
//!    destructuring `{ rewrites } = ...`;
//! 3. with the bodies blanked, both revisions have the same runtime JavaScript
//!    ([`crate::erase::erased_equal`]: types, comments and formatting don't
//!    count).
//!
//! Everything else the change does stays: the file is seeded as modified (its
//! importers, such as a test of the config itself, and the tasks that read it
//! are affected), and so are the nodes that have it as an input for another
//! reason and the uncertain reads that cover it. Only the jest tests that read
//! it through `next/jest` are left out: tests whose jest config sits next to
//! the file and imports `next/jest`.
//!
//! # A change to what the config loads: [`prepare`]
//!
//! The graph keeps the config a content input of the tests and gives the
//! config's own node the edges to what it loads; no edge leads from a test to
//! those modules. [`prepare`] closes that: for every Next.js config that jest
//! tests read, it evaluates the modules and files the config loads as entries
//! of their own, in one more evaluation with the rules of the
//! tests (no type-only edges, erased-equal files, barrels), and a test is
//! affected when one of them is, with the chain through the config as its
//! witness. The config's node itself is evaluated too: when it is a seed for
//! another reason than its own edit (a file appears where its resolution
//! looked, a path inside a directory it lists changes), or carries an
//! open load, the tests get that verdict. A file read or a directory listing of
//! the config that is relative to the working directory
//! (`path.join(process.cwd(), 'data.json')`) with a literal path is resolved
//! with the working directories of the tests, in whose process the config
//! loads: that path, and what lies below it, is an input, and nothing else is.
//! Any other read relative to the working directory (a pattern, a module load)
//! keeps the engine's verdict for a file with no working directory: uncertain
//! within the package.
//!
//! The entries of that evaluation get what every entry gets after the engine:
//! the names of the code loaded with a read whose path is computed
//! (`crate::names`) and the bound on loads with an unknown target
//! (`crate::loads`) are evaluated for them too, since the engine's options take
//! such reads and loads as bounded by those layers. The layers see the config
//! with all its imports. Their result for a module the config loads counts as
//! it is. Their result for the config itself counts when a file of the config's
//! process (what it reaches without the imports that are left out) holds a read
//! with a computed path, for names, or a load with an unknown target, for
//! loads: when none does, no such read runs while the config loads (the ones in
//! a left-out import sit in functions that only route functions call), and a
//! name has nothing to be the path of. A layer is not run for these entries at
//! all when no config's process holds a site of its kind.
//!
//! The rows of the table narrow it, by proof:
//!
//! - a load whose reference lies inside a route function body is left out;
//! - an `import` declaration (or a top-level `const ... =
//!   require('<literal>')`) is left out for a change when two things hold. (a)
//!   *Its values reach nothing that runs at load*: with the route function
//!   bodies blanked no reference to any of its bindings is left, the module has
//!   no `eval`, and its specifier appears nowhere else in the module. (b)
//!   *Loading it does the same before and after the change.* The imported file
//!   and every file of the repository it loads at its top level, transitively,
//!   is *inert* ([`crate::loadcode`]): its top level calls nothing of the
//!   repository, so no function body of those files runs at load, and a read
//!   that is written inside a function (a directory the function lists) is not
//!   part of the load. Then the load can only differ if the code outside
//!   function bodies of one of those files differs
//!   ([`crate::loadcode::same_load`], or both revisions are side-effect free
//!   modules that load the same modules), if one of them is added or deleted,
//!   resolves differently (a changed manifest or tsconfig it resolved through,
//!   a file that appears where its resolution looked), or loads a package that
//!   changed. Each of these makes the import count for that change; without a
//!   base revision a modified file counts.
//!
//! No value flow is followed beyond that: an import that the top level so much
//! as names (an alias, an argument of a wrapper) counts.
//!
//! # The premise both parts share
//!
//! That nothing calls a route function while the config loads. Inside the
//! module this is checked (2). A package the config hands its object to
//! (`withPlugin(config)`) could call them; the ones in use wrap them in
//! functions of their own, which Next.js calls later. That is an assumption
//! about those packages: the notes of a change say when it was used, `--strict`
//! does not make it (every edit of the config and every module it loads
//! counts), and `TURBO_TRACE_DEBUG_CONFIG_FIELDS=0` (edits) and
//! `TURBO_TRACE_DEBUG_CONFIG_IMPORTS=all` (loads) turn the narrowing off.
//! `TURBO_TRACE_DEBUG_CONFIG_IMPORTS=0` turns [`prepare`] off altogether: the
//! reading before it existed, for A/B measurements.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::Path,
};

use turborepo_trace_graph::{
    AffectedOptions, AffectedReport, Cause, Change, ChangeKind, ChangeSet, Engine, EntryResult,
    GraphView, NodeId, NodeSet, RefFlags, RefKind, UncertainSite, UncertaintyKind, Verdict,
};
use turborepo_trace_lang_js::{Dialect, extract::exports::scan_shape};
use turborepo_trace_store::Graph;

use crate::lockdiff::Git;

/// The graph the CLI's engine runs on (node ids are the store graph's).
type View<'g> = turborepo_trace_graph::Patched<'g, Graph>;

/// The functions of a Next.js config that only route building calls.
const ROUTE_FUNCTIONS: &[&str] = &["rewrites", "redirects", "headers"];

/// `next.config` file names.
const CONFIG_NAMES: &[&str] = &[
    "next.config.js",
    "next.config.mjs",
    "next.config.cjs",
    "next.config.ts",
    "next.config.mts",
    "next.config.cts",
];

/// What the route-rule edits of `next.config.*` files change in an `affected`
/// query.
#[derive(Debug, Default)]
pub struct Effects {
    /// File changes that are applied through `seeds` instead.
    pub drop: BTreeSet<String>,
    /// The nodes to seed, each with its cause.
    pub seeds: Vec<(NodeId, Cause)>,
    /// Readable notes for `--stats`.
    pub notes: Vec<String>,
}

/// Whether the base and head revisions of the Next.js config at `path` differ
/// only inside the bodies of route functions (see the module docs): the names
/// of the route functions of the head revision; the error says why the change
/// is not of that kind.
pub fn route_rules_only(path: &str, base: &str, head: &str) -> Result<Vec<&'static str>, String> {
    let sites = crate::jsblank::Sites {
        props: ROUTE_FUNCTIONS,
        what: crate::jsblank::What::FunctionBodies,
        wrapper: None,
    };
    let (_, h) = crate::jsblank::equal_outside(path, base, head, sites)?;
    Ok(h.names.into_iter().collect())
}

fn dir_of(path: &str) -> &str {
    path.rfind('/').map_or("", |i| &path[..i])
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Among the nodes that have the Next.js config at `path` as an input: the jest
/// tests that read it through `next/jest` (their jest config is in the config's
/// directory and imports `next/jest`), sorted.
fn next_jest_tests(engine: &Engine<'_, View<'_>>, path: &str) -> Vec<NodeId> {
    let view = engine.graph();
    let g = view.base();
    let dir = dir_of(path);
    // jest config node → whether it imports `next/jest` and sits next to the Next
    // config.
    let mut configs: HashMap<NodeId, bool> = HashMap::new();
    let mut out = Vec::new();
    engine.match_inputs(path, &mut |n| {
        if turborepo_trace_store::tasks::is_pseudo_lang(g.lang(n)) {
            return;
        }
        let through = view.out_edges(n).any(|e| {
            let (Some(spec), Some(to)) = (e.specifier, e.to) else {
                return false;
            };
            spec.starts_with("jest:config ")
                && *configs.entry(to).or_insert_with(|| {
                    dir_of(g.path(to)) == dir
                        && view
                            .out_edges(to)
                            .any(|e| matches!(e.specifier, Some("next/jest" | "next/jest.js")))
                })
        });
        if through {
            out.push(n);
        }
    });
    out.sort_unstable();
    out.dedup();
    out
}

/// Applies the modified `next.config.*` files of `changes` whose edits are
/// route rules only (see the module docs). `skip` names changes that are
/// handled elsewhere. With `strict` every edit counts (the premise about
/// wrappers is not assumed).
pub fn effects(
    engine: &Engine<'_, View<'_>>,
    root: &Path,
    changes: &[Change],
    base: Option<&Git>,
    strict: bool,
    skip: &dyn Fn(&str) -> bool,
) -> Effects {
    let mut fx = Effects::default();
    let Some(base) = base.filter(|_| crate::configdiff::enabled() && !strict) else {
        return fx;
    };
    let g = engine.graph().base();
    for c in changes {
        if c.kind != ChangeKind::Modified
            || !CONFIG_NAMES.contains(&file_name(&c.path))
            || skip(&c.path)
        {
            continue;
        }
        let Some(node) = g.node_by_path(&c.path) else {
            continue;
        };
        let tests = next_jest_tests(engine, &c.path);
        if tests.is_empty() {
            continue;
        }
        let (Some(old), Ok(new)) = (
            base.show(&c.path),
            std::fs::read_to_string(root.join(&c.path)),
        ) else {
            continue;
        };
        // The same runtime JavaScript at both revisions: the rule for such files
        // applies (crate::erase).
        if crate::erase::erased_equal(&c.path, &old, &new).is_ok() {
            continue;
        }
        let names = match route_rules_only(&c.path, &old, &new) {
            Ok(names) => names,
            Err(why) => {
                fx.notes.push(format!(
                    "next config {}: an input change for {} jest test(s) of its next/jest \
                     project: {why}",
                    c.path,
                    tests.len()
                ));
                continue;
            }
        };
        // Everything the file change does, except the input of those tests.
        let modified = || Cause::Modified {
            path: c.path.clone(),
        };
        fx.seeds.push((node, modified()));
        engine.match_inputs(&c.path, &mut |n| {
            if tests.binary_search(&n).is_err() {
                fx.seeds.push((n, modified()));
            }
        });
        engine.match_scopes(&c.path, &mut |n, dir, pattern| {
            fx.seeds.push((
                n,
                Cause::Scope {
                    path: c.path.clone(),
                    kind: c.kind,
                    dir: dir.to_owned(),
                    pattern: pattern.to_owned(),
                },
            ));
        });
        fx.drop.insert(c.path.clone());
        fx.notes.push(format!(
            "next config {}: only the bodies of its route functions changed ({}): not an input \
             change for {} jest test(s) (next/jest does not run them)",
            c.path,
            names.join(", "),
            tests.len()
        ));
    }
    fx
}

/// How the modules a config loads count for the tests that read it
/// (`TURBO_TRACE_DEBUG_CONFIG_IMPORTS`).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Loads {
    /// Not at all (`0`): the reading before [`prepare`] existed.
    Off,
    /// Every load counts (`all`, and `--strict`).
    All,
    /// Loads that provably cannot matter are left out (the default).
    Proven,
}

/// The mode the environment and `--strict` ask for.
pub fn loads_mode(strict: bool) -> Loads {
    let v = turborepo_trace_core::debug::var_os("CONFIG_IMPORTS");
    match v.as_ref().and_then(|v| v.to_str()) {
        _ if strict => Loads::All,
        Some("0") => Loads::Off,
        Some("all") => Loads::All,
        _ => Loads::Proven,
    }
}

/// The references of a config module that load something when the module runs.
const LOAD_KINDS: &[RefKind] = &[
    RefKind::Import,
    RefKind::Require,
    RefKind::DynamicImport,
    RefKind::ReExport,
    RefKind::FsRead,
    RefKind::FsReadDir,
    RefKind::Asset,
    RefKind::Worker,
    RefKind::ChildProcess,
];

/// The specifiers of the `import` declarations of `code` (a config module with
/// its route function bodies blanked) that nothing in it refers to: every
/// binding of every declaration of the specifier has no reference, and the
/// specifier is written nowhere else. `None` when the module cannot be
/// analysed.
fn unused_imports(path: &str, code: &str) -> Option<BTreeSet<String>> {
    use oxc_ast::ast::{ImportDeclarationSpecifier, Statement};
    let st = oxc_span::SourceType::from_path(path).ok()?;
    let alloc = oxc_allocator::Allocator::default();
    let ret = oxc_parser::Parser::new(&alloc, code, st).parse();
    if ret.fatal_error || !ret.diagnostics.is_empty() || ret.is_flow_language {
        return None;
    }
    let sem = oxc_semantic::SemanticBuilder::new().build(&ret.program);
    let scoping = sem.semantic.scoping();
    // A direct `eval` reads bindings without naming them where the analysis can
    // see.
    if scoping
        .root_unresolved_references()
        .keys()
        .any(|name| name.as_str() == "eval")
    {
        return None;
    }
    let mut declared: HashMap<String, (usize, bool)> = HashMap::new();
    for s in &ret.program.body {
        let Statement::ImportDeclaration(d) = s else {
            continue;
        };
        if d.import_kind.is_type() {
            continue;
        }
        let spec = d.source.value.as_str()?;
        let unused = d.specifiers.as_ref().is_some_and(|v| {
            !v.is_empty()
                && v.iter().all(|sp| {
                    let local = match sp {
                        ImportDeclarationSpecifier::ImportSpecifier(s) => &s.local,
                        ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => &s.local,
                        ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => &s.local,
                    };
                    scoping
                        .get_resolved_references(local.symbol_id())
                        .next()
                        .is_none()
                })
        });
        let slot = declared.entry(spec.to_owned()).or_insert((0, true));
        slot.0 += 1;
        slot.1 &= unused;
    }
    // `const { a, b } = require('./x')` at the top level, the same way.
    for s in &ret.program.body {
        let Statement::VariableDeclaration(v) = s else {
            continue;
        };
        for d in &v.declarations {
            let Some(oxc_ast::ast::Expression::CallExpression(c)) = &d.init else {
                continue;
            };
            let (
                oxc_ast::ast::Expression::Identifier(callee),
                [oxc_ast::ast::Argument::StringLiteral(arg)],
            ) = (&c.callee, c.arguments.as_slice())
            else {
                continue;
            };
            let (true, Some(spec)) = (callee.name == "require", arg.value.as_str()) else {
                continue;
            };
            let unused = d.id.get_binding_identifiers().iter().all(|b| {
                scoping
                    .get_resolved_references(b.symbol_id())
                    .next()
                    .is_none()
            });
            let slot = declared.entry(spec.to_owned()).or_insert((0, true));
            slot.0 += 1;
            slot.1 &= unused;
        }
    }
    Some(
        declared
            .into_iter()
            .filter(|(spec, (n, unused))| *unused && code.matches(spec.as_str()).count() == *n)
            .map(|(spec, _)| spec)
            .collect(),
    )
}

fn is_js(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, ext)| {
        matches!(
            ext,
            "js" | "mjs" | "cjs" | "jsx" | "ts" | "mts" | "cts" | "tsx"
        )
    })
}

/// The files of the repository that loading the module `start` loads, itself
/// included, when every one of them is inert ([`crate::loadcode`]); else which
/// file stands against it.
fn inert_closure(
    engine: &Engine<'_, View<'_>>,
    root: &Path,
    start: NodeId,
) -> Result<Vec<NodeId>, String> {
    let view = engine.graph();
    let g = view.base();
    let mut seen = vec![start];
    let mut i = 0;
    while i < seen.len() {
        if seen.len() > 256 {
            return Err("it loads more than 256 files".to_owned());
        }
        let n = seen[i];
        i += 1;
        let path = g.path(n);
        if g.is_external(n) || turborepo_trace_store::tasks::is_pseudo_lang(g.lang(n)) {
            return Err(format!("{path} is not a file of the repository"));
        }
        // A data file (JSON): loading it defines a value.
        if !is_js(path) {
            continue;
        }
        let head = std::fs::read_to_string(root.join(path))
            .map_err(|_| format!("{path} is not readable"))?;
        let code =
            crate::loadcode::scan(path, &head).ok_or_else(|| format!("{path} does not parse"))?;
        if let Some(why) = &code.not_inert {
            return Err(format!(
                "loading {path} may do more than define things: {why}"
            ));
        }
        for spec in &code.loads {
            let mut resolved = false;
            for e in view.out_edges(n) {
                if e.specifier != Some(spec.as_str())
                    || e.flags().contains(RefFlags::TYPE_ONLY)
                    || !matches!(
                        e.kind(),
                        RefKind::Import | RefKind::ReExport | RefKind::Require
                    )
                {
                    continue;
                }
                if let Some(to) = e.to {
                    resolved = true;
                    // A package is not followed: it counts when it changes
                    // ([`load_changed`]).
                    if !g.is_external(to) && !seen.contains(&to) {
                        seen.push(to);
                    }
                }
            }
            if !resolved && !crate::loadcode::is_builtin(spec) && spec.starts_with(['.', '/']) {
                return Err(format!("{path} loads {spec}, which is not resolved"));
            }
        }
    }
    Ok(seen)
}

/// What the change does to the nodes of the graph directly, as far as
/// [`load_changed`] needs it: the nodes whose resolution read a changed file or
/// probed a path that appeared or went away, and the nodes seeded by the
/// caller's field-level rules.
struct Direct {
    nodes: BTreeSet<NodeId>,
}

impl Direct {
    fn new(engine: &Engine<'_, View<'_>>, cs: &ChangeSet, changes: &[Change]) -> Direct {
        let mut nodes = BTreeSet::new();
        for c in changes {
            engine.match_inputs(&c.path, &mut |n| {
                nodes.insert(n);
            });
            if c.kind != ChangeKind::Modified {
                engine.match_probes(&c.path, &mut |n, _| {
                    nodes.insert(n);
                });
            }
        }
        // Not the seeds that only type resolution observes: a config and what it loads
        // are transpiled, not checked.
        nodes.extend(cs.seeds.iter().map(|(n, _)| *n));
        Direct { nodes }
    }
}

/// Whether the change can make the load of the inert files `files` differ
/// (module docs, b): the file and the reason, or `None`.
fn load_changed(
    engine: &Engine<'_, View<'_>>,
    root: &Path,
    files: &[NodeId],
    cs: &ChangeSet,
    changes: &[Change],
    direct: &Direct,
    base: Option<&Git>,
) -> Option<(NodeId, String)> {
    let view = engine.graph();
    let g = view.base();
    for &f in files {
        let path = g.path(f);
        if let Some(c) = changes.iter().find(|c| c.path == path) {
            if c.kind != ChangeKind::Modified {
                return Some((f, "it was added".to_owned()));
            }
            let same = is_js(path)
                && base.and_then(|b| b.show(path)).is_some_and(|old| {
                    let Ok(new) = std::fs::read_to_string(root.join(path)) else {
                        return false;
                    };
                    if crate::loadcode::same_load(path, &old, &new) {
                        return true;
                    }
                    // Both revisions only define things and load the same modules.
                    let dialect = Dialect::from_path(Path::new(path));
                    let (b, h) = (
                        scan_shape(old.as_bytes(), dialect),
                        scan_shape(new.as_bytes(), dialect),
                    );
                    b.understood
                        && h.understood
                        && b.side_effect_free
                        && h.side_effect_free
                        && b.executes == h.executes
                });
            if !same {
                return Some((
                    f,
                    "its code outside function bodies changed, or its base revision is not at hand"
                        .to_owned(),
                ));
            }
        }
        if direct.nodes.contains(&f) {
            return Some((
                f,
                "it resolves differently, or a field-level rule seeds it".to_owned(),
            ));
        }
        // A package it loads changed.
        let own = g.package(f);
        for e in view.out_edges(f) {
            let Some(to) = e.to.filter(|t| g.is_external(*t)) else {
                continue;
            };
            let p = g.package(to);
            if cs.packages.contains(&p)
                || cs
                    .scoped
                    .iter()
                    .any(|s| s.package == p && s.importers.contains(&own))
            {
                return Some((
                    f,
                    format!(
                        "the package {} that it loads changed",
                        g.package_info(p).name
                    ),
                ));
            }
        }
    }
    None
}

/// The paths that the working-directory reads of the config `c` name
/// (`path.join(process.cwd(), 'data.json')`), relative to the working
/// directories of `tests`: the config loads in their process. A changed path
/// counts when it is one of them or lies below one (a directory that is read or
/// listed). Reads inside `bodies` (route functions) never run.
///
/// `None` when the engine's verdict about the config's uncertainty has to
/// stand: a working directory is not known, or one of the reads is not a file
/// read or a directory listing with one literal path below the working
/// directory (a pattern, alternatives, a guessed value, a module load, whose
/// own imports would count). The per-test layer of the graph
/// (`turborepo_trace_store` `compose/cwd.rs`) resolves the same references for
/// the files a test runs; the config is not one of them, it is read by
/// `next/jest`.
fn cwd_reads(
    view: &View<'_>,
    c: NodeId,
    bodies: &[(u32, u32)],
    tests: &[NodeId],
    cwds: &HashMap<u32, Vec<&str>>,
) -> Option<Vec<String>> {
    let records: Vec<_> = view
        .uncertainties(c)
        .filter(|u| u.kind == UncertaintyKind::CwdDependent)
        .filter(|u| {
            !u.span
                .is_some_and(|s| bodies.iter().any(|&(a, z)| a <= s.start && s.end <= z))
        })
        .collect();
    if records.is_empty() {
        return Some(Vec::new());
    }
    let mut dirs: Vec<&str> = Vec::new();
    for t in tests {
        let own = cwds.get(&t.0).filter(|v| !v.is_empty())?;
        for d in own {
            if !dirs.contains(d) {
                dirs.push(d);
            }
        }
    }
    let mut out = Vec::new();
    for u in records {
        // A guessed value (the wording of the extractor's records, as the per-test
        // layer reads it).
        if u.detail.contains("initial value is used") || u.detail.contains("assumed to be `path.") {
            return None;
        }
        let mut found = false;
        for e in view.out_edges(c) {
            let same = match (u.span, e.span) {
                (Some(a), Some(b)) => a.start < b.end && b.start < a.end,
                _ => u.line != 0 && e.line == u.line,
            };
            if !same {
                continue;
            }
            if !matches!(
                e.kind(),
                RefKind::FsRead | RefKind::FsReadDir | RefKind::Asset
            ) {
                return None;
            }
            let spec = e.specifier?;
            let spec = spec
                .strip_prefix("./")
                .unwrap_or(spec)
                .trim_end_matches('/');
            let plain = !spec.is_empty()
                && !spec.starts_with('/')
                && !spec.contains(['*', '<', '|', ' ', '\\'])
                && spec
                    .split('/')
                    .all(|s| !s.is_empty() && s != "." && s != "..");
            if !plain {
                return None;
            }
            found = true;
            for d in &dirs {
                out.push(if d.is_empty() {
                    spec.to_owned()
                } else {
                    format!("{d}/{spec}")
                });
            }
        }
        if !found {
            return None;
        }
    }
    Some(out)
}

/// A test selected because something its Next.js config loads is affected.
#[derive(Clone, Debug)]
pub struct Hit {
    /// The chain from the changed (or uncertain) node to the config, both
    /// inclusive.
    witness: Vec<NodeId>,
    verdict: Verdict,
    cause: Option<Cause>,
    uncertain: Option<UncertainSite>,
}

/// What the modules that Next.js configs load add to an `affected` query.
#[derive(Debug, Default)]
pub struct Loaded {
    /// The tests, each with the chain that selects it.
    pub tests: BTreeMap<NodeId, Hit>,
    /// Readable notes for `--stats`, `--explain` and JSON.
    pub notes: Vec<String>,
}

/// The evaluation of what the configs load, before the layers that select
/// entries after the engine have run for it ([`prepare`], [`Pending::finish`]).
pub struct Pending {
    /// (config, its tests, its loads, the byte ranges of its route function
    /// bodies)
    #[allow(clippy::type_complexity)]
    configs: Vec<(NodeId, Vec<NodeId>, Vec<Load>, Vec<(u32, u32)>)>,
    /// The entries of the evaluation: the configs and the loads that count.
    /// They are evaluated without type-only edges, like the tests they
    /// stand for.
    pub set: NodeSet,
    /// The engine's results for them.
    evaluated: Vec<EntryResult>,
    /// What the layers that select entries after the engine add for them. The
    /// options of the evaluation take a read with a computed path as
    /// bounded by the names of the code loaded with it (`crate::names`) and
    /// a load with an unknown target as bounded by the declared packages
    /// (`crate::loads`), so both layers have to run for these entries
    /// too. One report each: a layer gives one result per entry and skips an
    /// entry that is in its report already.
    pub named: AffectedReport,
    /// See [`Pending::named`].
    pub bounded: AffectedReport,
    /// Per config, the files of its process ([`process_files`]); computed when
    /// a layer asks ([`Pending::holds`]).
    process: Option<Vec<NodeSet>>,
}

/// The imports of a config that are left out.
fn left_out(loads: &[Load]) -> Vec<NodeId> {
    loads
        .iter()
        .filter(|l| l.removed.is_some())
        .map(|l| l.target)
        .collect()
}

/// The files of the process that loads the config `c`: what `c` reaches over
/// any edge when the imports in `left_out` are not followed from it (with none
/// left out, all it reaches). A superset of what the process loads (every kind
/// of edge is followed), which is the safe side for "no file of the process
/// holds such a read".
fn process_files(view: &View<'_>, c: NodeId, left_out: &[NodeId]) -> NodeSet {
    let mut seen = NodeSet::new(view.node_count());
    seen.insert(c);
    let mut stack = vec![c];
    while let Some(n) = stack.pop() {
        for e in view.out_edges(n) {
            let Some(to) = e.to else { continue };
            if n == c && left_out.contains(&to) {
                continue;
            }
            if seen.insert(to) {
                stack.push(to);
            }
        }
    }
    seen
}

/// One load of a config.
struct Load {
    target: NodeId,
    /// Why it is left out, if it is, and the files that loading it loads.
    removed: Option<(&'static str, Vec<NodeId>)>,
    /// A change to what loading it does, found without the engine: the file and
    /// why.
    forced: Option<(NodeId, String)>,
}

/// Evaluates what the Next.js configs of `next/jest` projects load (see the
/// module docs). `opts` are the options of the main evaluation, `entries` its
/// entries. `None`: nothing to evaluate. The caller runs the names and the
/// loads layer for [`Pending::set`] into [`Pending::layered`], then
/// [`Pending::finish`].
pub fn prepare(
    engine: &Engine<'_, View<'_>>,
    root: &Path,
    cs: &ChangeSet,
    opts: &AffectedOptions,
    git: Option<&Git>,
    entries: &NodeSet,
    strict: bool,
) -> Option<Pending> {
    // The file changes that no field-level rule replaced by seeds (a manifest whose
    // changed dependencies were applied per consumer is not a change of every
    // file that resolved through it).
    let changes: &[Change] = &cs.files;
    let mode = loads_mode(strict);
    if mode == Loads::Off {
        return None;
    }
    let view = engine.graph();
    let g = view.base();
    // The configs that jest tests among the entries read, with those tests and the
    // loads. (config, its tests, its loads, the byte ranges of its route
    // function bodies)
    #[allow(clippy::type_complexity)]
    let mut configs: Vec<(NodeId, Vec<NodeId>, Vec<Load>, Vec<(u32, u32)>)> = Vec::new();
    let mut set = NodeSet::new(g.node_count());
    let mut direct: Option<Direct> = None;
    for i in 0..g.node_count() {
        let c = NodeId(i);
        let path = g.path(c);
        if g.is_external(c) || !CONFIG_NAMES.contains(&file_name(path)) {
            continue;
        }
        let tests: Vec<NodeId> = next_jest_tests(engine, path)
            .into_iter()
            .filter(|t| entries.contains(*t))
            .collect();
        if tests.is_empty() {
            continue;
        }
        // The route function bodies and the imports only they use.
        let blanked = (mode == Loads::Proven)
            .then(|| std::fs::read_to_string(root.join(path)).ok())
            .flatten()
            .and_then(|text| {
                let sites = crate::jsblank::Sites {
                    props: ROUTE_FUNCTIONS,
                    what: crate::jsblank::What::FunctionBodies,
                    wrapper: None,
                };
                crate::jsblank::blanked(path, &text, sites).ok()
            });
        let unused = blanked
            .as_ref()
            .and_then(|b| unused_imports(path, &b.code))
            .unwrap_or_default();
        let in_body = |span: Option<turborepo_trace_graph::Span>| {
            span.zip(blanked.as_ref())
                .is_some_and(|(s, b)| b.spans.iter().any(|&(a, z)| a <= s.start && s.end <= z))
        };
        let mut loads: Vec<Load> = Vec::new();
        let mut closures: HashMap<NodeId, Result<Vec<NodeId>, String>> = HashMap::new();
        for e in view.out_edges(c) {
            let Some(to) = e.to else { continue };
            if !LOAD_KINDS.contains(&e.kind())
                || e.flags().contains(RefFlags::TYPE_ONLY)
                || g.is_external(to)
                || to == c
            {
                continue;
            }
            let (removed, forced) = if in_body(e.span) {
                (
                    Some((
                        "it is loaded inside a route function, which next/jest never calls",
                        vec![to],
                    )),
                    None,
                )
            } else if matches!(e.kind(), RefKind::Import | RefKind::Require)
                && e.specifier.is_some_and(|s| unused.contains(s))
            {
                match closures
                    .entry(to)
                    .or_insert_with(|| inert_closure(engine, root, to))
                {
                    Ok(files) => {
                        let direct = direct.get_or_insert_with(|| Direct::new(engine, cs, changes));
                        match load_changed(engine, root, files, cs, changes, direct, git) {
                            Some(hit) => (None, Some(hit)),
                            None => (
                                Some((
                                    "only route functions use it, which next/jest never calls, \
                                     and loading it does the same as before",
                                    files.clone(),
                                )),
                                None,
                            ),
                        }
                    }
                    Err(_) => (None, None),
                }
            } else {
                (None, None)
            };
            match loads.iter_mut().find(|l| l.target == to) {
                // One counting reference makes the module count.
                Some(l) => {
                    if removed.is_none() {
                        l.removed = None;
                        if l.forced.is_none() {
                            l.forced = forced;
                        }
                    }
                }
                None => loads.push(Load {
                    target: to,
                    removed,
                    forced,
                }),
            }
        }
        set.insert(c);
        for l in loads.iter().filter(|l| l.removed.is_none()) {
            set.insert(l.target);
        }
        let bodies = blanked.map(|b| b.spans).unwrap_or_default();
        configs.push((c, tests, loads, bodies));
    }
    if configs.is_empty() {
        return None;
    }
    // One evaluation of the loads, with the rules of a test that transpiles file by
    // file. Without the graph's mock facts: changed exports then takes no
    // definition-level shortcut for these few entries (a modified file counts
    // with all its exports).
    let outcome = crate::barrel::evaluate(engine, root, cs, &set, opts, git, Some(&set), None);
    let mut named = outcome.report;
    let mut evaluated: Vec<EntryResult> = std::mem::take(&mut named.entries);
    let bounded = AffectedReport {
        policy: named.policy,
        entry_count: named.entry_count,
        entries: Vec::new(),
        seed_count: 0,
        reached_count: 0,
        unmatched: Vec::new(),
    };
    // A loaded module that is itself modified is affected, whatever it exports (the
    // barrel rule defers a modified module to the entries that use its exports,
    // and here the module is the entry), unless its runtime JavaScript did not
    // change.
    for (_, _, loads, _) in &configs {
        for l in loads.iter().filter(|l| l.removed.is_none()) {
            // What loading an import does changed (found by `load_changed`): the engine may
            // not connect the file to the import (a read inside a function is no edge of
            // the load), so the verdict is made here.
            if let Some((f, _)) = &l.forced
                && !evaluated
                    .iter()
                    .any(|e| e.entry == l.target && e.cause.is_some())
            {
                evaluated.retain(|e| e.entry != l.target);
                evaluated.push(EntryResult {
                    entry: l.target,
                    verdict: Verdict::Affected,
                    witness: if *f == l.target {
                        vec![l.target]
                    } else {
                        vec![*f, l.target]
                    },
                    cause: Some(Cause::Modified {
                        path: g.path(*f).to_owned(),
                    }),
                    uncertain: None,
                });
                continue;
            }
            let path = g.path(l.target);
            if cs.files.iter().any(|c| c.path == path)
                && !outcome.erased.iter().any(|e| e == path)
                && !evaluated
                    .iter()
                    .any(|e| e.entry == l.target && e.cause.is_some())
            {
                evaluated.retain(|e| e.entry != l.target);
                evaluated.push(EntryResult {
                    entry: l.target,
                    verdict: Verdict::Affected,
                    witness: vec![l.target],
                    cause: Some(Cause::Modified {
                        path: path.to_owned(),
                    }),
                    uncertain: None,
                });
            }
        }
    }
    Some(Pending {
        configs,
        set,
        evaluated,
        named,
        bounded,
        process: None,
    })
}

impl Pending {
    /// Whether a layer whose sites are `sites` (the files with a read that
    /// names bound, or with a load that the declared packages bound) can
    /// select anything here: whether a file of some config's process is one
    /// of them. When none is, no such read or load runs while a config
    /// loads, the layer's results would not count ([`Pending::finish`]) and
    /// the caller need not run it for [`Pending::set`].
    pub fn holds(&mut self, engine: &Engine<'_, View<'_>>, sites: &[NodeId]) -> bool {
        if sites.is_empty() {
            return false;
        }
        let view = engine.graph();
        let configs = &self.configs;
        let process = self.process.get_or_insert_with(|| {
            configs
                .iter()
                .map(|(c, _, loads, _)| process_files(view, *c, &left_out(loads)))
                .collect()
        });
        process.iter().any(|p| sites.iter().any(|s| p.contains(*s)))
    }

    /// The tests that what their config loads selects, once the layers have
    /// run: `names_opts` and `loads_opts` are the options they ran with.
    pub fn finish(
        self,
        engine: &Engine<'_, View<'_>>,
        cs: &ChangeSet,
        names_opts: &AffectedOptions,
        loads_opts: &AffectedOptions,
    ) -> Loaded {
        let mut out = Loaded::default();
        let changes: &[Change] = &cs.files;
        let view = engine.graph();
        let g = view.base();
        let configs = &self.configs;
        let results: HashMap<NodeId, &EntryResult> = self
            .evaluated
            .iter()
            .filter(|e| e.verdict != Verdict::Unaffected)
            .map(|e| (e.entry, e))
            .collect();
        // What the names and the loads layer select.
        fn of(report: &AffectedReport, n: NodeId) -> Vec<&EntryResult> {
            report
                .entries
                .iter()
                .filter(|e| e.entry == n && e.verdict != Verdict::Unaffected)
                .collect()
        }
        // The files with a read that names bound, and with a load that the declared
        // packages bound.
        let named_sites = if self.named.entries.is_empty() {
            Vec::new()
        } else {
            engine.named_sites(names_opts)
        };
        let declared_sites = if self.bounded.entries.is_empty() {
            Vec::new()
        } else {
            engine.declared_sites(loads_opts)
        };
        // The tests' working directories, when a config reads relative to one.
        let cwds: HashMap<u32, Vec<&str>> = if configs.iter().any(|(c, _, _, _)| {
            view.uncertainties(*c)
                .any(|u| u.kind == UncertaintyKind::CwdDependent)
        }) {
            g.tests_with_cwds()
        } else {
            HashMap::new()
        };
        for (i, (c, tests, loads, bodies)) in configs.iter().enumerate() {
            let path = g.path(*c);
            // Reads relative to the working directory: the config loads in the tests'
            // process, so they are files of the tests' working directories.
            let cwd_paths = cwd_reads(view, *c, bodies, tests, &cwds);
            let cwd_hit = cwd_paths.as_ref().and_then(|paths| {
                let p = paths.iter().find(|p| {
                    changes.iter().any(|c| {
                        c.path
                            .strip_prefix(p.as_str())
                            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
                    })
                })?;
                Some(EntryResult {
                    entry: *c,
                    verdict: Verdict::Affected,
                    witness: vec![*c],
                    cause: Some(Cause::Modified { path: p.clone() }),
                    uncertain: None,
                })
            });
            // The config's node as a seed of its own: anything but its own edit, which is
            // the content input's business ([`effects`]).
            let own = results.get(c).filter(|r| {
                r.witness.as_slice() == [*c]
                    && match &r.cause {
                        Some(Cause::Modified { path: p } | Cause::Deleted { path: p }) => p != path,
                        Some(_) => true,
                        // Its own uncertainty, unless that is the working-directory reads
                        // resolved above.
                        None => r.uncertain.as_ref().is_some_and(|u| {
                            u.node == *c
                                && !(cwd_paths.is_some()
                                    && u.reasons
                                        .iter()
                                        .all(|(k, _, _)| *k == UncertaintyKind::CwdDependent))
                        }),
                    }
            });
            let own = own.copied().or(cwd_hit.as_ref());
            let mut found: Vec<&EntryResult> = Vec::new();
            found.extend(own);
            // The layers see the config with everything it imports, also the imports that
            // are left out: their reads sit in functions that do not run when the config
            // loads. A layer's result for the config counts when a file of the config's
            // process holds a read (a load) of the layer's kind; when none does, nothing
            // of that kind runs at load, whatever the names say. (One result per entry is
            // all a layer gives, so a result is not judged by the read it happens to cite.)
            let (by_name, by_load) = (of(&self.named, *c), of(&self.bounded, *c));
            let runs = |sites: &[NodeId]| {
                self.process
                    .as_ref()
                    .is_some_and(|p| sites.iter().any(|s| p[i].contains(*s)))
            };
            if !by_name.is_empty() && runs(&named_sites) {
                found.extend(by_name);
            }
            if !by_load.is_empty() && runs(&declared_sites) {
                found.extend(by_load);
            }
            for l in loads.iter().filter(|l| l.removed.is_none()) {
                found.extend(results.get(&l.target).copied());
                found.extend(of(&self.named, l.target));
                found.extend(of(&self.bounded, l.target));
            }
            // A change is a stronger reason than uncertainty.
            let hit = found
                .into_iter()
                .min_by_key(|r| (r.cause.is_none(), r.witness.len()));
            if let Some(r) = hit {
                let mut witness = r.witness.clone();
                if witness.last() != Some(c) {
                    witness.push(*c);
                }
                let through = r.witness.last().map_or(path, |n| g.path(*n));
                out.notes.push(format!(
                    "next config {path}: {} that it loads is affected: an input change for {} \
                     jest test(s) of its next/jest project",
                    if through == path { "a path" } else { through },
                    tests.len()
                ));
                for t in tests {
                    out.tests.entry(*t).or_insert_with(|| Hit {
                        witness: witness.clone(),
                        verdict: r.verdict,
                        cause: r.cause.clone(),
                        uncertain: r.uncertain.clone(),
                    });
                }
                continue;
            }
            // Why a changed module did not select them.
            for l in loads {
                if let Some((why, files)) = &l.removed
                    && let Some(changed) = files
                        .iter()
                        .map(|f| g.path(*f))
                        .find(|p| changes.iter().any(|c| c.path == *p))
                {
                    out.notes.push(format!(
                        "next config {path}: {changed} changed, which the config loads through \
                         {}: not an input change for {} jest test(s): {why} (assumed: no package \
                         the config is wrapped in calls a route function while it loads; --strict \
                         counts it)",
                        g.path(l.target),
                        tests.len()
                    ));
                }
            }
        }
        out
    }
}

impl Loaded {
    /// Adds the verdicts to `report`: among `entries`, the tests whose config
    /// loads something affected.
    pub fn apply(&self, g: &Graph, entries: &NodeSet, report: &mut AffectedReport) {
        if self.tests.is_empty() {
            return;
        }
        let mut at: HashMap<NodeId, usize> = HashMap::new();
        for (i, e) in report.entries.iter().enumerate() {
            at.insert(e.entry, i);
        }
        let mut added = false;
        for (&test, hit) in &self.tests {
            if !entries.contains(test) {
                continue;
            }
            let mut witness = hit.witness.clone();
            witness.push(test);
            let result = EntryResult {
                entry: test,
                verdict: hit.verdict,
                witness,
                cause: hit.cause.clone(),
                uncertain: hit.uncertain.clone(),
            };
            match at.get(&test) {
                // A change is a stronger reason than uncertainty.
                Some(&i) if report.entries[i].cause.is_none() && result.cause.is_some() => {
                    report.entries[i] = result;
                }
                Some(_) => {}
                None => {
                    at.insert(test, report.entries.len());
                    report.entries.push(result);
                    added = true;
                }
            }
        }
        if added {
            report
                .entries
                .sort_by(|a, b| g.path(a.entry).cmp(g.path(b.entry)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: &str = "apps/site/next.config.mjs";

    fn config(rewrites: &str, rest: &str) -> String {
        format!(
            "import {{ withPlugin }} from 'plugin';\n\nconst nextConfig = {{\n  \
             transpilePackages: ['@acme/ui'],\n  async rewrites() {{\n    return {{\n      \
             beforeFiles: [\n{rewrites}      ],\n    }};\n  }},\n{rest}}};\n\nexport default \
             withPlugin(nextConfig);\n"
        )
    }

    const ENTRY_A: &str =
        "        { source: '/a/:path*', destination: 'https://a.example/:path*' },\n";
    const ENTRY_B: &str = "        {\n          source: '/:team/:project/c/:id',\n          destination: 'https://b.example/:team/:project/:id',\n        },\n";

    #[test]
    fn an_entry_of_a_route_function_is_not_a_load_time_change() {
        let base = config(&format!("{ENTRY_A}{ENTRY_B}"), "");
        let head = config(ENTRY_A, "");
        assert_eq!(route_rules_only(P, &base, &head), Ok(vec!["rewrites"]));
        // Function expressions and arrows, with a block or an expression.
        let shape = |body: &str| {
            format!(
                "module.exports = {{\n  redirects: async function () {{ return [{body}]; }},\n  \
                 headers: async () => [{body}],\n  rewrites: () => {{ return [{body}]; }},\n}};\n"
            )
        };
        assert_eq!(
            route_rules_only(
                "next.config.js",
                &shape("{ source: '/a' }"),
                &shape("{ source: '/b' }, call()")
            ),
            Ok(vec!["headers", "redirects", "rewrites"])
        );
        // Comments and formatting outside don't count either.
        let head = config(ENTRY_A, "  // served by the proxy now\n");
        assert_eq!(route_rules_only(P, &base, &head), Ok(vec!["rewrites"]));
    }

    #[test]
    fn everything_else_is_a_change() {
        let base = config(ENTRY_A, "");
        let err = |head: &str| route_rules_only(P, &base, head).unwrap_err();
        // A field next/jest reads, and one it does not read (loading validates it).
        let head = base.replace("['@acme/ui']", "['@acme/ui', '@acme/icons']");
        assert!(
            err(&head).contains("runtime code differs"),
            "{}",
            err(&head)
        );
        let head = config(ENTRY_A, "  basePath: '/docs',\n");
        assert!(
            err(&head).contains("runtime code differs"),
            "{}",
            err(&head)
        );
        // Code that runs when the module loads.
        let head = base.replace("const nextConfig", "check();\nconst nextConfig");
        assert!(
            err(&head).contains("runtime code differs"),
            "{}",
            err(&head)
        );
        // The function's parameters and the property itself are outside the body.
        let head = base.replace("async rewrites()", "async rewrites(extra = load())");
        assert!(
            err(&head).contains("runtime code differs"),
            "{}",
            err(&head)
        );
        let head = base.replace("async rewrites()", "async redirects()");
        assert!(
            err(&head).contains("runtime code differs"),
            "{}",
            err(&head)
        );
        // A syntax error inside a body breaks the load.
        let head = config("        { source: '/a' destination: '/b' },\n", "");
        assert!(err(&head).contains("head: parse errors"), "{}", err(&head));
        // A getter runs when the property is read.
        let getter =
            |v: &str| format!("module.exports = {{ get rewrites() {{ return {v}; }} }};\n");
        assert!(route_rules_only("next.config.js", &getter("[1]"), &getter("[2]")).is_err());
        // A route function defined elsewhere is ordinary code.
        let named = |v: &str| {
            format!(
                "async function rules() {{ return {v}; }}\nmodule.exports = {{ rewrites: rules \
                 }};\n"
            )
        };
        assert!(route_rules_only("next.config.js", &named("[1]"), &named("[2]")).is_err());
    }

    #[test]
    fn a_module_that_may_call_a_route_function_itself() {
        let with = |tail: &str, v: &str| {
            format!(
                "const nextConfig = {{\n  async rewrites() {{ return {v}; \
                 }},\n}};\n{tail}\nexport default nextConfig;\n"
            )
        };
        for tail in [
            "await nextConfig.rewrites();",
            "const rules = await nextConfig['rewrites']();",
            "const { rewrites } = nextConfig;\nawait rewrites();",
            "const { rewrites: r } = nextConfig;\nawait r();",
        ] {
            let e = route_rules_only(P, &with(tail, "[1]"), &with(tail, "[2]")).unwrap_err();
            assert!(
                e.contains("`rewrites` is used outside its definition"),
                "{tail}: {e}"
            );
        }
        // A variable of that name is not the function.
        let tail = "const redirects = [];\nconsole.log(redirects.length);";
        assert_eq!(
            route_rules_only(P, &with(tail, "[1]"), &with(tail, "[2]")),
            Ok(vec!["rewrites"])
        );
    }
}
