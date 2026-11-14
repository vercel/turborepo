//! Named paths (`turborepo_trace_graph::names`): a read with a computed path is
//! bounded by the path-like strings and file locations of the code loaded with
//! it.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use turborepo_trace_graph::{
    AffectedOptions, Cause, Change, ChangeKind, ChangeSet, Disposition, Effect, Engine, GraphView,
    MemGraph, NameIndex, NodeId, NodeSet, Origin, RefFlags, RefKind, UncertaintyKind, Verdict,
};

struct Fixture {
    g: MemGraph,
    ix: NameIndex,
}

/// ```text
/// test/a.test.ts → src/reader.ts          readFileSync(p), p a parameter (line 7)
/// test/a.test.ts → test/helpers.ts        the string 'fixtures/a.json' (line 3)
/// test/b.test.ts → src/reader.ts          no naming file in its closure
/// test/c.test.ts → test/helpers.ts        no computed read in its closure
/// test/d.test.ts → task:pkg#build → scripts/build.ts   the string 'logos' (line 9); and → src/reader.ts
/// test/e.test.ts → src/loader.ts          require(p): a module load stays open
/// test/f.test.ts → src/reader.ts, test/local.ts        local.ts uses __dirname (line 2)
/// test/g.test.ts → (type only) test/helpers.ts, → src/reader.ts
/// test/h.test.ts → src/reader.ts, src/table.ts         table.ts reads data/routes.json (fs_read),
///                                                      which has the string 'handlers/get'
/// test/i.test.ts → src/reader.ts, src/indirect.ts      indirect.ts has the string 'data/routes.json'
/// pkg/test/j.test.ts → src/reader.ts, src/cwd.ts       cwd.ts uses process.cwd() (line 5)
/// pkg/test/k.test.ts → src/reader.ts, src/self.ts      self.ts uses testPath (line 6)
/// pkg/test/l.test.ts → src/rel.ts                      readFileSync(name) relative to the cwd (line 8)
/// pkg/test/m.test.ts → src/glob.ts                     a pattern too broad to expand (line 9)
/// ```
fn fixture() -> Fixture {
    let mut b = MemGraph::builder();
    let reader = b.file("src/reader.ts");
    b.uncertainty(reader, UncertaintyKind::ComputedSpecifier, 7, "p")
        .uncertainty_class(Effect::Read, Origin::Param);
    let helpers = b.file("test/helpers.ts");
    let loader = b.file("src/loader.ts");
    b.uncertainty(loader, UncertaintyKind::ComputedSpecifier, 4, "p")
        .uncertainty_class(Effect::Load, Origin::Param);
    let local = b.file("test/local.ts");
    let build = b.file("scripts/build.ts");
    let pkg = b.package(".", None, "");
    let task = b.add_file("task:pkg#build", pkg, Some("task"), false);
    b.edge(task, build, RefKind::Import);
    let table = b.file("src/table.ts");
    let routes = b.file("data/routes.json");
    b.edge(table, routes, RefKind::FsRead);
    let indirect = b.file("src/indirect.ts");
    let cwd = b.file("src/cwd.ts");
    let own = b.file("src/self.ts");
    let rel = b.file("src/rel.ts");
    b.uncertainty(rel, UncertaintyKind::CwdDependent, 8, "name")
        .uncertainty_class(Effect::Read, Origin::Cwd);
    let glob = b.file("src/glob.ts");
    b.uncertainty(glob, UncertaintyKind::PatternTooBroad, 9, "assets/**")
        .uncertainty_class(Effect::Read, Origin::Unknown);
    let t = |b: &mut turborepo_trace_graph::GraphBuilder, path: &str, deps: &[NodeId]| {
        let n = b.file(path);
        for &d in deps {
            b.edge(n, d, RefKind::Import);
        }
        n
    };
    t(&mut b, "test/a.test.ts", &[reader, helpers]);
    t(&mut b, "test/b.test.ts", &[reader]);
    t(&mut b, "test/c.test.ts", &[helpers]);
    t(&mut b, "test/d.test.ts", &[task, reader]);
    t(&mut b, "test/e.test.ts", &[loader, helpers]);
    t(&mut b, "test/f.test.ts", &[reader, local]);
    let g_test = t(&mut b, "test/g.test.ts", &[reader]);
    b.add_edge(
        g_test,
        turborepo_trace_graph::EdgeSpec::new(Some(helpers), RefKind::Import)
            .flags(RefFlags::TYPE_ONLY),
    );
    t(&mut b, "test/h.test.ts", &[reader, table]);
    t(&mut b, "test/i.test.ts", &[reader, indirect]);
    t(&mut b, "pkg/test/j.test.ts", &[reader, cwd]);
    t(&mut b, "pkg/test/k.test.ts", &[reader, own]);
    t(&mut b, "pkg/test/l.test.ts", &[rel]);
    t(&mut b, "pkg/test/m.test.ts", &[glob]);
    b.file("test/fixtures/a.json");
    let g = b.build().unwrap();
    let mut ix = NameIndex::new();
    ix.add_shape(helpers, "fixtures/a.json", 3);
    ix.add_shape(build, "logos", 9);
    ix.add_location(local, "test", 2);
    ix.add_shape(routes, "handlers/get", 4);
    ix.mark_data(routes);
    ix.add_shape(indirect, "data/routes.json", 2);
    ix.add_cwd_user(cwd, 5);
    ix.add_test_path_user(own, 6);
    ix.add_shape(glob, "assets", 9);
    Fixture { g, ix }
}

fn opts() -> AffectedOptions {
    AffectedOptions::default().named(true)
}

fn entries(g: &MemGraph) -> NodeSet {
    Engine::new(g).query("**/*.test.ts").unwrap().nodes
}

/// Every entry runs in the directory above its `test` directory.
fn cwds(g: &MemGraph) -> impl Fn(NodeId) -> Vec<String> + '_ {
    |n| {
        let dir = g.path(n).rsplit_once('/').map_or("", |x| x.0);
        vec![
            dir.strip_suffix("test")
                .unwrap_or(dir)
                .trim_end_matches('/')
                .to_owned(),
        ]
    }
}

/// Entries selected through names.
fn named(f: &Fixture, kind: ChangeKind, path: &str) -> Vec<String> {
    let g = &f.g;
    let e = Engine::new(g);
    let mut named: Vec<String> = e
        .named_affected(
            &f.ix,
            &[Change::new(kind, path)],
            &entries(g),
            None,
            &opts(),
            &cwds(g),
            &|_| None,
        )
        .iter()
        .map(|r| g.path(r.entry).to_owned())
        .collect();
    named.sort();
    named
}

/// Entries selected by the engine's pass (changes and open uncertainty).
fn plain(f: &Fixture, path: &str, o: &AffectedOptions) -> Vec<String> {
    let g = &f.g;
    let mut cs = ChangeSet::new();
    cs.push(ChangeKind::Modified, path);
    let mut v: Vec<String> = Engine::new(g)
        .affected(&cs, &entries(g), o)
        .entries
        .iter()
        .filter(|r| r.verdict == Verdict::Affected)
        .map(|r| g.path(r.entry).to_owned())
        .collect();
    v.sort();
    v
}

#[test]
fn a_named_read_is_not_open() {
    let f = fixture();
    let d = |path: &str, o: &AffectedOptions| {
        let n = f.g.node_by_path(path).unwrap();
        let u = f.g.uncertainties(n).next().unwrap();
        o.disposition(&u, f.g.lang(n))
    };
    for site in ["src/reader.ts", "src/rel.ts", "src/glob.ts"] {
        assert_eq!(
            d(site, &AffectedOptions::default()),
            Disposition::Open,
            "{site}"
        );
        assert_eq!(d(site, &opts()), Disposition::Named, "{site}");
        assert_eq!(d(site, &opts().strict(true)), Disposition::Open, "{site}");
    }
    // A module load with a computed specifier stays open: what it loads brings its
    // own dependencies.
    assert_eq!(d("src/loader.ts", &opts()), Disposition::Open);
}

#[test]
fn an_unrelated_change_selects_only_open_uncertainty() {
    let f = fixture();
    // Without names every test that reaches a read is selected.
    assert_eq!(
        plain(&f, "README.md", &AffectedOptions::default()).len(),
        12
    );
    // With names only the module load keeps its test selected.
    assert_eq!(plain(&f, "README.md", &opts()), ["test/e.test.ts"]);
    // README.md is at the repository root: nothing names it, and no entry runs
    // there.
    assert!(named(&f, ChangeKind::Modified, "README.md").is_empty());
}

#[test]
fn a_change_to_a_named_path_selects_the_tests_that_load_name_and_read() {
    let f = fixture();
    for kind in [ChangeKind::Modified, ChangeKind::Added, ChangeKind::Deleted] {
        // a: loads the helper with the string and the reader. f: test/local.ts names
        // everything below test/. Not b (no name), c (no computed read), d (the string
        // 'logos' does not match), g (a type-only import loads no code).
        assert_eq!(
            named(&f, kind, "test/fixtures/a.json"),
            ["test/a.test.ts", "test/f.test.ts"],
            "{kind:?}"
        );
    }
    // A file added later anywhere with that shape.
    assert_eq!(
        named(&f, ChangeKind::Added, "other/fixtures/a.json"),
        ["test/a.test.ts"]
    );
    // The same for entries evaluated without type-only edges.
    let g = &f.g;
    let e = Engine::new(g);
    let all = entries(g);
    let named: Vec<&str> = e
        .named_affected(
            &f.ix,
            &[Change::new(ChangeKind::Added, "other/fixtures/a.json")],
            &all,
            Some(&all),
            &opts(),
            &cwds(g),
            &|_| None,
        )
        .iter()
        .map(|r| g.path(r.entry))
        .collect();
    assert_eq!(named, ["test/a.test.ts"]);
}

#[test]
fn names_do_not_travel_through_tasks() {
    let f = fixture();
    // scripts/build.ts names `logos`; test d reaches it only through a task, so the
    // string cannot flow into the read that test d loads.
    assert!(named(&f, ChangeKind::Modified, "static/logos/x.svg").is_empty());
}

/// A script with a named read and a name of its own: the consumers of a task
/// that runs it are affected when a path the script names changes; the
/// consumers of a task that only reads it (a type-check) are not.
#[test]
fn a_script_passes_its_names_on_through_the_task_that_runs_it() {
    let mut b = MemGraph::builder();
    let pkg = b.package(".", None, "");
    let script = b.file("scripts/build.ts");
    b.uncertainty(script, UncertaintyKind::ComputedSpecifier, 12, "file")
        .uncertainty_class(Effect::Read, Origin::Param);
    let runs = b.add_file("task:pkg#build", pkg, Some("task"), false);
    b.edge(runs, script, RefKind::Import);
    let checks = b.add_file("task:pkg#typecheck", pkg, Some("task"), false);
    b.add_edge(
        checks,
        turborepo_trace_graph::EdgeSpec::new(Some(script), RefKind::FsRead).flags(RefFlags::DATA),
    );
    let ran = b.file("test/ran.test.ts");
    b.edge(ran, runs, RefKind::Import);
    let checked = b.file("test/checked.test.ts");
    b.edge(checked, checks, RefKind::Import);
    // A runner config that names the script as a reporter: a data edge between
    // files.
    let config = b.file("runner.config.ts");
    b.add_edge(
        config,
        turborepo_trace_graph::EdgeSpec::new(Some(script), RefKind::Import).flags(RefFlags::DATA),
    );
    let configured = b.file("test/configured.test.ts");
    b.edge(configured, config, RefKind::Import);
    let g = b.build().unwrap();
    let mut ix = NameIndex::new();
    ix.add_shape(script, "logos", 9);
    let f = Fixture { g, ix };
    assert_eq!(
        named(&f, ChangeKind::Modified, "static/logos/x.svg"),
        ["test/ran.test.ts"]
    );
    assert!(named(&f, ChangeKind::Modified, "static/other/x.svg").is_empty());
}

#[test]
fn data_files_name_paths_for_their_readers() {
    let f = fixture();
    // data/routes.json has the string 'handlers/get'. h loads the file that reads
    // it through an edge. i only names the data file: which file it reads is
    // not known, so the data file's strings are not its names.
    assert_eq!(
        named(&f, ChangeKind::Modified, "svc/handlers/get.ts"),
        ["test/h.test.ts"]
    );
    // The data file itself is named by src/indirect.ts.
    assert_eq!(
        named(&f, ChangeKind::Modified, "data/routes.json"),
        ["test/i.test.ts"]
    );
    // A read whose path comes from parsed data is bounded like the others.
    let mut b = MemGraph::builder();
    let n = b.file("src/data.ts");
    b.uncertainty(n, UncertaintyKind::ComputedSpecifier, 4, "row.path")
        .uncertainty_class(Effect::Read, Origin::Data);
    let g = b.build().unwrap();
    let u = g.uncertainties(n).next().unwrap();
    assert_eq!(opts().disposition(&u, g.lang(n)), Disposition::Named);
}

#[test]
fn the_working_directory_and_the_test_directory_are_names_of_the_entry() {
    let f = fixture();
    // j: src/cwd.ts uses the working directory, and j runs in pkg/. k: src/self.ts
    // uses the test path, and k is in pkg/test/. l: its read is relative to the
    // working directory.
    assert_eq!(
        named(&f, ChangeKind::Added, "pkg/test/data/new.bin"),
        [
            "pkg/test/j.test.ts",
            "pkg/test/k.test.ts",
            "pkg/test/l.test.ts"
        ]
    );
    assert_eq!(
        named(&f, ChangeKind::Modified, "pkg/src/x.ts"),
        ["pkg/test/j.test.ts", "pkg/test/l.test.ts"]
    );
    assert!(named(&f, ChangeKind::Modified, "elsewhere/x.ts").is_empty());
    // A file that steps up from the working directory reaches that much further.
    let mut up = fixture();
    let cwd = up.g.node_by_path("src/cwd.ts").unwrap();
    up.ix.set_ups(cwd, 1);
    assert_eq!(
        named(&up, ChangeKind::Modified, "elsewhere/x.ts"),
        ["pkg/test/j.test.ts"]
    );
    // m: the pattern's directory is a string of the file.
    assert_eq!(
        named(&f, ChangeKind::Added, "pkg/assets/deep/x.png"),
        [
            "pkg/test/j.test.ts",
            "pkg/test/l.test.ts",
            "pkg/test/m.test.ts"
        ]
    );
    assert_eq!(
        named(&f, ChangeKind::Added, "other/assets/x.png"),
        ["pkg/test/m.test.ts"]
    );
}

#[test]
fn the_cause_names_the_string_and_the_read() {
    let f = fixture();
    let g = &f.g;
    let e = Engine::new(g);
    let r = e.named_affected(
        &f.ix,
        &[Change::new(ChangeKind::Modified, "test/fixtures/a.json")],
        &entries(g),
        None,
        &opts(),
        &cwds(g),
        &|_| None,
    );
    let a = r
        .iter()
        .find(|r| g.path(r.entry) == "test/a.test.ts")
        .unwrap();
    let chain: Vec<&str> = a.witness.iter().map(|&n| g.path(n)).collect();
    assert_eq!(chain, ["test/helpers.ts", "test/a.test.ts"]);
    assert_eq!(
        a.cause,
        Cause::Named {
            path: "test/fixtures/a.json".to_owned(),
            by: "the string 'fixtures/a.json'".to_owned(),
            line: 3,
            site: "src/reader.ts".to_owned(),
            site_line: 7,
        }
    );
    let mut report = e.affected(&ChangeSet::new(), &entries(g), &opts());
    report.entries = r.into_iter().map(|r| r.into_entry()).collect();
    let mut out = Vec::new();
    turborepo_trace_graph::output::write_affected_explain(g, &report, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains(
            "affected test/a.test.ts: changed test/fixtures/a.json, named by the string \
             'fixtures/a.json' in test/helpers.ts:3 (loaded with the read at src/reader.ts:7, \
             whose path is computed): test/helpers.ts → test/a.test.ts"
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "affected test/f.test.ts: changed test/fixtures/a.json, named by its own location \
             (below test/) in test/local.ts:2 (loaded with the read at src/reader.ts:7, whose \
             path is computed): test/local.ts → test/f.test.ts"
        ),
        "{text}"
    );
}

/// ```text
/// pkg/sub/test/n.test.ts → src/reader.ts, src/near.ts, src/mid.ts → src/far.ts
///     near.ts uses the working directory; far.ts uses it and has two parent steps
/// pkg/sub/test/o.test.ts → src/reader.ts, src/envs.ts
///     envs.ts hands the working directory and the one two above it to a reader of `.env*`
/// ```
fn cwd_fixture() -> Fixture {
    let mut b = MemGraph::builder();
    let reader = b.file("src/reader.ts");
    b.uncertainty(reader, UncertaintyKind::ComputedSpecifier, 7, "p")
        .uncertainty_class(Effect::Read, Origin::Param);
    let near = b.file("src/near.ts");
    let far = b.file("src/far.ts");
    let mid = b.file("src/mid.ts");
    b.edge(mid, far, RefKind::Import);
    let envs = b.file("src/envs.ts");
    let n = b.file("pkg/sub/test/n.test.ts");
    for d in [reader, near, mid] {
        b.edge(n, d, RefKind::Import);
    }
    let o = b.file("pkg/sub/test/o.test.ts");
    for d in [reader, envs] {
        b.edge(o, d, RefKind::Import);
    }
    let g = b.build().unwrap();
    let mut ix = NameIndex::new();
    ix.add_cwd_user(near, 3);
    ix.add_cwd_user(far, 4);
    ix.set_ups(far, 2);
    ix.add_cwd_dir(envs, "", ".env*", 5);
    ix.add_cwd_dir(envs, "../..", ".env*", 5);
    Fixture { g, ix }
}

#[test]
fn an_entry_is_asked_about_every_working_directory_user_it_loads() {
    let f = cwd_fixture();
    let n = ["pkg/sub/test/n.test.ts".to_owned()];
    // Below the working directory: the nearest user names it.
    assert_eq!(named(&f, ChangeKind::Modified, "pkg/sub/data/x.txt"), n);
    // Two directories above it: only the user further away, with its parent steps,
    // does.
    assert_eq!(named(&f, ChangeKind::Modified, "other/x.txt"), n);
}

#[test]
fn directories_on_the_working_directory_name_their_pattern() {
    let f = cwd_fixture();
    let o = "pkg/sub/test/o.test.ts".to_owned();
    let selects = |path: &str| named(&f, ChangeKind::Modified, path).contains(&o);
    // `.env*` directly in the working directory and two directories above it.
    assert!(selects("pkg/sub/.env.local"));
    assert!(selects(".env"));
    // Not in between, not below, not other files there.
    assert!(!selects("pkg/.env"));
    assert!(!selects("pkg/sub/conf/.env"));
    assert!(!selects("pkg/sub/env.ts"));
}

/// Packages: `pkg/a` (with `pkg/a/nested` inside it), `pkg/b`, `pkg/c`,
/// `pkg/lib`, `pkg/w`, and the root.
///
/// ```text
/// pkg/lib/reader.ts            readFileSync(p), p a parameter (line 7): the only named read
/// pkg/a/test/a.test.ts  → reader, pkg/a/src/paths.ts    the string 'fixtures' (line 3)
/// pkg/a/nested/test/n.test.ts → reader, pkg/a/src/paths.ts
/// pkg/b/test/b.test.ts  → reader, pkg/b/src/paths.ts    the string 'fixtures' (line 3)
/// test/root.test.ts     → reader, pkg/b/src/paths.ts
/// pkg/w/test/w.test.ts  → reader, pkg/w/src/up.ts       uses the working directory, two parent steps
///                       → pkg/w/src/sibling.ts          hands `cwd/../b` to a function (followed)
///                       → pkg/w/src/loc.ts              its location goes to pkg/a/data (followed)
/// ```
fn package_fixture() -> Fixture {
    let mut b = MemGraph::builder();
    let root = b.package(".", None, "");
    let pa = b.package("a", None, "pkg/a");
    let pn = b.package("nested", None, "pkg/a/nested");
    let pb = b.package("b", None, "pkg/b");
    let pl = b.package("lib", None, "pkg/lib");
    let pc = b.package("c", None, "pkg/c");
    let pw = b.package("w", None, "pkg/w");
    let ts = Some("js");
    b.add_file("pkg/c/index.ts", pc, ts, false);
    b.add_file("pkg/a/nested/index.ts", pn, ts, false);
    let reader = b.add_file("pkg/lib/reader.ts", pl, ts, false);
    b.uncertainty(reader, UncertaintyKind::ComputedSpecifier, 7, "p")
        .uncertainty_class(Effect::Read, Origin::Param);
    let a_paths = b.add_file("pkg/a/src/paths.ts", pa, ts, false);
    let b_paths = b.add_file("pkg/b/src/paths.ts", pb, ts, false);
    let up = b.add_file("pkg/w/src/up.ts", pw, ts, false);
    let sibling = b.add_file("pkg/w/src/sibling.ts", pw, ts, false);
    let loc = b.add_file("pkg/w/src/loc.ts", pw, ts, false);
    let a_test = b.add_file("pkg/a/test/a.test.ts", pa, ts, false);
    for d in [reader, a_paths] {
        b.edge(a_test, d, RefKind::Import);
    }
    let n_test = b.add_file("pkg/a/nested/test/n.test.ts", pn, ts, false);
    for d in [reader, a_paths] {
        b.edge(n_test, d, RefKind::Import);
    }
    let b_test = b.add_file("pkg/b/test/b.test.ts", pb, ts, false);
    for d in [reader, b_paths] {
        b.edge(b_test, d, RefKind::Import);
    }
    let root_test = b.add_file("test/root.test.ts", root, ts, false);
    for d in [reader, b_paths] {
        b.edge(root_test, d, RefKind::Import);
    }
    let w_test = b.add_file("pkg/w/test/w.test.ts", pw, ts, false);
    for d in [reader, up, sibling, loc] {
        b.edge(w_test, d, RefKind::Import);
    }
    let g = b.build().unwrap();
    let mut ix = NameIndex::new();
    ix.add_shape(a_paths, "fixtures", 3);
    ix.add_shape(b_paths, "fixtures", 3);
    ix.add_cwd_user(up, 4);
    ix.set_ups(up, 2);
    ix.add_cwd_dir(sibling, "../b", "**", 5);
    ix.add_location(loc, "pkg/a/data", 6);
    Fixture { g, ix }
}

/// Entries selected through names for a modification of `path`, with every test
/// running in its package directory; `assume`: the package assumption.
fn named_in(f: &Fixture, path: &str, assume: bool) -> Vec<String> {
    let g = &f.g;
    let e = Engine::new(g);
    let cwds = |n: NodeId| {
        vec![
            g.path(n)
                .split_once("/test/")
                .map_or("", |x| x.0)
                .to_owned(),
        ]
    };
    let mut named: Vec<String> = e
        .named_affected(
            &f.ix,
            &[Change::new(ChangeKind::Modified, path)],
            &entries(g),
            None,
            &opts().named_in_package(assume),
            &cwds,
            &|_| None,
        )
        .iter()
        .map(|r| g.path(r.entry).to_owned())
        .collect();
    named.sort();
    named
}

#[test]
fn a_string_names_a_path_only_in_the_package_of_the_entry_or_of_the_read() {
    let f = package_fixture();
    let (n, a, b, r) = (
        "pkg/a/nested/test/n.test.ts",
        "pkg/a/test/a.test.ts",
        "pkg/b/test/b.test.ts",
        "test/root.test.ts",
    );
    // The tests with the string (w.test.ts has none; its names are the next
    // test's).
    let strings = |path: &str, assume: bool| -> Vec<String> {
        let mut v = named_in(&f, path, assume);
        v.retain(|t| !t.starts_with("pkg/w/"));
        v
    };
    let all = [n, a, b, r];
    // In the entry's package: only the tests of that package, whichever file has
    // the string. A package inside another one is part of it: both tests below
    // pkg/a.
    assert_eq!(strings("pkg/a/fixtures/x.json", true), [n, a]);
    assert_eq!(strings("pkg/b/fixtures/x.json", true), [b]);
    // In the package of the file with the read: every test that loads the read and
    // a name.
    assert_eq!(strings("pkg/lib/fixtures/x.json", true), all);
    // In a package that neither a test nor the read belongs to: nobody.
    assert!(strings("pkg/c/fixtures/x.json", true).is_empty());
    assert_eq!(strings("pkg/a/nested/fixtures/x.json", true), [n, a]);
    // The root package is its own files: the root without the other packages.
    assert_eq!(strings("fixtures/x.json", true), [r]);
    assert_eq!(strings("docs/fixtures/x.json", true), [r]);
    // Without the assumption a string names the shape anywhere.
    for path in [
        "pkg/a/fixtures/x.json",
        "pkg/c/fixtures/x.json",
        "fixtures/x.json",
    ] {
        assert_eq!(strings(path, false), all, "{path}");
    }
    // Not the shape: nobody, either way.
    assert!(strings("pkg/a/other/x.json", false).is_empty());
}

#[test]
fn names_that_say_where_they_go_are_not_bounded_by_the_package() {
    let f = package_fixture();
    let w = ["pkg/w/test/w.test.ts"];
    // A location that goes into another package names what is there.
    assert_eq!(named_in(&f, "pkg/a/data/x.bin", true), w);
    // A followed working-directory path: `cwd/../b` of a test of pkg/w is pkg/b.
    assert_eq!(named_in(&f, "pkg/b/zz/x.bin", true), w);
    // The working directory as such, raised by the file's parent steps, stays in
    // the package: two steps above pkg/w is the root, but the root is not a
    // file of pkg/w.
    assert_eq!(named_in(&f, "pkg/w/zz/x.bin", true), w);
    assert!(named_in(&f, "zz/x.bin", true).is_empty());
    assert!(named_in(&f, "pkg/c/zz/x.bin", true).is_empty());
    // It does reach the package of the file with the read.
    assert_eq!(named_in(&f, "pkg/lib/zz/x.bin", true), w);
    // Without the assumption it names everything below the raised directory.
    assert_eq!(named_in(&f, "zz/x.bin", false), w);
    assert_eq!(named_in(&f, "pkg/c/zz/x.bin", false), w);
}
