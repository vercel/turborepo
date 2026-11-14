//! The "runs" relation: which tests a task executes ([`TestRun`],
//! [`TaskSpec::runs`]), and which files define how it runs them
//! ([`TaskSpec::definition`]).
//!
//! A test does not depend on the CI job (or package script) that runs it for
//! its contents, so nothing here is an input of anything. The relation answers
//! one question: when the *definition* of a job changes (runner image, tool
//! versions, environment, flags, the scripts its steps run), which tests can
//! change their outcome? Exactly the tests the job runs.
//!
//! | Command | Recorded |
//! |---|---|
//! | a test runner (`jest`, `vitest`, `pytest`, `go test`, `cargo test`, `playwright test`, `mocha`, `node --test`) | [`TestRun::Tests`]: the framework and the directory the run is rooted at (config roots, package arguments), by the runner's recognizer; its config file is a definition file |
//! | a task reference (`pnpm run x`, `pnpm -F web test`, `turbo run x`, `make x`, `npm-run-all`) | [`TestRun::Task`]: every task dependency the command adds. A computed package (`pnpm -F "$PKG" test`) means every package. A computed task name is [`TestRun::Computed`] ([`TestRun::Unknown`] when the command names `test`) |
//! | `turbo run x` | also the tasks `x` depends on in the root `turbo.json` (`dependsOn`, transitively): turbo runs them even when a package has no `x` script |
//! | a script run by an interpreter (`node x.mjs`, `tsx x.ts`, `python x.py`, a file with a `node` shebang, a repository `bin`) | the script is a definition file; its text is scanned for [launch words](launch_word), for a JS/TS script without its comments |
//! | an unknown program | the rest of its command line, if that is a recognized command (`timeout 600 pnpm test`); else its text is scanned |
//! | a partly recognized command that mapped nothing, `docker run`/`compose` | its text is scanned |
//! | shell scripts (`bash x.sh`, `./x.sh`) | definition files; their commands are recognized one by one |
//!
//! **Launch words.** Text the recognizers can't see through counts as starting
//! tests only on evidence: it names a test runner (`jest`, `vitest`, `pytest`,
//! `gotestsum`, `go test`, `cargo test`, `playwright`, `mocha`, `cypress`): the
//! tests of that framework; or it names a task runner (`turbo`, a package
//! manager, `make`, `nx`) on a line that also contains `test`, or `turbo` with
//! a pipeline task that depends on a test task: every test. Text that merely
//! names `turbo` or a package manager (a build, deploy or scoring script)
//! starts no tests. This is the one assumption of the relation: tests are
//! started by naming a test runner, or a task runner with a test task, in the
//! job's own text or in a script the job runs directly. A script that hides the
//! launch in a module it imports is not seen. A comment starts nothing: the
//! comments of a JS/TS script are not scanned (a provisioning script that says
//! which jest setup it serves runs no jest test).
//!
//! **Definition files** are what a command *is*, not what it reads: the script
//! file, not the application modules the script imports, and not the sources a
//! build or test command consumes. Those are inputs: they make the task an
//! affected task, not a changed way of running tests.
//!
//! The hook is [`observe`], called by [`crate::registry::Registry::recognize`]
//! for every command (also the commands of inlined shell scripts and wrapped
//! tools).

use std::{cell::Cell, path::Path};

use turborepo_trace_frameworks::fsx;

use crate::{
    registry::{CommandReport, Recognition, RecognizeCx},
    spec::{Command, EntryLang, TaskRef, TaskSpec, TestRun},
};

/// The lengths of a spec's lists before a command is recognized.
#[derive(Copy, Clone, Debug)]
pub struct Mark {
    entries: usize,
    deps: usize,
    runs: usize,
}

/// Marks `out` before a command is recognized (see [`observe`]).
pub fn mark(out: &TaskSpec) -> Mark {
    Mark {
        entries: out.entries.len(),
        deps: out.task_deps.len(),
        runs: out.runs.len(),
    }
}

/// Recognizers that run a script file with an interpreter and nothing else: the
/// entries they add are the program the task executes (not configs written in
/// code, and not what commands inside a shell script add; shell scripts record
/// themselves when inlined).
const SCRIPT_RUNNERS: &[&str] = &["node", "python", "deno", "go", "script-file", "bin-package"];

/// What a launch word stands for.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Launch {
    /// A test runner of a framework (`jest`).
    Framework(&'static str),
    /// A task runner with a test task: any test.
    Any,
}

/// Whether a value was computed at run time (an unexpanded variable or
/// substitution).
fn computed(s: &str) -> bool {
    s.contains('$') || s.contains('`')
}

/// Whether `text[i..i + len]` is a word: not part of a longer name, path or
/// package name.
fn is_word(text: &str, i: usize, len: usize) -> bool {
    let b = text.as_bytes();
    let name = |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.');
    let before = i == 0 || !(name(b[i - 1]) || matches!(b[i - 1], b'/' | b'@'));
    let after = i + len >= b.len() || !name(b[i + len]);
    before && after
}

fn has_word(text: &str, word: &str) -> bool {
    let mut from = 0;
    while let Some(i) = text[from..].find(word) {
        let at = from + i;
        if is_word(text, at, word.len()) {
            return true;
        }
        from = at + word.len();
    }
    false
}

/// The first launch word in `text` (see the module docs), with what it stands
/// for.
pub fn launch_word(text: &str) -> Option<(&'static str, Launch)> {
    const RUNNERS: &[(&str, &str)] = &[
        ("jest", "jest"),
        ("vitest", "vitest"),
        ("pytest", "pytest"),
        ("gotestsum", "gotest"),
        ("playwright", "playwright"),
        ("mocha", "mocha"),
        ("cypress", "cypress"),
    ];
    // A runner started through another program, on one line: `go test`,
    // `spawn('pnpm', ['run', 'test:unit'])`, `turbo run test`.
    const WITH_TEST: &[(&str, Option<&str>)] = &[
        ("go", Some("gotest")),
        ("cargo", Some("cargo-test")),
        ("turbo", None),
        ("pnpm", None),
        ("npm", None),
        ("yarn", None),
        ("bun", None),
        ("make", None),
        ("nx", None),
        ("deno", None),
    ];
    let mut any: Option<&'static str> = None;
    for line in text.lines() {
        for (w, fw) in RUNNERS {
            if has_word(line, w) {
                return Some((w, Launch::Framework(fw)));
            }
        }
        if line.contains("test") {
            for (w, fw) in WITH_TEST {
                if has_word(line, w) {
                    match fw {
                        Some(fw) => return Some((w, Launch::Framework(fw))),
                        None if any.is_none() => any = Some(w),
                        None => {}
                    }
                }
            }
        }
    }
    any.map(|w| (w, Launch::Any))
}

/// A turbo pipeline task named on a line with `turbo` that runs a test task
/// (its `dependsOn` closure in the root config has a task with `test` in its
/// name): `turbo ci:checks`.
fn turbo_test_task(text: &str, repo_root: &Path) -> Option<String> {
    for line in text.lines().filter(|l| has_word(l, "turbo")) {
        let words = line
            .split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '-' | '#')))
            .filter(|w| !w.is_empty() && *w != "turbo")
            .take(32);
        for w in words {
            let name = w.rsplit('#').next().unwrap_or(w);
            if turbo_pipeline(repo_root, name)
                .iter()
                .any(|(_, n, _)| n.contains("test"))
            {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// Scans `text` (shown as `what`) for a launch word and records what it found.
pub fn scan_text(text: &str, what: &str, repo_root: &Path, out: &mut TaskSpec) {
    match launch_word(text) {
        Some((_, Launch::Framework(fw))) => out.test_run(fw, repo_root),
        Some((w, Launch::Any)) => out.runs_unknown(format!("{what} names `{w}` with a test task")),
        None => {
            if let Some(task) = turbo_test_task(text, repo_root) {
                out.runs_unknown(format!("{what} names the turbo task `{task}`"));
            }
        }
    }
}

/// The text of the JS/TS script at `path` with everything that is not code
/// blanked: comments (and JSX text), byte for byte, so lines stay lines. `None`
/// when the lexer had to recover somewhere: then nothing says what is a
/// comment, and the whole text counts.
fn js_code(path: &Path, text: &str) -> Option<String> {
    use turborepo_trace_lang_js::lexer::{Lexer, TokKind};
    let src = text.as_bytes();
    let mut out: Vec<u8> = src
        .iter()
        .map(|&b| if matches!(b, b'\n' | b'\r') { b } else { b' ' })
        .collect();
    let mut lexer = Lexer::new(src, turborepo_trace_lang_js::Dialect::from_path(path));
    loop {
        let t = lexer.next_token();
        match t.kind {
            TokKind::Eof => break,
            // A `///` comment before any code.
            TokKind::TripleSlash => continue,
            _ => {}
        }
        let (start, end) = (t.start as usize, (t.end as usize).min(src.len()));
        if start < end {
            out[start..end].copy_from_slice(&src[start..end]);
        }
    }
    if lexer.jsx_failed() || lexer.take_error().is_some() {
        return None;
    }
    String::from_utf8(out).ok()
}

/// Scans the parts of a command line the shell parser does not model: command
/// substitutions (`$(…)`, backticks) and `eval`.
pub fn scan_unsupported_shell(line: &str, repo_root: &Path, out: &mut TaskSpec) {
    let b = line.as_bytes();
    let mut hidden = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'$' && b.get(i + 1) == Some(&b'(') && b.get(i + 2) != Some(&b'(') {
            let start = i + 2;
            let mut depth = 1;
            let mut j = start;
            while j < b.len() && depth > 0 {
                match b[j] {
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    _ => {}
                }
                j += 1;
            }
            hidden.push_str(&String::from_utf8_lossy(
                &b[start..j.saturating_sub(1).max(start)],
            ));
            hidden.push('\n');
            i = j;
        } else if b[i] == b'`' {
            let start = i + 1;
            let end = b[start..]
                .iter()
                .position(|c| *c == b'`')
                .map_or(b.len(), |p| start + p);
            hidden.push_str(&String::from_utf8_lossy(&b[start..end]));
            hidden.push('\n');
            i = end + 1;
        } else {
            i += 1;
        }
    }
    for l in line.lines() {
        let l = l.trim_start();
        if l.starts_with("eval ") || l.contains(" eval ") {
            hidden.push_str(l);
            hidden.push('\n');
        }
    }
    if !hidden.is_empty() {
        scan_text(&hidden, "a command substitution", repo_root, out);
    }
}

/// The task names turbo runs for task `name` beyond `name` itself: the
/// `dependsOn` closure in the root turbo config. `true` marks names reached
/// through another package (`^x`), which then run in other packages than the
/// selected ones; `pkg#x` entries name their package.
fn turbo_pipeline(repo_root: &Path, name: &str) -> Vec<(Option<String>, String, bool)> {
    // Not recorded: every task of a turbo repository is keyed by the root config
    // already.
    let Some((_, Some(doc))) =
        turborepo_trace_core::record::pause(|| crate::sources::turbo::read_turbo(repo_root))
    else {
        return Vec::new();
    };
    let Some(tasks) = doc
        .get("tasks")
        .or_else(|| doc.get("pipeline"))
        .and_then(|t| t.as_object())
    else {
        return Vec::new();
    };
    let mut out: Vec<(Option<String>, String, bool)> = Vec::new();
    let mut todo: Vec<(String, bool)> = vec![(name.to_string(), false)];
    let mut seen: Vec<(String, bool)> = Vec::new();
    while let Some((n, up)) = todo.pop() {
        if seen.contains(&(n.clone(), up)) || seen.len() > 256 {
            continue;
        }
        seen.push((n.clone(), up));
        // The definition for every package, and any package-specific one (`web#build`).
        for (key, def) in tasks {
            let applies = key == &n || key.rsplit_once('#').is_some_and(|(_, k)| k == n);
            if !applies {
                continue;
            }
            let Some(deps) = def.get("dependsOn").and_then(|d| d.as_array()) else {
                continue;
            };
            for d in deps.iter().filter_map(|d| d.as_str()) {
                if d.starts_with('$') {
                    continue;
                }
                if let Some(x) = d.strip_prefix('^') {
                    out.push((None, x.to_string(), true));
                    todo.push((x.to_string(), true));
                } else if let Some((p, x)) = d.split_once('#') {
                    let p = if p == "//" { "." } else { p };
                    out.push((Some(p.to_string()), x.to_string(), up));
                    todo.push((x.to_string(), true));
                } else {
                    out.push((None, d.to_string(), up));
                    todo.push((d.to_string(), up));
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

thread_local! {
    /// Set while the rest of an unknown program's command line is being recognized.
    static IN_WRAPPED: Cell<bool> = const { Cell::new(false) };
}

/// An unknown program that wraps a recognized command (`timeout 600
/// pnpm test`, `retry 3 jest`): what the wrapped command runs. `false` when no
/// rest of the command line maps to a run.
fn wrapped_runs(cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> bool {
    if IN_WRAPPED.get() {
        return false;
    }
    let reg = crate::registry::registry();
    for i in 1..cmd.argv.len().min(12) {
        let w = &cmd.argv[i];
        if w.dynamic || w.text.starts_with('-') {
            continue;
        }
        let sub = cmd.shifted(i);
        let known = reg.find(&sub).is_some()
            || sub
                .program_name()
                .is_some_and(|p| crate::registry::WRAPPERS.contains(&p));
        if !known {
            continue;
        }
        let mut scratch = TaskSpec::default();
        IN_WRAPPED.set(true);
        let _ = reg.recognize(&sub, cx, &mut scratch);
        IN_WRAPPED.set(false);
        if !scratch.runs.is_empty() {
            out.runs.extend(scratch.runs);
            out.definition.extend(scratch.definition);
            return true;
        }
    }
    false
}

/// Records what the command `cmd` (recognized as `rep`, with `out` marked by
/// [`mark`] before) runs and which files define it: see the module docs.
pub fn observe(
    cmd: &Command,
    rep: &CommandReport,
    cx: &RecognizeCx<'_>,
    m: Mark,
    out: &mut TaskSpec,
) {
    let recognizer = rep.recognizer.unwrap_or("");
    // 1. Task references.
    let deps: Vec<TaskRef> = out.task_deps[m.deps.min(out.task_deps.len())..]
        .iter()
        .map(|d| d.target.clone())
        .collect();
    for target in deps {
        let (package, name) = match &target {
            TaskRef::Local { name } | TaskRef::Upstream { name } | TaskRef::All { name } => {
                (None, name.clone())
            }
            TaskRef::Package { package, name } => (Some(package.clone()), name.clone()),
            TaskRef::Id { .. } => {
                out.runs_task(target);
                continue;
            }
        };
        // turbo passes the words after `--` on to the tasks (`turbo build -- --mode
        // "$M"`): they are arguments, not task names.
        if recognizer == "turbo" {
            let split = cmd.argv.iter().position(|w| w.text == "--");
            let passed_on = split.is_some_and(|i| {
                cmd.argv[i + 1..].iter().any(|w| w.text == name)
                    && !cmd.argv[..i].iter().any(|w| w.text == name)
            });
            if passed_on {
                continue;
            }
        }
        if computed(&name) {
            // Which task runs is decided at run time: a test task if the command says so,
            // else only for a task that is otherwise known to run tests.
            let reason = format!("`{}` runs a computed task", rep.text.trim());
            if rep.text.contains("test") {
                out.runs_unknown(reason);
            } else {
                out.runs_computed(reason);
            }
            continue;
        }
        let all = package.as_deref().is_some_and(computed);
        if all {
            out.runs_task(TaskRef::All { name: name.clone() });
        } else {
            out.runs_task(target.clone());
        }
        if recognizer == "turbo" {
            for (pkg, n, upstream) in turbo_pipeline(cx.repo_root, &name) {
                let t = match (pkg, &target) {
                    (Some(package), _) => TaskRef::Package { package, name: n },
                    (None, TaskRef::Package { package, .. }) if !upstream && !all => {
                        TaskRef::Package {
                            package: package.clone(),
                            name: n,
                        }
                    }
                    (None, TaskRef::Local { .. }) if !upstream => TaskRef::Local { name: n },
                    _ => TaskRef::All { name: n },
                };
                out.runs_task(t);
            }
        }
    }
    // 2. `node --test`: the built-in runner (its files are tests, not tooling).
    if recognizer == "node" && cmd.argv.iter().any(|w| w.text == "--test") {
        out.test_run("node-test", cmd.cwd.clone());
        return;
    }
    // 3. Scripts run by an interpreter: the script defines the task; its text may
    //    start tests.
    if SCRIPT_RUNNERS.contains(&recognizer) {
        let entries: Vec<_> = out.entries[m.entries.min(out.entries.len())..]
            .iter()
            .map(|e| (e.path.clone(), e.lang))
            .collect();
        for (p, lang) in entries {
            // An installed package, by the path below the root (the root may be below a
            // directory called `node_modules`).
            if p.strip_prefix(cx.repo_root).map_or(true, |rel| {
                rel.components().any(|c| c.as_os_str() == "node_modules")
            }) {
                continue;
            }
            out.defines(p.clone());
            if lang == EntryLang::Auto && recognizer != "script-file" {
                continue;
            }
            // Recorded: the script's text decides what the task runs.
            if let Ok(text) = fsx::read_to_string(&p) {
                // A comment starts nothing.
                let code = (lang == EntryLang::Js || EntryLang::of(&p) == EntryLang::Js)
                    .then(|| js_code(&p, &text))
                    .flatten();
                let text = code.unwrap_or(text);
                let shown = p
                    .strip_prefix(cx.repo_root)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .replace('\\', "/");
                scan_text(&text, &format!("script {shown}"), cx.repo_root, out);
            }
        }
    }
    // 4. Commands that mapped nothing and that the recognizers can't see
    // through.
    if out.runs.len() != m.runs {
        return;
    }
    if rep.recognition == Recognition::Unknown && wrapped_runs(cmd, cx, out) {
        return;
    }
    let opaque = matches!(rep.recognition, Recognition::Unknown | Recognition::Partial)
        || recognizer == "docker";
    if opaque {
        scan_text(
            &rep.text,
            &format!("`{}`", rep.text.trim()),
            cx.repo_root,
            out,
        );
    }
}

/// Whether `run` stands for every test.
pub fn is_unknown(run: &TestRun) -> bool {
    matches!(run, TestRun::Unknown { .. })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_words() {
        assert_eq!(
            launch_word("spawnSync('jest', ['--ci'])"),
            Some(("jest", Launch::Framework("jest")))
        );
        assert_eq!(launch_word("import cfg from './jest.config.js'"), None);
        assert_eq!(launch_word("require('ts-jest')"), None);
        assert_eq!(launch_word("import { expect } from '@jest/globals'"), None);
        assert_eq!(
            launch_word("x\ngo test ./...\n"),
            Some(("go", Launch::Framework("gotest")))
        );
        assert_eq!(launch_word("go build ./..."), None);
        assert_eq!(launch_word("pnpm install --frozen-lockfile"), None);
        assert_eq!(
            launch_word("execa('pnpm', ['run', 'test:unit'])"),
            Some(("pnpm", Launch::Any))
        );
        // A task runner alone is no evidence of a test launch: a build or scoring
        // script.
        assert_eq!(
            launch_word("execSync('turbo run build --filter=web')"),
            None
        );
        assert_eq!(launch_word("timeout 60 turbo ci:checks"), None);
        assert_eq!(
            launch_word("turbo run test --filter=web"),
            Some(("turbo", Launch::Any))
        );
        assert_eq!(launch_word("aws s3 cp a b && kubectl apply -f x"), None);
        assert_eq!(launch_word("echo latest > out.txt"), None);
    }

    #[test]
    fn comments_of_a_script_are_not_code() {
        let ts = Path::new("/r/services/db.ts");
        let src = "// Mirrors the shared jest beforeAll table setup.\n\
                   /* pnpm run test */ const url = 'http://127.0.0.1'; // vitest\n\
                   start(`${url}/x`);\n";
        let code = js_code(ts, src).expect("lexes");
        assert_eq!(code.len(), src.len());
        assert_eq!(code.lines().count(), src.lines().count());
        assert!(code.contains("const url = 'http://127.0.0.1';"), "{code}");
        assert!(code.contains("start(`${url}/x`);"), "{code}");
        assert_eq!(launch_word(&code), None);
        assert!(launch_word(src).is_some());
        // A launch in code stays one, whatever the comments say.
        let src = "// no tests here\nspawnSync('jest', ['--ci']); // run them\n";
        let code = js_code(ts, src).expect("lexes");
        assert_eq!(
            launch_word(&code),
            Some(("jest", Launch::Framework("jest")))
        );
        // A string or a template is code.
        let src = "const cmd = `pnpm run test:unit`; /* x */ const s = \"a // jest\";\n";
        let code = js_code(ts, src).expect("lexes");
        assert_eq!(
            launch_word(&code),
            Some(("jest", Launch::Framework("jest")))
        );
        // Input the lexer has to recover from: nothing says what a comment is.
        assert_eq!(js_code(ts, "const s = 'unterminated\n// jest\n"), None);
        assert_eq!(js_code(ts, "/* never closed: jest"), None);
    }

    #[test]
    fn substitutions() {
        let mut out = TaskSpec::default();
        scan_unsupported_shell(
            "X=$(pnpm --silent test:list)\necho $X",
            Path::new("/r"),
            &mut out,
        );
        assert_eq!(out.runs.len(), 1);
        assert!(is_unknown(&out.runs[0]));
        let mut out = TaskSpec::default();
        scan_unsupported_shell("X=$(date +%s)\necho $((1 + 2))", Path::new("/r"), &mut out);
        assert!(out.runs.is_empty());
    }
}
