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
//! 2. the task is a script of its package's manifest, and the adapter reads the
//!    same run in that script
//!    ([`turborepo_trace_frameworks::nodetest::manifest_scripts`]): the files
//!    are tests in the graph, with this run's preloads and env files as inputs,
//!    and nothing around the command (the environment, a wrapper's env file, an
//!    `.npmrc`) loads what is not read;
//! 3. no file the run names is a symbolic link (Node runs it by the link's
//!    path; the graph has the file it leads to);
//! 4. no command of the task before it writes build output where the run looks
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
    fsx,
    nodetest::{self, Parsed, Run},
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
        !real.cwd_computed
            && !computed_options
            && built_before(run, out).is_none()
            && claimed(run, cx)
            && !names_a_link(run)
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

/// Whether the adapter lists the files of `run` as tests, with what the run
/// loads as their inputs: the script of this task, in the manifest of its
/// package, holds the same run as the adapter reads it. (The adapter reads
/// the script's text for what the words of one command do not show: the
/// environment the commands before it leave, the env files of a wrapper, an
/// `.npmrc`. Another script's equal command line says nothing about this
/// one.)
fn claimed(run: &Run, cx: &RecognizeCx<'_>) -> bool {
    let Some(script) = cx.task_name else {
        return false;
    };
    nodetest::manifest_scripts(&cx.package_root.join("package.json"))
        .iter()
        .any(|(name, runs)| name == script && runs.iter().any(|r| r.same_command(run)))
}

/// Whether a file the run names is a symbolic link: Node runs it by the
/// link's path, and the graph has the file it leads to, which the run may not
/// name.
fn names_a_link(run: &Run) -> bool {
    use turborepo_trace_frameworks::{TestPattern, pattern::CompiledPattern};
    let is_link = |p: &Path| std::fs::symlink_metadata(p).is_ok_and(|m| m.is_symlink());
    let mut link = false;
    for pattern in &run.patterns {
        let (prefix, rest) = crate::glob::split_static_prefix(pattern);
        let base = run.cwd.join(prefix);
        if rest.is_empty() {
            link |= is_link(&base);
            continue;
        }
        let glob = CompiledPattern::new(&TestPattern::Glob(rest.to_owned()));
        fsx::walk_files_matching(&base, "**", &mut |p| {
            if let Ok(rel) = p.strip_prefix(&base)
                && glob.matches(&rel.to_string_lossy())
            {
                link |= is_link(p);
            }
        });
    }
    link
}
