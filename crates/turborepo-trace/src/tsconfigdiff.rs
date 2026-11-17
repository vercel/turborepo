//! `affected --since`: a `tsconfig.json` edit that a test run which only
//! transpiles cannot observe.
//!
//! A `tsconfig.json` is an input of every file whose resolution consulted it
//! (`paths`, `baseUrl`), so any edit of it selects every test below it. Type
//! checkers and builds read the whole file. A test runner that transpiles file
//! by file (`turborepo_trace_graph::barrel::Runner::Isolated`) reads a few
//! fields of it, or none.
//!
//! # What a transpile-only run reads of a tsconfig
//!
//! | Transform | What it reads |
//! |---|---|
//! | `babel-jest`, `@swc/jest`, `esbuild-jest`, `jest-esbuild`, `@sucrase/jest-plugin` | nothing: their options come from their own config (`.babelrc`, `.swcrc`, the jest config) |
//! | `@swc-node/jest` | `compilerOptions` of the project's tsconfig, with its `extends` chain (target, module, JSX, decorators, `paths`, `baseUrl`, ...) |
//! | the SWC transformer of `next/jest` | of the application's tsconfig, with its `extends` chain: `compilerOptions.paths`, `baseUrl`, `experimentalDecorators`, `emitDecoratorMetadata`, `useDefineForClassFields`, `jsxImportSource` (`next/src/build/swc/options.ts`, `load-jsconfig.ts`) |
//! | `ts-jest` with `isolatedModules` | every compiler option, with the `extends` chain: `ts.transpileModule` gets them, emits by them and reports option errors |
//! | Vite's transform (vitest) | of the tsconfig that applies to each file, with its `extends` chain: `target`, `useDefineForClassFields`, `importsNotUsedAsValues`, `preserveValueImports`, `verbatimModuleSyntax`, `jsx`, `jsxFactory`, `jsxFragmentFactory`, `jsxImportSource`, `experimentalDecorators`, `alwaysStrict`. Which tsconfig applies is the nearest one, or one of its `references` that includes the file when the nearest one does not (`files`, `include`, `exclude`, `allowJs`); every referenced config is parsed |
//! | loading the runner's own config | jest loads a `jest.config.ts` through `ts-node`, which type-checks it with the project's compiler options; Vite bundles a `vitest.config.ts` with esbuild, which reads the emit options |
//! | a Vite plugin from a package | anything: `vite-tsconfig-paths` builds its list of projects from the `references` |
//!
//! So of the compiler options none is decided here: `ts-jest` and `ts-node` see
//! all of them, and telling apart the tests of one transform from the tests of
//! another needs seeds per entry. `include`, `exclude` and `files` are not
//! decided either: Vite uses them to pick the config of a file. One field is
//! decided: **`references`**.
//!
//! # `references`
//!
//! The list of projects a build orders (`tsc -b`) and an editor follows. For a
//! transpile-only run it can matter in two ways, and [`effects`] excludes both:
//!
//! 1. *Vite parses every referenced config and may use it for a file* the
//!    referencing config does not include. A reference that is added or removed
//!    is therefore only left out when the referenced project loads at the
//!    revision that lists it, nothing its load read changed in the same diff,
//!    and it cannot include a file of the referencing config's directory: its
//!    directory is neither that directory nor above or below it, and neither is
//!    a directory that its `files` and `include` (its own, or the ones it
//!    inherits; everything below it without any) name files of. What the
//!    project is at a revision (which file the reference names, which files its
//!    `extends` chain names, what it inherits) is the answer of the loader the
//!    tracer uses (`turborepo_trace_lang_js::resolve::governance::Projects`:
//!    the working tree, and the working tree with the diff's files as they
//!    were), not of rules written here.
//! 2. *A Vite plugin may read the references.* `vite-tsconfig-paths` does. When
//!    the config of a vitest project (with the files of the repository it
//!    loads) imports it and a test of that project can load a file that
//!    resolves through the changed config, the file stays a whole-file change.
//!    A plugin the graph does not know makes its project's tests uncertain on
//!    every change already (`turborepo_trace_frameworks::viteplugins`).
//!
//! The list must be well-formed at both revisions (an array of objects with a
//! string `path`): TypeScript reports anything else as an error of the config,
//! which `ts-jest`, `ts-node` and `next/jest` raise.
//!
//! When that holds and `references` is the only top-level field that differs
//! (or nothing but comments and formatting does), the change is applied like
//! this:
//!
//! - the file's node is seeded: what imports or reads the file, and the tasks
//!   that take it as an input, are affected as for any edit;
//! - the nodes that have it as an input through resolution are seeded for the
//!   entries that are *not* transpile-only
//!   ([`turborepo_trace_graph::ChangeSet::type_seeds`]): type-check and build
//!   tasks, tests of runners that type-check, tests of other frameworks;
//! - the uncertain reads that cover the file are seeded as before.
//!
//! A jest test of a transpile-only transform and a vitest test are not selected
//! through the file.
//!
//! `TURBO_TRACE_DEBUG_TSCONFIG_FIELDS=0` turns the rule off (and
//! `TURBO_TRACE_DEBUG_CONFIG_FIELDS=0`, with every other field-level rule). The
//! rule is argued from the tools' sources (in the Vite case, a reference to a
//! config next to the changed one that includes its files changes how they are
//! compiled) and `crates/turborepo-trace/tests/affected_tsconfig.rs` pins the
//! conditions.

use std::{
    collections::{BTreeSet, HashMap},
    path::{Component, Path, PathBuf},
};

use serde_json::Value;
use turborepo_trace_graph::{
    Cause, Change, ChangeKind, Engine, GraphView, NodeId, NodeSet, RefKind,
};
use turborepo_trace_store::Graph;

use crate::lockdiff::Git;

/// The graph the CLI's engine runs on (node ids are the store graph's).
type View<'g> = turborepo_trace_graph::Patched<'g, Graph>;

/// What the `references` edits of tsconfig files change in an `affected` query.
#[derive(Debug, Default)]
pub struct Effects {
    /// File changes that are applied through the seeds instead.
    pub drop: BTreeSet<String>,
    /// The nodes to seed, each with its cause.
    pub seeds: Vec<(NodeId, Cause)>,
    /// The nodes to seed for entries that are not transpile-only.
    pub type_seeds: Vec<(NodeId, Cause)>,
    /// Readable notes for `--stats` and `--explain`.
    pub notes: Vec<String>,
}

/// Whether the rule is on (`TURBO_TRACE_DEBUG_TSCONFIG_FIELDS=0` turns it off).
pub fn enabled() -> bool {
    crate::configdiff::enabled()
        && turborepo_trace_core::debug::var_os("TSCONFIG_FIELDS").is_none_or(|v| v != "0")
}

/// Whether `file` (a base name) is a TypeScript or JavaScript project config.
fn is_tsconfig(file: &str) -> bool {
    (file.starts_with("tsconfig.") || file.starts_with("jsconfig.")) && file.ends_with(".json")
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn dir_of(path: &str) -> &str {
    path.rfind('/').map_or("", |i| &path[..i])
}

/// `rel` joined to the root-relative directory `dir`, normalized; `None` when
/// it leaves the repository or is absolute.
fn join(dir: &str, rel: &str) -> Option<String> {
    if rel.starts_with('/') || rel.contains('\\') || rel.contains(':') {
        return None;
    }
    let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
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

/// Whether one of the root-relative directories is the other or above it.
fn nested(a: &str, b: &str) -> bool {
    let within = |x: &str, y: &str| {
        y.is_empty() || x == y || x.strip_prefix(y).is_some_and(|rest| rest.starts_with('/'))
    };
    within(a, b) || within(b, a)
}

/// The paths of a well-formed `references` value, in order (absent: none);
/// `None` when TypeScript would report the value.
fn reference_paths(doc: &Value) -> Option<Vec<String>> {
    let Some(v) = doc.get("references") else {
        return Some(Vec::new());
    };
    let mut out = Vec::new();
    for r in v.as_array()? {
        let o = r.as_object()?;
        out.push(o.get("path")?.as_str()?.to_owned());
        if o.iter().any(|(k, v)| {
            k != "path" && !(matches!(k.as_str(), "prepend" | "circular") && v.is_boolean())
        }) {
            return None;
        }
    }
    Some(out)
}

/// The projects of the working tree and of the base, as the tracer's loader
/// loads them.
type Projects = turborepo_trace_lang_js::resolve::governance::Projects;

/// The loader for the two revisions of `changes` (the base: the working tree
/// with the diff's files as they were).
fn projects(root: &Path, changes: &[Change], base: &Git) -> Projects {
    use turborepo_trace_lang_js::resolve::governance::Overlay;
    let overlay = base.reader().map(|read| {
        let root_buf = root.to_path_buf();
        Overlay::new(
            changes
                .iter()
                .map(|c| (root.join(&c.path), c.kind != ChangeKind::Added)),
            Box::new(move |p: &Path| read(&p.strip_prefix(&root_buf).ok()?.to_string_lossy())),
        )
        .with_links(base.links_at_base().iter().map(|l| root.join(l)))
    });
    Projects::new(overlay)
}

/// Why the project that a reference names (`target`: root-relative, as the
/// reference is written), as it is at one revision, may matter to a file of the
/// directory `owner` when a config there lists it under `references`; `Ok` when
/// it cannot (see the module docs, 1).
///
/// What the project is, is asked of the loader the tracer uses: which file the
/// reference names (a file, a directory with a `tsconfig.json`, the name with
/// `.json`) and which files its `extends` entries name (the same three, a
/// package's `exports`, a link). Nothing here resolves a reference or an
/// `extends`: a second set of rules named another file than the loader where
/// the two differed, and cleared an edit that made a project govern a file.
///
/// What the project includes is read in both ways there are. The loader reads
/// an inherited `files` or `include` relative to the project (and by that the
/// graph says which config governs a file); TypeScript, and the parser Vite
/// uses, read it relative to the config that writes it. The project is left out
/// only when it stays apart from the directory of the config under both
/// readings.
fn reference_is_inert(
    root: &Path,
    base: &Git,
    projects: &Projects,
    at_base: bool,
    target: &str,
    owner: &str,
    changed: &dyn Fn(&str) -> bool,
) -> Result<(), String> {
    let name = if target.is_empty() { "." } else { target };
    let Some(p) = projects.load(at_base, &root.join(target)) else {
        return Err("there is no single base revision to load the project at".to_owned());
    };
    if let Some(e) = &p.error {
        // (The loader names the place by its whole path.)
        let e = e.replace(&format!("{}/", root.display()), "");
        return Err(format!("the referenced project {name} does not load: {e}"));
    }
    // A path below the root as a diff names it; `None` outside the repository.
    let rel = |abs: &Path| -> Option<String> {
        let mut out = Vec::new();
        for c in abs.strip_prefix(root).ok()?.components() {
            match c {
                Component::Normal(s) => out.push(s.to_str()?.to_owned()),
                _ => return None,
            }
        }
        Some(out.join("/"))
    };
    let Some(dir) = rel(&p.dir) else {
        return Err(format!(
            "the referenced project {name} is outside the repository"
        ));
    };
    if nested(&dir, owner) {
        return Err(format!(
            "the referenced project {name} is in, above or below the config's directory"
        ));
    }
    // Nothing its load read changes in the same diff.
    let read: Vec<String> = p.read.iter().filter_map(|f| rel(f)).collect();
    if let Some(r) = read.iter().find(|r| changed(r)) {
        return Err(format!("{r} changes in the same diff"));
    }
    let apart = |at: &str| -> Result<(), String> {
        if nested(at, owner) {
            return Err(format!(
                "{name} includes files from {}, which holds or is held by the directory of the \
                 config",
                if at.is_empty() {
                    "the repository root"
                } else {
                    at
                }
            ));
        }
        Ok(())
    };
    // As the loader reads it: its own `files` and `include` or the ones it
    // inherits, made absolute (everything below the project without any). Each
    // pattern, up to its first name with a wildcard, names a directory that the
    // files it includes lie in.
    let own = [p.dir.clone()];
    let patterns: Vec<&PathBuf> = match (&p.files, &p.include) {
        (None, None) => own.iter().collect(),
        (files, include) => files.iter().chain(include.iter()).flatten().collect(),
    };
    for pattern in patterns {
        let mut prefix = PathBuf::new();
        for c in pattern.components() {
            match c {
                Component::Normal(s) if s.to_string_lossy().contains(['*', '?', '{', '[', '$']) => {
                    break;
                }
                Component::ParentDir | Component::CurDir => {
                    return Err(format!(
                        "{name} includes {}, which may leave its directory",
                        rel(pattern).unwrap_or_else(|| pattern.display().to_string())
                    ));
                }
                c => prefix.push(c),
            }
        }
        let Some(at) = rel(&prefix) else {
            return Err(format!(
                "{name} includes files outside the repository ({})",
                pattern.display()
            ));
        };
        apart(&at)?;
    }
    // As TypeScript reads it: the configs the load read, each with the `files` and
    // `include` it writes, relative to itself (`${configDir}`: to the project). The
    // project's own replace inherited ones; without its own, the ones of an
    // extended config apply (which one, when several write them, is not worked
    // out: all are checked).
    let config = rel(&p.config);
    let mut definers: Vec<(String, Vec<String>)> = Vec::new();
    let mut own_defines = false;
    for at in &read {
        if file_name(at) == "package.json" {
            continue;
        }
        // A file of the repository as it was at the revision; what git does not have
        // there (an installed package, a generated file) from the working tree.
        let text = at_base
            .then(|| base.show_config(at))
            .flatten()
            .or_else(|| std::fs::read_to_string(root.join(at)).ok())
            .ok_or_else(|| format!("{at} cannot be read"))?;
        // The loader goes on without a base whose package is not installed; what that
        // base would add is not known.
        if turborepo_trace_lang_js::resolve::governance::extends_uninstalled(&root.join(at), &text)
        {
            return Err(format!(
                "{at}: `extends` names a package that is not installed here"
            ));
        }
        let doc = turborepo_trace_tasks::sources::turbo::parse_jsonc(&text)
            .filter(Value::is_object)
            .ok_or_else(|| format!("{at} does not parse"))?;
        let mut written: Vec<String> = Vec::new();
        let mut defined = false;
        for key in ["files", "include"] {
            match doc.get(key) {
                None => {}
                Some(Value::Array(xs)) => {
                    defined = true;
                    for x in xs {
                        written.push(
                            x.as_str()
                                .ok_or_else(|| format!("{at}: `{key}` is not a list of strings"))?
                                .to_owned(),
                        );
                    }
                }
                Some(_) => return Err(format!("{at}: `{key}` is not a list")),
            }
        }
        if defined {
            let own = config.as_deref() == Some(at.as_str());
            if own {
                definers.clear();
            }
            if own || !own_defines {
                definers.push((at.clone(), written));
            }
            own_defines |= own;
        }
    }
    for (at, patterns) in &definers {
        for pattern in patterns {
            let (from, pattern) = match pattern.strip_prefix("${configDir}") {
                Some(rest) => (dir.as_str(), rest.trim_start_matches('/')),
                None => (dir_of(at), pattern.as_str()),
            };
            apart(from)?;
            if pattern.starts_with('/')
                || pattern.contains("${")
                || pattern.contains('\\')
                || pattern.split('/').any(|s| s == "..")
            {
                return Err(format!(
                    "{name} includes {pattern:?}, which may leave its directory"
                ));
            }
        }
    }
    Ok(())
}

/// Vite plugins that read the project references of tsconfig files (module
/// docs, 2).
const REFERENCE_READERS: &[&str] = &["vite-tsconfig-paths"];

/// Whether the vitest config `config`, or a file of the repository it loads,
/// imports one of [`REFERENCE_READERS`]: the package, or why it cannot be ruled
/// out.
fn reads_references(view: &View<'_>, config: NodeId) -> Option<String> {
    let g = view.base();
    let mut seen = vec![config];
    let mut i = 0;
    while i < seen.len() {
        if seen.len() > 512 {
            return Some("more than 512 files".to_owned());
        }
        let n = seen[i];
        i += 1;
        for e in view.out_edges(n) {
            if !matches!(
                e.kind(),
                RefKind::Import | RefKind::Require | RefKind::DynamicImport | RefKind::ReExport
            ) {
                continue;
            }
            // Labelled edges (`vitest:config <path>`, a generated file) are not imports of
            // the config's code.
            let Some(spec) = e.specifier.filter(|s| !s.contains(' ')) else {
                continue;
            };
            if let Some(p) = crate::configdiff::package_of_spec(spec)
                && REFERENCE_READERS.contains(&p)
            {
                return Some(p.to_owned());
            }
            if let Some(to) = e.to
                && !g.is_external(to)
                && g.lang(to).is_some()
                && !seen.contains(&to)
            {
                seen.push(to);
            }
        }
    }
    None
}

/// A vitest config that reads project references ([`reads_references`]) and
/// whose tests can load one of `readers` (reverse reachability over every
/// edge): its path and what it loads.
fn reference_reader(engine: &Engine<'_, View<'_>>, readers: &[NodeId]) -> Option<String> {
    let view = engine.graph();
    let g = view.base();
    let rev = engine.reverse();
    let mut seen = NodeSet::new(g.node_count());
    let mut queue: Vec<NodeId> = Vec::new();
    for &n in readers {
        if seen.insert(n) {
            queue.push(n);
        }
    }
    let mut checked: HashMap<NodeId, Option<String>> = HashMap::new();
    let mut i = 0;
    while i < queue.len() {
        let n = queue[i];
        i += 1;
        for e in view.out_edges(n) {
            if let (Some(spec), Some(to)) = (e.specifier, e.to)
                && spec.starts_with("vitest:config ")
                && let Some(what) = checked
                    .entry(to)
                    .or_insert_with(|| reads_references(view, to))
            {
                return Some(format!("{} loads {what}", g.path(to)));
            }
        }
        for &s in rev.neighbors(n).0 {
            let s = NodeId(s);
            if seen.insert(s) {
                queue.push(s);
            }
        }
    }
    None
}

/// Applies the modified tsconfig files of `changes` whose `references` are all
/// that differ (see the module docs). `skip` names changes that are handled
/// elsewhere.
// Invariant: `o` is a `json!` object literal.
#[allow(clippy::expect_used)]
pub fn effects(
    engine: &Engine<'_, View<'_>>,
    root: &Path,
    changes: &[Change],
    base: Option<&Git>,
    skip: &dyn Fn(&str) -> bool,
) -> Effects {
    let mut fx = Effects::default();
    let Some(base) = base.filter(|_| enabled()) else {
        return fx;
    };
    let g = engine.graph().base();
    // The base's contents of the modified configs, in one read.
    let modified: Vec<&str> = changes
        .iter()
        .filter(|c| {
            c.kind == ChangeKind::Modified && is_tsconfig(file_name(&c.path)) && !skip(&c.path)
        })
        .map(|c| c.path.as_str())
        .collect();
    base.preload(&modified);
    let changed = |p: &str| changes.iter().any(|c| c.path == p);
    // The loader of referenced projects, made for the first reference that came or
    // went.
    let loader: std::cell::OnceCell<Projects> = std::cell::OnceCell::new();
    for c in changes {
        if c.kind != ChangeKind::Modified || !is_tsconfig(file_name(&c.path)) || skip(&c.path) {
            continue;
        }
        let whole = |fx: &mut Effects, why: String| {
            fx.notes
                .push(format!("tsconfig {}: a whole-file change: {why}", c.path));
        };
        let parse = |text: Option<String>| {
            text.and_then(|t| turborepo_trace_tasks::sources::turbo::parse_jsonc(&t))
                .filter(Value::is_object)
        };
        let (Some(old), Some(new)) = (
            parse(base.show_config(&c.path)),
            parse(std::fs::read_to_string(root.join(&c.path)).ok()),
        ) else {
            whole(&mut fx, "one side does not parse".to_owned());
            continue;
        };
        let (o, n) = (
            old.as_object().expect("object"),
            new.as_object().expect("object"),
        );
        let fields: Vec<&str> = o
            .keys()
            .chain(n.keys())
            .map(String::as_str)
            .filter(|k| o.get(*k) != n.get(*k))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if let Some(other) = fields.iter().find(|f| **f != "references") {
            whole(
                &mut fx,
                format!("`{other}` changed, which a transpile-only run may read"),
            );
            continue;
        }
        let dir = dir_of(&c.path);
        let verdict = (|| -> Result<Vec<String>, String> {
            let (Some(before), Some(after)) = (reference_paths(&old), reference_paths(&new)) else {
                return Err("`references` is not a list of projects at one revision".to_owned());
            };
            // The first referenced project that includes a file wins: the projects both
            // revisions list must keep their order.
            let common = |a: &[String], b: &[String]| -> Vec<String> {
                a.iter().filter(|r| b.contains(r)).cloned().collect()
            };
            if common(&before, &after) != common(&after, &before) {
                return Err("the order of the referenced projects changed".to_owned());
            }
            let mut touched = Vec::new();
            // A project that came is asked about in the working tree, one that went at
            // the base.
            for (list, other, at_base) in [(&after, &before, false), (&before, &after, true)] {
                for r in list.iter().filter(|r| !other.contains(r)) {
                    let target = join(dir, r)
                        .ok_or_else(|| format!("the reference {r:?} leaves the repository"))?;
                    let projects = loader.get_or_init(|| projects(root, changes, base));
                    reference_is_inert(root, base, projects, at_base, &target, dir, &changed)?;
                    if !touched.contains(r) {
                        touched.push(r.clone());
                    }
                }
            }
            Ok(touched)
        })();
        let touched = match verdict {
            Ok(t) => t,
            Err(why) => {
                whole(&mut fx, why);
                continue;
            }
        };
        let cause = || Cause::Modified {
            path: c.path.clone(),
        };
        let mut readers = Vec::new();
        engine.match_inputs(&c.path, &mut |n| readers.push(n));
        readers.sort_unstable();
        readers.dedup();
        if let Some(reader) = reference_reader(engine, &readers) {
            whole(
                &mut fx,
                format!(
                    "{reader}, a Vite plugin that reads project references, and its tests can \
                     load a file that resolves through the config"
                ),
            );
            continue;
        }
        if let Some(node) = g.node_by_path(&c.path) {
            fx.seeds.push((node, cause()));
        }
        fx.type_seeds.extend(readers.iter().map(|&n| (n, cause())));
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
            "tsconfig {}: {}: not an input change for tests that only transpile; the {} node(s) \
             that resolve through it are seeded for type-checking and build entries only",
            c.path,
            if touched.is_empty() {
                "only comments or formatting changed".to_owned()
            } else {
                format!(
                    "only `references` changed ({}), and no referenced project that came or went \
                     can include a file of {}",
                    touched.join(", "),
                    if dir.is_empty() {
                        "the repository root"
                    } else {
                        dir
                    }
                )
            },
            readers.len()
        ));
    }
    fx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_paths() {
        for f in ["tsconfig.json", "tsconfig.build.json", "jsconfig.json"] {
            assert!(is_tsconfig(f), "{f}");
        }
        for f in ["tsconfig.ts", "package.json", "my-tsconfig.json"] {
            assert!(!is_tsconfig(f), "{f}");
        }
        assert_eq!(
            join("packages/gate", "../otel").as_deref(),
            Some("packages/otel")
        );
        assert_eq!(
            join("packages/gate", "./sub/x.json").as_deref(),
            Some("packages/gate/sub/x.json")
        );
        assert_eq!(join("", "a").as_deref(), Some("a"));
        assert_eq!(join("a", "../.."), None);
        assert_eq!(join("a", "/abs"), None);
        assert!(nested("packages/gate", "packages/gate"));
        assert!(nested("packages/gate/sub", "packages/gate"));
        assert!(nested("packages", "packages/gate"));
        assert!(nested("", "packages/gate"));
        assert!(!nested("packages/gate", "packages/otel"));
        assert!(!nested("packages/gate", "packages/gate-2"));
    }

    #[test]
    fn well_formed_references() {
        let paths = |json: &str| reference_paths(&serde_json::from_str(json).unwrap());
        assert_eq!(paths("{}"), Some(Vec::new()));
        assert_eq!(paths(r#"{"references":[]}"#), Some(Vec::new()));
        assert_eq!(
            paths(r#"{"references":[{"path":"../b"},{"path":"../a","prepend":false}]}"#),
            Some(vec!["../b".to_owned(), "../a".to_owned()])
        );
        for bad in [
            r#"{"references":{}}"#,
            r#"{"references":["../a"]}"#,
            r#"{"references":[{}]}"#,
            r#"{"references":[{"path":1}]}"#,
            r#"{"references":[{"path":"../a","extra":1}]}"#,
            r#"{"references":[{"path":"../a","prepend":"yes"}]}"#,
        ] {
            assert_eq!(paths(bad), None, "{bad}");
        }
    }
}
