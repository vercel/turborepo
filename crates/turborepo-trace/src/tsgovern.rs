//! `affected --since`: a tsconfig project that governed a file at the base and
//! does not govern it now.
//!
//! The query reads the graph of the working tree. There, a project that a
//! config above a file lists under `references` is an input of the file only
//! when it governs it (the file's alias imports resolve through its `paths`) or
//! does not load. A project that loads and does not include the file is none:
//! an edit of a sibling package's tsconfig selects nothing through the files
//! below the referencing config. The file keeps what the project's load read as
//! a **key observation** (`turborepo_trace_core::record::key`), which no query
//! selects through.
//!
//! That leaves out the change that takes the file away from the project. At the
//! base the project included it, its imports resolved through the project's
//! `paths`; now the project's `include` (or `files`, `exclude`, `allowJs`, or a
//! config it extends) says otherwise and another config governs the file. The
//! project is no input of the file in the graph, and before this rule nothing
//! was selected. The same holds for a project that did not load at the base and
//! loads now without including the file: the file carried the load failure at
//! the base (a transpiler that parses every referenced config fails on it), and
//! nothing of that is in the graph.
//!
//! # The rule
//!
//! For every changed path that a file has among its key observations
//! ([`turborepo_trace_store::Graph::key_observers`]):
//!
//! 1. A modified config whose `extends`, `files`, `include`, `exclude` and
//!    `compilerOptions.allowJs` are the same at both revisions, and that the
//!    resolver's parser accepts at both, loads or fails as before and covers
//!    the same files
//!    (`turborepo_trace_lang_js::resolve::governance::governing_fields`):
//!    nothing is seeded. `paths`, `baseUrl` and every other option of a project
//!    only matter to the files it governs, and it is an input of those. A
//!    `package.json` (the manifest of a package a config extends through) with
//!    the same resolution view is the same to the resolver.
//! 2. Otherwise config discovery is run for each of those files on the base
//!    revision and on the working tree, by the code the tracer runs
//!    (`turborepo_trace_lang_js::resolve::governance::Compare`; the base is the
//!    working tree with the diff's files as they were). A file whose governing
//!    config differs, or for which the load failures of the configs it consults
//!    differ, is seeded as changed. The other files that consulted the project
//!    are not.
//! 3. A third-party package that changed (a lockfile) and that such a project's
//!    `extends` chain goes through: its contents at the base are not in the
//!    tree, so every file that observed it is seeded. So is every file that
//!    observed a name which the lockfile makes resolve to something else for an
//!    importer. The files of a workspace package are paths of the tree: its
//!    name alone seeds nothing.
//!
//! Without a single base to read (no `--since`, several merge bases) step 2
//! cannot run: every file that observed a changed path that step 1 does not
//! clear is seeded.
//!
//! So an edit of a referenced project's config selects, besides what the
//! project governs now (its input): for `include` / `files` / `exclude` /
//! `allowJs` / `extends`, the dependents of the files that the project governed
//! at the base and does not govern now; for a config that starts or stops
//! loading, the dependents of every file below the configs that reference it;
//! for anything else, nothing more.
//!
//! `TURBO_TRACE_DEBUG_TSCONFIG_GOVERNED=0` turns the rule off (for A/B
//! measurements: without it the selection is unsound for these changes).

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use rayon::prelude::*;
use turborepo_trace_graph::{Cause, Change, ChangeKind, Engine, GraphView, NodeId};
use turborepo_trace_lang_js::resolve::governance::{Compare, Moved, Overlay, governing_fields};
use turborepo_trace_store::Graph;

use crate::lockdiff::Git;

/// The graph the CLI's engine runs on (node ids are the store graph's).
type View<'g> = turborepo_trace_graph::Patched<'g, Graph>;

/// What the rule adds to an `affected` query.
#[derive(Debug, Default)]
pub struct Effects {
    /// The files to seed, each with its cause.
    pub seeds: Vec<(NodeId, Cause)>,
    /// Readable notes for `--stats` and `--explain`.
    pub notes: Vec<String>,
}

/// Whether the rule is on (`TURBO_TRACE_DEBUG_TSCONFIG_GOVERNED=0` turns it
/// off).
pub fn enabled() -> bool {
    turborepo_trace_core::debug::var_os("TSCONFIG_GOVERNED").is_none_or(|v| v != "0")
}

/// A changed package: its name and the change that says so.
pub struct ChangedPackage<'a> {
    /// The package name.
    pub name: &'a str,
    /// The name resolves to something else for an importer (a workspace link
    /// that became a registry version): it counts for a first-party link
    /// too. Otherwise the contents of a third-party package changed; a
    /// workspace package's files are in the diff.
    pub relinked: bool,
    /// The cause to seed with.
    pub cause: Cause,
}

/// Whether the change of `c` cannot move what a project includes or whether it
/// loads (module docs, 1). `old`: its contents at the base.
fn inert(root: &Path, c: &Change, old: Option<&[u8]>) -> bool {
    if c.kind != ChangeKind::Modified {
        return false;
    }
    let (Some(old), Ok(new)) = (old, std::fs::read(root.join(&c.path))) else {
        return false;
    };
    if c.path.rsplit('/').next() == Some("package.json") {
        let view = turborepo_trace_core::manifest::view;
        return matches!((view(old), view(&new)), (Some(a), Some(b)) if a == b);
    }
    let fields = |b: &[u8]| std::str::from_utf8(b).ok().and_then(governing_fields);
    matches!((fields(old), fields(&new)), (Some(a), Some(b)) if a == b)
}

fn rel<'a>(root: &Path, p: &'a Path) -> std::borrow::Cow<'a, str> {
    p.strip_prefix(root).unwrap_or(p).to_string_lossy()
}

/// One sentence on how the discovery of a file differs.
fn describe(root: &Path, m: &Moved) -> String {
    let config = |c: &Option<PathBuf>| match c {
        Some(c) => rel(root, c).into_owned(),
        None => "no config".to_owned(),
    };
    let mut parts = Vec::new();
    if m.base != m.head {
        parts.push(format!(
            "governed by {} at the base and by {} now",
            config(&m.base),
            config(&m.head)
        ));
    }
    if !m.loads.is_empty() {
        parts.push(format!(
            "{} loads with other failures than at the base",
            m.loads
                .iter()
                .map(|c| rel(root, c).into_owned())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    parts.join("; ")
}

/// Applies the rule (see the module docs) to `changes`. `packages`: the
/// third-party packages that changed.
pub fn effects(
    engine: &Engine<'_, View<'_>>,
    root: &Path,
    changes: &[Change],
    base: Option<&Git>,
    packages: &[ChangedPackage<'_>],
) -> Effects {
    let mut fx = Effects::default();
    if !enabled() {
        return fx;
    }
    let g = engine.graph().base();
    if !g.has_key_observations() {
        return fx;
    }
    let mut seeded: Vec<bool> = Vec::new();
    let mut seed = |fx: &mut Effects, n: NodeId, cause: Cause| {
        if seeded.len() <= n.index() {
            seeded.resize(n.index() + 1, false);
        }
        if !std::mem::replace(&mut seeded[n.index()], true) {
            fx.seeds.push((n, cause));
        }
    };
    let cause = |c: &Change| match c.kind {
        ChangeKind::Deleted => Cause::Deleted {
            path: c.path.clone(),
        },
        _ => Cause::Modified {
            path: c.path.clone(),
        },
    };

    // 3. Third-party packages.
    let names: Vec<(&str, bool)> = packages.iter().map(|p| (p.name, p.relinked)).collect();
    let mut by_package: BTreeMap<u32, usize> = BTreeMap::new();
    for (n, i) in g.key_package_observers(&names) {
        *by_package.entry(i).or_default() += 1;
        seed(&mut fx, n, packages[i as usize].cause.clone());
    }
    for (i, files) in by_package {
        fx.notes.push(format!(
            "tsconfig: the package {} changed, and a referenced tsconfig project extends a config \
             of it: the {files} file(s) below the configs that reference the project are seeded \
             (what the package held at the base is not in the tree)",
            names[i as usize].0
        ));
    }

    // The changed paths that files observed, with their observers.
    let paths: Vec<&str> = changes.iter().map(|c| c.path.as_str()).collect();
    let mut observers: BTreeMap<u32, Vec<NodeId>> = BTreeMap::new();
    for (n, i) in g.key_observers(&paths) {
        observers.entry(i).or_default().push(n);
    }
    turborepo_trace_core::timing::mark("affected: tsconfig key observers");
    if observers.is_empty() {
        return fx;
    }
    // The base's contents of the modified ones, in one read (and what the field
    // rule of `tsconfigdiff` read of them is not read again).
    if let Some(git) = base {
        let modified: Vec<&str> = observers
            .keys()
            .map(|&i| &changes[i as usize])
            .filter(|c| c.kind == ChangeKind::Modified)
            .map(|c| c.path.as_str())
            .collect();
        git.preload(&modified);
    }
    let reader = base.and_then(Git::reader);
    turborepo_trace_core::timing::mark("affected: tsconfig base contents");
    // 1. Changes that cannot move a project.
    let mut moved: Vec<u32> = Vec::new();
    // What was read of the base on the way: the comparison reads it again
    // otherwise.
    let mut read_at_base: Vec<(u32, Vec<u8>)> = Vec::new();
    for (&i, files) in &observers {
        let c = &changes[i as usize];
        let old = match (&reader, c.kind) {
            (Some(read), ChangeKind::Modified) => read(&c.path),
            _ => None,
        };
        let cleared = inert(root, c, old.as_deref());
        if let Some(old) = old {
            read_at_base.push((i, old));
        }
        if cleared {
            fx.notes.push(format!(
                "tsconfig {}: {} file(s) consulted it as a project that a config above them \
                 references, without being governed by it; its `extends`, `files`, `include`, \
                 `exclude` and `allowJs` are as at the base, so it includes the same files and \
                 loads as before: nothing is selected through them",
                c.path,
                files.len()
            ));
        } else {
            moved.push(i);
        }
    }
    if moved.is_empty() {
        return fx;
    }
    // 2. Discovery at both revisions, or every observer without a base to read.
    let compare = reader.map(|read| {
        let root_buf = root.to_path_buf();
        let overlay = Overlay::new(
            changes
                .iter()
                .map(|c| (root.join(&c.path), c.kind != ChangeKind::Added)),
            Box::new(move |p: &Path| read(&p.strip_prefix(&root_buf).ok()?.to_string_lossy())),
        )
        // The links of the base among the changed paths lead where they led then.
        .with_links(
            base.map(Git::links_at_base)
                .unwrap_or_default()
                .iter()
                .map(|l| root.join(l)),
        );
        for (i, bytes) in read_at_base {
            overlay.preload(&root.join(&changes[i as usize].path), bytes);
        }
        Compare::new(overlay)
    });
    // Per file that observed one of them: how its discovery differs, if it does.
    let mut files: Vec<NodeId> = moved
        .iter()
        .flat_map(|i| observers[i].iter().copied())
        .collect();
    files.sort_unstable_by_key(|n| n.0);
    files.dedup();
    let verdicts: BTreeMap<u32, String> = files
        .par_iter()
        .filter_map(|&n| {
            let why = match &compare {
                Some(cmp) => describe(root, &cmp.moved(&root.join(g.path(n)))?),
                None => "there is no single base revision to compare with".to_owned(),
            };
            Some((n.0, why))
        })
        .collect();
    turborepo_trace_core::timing::mark("affected: tsconfig discovery at the base and now");
    for &i in &moved {
        let c = &changes[i as usize];
        let (mut count, mut first) = (0usize, None::<String>);
        for &n in &observers[&i] {
            let Some(why) = verdicts.get(&n.0) else {
                continue;
            };
            count += 1;
            if first.is_none() {
                first = Some(format!("{}: {why}", g.path(n)));
            }
            seed(&mut fx, n, cause(c));
        }
        fx.notes.push(match first {
            Some(first) => format!(
                "tsconfig {}: a project that a config references, or a config such a project \
                 extends, changed in what it includes or whether it loads: {count} of the {} \
                 file(s) that consulted it without being governed by it are governed differently \
                 than at the base and are seeded as changed ({first})",
                c.path,
                observers[&i].len()
            ),
            None => format!(
                "tsconfig {}: a project that a config references, or a config such a project \
                 extends, changed in what it includes or whether it loads: none of the {} file(s) \
                 that consulted it without being governed by it is governed differently than at \
                 the base",
                c.path,
                observers[&i].len()
            ),
        });
    }
    fx
}
