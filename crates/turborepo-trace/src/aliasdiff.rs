//! `affected --since`: an alias entry of a vitest config.
//!
//! A test depends on its runner config as a whole: every test of a project has
//! a `vitest:config` edge to the config file, so any edit of the file selects
//! the project's tests. One kind of edit says exactly which tests can observe
//! it: an entry of `resolve.alias` (or `test.alias`). An alias rewrites import
//! requests. A test whose modules make no request that the old and the new
//! alias table answer differently loads the same modules as before.
//!
//! [`effects`] applies a modified vitest config that way when all of this
//! holds:
//!
//! 1. The edit is confined to one alias value. With the values of the `alias`
//!    properties blanked, the base revision and the working tree have the same
//!    runtime JavaScript ([`crate::jsblank`]: both parse, nothing else in the
//!    module reads an `alias` property), and exactly one of those values
//!    differs, appears or goes away. A `resolve: { alias }` block that holds
//!    nothing else counts as its alias, and so does an added import of Node's
//!    `url` or `path` (the new value's `fileURLToPath(new URL('./stub.ts',
//!    import.meta.url))`).
//! 2. The adapter read both tables. `turborepo_trace_frameworks` loads both
//!    revisions (the base one with the file's old contents in place); what it
//!    makes of them is equal except for the aliases, the aliases differ, and
//!    neither revision has a value it could not read in an alias field. With
//!    (1), the value that changed is the one the runner uses.
//! 3. Dependencies are not run through vite. By default vitest loads the
//!    packages of `node_modules` natively, where aliases do not apply; the
//!    graph records the requests of the repository's own files. A config that
//!    inlines dependencies (`inline`, `noExternal`, an `optimizer`) is not
//!    handled here.
//!
//! Then the tests of the config are selected through it only if they can load a
//! file with an *affected request*: a recorded import specifier (resolved or
//! not, also of a mock) that an added, removed or changed alias matches, or
//! that the two tables rewrite differently (a reordering). "Can load" is
//! reachability over every edge of the graph, which errs on the side of
//! selecting.
//!
//! The rest of what the file change does stays: other nodes that depend on the
//! file (a task that reads it, a module that imports it), other inputs and
//! covering uncertain reads are seeded. The config is also a definition file of
//! the package script that runs the tests ([`crate::workflows`]); for a change
//! applied here it does not count as one, since what the script's tests can
//! observe of it is exactly the alias.
//!
//! jest's `moduleNameMapper` is not handled: jest applies it to every module it
//! loads, including the files of dependencies, whose requests the graph does
//! not record.

use std::{
    collections::{BTreeSet, HashMap},
    path::Path,
};

use turborepo_trace_frameworks::inputs::{Alias, FrameworkInputs, ResolutionOverrides};
use turborepo_trace_graph::{Cause, Change, ChangeKind, Engine, GraphView, NodeId, NodeSet};
use turborepo_trace_lang_js::extract::config::ConfigVal;
use turborepo_trace_store::Graph;

use crate::lockdiff::Git;

/// The graph the CLI's engine runs on (node ids are the store graph's).
type View<'g> = turborepo_trace_graph::Patched<'g, Graph>;

/// The properties that hold vite aliases (`resolve.alias`, `test.alias`).
const ALIAS_PROPERTIES: &[&str] = &["alias"];

/// Node built-ins whose import has no effect of its own.
const PURE_BUILTINS: &[&str] = &["node:url", "url", "node:path", "path"];

/// Config keys that make vitest run dependencies through vite.
const INLINING_KEYS: &[&str] = &["inline", "noExternal", "optimizer"];

/// What the alias edits of vitest configs change in an `affected` query.
#[derive(Debug, Default)]
pub struct Effects {
    /// File changes that are applied through `seeds` instead.
    pub drop: BTreeSet<String>,
    /// The nodes to seed, each with its cause.
    pub seeds: Vec<(NodeId, Cause)>,
    /// Readable notes for `--stats`.
    pub notes: Vec<String>,
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// `v` without the aliases and without source lines (an entry more shifts every
/// line below it).
fn comparable(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(o) => {
            o.remove("aliases");
            o.remove("line");
            for x in o.values_mut() {
                comparable(x);
            }
        }
        serde_json::Value::Array(xs) => xs.iter_mut().for_each(comparable),
        _ => {}
    }
}

/// Whether `v` has a key of `keys` anywhere.
fn has_key(v: &ConfigVal, keys: &[&str], depth: u32) -> bool {
    if depth > 16 {
        return true;
    }
    match v {
        ConfigVal::Obj(o) => o
            .entries
            .iter()
            .any(|(k, x)| keys.contains(&k.as_str()) || has_key(x, keys, depth + 1)),
        ConfigVal::Arr(xs) | ConfigVal::Alts(xs) => xs.iter().any(|x| has_key(x, keys, depth + 1)),
        _ => false,
    }
}

/// The alias tables of two revisions of one config, per project whose table
/// differs: what the runner makes of both is otherwise equal. The error says
/// why the change is not an alias change the adapter can vouch for.
fn alias_tables<'a>(
    old: &'a FrameworkInputs,
    new: &'a FrameworkInputs,
) -> Result<Vec<(&'a ResolutionOverrides, &'a ResolutionOverrides)>, String> {
    if old.projects.len() != new.projects.len() {
        return Err("the projects differ".to_owned());
    }
    let unread = |i: &FrameworkInputs| {
        i.uncertainties
            .iter()
            .chain(i.projects.iter().flat_map(|p| &p.uncertainties))
            .any(|u| u.field.as_deref().is_some_and(|f| f.contains("alias")))
    };
    if unread(old) || unread(new) {
        return Err("an alias the adapter could not read".to_owned());
    }
    let value = |i: &FrameworkInputs| {
        let mut v = serde_json::to_value(i).map_err(|e| e.to_string())?;
        comparable(&mut v);
        Ok::<_, String>(v)
    };
    if value(old)? != value(new)? {
        return Err("the runner's inputs differ outside the aliases".to_owned());
    }
    let pairs: Vec<_> = old
        .projects
        .iter()
        .zip(&new.projects)
        .filter(|(o, n)| o.resolution.aliases != n.resolution.aliases)
        .map(|(o, n)| (&o.resolution, &n.resolution))
        .collect();
    if pairs.is_empty() {
        return Err("the adapter reads the same aliases at both revisions".to_owned());
    }
    Ok(pairs)
}

/// Which requests an alias change affects.
struct Changed<'a> {
    /// The alias tables that differ, base and head.
    tables: Vec<(&'a ResolutionOverrides, &'a ResolutionOverrides)>,
    /// The aliases of one side only, each as a table of its own.
    entries: Vec<ResolutionOverrides>,
}

impl<'a> Changed<'a> {
    fn new(tables: Vec<(&'a ResolutionOverrides, &'a ResolutionOverrides)>) -> Self {
        let mut entries: Vec<ResolutionOverrides> = Vec::new();
        let mut one_sided = |a: &[Alias], b: &[Alias]| {
            for x in a.iter().filter(|x| !b.contains(x)) {
                entries.push(ResolutionOverrides {
                    aliases: vec![x.clone()],
                    ..ResolutionOverrides::default()
                });
            }
        };
        for (o, n) in &tables {
            one_sided(&o.aliases, &n.aliases);
            one_sided(&n.aliases, &o.aliases);
        }
        Changed { tables, entries }
    }

    /// Whether the two revisions may answer `request` differently: an alias of
    /// one side only matches it, or the tables rewrite it differently.
    fn affects(&self, request: &str) -> bool {
        self.entries.iter().any(|e| e.apply(request).is_some())
            || self.tables.iter().any(|(o, n)| {
                o.apply(request).map(|h| h.requests) != n.apply(request).map(|h| h.requests)
            })
    }
}

/// The request a recorded specifier stands for: the import as written.
/// Framework edges are labelled (`vitest:resolve.alias <request> ->
/// <rewritten>`, `vitest:mock <request>`).
fn request_of(spec: &str) -> &str {
    let rest = match spec.split_once(' ') {
        Some((label, rest))
            if label.contains(':')
                && label.bytes().all(|b| {
                    b.is_ascii_alphanumeric() || matches!(b, b':' | b'_' | b'-' | b'.')
                }) =>
        {
            rest
        }
        _ => spec,
    };
    rest.split_once(" -> ").map_or(rest, |(r, _)| r).trim()
}

/// Among `tests`, the ones that reach a file with a request `changed` affects
/// (over every edge of the graph), sorted; and the number of such files.
fn reaching(
    engine: &Engine<'_, View<'_>>,
    tests: &[NodeId],
    changed: &Changed<'_>,
) -> (Vec<NodeId>, usize) {
    let view = engine.graph();
    let g = view.base();
    let n = g.node_count();
    let fwd = engine.forward();
    // What the tests can load.
    let mut seen = NodeSet::new(n);
    let mut todo: Vec<NodeId> = Vec::new();
    for &t in tests {
        if seen.insert(t) {
            todo.push(t);
        }
    }
    let mut memo: HashMap<&str, bool> = HashMap::new();
    let mut affected: Vec<NodeId> = Vec::new();
    while let Some(v) = todo.pop() {
        if !turborepo_trace_store::tasks::is_pseudo_lang(g.lang(v)) && !g.is_external(v) {
            let hit = view.out_edges(v).any(|e| {
                e.specifier.is_some_and(|spec| {
                    let request = request_of(spec);
                    !request.is_empty()
                        && *memo
                            .entry(request)
                            .or_insert_with(|| changed.affects(request))
                })
            });
            if hit {
                affected.push(v);
            }
        }
        for &s in fwd.neighbors(v).0 {
            let s = NodeId(s);
            if seen.insert(s) {
                todo.push(s);
            }
        }
    }
    // Who reaches such a file.
    let rev = engine.reverse();
    let mut reach = NodeSet::new(n);
    let mut todo = Vec::new();
    for &f in &affected {
        if reach.insert(f) {
            todo.push(f);
        }
    }
    while let Some(v) = todo.pop() {
        for &p in rev.neighbors(v).0 {
            let p = NodeId(p);
            if reach.insert(p) {
                todo.push(p);
            }
        }
    }
    let mut out: Vec<NodeId> = tests
        .iter()
        .copied()
        .filter(|t| reach.contains(*t))
        .collect();
    out.sort_unstable();
    (out, affected.len())
}

/// Whether the edit of the vitest config at `path` is an alias edit (see the
/// module docs): the adapter's inputs for both revisions. The error says why
/// not.
fn alias_edit(
    root: &Path,
    path: &str,
    old: &str,
    new: &str,
) -> Result<(FrameworkInputs, FrameworkInputs), String> {
    let Some(fw) = turborepo_trace_frameworks::framework("vitest") else {
        return Err("no vitest adapter".to_owned());
    };
    // An import of a Node built-in that only the head revision has loads nothing of
    // the repository and binds a name nothing else can use (a second
    // declaration of a name is an early error, which the comparison below
    // rejects): the new alias value uses it.
    crate::erase::canonical(path, new).map_err(|e| format!("head: {e}"))?;
    let head = crate::jsblank::without_imports(path, new, &|module, text| {
        PURE_BUILTINS.contains(&module) && !old.contains(text)
    })?;
    let sites = crate::jsblank::Sites {
        props: ALIAS_PROPERTIES,
        what: crate::jsblank::What::Values,
        // `resolve: { alias }` and no `resolve` at all are the same vite config once the
        // aliases are accounted for.
        wrapper: Some("resolve"),
    };
    let (b, h) = crate::jsblank::equal_outside(path, old, &head, sites)?;
    // Exactly one alias value changed, appeared or went away.
    let only = |x: &[String], y: &[String]| {
        let mut y: Vec<&String> = y.iter().collect();
        x.iter()
            .filter(|s| match y.iter().position(|t| t == s) {
                Some(i) => {
                    y.swap_remove(i);
                    false
                }
                None => true,
            })
            .count()
    };
    let (gone, come) = (only(&b.sites, &h.sites), only(&h.sites, &b.sites));
    if gone > 1 || come > 1 || gone + come == 0 {
        return Err("not exactly one alias value differs".to_owned());
    }
    let abs = root.join(path);
    // Dependencies run through vite make requests the graph does not record.
    let loaded = turborepo_trace_frameworks::jsconfig::load(&abs, Default::default());
    let inlines = INLINING_KEYS.iter().any(|k| new.contains(k))
        || loaded
            .value
            .as_ref()
            .is_none_or(|v| has_key(v, INLINING_KEYS, 0));
    if inlines {
        return Err("the config may run dependencies through vite".to_owned());
    }
    let head = fw.load(&abs);
    // The same for every other config file the adapter read (a base config it
    // merges).
    let others = head
        .config_file
        .iter()
        .chain(
            head.projects
                .iter()
                .flat_map(|p| p.config_file.iter().chain(p.global.config_files.iter())),
        )
        .filter(|f| **f != abs);
    for f in others {
        let inlines = std::fs::read_to_string(f)
            .is_ok_and(|text| INLINING_KEYS.iter().any(|k| text.contains(k)));
        if inlines {
            return Err("a config it loads may run dependencies through vite".to_owned());
        }
    }
    let base = turborepo_trace_frameworks::fsx::with_contents(
        vec![(abs.clone(), old.as_bytes().to_vec())],
        || fw.load(&abs),
    );
    Ok((base, head))
}

/// Applies the modified vitest configs of `changes` whose edits are alias
/// entries (see the module docs). `skip` names changes that are handled
/// elsewhere.
pub fn effects(
    engine: &Engine<'_, View<'_>>,
    root: &Path,
    changes: &[Change],
    base: Option<&Git>,
    skip: &dyn Fn(&str) -> bool,
) -> Effects {
    let mut fx = Effects::default();
    let Some(base) = base.filter(|_| crate::configdiff::enabled()) else {
        return fx;
    };
    let Some(fw) = turborepo_trace_frameworks::framework("vitest") else {
        return fx;
    };
    let view = engine.graph();
    let g = view.base();
    for c in changes {
        let name = file_name(&c.path);
        if c.kind != ChangeKind::Modified
            || !fw.config_file_names().contains(&name)
            || name.ends_with(".json")
            || skip(&c.path)
        {
            continue;
        }
        let Some(node) = g.node_by_path(&c.path) else {
            continue;
        };
        // The tests the file configures, and what else has an edge to it.
        let mut tests: Vec<NodeId> = Vec::new();
        let mut others: Vec<NodeId> = Vec::new();
        let mut prev = None;
        for &s in engine.reverse().neighbors(node).0 {
            if prev == Some(s) {
                continue;
            }
            prev = Some(s);
            let src = NodeId(s);
            let (mut config, mut other) = (false, false);
            for e in view.out_edges(src).filter(|e| e.to == Some(node)) {
                if e.specifier.is_some_and(|s| s.starts_with("vitest:config ")) {
                    config = true;
                } else {
                    other = true;
                }
            }
            if config && !other {
                tests.push(src);
            } else {
                others.push(src);
            }
        }
        if tests.is_empty() {
            continue;
        }
        tests.sort_unstable();
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
        let whole = |fx: &mut Effects, why: String| {
            fx.notes.push(format!(
                "runner config {}: a change for each of its {} test(s): {why}",
                c.path,
                tests.len()
            ));
        };
        let (old_inputs, new_inputs) = match alias_edit(root, &c.path, &old, &new) {
            Ok(x) => x,
            Err(why) => {
                whole(&mut fx, why);
                continue;
            }
        };
        let tables = match alias_tables(&old_inputs, &new_inputs) {
            Ok(t) => t,
            Err(why) => {
                whole(&mut fx, why);
                continue;
            }
        };
        let changed = Changed::new(tables);
        // The tests this file governs. A test of another config that merges this one
        // (the file is a helper there) gets what that config makes of it, which
        // was not looked at: it stays selected.
        let abs = root.join(&c.path);
        let (governed, merged): (Vec<NodeId>, Vec<NodeId>) = tests.iter().partition(|t| {
            fw.config_for(&root.join(g.path(**t)))
                .is_some_and(|cf| cf == abs)
        });
        let (selected, files) = reaching(engine, &governed, &changed);
        // Everything the file change does, except the config edge of those tests.
        let modified = || Cause::Modified {
            path: c.path.clone(),
        };
        for &n in others.iter().chain(&merged).chain(&selected) {
            fx.seeds.push((n, modified()));
        }
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
            "runner config {}: only alias entries changed ({} alias(es) of one revision only): {} \
             of its {} test(s) can load one of the {} file(s) with an affected request{}",
            c.path,
            changed.entries.len(),
            selected.len(),
            governed.len(),
            files,
            if merged.is_empty() {
                String::new()
            } else {
                format!(
                    "; {} test(s) of configs that merge it stay selected",
                    merged.len()
                )
            }
        ));
    }
    fx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests() {
        assert_eq!(request_of("server-only"), "server-only");
        assert_eq!(request_of("./a b"), "./a b");
        assert_eq!(
            request_of("vitest:resolve.alias server-only -> packages/x/stub.ts"),
            "server-only"
        );
        assert_eq!(request_of("vitest:mock @/lib/db"), "@/lib/db");
        assert_eq!(
            request_of("vitest:config packages/x/vitest.config.ts"),
            "packages/x/vitest.config.ts"
        );
    }

    #[test]
    fn inlining_keys() {
        use turborepo_trace_lang_js::extract::config::ConfigObj;
        let obj = |entries: Vec<(&str, ConfigVal)>| {
            ConfigVal::Obj(ConfigObj {
                entries: entries
                    .into_iter()
                    .map(|(k, v)| (k.to_owned(), v))
                    .collect(),
                open: None,
            })
        };
        let plain = obj(vec![(
            "test",
            obj(vec![("globals", ConfigVal::Bool(true))]),
        )]);
        assert!(!has_key(&plain, INLINING_KEYS, 0));
        let inlined = obj(vec![(
            "test",
            obj(vec![(
                "server",
                obj(vec![(
                    "deps",
                    obj(vec![("inline", ConfigVal::Arr(vec![]))]),
                )]),
            )]),
        )]);
        assert!(has_key(&inlined, INLINING_KEYS, 0));
    }
}
