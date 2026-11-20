//! `node --test`: which test files a run of the Node.js test runner executes,
//! for the runs relation ([`crate::runs`]).
//!
//! The adapter ([`turborepo_trace_frameworks::nodetest`]) lists the files of
//! the runs it reads in `package.json` scripts as tests. A task is known to run
//! exactly those when its command is such a run:
//!
//! 1. the command line is read by the adapter's own reader
//!    ([`turborepo_trace_frameworks::nodetest::parse`]) from the words this
//!    crate's shell parser produced (after wrappers and forwarding bins are
//!    removed), and says its files and preloads exactly;
//! 2. the manifests that govern the working directory hold an equal run
//!    ([`turborepo_trace_frameworks::nodetest::governing_runs`]): the files are
//!    tests in the graph, with this run's preloads and env files as inputs;
//! 3. no command of the task before it writes build output where the run looks
//!    for files (`tsc && node --test dist/*.test.js`, or the default patterns
//!    after a build into the working directory): compiled test files are no
//!    source files, and nothing maps them back.
//!
//! Then the task gets one run record per glob (`tests:node-test:<glob>`: the
//! tests of the framework that the glob names). Otherwise it gets a record of
//! a runner no framework of the graph has
//! ([`turborepo_trace_frameworks::nodetest::UNMAPPED`]), and runs whole. A run
//! whose explicit files are no tests of the graph (the output directory of
//! another task) is found when the selection is made.

use std::path::Path;

use turborepo_trace_frameworks::{
    Framework,
    nodetest::{self, NodeTest, Parsed, Run},
};

use crate::{
    registry::{RecognizeCx, strip_wrappers},
    spec::{Command, OutputKind, TaskSpec},
};

/// Why the run cannot be mapped because of what the task builds before it.
fn built_before(run: &Run, out: &TaskSpec) -> Option<String> {
    for o in &out.outputs {
        if o.kind != OutputKind::Artifact || !o.path.starts_with(&run.cwd) {
            continue;
        }
        let name = o.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        // Reports of a coverage tool hold no test files.
        if matches!(name, "coverage" | ".nyc_output") || o.path.extension().is_some() {
            continue;
        }
        let hit = run.defaults
            || run.patterns.iter().any(|p| {
                let (prefix, _) = crate::glob::split_static_prefix(p);
                let at = run.cwd.join(prefix);
                at.starts_with(&o.path) || o.path.starts_with(&at)
            });
        if hit {
            return Some(format!(
                "the task builds into {} before the run: compiled test files are not source files",
                o.path.strip_prefix(&run.cwd).unwrap_or(&o.path).display()
            ));
        }
    }
    None
}

/// Records what the `node --test` command `cmd` runs (module docs). `false`
/// when the command is no run of the test runner.
pub fn record(cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> bool {
    let real = strip_wrappers(cmd, cx, &mut TaskSpec::default());
    let Some(program) = real.program_name() else {
        return false;
    };
    let args: Vec<(&str, bool)> = real
        .arg_words()
        .iter()
        .map(|w| (w.text.as_str(), w.dynamic))
        .collect();
    let assigned = real.assigns.iter().rev().find(|(k, _)| k == "NODE_OPTIONS");
    let computed_options = assigned.is_some_and(|(_, v)| v.dynamic);
    let node_options = match assigned {
        Some((_, v)) => Some(v.text.as_str()),
        None => cx.env.get("NODE_OPTIONS").map(String::as_str),
    };
    let parsed = nodetest::parse(program, &args, node_options, &real.cwd);
    let run = match parsed {
        Parsed::NotATestRun => {
            // Another program of the node family with `--test` (`ts-node --test`).
            if nodetest::parse("node", &args, None, &real.cwd) == Parsed::NotATestRun {
                return false;
            }
            out.test_run(nodetest::UNMAPPED, real.cwd.clone());
            return true;
        }
        Parsed::Unmapped(_) => None,
        Parsed::Run(run) => Some(run),
    };
    let mapped = run.filter(|run| {
        !real.cwd_computed && !computed_options && built_before(run, out).is_none() && claimed(run)
    });
    match mapped {
        Some(run) => {
            for glob in run.launch_globs() {
                out.test_run(nodetest::ID, glob);
            }
        }
        None => out.test_run(nodetest::UNMAPPED, real.cwd.clone()),
    }
    true
}

/// Whether the adapter lists the files of `run` as tests, with its preloads:
/// the manifests that govern the working directory hold an equal run.
fn claimed(run: &Run) -> bool {
    let probe = run.cwd.join("__turbotrace_probe__");
    NodeTest.config_for(&probe).is_some_and(|config| {
        nodetest::governing_runs(config.parent().unwrap_or(Path::new("/")))
            .iter()
            .any(|(_, _, r)| r == run)
    })
}
