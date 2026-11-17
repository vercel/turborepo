//! The selection is a function of the tree, not of the place the checkout sits
//! at.
//!
//! Every fixture repository of `fixtures/` (and one written here, with the
//! shapes that leave a repository) is copied to several places: below the
//! temporary directory, below a directory called `tmp`, below a `node_modules`
//! directory and other directory names that mean something inside a repository
//! (`dist`, `vendor`, `.git`), at a deep and at a short path, and behind a
//! symbolic link (the two repositories written here also on a RAM disk below
//! `/dev`, where there is one). At every place the graph must be the same
//! (its export without the root, and its shards: the same bytes under the same
//! keys), and so must `floor` and `affected` for a null change, for
//! single files (every file of a small repository, [`SAMPLE`] evenly spaced
//! ones of a larger one) and for every file at once: of a graph built with one
//! thread and asked as it is, and again of a graph that several threads built
//! and that the query refreshes first (the defaults of both).
//!
//! A second kind of test: at an ordinary place, a repository whose code names
//! its own checkout (by its absolute path, by its directory's name) is answered
//! as the tool answered it before it stopped looking at that place.
//!
//! What may differ between two places is one thing: the bytes of a shard that
//! names its checkout, a task line in which a runner's own location is expanded
//! (`{{justfile_directory()}}`, `${{ github.workspace }}`). `graph pack` lists
//! those and `unpack` at another root traces them again. The root is no part of
//! a shard key, and every path of an export is relative to the root.
//!
//! A CI system checks out below `/home/runner/work`, `/vercel/path0`,
//! `/builds`, `/tmp` or `/private/var/folders`; a selection that depends on
//! which one it is cannot be shared, cached or reproduced on a laptop.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::Value;
use turborepo_trace_store::shard::Pack;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../turborepo-trace-testkit/fixtures")
}

/// Copies a tree, links as links.
fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let (src, dst) = (e.path(), to.join(e.file_name()));
        let t = e.file_type().unwrap();
        if t.is_symlink() {
            turborepo_trace_core::os::symlink(std::fs::read_link(&src).unwrap(), &dst).unwrap();
        } else if t.is_dir() {
            copy_dir(&src, &dst);
        } else {
            std::fs::copy(&src, &dst).unwrap();
        }
    }
}

/// What `graph pack` did in the standalone tooling: the graph of the checkout
/// at `dir` saved to `to`. `turbo trace` has no pack file: its graph travels as
/// its directory (copied with the checkout's files, or restored from turbo's
/// cache), so a copy of that directory stands for the pack.
fn pack_graph(dir: &Path, to: &Path) {
    let _ = std::fs::remove_dir_all(to);
    copy_dir(&dir.join(".turbo/trace"), to);
}

/// What `--restore` did: a checkout whose graph directory holds nothing
/// starts from the saved graph.
fn restore_graph(dir: &Path, from: Option<&Path>) {
    let graph = dir.join(".turbo/trace");
    if let Some(from) = from
        && !graph.exists()
    {
        copy_dir(from, &graph);
    }
}

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// One place a repository is copied to.
struct Place {
    /// What the place is, for messages.
    name: &'static str,
    /// The directory the repository is copied to.
    dir: PathBuf,
    /// The path the tool is given as the root (through a link for one place).
    root: PathBuf,
}

/// The places, and the directories to remove afterwards.
struct Lab {
    places: Vec<Place>,
    _temp: tempfile::TempDir,
    plain: PathBuf,
    /// Other directories made for places.
    extra: Vec<PathBuf>,
}

impl Drop for Lab {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.plain);
        for d in &self.extra {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}

impl Lab {
    /// `name` keeps the labs of the tests of this file apart.
    fn new(name: &str) -> Lab {
        Lab::with(name, false)
    }

    /// [`Lab::new`], with a place on a RAM disk when `ram_disk`.
    fn with(name: &str, ram_disk: bool) -> Lab {
        // Below the temporary directory (`/tmp`, `/var/folders/…`), as short as it
        // gets.
        let temp = tempfile::tempdir().unwrap();
        let t = temp.path().canonicalize().unwrap();
        // Not below it, unless this checkout is: cargo's directory for test files.
        let plain = support::outside_tmp().join(format!("location-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&plain);
        std::fs::create_dir_all(&plain).unwrap();
        let n = plain.canonicalize().unwrap();
        let place = |name: &'static str, dir: PathBuf| Place {
            name,
            root: dir.clone(),
            dir,
        };
        let mut extra = Vec::new();
        let mut places = vec![
            place("below the temporary directory", t.join("repo")),
            place("outside the temporary directory", n.join("plain/checkout")),
            place("below a directory called tmp", n.join("tmp/repo")),
            place(
                "below a node_modules directory",
                n.join("node_modules/pkg/repo"),
            ),
            place(
                "deep, below directories with names that mean something in a repository",
                n.join("deep/dist/build/vendor/src/__tests__/.git/site-packages/x/repo"),
            ),
        ];
        // On a RAM disk, where a CI system may keep its checkouts: below `/dev`, which
        // holds no files to trace otherwise. Only where there is one to write
        // to, and only for the small repositories of this file (such a disk is
        // small).
        let shm = Path::new("/dev/shm").join(format!("location-{}-{name}", std::process::id()));
        if ram_disk && std::fs::create_dir_all(&shm).is_ok() {
            places.push(place("on a RAM disk below /dev", shm.join("repo")));
            extra.push(shm);
        }
        // Behind a link: the root the tool is given is not the real path.
        let real = n.join("real/target/repo");
        std::fs::create_dir_all(real.parent().unwrap()).unwrap();
        turborepo_trace_core::os::symlink(n.join("real/target"), n.join("link")).unwrap();
        places.push(Place {
            name: "behind a symbolic link",
            dir: real,
            root: n.join("link/repo"),
        });
        Lab {
            places,
            _temp: temp,
            plain,
            extra,
        }
    }
}

fn harness(place: &Place, args: &[&str]) -> String {
    // The root is given, not found: for one place it is a path through a link.
    let o = Command::new(support::harness())
        .args(args)
        .arg("--root")
        .arg(&place.root)
        .current_dir(&place.root)
        .output()
        .expect("run the harness");
    assert!(
        o.status.success(),
        "{}: turbotrace {args:?} failed: {}",
        place.name,
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout).unwrap()
}

/// What the tool says about a repository at one place.
struct Seen {
    /// The graph's export without the root.
    export: Value,
    /// The shards, key and bytes, sorted (see the development repository's
    /// `host_paths.rs`: read through the store's index, since the order of
    /// the log is no part of a graph).
    shards: Vec<(String, Vec<u8>)>,
    /// Command lines and what they print.
    outputs: Vec<(String, String)>,
    /// The configs whose test patterns match through the directories above the
    /// checkout here (`lint`, rule `test-pattern-above-root`): which files
    /// are tests depends on the place, as it does for the runner.
    located: Vec<String>,
}

/// The configs that `lint` reports for `test-pattern-above-root` at `place`.
fn located(place: &Place) -> Vec<String> {
    // (Only this rule: an error of another one is the exit code.)
    let json = harness(
        place,
        &[
            "lint",
            "--tests",
            "all",
            "--format",
            "json",
            "--no-baseline",
            "--rule",
            "unconstrained-load=off",
            "--rule",
            "cwd-path-explained=off",
        ],
    );
    let v: Value = serde_json::from_str(&json).unwrap();
    let mut out: Vec<String> = v["findings"]
        .as_array()
        .map(|f| {
            f.iter()
                .filter(|f| f["rule"] == "test-pattern-above-root")
                .filter_map(|f| f["file"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out.dedup();
    out
}

fn shards(graph: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let store = graph.join("store");
    let (keys, _) = Pack::index_only(&store);
    if let Some(pack) = Pack::open(&store) {
        assert_eq!(pack.len(), keys.len(), "the store's two indexes");
        for key in keys {
            let shard = pack.get(&key).expect("a shard of the index");
            out.push((key, shard.file_bytes()));
        }
    }
    out.sort();
    out
}

/// The first-party files of an export, as paths relative to the root.
fn files_of(export: &Value) -> Vec<String> {
    export["files"]
        .as_array()
        .map(|f| {
            f.iter()
                .filter_map(|r| r["path"].as_str())
                .filter(|p| {
                    !p.starts_with('/')
                        && !p.starts_with("..")
                        && !p.contains('#')
                        && !p.split('/').any(|c| c == "node_modules")
                })
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Builds the graph of the repository at `place` and asks it the questions;
/// `changes` are the files whose change is asked about (those of the first
/// place, for every place).
fn see(place: &Place, changes: &mut Option<Vec<String>>) -> Seen {
    // One thread: which file is the first to fill a memo is no part of this test
    // (the development repository's `host_paths.rs` has that one; the targets of a
    // crate are recorded on whichever file asked first).
    harness(place, &["graph", "build", "--threads", "1"]);
    let json = harness(
        place,
        &["graph", "export", "--no-refresh", "--format", "json"],
    );
    let mut export: Value = serde_json::from_str(&json).unwrap();
    export.as_object_mut().unwrap().remove("root");
    let shards = shards(&place.dir.join(".turbo/trace"));
    let changes = changes.get_or_insert_with(|| {
        let mut all = files_of(&export);
        all.sort();
        all.retain(|f| place.dir.join(f).is_file());
        // Single files, evenly spaced: every file of a small repository.
        let step = all.len().div_ceil(SAMPLE).max(1);
        all.into_iter().step_by(step).collect()
    });
    let mut outputs = Vec::new();
    let mut ask = |args: &[&str]| {
        outputs.push((args.join(" "), harness(place, args)));
    };
    ask(&["floor", "--tests", "all", "--no-refresh"]);
    let affected = ["affected", "--tests", "all", "--no-refresh", "--changed"];
    // A change of nothing in the graph: what is selected on every change.
    let mut null = affected.to_vec();
    null.push("no/such/file.txt");
    ask(&null);
    for c in changes.iter() {
        let mut one = affected.to_vec();
        one.push(c.as_str());
        ask(&one);
    }
    let mut every = vec!["affected", "--tests", "all", "--no-refresh"];
    for f in changes.iter() {
        every.extend(["--changed", f.as_str()]);
    }
    if !changes.is_empty() {
        ask(&every);
    }
    // The same of a graph that several threads built (the default) and that the
    // query refreshes first (the default): neither is a part of the answer.
    std::fs::remove_dir_all(place.dir.join(".turbo/trace")).unwrap();
    harness(place, &["graph", "build"]);
    ask(&[
        "affected",
        "--tests",
        "all",
        "--changed",
        "no/such/file.txt",
    ]);
    every.retain(|a| *a != "--no-refresh");
    if !changes.is_empty() {
        ask(&every);
    }
    Seen {
        export,
        shards,
        outputs,
        located: located(place),
    }
}

/// How many single files a change is asked for, per repository.
const SAMPLE: usize = 12;

/// The rows of `table` that only one side has, for a message.
fn table_diff(a: &Value, b: &Value, table: &str) -> String {
    let rows = |v: &Value| -> Vec<String> {
        v[table]
            .as_array()
            .map(|r| r.iter().map(Value::to_string).collect())
            .unwrap_or_else(|| vec![v[table].to_string()])
    };
    let (ra, rb) = (rows(a), rows(b));
    let only = |x: &[String], y: &[String]| -> Vec<String> {
        let y: std::collections::HashSet<&String> = y.iter().collect();
        x.iter()
            .filter(|r| !y.contains(r))
            .take(4)
            .map(|r| r.chars().take(400).collect())
            .collect()
    };
    // The same rows in another order (or another number of times): the first row
    // that differs, at both places.
    let order = match ra.iter().zip(&rb).position(|(x, y)| x != y) {
        Some(i) if only(&ra, &rb).is_empty() && only(&rb, &ra).is_empty() => format!(
            "; the same rows, row {i} at the first place: {}; here: {}",
            ra[i].chars().take(400).collect::<String>(),
            rb[i].chars().take(400).collect::<String>()
        ),
        _ => String::new(),
    };
    format!(
        "{table}: {} and {} row(s); only at the first place: {:#?}; only here: {:#?}{order}",
        ra.len(),
        rb.len(),
        only(&ra, &rb),
        only(&rb, &ra)
    )
}

/// What differs between the bytes of two shards, as the texts in them (runs of
/// printable characters: paths, specifiers, record details): the ones only one
/// side has, and where the same texts first come in another order. For a
/// message: "the bytes differ" does not say which record does.
fn shard_diff(a: &[u8], b: &[u8]) -> String {
    let texts = |bytes: &[u8]| -> Vec<String> {
        bytes
            .split(|c| !(c.is_ascii_graphic() || *c == b' '))
            .filter(|run| run.len() >= 4)
            .map(|run| String::from_utf8_lossy(run).into_owned())
            .collect()
    };
    let (ta, tb) = (texts(a), texts(b));
    let only = |x: &[String], y: &[String]| -> Vec<String> {
        let y: std::collections::HashSet<&String> = y.iter().collect();
        x.iter()
            .filter(|t| !y.contains(t))
            .take(8)
            .map(|t| t.chars().take(300).collect())
            .collect()
    };
    let order = match ta.iter().zip(&tb).position(|(x, y)| x != y) {
        Some(i) => format!(
            "text {i} of {} and {}: {:?} at the first place, {:?} here",
            ta.len(),
            tb.len(),
            &ta[i.saturating_sub(1)..(i + 3).min(ta.len())],
            &tb[i.saturating_sub(1)..(i + 3).min(tb.len())]
        ),
        None => format!("{} and {} texts, the common ones equal", ta.len(), tb.len()),
    };
    let at = a.iter().zip(b).position(|(x, y)| x != y);
    format!(
        "{} and {} bytes, first differing byte {at:?}; texts only at the first place: {:#?}; only \
         here: {:#?}; first difference in order: {order}",
        a.len(),
        b.len(),
        only(&ta, &tb),
        only(&tb, &ta)
    )
}

/// Compares what the tool says at every place with the first; returns the
/// differences.
fn compare(fixture: &str, places: &[&Place], seen: &[&Seen]) -> Vec<String> {
    compare_kinds(fixture, places, seen)
        .into_iter()
        .map(|(_, text)| text)
        .collect()
}

/// [`compare`], each difference with what kind of thing differs: `table <name>`
/// (of the export), `shard keys`, `shard bytes`, `floor` or `affected` (what
/// the command prints).
fn compare_kinds(fixture: &str, places: &[&Place], seen: &[&Seen]) -> Vec<(String, String)> {
    let mut diffs = Vec::new();
    let first = seen[0];
    for (&place, &s) in places.iter().zip(seen).skip(1) {
        let at = format!("{fixture}, {} ({})", place.name, place.root.display());
        if s.export != first.export {
            for table in first.export.as_object().unwrap().keys() {
                if s.export[table] != first.export[table] {
                    diffs.push((
                        format!("table {table}"),
                        format!("{at}: {}", table_diff(&first.export, &s.export, table)),
                    ));
                }
            }
        }
        let keys = |x: &Seen| x.shards.iter().map(|s| s.0.clone()).collect::<Vec<_>>();
        if keys(s) != keys(first) {
            diffs.push((
                "shard keys".to_owned(),
                format!("{at}: the shard keys differ"),
            ));
        } else {
            // A shard that names its checkout holds the absolute root, by design: a task
            // line in which the runner's own location is expanded
            // (`{{justfile_directory()}}`, `${{ github.workspace }}`). `graph
            // pack` lists such shards and `unpack` at another root traces them
            // again (`crates/turborepo-trace-store/tests/artifact.rs`). They are compared
            // by key and through the export; every other shard byte for byte.
            let names = |bytes: &[u8], p: &Place| {
                let root = p.dir.as_os_str().as_encoded_bytes();
                bytes.windows(root.len()).any(|w| w == root)
            };
            for ((key, a), (_, b)) in first.shards.iter().zip(&s.shards) {
                if a != b && !(names(a, places[0]) && names(b, place)) {
                    diffs.push((
                        "shard bytes".to_owned(),
                        format!(
                            "{at}: the bytes of shard {key} differ: {}",
                            shard_diff(a, b)
                        ),
                    ));
                    break;
                }
            }
        }
        for ((cmd, a), (_, b)) in first.outputs.iter().zip(&s.outputs) {
            if a != b {
                diffs.push((
                    cmd.split(' ').next().unwrap_or_default().to_owned(),
                    format!("{at}: `harness {cmd}` prints\n{b}\nand at the first place\n{a}"),
                ));
            }
        }
    }
    diffs
}

/// What is known to differ, and open: below a directory called `node_modules`
/// the `paths` of a tsconfig are not applied to the repository's own files. The
/// resolver library (`oxc_resolver`, `CachedPath::inside_node_modules`) takes
/// every file below a directory of that name for a file of an installed
/// package, by its absolute path, and gives such a file no path aliases. So
/// there a file governed by a tsconfig with `paths`, `baseUrl` or `rootDirs`
/// gets an open record (what its imports resolve to is not known), and so does
/// each of its imports that does not resolve: a test behind it runs on every
/// change. The one place where this tool is not the same
/// tool, until the resolver sees paths from the root on. These are the fixtures
/// with such configs, each with the kinds of things that differ there
/// ([`compare_kinds`]) and nothing else: the edges and records of the imports
/// through an alias and the files' open records, the probes of their resolution
/// (so the shards that hold them), and what is selected through them.
/// The key of a shard with a file that such a config governs holds the place as
/// well (the resolver records the fact, `FileRec::aliased`, `fw::place_state`):
/// a graph moved across the boundary is traced again there. A config that gives
/// no aliases leaves the key alone (it used to be guessed from the walked
/// tsconfigs: one that extends another or references projects counted). The
/// test fails when a fixture stops differing or differs in another kind of
/// thing: correct the list then.
///
/// (Not seen by this test, and open as well: `TMPDIR=<dir>/node_modules/x cargo
/// test -p turborepo-trace` still fails in `affected_tsconfig.rs`.)
const KNOWN: &[(&str, &str, &[&str])] = &[
    (
        "impact/tsconfig-paths",
        "below a node_modules directory",
        KNOWN_KINDS_TSCONFIG_PATHS,
    ),
    (
        "js/resolve",
        "below a node_modules directory",
        KNOWN_KINDS_JS_RESOLVE,
    ),
    (
        "next-e2e",
        "below a node_modules directory",
        KNOWN_KINDS_NEXT_E2E,
    ),
    (
        "frameworks/next-app",
        "below a node_modules directory",
        KNOWN_KINDS_NEXT_APP,
    ),
];
const KNOWN_KINDS_TSCONFIG_PATHS: &[&str] = &[
    "affected",
    "floor",
    "shard keys",
    "table edges",
    "table probes",
    "table uncertainties",
];
const KNOWN_KINDS_JS_RESOLVE: &[&str] = &[
    "floor",
    "shard keys",
    "table edges",
    "table probes",
    "table uncertainties",
];
const KNOWN_KINDS_NEXT_E2E: &[&str] = KNOWN_KINDS_JS_RESOLVE;
const KNOWN_KINDS_NEXT_APP: &[&str] =
    &["floor", "shard keys", "table edges", "table uncertainties"];

/// Copies each repository to every place of a lab and compares what the tool
/// says there.
fn check(lab_name: &str, repos: &[(String, PathBuf)]) {
    let lab = Lab::new(lab_name);
    let mut diffs = Vec::new();
    // (repository, place) pairs that are not compared: the place decides the tests.
    let mut differing = 0;
    for (name, source) in repos {
        let mut changes = None;
        let mut seen = Vec::new();
        for place in &lab.places {
            copy_dir(source, &place.dir);
            seen.push(see(place, &mut changes));
            std::fs::remove_dir_all(&place.dir).unwrap();
        }
        assert!(!seen[0].shards.is_empty(), "{name}: no shard");
        assert!(
            seen[0].located.is_empty(),
            "{name}: test patterns match above the checkout at {}: {:?}",
            lab.places[0].root.display(),
            seen[0].located
        );
        for (i, place) in lab.places.iter().enumerate().skip(1) {
            // jest matches absolute paths: below a directory called `node_modules` it
            // finds no test, and below one called `__tests__` every file is one. There the
            // graph of a repository with jest projects is another one, as the runner's
            // answer is (`lint` says so, and `patterns_that_name_a_place` asserts the
            // answers); every repository without them is the same there too.
            if !seen[i].located.is_empty() {
                if !NAMED.contains(&place.name) {
                    diffs.push(format!(
                        "{name}, {}: test patterns match above the checkout: {:?}",
                        place.name, seen[i].located
                    ));
                }
                differing += 1;
                continue;
            }
            let found = compare_kinds(name, &[&lab.places[0], place], &[&seen[0], &seen[i]]);
            let known = KNOWN
                .iter()
                .find(|(n, p, _)| *n == name.as_str() && *p == place.name);
            match known {
                Some((_, _, kinds)) => {
                    let mut differs: Vec<&str> = found.iter().map(|(k, _)| k.as_str()).collect();
                    differs.sort();
                    differs.dedup();
                    if differs != *kinds {
                        diffs.push(format!(
                            "{name}, {}: differs in {differs:?}, known to differ in {kinds:?}: \
                             correct KNOWN\n{}",
                            place.name,
                            found
                                .iter()
                                .map(|(_, t)| t.as_str())
                                .collect::<Vec<_>>()
                                .join("\n")
                        ));
                    }
                }
                None => diffs.extend(found.into_iter().map(|(_, t)| t)),
            }
        }
    }
    assert!(
        diffs.is_empty(),
        "{} difference(s) between places:\n{}",
        diffs.len(),
        diffs.join("\n")
    );
    // Most pairs are compared: a repository differs at two of the places at most.
    assert!(
        differing <= 2 * repos.len(),
        "{differing} pairs of repository and place were not compared"
    );
}

/// The places where the place decides nothing for the tests of the repository
/// `fixture`, each with what was seen there. The first place is one of them,
/// and every other place is one of [`NAMED`].
fn ordinary<'a>(
    fixture: &str,
    places: &'a [Place],
    seen: &'a [Seen],
) -> (Vec<&'a Place>, Vec<&'a Seen>) {
    assert!(
        seen[0].located.is_empty(),
        "{fixture}: {:?}",
        seen[0].located
    );
    let mut out = (Vec::new(), Vec::new());
    for (place, s) in places.iter().zip(seen) {
        if s.located.is_empty() {
            out.0.push(place);
            out.1.push(s);
        } else {
            assert!(
                NAMED.contains(&place.name),
                "{fixture}, {}: test patterns match above the checkout: {:?}",
                place.name,
                s.located
            );
        }
    }
    out
}

/// The places whose directories jest's default patterns name: `/node_modules/`
/// leaves every file out, `**/__tests__/**` makes every file a test.
const NAMED: &[&str] = &[
    "below a node_modules directory",
    "deep, below directories with names that mean something in a repository",
];

/// The directories of `fixtures/<group>` (or their `sub` directory), sorted.
fn group(group: &str, sub: Option<&str>) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = std::fs::read_dir(fixtures().join(group))
        .unwrap()
        .map(|e| e.unwrap())
        .filter(|e| e.file_type().unwrap().is_dir())
        .map(|e| {
            let name = format!("{group}/{}", e.file_name().to_string_lossy());
            match sub {
                Some(s) => (name, e.path().join(s)),
                None => (name, e.path()),
            }
        })
        .filter(|(_, p)| p.is_dir())
        .collect();
    out.sort();
    assert!(!out.is_empty(), "no fixture in {group}");
    out
}

fn one(name: &str) -> Vec<(String, PathBuf)> {
    vec![(name.to_owned(), fixtures().join(name))]
}

#[test]
fn framework_fixtures_are_the_same_at_every_place() {
    check("frameworks", &group("frameworks", None));
}

#[test]
fn impact_fixtures_are_the_same_at_every_place() {
    check("impact", &group("impact", Some("repo")));
}

#[test]
fn task_fixtures_are_the_same_at_every_place() {
    check("tasks", &group("tasks", None));
}

#[test]
fn go_and_rust_fixtures_are_the_same_at_every_place() {
    let mut repos = group("go", None);
    repos.extend(group("rust", None));
    check("langs", &repos);
}

#[test]
fn js_fixtures_are_the_same_at_every_place() {
    // `js/nft` is one repository: Node File Trace's cases, with every shape of a
    // path.
    check("js", &group("js", None));
}

#[test]
fn next_and_host_path_fixtures_are_the_same_at_every_place() {
    let mut repos = one("next-e2e");
    repos.extend(one("next-build/lab"));
    repos.extend(one("host-paths"));
    check("next", &repos);
}

/// A repository whose files leave it in every way a path can: by relative steps
/// above the root (a module load, a pattern, a read, a listing, a tsconfig, a
/// jest config, a script, Python, Go), by an absolute path (a temporary
/// directory, a system directory, the place a container build would copy the
/// repository to), through the temporary directory of the run, by steps above
/// the root that come back in through the directory's own name (in code, and in
/// a script that a task runs). (Test patterns that name a directory above the
/// root: [`patterns_that_name_a_place_find_what_jest_finds_there`].)
fn leaving(root: &Path) {
    write(
        root,
        "package.json",
        r#"{"name":"loc","private":true,"workspaces":["packages/*"],
            "scripts":{"gen":"node ../outside/gen.js > /tmp/out.txt && cat /etc/hosts",
                       "reenter":"sh scripts/reenter.sh"}}"#,
    );
    // A script that a task runs: back in through a directory name, and the path of
    // a container.
    write(
        root,
        "scripts/reenter.sh",
        "#!/bin/sh\ncat ../repo/data/in.json\ncat /app/docs/notes.txt\n",
    );
    // A vitest project of its own (no test of the root's jest project): its runner
    // is started in the package or in the root. A path relative to the working
    // directory that comes back in through a directory name is read from the
    // root too; one that the package's directory explains (`packages/a` exists)
    // is not.
    write(
        root,
        "packages/e/package.json",
        r#"{"name":"@loc/e","version":"1.0.0","devDependencies":{"vitest":"1.0.0"}}"#,
    );
    write(
        root,
        "packages/e/vitest.config.js",
        "export default { test: { include: ['checks/**/*.check.js'] } };\n",
    );
    write(
        root,
        "packages/e/checks/cwd.check.js",
        "import fs from 'node:fs';\nimport { test } from 'vitest';\ntest('cwd', () => {\n  \
         fs.readFileSync('../repo/data/root.json');\n  \
         fs.readFileSync('../../packages/a/dist/gen.json');\n});\n",
    );
    write(root, "data/root.json", "{}\n");
    write(root, "dist/gen.json", "{}\n");
    write(root, "jest.config.js", "module.exports = {};\n");
    write(root, "data/in.json", "{}\n");
    write(root, "data/abs.json", "{}\n");
    write(root, "docs/notes.txt", "x\n");
    write(
        root,
        "packages/a/package.json",
        r#"{"name":"@loc/a","version":"1.0.0"}"#,
    );
    // Inside: the control.
    write(
        root,
        "packages/a/src/in.js",
        "module.exports = require('../../../data/in.json');\n",
    );
    write(
        root,
        "packages/a/src/in.test.js",
        "require('./in');\ntest('in', () => {});\n",
    );
    const HEAD: &str = "const fs = require('node:fs');\nconst os = require('node:os');\nconst \
                        path = require('node:path');\n";
    // Relative steps above the root.
    write(
        root,
        "packages/a/src/climb.js",
        &format!(
            "{HEAD}module.exports = [\n  () => require('../../../../outside/gen.js'),\n  (n) => \
             require('../../../../outside/' + n),\n  () => fs.readFileSync(path.join(__dirname, \
             '../../../../sibling/data.json')),\n  () => fs.readdirSync(path.join(__dirname, \
             '..', '..', '..', '..')),\n  () => fs.existsSync(path.join(__dirname, \
             '../../../../node_modules/x')),\n  () => import('../../../../../../far.mjs'),\n];\n"
        ),
    );
    write(
        root,
        "packages/a/src/climb.test.js",
        "require('./climb');\ntest('climb', () => {});\n",
    );
    // Above the root and back in through a directory name (`repo` is what some of
    // the places are called).
    write(
        root,
        "packages/a/src/reenter.js",
        &format!(
            "{HEAD}module.exports = [\n  () => require('../../../../repo/data/in.json'),\n  () => \
             require('../../../../repo/packages/a/src/in.js'),\n  () => \
             fs.readFileSync(path.join(__dirname, '../../../../repo/docs/notes.txt')),\n  () => \
             fs.readFileSync(path.join(__dirname, '../../../../checkout/docs/notes.txt')),\n];\n"
        ),
    );
    write(
        root,
        "packages/a/src/reenter.test.js",
        "require('./reenter');\ntest('reenter', () => {});\n",
    );
    // Absolute paths, and the temporary directory of the run.
    write(
        root,
        "packages/a/src/abs.js",
        &format!(
            "{HEAD}module.exports = [\n  () => require('/tmp/out/bundle.js'),\n  () => \
             require(path.join(os.tmpdir(), 'gen.js')),\n  (n) => import('/var/tmp/gen-' + n + \
             '.mjs'),\n  () => require('/usr/lib/addon.node'),\n  () => \
             fs.readFileSync('/etc/hosts'),\n  () => fs.readFileSync('/tmp/data.json'),\n  () => \
             fs.readdirSync('/tmp'),\n  () => require('/srv/app/.git/hooks/x.js'),\n  () => \
             fs.readFileSync('/app/docs/notes.txt'),\n  () => \
             require('/workspace/data/abs.json'),\n];\n"
        ),
    );
    write(
        root,
        "packages/a/src/abs.test.js",
        "require('./abs');\ntest('abs', () => {});\n",
    );
    // A tsconfig that extends and maps above the root.
    write(
        root,
        "packages/a/tsconfig.json",
        r#"{"extends":"../../../base/tsconfig.json",
            "compilerOptions":{"baseUrl":".","paths":{"@out/*":["../../../out/*"],"@in/*":["../../data/*"]}}}"#,
    );
    write(
        root,
        "packages/a/src/mapped.ts",
        "import out from '@out/thing';\nimport data from '@in/in.json';\nexport default [out, \
         data];\n",
    );
    write(
        root,
        "packages/a/src/mapped.test.ts",
        "import './mapped';\ntest('mapped', () => {});\n",
    );
    // A jest project whose config names files above the root.
    write(
        root,
        "packages/b/package.json",
        r#"{"name":"@loc/b","version":"1.0.0","scripts":{"test":"jest","pre":"sh ../../scripts/run.sh"}}"#,
    );
    write(
        root,
        "packages/b/jest.config.js",
        "module.exports = {\n  setupFiles: ['<rootDir>/../../../outside/setup.js'],\n  \
         moduleNameMapper: { '^@up/(.*)$': '<rootDir>/../../../outside/$1' },\n};\n",
    );
    write(
        root,
        "packages/b/src/b.test.js",
        "require('@up/thing');\ntest('b', () => {});\n",
    );
    // A script, Python and Go.
    write(
        root,
        "scripts/run.sh",
        "#!/bin/sh\n. ../../env.sh\n. \"$(dirname \"$0\")/../../outside.sh\"\ncat /tmp/x \
         ../docs/notes.txt\nnode ../../outside/tool.js\n",
    );
    write(
        root,
        "tools/read.py",
        "import os\nopen('../../outside.txt')\nopen('/tmp/x')\nopen(os.path.join(os.path.\
         dirname(__file__), '..', '..', 'outside', \
         'x.txt'))\nopen(os.path.join(os.path.dirname(__file__), '..', 'docs', 'notes.txt'))\n",
    );
    write(
        root,
        "tools/test_read.py",
        "import read\n\ndef test_read():\n    pass\n",
    );
    write(root, "svc/go.mod", "module example.com/svc\n\ngo 1.22\n");
    write(
        root,
        "svc/main.go",
        "package main\n\nimport (\n\t\"os\"\n\t\"path/filepath\"\n)\n\nfunc main() \
         {\n\tos.ReadFile(\"../../outside.txt\")\n\tos.ReadFile(\"/tmp/x\")\n\tos.\
         ReadFile(filepath.Join(\"..\", \"..\", \"repo\", \"data\", \
         \"in.json\"))\n\tos.ReadFile(filepath.Join(\"..\", \"docs\", \"notes.txt\"))\n}\n",
    );
    write(
        root,
        "svc/main_test.go",
        "package main\n\nimport \"testing\"\n\nfunc TestMain(t *testing.T) { main() }\n",
    );
}

#[test]
fn a_repository_whose_paths_leave_it_is_the_same_at_every_place() {
    let lab = Lab::with("leaving", true);
    let source = lab.plain.join("source");
    leaving(&source);
    let mut changes = Some(
        [
            "data/in.json",
            "docs/notes.txt",
            "packages/a/src/in.js",
            "packages/a/src/climb.js",
            "packages/a/src/abs.js",
            "packages/a/tsconfig.json",
            "packages/b/jest.config.js",
            "scripts/run.sh",
            "data/abs.json",
            "data/root.json",
            "dist/gen.json",
            "packages/a/dist/gen.json",
        ]
        .map(str::to_owned)
        .to_vec(),
    );
    let mut seen = Vec::new();
    for place in &lab.places {
        copy_dir(&source, &place.dir);
        seen.push(see(place, &mut changes));
    }
    // Its jest projects have jest's default patterns: compared where the place
    // decides nothing for them.
    let (places, seen) = ordinary("leaving", &lab.places, &seen);
    let diffs = compare("leaving", &places, &seen);
    assert!(
        diffs.is_empty(),
        "{} difference(s) between places:\n{}",
        diffs.len(),
        diffs.join("\n")
    );
    // And the answers are the right ones, not only the same ones: what stays inside
    // is a dependency, what leaves selects nothing.
    let first = seen[0];
    let of = |change: &str| -> Vec<String> {
        first
            .outputs
            .iter()
            .find(|(cmd, _)| cmd.ends_with(&format!("--no-refresh --changed {change}")))
            .unwrap_or_else(|| panic!("no answer for {change}"))
            .1
            .lines()
            .map(str::to_owned)
            .collect()
    };
    let floor = of("no/such/file.txt");
    let with = |change: &str| -> Vec<String> {
        of(change)
            .into_iter()
            .filter(|t| !floor.contains(t))
            .collect()
    };
    // A file that a climb above the root and back in names
    // (`../../../../repo/data/…`, through whatever the checkout's directory is
    // called) is a dependency at every place, as it is in a checkout called
    // `repo`. The Go file's `../../repo/data/in.json` is relative to the
    // working directory of its test (`svc`, one level down): from there it
    // climbs one level above the root and names the same file.
    assert_eq!(
        with("data/in.json"),
        [
            "packages/a/src/in.test.js",
            "packages/a/src/reenter.test.js",
            "svc/main_test.go"
        ]
    );
    // A path relative to the working directory of a test in a package, through a
    // directory name: the root is a working directory of the test too. Not
    // where the package's own directory explains the path.
    assert_eq!(with("data/root.json"), ["packages/e/checks/cwd.check.js"]);
    assert_eq!(with("dist/gen.json"), [""; 0]);
    assert_eq!(
        with("packages/a/dist/gen.json"),
        ["packages/e/checks/cwd.check.js"]
    );
    // The Go test reads the file from inside.
    assert_eq!(
        with("docs/notes.txt"),
        ["packages/a/src/reenter.test.js", "svc/main_test.go"]
    );
    // So is a file that an absolute path of another place names
    // (`/app/docs/notes.txt`: the repository mounted at `/app`). The test that
    // holds it is selected on every change (it loads a module from a temporary
    // directory): the edges say it.
    let files: Vec<&str> = first.export["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["path"].as_str())
        .collect();
    let node = |path: &str| files.iter().position(|f| *f == path).unwrap() as u64;
    for (target, said) in [
        (
            "docs/notes.txt",
            "elsewhere /app/docs/notes.txt, an absolute path: it names docs/notes.txt if the \
             repository is mounted at /app",
        ),
        (
            "data/abs.json",
            "elsewhere /workspace/data/abs.json, an absolute path: it names data/abs.json if the \
             repository is mounted at /workspace",
        ),
    ] {
        let (from, to) = (node("packages/a/src/abs.js"), node(target));
        assert!(
            first.export["edges"].as_array().unwrap().iter().any(|e| {
                e["from"].as_u64() == Some(from)
                    && e["to"].as_u64() == Some(to)
                    && e["specifier"].as_str() == Some(said)
            }),
            "no edge from packages/a/src/abs.js to {target}"
        );
    }
    assert!(floor.contains(&"packages/a/src/abs.test.js".to_owned()));
    // The task whose script reads through the checkout's name and by the path of a
    // container depends on both files, at every place (the outputs are compared
    // above).
    let task = |change: &str| {
        let args = ["affected", "--tasks", "task:.#reenter", "--changed", change];
        harness(&lab.places[0], &args)
    };
    assert_eq!(task("data/in.json").trim(), "task:.#reenter");
    assert_eq!(task("docs/notes.txt").trim(), "task:.#reenter");
    assert_eq!(task("data/abs.json").trim(), "");
    // A module below a temporary directory was generated at run time: the load is
    // open, wherever the checkout is.
    let texts: Vec<&str> = first.export["uncertainties"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|u| u["detail"].as_str())
        .collect();
    for spelled in ["/tmp/out/bundle.js", "/var/tmp/gen-"] {
        assert!(
            texts
                .iter()
                .any(|t| t.contains(spelled) && t.contains("loaded from a temporary directory")),
            "{spelled}: {texts:#?}"
        );
    }
    assert!(
        !texts
            .iter()
            .any(|t| t.contains("../") && t.contains("temporary directory")),
        "a path that leaves by relative steps is no temporary path: {texts:#?}"
    );
}

/// An absolute path that is inside the checkout at one of the places: the code
/// names the directory of one machine. What it names is the same dependency at
/// every place (the repository's files that its trailing components name), and
/// what is selected is the same everywhere. (The text of its edges and records
/// differs at that one place: a shard stores no text with the checkout's own
/// absolute path, so there the path is shown from the root on.)
#[test]
fn an_absolute_path_into_the_checkout_of_one_place_is_the_same_dependency_everywhere() {
    let lab = Lab::with("absolute", true);
    let inside = lab.places[1].dir.display().to_string();
    let source = lab.plain.join("source");
    write(
        &source,
        "package.json",
        r#"{"name":"abs","version":"1.0.0"}"#,
    );
    write(&source, "jest.config.js", "module.exports = {};\n");
    write(&source, "data/abs.json", "{}\n");
    write(&source, "data/other.json", "{}\n");
    write(&source, "docs/notes.txt", "x\n");
    write(&source, "docs/more.txt", "x\n");
    write(
        &source,
        "src/in.js",
        "module.exports = require('../data/abs.json');\n",
    );
    write(
        &source,
        "src/in.test.js",
        "require('./in');\ntest('in', () => {});\n",
    );
    write(
        &source,
        "src/abs.js",
        &format!(
            "const fs = require('node:fs');\nmodule.exports = [\n  () => \
             require('{inside}/data/abs.json'),\n  () => \
             fs.readFileSync('{inside}/docs/notes.txt'),\n];\n"
        ),
    );
    write(
        &source,
        "src/abs.test.js",
        "require('./abs');\ntest('abs', () => {});\n",
    );
    write(
        &source,
        "src/dir.js",
        &format!(
            "const fs = require('node:fs');\nmodule.exports = [\n  () => \
             fs.readdirSync('{inside}/docs'),\n  (n) => require('{inside}/data/' + n),\n];\n"
        ),
    );
    write(
        &source,
        "src/dir.test.js",
        "require('./dir');\ntest('dir', () => {});\n",
    );
    write(
        &source,
        "tools/read.py",
        &format!("open('{inside}/docs/notes.txt')\n"),
    );
    write(&source, "svc/go.mod", "module example.com/svc\n\ngo 1.22\n");
    write(
        &source,
        "svc/main_test.go",
        &format!(
            "package svc\n\nimport (\n\t\"os\"\n\t\"testing\"\n)\n\nfunc TestAbs(t *testing.T) \
             {{\n\tos.ReadFile(\"{inside}/docs/notes.txt\")\n}}\n"
        ),
    );
    let mut changes = Some(
        [
            "data/abs.json",
            "data/other.json",
            "docs/notes.txt",
            "docs/more.txt",
            "src/abs.js",
        ]
        .map(str::to_owned)
        .to_vec(),
    );
    let mut seen = Vec::new();
    for place in &lab.places {
        copy_dir(&source, &place.dir);
        seen.push(see(place, &mut changes));
    }
    // Rows without their text.
    let shape = |s: &Seen, table: &str| -> Vec<String> {
        s.export[table]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                let mut r = r.clone();
                let o = r.as_object_mut().unwrap();
                o.remove("specifier");
                o.remove("detail");
                r.to_string()
            })
            .collect()
    };
    let (places, seen) = ordinary("absolute", &lab.places, &seen);
    let first = seen[0];
    for (place, s) in places.iter().zip(&seen).skip(1) {
        assert_eq!(s.outputs, first.outputs, "{}", place.name);
        for table in first.export.as_object().unwrap().keys() {
            if !first.export[table].is_array() {
                assert_eq!(
                    s.export[table], first.export[table],
                    "{table}, {}",
                    place.name
                );
                continue;
            }
            assert_eq!(
                shape(s, table),
                shape(first, table),
                "{table}, {}",
                place.name
            );
        }
    }
    // The text is the same too, at every place but the one the path names.
    for (i, (place, s)) in places.iter().zip(&seen).enumerate().skip(2) {
        assert_eq!(s.export, first.export, "{} ({i})", place.name);
    }
    let of = |change: &str| -> Vec<String> {
        first
            .outputs
            .iter()
            .find(|(cmd, _)| cmd.ends_with(&format!("--no-refresh --changed {change}")))
            .unwrap_or_else(|| panic!("no answer for {change}"))
            .1
            .lines()
            .map(str::to_owned)
            .collect()
    };
    // The same dependency as the path from inside: the file, a file of the
    // directory, a file the pattern can name.
    assert_eq!(
        of("data/abs.json"),
        ["src/abs.test.js", "src/dir.test.js", "src/in.test.js"]
    );
    assert_eq!(of("data/other.json"), ["src/dir.test.js"]);
    assert_eq!(
        of("docs/notes.txt"),
        ["src/abs.test.js", "src/dir.test.js", "svc/main_test.go"]
    );
    assert_eq!(of("docs/more.txt"), ["src/dir.test.js"]);
    assert_eq!(of("no/such/file.txt"), [""; 0]);
    // A file appearing where another reading of the path looks selects too: the
    // path names `notes.txt` at the root if the repository is mounted at `<the
    // place>/docs`.
    let added = harness(
        &lab.places[0],
        &["affected", "--tests", "all", "--changed", "notes.txt"],
    );
    assert!(added.lines().any(|l| l == "src/abs.test.js"), "{added}");
    // `--explain` says what the path is.
    let why = harness(
        &lab.places[0],
        &[
            "affected",
            "--tests",
            "all",
            "--changed",
            "docs/notes.txt",
            "--explain",
            "src/abs.test.js",
        ],
    );
    let said = format!(
        "src/abs.js refers to {inside}/docs/notes.txt, an absolute path: it names docs/notes.txt \
         if the repository is mounted at {inside}"
    );
    assert!(why.contains(&said), "{why}");
}

/// The second kind of test: at an ordinary place, a repository whose code names
/// its own checkout is answered as before. The code is written at test time
/// with the absolute path of the directory it is in and with that directory's
/// name (a container build's `WORKDIR /app` with tests that read
/// `/app/fixtures/x.json`; a monorepo's `../<its name>/x`), in the shapes that
/// used to resolve at that place only: a read, a listing, a computed name and a
/// module load by the absolute path, in JS and in Go; a read and a module load
/// through the directory's name, from the file's own location and
/// from the working directory of its test; jest patterns that name the
/// directory; and a task whose script reads both ways.
///
/// What is asserted is what the tool selected for these changes when it looked
/// the paths up where they land (before the selection stopped depending on
/// the place).
#[test]
fn at_an_ordinary_place_code_that_names_its_checkout_selects_what_it_did() {
    let lab = Lab::new("ordinary");
    let place = &lab.places[1];
    let root = &place.dir;
    let at = root.display().to_string();
    // A checkout below a temporary directory is no ordinary place: a path below
    // `/tmp` that the code writes out is a temporary file.
    if turborepo_trace_core::fs::is_temp_path(root) {
        eprintln!("skipped: the test directory {at} is below a temporary directory");
        return;
    }
    let name = root.file_name().unwrap().to_str().unwrap();
    write(
        root,
        "package.json",
        r#"{"name":"own","version":"1.0.0","scripts":{"check":"sh scripts/run.sh"}}"#,
    );
    write(root, "jest.config.js", "module.exports = {};\n");
    for f in [
        "abs.json",
        "reenter.json",
        "cwd.json",
        "other.json",
        "dir/a.json",
        "dir/b.json",
    ] {
        write(root, &format!("data/{f}"), "{}\n");
    }
    for f in ["absonly", "reonly", "plain"] {
        write(root, &format!("src/lib/{f}.js"), "module.exports = 1;\n");
    }
    const FS: &str = "const fs = require('node:fs');\n";
    write(
        root,
        "src/abs.test.js",
        &format!("{FS}test('abs', () => {{ fs.readFileSync(\"{at}/data/abs.json\"); }});\n"),
    );
    write(
        root,
        "src/absreq.test.js",
        &format!("require('{at}/src/lib/absonly.js');\ntest('absreq', () => {{}});\n"),
    );
    write(
        root,
        "src/reenter.test.js",
        &format!(
            "{FS}const path = \
             require('node:path');\nrequire('../../{name}/src/lib/reonly.js');\ntest('reenter', \
             () => {{\n  fs.readFileSync(path.join(__dirname, \
             '../../{name}/data/reenter.json'));\n}});\n"
        ),
    );
    write(
        root,
        "src/dir.test.js",
        &format!(
            "{FS}test('dir', () => {{\n  fs.readdirSync('{at}/data/dir').map((n) => \
             fs.readFileSync('{at}/data/dir/' + n));\n}});\n"
        ),
    );
    write(
        root,
        "src/plain.test.js",
        "require('./lib/plain');\ntest('plain', () => {});\n",
    );
    // Relative to the working directory of the test (the root, where jest is
    // started).
    write(
        root,
        "src/cwd.test.js",
        &format!("{FS}test('cwd', () => {{ fs.readFileSync('../{name}/data/cwd.json'); }});\n"),
    );
    write(root, "svc/go.mod", "module example.com/svc\n\ngo 1.22\n");
    write(
        root,
        "svc/main_test.go",
        &format!(
            "package svc\n\nimport (\n\t\"os\"\n\t\"testing\"\n)\n\nfunc TestAbs(t *testing.T) \
             {{\n\tos.ReadFile(\"{at}/data/abs.json\")\n}}\n"
        ),
    );
    write(
        root,
        "scripts/run.sh",
        &format!("#!/bin/sh\ncat {at}/data/abs.json\ncat ../{name}/data/reenter.json\n"),
    );
    // jest patterns that name the checkout's directory: jest matches absolute
    // paths.
    for (dir, config) in [
        (
            "a",
            format!("module.exports = {{ testMatch: ['**/{name}/a/checks/**/*.js'] }};\n"),
        ),
        (
            "b",
            format!("module.exports = {{ testRegex: '{name}/b/checks/.*[.]js$' }};\n"),
        ),
    ] {
        write(
            root,
            &format!("{dir}/package.json"),
            &format!(r#"{{"name":"{dir}","version":"1.0.0"}}"#),
        );
        write(root, &format!("{dir}/jest.config.js"), &config);
        write(root, &format!("{dir}/lib.js"), "module.exports = 1;\n");
        write(
            root,
            &format!("{dir}/checks/one.js"),
            "require('../lib');\ntest('one', () => {});\n",
        );
    }
    // The same with a wildcard after the directory, and with a directory further
    // up: a `**` or `.*` that lies across the root, and a rest that is a part
    // of the file name.
    let above = root
        .parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    for (dir, config) in [
        (
            "c",
            format!("module.exports = {{ testMatch: ['**/{name}/**/checks/**/*.js'] }};\n"),
        ),
        (
            "d",
            format!("module.exports = {{ testRegex: '{name}/.*/checks/.*[.]js$' }};\n"),
        ),
        (
            "e",
            format!("module.exports = {{ testMatch: ['**/{above}/**/checks/**/*.js'] }};\n"),
        ),
        (
            "f",
            format!("module.exports = {{ testRegex: '{above}/.*[.]check[.]js$' }};\n"),
        ),
        // An expression with no `/` between a name above the root and its wildcard, and
        // with the separator written as a class and as `.` (a review case).
        (
            "g",
            format!("module.exports = {{ testRegex: '{name}.*[.]check[.]js$' }};\n"),
        ),
        (
            "h",
            format!("module.exports = {{ testRegex: '{above}.*[.]check[.]js$' }};\n"),
        ),
        (
            "i",
            format!("module.exports = {{ testRegex: '{name}[/]i[/]src[/].*[.]check[.]js$' }};\n"),
        ),
        (
            "j",
            format!("module.exports = {{ testRegex: '{above}.{name}.j.src.*[.]check[.]js$' }};\n"),
        ),
    ] {
        write(
            root,
            &format!("{dir}/package.json"),
            &format!(r#"{{"name":"{dir}","version":"1.0.0"}}"#),
        );
        write(root, &format!("{dir}/jest.config.js"), &config);
        write(root, &format!("{dir}/lib.js"), "module.exports = 1;\n");
        for t in ["checks/one.js", "src/a.check.js"] {
            write(
                root,
                &format!("{dir}/{t}"),
                "require('../lib');\ntest('t', () => {});\n",
            );
        }
    }
    harness(place, &["graph", "build"]);
    let tests = |change: &str| -> Vec<String> {
        harness(place, &["affected", "--tests", "all", "--changed", change])
            .lines()
            .map(str::to_owned)
            .collect()
    };
    // What an earlier build selected at such a place (`/vercel/cmp/repo`).
    assert_eq!(
        tests("data/abs.json"),
        ["src/abs.test.js", "svc/main_test.go"]
    );
    assert_eq!(tests("data/reenter.json"), ["src/reenter.test.js"]);
    assert_eq!(tests("data/cwd.json"), ["src/cwd.test.js"]);
    assert_eq!(tests("data/dir/a.json"), ["src/dir.test.js"]);
    assert_eq!(tests("data/dir/new.json"), ["src/dir.test.js"]);
    assert_eq!(tests("data/other.json"), [""; 0]);
    assert_eq!(tests("src/lib/absonly.js"), ["src/absreq.test.js"]);
    assert_eq!(tests("src/lib/reonly.js"), ["src/reenter.test.js"]);
    assert_eq!(tests("src/lib/plain.js"), ["src/plain.test.js"]);
    assert_eq!(tests("a/lib.js"), ["a/checks/one.js"]);
    assert_eq!(tests("b/lib.js"), ["b/checks/one.js"]);
    assert_eq!(tests("c/lib.js"), ["c/checks/one.js"]);
    assert_eq!(tests("d/lib.js"), ["d/checks/one.js"]);
    assert_eq!(tests("e/lib.js"), ["e/checks/one.js"]);
    assert_eq!(tests("f/lib.js"), ["f/src/a.check.js"]);
    for dir in ["g", "h", "i", "j"] {
        assert_eq!(
            tests(&format!("{dir}/lib.js")),
            [format!("{dir}/src/a.check.js")],
            "{dir}"
        );
    }
    // `--explain` says that the place decides it.
    let explain = harness(
        place,
        &[
            "affected",
            "--tests",
            "all",
            "--changed",
            "c/lib.js",
            "--explain",
            "c/checks/one.js",
        ],
    );
    assert!(
        explain.contains(&format!(
            "assumed: c/checks/one.js is a jest test here and none in a checkout elsewhere: the \
             test pattern `**/{name}/**/checks/**/*.js` matches through "
        )),
        "{explain}"
    );
    assert_eq!(tests("no/such/file.txt"), [""; 0]);
    let task = |change: &str| {
        let args = ["affected", "--tasks", "task:.#check", "--changed", change];
        harness(place, &args)
    };
    assert_eq!(task("data/abs.json").trim(), "task:.#check");
    assert_eq!(task("data/reenter.json").trim(), "task:.#check");
    assert_eq!(task("data/other.json").trim(), "");
}

/// The repository root is a working directory of a test whose runner can be
/// started there (jest with a path filter, pytest, Playwright), also when the
/// test's own package directory is known: a path relative to the working
/// directory that climbs above the root and comes back in through the
/// checkout's name is read from the root, as it was when the tool looked such a
/// path up where it lands. Not for `go test`, which runs a test in its
/// package's directory.
///
/// The reading from the root is not taken where the test's own directory
/// explains the path: `../../packages/ui/dist/gen.json`, written for a runner
/// started in `apps/web`, names `dist/gen.json` from the root only in a
/// checkout kept below `…/packages/ui/`. That is an assumption: `--strict` and
/// `TURBO_TRACE_DEBUG_CWD_EXPLAINED=0` take the reading, and `--explain` states
/// it.
#[test]
fn the_root_is_a_working_directory_of_a_test_in_a_package() {
    let lab = Lab::new("cwdroot");
    let place = &lab.places[1];
    let root = &place.dir;
    let name = root.file_name().unwrap().to_str().unwrap();
    const FS: &str = "const fs = require('node:fs');\n";
    write(
        root,
        "package.json",
        r#"{"name":"root","private":true,"version":"1.0.0","workspaces":["packages/*","apps/*"]}"#,
    );
    for f in [
        "rootname", "py", "go", "goroot", "cfg", "setup", "helper", "list/a", "other",
    ] {
        write(root, &format!("data/{f}.json"), "{}\n");
    }
    write(root, "dist/gen.json", "{}\n");
    write(root, "dist/dir/a.json", "{}\n");
    // jest in a package: started in the package or, with a path filter, in the
    // root.
    write(
        root,
        "packages/a/package.json",
        r#"{"name":"a","version":"1.0.0","scripts":{"test":"jest"}}"#,
    );
    // A module that the config loads reads the same way.
    write(
        root,
        "packages/a/jest.config.js",
        "require('./cfg-helper');\nmodule.exports = { setupFiles: ['<rootDir>/setup.js'] };\n",
    );
    write(
        root,
        "packages/a/cfg-helper.js",
        &format!("{FS}fs.readFileSync('../{name}/data/cfg.json');\n"),
    );
    write(
        root,
        "packages/a/setup.js",
        &format!("{FS}fs.readFileSync('../{name}/data/setup.json');\n"),
    );
    write(
        root,
        "packages/a/src/rootname.test.js",
        &format!(
            "{FS}test('rootname', () => {{ fs.readFileSync('../{name}/data/rootname.json'); }});\n"
        ),
    );
    // A helper of another package, a listing and a computed name below the path.
    write(
        root,
        "packages/a/src/helper.test.js",
        "require('../../h/read');\ntest('helper', () => {});\n",
    );
    write(
        root,
        "packages/a/src/plain.test.js",
        "test('plain', () => {});\n",
    );
    write(
        root,
        "packages/h/package.json",
        r#"{"name":"h","version":"1.0.0"}"#,
    );
    write(
        root,
        "packages/h/read.js",
        &format!(
            "{FS}module.exports = () => [\n  fs.readFileSync('../{name}/data/helper.json'),\n  \
             fs.readdirSync('../{name}/data/list'),\n];\n"
        ),
    );
    // The shape of a path written for the package's directory: `packages/ui`
    // exists.
    write(
        root,
        "packages/ui/package.json",
        r#"{"name":"ui","version":"1.0.0"}"#,
    );
    write(
        root,
        "apps/web/package.json",
        r#"{"name":"web","version":"1.0.0","scripts":{"test":"jest"}}"#,
    );
    write(root, "apps/web/jest.config.js", "module.exports = {};\n");
    write(
        root,
        "apps/web/src/gen.test.js",
        &format!(
            "{FS}test('gen', () => {{\n  fs.readFileSync('../../packages/ui/dist/gen.json');\n  \
             fs.readdirSync('../../packages/ui/dist/dir');\n}});\n"
        ),
    );
    // A script of another package, written for that package's directory:
    // `packages/ui` exists next to it, so the path is not read from the root
    // either (where `../ui/…` would name a file of a checkout called `ui`).
    write(
        root,
        "packages/s/package.json",
        r#"{"name":"s","version":"1.0.0"}"#,
    );
    write(
        root,
        "packages/s/scripts/check.js",
        &format!("{FS}module.exports = () => fs.readFileSync('../ui/data.json');\n"),
    );
    write(
        root,
        "apps/web/src/script.test.js",
        "require('../../../packages/s/scripts/check');\ntest('script', () => {});\n",
    );
    write(root, "data.json", "{}\n");
    // Playwright: started in the config's directory or in the root. The global
    // setup and a spec read the same way.
    write(
        root,
        "apps/e2e/package.json",
        r#"{"name":"e2e","version":"1.0.0","devDependencies":{"@playwright/test":"1.40.0"}}"#,
    );
    write(
        root,
        "apps/e2e/playwright.config.ts",
        "export default { testDir: './specs', globalSetup: './global-setup.ts' };\n",
    );
    write(
        root,
        "apps/e2e/global-setup.ts",
        &format!(
            "import fs from 'node:fs';\nexport default async function setup() {{\n  \
             fs.readFileSync('../{name}/data/pw.json');\n}}\n"
        ),
    );
    write(
        root,
        "apps/e2e/specs/a.spec.ts",
        &format!(
            "import fs from 'node:fs';\nimport {{ test }} from '@playwright/test';\ntest('a', \
             async () => {{ fs.readFileSync('../{name}/data/spec.json'); }});\n"
        ),
    );
    write(
        root,
        "apps/e2e/specs/b.spec.ts",
        "import { test } from '@playwright/test';\ntest('b', async () => {});\n",
    );
    write(root, "data/pw.json", "{}\n");
    write(root, "data/spec.json", "{}\n");
    // pytest: started in the project's directory or in the root.
    write(
        root,
        "py/pyproject.toml",
        "[project]\nname = \"py\"\nversion = \"0.1.0\"\n",
    );
    write(
        root,
        "py/tests/test_read.py",
        &format!("def test_read():\n    open(\"../{name}/data/py.json\").read()\n"),
    );
    // go test: in the package's directory, one level down, and nowhere else.
    write(root, "svc/go.mod", "module example.com/svc\n\ngo 1.22\n");
    write(
        root,
        "svc/main_test.go",
        &format!(
            "package svc\n\nimport (\n\t\"os\"\n\t\"testing\"\n)\n\nfunc TestRead(t *testing.T) \
             {{\n\tos.ReadFile(\"../../{name}/data/go.json\")\n\tos.ReadFile(\"../{name}/data/\
             goroot.json\")\n}}\n"
        ),
    );
    harness(place, &["graph", "build"]);
    let run = |extra: &[&str], change: &str| -> Vec<String> {
        let mut args = vec!["affected", "--tests", "all", "--changed", change];
        args.extend(extra);
        harness(place, &args).lines().map(str::to_owned).collect()
    };
    let all_of_a = [
        "packages/a/src/helper.test.js",
        "packages/a/src/plain.test.js",
        "packages/a/src/rootname.test.js",
    ];
    for strict in [&[][..], &["--strict"][..]] {
        let tests = |change: &str| run(strict, change);
        assert_eq!(
            tests("data/rootname.json"),
            ["packages/a/src/rootname.test.js"],
            "{strict:?}"
        );
        assert_eq!(
            tests("data/helper.json"),
            ["packages/a/src/helper.test.js"],
            "{strict:?}"
        );
        assert_eq!(
            tests("data/list/a.json"),
            ["packages/a/src/helper.test.js"],
            "{strict:?}"
        );
        assert_eq!(
            tests("data/list/new.json"),
            ["packages/a/src/helper.test.js"],
            "{strict:?}"
        );
        assert_eq!(tests("data/cfg.json"), all_of_a, "{strict:?}");
        assert_eq!(tests("data/setup.json"), all_of_a, "{strict:?}");
        assert_eq!(tests("data/go.json"), ["svc/main_test.go"], "{strict:?}");
        // `go test` is never started in the root.
        assert_eq!(tests("data/goroot.json"), [""; 0], "{strict:?}");
        assert_eq!(tests("data/other.json"), [""; 0], "{strict:?}");
        // What the path names from the package's directory, when it appears.
        assert_eq!(
            tests("packages/ui/dist/gen.json"),
            ["apps/web/src/gen.test.js"],
            "{strict:?}"
        );
        assert_eq!(
            tests("packages/ui/dist/dir/new.json"),
            ["apps/web/src/gen.test.js"],
            "{strict:?}"
        );
    }
    // The path that the package's directory explains is not read from the root,
    // unless the assumption is off.
    assert_eq!(run(&[], "dist/gen.json"), [""; 0]);
    assert_eq!(run(&[], "dist/dir/a.json"), [""; 0]);
    for change in ["dist/gen.json", "dist/dir/a.json"] {
        assert_eq!(run(&["--strict"], change), ["apps/web/src/gen.test.js"]);
    }
    // The same for the path that the package of the script explains.
    assert_eq!(run(&[], "data.json"), [""; 0]);
    assert_eq!(
        run(&["--strict"], "data.json"),
        ["apps/web/src/script.test.js"]
    );
    let off = Command::new(support::harness())
        .current_dir(root)
        .env("TURBO_TRACE_DEBUG_CWD_EXPLAINED", "0")
        .args(["affected", "--tests", "all", "--changed", "dist/gen.json"])
        .arg("--root")
        .arg(&place.root)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&off.stdout).trim(),
        "apps/web/src/gen.test.js"
    );
    let explain = harness(
        place,
        &[
            "affected",
            "--tests",
            "all",
            "--changed",
            "dist/gen.json",
            "--explain",
            "apps/web/src/gen.test.js",
        ],
    );
    assert!(
        explain.contains("unaffected apps/web/src/gen.test.js")
            && explain.contains(
                "assumed: apps/web/src/gen.test.js:3 reads ../../packages/ui/dist/gen.json \
                 relative to the working directory; from apps/web/ that path goes through \
                 packages/ui/, which exists, so it is not also read from the repository root, \
                 where it would name dist/gen.json if the repository is checked out in a \
                 directory called packages/ui"
            )
            && explain.contains("--strict does not assume it"),
        "{explain}"
    );
}

/// A computed name directly below an absolute directory (`'/app/' + name`,
/// `path.join('/app', name)`) and a listing of one (`readdirSync('/app')`) have
/// the reading "the repository is mounted there": a read of the repository root
/// with a computed name, which keeps the record such a read keeps inside the
/// repository. Not for a directory of the system. And a read below a temporary
/// directory that the code writes out names no file of the repository by
/// assumption: it keeps a record that is ignored by default and open under
/// `--strict` and with `TURBO_TRACE_DEBUG_TEMP_READS=0`; `--explain` states it.
#[test]
fn a_computed_name_at_the_mount_point_and_a_read_below_a_temporary_directory() {
    let lab = Lab::new("mount");
    let place = &lab.places[1];
    let root = &place.dir;
    write(root, "package.json", r#"{"name":"m","version":"1.0.0"}"#);
    write(root, "jest.config.js", "module.exports = {};\n");
    write(root, "data/x.json", "{}\n");
    write(root, "fixtures/x.json", "{}\n");
    const HEAD: &str = "const fs = require('node:fs');\nconst path = require('node:path');\nconst \
                        name = String(Math.random());\n";
    for (test, body) in [
        ("tail", "fs.readFileSync('/app/' + name);"),
        ("join", "fs.readFileSync(path.join('/app', name));"),
        ("list", "fs.readdirSync('/app');"),
        ("mid", "fs.readFileSync(`/app/${name}/x.json`);"),
        ("deep", "fs.readFileSync('/srv/app/data/' + name);"),
        ("tmp", "fs.readFileSync('/tmp/fixtures/x.json');"),
        (
            "sys",
            "fs.readFileSync('/usr/' + name); fs.readdirSync('/etc');",
        ),
        ("plain", ""),
    ] {
        write(
            root,
            &format!("src/{test}.test.js"),
            &format!("{HEAD}test('{test}', () => {{ {body} }});\n"),
        );
    }
    harness(place, &["graph", "build"]);
    let run = |extra: &[&str], env: Option<(&str, &str)>, change: &str| -> Vec<String> {
        let mut c = Command::new(support::harness());
        c.args(["affected", "--tests", "all", "--changed", change])
            .args(extra)
            .arg("--root")
            .arg(&place.root)
            .current_dir(&place.root);
        if let Some((k, v)) = env {
            c.env(k, v);
        }
        let o = c.output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8(o.stdout)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    };
    // The records, by file.
    let export: Value = serde_json::from_str(&harness(
        place,
        &["graph", "export", "--no-refresh", "--format", "json"],
    ))
    .unwrap();
    let details = |file: &str| -> Vec<String> {
        let ix = export["files"]
            .as_array()
            .unwrap()
            .iter()
            .position(|f| f["path"] == file)
            .unwrap() as u64;
        export["uncertainties"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|u| u["file"].as_u64() == Some(ix))
            .map(|u| {
                format!(
                    "{} {}",
                    u["kind"].as_str().unwrap(),
                    u["detail"].as_str().unwrap()
                )
            })
            .collect()
    };
    for test in ["tail", "join"] {
        assert_eq!(
            details(&format!("src/{test}.test.js")),
            ["pattern_too_broad /app/*: . is the runtime working directory; not expanded"],
            "{test}"
        );
    }
    assert_eq!(
        details("src/list.test.js"),
        ["pattern_too_broad directory . is the runtime working directory; not expanded"]
    );
    assert!(
        details("src/deep.test.js").contains(
            &"pattern_too_broad /srv/app/data/*: . is the runtime working directory; not expanded"
                .to_owned()
        ),
        "{:?}",
        details("src/deep.test.js")
    );
    assert!(
        !details("src/sys.test.js")
            .iter()
            .any(|d| d.contains("not expanded")),
        "{:?}",
        details("src/sys.test.js")
    );
    assert_eq!(
        details("src/tmp.test.js"),
        [format!(
            "generated_file /tmp/fixtures/x.json {}",
            turborepo_trace_core::output::TEMP_READ_NOTE
        )]
    );
    // A directory computed directly below the mount point: read as mounted at `/`
    // (`app/*/x.json`), and not as `*/x.json` from the root. An assumption with a
    // record.
    assert_eq!(
        details("src/mid.test.js"),
        [format!(
            "generated_file /app/*/x.json {}",
            turborepo_trace_core::output::MOUNT_READ_NOTE
        )]
    );
    // By default: the directory the deeper path names, and nothing for the others
    // (their records are bounded by names, or ignored).
    assert_eq!(run(&[], None, "data/x.json"), ["src/deep.test.js"]);
    assert_eq!(run(&[], None, "fixtures/x.json"), [""; 0]);
    assert_eq!(run(&[], None, "no/such/file.txt"), [""; 0]);
    // Under --strict every one of these records is open.
    let strict = run(&["--strict"], None, "no/such/file.txt");
    for test in ["tail", "join", "list", "mid", "deep", "tmp"] {
        assert!(
            strict.contains(&format!("src/{test}.test.js")),
            "{test}: {strict:?}"
        );
    }
    assert!(
        !strict.contains(&"src/plain.test.js".to_owned()),
        "{strict:?}"
    );
    // The switch opens the temporary read alone.
    assert_eq!(
        run(
            &[],
            Some(("TURBO_TRACE_DEBUG_TEMP_READS", "0")),
            "no/such/file.txt"
        ),
        ["src/tmp.test.js"]
    );
    // So does the switch of the computed directory below a mount point.
    assert_eq!(
        run(
            &[],
            Some(("TURBO_TRACE_DEBUG_MOUNT_READS", "0")),
            "no/such/file.txt"
        ),
        ["src/mid.test.js"]
    );
    let explain = harness(
        place,
        &[
            "affected",
            "--tests",
            "all",
            "--changed",
            "fixtures/x.json",
            "--explain",
            "src/mid.test.js",
        ],
    );
    assert!(
        explain.contains(
            "assumed: a directory that is computed directly below a place the repository may be \
             mounted at (/app/*/x.json) names no directory of the repository (--strict does not \
             assume it)"
        ),
        "{explain}"
    );
    assert_eq!(
        turborepo_trace_graph::output::MOUNT_READ_NOTE,
        turborepo_trace_core::output::MOUNT_READ_NOTE
    );
    let explain = harness(
        place,
        &[
            "affected",
            "--tests",
            "all",
            "--changed",
            "fixtures/x.json",
            "--explain",
            "src/tmp.test.js",
        ],
    );
    assert!(
        explain.contains(
            "assumed: a path below a temporary directory that the code writes out \
             (/tmp/fixtures/x.json) names no file of the repository: what is there is made at run \
             time (--strict does not assume it)"
        ),
        "{explain}"
    );
    assert_eq!(
        turborepo_trace_graph::output::TEMP_READ_NOTE,
        turborepo_trace_core::output::TEMP_READ_NOTE
    );
}

/// The selection document of `affected --tests jest` for a change of `changed`:
/// the entries (selected and not), the files that look like tests and are no
/// entry, and whether the package of each directory has to run.
fn selection(
    place: &Place,
    changed: &str,
    extra: &[&str],
) -> (Vec<String>, Vec<String>, Vec<(String, String)>) {
    let mut args = vec![
        "affected",
        "--tests",
        "jest",
        "--format",
        "selection",
        "--mode",
        "enforce",
        "--changed",
        changed,
    ];
    args.extend_from_slice(extra);
    let v: Value = serde_json::from_str(&harness(place, &args)).unwrap();
    let (mut entries, mut unknown, mut runs) = (Vec::new(), Vec::new(), Vec::new());
    for p in v["packages"].as_array().into_iter().flatten() {
        for s in p["selected"].as_array().into_iter().flatten() {
            let path = s["path"].as_str().or(s.as_str()).unwrap();
            entries.push(path.to_owned());
        }
        for key in ["unselected", "unknown"] {
            for s in p[key].as_array().into_iter().flatten() {
                let list = if key == "unknown" {
                    &mut unknown
                } else {
                    &mut entries
                };
                list.push(s.as_str().unwrap().to_owned());
            }
        }
        runs.push((
            p["dir"]
                .as_str()
                .or(p["path"].as_str())
                .unwrap_or_default()
                .to_owned(),
            p["run"].as_str().unwrap_or_default().to_owned(),
        ));
    }
    entries.sort();
    unknown.sort();
    (entries, unknown, runs)
}

/// A review case: `testMatch: ['**/src/**/*.test.js']`, and a
/// file `tools/x.test.js` that another runner starts. At an ordinary place it
/// is no jest test but looks like a test: the selection document lists it as
/// unknown and the package has to run when an input of that runner changes, as
/// before the tool stopped looking at the place of the checkout.
#[test]
fn a_test_like_file_that_no_pattern_matches_here_stays_unknown() {
    let lab = Lab::new("unknown");
    let place = &lab.places[1];
    let root = &place.dir;
    // Not below a directory called `src`: there the pattern matches the file, for
    // jest too.
    if root.iter().any(|c| c == "src") {
        eprintln!(
            "skipped: {} is below a directory called src",
            root.display()
        );
        return;
    }
    write(
        root,
        "package.json",
        r#"{"name":"mo","version":"1.0.0","scripts":{"test":"jest","test:tools":"sh tools/run-all.sh"},"devDependencies":{"jest":"29.0.0"}}"#,
    );
    write(
        root,
        "jest.config.js",
        "module.exports = { testMatch: ['**/src/**/*.test.js'] };\n",
    );
    write(
        root,
        "src/a.test.js",
        "require('./lib');\ntest('a', () => {});\n",
    );
    write(root, "src/lib.js", "module.exports = 1;\n");
    write(
        root,
        "tools/x.test.js",
        "const assert = require('node:assert');\nrequire('./helper');\nassert.ok(global.ready);\n",
    );
    write(root, "tools/helper.js", "module.exports = 1;\n");
    write(root, "tools/setup.js", "global.ready = true;\n");
    write(root, "tools/fixture.txt", "x\n");
    write(
        root,
        "tools/run-all.sh",
        "#!/bin/sh\nfor f in tools/*.test.js; do node -r ./tools/setup.js \"$f\" < \
         tools/fixture.txt; done\n",
    );
    harness(place, &["graph", "build"]);
    assert_eq!(located(place), [""; 0]);
    for changed in ["tools/setup.js", "tools/fixture.txt", "tools/run-all.sh"] {
        let (entries, unknown, runs) = selection(place, changed, &[]);
        assert_eq!(entries, ["src/a.test.js"], "{changed}");
        assert_eq!(unknown, ["tools/x.test.js"], "{changed}");
        assert!(
            runs.iter().all(|(_, run)| run == "selected"),
            "{changed}: {runs:?}"
        );
    }
}

/// A review case: a checkout whose directory is called like a
/// directory inside it (`ui`, and `packages/ui` exists). A helper in
/// `packages/util` reads `../ui/data/h.json` relative to the working directory.
/// A test of the root project runs in the root and nowhere else: the path is
/// read from there (`data/h.json`), whatever the helper's package explains. A
/// test of a package has that package's directory to run in: there the path is
/// taken to be written for the helper's package (`packages/ui` exists)
/// and is not read from the root, an assumption that `--explain` states, that
/// the lint rule `cwd-path-explained` lists, and that `--strict` does not make.
#[test]
fn a_test_that_only_runs_in_the_root_reads_a_climbing_path_from_the_root() {
    let lab = Lab::new("cwdonly");
    let dir = lab.plain.canonicalize().unwrap().join("x/ui");
    let place = Place {
        name: "in a directory called ui",
        root: dir.clone(),
        dir,
    };
    let root = &place.dir;
    write(
        root,
        "package.json",
        r#"{"name":"root","private":true,"version":"1.0.0","workspaces":["packages/*","apps/*"]}"#,
    );
    write(
        root,
        "jest.config.js",
        "module.exports = { testMatch: ['<rootDir>/roottests/**/*.test.js'] };\n",
    );
    write(root, "data/h.json", "{}\n");
    write(root, "data/other.json", "{}\n");
    write(
        root,
        "packages/ui/package.json",
        r#"{"name":"ui","version":"1.0.0"}"#,
    );
    write(root, "packages/ui/index.js", "module.exports = 1;\n");
    write(
        root,
        "packages/util/package.json",
        r#"{"name":"util","version":"1.0.0"}"#,
    );
    write(
        root,
        "packages/util/h.js",
        "const fs = require('node:fs');\nmodule.exports = () => \
         fs.readFileSync('../ui/data/h.json');\n",
    );
    write(
        root,
        "roottests/r.test.js",
        "require('../packages/util/h');\ntest('r', () => {});\n",
    );
    write(
        root,
        "apps/web/package.json",
        r#"{"name":"web","version":"1.0.0","scripts":{"test":"jest"}}"#,
    );
    write(root, "apps/web/jest.config.js", "module.exports = {};\n");
    write(
        root,
        "apps/web/src/h.test.js",
        "require('../../../packages/util/h');\ntest('h', () => {});\n",
    );
    harness(&place, &["graph", "build"]);
    let tests = |extra: &[&str], change: &str| -> Vec<String> {
        let mut args = vec!["affected", "--tests", "all", "--changed", change];
        args.extend_from_slice(extra);
        harness(&place, &args).lines().map(str::to_owned).collect()
    };
    // The root project's test runs in the root only: the reading is taken.
    assert_eq!(tests(&[], "data/h.json"), ["roottests/r.test.js"]);
    assert_eq!(tests(&[], "data/other.json"), [""; 0]);
    // Without the assumption, the package's test reads it from the root too.
    assert_eq!(
        tests(&["--strict"], "data/h.json"),
        ["apps/web/src/h.test.js", "roottests/r.test.js"]
    );
    // The assumption is stated where it decides, and the lint rule lists the
    // reference.
    let explain = harness(
        &place,
        &[
            "affected",
            "--tests",
            "all",
            "--changed",
            "data/h.json",
            "--explain",
            "apps/web/src/h.test.js",
        ],
    );
    assert!(
        explain.contains(
            "assumed: packages/util/h.js:2 reads ../ui/data/h.json relative to the working \
             directory; from packages/util/ that path goes through packages/ui/, which exists"
        ),
        "{explain}"
    );
    let json = harness(
        &place,
        &[
            "lint",
            "--tests",
            "all",
            "--format",
            "json",
            "--no-baseline",
            "--rule",
            "unconstrained-load=off",
            "--rule",
            "cwd-path-explained=warn",
        ],
    );
    let v: Value = serde_json::from_str(&json).unwrap();
    let found: Vec<(&str, &str, &str)> = v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["rule"] == "cwd-path-explained")
        .map(|f| {
            (
                f["file"].as_str().unwrap(),
                f["key"].as_str().unwrap(),
                f["message"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(found.len(), 1, "{v}");
    assert_eq!(
        (found[0].0, found[0].1),
        ("packages/util/h.js", "../ui/data/h.json")
    );
    assert!(
        found[0].2.starts_with(
            "`../ui/data/h.json` is read relative to the working directory; from the repository \
             root it names `data/h.json` only if the repository is checked out in a directory \
             called ui."
        ),
        "{}",
        found[0].2
    );
}

/// A config that computes its patterns and directories from its own location
/// (`__dirname`) says the same at every place: the graph of such a repository
/// is the same wherever it is checked out (its shards under the same keys),
/// like one whose config writes `<rootDir>`.
#[test]
fn a_config_that_computes_from_its_location_is_the_same_at_every_place() {
    let source =
        support::outside_tmp().join(format!("location-src-computed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&source);
    write(
        &source,
        "package.json",
        r#"{"name":"computed","private":true,"version":"1.0.0","workspaces":["packages/*"]}"#,
    );
    write(
        &source,
        "jest.config.js",
        "const path = require('node:path');\nmodule.exports = {\n  rootDir: __dirname,\n  roots: \
         [path.join(__dirname, 'src')],\n  testMatch: [path.join(__dirname, \
         'src/**/*.test.js')],\n  testPathIgnorePatterns: [`${__dirname}/src/skip/`],\n};\n",
    );
    write(&source, "src/lib.js", "module.exports = 1;\n");
    write(
        &source,
        "src/a.test.js",
        "require('./lib');\ntest('a', () => {});\n",
    );
    write(
        &source,
        "src/skip/b.test.js",
        "require('../lib');\ntest('b', () => {});\n",
    );
    write(
        &source,
        "packages/p/package.json",
        r#"{"name":"p","version":"1.0.0"}"#,
    );
    write(
        &source,
        "packages/p/jest.config.js",
        "const path = require('node:path');\nmodule.exports = {\n  rootDir: \
         path.resolve(__dirname, '..', '..'),\n  roots: [__dirname],\n  testRegex: \
         `${__dirname}/checks/.*[.]js$`,\n};\n",
    );
    write(&source, "packages/p/lib.js", "module.exports = 1;\n");
    write(
        &source,
        "packages/p/checks/one.js",
        "require('../lib');\ntest('one', () => {});\n",
    );
    check("computed", &[("computed".to_owned(), source.clone())]);
    let _ = std::fs::remove_dir_all(&source);
}

/// The graph store writes the reading of a path that leaves the repository into
/// the specifier of its edges, and `--explain` finds it there: one text in both
/// crates.
#[test]
fn the_store_and_the_query_agree_on_how_a_reading_is_marked() {
    assert_eq!(
        turborepo_trace_graph::output::ELSEWHERE_PREFIX,
        turborepo_trace_core::output::ELSEWHERE_PREFIX
    );
}

/// Probe (a macOS CI failure): on macOS the temporary
/// directory is deep (`/private/var/folders/zz/xxxx/T/.tmpAb12`), on Linux it
/// is `/tmp`. The fixture's `import('../../../../../../far.mjs')` from
/// `packages/a/src` climbs six levels: below a Linux temp directory it lands at
/// `/far.mjs`, below a macOS one it lands below `/private/var/folders/…`. This
/// puts the repository four directories below the Linux temporary directory,
/// which is how deep the macOS one is, and compares it with an ordinary place.
#[test]
fn a_checkout_deep_below_the_temporary_directory_is_the_same() {
    let lab = Lab::with("deep-2", false);
    let source = lab.plain.join("source");
    leaving(&source);
    let deep = lab
        ._temp
        .path()
        .canonicalize()
        .unwrap()
        .join("v/f/zz/T/repo");
    let places = [
        Place {
            name: "outside the temporary directory",
            dir: lab.plain.join("plain/checkout"),
            root: lab.plain.join("plain/checkout"),
        },
        Place {
            name: "four directories below the temporary directory",
            dir: deep.clone(),
            root: deep,
        },
    ];
    let mut changes = None;
    let mut seen = Vec::new();
    for place in &places {
        copy_dir(&source, &place.dir);
        seen.push(see(place, &mut changes));
    }
    let diffs = compare(
        "deep-2",
        &places.iter().collect::<Vec<_>>(),
        &seen.iter().collect::<Vec<_>>(),
    );
    assert!(
        diffs.is_empty(),
        "{} difference(s) between places:\n{}",
        diffs.len(),
        diffs.join("\n")
    );
}

/// Probe: a vitest test that imports through a tsconfig `paths`
/// alias, in a checkout below a directory called `node_modules`. An earlier
/// version had the test "unknown" there (it must run); this one finds the
/// vitest test, but the alias is not applied below `node_modules` (README,
/// "Rust"). The unresolved import used to be "covered by the paths its
/// resolution probed", which do not include the aliased file, and the test was
/// unselected on a change of the file it imports. Now the file has an open
/// record there and the test runs; `--explain` says why and no longer claims
/// the probes bound the import. At an ordinary place it is selected through the
/// alias.
#[test]
fn a_test_behind_a_path_alias_below_node_modules_is_not_cleared() {
    let lab = Lab::new("alias");
    let make = |root: &Path| {
        write(
            root,
            "package.json",
            r#"{"name":"p","version":"1.0.0","type":"module","devDependencies":{"vitest":"1.0.0"}}"#,
        );
        write(
            root,
            "tsconfig.json",
            r#"{"compilerOptions":{"baseUrl":".","paths":{"@lib/*":["src/lib/*"]}}}"#,
        );
        write(
            root,
            "vitest.config.ts",
            "export default { test: { include: ['test/**/*.test.ts'] } };\n",
        );
        write(root, "src/lib/x.ts", "export const x = 1;\n");
        write(
            root,
            "test/a.test.ts",
            "import { x } from '@lib/x';\nimport { test } from 'vitest';\ntest('a', () => { x; \
             });\n",
        );
    };
    let mut wrong = Vec::new();
    for (name, dir) in [
        ("an ordinary place", lab.plain.join("work/repo")),
        (
            "below a node_modules directory",
            lab.plain.join("node_modules/pkg/repo"),
        ),
    ] {
        make(&dir);
        let place = Place {
            name: "probe",
            root: dir.clone(),
            dir,
        };
        let doc: Value = serde_json::from_str(&harness(
            &place,
            &[
                "affected",
                "--tests",
                "all",
                "--changed",
                "src/lib/x.ts",
                "--format",
                "selection",
            ],
        ))
        .unwrap();
        let pkg = &doc["packages"][0];
        let runs = pkg["selected"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|s| s["path"] == "test/a.test.ts")
            || pkg["unknown"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|s| s == "test/a.test.ts");
        if !runs {
            wrong.push(format!("{name}: {pkg}"));
        }
        let why = harness(
            &place,
            &[
                "affected",
                "--tests",
                "all",
                "--changed",
                "src/lib/x.ts",
                "--explain",
                "test/a.test.ts",
            ],
        );
        let probed = why
            .lines()
            .any(|l| l.contains("@lib/x") && l.contains("by the paths its resolution probed"));
        let said = why.contains("uncertain (conditional_resolution");
        if probed || said != name.starts_with("below") {
            wrong.push(format!("{name}: --explain says\n{why}"));
        }
        // The records say why.
        let records = harness(&place, &["floor", "--tests", "all", "--no-refresh"]);
        let reason = "are not applied to it";
        if records.contains(reason) != name.starts_with("below") {
            wrong.push(format!("{name}: `floor` says\n{records}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "test/a.test.ts does not run, or --explain is wrong:\n{}",
        wrong.join("\n")
    );
}

/// Probe (the macOS CI failure of
/// `a_repository_whose_paths_leave_it_is_the_same_at_every_place`): a task's
/// inputs are sorted by their absolute paths (`TaskSpec::normalize`), and its
/// edges follow that order. An absolute path that a command writes out
/// (`/u/docs/notes.txt`) sorts before a file of the checkout at one place and
/// after it at another: the same edges in another order, and a shard whose
/// bytes differ. On macOS the fixture's `/app/…` sorts after a checkout below
/// `/Users` and before one below `/private/var/folders`; on Linux `/tmp` and
/// `/home` both sort after `/app`. Here `/u/…` sits between the two places of a
/// Linux sandbox (`/tmp` and the target directory, `/vercel/…` there), so the
/// difference shows on Linux.
#[test]
fn a_task_that_reads_an_absolute_path_has_the_same_shard_at_every_place() {
    let lab = Lab::new("order");
    let source = lab.plain.join("source");
    write(
        &source,
        "package.json",
        r#"{"name":"o","private":true,"scripts":{"reenter":"sh scripts/reenter.sh"}}"#,
    );
    write(
        &source,
        "scripts/reenter.sh",
        "#!/bin/sh\ncat ../repo/data/in.json\ncat /u/docs/notes.txt\n",
    );
    write(&source, "data/in.json", "{}\n");
    write(&source, "docs/notes.txt", "x\n");
    let t = lab._temp.path().canonicalize().unwrap();
    let places = [
        Place {
            name: "below the temporary directory",
            dir: t.join("repo"),
            root: t.join("repo"),
        },
        Place {
            name: "in the target directory",
            dir: lab.plain.join("plain/repo"),
            root: lab.plain.join("plain/repo"),
        },
    ];
    let mut changes = None;
    let mut seen = Vec::new();
    for place in &places {
        copy_dir(&source, &place.dir);
        seen.push(see(place, &mut changes));
    }
    let diffs = compare(
        "order",
        &places.iter().collect::<Vec<_>>(),
        &seen.iter().collect::<Vec<_>>(),
    );
    assert!(
        diffs.is_empty(),
        "{} difference(s) between places:\n{}",
        diffs.len(),
        diffs.join("\n")
    );
}

// ---------------------------------------------------------------------------------------
// Review probes of the fixes for the two probes above.

/// Whether `test` runs (is selected or unknown) in a `--format selection`
/// answer.
fn selection_runs(doc: &Value, test: &str) -> bool {
    doc["packages"].as_array().into_iter().flatten().any(|pkg| {
        pkg["selected"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|s| s["path"] == test)
            || pkg["unknown"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|s| s == test)
    })
}

/// F1 harness: `make` writes a vitest repository; at an ordinary place and at
/// the places below (or named) `node_modules`, a change of `changed` must run
/// `test`. Returns what went wrong, with the uncertainty report of the place.
fn alias_case(name: &str, make: &dyn Fn(&Path), changed: &str, test: &str) -> Vec<String> {
    let lab = Lab::new(&format!("moved-{name}"));
    let mut wrong = Vec::new();
    for (pname, dir) in [
        ("an ordinary place", lab.plain.join("work/repo")),
        (
            "below a node_modules directory",
            lab.plain.join("node_modules/pkg/repo"),
        ),
        (
            "directly below a node_modules directory",
            lab.plain.join("nm1/node_modules/repo"),
        ),
        (
            "a root that is a node_modules directory",
            lab.plain.join("nm2/node_modules"),
        ),
    ] {
        make(&dir);
        let place = Place {
            name: "probe",
            root: dir.clone(),
            dir,
        };
        let doc: Value = serde_json::from_str(&harness(
            &place,
            &[
                "affected",
                "--tests",
                "all",
                "--changed",
                changed,
                "--format",
                "selection",
            ],
        ))
        .unwrap();
        if !selection_runs(&doc, test) {
            let records = harness(&place, &["floor", "--tests", "all", "--no-refresh"]);
            wrong.push(format!(
                "{name}, {pname}: {test} does not run on a change of {changed}: \
                 {doc}\nuncertainty:\n{records}"
            ));
        }
    }
    wrong
}

fn vitest_alias_repo(root: &Path, tsconfig: &str) {
    write(
        root,
        "package.json",
        r#"{"name":"p","version":"1.0.0","type":"module","devDependencies":{"vitest":"1.0.0"}}"#,
    );
    write(root, "tsconfig.json", tsconfig);
    write(
        root,
        "vitest.config.ts",
        "export default { test: { include: ['test/**/*.test.ts'] } };\n",
    );
}

/// Probe: the cases of an alias that is not applied below
/// `node_modules`.
#[test]
fn aliases_below_node_modules() {
    let mut wrong = Vec::new();
    // An installed package answers the import below node_modules (the alias wins in
    // the runner): the import resolves, so only the file's own record can say
    // it.
    wrong.extend(alias_case(
        "installed",
        &|r| {
            vitest_alias_repo(
                r,
                r#"{"compilerOptions":{"baseUrl":".","paths":{"lib/*":["src/lib/*"]}}}"#,
            );
            write(r, "src/lib/x.ts", "export const x = 1;\n");
            write(
                r,
                "node_modules/lib/package.json",
                r#"{"name":"lib","version":"1.0.0"}"#,
            );
            write(r, "node_modules/lib/x.js", "export const x = 2;\n");
            write(
                r,
                "test/a.test.ts",
                "import { x } from 'lib/x';\nimport { test } from 'vitest';\ntest('a', () => { x; \
                 });\n",
            );
        },
        "src/lib/x.ts",
        "test/a.test.ts",
    ));
    // baseUrl only.
    wrong.extend(alias_case(
        "baseurl",
        &|r| {
            vitest_alias_repo(r, r#"{"compilerOptions":{"baseUrl":"src"}}"#);
            write(r, "src/lib/x.ts", "export const x = 1;\n");
            write(
                r,
                "test/a.test.ts",
                "import { x } from 'lib/x';\nimport { test } from 'vitest';\ntest('a', () => { x; \
                 });\n",
            );
        },
        "src/lib/x.ts",
        "test/a.test.ts",
    ));
    // paths only in the base of an extends chain.
    wrong.extend(alias_case(
        "extends",
        &|r| {
            vitest_alias_repo(
                r,
                r#"{"extends":"./tsconfig.base.json","include":["src","test"]}"#,
            );
            write(
                r,
                "tsconfig.base.json",
                r#"{"compilerOptions":{"paths":{"@lib/*":["./src/lib/*"]}}}"#,
            );
            write(r, "src/lib/x.ts", "export const x = 1;\n");
            write(
                r,
                "test/a.test.ts",
                "import { x } from '@lib/x';\nimport { test } from 'vitest';\ntest('a', () => { \
                 x; });\n",
            );
        },
        "src/lib/x.ts",
        "test/a.test.ts",
    ));
    // A solution config whose referenced project has the paths.
    wrong.extend(alias_case(
        "references",
        &|r| {
            vitest_alias_repo(
                r,
                r#"{"files":[],"references":[{"path":"./tsconfig.app.json"}]}"#,
            );
            write(
                r,
                "tsconfig.app.json",
                r#"{"compilerOptions":{"composite":true,"paths":{"@lib/*":["./src/lib/*"]}},"include":["src","test"]}"#,
            );
            write(r, "src/lib/x.ts", "export const x = 1;\n");
            write(
                r,
                "test/a.test.ts",
                "import { x } from '@lib/x';\nimport { test } from 'vitest';\ntest('a', () => { x; });\n",
            );
        },
        "src/lib/x.ts",
        "test/a.test.ts",
    ));
    // The alias is in a source file the test imports relatively.
    wrong.extend(alias_case(
        "indirect",
        &|r| {
            vitest_alias_repo(
                r,
                r#"{"compilerOptions":{"paths":{"@lib/*":["./src/lib/*"]}}}"#,
            );
            write(r, "src/lib/x.ts", "export const x = 1;\n");
            write(r, "src/util.ts", "export { x } from '@lib/x';\n");
            write(
                r,
                "test/a.test.ts",
                "import { x } from '../src/util';\nimport { test } from 'vitest';\ntest('a', () \
                 => { x; });\n",
            );
        },
        "src/lib/x.ts",
        "test/a.test.ts",
    ));
    // rootDirs: a relative import that only resolves through the other root
    // directory.
    wrong.extend(alias_case(
        "rootdirs",
        &|r| {
            vitest_alias_repo(r, r#"{"compilerOptions":{"rootDirs":["src","generated"]}}"#);
            write(r, "generated/gen.ts", "export const g = 1;\n");
            write(r, "src/use.ts", "export { g } from './gen';\n");
            write(
                r,
                "test/a.test.ts",
                "import { g } from '../src/use';\nimport { test } from 'vitest';\ntest('a', () => \
                 { g; });\n",
            );
        },
        "generated/gen.ts",
        "test/a.test.ts",
    ));
    assert!(wrong.is_empty(), "{}", wrong.join("\n\n"));
}

/// Probe: at an ordinary place, nothing of the alias rule fires,
/// also for a repository with its own `node_modules` and a tsconfig with paths.
#[test]
fn aliases_at_an_ordinary_place_have_no_open_record() {
    let lab = Lab::new("ordinary-2");
    let dir = lab.plain.join("work/repo");
    vitest_alias_repo(
        &dir,
        r#"{"compilerOptions":{"baseUrl":".","paths":{"@lib/*":["src/lib/*"]}}}"#,
    );
    write(&dir, "src/lib/x.ts", "export const x = 1;\n");
    write(&dir, "src/lib/y.ts", "export const y = 1;\n");
    write(
        &dir,
        "node_modules/dep/package.json",
        r#"{"name":"dep","version":"1.0.0"}"#,
    );
    write(&dir, "node_modules/dep/index.ts", "export const d = 1;\n");
    write(
        &dir,
        "test/a.test.ts",
        "import { x } from '@lib/x';\nimport { test } from 'vitest';\ntest('a', () => { x; });\n",
    );
    let place = Place {
        name: "probe",
        root: dir.clone(),
        dir,
    };
    let records = harness(&place, &["floor", "--tests", "all"]);
    assert!(
        !records.contains("are not applied to it"),
        "an ordinary place has the alias record:\n{records}"
    );
    let doc: Value = serde_json::from_str(&harness(
        &place,
        &[
            "affected",
            "--tests",
            "all",
            "--changed",
            "src/lib/y.ts",
            "--format",
            "selection",
        ],
    ))
    .unwrap();
    assert!(
        !selection_runs(&doc, "test/a.test.ts"),
        "a change of an unrelated file selects the test at an ordinary place: {doc}"
    );
}

/// Probe: an absolute path that a command writes out and that lies
/// inside the checkout at one place (and outside it at the others) is ordered
/// by its path from the root there and by its absolute path elsewhere: the
/// edges of the task in another order.
#[test]
#[ignore = "by design (kept ignored): an absolute path a command writes into this checkout counts \
            as an ordinary input of it (crates/tasks registry.rs:629, recognize_command), so other \
            places have other probes for it; not an ordering bug"]
fn a_written_path_into_one_checkout_orders_the_same() {
    let lab = Lab::new("written");
    let inside = lab.places[1].dir.display().to_string();
    let source = lab.plain.join("source");
    write(
        &source,
        "package.json",
        r#"{"name":"o","private":true,"scripts":{"reenter":"sh scripts/reenter.sh"}}"#,
    );
    write(
        &source,
        "scripts/reenter.sh",
        &format!("#!/bin/sh\ncat zz/in.json\ncat {inside}/docs/notes.txt\n"),
    );
    write(&source, "zz/in.json", "{}\n");
    write(&source, "docs/notes.txt", "x\n");
    let places = [&lab.places[0], &lab.places[1]];
    let mut changes = None;
    let mut seen = Vec::new();
    for place in places {
        copy_dir(&source, &place.dir);
        seen.push(see(place, &mut changes));
    }
    let diffs = compare("written", &places, &seen.iter().collect::<Vec<_>>());
    assert!(
        diffs.is_empty(),
        "{} difference(s) between places:\n{}",
        diffs.len(),
        diffs.join("\n")
    );
}

/// Probe: a task's entries (`node /u/bin/a.js`, `node zz/b.js`) are
/// still in the order of their absolute paths, which puts `/u/…` between a
/// checkout below `/tmp` and one below `/vercel` (or `/Users`).
#[test]
fn task_entries_order_the_same_at_every_place() {
    let lab = Lab::new("entries");
    let source = lab.plain.join("source");
    write(
        &source,
        "package.json",
        r#"{"name":"o","private":true,"scripts":{"go":"node /u/bin/a.js && node zz/b.js && cat zz/in.json && cat /u/docs/notes.txt && cat ../repo/aa/in.json"}}"#,
    );
    write(&source, "bin/a.js", "console.log(1);\n");
    write(&source, "zz/b.js", "console.log(2);\n");
    write(&source, "zz/in.json", "{}\n");
    write(&source, "aa/in.json", "{}\n");
    write(&source, "docs/notes.txt", "x\n");
    let t = lab._temp.path().canonicalize().unwrap();
    let places = [
        Place {
            name: "below the temporary directory",
            dir: t.join("repo"),
            root: t.join("repo"),
        },
        Place {
            name: "in the target directory",
            dir: lab.plain.join("plain/repo"),
            root: lab.plain.join("plain/repo"),
        },
    ];
    let mut changes = None;
    let mut seen = Vec::new();
    for place in &places {
        copy_dir(&source, &place.dir);
        seen.push(see(place, &mut changes));
    }
    let diffs = compare(
        "entries",
        &places.iter().collect::<Vec<_>>(),
        &seen.iter().collect::<Vec<_>>(),
    );
    assert!(
        diffs.is_empty(),
        "{} difference(s) between places:\n{}",
        diffs.len(),
        diffs.join("\n")
    );
}

/// Probe: a link inside the root that points outside it, as a task
/// input, next to a written absolute path and a climbing one.
#[test]
fn a_link_out_of_the_root_as_a_task_input_orders_the_same() {
    let lab = Lab::new("link-2");
    let source = lab.plain.join("source");
    let outside = lab.plain.join("outside");
    write(&outside, "o.txt", "o\n");
    write(
        &source,
        "package.json",
        r#"{"name":"o","private":true,"scripts":{"go":"cat out/o.txt && cat /u/docs/notes.txt && cat ../repo/mm/in.json"}}"#,
    );
    write(&source, "mm/in.json", "{}\n");
    write(&source, "docs/notes.txt", "x\n");
    turborepo_trace_core::os::symlink(&outside, source.join("out")).unwrap();
    let t = lab._temp.path().canonicalize().unwrap();
    let places = [
        Place {
            name: "below the temporary directory",
            dir: t.join("repo"),
            root: t.join("repo"),
        },
        Place {
            name: "in the target directory",
            dir: lab.plain.join("plain/repo"),
            root: lab.plain.join("plain/repo"),
        },
    ];
    let mut changes = None;
    let mut seen = Vec::new();
    for place in &places {
        copy_dir(&source, &place.dir);
        seen.push(see(place, &mut changes));
    }
    let diffs = compare(
        "link-2",
        &places.iter().collect::<Vec<_>>(),
        &seen.iter().collect::<Vec<_>>(),
    );
    assert!(
        diffs.is_empty(),
        "{} difference(s) between places:\n{}",
        diffs.len(),
        diffs.join("\n")
    );
}

/// Probe: whether the checkout lies below `node_modules` decides the
/// open records of the alias rule, but it is no part of the shard key: a graph
/// built at an ordinary place and moved below `node_modules` keeps the records
/// of the ordinary place on refresh (and one moved the other way keeps the open
/// records). The refreshed answer differs from a cold build at the same place:
/// a cache changes the answer.
#[test]
fn a_graph_moved_below_node_modules_answers_as_a_cold_build_there() {
    let lab = Lab::new("move-2");
    let make = |r: &Path| {
        vitest_alias_repo(
            r,
            r#"{"compilerOptions":{"baseUrl":".","paths":{"@lib/*":["src/lib/*"]}}}"#,
        );
        write(r, "src/lib/x.ts", "export const x = 1;\n");
        write(r, "src/lib/y.ts", "export const y = 1;\n");
        write(
            r,
            "test/a.test.ts",
            "import { x } from '@lib/x';\nimport { test } from 'vitest';\ntest('a', () => { x; \
             });\n",
        );
    };
    let ask = |dir: &Path| -> String {
        let place = Place {
            name: "probe",
            root: dir.to_path_buf(),
            dir: dir.to_path_buf(),
        };
        let doc: Value = serde_json::from_str(&harness(
            &place,
            &[
                "affected",
                "--tests",
                "all",
                "--changed",
                "src/lib/y.ts",
                "--format",
                "selection",
            ],
        ))
        .unwrap();
        doc["packages"].to_string()
    };
    let mut wrong = Vec::new();
    for (from, to) in [
        ("a/repo", "node_modules/pkg/repo"),
        ("node_modules/pkg2/repo", "b/repo"),
    ] {
        let (a, z) = (lab.plain.join(from), lab.plain.join(to));
        make(&a);
        ask(&a);
        std::fs::create_dir_all(z.parent().unwrap()).unwrap();
        std::fs::rename(&a, &z).unwrap();
        let refreshed = ask(&z);
        std::fs::remove_dir_all(z.join(".turbo/trace")).unwrap();
        let cold = ask(&z);
        if refreshed != cold {
            wrong.push(format!(
                "built at {from}, moved to {to}: refreshed {refreshed}, cold {cold}"
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Probe: two inputs outside the root, one climbing out of it
/// (`../sib/a.txt`, whose absolute path depends on the place) and one written
/// absolutely (`/u/…`), are ordered by their absolute paths, which puts them in
/// another order below `/tmp` than below `/vercel` (or `/Users`).
#[test]
fn two_kinds_of_outside_inputs_order_the_same() {
    let lab = Lab::new("outside-2");
    let source = lab.plain.join("source");
    write(
        &source,
        "package.json",
        r#"{"name":"o","private":true,"scripts":{"go":"cat ../sib/a.txt && cat /u/docs/notes.txt && cat zz/in.json"}}"#,
    );
    write(&source, "zz/in.json", "{}\n");
    write(&source, "docs/notes.txt", "x\n");
    let t = lab._temp.path().canonicalize().unwrap();
    let places = [
        Place {
            name: "below the temporary directory",
            dir: t.join("repo"),
            root: t.join("repo"),
        },
        Place {
            name: "in the target directory",
            dir: lab.plain.join("plain/repo"),
            root: lab.plain.join("plain/repo"),
        },
    ];
    let mut changes = None;
    let mut seen = Vec::new();
    for place in &places {
        write(place.dir.parent().unwrap(), "sib/a.txt", "a\n");
        copy_dir(&source, &place.dir);
        seen.push(see(place, &mut changes));
    }
    let diffs = compare(
        "outside-2",
        &places.iter().collect::<Vec<_>>(),
        &seen.iter().collect::<Vec<_>>(),
    );
    assert!(
        diffs.is_empty(),
        "{} difference(s) between places:\n{}",
        diffs.len(),
        diffs.join("\n")
    );
}

// ---------------------------------------------------------------------------------------
// Review probes of the fixes for moved graphs (the cache) and task entries.

/// What a graph answers at `dir`: the selection on a change of `changed`, and
/// the open records. With `restore`, a graph directory that holds nothing
/// starts from that file.
fn ask_at(dir: &Path, changed: &str, restore: Option<&Path>) -> String {
    let place = Place {
        name: "probe",
        root: dir.to_path_buf(),
        dir: dir.to_path_buf(),
    };
    restore_graph(dir, restore);
    let args = vec![
        "affected",
        "--tests",
        "all",
        "--changed",
        changed,
        "--format",
        "selection",
    ];
    let doc: Value = serde_json::from_str(&harness(&place, &args)).unwrap();
    let records = harness(&place, &["floor", "--tests", "all", "--no-refresh"]);
    format!("{}\n{records}", doc["packages"])
}

/// Builds at `from`, moves the checkout to `to` and compares the refreshed
/// answer with a cold one there. `make` writes the repository; `around` writes
/// what lies outside it (called for both places). With `pack`, the graph
/// travels as a `graph pack` file to a fresh copy instead of with the
/// directory.
fn move_and_compare(
    lab: &Lab,
    name: &str,
    make: &dyn Fn(&Path),
    around: &dyn Fn(&Path),
    (from, to): (&str, &str),
    changed: &str,
    pack: bool,
) -> Option<String> {
    let (a, z) = (
        lab.plain.join(name).join(from),
        lab.plain.join(name).join(to),
    );
    make(&a);
    around(&a);
    ask_at(&a, changed, None);
    std::fs::create_dir_all(z.parent().unwrap()).unwrap();
    let refreshed = if pack {
        let file = lab
            .plain
            .join(name)
            .join(format!("{}.pack", from.replace('/', "_")));
        pack_graph(&a, &file);
        make(&z);
        around(&z);
        ask_at(&z, changed, Some(&file))
    } else {
        std::fs::rename(&a, &z).unwrap();
        around(&z);
        ask_at(&z, changed, None)
    };
    std::fs::remove_dir_all(z.join(".turbo/trace")).unwrap();
    let cold = ask_at(&z, changed, None);
    (refreshed != cold).then(|| {
        format!(
            "{name}: built at {from}, {} to {to}:\nrefreshed {refreshed}\ncold {cold}",
            if pack { "packed and restored" } else { "moved" }
        )
    })
}

/// The vitest repository of the move probe above, with the config
/// `make_config` writes.
fn moved_vitest_repo(r: &Path, ext: &str) {
    write(
        r,
        "package.json",
        r#"{"name":"p","version":"1.0.0","type":"module","devDependencies":{"vitest":"1.0.0"}}"#,
    );
    write(
        r,
        &format!("vitest.config.{ext}"),
        &format!("export default {{ test: {{ include: ['test/**/*.test.{ext}'] }} }};\n"),
    );
    write(r, &format!("src/lib/x.{ext}"), "export const x = 1;\n");
    write(r, &format!("src/lib/y.{ext}"), "export const y = 1;\n");
    write(
        r,
        &format!("test/a.test.{ext}"),
        "import { x } from '@lib/x';\nimport { test } from 'vitest';\ntest('a', () => { x; });\n",
    );
}

const ALIAS_PATHS: &str = r#"{"compilerOptions":{"paths":{"@lib/*":["./src/lib/*"]}}}"#;

/// Both directions across the boundary, and between two places below it.
const MOVE_WAYS: &[(&str, &str)] = &[
    ("a/repo", "node_modules/pkg/repo"),
    ("node_modules/pkg2/repo", "b/repo"),
    ("node_modules/pkg3/repo", "nm/node_modules/repo"),
];

/// A case: its name, what writes the repository, what writes around it, the
/// changed file.
type MoveCase<'a> = (&'a str, &'a dyn Fn(&Path), &'a dyn Fn(&Path), &'a str);

fn move_cases(lab_name: &str, cases: &[MoveCase], pack: bool) {
    let lab = Lab::new(lab_name);
    let mut wrong = Vec::new();
    for (name, make, around, changed) in cases {
        for (i, way) in MOVE_WAYS.iter().enumerate() {
            wrong.extend(move_and_compare(
                &lab,
                &format!("{name}{i}"),
                make,
                around,
                *way,
                changed,
                pack,
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n\n"));
}

fn nothing_around(_: &Path) {}

/// The author's cases and the brief's: sound ones are expected to pass.
#[test]
fn a_moved_graph_answers_as_a_cold_build_configs_in_the_walk() {
    let plain = |r: &Path| {
        moved_vitest_repo(r, "ts");
        write(r, "tsconfig.json", ALIAS_PATHS);
    };
    let jsconfig = |r: &Path| {
        moved_vitest_repo(r, "js");
        write(r, "jsconfig.json", ALIAS_PATHS);
    };
    let extends_pkg = |r: &Path| {
        moved_vitest_repo(r, "ts");
        write(
            r,
            "tsconfig.json",
            r#"{"extends":"@cfg/base/tsconfig.json"}"#,
        );
        write(
            r,
            "node_modules/@cfg/base/package.json",
            r#"{"name":"@cfg/base","version":"1.0.0"}"#,
        );
        write(
            r,
            "node_modules/@cfg/base/tsconfig.json",
            r#"{"compilerOptions":{"paths":{"@lib/*":["../../../src/lib/*"]}}}"#,
        );
    };
    let references = |r: &Path| {
        write(
            r,
            "package.json",
            r#"{"name":"root","private":true,"workspaces":["packages/*"]}"#,
        );
        write(
            r,
            "tsconfig.json",
            r#"{"files":[],"references":[{"path":"./configs/a"}]}"#,
        );
        write(
            r,
            "configs/a/tsconfig.json",
            r#"{"compilerOptions":{"composite":true,"paths":{"@lib/*":["../../packages/a/src/lib/*"]}},"include":["../../packages/a/**/*"]}"#,
        );
        moved_vitest_repo(&r.join("packages/a"), "ts");
        write(
            r,
            "packages/b/package.json",
            r#"{"name":"b","version":"1.0.0"}"#,
        );
        write(r, "packages/b/index.ts", "export const b = 1;\n");
    };
    let sfc = |r: &Path| {
        moved_vitest_repo(r, "ts");
        write(r, "tsconfig.json", ALIAS_PATHS);
        write(
            r,
            "src/App.vue",
            "<script setup lang=\"ts\">\nimport { x } from \
             '@lib/x';\nx;\n</script>\n<template><p/></template>\n",
        );
        write(
            r,
            "src/App.svelte",
            "<script lang=\"ts\">\nimport { x } from '@lib/x';\nx;\n</script>\n<p/>\n",
        );
        write(
            r,
            "test/b.test.ts",
            "import App from '../src/App.vue';\nimport S from '../src/App.svelte';\nimport { test \
             } from 'vitest';\ntest('b', () => { App; S; });\n",
        );
    };
    move_cases(
        "walk",
        &[
            ("plain", &plain, &nothing_around, "src/lib/y.ts"),
            ("jsconfig", &jsconfig, &nothing_around, "src/lib/y.js"),
            ("extendspkg", &extends_pkg, &nothing_around, "src/lib/y.ts"),
            (
                "references",
                &references,
                &nothing_around,
                "packages/a/src/lib/y.ts",
            ),
            ("sfc", &sfc, &nothing_around, "src/lib/y.ts"),
        ],
        false,
    );
}

/// A `tsconfig.json` that `.gitignore` names is not in the walk's file list,
/// but the resolver reads it.
#[test]
fn a_moved_graph_with_an_ignored_tsconfig() {
    let make = |r: &Path| {
        moved_vitest_repo(r, "ts");
        write(r, "tsconfig.json", ALIAS_PATHS);
        write(r, ".gitignore", "tsconfig.json\n");
    };
    move_cases(
        "ignored-2",
        &[("ignored", &make, &nothing_around, "src/lib/y.ts")],
        false,
    );
}

/// A `tsconfig.json` that is a link to a file of another name.
#[test]
fn a_moved_graph_with_a_linked_tsconfig() {
    let make = |r: &Path| {
        moved_vitest_repo(r, "ts");
        write(
            r,
            "cfg/base.json",
            r#"{"compilerOptions":{"paths":{"@lib/*":["../src/lib/*"]}}}"#,
        );
        let _ = std::fs::remove_file(r.join("tsconfig.json"));
        turborepo_trace_core::os::symlink("cfg/base.json", r.join("tsconfig.json")).unwrap();
    };
    move_cases(
        "linked-2",
        &[("linked", &make, &nothing_around, "src/lib/y.ts")],
        false,
    );
}

/// JSON may write a key with an escape: `"paths"` is `paths`.
#[test]
fn a_moved_graph_with_an_escaped_option_name() {
    let make = |r: &Path| {
        moved_vitest_repo(r, "ts");
        write(
            r,
            "tsconfig.json",
            r#"{"compilerOptions":{"paths":{"@lib/*":["./src/lib/*"]}}}"#,
        );
    };
    move_cases(
        "escaped-2",
        &[("escaped", &make, &nothing_around, "src/lib/y.ts")],
        false,
    );
}

/// The config that governs the files lies above the root (discovery walks up to
/// the file system's root), at the same relative place before and after the
/// move.
#[test]
fn a_moved_graph_with_a_tsconfig_above_the_root() {
    let make = |r: &Path| moved_vitest_repo(r, "ts");
    let around = |r: &Path| {
        write(
            r.parent().unwrap(),
            "tsconfig.json",
            r#"{"compilerOptions":{"paths":{"@lib/*":["./repo/src/lib/*"]}}}"#,
        );
    };
    let lab = Lab::new("above");
    let mut wrong = Vec::new();
    for (i, way) in [
        ("a/repo", "node_modules/repo"),
        ("node_modules/repo", "b/repo"),
    ]
    .into_iter()
    .enumerate()
    {
        wrong.extend(move_and_compare(
            &lab,
            &format!("above{i}"),
            &make,
            &around,
            way,
            "src/lib/y.ts",
            false,
        ));
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n\n"));
}

/// The option appears with a later edit: below `node_modules` first without it,
/// then with it; and built at an ordinary place without it, moved, then edited.
#[test]
fn an_option_that_appears_with_an_edit() {
    let lab = Lab::new("edit");
    let mut wrong = Vec::new();
    let changed = "src/lib/y.ts";
    for (i, (from, to)) in [
        ("node_modules/pkg/repo", None),
        ("a/repo", Some("node_modules/pkg/repo")),
        ("node_modules/pkg/repo", Some("b/repo")),
    ]
    .into_iter()
    .enumerate()
    {
        let a = lab.plain.join(format!("edit{i}")).join(from);
        moved_vitest_repo(&a, "ts");
        write(&a, "tsconfig.json", "{}\n");
        ask_at(&a, changed, None);
        let z = match to {
            Some(to) => {
                let z = lab.plain.join(format!("edit{i}")).join(to);
                std::fs::create_dir_all(z.parent().unwrap()).unwrap();
                std::fs::rename(&a, &z).unwrap();
                ask_at(&z, changed, None);
                z
            }
            None => a,
        };
        write(&z, "tsconfig.json", ALIAS_PATHS);
        let refreshed = ask_at(&z, changed, None);
        std::fs::remove_dir_all(z.join(".turbo/trace")).unwrap();
        let cold = ask_at(&z, changed, None);
        if refreshed != cold {
            wrong.push(format!(
                "{from} -> {to:?}, then paths added:\nrefreshed {refreshed}\ncold {cold}"
            ));
        }
        // And the option goes away again.
        write(&z, "tsconfig.json", "{}\n");
        let refreshed = ask_at(&z, changed, None);
        std::fs::remove_dir_all(z.join(".turbo/trace")).unwrap();
        let cold = ask_at(&z, changed, None);
        if refreshed != cold {
            wrong.push(format!(
                "{from} -> {to:?}, then paths removed:\nrefreshed {refreshed}\ncold {cold}"
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n\n"));
}

/// A directory next to the checkout with a config of its own: below
/// `node_modules` its files are files of an installed package, elsewhere its
/// aliases apply.
#[test]
fn a_moved_graph_with_a_sibling_that_has_aliases() {
    let make = |r: &Path| {
        moved_vitest_repo(r, "ts");
        write(
            r,
            "test/a.test.ts",
            "import { s } from '../../shared/s';\nimport { test } from 'vitest';\ntest('a', () => \
             { s; });\n",
        );
    };
    let around = |r: &Path| {
        let p = r.parent().unwrap();
        write(
            p,
            "shared/tsconfig.json",
            r#"{"compilerOptions":{"paths":{"@s/*":["./lib/*"]}}}"#,
        );
        write(p, "shared/s.ts", "export { t as s } from '@s/t';\n");
        write(p, "shared/lib/t.ts", "export const t = 1;\n");
        write(p, "shared/lib/u.ts", "export const u = 1;\n");
    };
    let lab = Lab::new("sibling");
    let mut wrong = Vec::new();
    for (i, way) in [
        ("a/repo", "node_modules/repo"),
        ("node_modules/repo", "b/repo"),
    ]
    .into_iter()
    .enumerate()
    {
        wrong.extend(move_and_compare(
            &lab,
            &format!("sibling{i}"),
            &make,
            &around,
            way,
            "src/lib/y.ts",
            false,
        ));
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n\n"));
}

/// Two checkouts, below directories called `a` and `z`, each with a directory
/// `sib` next to it; a directory `m` next to `a` and `z` is reached by one more
/// step up. By absolute path, `a/sib` sorts before `m` and `z/sib` after it.
fn two_heights(name: &str, scripts: &str, more: &dyn Fn(&Path)) -> Vec<String> {
    let lab = Lab::new(name);
    let base = lab.plain.join("h");
    let places = [
        Place {
            name: "below a directory called a",
            dir: base.join("a/repo"),
            root: base.join("a/repo"),
        },
        Place {
            name: "below a directory called z",
            dir: base.join("z/repo"),
            root: base.join("z/repo"),
        },
    ];
    for f in ["e.js", "i.txt", "x.ignore", "base.json", "in.ts"] {
        let text = match f {
            "base.json" => "{}\n",
            "in.ts" => "export const o = 1;\n",
            _ => "1;\n",
        };
        write(&base, &format!("m/{f}"), text);
        write(&base, &format!("a/sib/{f}"), text);
        write(&base, &format!("z/sib/{f}"), text);
    }
    let mut changes = None;
    let mut seen = Vec::new();
    for place in &places {
        write(
            &place.dir,
            "package.json",
            &format!(r#"{{"name":"o","private":true,"scripts":{{{scripts}}}}}"#),
        );
        write(&place.dir, "zz/b.js", "console.log(2);\n");
        write(&place.dir, "zz/b.ts", "export const b = 1;\n");
        write(&place.dir, "zz/in.json", "{}\n");
        write(&place.dir, "zz/x.ignore", "dist\n");
        more(&place.dir);
        seen.push(see(place, &mut changes));
    }
    compare(
        name,
        &places.iter().collect::<Vec<_>>(),
        &seen.iter().collect::<Vec<_>>(),
    )
}

fn assert_same_everywhere(diffs: Vec<String>) {
    assert!(
        diffs.is_empty(),
        "{} difference(s) between places:\n{}",
        diffs.len(),
        diffs.join("\n")
    );
}

#[test]
fn entries_outside_at_two_heights_order_the_same() {
    assert_same_everywhere(two_heights(
        "ent-2",
        r#""go":"node ../sib/e.js && node ../../m/e.js && node zz/b.js""#,
        &|_| {},
    ));
}

#[test]
fn missing_entries_outside_at_two_heights_order_the_same() {
    assert_same_everywhere(two_heights(
        "entmiss",
        r#""go":"node ../sib/nope.js && node ../../m/nope.js && node zz/nope.js""#,
        &|_| {},
    ));
}

#[test]
fn inputs_outside_at_two_heights_order_the_same() {
    assert_same_everywhere(two_heights(
        "inp-2",
        r#""go":"cat ../sib/i.txt && cat ../../m/i.txt && cat zz/in.json""#,
        &|_| {},
    ));
}

#[test]
fn missing_inputs_outside_at_two_heights_order_the_same() {
    assert_same_everywhere(two_heights(
        "inpmiss",
        r#""go":"cat ../sib/nope.txt && cat ../../m/nope.txt && cat zz/nope.json""#,
        &|_| {},
    ));
}

#[test]
fn configs_outside_at_two_heights_order_the_same() {
    assert_same_everywhere(two_heights(
        "cfg-2",
        r#""go":"eslint --ignore-path ../sib/x.ignore zz && eslint --ignore-path ../../m/x.ignore zz && eslint --ignore-path zz/x.ignore zz""#,
        &|_| {},
    ));
}

#[test]
fn missing_configs_outside_at_two_heights_order_the_same() {
    assert_same_everywhere(two_heights(
        "cfgmiss",
        r#""go":"eslint --ignore-path ../sib/nope.ignore zz && eslint --ignore-path ../../m/nope.ignore zz && eslint --ignore-path zz/nope.ignore zz""#,
        &|_| {},
    ));
}

/// A part (a project of `tsc -b`) whose config extends two configs outside the
/// root, at two heights, next to one inside: by absolute path the configs
/// inside sort before `m` below `a` and after it below `z`. Each group's
/// records are in the order of their stored values ([`turborepo_trace_store`]'s
/// `Tables::intern`), and a path outside the repository is stored as it is, so
/// the order is the same at every place.
///
/// Not part of it: a config next to the checkout (`../sib`). Below `a` and
/// below `z` that is another file, named by its absolute path, so the two
/// shards have other keys (one outside config alone does it, nothing is ordered
/// there): a graph moved to another place traces such a shard again. Keying it
/// by its path from the root would check an `extends` written absolutely at
/// another path after a move.
#[test]
fn a_parts_configs_alone_outside_at_two_heights_order_the_same() {
    assert_same_everywhere(two_heights(
        "part2",
        r#""go":"tsc -b zz/tsconfig.json""#,
        &|r| {
            write(
                r.parent().unwrap().parent().unwrap(),
                "n/deep/base.json",
                "{}\n",
            );
            write(
                r,
                "zz/tsconfig.json",
                r#"{"extends":["../../../n/deep/base.json","../../../m/base.json","./base.json"],"compilerOptions":{"composite":true,"outDir":"../dist"},"files":["b.ts"]}"#,
            );
            write(r, "zz/base.json", "{}\n");
        },
    ));
}

/// The shape of the earlier probes (a climbing path and one written
/// absolutely), with both missing so that both land in the same list.
#[test]
fn two_kinds_of_missing_outside_paths_order_the_same() {
    let lab = Lab::new("kinds");
    let source = lab.plain.join("source");
    write(
        &source,
        "package.json",
        r#"{"name":"o","private":true,"scripts":{"ent":"node ../sib/nope.js && node /u/bin/nope.js && node zz/b.js","inp":"cat ../sib/nope.txt && cat /u/docs/nope.txt && cat zz/in.json","cfg":"eslint --ignore-path ../sib/nope.ignore zz && eslint --ignore-path /u/nope.ignore zz"}}"#,
    );
    write(&source, "zz/b.js", "console.log(2);\n");
    write(&source, "zz/in.json", "{}\n");
    let t = lab._temp.path().canonicalize().unwrap();
    let places = [
        Place {
            name: "below the temporary directory",
            dir: t.join("repo"),
            root: t.join("repo"),
        },
        Place {
            name: "in the target directory",
            dir: lab.plain.join("plain/repo"),
            root: lab.plain.join("plain/repo"),
        },
    ];
    let mut changes = None;
    let mut seen = Vec::new();
    for place in &places {
        copy_dir(&source, &place.dir);
        seen.push(see(place, &mut changes));
    }
    assert_same_everywhere(compare(
        "kinds",
        &places.iter().collect::<Vec<_>>(),
        &seen.iter().collect::<Vec<_>>(),
    ));
}

/// An output written absolutely outside the root next to one inside it: `/u/…`
/// sorts after a checkout below `/tmp` and before one below `/vercel` (or
/// `/Users`).
#[test]
fn an_absolute_output_next_to_one_inside_orders_the_same() {
    let lab = Lab::new("absout");
    let source = lab.plain.join("source");
    write(
        &source,
        "package.json",
        r#"{"name":"o","private":true,"scripts":{"go":"cp zz/in.json /u/out/x.json && cp zz/in.json zz/out.json && node zz/b.js > /u/out/log.txt && node zz/b.js > zz/log.txt"}}"#,
    );
    write(&source, "zz/b.js", "console.log(2);\n");
    write(&source, "zz/in.json", "{}\n");
    let t = lab._temp.path().canonicalize().unwrap();
    let places = [
        Place {
            name: "below the temporary directory",
            dir: t.join("repo"),
            root: t.join("repo"),
        },
        Place {
            name: "in the target directory",
            dir: lab.plain.join("plain/repo"),
            root: lab.plain.join("plain/repo"),
        },
    ];
    let mut changes = None;
    let mut seen = Vec::new();
    for place in &places {
        copy_dir(&source, &place.dir);
        seen.push(see(place, &mut changes));
    }
    assert_same_everywhere(compare(
        "absout",
        &places.iter().collect::<Vec<_>>(),
        &seen.iter().collect::<Vec<_>>(),
    ));
}

/// Tool, not a probe: the shard keys of every fixture at one fixed ordinary
/// place, as built by the binary `TRACE_PROBE_BIN`, written to
/// `TRACE_PROBE_OUT` (to compare an earlier binary with the candidate; the
/// binary must write the candidate's shard format).
#[test]
#[ignore = "tool: needs TRACE_PROBE_BIN and TRACE_PROBE_OUT"]
fn dump_the_keys_of_the_fixtures() {
    let bin = std::env::var("TRACE_PROBE_BIN").unwrap();
    let out = std::env::var("TRACE_PROBE_OUT").unwrap();
    let mut repos = group("frameworks", None);
    repos.extend(group("impact", Some("repo")));
    repos.extend(group("tasks", None));
    repos.extend(group("go", None));
    repos.extend(group("python", None));
    repos.extend(group("rust", None));
    repos.extend(group("shell", None));
    repos.extend(group("js", None));
    repos.extend(one("next-e2e"));
    repos.extend(one("next-build/lab"));
    repos.extend(one("host-paths"));
    let base = support::outside_tmp().join("keys");
    let _ = std::fs::remove_dir_all(&base);
    let mut text = String::new();
    for (name, source) in repos {
        let dir = base.join("repo");
        copy_dir(&source, &dir);
        let o = Command::new(&bin)
            .args(["graph", "build", "--threads", "1", "--root"])
            .arg(&dir)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        for (key, bytes) in shards(&dir.join(".turbo/trace")) {
            text.push_str(&format!(
                "{name} {key} {:032x}\n",
                xxhash_rust::xxh3::xxh3_128(&bytes)
            ));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::write(out, text).unwrap();
}

// ---------------------------------------------------------------------------------------
// Probes (the
// resolver's fact that a config with aliases governs a file,
// `FileRec::aliased`, in place of the guess).

/// What a graph answers at `dir`: the selection on a change of each of
/// `changed`, the open records and the export. With `restore`, a graph
/// directory that holds nothing starts from that file.
fn ask(dir: &Path, changed: &[&str], restore: Option<&Path>) -> String {
    let place = Place {
        name: "probe",
        root: dir.to_path_buf(),
        dir: dir.to_path_buf(),
    };
    restore_graph(dir, restore);
    let mut out = String::new();
    for c in changed {
        let args = vec![
            "affected",
            "--tests",
            "all",
            "--changed",
            c,
            "--format",
            "selection",
        ];
        let doc: Value = serde_json::from_str(&harness(&place, &args)).unwrap();
        out.push_str(&format!("changed {c}: {}\n", doc["packages"]));
    }
    out.push_str(&harness(
        &place,
        &["floor", "--tests", "all", "--no-refresh"],
    ));
    let json = harness(
        &place,
        &["graph", "export", "--no-refresh", "--format", "json"],
    );
    let mut export: Value = serde_json::from_str(&json).unwrap();
    export.as_object_mut().unwrap().remove("root");
    out.push_str(&format!("\nexport {export}"));
    out
}

/// A step of a probe: the checkout moves (to a path below the case's
/// directory), a file is written (`None`: removed), the graph is packed and a
/// fresh copy at another place starts from the pack, or the graph is asked (and
/// so refreshed).
enum MoveStep<'a> {
    Move(&'a str),
    Write(&'a str, Option<&'a str>),
    Pack(&'a str),
    Ask,
}

/// Builds at `start`, runs `steps`, and compares the refreshed answer with a
/// cold one at the last place. The difference, shortened.
fn run(
    lab: &Lab,
    name: &str,
    make: &dyn Fn(&Path),
    start: &str,
    steps: &[MoveStep],
    changed: &[&str],
) -> Option<String> {
    let base = lab.plain.join(name);
    let mut at = base.join(start);
    make(&at);
    ask(&at, changed, None);
    let mut restore = None;
    let mut written: Vec<(String, Option<String>)> = Vec::new();
    for s in steps {
        match s {
            MoveStep::Move(to) => {
                let z = base.join(to);
                std::fs::create_dir_all(z.parent().unwrap()).unwrap();
                std::fs::rename(&at, &z).unwrap();
                at = z;
            }
            MoveStep::Write(rel, content) => {
                match content {
                    Some(c) => write(&at, rel, c),
                    None => std::fs::remove_file(at.join(rel)).unwrap(),
                }
                written.push((rel.to_string(), content.map(str::to_owned)));
            }
            MoveStep::Pack(to) => {
                let file = base.join("graph.pack");
                pack_graph(&at, &file);
                at = base.join(to);
                make(&at);
                for (rel, content) in &written {
                    match content {
                        Some(c) => write(&at, rel, c),
                        None => std::fs::remove_file(at.join(rel)).unwrap(),
                    }
                }
                restore = Some(file);
            }
            MoveStep::Ask => {
                ask(&at, changed, restore.take().as_deref());
            }
        }
    }
    let refreshed = ask(&at, changed, restore.as_deref());
    std::fs::remove_dir_all(at.join(".turbo/trace")).unwrap();
    let cold = ask(&at, changed, None);
    (refreshed != cold).then(|| {
        let short = |s: &str| -> String {
            s.lines()
                .filter(|l| !l.starts_with("export "))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let (r, c) = (short(&refreshed), short(&cold));
        format!(
            "{name}: built at {start}, refreshed at {}{}:\nrefreshed {r}\ncold {c}",
            at.strip_prefix(&base).unwrap().display(),
            if r == c {
                " (only the exports differ)"
            } else {
                ""
            }
        )
    })
}

/// A case: its name, what writes the repository, the changed files asked about.
type BothWaysCase<'a> = (&'a str, &'a dyn Fn(&Path), &'a [&'a str]);
/// A sequence: its name, what writes the repository, where it is built, the
/// steps.
type MoveSequence<'a> = (&'a str, &'a dyn Fn(&Path), &'a str, Vec<MoveStep<'a>>);

/// Each case in both directions across the boundary: moved, and packed and
/// restored.
fn both_ways(lab_name: &str, cases: &[BothWaysCase]) {
    use MoveStep::*;
    let lab = Lab::new(lab_name);
    let mut wrong = Vec::new();
    for (name, make, changed) in cases {
        let ways: [(&str, &[MoveStep]); 4] = [
            ("a/repo", &[Move("node_modules/pkg/repo")]),
            ("node_modules/pkg/repo", &[Move("b/repo")]),
            ("a/repo", &[Pack("node_modules/pkg/repo")]),
            ("node_modules/pkg/repo", &[Pack("b/repo")]),
        ];
        for (i, (start, steps)) in ways.into_iter().enumerate() {
            wrong.extend(run(
                &lab,
                &format!("{name}{i}"),
                make,
                start,
                steps,
                changed,
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n\n"));
}

const ALIAS: &str = r#"{"compilerOptions":{"paths":{"@lib/*":["./lib/*"]}}}"#;

/// A workspace whose root config (jest, a `jsconfig.json` with `paths` next to
/// it) names a setup file through the alias, and whose only test is in a
/// package with a config of its own that gives no alias. The request is
/// resolved as if written in a file next to the runner's config
/// (`Fw::resolve`), so the root's aliases decide it; the record is on the test,
/// in the shard of the package, which has no file that a config with aliases
/// governs.
fn jest_setup(r: &Path) {
    write(
        r,
        "package.json",
        r#"{"name":"root","private":true,"workspaces":["packages/*"],"devDependencies":{"jest":"29.0.0"}}"#,
    );
    write(
        r,
        "jest.config.js",
        "module.exports = { setupFiles: ['@lib/setup'], testMatch: ['**/*.test.js'] };\n",
    );
    write(r, "jsconfig.json", ALIAS);
    write(r, "lib/setup.js", "global.s = 1;\n");
    write(
        r,
        "packages/a/package.json",
        r#"{"name":"a","version":"1.0.0"}"#,
    );
    write(r, "packages/a/jsconfig.json", "{}\n");
    write(r, "packages/a/a.test.js", "test('a', () => {});\n");
}

/// The same with vitest and `tsconfig.json`.
fn vitest_setup(r: &Path) {
    write(
        r,
        "package.json",
        r#"{"name":"root","private":true,"type":"module","workspaces":["packages/*"],"devDependencies":{"vitest":"1.0.0"}}"#,
    );
    write(
        r,
        "vitest.config.ts",
        "export default { test: { include: ['packages/*/test/**/*.test.ts'], setupFiles: \
         ['@lib/setup'] } };\n",
    );
    write(r, "tsconfig.json", ALIAS);
    write(r, "lib/setup.ts", "export const s = 1;\n");
    write(
        r,
        "packages/a/package.json",
        r#"{"name":"a","version":"1.0.0","type":"module"}"#,
    );
    write(r, "packages/a/tsconfig.json", "{}\n");
    write(
        r,
        "packages/a/test/a.test.ts",
        "import { test } from 'vitest';\ntest('a', () => {});\n",
    );
}

#[test]
fn a_moved_graph_with_a_runner_request_through_another_packages_alias() {
    both_ways(
        "fw",
        &[
            ("jest", &jest_setup, &["lib/setup.js"]),
            ("vitest", &vitest_setup, &["lib/setup.ts"]),
        ],
    );
}

/// A vitest package whose test imports `spec`, with the config `tsconfig`.
fn repo(r: &Path, tsconfig: &str, spec: &str) {
    write(
        r,
        "package.json",
        r##"{"name":"p","version":"1.0.0","type":"module","imports":{"#imp":"./lib/i.ts"},"devDependencies":{"vitest":"1.0.0"}}"##,
    );
    write(
        r,
        "vitest.config.ts",
        "export default { test: { include: ['test/**/*.test.ts'] } };\n",
    );
    write(r, "tsconfig.json", tsconfig);
    write(r, "lib/x.ts", "export const x = 1;\n");
    write(r, "lib/i.ts", "export const i = 1;\n");
    write(r, "lib/y.ts", "export const y = 1;\n");
    write(
        r,
        "test/a.test.ts",
        &format!(
            "import {{ x }} from '{spec}';\nimport {{ test }} from 'vitest';\ntest('a', () => {{ \
             x; }});\n"
        ),
    );
}

/// Each way a config gives aliases, alone; and resolution that asks no config
/// (an import with a `.js` extension of a `.ts` file, the manifest's
/// `imports`).
#[test]
fn a_moved_graph_each_kind_of_alias_and_none() {
    let paths = |r: &Path| repo(r, ALIAS, "@lib/x");
    let base_url = |r: &Path| repo(r, r#"{"compilerOptions":{"baseUrl":"."}}"#, "lib/x");
    let root_dirs = |r: &Path| {
        repo(
            r,
            r#"{"compilerOptions":{"rootDirs":["test","lib"]}}"#,
            "./x",
        )
    };
    let none = |r: &Path| {
        repo(r, "{}\n", "../lib/x.js");
        write(
            r,
            "test/b.test.ts",
            "import { i } from '#imp';\nimport { test } from 'vitest';\ntest('b', () => { i; });\n",
        );
    };
    let changed: &[&str] = &["lib/x.ts", "lib/i.ts", "lib/y.ts"];
    both_ways(
        "kinds-2",
        &[
            ("paths", &paths, changed),
            ("baseurl", &base_url, changed),
            ("rootdirs", &root_dirs, changed),
            ("none", &none, changed),
        ],
    );
}

/// The only files that the config with aliases governs are a `.vue` and a
/// `.svelte` file.
#[test]
fn a_moved_graph_where_only_single_file_components_are_governed() {
    let make = |r: &Path| {
        write(
            r,
            "package.json",
            r#"{"name":"p","version":"1.0.0","type":"module","devDependencies":{"vitest":"1.0.0"}}"#,
        );
        write(
            r,
            "vitest.config.mjs",
            "export default { test: { include: ['test/**/*.test.mjs'] } };\n",
        );
        write(
            r,
            "ui/tsconfig.json",
            r#"{"compilerOptions":{"paths":{"@lib/*":["../lib/*"]}}}"#,
        );
        write(r, "lib/x.ts", "export const x = 1;\n");
        write(r, "lib/y.ts", "export const y = 1;\n");
        write(
            r,
            "ui/App.vue",
            "<script setup lang=\"ts\">\nimport { x } from \
             '@lib/x';\nx;\n</script>\n<template><p/></template>\n",
        );
        write(
            r,
            "ui/App.svelte",
            "<script lang=\"ts\">\nimport { y } from '@lib/y';\ny;\n</script>\n<p/>\n",
        );
        write(
            r,
            "test/v.test.mjs",
            "import App from '../ui/App.vue';\nimport { test } from 'vitest';\ntest('v', () => { \
             App; });\n",
        );
        write(
            r,
            "test/s.test.mjs",
            "import S from '../ui/App.svelte';\nimport { test } from 'vitest';\ntest('s', () => { \
             S; });\n",
        );
    };
    both_ways("sfc-2", &[("sfc", &make, &["lib/x.ts", "lib/y.ts"])]);
}

/// Edits and moves in several orders: a file of an aliased shard that a refresh
/// keeps, a config that gains or loses `paths` before, with and after a move,
/// and one that takes them from a config of an installed package that gains or
/// loses them.
#[test]
fn edits_and_moves() {
    use MoveStep::*;
    let with = |r: &Path| repo(r, ALIAS, "@lib/x");
    let without = |r: &Path| repo(r, "{}\n", "@lib/x");
    let ext = |base: &'static str| {
        move |r: &Path| {
            repo(r, r#"{"extends":"@cfg/base/tsconfig.json"}"#, "@lib/x");
            write(
                r,
                "node_modules/@cfg/base/package.json",
                r#"{"name":"@cfg/base","version":"1.0.0"}"#,
            );
            write(r, "node_modules/@cfg/base/tsconfig.json", base);
        }
    };
    const BASE: &str = r#"{"compilerOptions":{"paths":{"@lib/*":["../../../lib/*"]}}}"#;
    let ext_with = ext(BASE);
    let (nm, a, b) = ("node_modules/pkg/repo", "a/repo", "b/repo");
    let edit = Write("lib/y.ts", Some("export const y = 2;\n"));
    let edit_x = Write("lib/x.ts", Some("export const x = 2;\n"));
    let gain = Write("tsconfig.json", Some(ALIAS));
    let lose = Write("tsconfig.json", Some("{}\n"));
    let base_lose = Write("node_modules/@cfg/base/tsconfig.json", Some("{}\n"));
    let cases: Vec<MoveSequence> = vec![
        // A kept file: an unrelated edit, a refresh, then the move.
        ("kept-a", &with, a, vec![edit, Ask, Move(nm)]),
        (
            "kept-nm",
            &with,
            nm,
            vec![
                Write("lib/y.ts", Some("export const y = 2;\n")),
                Ask,
                Move(b),
            ],
        ),
        // The edit and the move in one refresh.
        ("edit+move-a", &with, a, vec![edit_x, Move(nm)]),
        (
            "edit+move-nm",
            &with,
            nm,
            vec![Write("lib/x.ts", Some("export const x = 2;\n")), Move(b)],
        ),
        // Gains and loses at one place.
        ("gain-a", &without, a, vec![gain]),
        ("lose-a", &with, a, vec![lose]),
        (
            "gain-nm",
            &without,
            nm,
            vec![Write("tsconfig.json", Some(ALIAS))],
        ),
        (
            "lose-nm",
            &with,
            nm,
            vec![Write("tsconfig.json", Some("{}\n"))],
        ),
        // Gains or loses, a refresh, then a move; and back.
        (
            "gain-ask-move",
            &without,
            a,
            vec![Write("tsconfig.json", Some(ALIAS)), Ask, Move(nm)],
        ),
        (
            "lose-ask-move",
            &with,
            a,
            vec![Write("tsconfig.json", Some("{}\n")), Ask, Move(nm)],
        ),
        (
            "gain-ask-move-nm",
            &without,
            nm,
            vec![Write("tsconfig.json", Some(ALIAS)), Ask, Move(b)],
        ),
        (
            "lose-ask-move-nm",
            &with,
            nm,
            vec![Write("tsconfig.json", Some("{}\n")), Ask, Move(b)],
        ),
        (
            "lose-move-gain",
            &with,
            a,
            vec![
                Write("tsconfig.json", Some("{}\n")),
                Ask,
                Move(nm),
                Ask,
                Write("tsconfig.json", Some(ALIAS)),
                Ask,
                Move(b),
            ],
        ),
        // With the move, in one refresh.
        (
            "gain+move",
            &without,
            a,
            vec![Write("tsconfig.json", Some(ALIAS)), Move(nm)],
        ),
        (
            "lose+move",
            &with,
            a,
            vec![Write("tsconfig.json", Some("{}\n")), Move(nm)],
        ),
        (
            "gain+move-nm",
            &without,
            nm,
            vec![Write("tsconfig.json", Some(ALIAS)), Move(b)],
        ),
        (
            "lose+move-nm",
            &with,
            nm,
            vec![Write("tsconfig.json", Some("{}\n")), Move(b)],
        ),
        // The config goes away, or appears.
        (
            "removed-move",
            &with,
            a,
            vec![Write("tsconfig.json", None), Ask, Move(nm)],
        ),
        (
            "removed+move-nm",
            &with,
            nm,
            vec![Write("tsconfig.json", None), Move(b)],
        ),
        // `extends` from an installed package.
        ("ext-move", &ext_with, a, vec![Move(nm)]),
        ("ext-move-nm", &ext_with, nm, vec![Move(b)]),
        (
            "ext-lose-move",
            &ext_with,
            a,
            vec![base_lose, Ask, Move(nm)],
        ),
        (
            "ext-lose+move-nm",
            &ext_with,
            nm,
            vec![
                Write("node_modules/@cfg/base/tsconfig.json", Some("{}\n")),
                Move(b),
            ],
        ),
        ("ext-pack", &ext_with, a, vec![Pack(nm)]),
        ("ext-pack-nm", &ext_with, nm, vec![Pack(b)]),
        (
            "gain-pack",
            &without,
            a,
            vec![Write("tsconfig.json", Some(ALIAS)), Pack(nm)],
        ),
        (
            "lose-pack-nm",
            &with,
            nm,
            vec![Write("tsconfig.json", Some("{}\n")), Pack(b)],
        ),
    ];
    let lab = Lab::new("edits");
    let changed: &[&str] = &["lib/x.ts", "lib/y.ts"];
    let mut wrong = Vec::new();
    for (name, make, start, steps) in &cases {
        wrong.extend(run(&lab, name, make, start, steps, changed));
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n\n"));
}

const BASE: &str = r#"{"compilerOptions":{"paths":{"@lib/*":["../../../lib/*"]}}}"#;
const BASE_FILE: &str = "node_modules/@cfg/base/tsconfig.json";
const BASE_MANIFEST: &str = "node_modules/@cfg/base/package.json";

/// A package whose config extends the one of an installed package `@cfg/base`.
fn extends(r: &Path, base: &str) {
    repo(r, r#"{"extends":"@cfg/base/tsconfig.json"}"#, "@lib/x");
    write(
        r,
        BASE_MANIFEST,
        r#"{"name":"@cfg/base","version":"1.0.0"}"#,
    );
    write(r, BASE_FILE, base);
}

fn sequences(lab_name: &str, cases: &[MoveSequence]) {
    let lab = Lab::new(lab_name);
    let changed: &[&str] = &["lib/x.ts", "lib/y.ts"];
    let mut wrong = Vec::new();
    for (name, make, start, steps) in cases {
        wrong.extend(run(&lab, name, make, start, steps, changed));
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n\n"));
}

/// The config of the installed package gains `paths` in place: the same name
/// and version, other contents (a patched install).
#[test]
#[ignore = "note: an installed package's tsconfig edited in place (same name and version) is not \
            seen by a refresh, with or without a move"]
fn an_installed_config_that_gains_paths_in_place() {
    use MoveStep::*;
    let without = |r: &Path| extends(r, "{}\n");
    let (nm, a, b) = ("node_modules/pkg/repo", "a/repo", "b/repo");
    let gain = || Write(BASE_FILE, Some(BASE));
    sequences(
        "inplace",
        &[
            ("gain", &without, a, vec![gain()]),
            ("gain-nm", &without, nm, vec![gain()]),
            ("gain-move", &without, a, vec![gain(), Ask, Move(nm)]),
            ("gain+move-nm", &without, nm, vec![gain(), Move(b)]),
            ("move-gain", &without, a, vec![Move(nm), Ask, gain()]),
        ],
    );
}

/// The same as an install of another version: the manifest's version changes
/// with it.
#[test]
fn an_installed_config_of_another_version_gains_paths() {
    use MoveStep::*;
    let without = |r: &Path| extends(r, "{}\n");
    let (nm, a, b) = ("node_modules/pkg/repo", "a/repo", "b/repo");
    let gain = || {
        [
            Write(BASE_FILE, Some(BASE)),
            Write(
                BASE_MANIFEST,
                Some(r#"{"name":"@cfg/base","version":"2.0.0"}"#),
            ),
        ]
    };
    let seq = |mut before: Vec<MoveStep<'static>>, after: Vec<MoveStep<'static>>| {
        before.extend(gain());
        before.extend(after);
        before
    };
    sequences(
        "version-2",
        &[
            ("gain", &without, a, seq(vec![], vec![])),
            ("gain-nm", &without, nm, seq(vec![], vec![])),
            ("gain-move", &without, a, seq(vec![], vec![Ask, Move(nm)])),
            ("gain+move-nm", &without, nm, seq(vec![], vec![Move(b)])),
            ("gain+move", &without, a, seq(vec![], vec![Move(nm)])),
            ("move-gain", &without, a, seq(vec![Move(nm), Ask], vec![])),
            (
                "move-gain-nm",
                &without,
                nm,
                seq(vec![Move(b), Ask], vec![]),
            ),
        ],
    );
}

/// The config is extended from a package of the workspace, linked into
/// `node_modules` (`"extends": "@cfg/base/tsconfig.json"`, the shared config
/// package of a monorepo).
#[test]
fn a_workspace_config_package_that_gains_or_loses_paths() {
    use MoveStep::*;
    const BASE: &str =
        r#"{"compilerOptions":{"paths":{"@lib/*":["../../lib/*","../../../lib/*"]}}}"#;
    const FILE: &str = "packages/cfg/tsconfig.json";
    let ws = |base: &'static str| {
        move |r: &Path| {
            repo(r, r#"{"extends":"@cfg/base/tsconfig.json"}"#, "@lib/x");
            write(
                r,
                "package.json",
                r#"{"name":"p","version":"1.0.0","type":"module","workspaces":["packages/*"],"devDependencies":{"vitest":"1.0.0","@cfg/base":"*"}}"#,
            );
            write(
                r,
                "packages/cfg/package.json",
                r#"{"name":"@cfg/base","version":"1.0.0"}"#,
            );
            write(r, FILE, base);
            std::fs::create_dir_all(r.join("node_modules/@cfg")).unwrap();
            let _ = std::fs::remove_file(r.join("node_modules/@cfg/base"));
            turborepo_trace_core::os::symlink(
                "../../packages/cfg",
                r.join("node_modules/@cfg/base"),
            )
            .unwrap();
        }
    };
    let (with, without) = (ws(BASE), ws("{}\n"));
    let (nm, a, b) = ("node_modules/pkg/repo", "a/repo", "b/repo");
    let gain = || Write(FILE, Some(BASE));
    let lose = || Write(FILE, Some("{}\n"));
    sequences(
        "ws",
        &[
            ("move", &with, a, vec![Move(nm)]),
            ("move-nm", &with, nm, vec![Move(b)]),
            ("gain", &without, a, vec![gain()]),
            ("gain-nm", &without, nm, vec![gain()]),
            ("gain-move", &without, a, vec![gain(), Ask, Move(nm)]),
            ("gain-move-nm", &without, nm, vec![gain(), Ask, Move(b)]),
            ("gain+move", &without, a, vec![gain(), Move(nm)]),
            ("gain+move-nm", &without, nm, vec![gain(), Move(b)]),
            ("move-gain", &without, a, vec![Move(nm), Ask, gain()]),
            ("move-gain-nm", &without, nm, vec![Move(b), Ask, gain()]),
            ("lose-move", &with, a, vec![lose(), Ask, Move(nm)]),
            ("lose+move-nm", &with, nm, vec![lose(), Move(b)]),
            ("move-lose", &with, a, vec![Move(nm), Ask, lose()]),
        ],
    );
}

/// Every fixture repository, built at one place and moved across the boundary,
/// both ways: the refreshed graph is the cold one (its export, its open
/// records, what a change of nothing selects).
#[test]
fn every_fixture_moved_across_node_modules() {
    let mut repos = group("frameworks", None);
    repos.extend(group("impact", Some("repo")));
    repos.extend(group("tasks", None));
    repos.extend(group("js", None));
    repos.extend(one("next-e2e"));
    repos.extend(one("next-build/lab"));
    repos.extend(one("host-paths"));
    // Cold builds of a multi-crate cargo workspace are not deterministic: remove
    // this exclusion when F3 is fixed.
    repos.retain(|(name, _)| name != "tasks/cargo");
    let lab = Lab::new("fixtures");
    let mut wrong = Vec::new();
    for (i, (name, source)) in repos.iter().enumerate() {
        let make = |r: &Path| copy_dir(source, r);
        for (j, (start, to)) in [
            ("a/repo", "node_modules/pkg/repo"),
            ("node_modules/pkg/repo", "b/repo"),
        ]
        .into_iter()
        .enumerate()
        {
            let case = format!("f{i}w{j}");
            if let Some(d) = run(
                &lab,
                &case,
                &make,
                start,
                &[MoveStep::Move(to)],
                &["no/such/file.txt"],
            ) {
                wrong.push(format!(
                    "{name}: {}",
                    d.chars().take(1200).collect::<String>()
                ));
            }
            let _ = std::fs::remove_dir_all(lab.plain.join(&case));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {} fixtures:\n{}",
        wrong.len(),
        repos.len(),
        wrong.join("\n\n")
    );
}

/// The request of a runner config inside one package: the package's
/// `tsconfig.json` gives no alias and does not cover a `.js` file (no
/// `allowJs`), the root's does both. The request is resolved as if written in a
/// `.js` file next to the config, so the root's aliases decide it, and no file
/// of the package is governed by the root's config.
#[test]
fn a_moved_graph_with_a_runner_request_that_the_config_above_governs() {
    let make = |r: &Path| {
        write(
            r,
            "package.json",
            r#"{"name":"root","private":true,"workspaces":["packages/*"]}"#,
        );
        write(
            r,
            "tsconfig.json",
            r#"{"compilerOptions":{"allowJs":true,"paths":{"@lib/*":["./lib/*"]}}}"#,
        );
        write(r, "lib/setup.ts", "export const s = 1;\n");
        write(
            r,
            "packages/a/package.json",
            r#"{"name":"a","version":"1.0.0","type":"module","devDependencies":{"vitest":"1.0.0"}}"#,
        );
        write(r, "packages/a/tsconfig.json", "{}\n");
        write(
            r,
            "packages/a/vitest.config.ts",
            "export default { test: { include: ['test/**/*.test.ts'], setupFiles: ['@lib/setup'] \
             } };\n",
        );
        write(
            r,
            "packages/a/test/a.test.ts",
            "import { test } from 'vitest';\ntest('a', () => {});\n",
        );
    };
    both_ways("fwin", &[("inside", &make, &["lib/setup.ts"])]);
}

/// A workspace that is not installed: the lockfile resolves `b/feature` of
/// package `a` to the workspace package `b`, and the request is resolved inside
/// `b` as if written in a `.js` file there (`__turbotrace_link__.js`), where
/// `b`'s config gives an alias for the package's own name. The record is on the
/// importing file of `a`.
#[test]
fn a_moved_graph_with_a_lockfile_link_into_a_package_with_aliases() {
    let make = |r: &Path| {
        write(
            r,
            "package.json",
            r#"{"name":"root","private":true,"workspaces":["packages/*"]}"#,
        );
        write(
            r,
            "package-lock.json",
            r#"{"name":"root","lockfileVersion":3,"requires":true,"packages":{"":{"name":"root","workspaces":["packages/*"]},"node_modules/a":{"resolved":"packages/a","link":true},"node_modules/b":{"resolved":"packages/b","link":true},"packages/a":{"name":"a","version":"1.0.0","dependencies":{"b":"*"}},"packages/b":{"name":"b","version":"1.0.0"}}}"#,
        );
        write(
            r,
            "packages/a/package.json",
            r#"{"name":"a","version":"1.0.0","type":"module","dependencies":{"b":"*"},"devDependencies":{"vitest":"1.0.0"}}"#,
        );
        write(r, "packages/a/tsconfig.json", "{}\n");
        write(
            r,
            "packages/a/vitest.config.ts",
            "export default { test: { include: ['test/**/*.test.ts'] } };\n",
        );
        write(
            r,
            "packages/a/test/a.test.ts",
            "import { f } from 'b/feature';\nimport { test } from 'vitest';\ntest('a', () => { f; \
             });\n",
        );
        write(
            r,
            "packages/b/package.json",
            r#"{"name":"b","version":"1.0.0","type":"module","exports":{"./*":"./dist/*.js"}}"#,
        );
        write(
            r,
            "packages/b/tsconfig.json",
            r#"{"compilerOptions":{"allowJs":true,"paths":{"b/*":["./src/*"]}}}"#,
        );
        write(r, "packages/b/src/feature.ts", "export const f = 1;\n");
    };
    both_ways("link-3", &[("link", &make, &["packages/b/src/feature.ts"])]);
}

// ---------------------------------------------------------------------------------------
// Probes (the fact on every resolution a config
// with aliases decides, and on what a link resolution adds).

const ROOT: &str = r#"{"compilerOptions":{"allowJs":true,"paths":{"@lib/*":["./lib/*"]}}}"#;

/// A workspace whose root config gives aliases and covers `.js` files, and a
/// package `a` whose own config gives none and covers no `.js` file: a request
/// resolved for a virtual `.js` file in `a` is decided by the root's aliases,
/// no file of `a` is.
fn workspace(r: &Path, root_tsconfig: &str, a_manifest: &str) {
    write(
        r,
        "package.json",
        r#"{"name":"root","private":true,"workspaces":["packages/*"]}"#,
    );
    write(r, "tsconfig.json", root_tsconfig);
    write(r, "packages/a/package.json", a_manifest);
    write(r, "packages/a/tsconfig.json", "{}\n");
}

const VITEST_A: &str =
    r#"{"name":"a","version":"1.0.0","type":"module","devDependencies":{"vitest":"1.0.0"}}"#;
const JEST_A: &str = r#"{"name":"a","version":"1.0.0","devDependencies":{"jest":"29.0.0"}}"#;

/// The tool of a task is looked up from the package root as if required by a
/// `.js` file there (`__turbotrace_task__.js`): the root's aliases decide where
/// `tsc` and `vitest` are found.
#[test]
fn a_moved_graph_with_a_task_tool_that_the_config_above_aliases() {
    let tsc = |r: &Path| {
        workspace(
            r,
            r#"{"compilerOptions":{"allowJs":true,"paths":{"typescript/*":["./tools/typescript/*"]}}}"#,
            r#"{"name":"a","version":"1.0.0","scripts":{"build":"tsc -p ."}}"#,
        );
        write(
            r,
            "tools/typescript/package.json",
            r#"{"name":"typescript","version":"9.9.9"}"#,
        );
        write(r, "packages/a/src/a.ts", "export const a = 1;\n");
    };
    let vitest = |r: &Path| {
        workspace(
            r,
            r#"{"compilerOptions":{"allowJs":true,"paths":{"vitest/*":["./tools/vitest/*"]}}}"#,
            r#"{"name":"a","version":"1.0.0","type":"module","scripts":{"test":"vitest run"}}"#,
        );
        write(
            r,
            "tools/vitest/package.json",
            r#"{"name":"vitest","version":"9.9.9"}"#,
        );
        write(
            r,
            "packages/a/vitest.config.ts",
            "export default { test: { include: ['test/**/*.test.ts'] } };\n",
        );
        write(r, "packages/a/test/a.test.ts", "test('a', () => {});\n");
    };
    let base_url = |r: &Path| {
        workspace(
            r,
            r#"{"compilerOptions":{"allowJs":true,"baseUrl":"./tools"}}"#,
            r#"{"name":"a","version":"1.0.0","scripts":{"build":"tsc -p ."}}"#,
        );
        write(
            r,
            "tools/typescript/package.json",
            r#"{"name":"typescript","version":"9.9.9"}"#,
        );
        write(r, "packages/a/src/a.ts", "export const a = 1;\n");
    };
    both_ways(
        "task",
        &[
            ("tsc", &tsc, &["tools/typescript/package.json"]),
            ("vitest", &vitest, &["tools/vitest/package.json"]),
            ("baseurl", &base_url, &["tools/typescript/package.json"]),
        ],
    );
}

/// Requests a runner's config makes for a test file, resolved for a virtual
/// `.js` file in the test's directory: a jest `moduleNameMapper` target, a
/// mocked module, a preset that decides which files are tests.
#[test]
fn a_moved_graph_with_other_runner_requests_that_the_config_above_governs() {
    let mapper = |r: &Path| {
        workspace(r, ROOT, JEST_A);
        write(
            r,
            "packages/a/jest.config.js",
            "module.exports = { moduleNameMapper: { '^@x/(.*)$': '@lib/$1' }, testMatch: \
             ['**/*.test.ts'] };\n",
        );
        write(r, "lib/dep.ts", "export const d = 1;\n");
        write(
            r,
            "packages/a/a.test.ts",
            "import { d } from '@x/dep';\ntest('a', () => { d; });\n",
        );
    };
    let mock = |r: &Path| {
        workspace(r, ROOT, VITEST_A);
        write(
            r,
            "packages/a/vitest.config.ts",
            "export default { test: { include: ['test/**/*.test.ts'] } };\n",
        );
        write(r, "lib/dep.ts", "export const d = 1;\n");
        write(
            r,
            "packages/a/test/a.test.ts",
            "import { test, vi } from 'vitest';\nvi.mock('@lib/dep');\ntest('a', () => {});\n",
        );
    };
    let jest_mock = |r: &Path| {
        workspace(r, ROOT, JEST_A);
        write(
            r,
            "packages/a/jest.config.js",
            "module.exports = { testMatch: ['**/*.test.ts'] };\n",
        );
        write(r, "lib/dep.ts", "export const d = 1;\n");
        write(
            r,
            "packages/a/a.test.ts",
            "jest.mock('@lib/dep');\ntest('a', () => {});\n",
        );
    };
    let preset = |r: &Path| {
        workspace(r, ROOT, JEST_A);
        write(
            r,
            "packages/a/jest.config.js",
            "module.exports = { preset: '@lib/p' };\n",
        );
        write(
            r,
            "lib/p/jest-preset.js",
            "module.exports = { testMatch: ['**/*.spec.ts'], setupFiles: ['@lib/setup'] };\n",
        );
        write(r, "lib/setup.ts", "export const s = 1;\n");
        write(r, "packages/a/a.spec.ts", "test('a', () => {});\n");
        write(r, "packages/a/b.test.ts", "test('b', () => {});\n");
    };
    let env = |r: &Path| {
        workspace(r, ROOT, JEST_A);
        write(
            r,
            "packages/a/jest.config.js",
            "module.exports = { testEnvironment: '@lib/env', globalSetup: '@lib/global', \
             transform: { '\\\\.ts$': '@lib/tr' }, testMatch: ['**/*.test.ts'] };\n",
        );
        write(r, "lib/env.ts", "export default class {}\n");
        write(r, "lib/global.ts", "export default () => {};\n");
        write(r, "lib/tr.ts", "export default {};\n");
        write(r, "packages/a/a.test.ts", "test('a', () => {});\n");
    };
    both_ways(
        "fw-2",
        &[
            ("mapper", &mapper, &["lib/dep.ts"]),
            ("mock", &mock, &["lib/dep.ts"]),
            ("jestmock", &jest_mock, &["lib/dep.ts"]),
            ("preset", &preset, &["lib/p/jest-preset.js", "lib/setup.ts"]),
            ("env", &env, &["lib/env.ts", "lib/global.ts", "lib/tr.ts"]),
        ],
    );
}

/// The aliases come from a base the config extends, or from a project the
/// config references: for a runner's request, for a lockfile link, and for a
/// file's own import.
#[test]
fn a_moved_graph_with_aliases_from_an_extended_or_referenced_config() {
    const PROJECT: &str = r#"{"compilerOptions":{"composite":true,"allowJs":true,"paths":{"@lib/*":["./lib/*"]}},"include":["**/*"]}"#;
    let runner = |r: &Path, files: &[(&str, &str)]| {
        workspace(r, "{}\n", VITEST_A);
        for (p, c) in files {
            write(r, p, c);
        }
        write(r, "lib/setup.ts", "export const s = 1;\n");
        write(
            r,
            "packages/a/vitest.config.ts",
            "export default { test: { include: ['test/**/*.test.ts'], setupFiles: ['@lib/setup'] \
             } };\n",
        );
        write(
            r,
            "packages/a/test/a.test.ts",
            "import { test } from 'vitest';\ntest('a', () => {});\n",
        );
    };
    let fw_extends = |r: &Path| {
        runner(
            r,
            &[
                ("tsconfig.json", r#"{"extends":"./config/base.json"}"#),
                (
                    "config/base.json",
                    r#"{"compilerOptions":{"allowJs":true,"paths":{"@lib/*":["../lib/*"]}}}"#,
                ),
            ],
        )
    };
    let fw_extends_base_url = |r: &Path| {
        runner(
            r,
            &[
                ("tsconfig.json", r#"{"extends":"./config/base.json"}"#),
                (
                    "config/base.json",
                    r#"{"compilerOptions":{"allowJs":true,"baseUrl":".."}}"#,
                ),
                ("@lib/setup.ts", "export const s = 2;\n"),
            ],
        )
    };
    let fw_reference = |r: &Path| {
        runner(
            r,
            &[
                (
                    "tsconfig.json",
                    r#"{"files":[],"references":[{"path":"./tsconfig.lib.json"}]}"#,
                ),
                ("tsconfig.lib.json", PROJECT),
            ],
        )
    };
    let link = |r: &Path, b_configs: &[(&str, &str)]| {
        write(
            r,
            "package.json",
            r#"{"name":"root","private":true,"workspaces":["packages/*"]}"#,
        );
        write(
            r,
            "package-lock.json",
            r#"{"name":"root","lockfileVersion":3,"requires":true,"packages":{"":{"name":"root","workspaces":["packages/*"]},"node_modules/a":{"resolved":"packages/a","link":true},"node_modules/b":{"resolved":"packages/b","link":true},"packages/a":{"name":"a","version":"1.0.0","dependencies":{"b":"*"}},"packages/b":{"name":"b","version":"1.0.0"}}}"#,
        );
        write(
            r,
            "packages/a/package.json",
            r#"{"name":"a","version":"1.0.0","type":"module","dependencies":{"b":"*"},"devDependencies":{"vitest":"1.0.0"}}"#,
        );
        write(r, "packages/a/tsconfig.json", "{}\n");
        write(
            r,
            "packages/a/vitest.config.ts",
            "export default { test: { include: ['test/**/*.test.ts'] } };\n",
        );
        write(
            r,
            "packages/a/test/a.test.ts",
            "import { f } from 'b/feature';\nimport { test } from 'vitest';\ntest('a', () => { f; \
             });\n",
        );
        write(
            r,
            "packages/b/package.json",
            r#"{"name":"b","version":"1.0.0","type":"module","exports":{"./*":"./dist/*.js"}}"#,
        );
        for (p, c) in b_configs {
            write(r, p, c);
        }
        write(r, "packages/b/src/feature.ts", "export const f = 1;\n");
    };
    let link_extends = |r: &Path| {
        link(
            r,
            &[
                (
                    "packages/b/tsconfig.json",
                    r#"{"extends":"./tsconfig.base.json"}"#,
                ),
                (
                    "packages/b/tsconfig.base.json",
                    r#"{"compilerOptions":{"allowJs":true,"paths":{"b/*":["./src/*"]}}}"#,
                ),
            ],
        )
    };
    let link_reference = |r: &Path| {
        link(
            r,
            &[
                (
                    "packages/b/tsconfig.json",
                    r#"{"files":[],"references":[{"path":"./tsconfig.src.json"}]}"#,
                ),
                (
                    "packages/b/tsconfig.src.json",
                    r#"{"compilerOptions":{"composite":true,"allowJs":true,"paths":{"b/*":["./src/*"]}},"include":["**/*"]}"#,
                ),
            ],
        )
    };
    // The link resolved inside `b` where no config of `b` covers the `.js` file:
    // the root's config does.
    let link_above = |r: &Path| {
        link(
            r,
            &[
                ("packages/b/tsconfig.json", "{}\n"),
                (
                    "tsconfig.json",
                    r#"{"compilerOptions":{"allowJs":true,"paths":{"b/*":["./packages/b/src/*"]}}}"#,
                ),
            ],
        )
    };
    let own = |r: &Path, files: &[(&str, &str)]| {
        workspace(r, "{}\n", VITEST_A);
        for (p, c) in files {
            write(r, p, c);
        }
        write(r, "lib/x.ts", "export const x = 1;\n");
        write(
            r,
            "packages/a/vitest.config.ts",
            "export default { test: { include: ['test/**/*.test.ts'] } };\n",
        );
        write(
            r,
            "packages/a/test/a.test.ts",
            "import { x } from '@lib/x';\nimport { test } from 'vitest';\ntest('a', () => { x; \
             });\n",
        );
    };
    let own_reference = |r: &Path| {
        own(
            r,
            &[
                (
                    "packages/a/tsconfig.json",
                    r#"{"files":[],"references":[{"path":"./tsconfig.test.json"}]}"#,
                ),
                (
                    "packages/a/tsconfig.test.json",
                    r#"{"compilerOptions":{"composite":true,"paths":{"@lib/*":["../../lib/*"]}},"include":["test/**/*"]}"#,
                ),
            ],
        )
    };
    let own_extends_chain = |r: &Path| {
        own(
            r,
            &[
                (
                    "packages/a/tsconfig.json",
                    r#"{"extends":"../../config/mid.json"}"#,
                ),
                ("config/mid.json", r#"{"extends":"./base.json"}"#),
                (
                    "config/base.json",
                    r#"{"compilerOptions":{"paths":{"@lib/*":["../lib/*"]}}}"#,
                ),
            ],
        )
    };
    both_ways(
        "cfg-3",
        &[
            ("fw-extends", &fw_extends, &["lib/setup.ts"]),
            (
                "fw-extends-baseurl",
                &fw_extends_base_url,
                &["lib/setup.ts", "@lib/setup.ts"],
            ),
            ("fw-reference", &fw_reference, &["lib/setup.ts"]),
            (
                "link-extends",
                &link_extends,
                &["packages/b/src/feature.ts"],
            ),
            (
                "link-reference",
                &link_reference,
                &["packages/b/src/feature.ts"],
            ),
            ("link-above", &link_above, &["packages/b/src/feature.ts"]),
            ("own-reference", &own_reference, &["lib/x.ts"]),
            ("own-extends-chain", &own_extends_chain, &["lib/x.ts"]),
        ],
    );
}

/// A shard whose only tie to a config with aliases is a runner's request: the
/// request or the aliases appear or go with an edit, before or after the move.
#[test]
fn edits_and_moves_of_a_runner_request() {
    use MoveStep::*;
    const WITH: &str = "export default { test: { include: ['test/**/*.test.ts'], setupFiles: \
                        ['@lib/setup'] } };\n";
    const WITHOUT: &str = "export default { test: { include: ['test/**/*.test.ts'] } };\n";
    let repo = |r: &Path, root: &str, config: &str| {
        workspace(r, root, VITEST_A);
        write(r, "lib/setup.ts", "export const s = 1;\n");
        write(r, "packages/a/vitest.config.ts", config);
        write(
            r,
            "packages/a/test/a.test.ts",
            "import { test } from 'vitest';\ntest('a', () => {});\n",
        );
    };
    let with = |r: &Path| repo(r, ROOT, WITH);
    let no_request = |r: &Path| repo(r, ROOT, WITHOUT);
    let no_alias = |r: &Path| repo(r, r#"{"compilerOptions":{"allowJs":true}}"#, WITH);
    let no_js = |r: &Path| repo(r, ALIAS, WITH);
    let (nm, a, b) = ("node_modules/pkg/repo", "a/repo", "b/repo");
    let config = "packages/a/vitest.config.ts";
    let plain = r#"{"compilerOptions":{"allowJs":true}}"#;
    let cases: Vec<MoveSequence> = vec![
        (
            "req-gain-move",
            &no_request,
            a,
            vec![Write(config, Some(WITH)), Ask, Move(nm)],
        ),
        (
            "req-gain-move-nm",
            &no_request,
            nm,
            vec![Write(config, Some(WITH)), Ask, Move(b)],
        ),
        (
            "req-move-gain",
            &no_request,
            a,
            vec![Move(nm), Ask, Write(config, Some(WITH))],
        ),
        (
            "req-gain+move",
            &no_request,
            a,
            vec![Write(config, Some(WITH)), Move(nm)],
        ),
        (
            "req-lose-move",
            &with,
            a,
            vec![Write(config, Some(WITHOUT)), Ask, Move(nm)],
        ),
        (
            "req-lose-move-nm",
            &with,
            nm,
            vec![Write(config, Some(WITHOUT)), Ask, Move(b)],
        ),
        (
            "req-lose+move-nm",
            &with,
            nm,
            vec![Write(config, Some(WITHOUT)), Move(b)],
        ),
        (
            "alias-gain-move",
            &no_alias,
            a,
            vec![Write("tsconfig.json", Some(ROOT)), Ask, Move(nm)],
        ),
        (
            "alias-gain-move-nm",
            &no_alias,
            nm,
            vec![Write("tsconfig.json", Some(ROOT)), Ask, Move(b)],
        ),
        (
            "alias-gain+move",
            &no_alias,
            a,
            vec![Write("tsconfig.json", Some(ROOT)), Move(nm)],
        ),
        (
            "alias-move-gain",
            &no_alias,
            a,
            vec![Move(nm), Ask, Write("tsconfig.json", Some(ROOT))],
        ),
        (
            "alias-lose-move",
            &with,
            a,
            vec![Write("tsconfig.json", Some(plain)), Ask, Move(nm)],
        ),
        (
            "alias-lose-move-nm",
            &with,
            nm,
            vec![Write("tsconfig.json", Some(plain)), Ask, Move(b)],
        ),
        (
            "alias-lose+move-nm",
            &with,
            nm,
            vec![Write("tsconfig.json", Some(plain)), Move(b)],
        ),
        (
            "allowjs-lose-move",
            &with,
            a,
            vec![Write("tsconfig.json", Some(ALIAS)), Ask, Move(nm)],
        ),
        (
            "allowjs-gain-move-nm",
            &no_js,
            nm,
            vec![Write("tsconfig.json", Some(ROOT)), Ask, Move(b)],
        ),
        (
            "setup-edit-move",
            &with,
            a,
            vec![
                Write("lib/setup.ts", Some("export const s = 2;\n")),
                Ask,
                Move(nm),
                Ask,
                Move(b),
            ],
        ),
        (
            "there-and-back",
            &with,
            a,
            vec![Move(nm), Ask, Move(b), Ask, Pack(nm)],
        ),
    ];
    let lab = Lab::new("seq");
    let mut wrong = Vec::new();
    for (name, make, start, steps) in &cases {
        wrong.extend(run(&lab, name, make, start, steps, &["lib/setup.ts"]));
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n\n"));
}

/// The workspace of
/// `a_moved_graph_with_a_lockfile_link_into_a_package_with_aliases`
/// with `b`'s config as given.
fn link(r: &Path, b_tsconfig: &str) {
    write(
        r,
        "package.json",
        r#"{"name":"root","private":true,"workspaces":["packages/*"]}"#,
    );
    write(
        r,
        "package-lock.json",
        r#"{"name":"root","lockfileVersion":3,"requires":true,"packages":{"":{"name":"root","workspaces":["packages/*"]},"node_modules/a":{"resolved":"packages/a","link":true},"node_modules/b":{"resolved":"packages/b","link":true},"packages/a":{"name":"a","version":"1.0.0","dependencies":{"b":"*"}},"packages/b":{"name":"b","version":"1.0.0"}}}"#,
    );
    write(
        r,
        "packages/a/package.json",
        r#"{"name":"a","version":"1.0.0","type":"module","dependencies":{"b":"*"},"devDependencies":{"vitest":"1.0.0"}}"#,
    );
    write(r, "packages/a/tsconfig.json", "{}\n");
    write(
        r,
        "packages/a/vitest.config.ts",
        "export default { test: { include: ['test/**/*.test.ts'] } };\n",
    );
    write(
        r,
        "packages/a/test/a.test.ts",
        "import { f } from 'b/feature';\nimport { test } from 'vitest';\ntest('a', () => { f; \
         });\n",
    );
    write(
        r,
        "packages/b/package.json",
        r#"{"name":"b","version":"1.0.0","type":"module","exports":{"./*":"./dist/*.js"}}"#,
    );
    write(r, "packages/b/tsconfig.json", b_tsconfig);
    write(r, "packages/b/src/feature.ts", "export const f = 1;\n");
}

/// The sequences of the two tests below: built at a place, the steps, then
/// refreshed against cold.
fn link_sequences(lab_name: &str, cases: &[MoveSequence]) {
    let lab = Lab::new(lab_name);
    let mut wrong = Vec::new();
    for (name, make, start, steps) in cases {
        wrong.extend(run(
            &lab,
            name,
            make,
            start,
            steps,
            &["packages/b/src/feature.ts"],
        ));
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n\n"));
}

const LINK_WITH: &str = r#"{"compilerOptions":{"allowJs":true,"paths":{"b/*":["./src/*"]}}}"#;
const LINK_WITHOUT: &str = r#"{"compilerOptions":{"allowJs":true}}"#;
const LINK_CONFIG: &str = "packages/b/tsconfig.json";

/// The config of the package a lockfile link leads into gains or loses its
/// aliases with an edit, and the checkout moves: the sequences that hold.
#[test]
fn edits_and_moves_of_a_lockfile_link() {
    use MoveStep::*;
    let with = |r: &Path| link(r, LINK_WITH);
    let without = |r: &Path| link(r, LINK_WITHOUT);
    let (nm, a, b) = ("node_modules/pkg/repo", "a/repo", "b/repo");
    let gain = || Write(LINK_CONFIG, Some(LINK_WITH));
    let lose = || Write(LINK_CONFIG, Some(LINK_WITHOUT));
    link_sequences(
        "linkseq",
        &[
            ("gain-nm", &without, nm, vec![gain()]),
            ("lose-nm", &with, nm, vec![lose()]),
            ("gain-move", &without, a, vec![gain(), Ask, Move(nm)]),
            ("gain+move", &without, a, vec![gain(), Move(nm)]),
            ("move-gain", &without, a, vec![Move(nm), Ask, gain()]),
            ("lose-move", &with, a, vec![lose(), Ask, Move(nm)]),
            ("lose-move-nm", &with, nm, vec![lose(), Ask, Move(b)]),
            ("lose+move-nm", &with, nm, vec![lose(), Move(b)]),
        ],
    );
}

/// The same where the edit is not seen: the importing package's key holds
/// nothing of the config that governs the virtual file inside the link's
/// target.
#[test]
#[ignore = "note: the tsconfig of a lockfile link's target package gains or loses aliases in \
            place: the importer's shard is not traced again (its key names nothing of that \
            config), with or without a move; the refreshed graph loses the test on a change of the \
            aliased file"]
fn a_lockfile_link_whose_target_config_gains_or_loses_aliases() {
    use MoveStep::*;
    let with = |r: &Path| link(r, LINK_WITH);
    let without = |r: &Path| link(r, LINK_WITHOUT);
    let (nm, a, b) = ("node_modules/pkg/repo", "a/repo", "b/repo");
    let gain = || Write(LINK_CONFIG, Some(LINK_WITH));
    let lose = || Write(LINK_CONFIG, Some(LINK_WITHOUT));
    link_sequences(
        "linkedit",
        &[
            ("gain", &without, a, vec![gain()]),
            ("lose", &with, a, vec![lose()]),
            ("gain-move-nm", &without, nm, vec![gain(), Ask, Move(b)]),
            ("gain+move-nm", &without, nm, vec![gain(), Move(b)]),
        ],
    );
}

/// A runner config that takes what decides its tests from a module it imports
/// through an alias of the root's config, with the tests in a package whose own
/// config gives none: the load of the config follows the import, and the tests'
/// shard has no file that a config with aliases governs.
#[test]
fn a_moved_graph_with_a_runner_config_that_imports_through_an_alias() {
    let root = |r: &Path| {
        write(
            r,
            "package.json",
            r#"{"name":"root","private":true,"type":"module","workspaces":["packages/*"],"devDependencies":{"vitest":"1.0.0"}}"#,
        );
        write(r, "tsconfig.json", ROOT);
        write(
            r,
            "vitest.config.ts",
            "import { include, setupFiles } from '@lib/shared';\nexport default { test: { \
             include, setupFiles } };\n",
        );
        write(
            r,
            "lib/shared.ts",
            "export const include = ['packages/*/spec/**/*.check.ts'];\nexport const setupFiles = \
             ['./lib/setup.ts'];\n",
        );
        write(r, "lib/setup.ts", "export const s = 1;\n");
        write(
            r,
            "packages/a/package.json",
            r#"{"name":"a","version":"1.0.0","type":"module"}"#,
        );
        write(r, "packages/a/tsconfig.json", "{}\n");
        write(r, "packages/a/spec/a.check.ts", "export {};\n");
        write(r, "packages/a/test/b.test.ts", "export {};\n");
    };
    let inside = |r: &Path| {
        workspace(r, ROOT, VITEST_A);
        write(
            r,
            "packages/a/vitest.config.ts",
            "import { include, setupFiles } from '@lib/shared';\nexport default { test: { \
             include, setupFiles } };\n",
        );
        write(
            r,
            "lib/shared.ts",
            "export const include = ['spec/**/*.check.ts'];\nexport const setupFiles = \
             ['../../lib/setup.ts'];\n",
        );
        write(r, "lib/setup.ts", "export const s = 1;\n");
        write(r, "packages/a/spec/a.check.ts", "export {};\n");
        write(r, "packages/a/test/b.test.ts", "export {};\n");
    };
    let jest_root = |r: &Path| {
        write(
            r,
            "package.json",
            r#"{"name":"root","private":true,"workspaces":["packages/*"],"devDependencies":{"jest":"29.0.0"}}"#,
        );
        write(r, "jsconfig.json", ALIAS);
        write(
            r,
            "jest.config.js",
            "const shared = require('@lib/shared');\nmodule.exports = { testMatch: \
             shared.testMatch, setupFiles: shared.setupFiles };\n",
        );
        write(
            r,
            "lib/shared.js",
            "module.exports = { testMatch: ['**/*.check.js'], setupFiles: \
             ['<rootDir>/lib/setup.js'] };\n",
        );
        write(r, "lib/setup.js", "global.s = 1;\n");
        write(
            r,
            "packages/a/package.json",
            r#"{"name":"a","version":"1.0.0"}"#,
        );
        write(r, "packages/a/jsconfig.json", "{}\n");
        write(r, "packages/a/a.check.js", "test('a', () => {});\n");
        write(r, "packages/a/b.test.js", "test('b', () => {});\n");
    };
    both_ways(
        "cfgimport",
        &[
            ("root", &root, &["lib/shared.ts", "lib/setup.ts"]),
            ("inside", &inside, &["lib/shared.ts", "lib/setup.ts"]),
            ("jest-root", &jest_root, &["lib/shared.js", "lib/setup.js"]),
        ],
    );
}
