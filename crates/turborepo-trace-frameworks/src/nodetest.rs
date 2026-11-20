//! The Node.js test runner (`node --test`).
//!
//! The runner has no config file: the command line says which files run and
//! what is loaded before them. So the "config" of this adapter is a
//! `package.json` whose scripts start the runner, and a project is one run:
//!
//! | Input | Shape |
//! |---|---|
//! | the files the run names (paths and globs, relative to the working directory), or the runner's default patterns | discovery (`glob_base`: the working directory) |
//! | `--import`, `--require`, `--loader` (also in a `NODE_OPTIONS=` prefix), `tsx` itself | setup-file entries of every test: they run in each test process |
//! | `--test-global-setup` | a global-setup entry |
//! | `--env-file`, `--env-file-if-exists` | files read for every test (a probe when missing) |
//! | `<test file>.snapshot` (`t.assert.snapshot`) | a snapshot edge of the test, else a probe |
//!
//! The `package.json` itself is no input of the tests: it changes with every
//! dependency entry. It is read without a record ([`manifest_runs`]); the
//! graph store keys the packages a manifest governs by its runs, and an edit
//! of a script selects the tests of the script's task.
//!
//! # The Node versions modelled
//!
//! Node 22, 24 and 26 (the release lines that are LTS now). Verified with
//! v22.23, v24.22 and v26.13:
//!
//! - Without file arguments the runner takes
//!   `**/{test,test/**/*,test-*,*[._-]test}.{js,mjs,cjs}` below the working
//!   directory, and the same with `ts`, `mts` and `cts` when types are stripped
//!   (the default in these lines; `--no-experimental-strip-types` turns it
//!   off). `tsx --test` passes its own pattern
//!   (`**/{test,test/**/*,test-*,*[.-_]test}.?(c|m)@(t|j)s`). The adapter takes
//!   the union: a file that only the other variant runs is listed as a test and
//!   is never left out. Which Node version a package runs is not read.
//! - A file argument is a glob that Node expands itself (braces and extglobs
//!   included). A shell expands an unquoted one first, with `**` as `*`: the
//!   files it names are among those the glob names for Node, so quoted and
//!   unquoted arguments are read alike.
//! - `**` never enters `node_modules`.
//! - Every word after the first file argument is a file argument.
//! - `--test-name-pattern`, `--test-skip-pattern`, `--test-shard`,
//!   `--test-only` and `--watch` leave no file out for good: every file is a
//!   test of the run.
//! - The tests run in the working directory of the runner.
//!
//! Node 20 and older have no globs (an argument is a path, and a directory is
//! searched): they are not modelled.
//!
//! # Runs that are not mapped
//!
//! [`parse`] answers [`Parsed::Unmapped`], and the files of such a run are not
//! listed as tests, when
//!
//! - an argument that decides the files or the preloads is computed (`$FILES`);
//! - an option is not one of those listed in this module, or names a config
//!   file (`--experimental-default-config-file`, `--experimental-config-file`),
//!   export conditions (`-C`, `--conditions`) or another tsconfig (`tsx
//!   --tsconfig`): what loads may differ from what the graph has;
//! - a file argument is absolute, leaves the working directory (`..`), starts
//!   with `!`, is a glob this crate does not translate exactly, or names a
//!   directory (Node 22 and 24 resolve it as a module, Node 26 searches it).
//!
//! The task that starts such a run runs whole (`turborepo-trace-tasks`,
//! `recognizers/nodetest.rs`).
//!
//! # Scripts
//!
//! [`script_runs`] reads a script's text with a small shell reader: commands
//! are split at `&&`, `||`, `;`, `|` and newlines, `cd <literal>` moves the
//! working directory of what follows, and a command is a run when one of its
//! words is `node`, `nodejs` or `tsx` followed by `--test` before any file.
//! The words before it (a package manager's `exec`, `cross-env`, a wrapper)
//! are not interpreted, except that an option that can move the working
//! directory (`-C`, `--dir`, `--cwd`, `--prefix`, `--filter`, ...) leaves the
//! run out. Whether the task really starts the run is the task recognizer's
//! question; this adapter only says which files such a run would take.

use std::path::{Path, PathBuf};

use crate::{
    Framework, ModuleResolver, fsx,
    fsx::PathFsx,
    inputs::{
        EdgeKind, FrameworkInputs, GlobalInputs, ImplicitEdge, InputRef, InputRole, Project,
        SnapshotPolicy, Target, TestDiscovery, TestInputs, TestPattern,
    },
    pattern::glob_is_exact,
};

/// The Node.js test runner adapter.
#[derive(Copy, Clone, Debug, Default)]
pub struct NodeTest;

/// The adapter's id, and the framework of its test records and run records.
pub const ID: &str = "node-test";

/// The runner name of a run record for a run that is not mapped to files
/// (`turborepo-trace-tasks`, [`crate::nodetest`] module docs): no framework
/// of the graph has this name, so the task runs whole.
pub const UNMAPPED: &str = "node --test";

/// Node's default pattern, with the TypeScript extensions.
pub const NODE_DEFAULT: &str = "**/{test,test/**/*,test-*,*[._-]test}.{js,mjs,cjs,ts,mts,cts}";

/// The pattern `tsx --test` passes when the command names no file.
pub const TSX_DEFAULT: &str = "**/{test,test/**/*,test-*,*[.-_]test}.?(c|m)@(t|j)s";

const CONFIG_NAMES: &[&str] = &["package.json"];

/// Programs that are the runner.
const PROGRAMS: &[&str] = &["node", "nodejs", "tsx"];

/// Options without a value that change neither the files nor what loads.
const FLAGS: &[&str] = &[
    "--test",
    "--watch",
    "--watch-preserve-output",
    "--test-only",
    "--test-force-exit",
    "--test-update-snapshots",
    "--test-randomize",
    "--test-coverage-include-all",
    "--experimental-test-coverage",
    "--experimental-test-module-mocks",
    "--experimental-test-snapshots",
    "--experimental-strip-types",
    "--no-experimental-strip-types",
    "--experimental-transform-types",
    "--experimental-require-module",
    "--no-experimental-require-module",
    "--experimental-detect-module",
    "--no-experimental-detect-module",
    "--experimental-vm-modules",
    "--experimental-import-meta-resolve",
    "--experimental-sqlite",
    "--experimental-websocket",
    "--experimental-eventsource",
    "--enable-source-maps",
    "--no-warnings",
    "--no-deprecation",
    "--trace-warnings",
    "--trace-deprecation",
    "--trace-uncaught",
    "--throw-deprecation",
    "--pending-deprecation",
    "--expose-gc",
    "--no-cache",
];

/// Options with a value (`--x=v` or `--x v`) that change neither the files nor
/// what loads.
const VALUE_OPTIONS: &[&str] = &[
    "--test-reporter",
    "--test-reporter-destination",
    "--test-concurrency",
    "--test-shard",
    "--test-timeout",
    "--test-name-pattern",
    "--test-skip-pattern",
    "--test-coverage-branches",
    "--test-coverage-functions",
    "--test-coverage-lines",
    "--test-coverage-include",
    "--test-coverage-exclude",
    "--test-random-seed",
    "--test-rerun-failures",
    "--test-isolation",
    "--experimental-test-isolation",
    "--experimental-test-tag-filter",
    "--watch-path",
    "--watch-kill-signal",
    "--disable-warning",
    "--unhandled-rejections",
    "--title",
    "--stack-trace-limit",
    "--max-old-space-size",
    "--max-semi-space-size",
    "--stack-size",
];

/// Options that load a module before every test.
const PRELOADS: &[&str] = &[
    "--import",
    "--require",
    "-r",
    "--loader",
    "--experimental-loader",
];

/// Options before the runner's word that can move the working directory of
/// what a wrapper starts.
const MOVES: &[&str] = &[
    "-C",
    "--dir",
    "--cwd",
    "--prefix",
    "--filter",
    "-F",
    "--workspace",
    "-w",
    "--chdir",
    "--directory",
    "--project",
];

/// An env file a run reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvFile {
    /// As written, relative to the working directory.
    pub path: String,
    /// `--env-file-if-exists`.
    pub optional: bool,
}

/// One run of the test runner, as its command line says it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    /// The working directory (absolute).
    pub cwd: PathBuf,
    /// The globs of the files it runs, relative to the working directory (a
    /// file's path is a glob that names it).
    pub patterns: Vec<String>,
    /// Whether the patterns are the runner's defaults (the command names no
    /// file).
    pub defaults: bool,
    /// Module requests loaded before every test, as written.
    pub preloads: Vec<String>,
    /// `--test-global-setup` modules.
    pub global_setup: Vec<String>,
    /// Env files.
    pub env_files: Vec<EnvFile>,
}

impl Run {
    /// The globs of the run's files from the working directory on, absolute:
    /// what a run record of the task names.
    pub fn launch_globs(&self) -> Vec<PathBuf> {
        self.patterns.iter().map(|p| self.cwd.join(p)).collect()
    }
}

/// Whether `glob`, the glob of a run record, is a default pattern of the
/// runner: one that may match no file.
pub fn is_default_glob(glob: &str) -> bool {
    glob.ends_with(NODE_DEFAULT) || glob.ends_with(TSX_DEFAULT)
}

/// What a command line is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Parsed {
    /// Not a run of the test runner.
    NotATestRun,
    /// A run whose files or preloads the command line does not say exactly;
    /// why.
    Unmapped(String),
    /// A run.
    Run(Run),
}

fn unmapped(why: impl Into<String>) -> Parsed {
    Parsed::Unmapped(why.into())
}

/// Reads the command line of `program` (its base name) with the arguments
/// `args` (the text of each word and whether a part of it is computed) and the
/// value of a `NODE_OPTIONS` assignment in front of it, started in `cwd`. See
/// the module docs for what is a run and what is mapped.
pub fn parse(
    program: &str,
    args: &[(&str, bool)],
    node_options: Option<&str>,
    cwd: &Path,
) -> Parsed {
    if !PROGRAMS.contains(&program) {
        return Parsed::NotATestRun;
    }
    let tsx = program == "tsx";
    let mut run = Run {
        cwd: cwd.to_path_buf(),
        patterns: Vec::new(),
        defaults: false,
        preloads: Vec::new(),
        global_setup: Vec::new(),
        env_files: Vec::new(),
    };
    let mut test = false;
    let mut problem: Option<String> = None;
    // The options of `NODE_OPTIONS` come first.
    let from_env: Vec<(&str, bool)> = match node_options {
        None => Vec::new(),
        Some(v) if v.contains(['"', '\'', '$', '`', '\\']) => {
            problem = Some("NODE_OPTIONS is not a plain list of options".into());
            Vec::new()
        }
        Some(v) => v.split_whitespace().map(|w| (w, false)).collect(),
    };
    let mut positional: Vec<(&str, bool)> = Vec::new();
    for (words, env) in [(from_env.as_slice(), true), (args, false)] {
        let mut i = 0;
        // `tsx watch --test`.
        if !env && tsx && words.first().is_some_and(|w| w.0 == "watch") {
            i = 1;
        }
        while i < words.len() {
            let (w, dynamic) = words[i];
            i += 1;
            if !positional.is_empty() {
                // After the first file every word is a file.
                positional.push((w, dynamic));
                continue;
            }
            if w == "--" {
                positional.extend(words[i..].iter().copied());
                break;
            }
            if !w.starts_with('-') || w == "-" {
                if env {
                    problem.get_or_insert_with(|| "NODE_OPTIONS names a file".into());
                    continue;
                }
                positional.push((w, dynamic));
                continue;
            }
            let (name, inline) = match w.split_once('=') {
                Some((n, v)) => (n, Some(v)),
                None => (w, None),
            };
            if name.contains(['$', '`']) {
                problem.get_or_insert_with(|| format!("the option `{w}` is computed"));
                continue;
            }
            // The value of an option that takes one: inline, else the next word.
            let value = |i: &mut usize| -> Option<(&str, bool)> {
                match inline {
                    Some(v) => Some((v, dynamic)),
                    None => {
                        let v = words.get(*i).copied();
                        *i += 1;
                        v
                    }
                }
            };
            if FLAGS.contains(&name) {
                test |= name == "--test";
            } else if VALUE_OPTIONS.contains(&name) {
                if value(&mut i).is_none() {
                    problem.get_or_insert_with(|| format!("`{name}` has no value"));
                }
            } else if PRELOADS.contains(&name) || name == "--test-global-setup" {
                match value(&mut i) {
                    Some((v, false)) if !v.is_empty() => {
                        if name == "--test-global-setup" {
                            run.global_setup.push(v.to_owned());
                        } else {
                            run.preloads.push(v.to_owned());
                        }
                    }
                    _ => {
                        problem
                            .get_or_insert_with(|| format!("the module of `{name}` is computed"));
                    }
                }
            } else if matches!(name, "--env-file" | "--env-file-if-exists") {
                match value(&mut i) {
                    Some((v, false)) if !v.is_empty() => run.env_files.push(EnvFile {
                        path: v.to_owned(),
                        optional: name == "--env-file-if-exists",
                    }),
                    _ => {
                        problem.get_or_insert_with(|| format!("the file of `{name}` is computed"));
                    }
                }
            } else {
                problem.get_or_insert_with(|| {
                    format!("the option `{name}` is not one the trace reads")
                });
                // Whether it takes the next word is not known either.
            }
        }
    }
    // With an option that is not read, a `--test` anywhere may be the runner's.
    let named = args.iter().any(|(w, _)| *w == "--test");
    if !(test || problem.is_some() && named) {
        return Parsed::NotATestRun;
    }
    if let Some(why) = problem {
        return unmapped(why);
    }
    if tsx {
        // The runner's own loader: which tsx is installed is an input of every test.
        run.preloads.insert(0, "tsx".to_owned());
    }
    // A word after the first file that looks like an option is a pattern that names
    // no file.
    let files: Vec<(&str, bool)> = positional
        .iter()
        .enumerate()
        .filter(|(i, (w, _))| *i == 0 || !w.starts_with('-'))
        .map(|(_, w)| *w)
        .collect();
    if files.is_empty() {
        run.defaults = true;
        if tsx {
            run.patterns.push(TSX_DEFAULT.to_owned());
        }
        run.patterns.push(NODE_DEFAULT.to_owned());
        return Parsed::Run(run);
    }
    for (w, dynamic) in files {
        if dynamic {
            return unmapped(format!("the file argument `{w}` is computed"));
        }
        let p = w.strip_prefix("./").unwrap_or(w);
        if p.is_empty() {
            continue;
        }
        if p.starts_with(['/', '~', '!']) || p.contains('\\') || p.split('/').any(|s| s == "..") {
            return unmapped(format!(
                "the file argument `{w}` is not a path or glob below the working directory"
            ));
        }
        if !glob_is_exact(p) {
            return unmapped(format!("the glob `{w}` is not translated exactly"));
        }
        if p.ends_with('/') || (!crate::pattern::is_glob(p) && fsx::is_dir(&cwd.join(p))) {
            return unmapped(format!(
                "the argument `{w}` is a directory, which Node versions read differently"
            ));
        }
        if !run.patterns.iter().any(|x| x == p) {
            run.patterns.push(p.to_owned());
        }
    }
    Parsed::Run(run)
}

/// How a command is joined to the one before it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Joint {
    /// The first command, `&&`, `;` or a newline.
    Then,
    /// `||`.
    Else,
    /// `|` or `&`.
    Other,
}

/// One simple command of a script: its words (text, computed) and how it is
/// joined to the one before.
struct Simple {
    words: Vec<(String, bool)>,
    joint: Joint,
    /// A construct the reader does not model (a subshell, a substitution in
    /// backticks, a brace group).
    complex: bool,
}

/// The state of [`commands`].
struct Reader {
    out: Vec<Simple>,
    cur: Simple,
    word: String,
    has_word: bool,
    dynamic: bool,
    /// The next word is the target of a redirection.
    skip_next: bool,
}

impl Reader {
    fn end_word(&mut self) {
        if !self.has_word {
            return;
        }
        let word = std::mem::take(&mut self.word);
        if !self.skip_next {
            self.cur.words.push((word, self.dynamic));
        }
        self.skip_next = false;
        self.has_word = false;
        self.dynamic = false;
    }

    fn end_command(&mut self, joint: Joint) {
        self.end_word();
        self.skip_next = false;
        let next = Simple {
            words: Vec::new(),
            joint,
            complex: false,
        };
        self.out.push(std::mem::replace(&mut self.cur, next));
    }

    fn push(&mut self, c: char) {
        self.has_word = true;
        self.word.push(c);
    }
}

/// Splits a script's text into simple commands.
fn commands(text: &str) -> Vec<Simple> {
    let b: Vec<char> = text.chars().collect();
    let mut r = Reader {
        out: Vec::new(),
        cur: Simple {
            words: Vec::new(),
            joint: Joint::Then,
            complex: false,
        },
        word: String::new(),
        has_word: false,
        dynamic: false,
        skip_next: false,
    };
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        i += 1;
        match c {
            '\'' => {
                r.has_word = true;
                while i < b.len() && b[i] != '\'' {
                    r.word.push(b[i]);
                    i += 1;
                }
                i += 1;
            }
            '"' => {
                r.has_word = true;
                while i < b.len() && b[i] != '"' {
                    match b[i] {
                        '\\' if i + 1 < b.len() && matches!(b[i + 1], '"' | '\\' | '$' | '`') => {
                            r.word.push(b[i + 1]);
                            i += 1;
                        }
                        '$' | '`' => {
                            r.dynamic = true;
                            r.word.push(b[i]);
                        }
                        x => r.word.push(x),
                    }
                    i += 1;
                }
                i += 1;
            }
            '\\' => {
                if i < b.len() {
                    if b[i] != '\n' {
                        r.push(b[i]);
                    }
                    i += 1;
                }
            }
            ' ' | '\t' | '\r' => r.end_word(),
            '\n' | ';' => r.end_command(Joint::Then),
            '&' if b.get(i) == Some(&'&') => {
                i += 1;
                r.end_command(Joint::Then);
            }
            '|' if b.get(i) == Some(&'|') => {
                i += 1;
                r.end_command(Joint::Else);
            }
            '|' | '&' => r.end_command(Joint::Other),
            '<' | '>' => {
                // A redirection: the file descriptor before it and the target after it
                // are no words of the command.
                if r.has_word && r.word.chars().all(|x| x.is_ascii_digit()) {
                    r.word.clear();
                    r.has_word = false;
                }
                r.end_word();
                while i < b.len() && matches!(b[i], '>' | '<' | '&') {
                    i += 1;
                }
                r.skip_next = true;
            }
            '#' if !r.has_word => {
                while i < b.len() && b[i] != '\n' {
                    i += 1;
                }
            }
            '(' | ')' | '`' => {
                r.cur.complex = true;
                r.dynamic = true;
                r.push(c);
            }
            '$' => {
                r.dynamic = true;
                r.push(c);
            }
            x => r.push(x),
        }
    }
    r.end_command(Joint::Then);
    r.out.retain(|s| !s.words.is_empty());
    r.out
}

fn base_name(word: &str) -> &str {
    word.rsplit('/').next().unwrap_or(word)
}

/// The runs a script's text starts in `dir` (the directory of its
/// `package.json`), with exactly known files and preloads (module docs,
/// "Scripts").
pub fn script_runs(dir: &Path, text: &str) -> Vec<Run> {
    let mut out = Vec::new();
    // Where the commands run; `None` once a `cd` is not followed.
    let mut cwd = Some(dir.to_path_buf());
    let mut after_cd = false;
    for cmd in commands(text) {
        // After `cd x ||`, `cd x |` or `cd x &` the directory of what follows is not
        // known.
        if after_cd && cmd.joint != Joint::Then {
            cwd = None;
        }
        after_cd = cmd.words.first().is_some_and(|w| w.0 == "cd");
        if after_cd {
            cwd = match (&cwd, cmd.words.as_slice()) {
                (Some(at), [_, (to, false)])
                    if cmd.joint == Joint::Then
                        && !cmd.complex
                        && !to.starts_with(['/', '~', '-']) =>
                {
                    Some(fsx::normalize(&at.join(to)))
                }
                _ => None,
            };
            continue;
        }
        let Some(at) = &cwd else { continue };
        if cmd.complex {
            continue;
        }
        let Some(i) = cmd
            .words
            .iter()
            .position(|(w, dynamic)| !dynamic && PROGRAMS.contains(&base_name(w)))
        else {
            continue;
        };
        let before = &cmd.words[..i];
        let moves = |w: &str| {
            let name = w.split_once('=').map_or(w, |(n, _)| n);
            MOVES.contains(&name)
        };
        if before.iter().any(|(w, dynamic)| *dynamic || moves(w)) {
            continue;
        }
        let node_options = before
            .iter()
            .rev()
            .find_map(|(w, _)| w.strip_prefix("NODE_OPTIONS="));
        let args: Vec<(&str, bool)> = cmd.words[i + 1..]
            .iter()
            .map(|(w, d)| (w.as_str(), *d))
            .collect();
        if let Parsed::Run(run) = parse(base_name(&cmd.words[i].0), &args, node_options, at) {
            out.push(run);
        }
    }
    out
}

/// The runs of the scripts of the `package.json` at `manifest`, each with the
/// name of its script. Runs that are equal (a `test` and a `test:watch` script)
/// are listed once.
pub fn manifest_runs(manifest: &Path) -> Vec<(String, Run)> {
    // Not recorded as a read: a manifest changes with every dependency entry, and
    // only its runs matter here. Whoever keeps what is computed from them keys it
    // by the runs themselves (the graph store: the first-party key of the packages
    // the manifest governs).
    let Ok(bytes) = turborepo_trace_core::record::pause(|| fsx::read(manifest)) else {
        return Vec::new();
    };
    // (Most manifests start no test runner: the text says so without a parse.)
    if !bytes.windows(6).any(|w| w == b"--test") {
        return Vec::new();
    }
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Vec::new();
    };
    let Some(scripts) = json.get("scripts").and_then(|s| s.as_object()) else {
        return Vec::new();
    };
    let dir = manifest.parent().unwrap_or(Path::new("/"));
    let mut out: Vec<(String, Run)> = Vec::new();
    for (name, text) in scripts {
        let Some(text) = text.as_str() else { continue };
        for run in script_runs(dir, text) {
            if !out.iter().any(|(_, r)| *r == run) {
                out.push((name.clone(), run));
            }
        }
    }
    out
}

/// The nearest `package.json` in `dir` or above it whose scripts start a run.
fn manifest_above(dir: &Path) -> Option<PathBuf> {
    dir.ancestors()
        .map(|d| d.join("package.json"))
        .find(|m| m.fx_is_file() && !manifest_runs(m).is_empty())
}

/// The runs that can take a file below `dir` as a test: those of the nearest
/// `package.json` with a run, and of every such manifest above it (a root
/// script can name the files of a package below it). What [`NodeTest`] loads
/// for that manifest.
pub fn governing_runs(dir: &Path) -> Vec<(PathBuf, String, Run)> {
    let mut out = Vec::new();
    let mut at = manifest_above(dir);
    while let Some(manifest) = at {
        for (name, run) in manifest_runs(&manifest) {
            out.push((manifest.clone(), name, run));
        }
        at = manifest
            .parent()
            .and_then(Path::parent)
            .and_then(manifest_above);
    }
    out
}

/// A module request of a run as an input target: a path is relative to the
/// working directory.
fn module(request: &str, cwd: &Path) -> Target {
    Target::Module {
        request: request.to_owned(),
        from_dir: cwd.to_path_buf(),
    }
}

fn project(name: &str, run: &Run) -> Project {
    let mut global = GlobalInputs::default();
    for p in &run.preloads {
        global.entries.push(InputRef {
            role: InputRole::SetupFile,
            target: module(p, &run.cwd),
        });
    }
    for p in &run.global_setup {
        global.entries.push(InputRef {
            role: InputRole::GlobalSetup,
            target: module(p, &run.cwd),
        });
    }
    for e in &run.env_files {
        let p = fsx::normalize(&run.cwd.join(&e.path));
        if p.fx_is_file() {
            global.reads.push(p);
        } else {
            global.probes.push(p);
        }
    }
    Project {
        name: name.to_owned(),
        // The manifest is no config file of the tests: of all it holds, only the
        // script's command is theirs, and an edit of the script selects them through
        // the script's task.
        config_file: None,
        root_dir: run.cwd.clone(),
        global,
        discovery: TestDiscovery {
            roots: vec![run.cwd.clone()],
            include: run
                .patterns
                .iter()
                .map(|p| TestPattern::Glob(p.clone()))
                .collect(),
            glob_base: Some(run.cwd.clone()),
            ..TestDiscovery::default()
        },
        snapshots: SnapshotPolicy::Default,
        ..Project::default()
    }
}

impl Framework for NodeTest {
    fn id(&self) -> &'static str {
        ID
    }

    fn config_file_names(&self) -> &'static [&'static str] {
        CONFIG_NAMES
    }

    fn is_config_file(&self, path: &Path) -> bool {
        path.file_name().is_some_and(|n| n == "package.json") && !manifest_runs(path).is_empty()
    }

    fn discover(&self, root: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        fsx::walk_files_matching(root, "**/package.json", &mut |p| {
            if !manifest_runs(p).is_empty() {
                out.push(p.to_path_buf());
            }
        });
        out.sort();
        out
    }

    fn config_for(&self, test: &Path) -> Option<PathBuf> {
        manifest_above(test.parent()?)
    }

    fn load(&self, config: &Path) -> FrameworkInputs {
        let dir = config.parent().unwrap_or(Path::new("/"));
        let projects = governing_runs(dir)
            .iter()
            .map(|(_, name, run)| project(name, run))
            .collect();
        FrameworkInputs {
            framework: ID,
            config_file: None,
            projects,
            uncertainties: Vec::new(),
        }
    }

    fn test_inputs(
        &self,
        project: &Project,
        test: &Path,
        _src: &[u8],
        _resolver: &dyn ModuleResolver,
    ) -> TestInputs {
        let mut out = TestInputs {
            test: test.to_path_buf(),
            ..TestInputs::default()
        };
        // (Asked for a preload module too: it has no snapshot.)
        if !project.discovery.matcher().is_test(test) {
            return out;
        }
        let mut name = test.as_os_str().to_owned();
        name.push(".snapshot");
        let snapshot = PathBuf::from(name);
        if snapshot.fx_is_file() {
            out.edges.push(ImplicitEdge {
                kind: EdgeKind::Snapshot,
                to: snapshot,
                reason: "default snapshot path".into(),
            });
        } else {
            out.probes.push(snapshot);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_of(program: &str, args: &[&str]) -> Parsed {
        let args: Vec<(&str, bool)> = args.iter().map(|a| (*a, a.contains('$'))).collect();
        parse(program, &args, None, Path::new("/r/pkg"))
    }

    fn patterns(p: Parsed) -> Vec<String> {
        match p {
            Parsed::Run(r) => r.patterns,
            other => panic!("not a run: {other:?}"),
        }
    }

    #[test]
    fn files_and_defaults() {
        assert_eq!(patterns(run_of("node", &["--test"])), [NODE_DEFAULT]);
        assert_eq!(
            patterns(run_of("tsx", &["--test"])),
            [TSX_DEFAULT, NODE_DEFAULT]
        );
        assert_eq!(
            patterns(run_of(
                "node",
                &[
                    "--import",
                    "tsx",
                    "--test",
                    "./src/**/*.test.ts",
                    "test/a.test.js"
                ]
            )),
            ["src/**/*.test.ts", "test/a.test.js"]
        );
        // Options with a value, in both forms; a name pattern is no file.
        assert_eq!(
            patterns(run_of(
                "node",
                &[
                    "--experimental-strip-types",
                    "--test-reporter",
                    "spec",
                    "--test-name-pattern=add",
                    "--test-concurrency=$JOBS",
                    "--test-shard",
                    "1/3",
                    "--test",
                    "--watch",
                    "a.test.ts"
                ]
            )),
            ["a.test.ts"]
        );
        // Every word after the first file is a file.
        assert_eq!(
            patterns(run_of(
                "node",
                &["--test", "a.test.js", "--test-reporter", "dot"]
            )),
            ["a.test.js", "dot"]
        );
        // `--test` after the script is the script's argument.
        assert_eq!(run_of("node", &["build.js", "--test"]), Parsed::NotATestRun);
        assert_eq!(run_of("node", &["build.js"]), Parsed::NotATestRun);
        assert_eq!(run_of("vitest", &["--test"]), Parsed::NotATestRun);
    }

    #[test]
    fn preloads_and_env_files() {
        let Parsed::Run(r) = parse(
            "node",
            &[
                ("--import=./setup.mjs", false),
                ("-r", false),
                ("dotenv/config", false),
                ("--env-file=.env.test", false),
                ("--env-file-if-exists", false),
                (".env.local", false),
                ("--test-global-setup=./global.mjs", false),
                ("--test", false),
            ],
            Some("--loader ts-node/esm --no-warnings"),
            Path::new("/r/pkg"),
        ) else {
            panic!("not a run");
        };
        assert_eq!(r.preloads, ["ts-node/esm", "./setup.mjs", "dotenv/config"]);
        assert_eq!(r.global_setup, ["./global.mjs"]);
        assert_eq!(
            r.env_files,
            [
                EnvFile {
                    path: ".env.test".into(),
                    optional: false
                },
                EnvFile {
                    path: ".env.local".into(),
                    optional: true
                }
            ]
        );
        let Parsed::Run(r) = run_of("tsx", &["--import", "./setup.ts", "--test", "a.test.ts"])
        else {
            panic!("not a run");
        };
        assert_eq!(r.preloads, ["tsx", "./setup.ts"]);
    }

    #[test]
    fn what_is_not_mapped() {
        for args in [
            &["--test", "$FILES"][..],
            &["--test", "../other/*.test.js"],
            &["--test", "/abs/a.test.js"],
            &["--test", "!src/skip/**"],
            &["--test", "src/"],
            &["--test", "src/!(a).test.js"],
            &["--import", "$SETUP", "--test"],
            &["--conditions=source", "--test"],
            &["-C", "source", "--test"],
            &["--experimental-default-config-file", "--test"],
            &["--experimental-config-file=node.config.json", "--test"],
            &["--some-new-option", "--test"],
            &["--env-file=$ENV", "--test"],
        ] {
            assert!(
                matches!(run_of("node", args), Parsed::Unmapped(_)),
                "{args:?}"
            );
        }
        assert!(matches!(
            run_of("tsx", &["--tsconfig", "tsconfig.test.json", "--test"]),
            Parsed::Unmapped(_)
        ));
        assert!(matches!(
            parse(
                "node",
                &[("--test", false)],
                Some("--import \"./a b.mjs\""),
                Path::new("/r")
            ),
            Parsed::Unmapped(_)
        ));
    }

    #[test]
    fn scripts() {
        let dir = Path::new("/r/pkg");
        let runs = |text: &str| script_runs(dir, text);
        let one = |text: &str| {
            let mut r = runs(text);
            assert_eq!(r.len(), 1, "{text}");
            r.remove(0)
        };
        assert_eq!(one("node --test").patterns, [NODE_DEFAULT]);
        assert_eq!(
            one("node --test \"src/**/*.test.ts\"").patterns,
            ["src/**/*.test.ts"]
        );
        assert_eq!(
            one("tsc --noEmit && node --import tsx --test 'test/*.test.ts' 2>&1 | tee log.txt")
                .patterns,
            ["test/*.test.ts"]
        );
        // Words before the runner are not interpreted.
        let r = one(
            "cross-env NODE_OPTIONS='--import ./setup.mjs' tool-run node --test test/a.test.js",
        );
        assert_eq!(r.preloads, ["./setup.mjs"]);
        assert_eq!(r.patterns, ["test/a.test.js"]);
        assert_eq!(one("pnpm exec tsx --test").preloads, ["tsx"]);
        // `cd` moves what follows.
        assert_eq!(
            one("cd tests && node --test").cwd,
            Path::new("/r/pkg/tests")
        );
        assert_eq!(one("cd tests; cd .. ; node --test").cwd, dir);
        // Not followed, or not a run.
        for text in [
            "cd \"$DIR\" && node --test",
            "cd tests || node --test",
            "(cd tests && node --test)",
            "pnpm -C ../other exec node --test",
            "pnpm --filter=web exec node --test",
            "node --test $FILES",
            "echo 'node --test'",
            "node scripts/test.js --test",
            // Another runner started as a file: a script of node, no run of its runner.
            "node ./node_modules/.bin/jest --ci",
            "node ./node_modules/.bin/jest --test",
            "node --experimental-vm-modules node_modules/.bin/jest",
            "jest",
            "# node --test",
        ] {
            assert!(runs(text).is_empty(), "{text}");
        }
        // Two runs in one script.
        assert_eq!(
            runs("node --test test/unit/*.test.js && node --test test/e2e/*.test.js").len(),
            2
        );
    }

    #[test]
    fn default_patterns_match_what_node_runs() {
        use crate::pattern::CompiledPattern;
        let node = CompiledPattern::new(&TestPattern::Glob(NODE_DEFAULT.into()));
        let tsx = CompiledPattern::new(&TestPattern::Glob(TSX_DEFAULT.into()));
        // What Node 22, 24 and 26 ran in a tree with these files.
        for f in [
            "a.test.js",
            "b-test.js",
            "c_test.js",
            "test-d.js",
            "test.js",
            "f.test.mjs",
            "g.test.cjs",
            "h.test.ts",
            "i.test.mts",
            "test/x.js",
            "test/sub/y.js",
            "test/z.ts",
            "test/fixtures/data.js",
            "src/test/n.js",
            "src/deep/q.test.js",
            "src/deep/er/t.test.ts",
        ] {
            assert!(node.matches(f), "{f}");
        }
        for f in [
            "plain.js",
            "e.spec.js",
            "__tests__/u.js",
            "lib/tests/v.js",
            "a.test.jsx",
        ] {
            assert!(!node.matches(f), "{f}");
        }
        // What tsx 4.23 ran, and Node's pattern does not name.
        for f in ["b2test.js", "fooXtest.ts", "a.test.js", "test/z.ts"] {
            assert!(tsx.matches(f), "{f}");
        }
        assert!(!tsx.matches("b-test.js"));
        assert!(is_default_glob(
            "packages/math/**/{test,test/**/*,test-*,*[._-]test}.{js,mjs,cjs,ts,mts,cts}"
        ));
        assert!(!is_default_glob("packages/math/test/*.test.js"));
        assert!(glob_is_exact(NODE_DEFAULT) && glob_is_exact(TSX_DEFAULT));
    }
}
