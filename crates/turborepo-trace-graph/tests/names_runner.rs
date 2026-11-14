//! Who runs a process (`turborepo_trace_graph::names`): a script that an entry
//! reaches only through a build task takes its working directory and package
//! from the task that runs it, not from the entry that consumes the task's
//! output.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use turborepo_trace_graph::{
    AffectedOptions, Cause, Change, ChangeKind, Effect, Engine, GraphView, MemGraph, NameIndex,
    NodeId, NodeSet, Origin, RefKind, UncertaintyKind,
};

struct Fixture {
    g: MemGraph,
    ix: NameIndex,
}

/// Packages `pkg/built`, `pkg/mid`, `pkg/odd`, `pkg/app`, `pkg/lib` and the
/// root.
///
/// ```text
/// pkg/built/build.ts          uses the working directory (line 4) → pkg/lib/reader.ts
///                             the string 'fixtures' (line 6)
/// pkg/lib/reader.ts           readFileSync(p), p a parameter (line 7)
/// task:pkg/built#build        runs pkg/built/build.ts, in pkg/built
/// task:pkg/mid#build          depends on task:pkg/built#build, runs in pkg/mid
/// pkg/odd/build.ts            uses the working directory (line 4) → pkg/lib/reader.ts
/// task:pkg/odd#build          runs pkg/odd/build.ts; where it runs is not known
/// task:.#root                 runs scripts/root.ts (the working directory, line 2), at the root
/// pkg/sh/gen.ts               uses the working directory (line 4) → pkg/lib/reader.ts
/// pkg/sh/gen.sh               a shell script that runs pkg/sh/gen.ts
/// task:pkg/sh#build           runs pkg/sh/gen.sh, in pkg/sh
///
/// pkg/app/test/output.test.ts   → task:pkg/built#build           consumes the output
/// pkg/app/test/direct.test.ts   → pkg/built/build.ts             loads the script itself
/// pkg/app/test/both.test.ts     → the task and the script
/// pkg/app/test/chain.test.ts    → task:pkg/mid#build             two tasks on the way
/// pkg/app/test/odd.test.ts      → task:pkg/odd#build
/// pkg/app/test/root.test.ts     → task:.#root
/// pkg/app/test/shell.test.ts    → task:pkg/sh#build
/// ```
fn fixture() -> Fixture {
    let mut b = MemGraph::builder();
    let root = b.package(".", None, "");
    let built = b.package("built", None, "pkg/built");
    let mid = b.package("mid", None, "pkg/mid");
    let odd = b.package("odd", None, "pkg/odd");
    let app = b.package("app", None, "pkg/app");
    let lib = b.package("lib", None, "pkg/lib");
    let sh = b.package("sh", None, "pkg/sh");
    let ts = Some("js");
    let reader = b.add_file("pkg/lib/reader.ts", lib, ts, false);
    b.uncertainty(reader, UncertaintyKind::ComputedSpecifier, 7, "p")
        .uncertainty_class(Effect::Read, Origin::Param);
    let build = b.add_file("pkg/built/build.ts", built, ts, false);
    b.edge(build, reader, RefKind::Import);
    let t_built = b.add_file("task:pkg/built#build", built, Some("task"), false);
    b.edge(t_built, build, RefKind::Import);
    let t_mid = b.add_file("task:pkg/mid#build", mid, Some("task"), false);
    b.edge(t_mid, t_built, RefKind::Import);
    let odd_build = b.add_file("pkg/odd/build.ts", odd, ts, false);
    b.edge(odd_build, reader, RefKind::Import);
    let t_odd = b.add_file("task:pkg/odd#build", odd, Some("task"), false);
    b.edge(t_odd, odd_build, RefKind::Import);
    let root_script = b.add_file("scripts/root.ts", root, ts, false);
    b.edge(root_script, reader, RefKind::Import);
    let t_root = b.add_file("task:.#root", root, Some("task"), false);
    b.edge(t_root, root_script, RefKind::Import);
    let gen_ts = b.add_file("pkg/sh/gen.ts", sh, ts, false);
    b.edge(gen_ts, reader, RefKind::Import);
    let gen_sh = b.add_file("pkg/sh/gen.sh", sh, Some("shell"), false);
    b.edge(gen_sh, gen_ts, RefKind::ChildProcess);
    let t_sh = b.add_file("task:pkg/sh#build", sh, Some("task"), false);
    b.edge(t_sh, gen_sh, RefKind::ChildProcess);
    let test = |b: &mut turborepo_trace_graph::GraphBuilder, name: &str, deps: &[NodeId]| {
        let n = b.add_file(&format!("pkg/app/test/{name}.test.ts"), app, ts, false);
        for &d in deps {
            b.edge(n, d, RefKind::Import);
        }
    };
    test(&mut b, "output", &[t_built]);
    test(&mut b, "direct", &[build]);
    test(&mut b, "both", &[t_built, build]);
    test(&mut b, "chain", &[t_mid]);
    test(&mut b, "odd", &[t_odd]);
    test(&mut b, "root", &[t_root]);
    test(&mut b, "shell", &[t_sh]);
    let g = b.build().unwrap();
    let mut ix = NameIndex::new();
    ix.add_cwd_user(build, 4);
    ix.add_shape(build, "fixtures", 6);
    ix.add_cwd_user(odd_build, 4);
    ix.add_cwd_user(root_script, 2);
    ix.add_cwd_user(gen_ts, 4);
    Fixture { g, ix }
}

fn results(f: &Fixture, path: &str, by_runner: bool) -> Vec<(String, Cause)> {
    let g = &f.g;
    let e = Engine::new(g);
    let entries: NodeSet = e.query("**/*.test.ts").unwrap().nodes;
    // A test runs in its package directory.
    let cwds = |n: NodeId| {
        vec![
            g.path(n)
                .split_once("/test/")
                .map_or("", |x| x.0)
                .to_owned(),
        ]
    };
    // A task runs in the directory of its id; `pkg/odd`'s somewhere else.
    let tasks = |n: NodeId| {
        let dir = g.path(n).strip_prefix("task:")?.rsplit_once('#')?.0;
        (dir != "pkg/odd").then(|| {
            vec![if dir == "." {
                String::new()
            } else {
                dir.to_owned()
            }]
        })
    };
    let opts = AffectedOptions::default()
        .named(true)
        .named_by_runner(by_runner);
    let mut out: Vec<(String, Cause)> = e
        .named_affected(
            &f.ix,
            &[Change::new(ChangeKind::Modified, path)],
            &entries,
            None,
            &opts,
            &cwds,
            &tasks,
        )
        .into_iter()
        .map(|r| {
            let name = g.path(r.entry);
            let name = name.rsplit('/').next().unwrap_or(name);
            (name.trim_end_matches(".test.ts").to_owned(), r.cause)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn named(f: &Fixture, path: &str, by_runner: bool) -> Vec<String> {
    results(f, path, by_runner)
        .into_iter()
        .map(|r| r.0)
        .collect()
}

#[test]
fn a_file_of_the_consumer_is_not_named_by_a_script_its_task_runs_elsewhere() {
    let f = fixture();
    // The consumer's own directory names the file only for the tests that run the
    // script themselves, and for the one whose task runs somewhere that is not
    // known.
    assert_eq!(
        named(&f, "pkg/app/src/page.ts", true),
        ["both", "direct", "odd", "shell"]
    );
    // Before runners: every test that reaches a script that uses the working
    // directory.
    assert_eq!(
        named(&f, "pkg/app/src/page.ts", false),
        ["both", "chain", "direct", "odd", "output", "root", "shell"]
    );
}

#[test]
fn a_file_the_task_can_read_selects_the_consumers_of_its_output() {
    let f = fixture();
    // Below the directory the task runs in, in the task's package: the tests above
    // the task, through one task or two. The test that only loads the script
    // runs it in its own directory.
    assert_eq!(
        named(&f, "pkg/built/data/x.json", true),
        ["both", "chain", "output"]
    );
    // Before runners nobody: the path is neither below the tests' directory nor in
    // their package.
    assert_eq!(named(&f, "pkg/built/data/x.json", false), [] as [&str; 0]);
    let r = results(&f, "pkg/built/data/x.json", true);
    let Cause::Named { by, site, .. } = &r.iter().find(|r| r.0 == "output").unwrap().1 else {
        panic!("not a named cause");
    };
    assert_eq!(
        by,
        "the working directory of task:pkg/built#build (pkg/built/)"
    );
    assert_eq!(site, "pkg/lib/reader.ts");
}

#[test]
fn a_string_counts_in_the_package_of_the_task() {
    let f = fixture();
    // `fixtures` of build.ts, in the package the script runs for.
    let r = results(&f, "pkg/built/fixtures/a.json", true);
    assert_eq!(
        r.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
        ["both", "chain", "output"]
    );
    assert!(
        matches!(&r[2].1, Cause::Named { by, .. } if by == "the string 'fixtures'"),
        "{:?}",
        r[2].1
    );
    // The same shape in the consumer's package: the string counts only for the
    // tests that load the script (the tests above the task of pkg/odd and above
    // the shell script are there for their own directory).
    let by_string = |by_runner: bool| -> Vec<String> {
        results(&f, "pkg/app/fixtures/a.json", by_runner)
            .into_iter()
            .filter(|r| matches!(&r.1, Cause::Named { by, .. } if by == "the string 'fixtures'"))
            .map(|r| r.0)
            .collect()
    };
    assert_eq!(by_string(true), ["both", "direct"]);
    assert_eq!(
        named(&f, "pkg/app/fixtures/a.json", true),
        ["both", "direct", "odd", "shell"]
    );
    assert_eq!(by_string(false), ["both", "chain", "direct", "output"]);
}

#[test]
fn the_nearest_task_runs_the_script() {
    let f = fixture();
    // A file of the package of the second task: that task consumes the output, it
    // does not run the script.
    assert_eq!(named(&f, "pkg/mid/data/x.json", true), [] as [&str; 0]);
}

#[test]
fn a_root_task_runs_at_the_root_for_the_root_package() {
    let f = fixture();
    // The root package's own files; not the files of the packages below the root.
    assert_eq!(named(&f, "config/x.json", true), ["root"]);
    assert_eq!(named(&f, "pkg/mid/data/x.json", true), [] as [&str; 0]);
}

#[test]
fn a_task_whose_directory_is_not_known_keeps_the_entry_s_names() {
    let f = fixture();
    // What the task of pkg/odd could read where it is defined is not claimed: the
    // answer is the one from before runners, the consumer's own directory.
    assert_eq!(named(&f, "pkg/odd/data/x.json", true), [] as [&str; 0]);
    assert_eq!(named(&f, "pkg/odd/data/x.json", false), [] as [&str; 0]);
    assert!(named(&f, "pkg/app/src/page.ts", true).contains(&"odd".to_owned()));
}

#[test]
fn a_shell_script_between_the_task_and_the_program_keeps_the_entry_s_names() {
    let f = fixture();
    // The task runs a shell script and the script runs the program: where the
    // script starts it is not followed, so nothing is claimed for the task's
    // directory, and the consumer keeps its own.
    assert_eq!(named(&f, "pkg/sh/data/x.json", true), [] as [&str; 0]);
    assert!(named(&f, "pkg/app/src/page.ts", true).contains(&"shell".to_owned()));
}
