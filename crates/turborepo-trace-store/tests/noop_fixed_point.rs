//! A refresh of an unchanged tree is a fixed point: it reuses every shard and
//! every shard's observed tree state, traces nothing, and leaves every file and
//! directory below the graph directory exactly as it was.
//!
//! The general check: for every fixture tree (the task, framework and impact
//! fixtures, the host-paths repository and the synthetic monorepo of
//! `reuse.rs`) and for every place a graph directory can be (next to the
//! repository, `<root>/.turbotrace`, a directory of another name inside the
//! root), build, refresh, refresh. The trees run on several threads at once, as
//! builds of different graphs in one process do.
//!
//! Regression tests for:
//! - a build that did not wait for its own background write of the observation
//!   cache when another build in the process waited at the same moment (the
//!   next refresh found no cache and checked every tree part again; a rare
//!   failure of `reuse.rs` under load);
//! - a graph directory inside the root under another name than `.turbotrace`:
//!   deep membership listings entered it and project discovery probed below it,
//!   so every build changed what the next one observed (the tracer's file
//!   system now hides it);
//! - `graph.json` rewritten by every refresh.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

mod support;

use support::monorepo::{Monorepo, backdate};
use turborepo_trace_core::Language;
use turborepo_trace_store::{BuildReport, GraphOptions, build};

fn langs() -> Vec<Arc<dyn Language>> {
    vec![Arc::new(turborepo_trace_lang_js::JsLanguage)]
}

fn rebuild(root: &Path, out: &Path) -> BuildReport {
    let opts = GraphOptions {
        root: root.to_path_buf(),
        out: Some(out.to_path_buf()),
        threads: 2,
        ..Default::default()
    };
    let r = build(&opts, &langs()).unwrap();
    // The observation cache is written in the background.
    turborepo_trace_store::wait_for_cache_writes_in(out);
    r
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let t = to.join(e.file_name());
        let ty = e.file_type().unwrap();
        if ty.is_dir() {
            copy_tree(&e.path(), &t);
        } else if ty.is_symlink() {
            turborepo_trace_core::os::symlink(std::fs::read_link(e.path()).unwrap(), &t).unwrap();
        } else {
            std::fs::copy(e.path(), &t).unwrap();
        }
    }
}

/// Every file and directory below `dir`: path, kind, size (files) and mtime.
fn snapshot(dir: &Path) -> Vec<(PathBuf, bool, u64, std::time::SystemTime)> {
    fn walk(d: &Path, out: &mut Vec<(PathBuf, bool, u64, std::time::SystemTime)>) {
        let md = std::fs::symlink_metadata(d).unwrap();
        let is_dir = md.is_dir();
        out.push((
            d.to_path_buf(),
            is_dir,
            if is_dir { 0 } else { md.len() },
            md.modified().unwrap(),
        ));
        if is_dir {
            for e in std::fs::read_dir(d).unwrap() {
                walk(&e.unwrap().path(), out);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out.sort();
    out
}

/// Where the graph directory is.
#[derive(Copy, Clone, Debug)]
enum Place {
    /// Next to the repository.
    Outside,
    /// `<root>/.turbotrace`, the default.
    Default,
    /// A directory of another name inside the root, not ignored.
    Named,
}

impl Place {
    fn dir(self, root: &Path) -> PathBuf {
        match self {
            Place::Outside => root.with_file_name("graph"),
            Place::Default => root.join(".turbotrace"),
            Place::Named => root.join("graph-cache"),
        }
    }
}

/// What a refresh did that a refresh of an unchanged tree must not do (empty:
/// nothing).
fn not_a_fixed_point(
    r: &BuildReport,
    first: &BuildReport,
    before: &[(PathBuf, bool, u64, std::time::SystemTime)],
    out: &Path,
) -> Vec<String> {
    let mut v = Vec::new();
    if !r.walk_cached {
        v.push("the tree was walked again".to_owned());
    }
    if r.traced != 0 || r.written != 0 || r.analyzed_files != 0 {
        v.push(format!(
            "{} shard(s) traced, {} written, {} file(s) analyzed",
            r.traced, r.written, r.analyzed_files
        ));
    }
    if r.reused != first.first_party + first.third_party {
        v.push(format!(
            "{} of {} shards reused",
            r.reused,
            first.first_party + first.third_party
        ));
    }
    if r.tree_reused != [first.first_party, 0] {
        v.push(format!(
            "tree parts [reused, checked again] = {:?}, not [{}, 0]",
            r.tree_reused, first.first_party
        ));
    }
    if r.manifest_written {
        v.push("graph.json written".to_owned());
    }
    if r.obs_written {
        v.push("observation cache written".to_owned());
    }
    let after = snapshot(out);
    if after != before {
        let changed: Vec<String> = after
            .iter()
            .filter(|e| !before.contains(e))
            .chain(before.iter().filter(|e| !after.iter().any(|a| a.0 == e.0)))
            .take(6)
            .map(|e| e.0.strip_prefix(out).unwrap_or(&e.0).display().to_string())
            .collect();
        v.push(format!(
            "files below the graph directory changed: {changed:?}"
        ));
    }
    v
}

/// Build, refresh, refresh of the tree at `root` (already old enough to be
/// trusted), with the graph directory at `place`: what the two refreshes did
/// wrong.
fn check(name: &str, root: &Path, place: Place) -> Vec<String> {
    let out = place.dir(root);
    let first = rebuild(root, &out);
    let mut wrong = Vec::new();
    for nth in ["first", "second"] {
        let before = snapshot(&out);
        let r = rebuild(root, &out);
        for w in not_a_fixed_point(&r, &first, &before, &out) {
            wrong.push(format!("{name}, graph {place:?}, {nth} refresh: {w}"));
        }
    }
    wrong
}

/// A tree to check: its name and how to create it in a directory.
type Fixture = (String, Box<dyn Fn(&Path) + Send + Sync>);

fn fixtures() -> Vec<Fixture> {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../turborepo-trace-testkit/fixtures");
    let mut v: Vec<Fixture> = Vec::new();
    for group in ["tasks", "frameworks", "impact"] {
        let mut names: Vec<String> = std::fs::read_dir(base.join(group))
            .unwrap()
            .map(|e| e.unwrap())
            .filter(|e| e.file_type().unwrap().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with('_'))
            .collect();
        names.sort();
        for n in names {
            let src = base.join(group).join(&n);
            v.push((
                format!("{group}/{n}"),
                Box::new(move |to| copy_tree(&src, to)),
            ));
        }
    }
    let src = base.join("host-paths");
    v.push((
        "host-paths".to_owned(),
        Box::new(move |to| copy_tree(&src, to)),
    ));
    // The tree of `reuse.rs`: root-level deep listings enter more gitignored
    // build-output directories than a shard's tree part keeps.
    v.push((
        "monorepo".to_owned(),
        Box::new(|to| {
            Monorepo {
                packages: 24,
                files_per_package: 6,
                third_party: 12,
                dist_dirs: 12,
                ..Default::default()
            }
            .generate_in(to)
            .unwrap();
        }),
    ));
    v
}

#[test]
fn a_refresh_of_an_unchanged_tree_is_a_fixed_point_for_every_fixture() {
    let fixtures = fixtures();
    assert!(fixtures.len() > 60, "{} fixtures", fixtures.len());
    let places = [Place::Outside, Place::Default, Place::Named];
    let jobs: Vec<(&Fixture, Place)> = fixtures
        .iter()
        .flat_map(|f| places.iter().map(move |p| (f, *p)))
        .collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let wrong = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..8 {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(((name, create), place)) = jobs.get(i) else {
                        break;
                    };
                    let t = tempfile::tempdir().unwrap();
                    let root = std::fs::canonicalize(t.path()).unwrap().join("repo");
                    std::fs::create_dir(&root).unwrap();
                    create(&root);
                    // The graph directory exists before the tree is aged, so that the first
                    // build does not modify the root directory (see the test below for a
                    // build that does).
                    std::fs::create_dir_all(place.dir(&root)).unwrap();
                    backdate(t.path(), 60).unwrap();
                    let w = check(name, &root, *place);
                    wrong.lock().unwrap().extend(w);
                }
            });
        }
    });
    let mut wrong = wrong.into_inner().unwrap();
    wrong.sort();
    assert!(
        wrong.is_empty(),
        "{} refreshes of an unchanged tree are not a fixed point:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// A tree written just now, with a graph directory the first build creates
/// inside it: its directories are too young to be trusted (a change in the same
/// clock tick would not show in an mtime), so the first build keeps no reusable
/// state. Once the tree is two seconds old, one refresh records it, and the
/// refresh after that is a fixed point.
#[test]
fn a_young_tree_converges_one_refresh_after_the_racy_window() {
    let young = |place: Place| {
        let t = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(t.path()).unwrap().join("repo");
        std::fs::create_dir(&root).unwrap();
        copy_tree(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../turborepo-trace-testkit/fixtures/tasks/turbo-monorepo"),
            &root,
        );
        let out = place.dir(&root);
        let first = rebuild(&root, &out);
        std::thread::sleep(
            turborepo_trace_store::hash::RACY_WINDOW + std::time::Duration::from_millis(200),
        );
        // Nothing is traced (shards are keyed by content), whatever is checked again.
        let r = rebuild(&root, &out);
        assert_eq!((r.traced, r.written), (0, 0), "{place:?}");
        for nth in ["second", "third"] {
            let before = snapshot(&out);
            let r = rebuild(&root, &out);
            let wrong = not_a_fixed_point(&r, &first, &before, &out);
            assert!(wrong.is_empty(), "{place:?}, {nth} refresh: {wrong:?}");
        }
    };
    std::thread::scope(|s| {
        for place in [Place::Default, Place::Named, Place::Outside] {
            s.spawn(move || young(place));
        }
    });
}
