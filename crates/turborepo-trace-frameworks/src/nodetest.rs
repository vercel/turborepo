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
//! - `**` never enters `node_modules`. It skips directories whose name starts
//!   with a dot too; the adapter lists the files below them (a file Node does
//!   not run is listed as a test: never one left out).
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
//! Neither is a run that loads what its command line does not say:
//!
//! - `NODE_PATH`, `TSX_TSCONFIG_PATH`, `TS_NODE_PROJECT` (and the like)
//!   assigned in front of the command, also through `cross-env` or `env`;
//! - a command before it in the script that changes the environment (`export`,
//!   a sourced file, a bare assignment);
//! - an env file (of `--env-file`, or of `dotenv`, `dotenvx`, `env-cmd` in
//!   front of the runner, whose files are inputs of the tests like the file of
//!   `--env-file`) that sets `NODE_OPTIONS` or one of those variables;
//! - an `.npmrc` in the package's directory or above it that sets
//!   `node-options`;
//! - `--test-isolation` other than `process` (the files share one process);
//! - a `--test-reporter` that is not built in (a module);
//! - a preload file, or a file it imports by a relative path, that calls
//!   `snapshot.setResolveSnapshotPath`;
//! - an env file, a preload file or an `.npmrc` that is no regular file or
//!   holds more than 4 MiB: it is not read;
//! - preload files that name more files by relative paths than are followed
//!   (2048 paths, 16 MiB);
//! - the same command line started a second time in the script by a run that is
//!   not mapped (after an `export`, behind such a variable or env file, in a
//!   group, a branch, a subshell or a directory that is not followed): the task
//!   recognizer sees one command at a time and cannot tell the two apart.
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

/// The longest file argument that is read as a glob.
const MAX_FILE_ARGUMENT: usize = 1024;

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
    /// Whether `other` is the same run as its command line says it: the
    /// working directory, the files and what is loaded first. (The env files
    /// of a wrapper in front of the runner are known to the reader of the
    /// script only.)
    pub fn same_command(&self, other: &Run) -> bool {
        self.cwd == other.cwd
            && self.patterns == other.patterns
            && self.preloads == other.preloads
            && self.global_setup == other.global_setup
    }

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
            } else if matches!(name, "--test-isolation" | "--experimental-test-isolation") {
                // In one process the files share state: what one loads, another sees.
                if !matches!(value(&mut i), Some(("process", false))) {
                    problem.get_or_insert_with(|| {
                        format!("`{name}`: the test files do not run in processes of their own")
                    });
                }
            } else if name == "--test-reporter" {
                // A reporter that is not built in is a module the run loads.
                if !matches!(
                    value(&mut i),
                    Some(("spec" | "tap" | "dot" | "junit" | "lcov", false))
                ) {
                    problem
                        .get_or_insert_with(|| "the reporter is a module, or computed".to_owned());
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
        if p.len() > MAX_FILE_ARGUMENT {
            return unmapped("a file argument is too long to read as a glob");
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

/// Words in front of a command that open a group or a branch of the shell.
const OPENERS: &[&str] = &[
    "{", "!", "if", "then", "else", "elif", "while", "until", "do",
];

fn base_name(word: &str) -> &str {
    word.rsplit('/').next().unwrap_or(word)
}

/// The runs a script's text starts in `dir` (the directory of its
/// `package.json`), with exactly known files and preloads (module docs,
/// "Scripts").
pub fn script_runs(dir: &Path, text: &str) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    // The runs the script starts that are not mapped, as far as their command
    // lines are read, and whether their working directory is known. The task
    // recognizer sees one command at a time: it cannot tell such a run from a
    // mapped one with the same command line, so neither is mapped.
    let mut refused: Vec<(Run, bool)> = Vec::new();
    // Where the commands run; `None` once a `cd` is not followed.
    let mut cwd = Some(dir.to_path_buf());
    let mut after_cd = false;
    // A command before changed the environment of what follows (`export`, a file
    // that is sourced, a bare assignment): what the runner then loads is not
    // read, so no run after it is mapped.
    let mut environment_changed = false;
    for cmd in commands(text) {
        // The command behind the words that open a group or a branch (`{ export
        // X=1`, `then . ./env.sh`).
        let opened = cmd
            .words
            .iter()
            .take_while(|(w, _)| OPENERS.contains(&w.as_str()))
            .count();
        let inner = &cmd.words[opened..];
        if inner.first().is_some_and(|(w, _)| {
            matches!(
                w.as_str(),
                "export" | "." | "source" | "eval" | "set" | "unset"
            )
        }) || (!inner.is_empty() && inner.iter().all(|(w, _)| assignment(w).is_some()))
        {
            environment_changed = true;
            continue;
        }
        if cmd.complex {
            // A subshell or a function: what it leaves in the environment, and where
            // its commands run, is not followed.
            if cmd.words.iter().any(|(w, _)| w.starts_with('(')) {
                environment_changed = true;
            }
            // A run inside it is not read word by word: no run of the script is
            // told from it.
            let plain = |w: &str| w.trim_start_matches('(').trim_end_matches(')').to_owned();
            let words: Vec<String> = cmd.words.iter().map(|(w, _)| plain(w)).collect();
            if words.iter().any(|w| PROGRAMS.contains(&base_name(w)))
                && words.iter().any(|w| w == "--test")
            {
                return Vec::new();
            }
        }
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
        let moved = before.iter().any(|(w, dynamic)| *dynamic || moves(w));
        // A variable that changes what the runner resolves or loads, and that is
        // not read here.
        let resolution = before
            .iter()
            .any(|(w, _)| assignment(w).is_some_and(|name| RESOLUTION_VARIABLES.contains(&name)));
        // The env files of a wrapper in front of the runner.
        let wrapper_env = wrapper_env_files(before);
        let node_options = before
            .iter()
            .rev()
            .find_map(|(w, _)| w.strip_prefix("NODE_OPTIONS="));
        let args: Vec<(&str, bool)> = cmd.words[i + 1..]
            .iter()
            .map(|(w, d)| (w.as_str(), *d))
            .collect();
        let at = cwd.as_deref().unwrap_or(dir);
        let Parsed::Run(mut run) = parse(base_name(&cmd.words[i].0), &args, node_options, at)
        else {
            continue;
        };
        let known_cwd = cwd.is_some() && !moved;
        match wrapper_env {
            Some(files) if known_cwd && !cmd.complex && !resolution && !environment_changed => {
                run.env_files.extend(files);
                if reads_only_what_is_modelled(&run) {
                    out.push(run);
                } else {
                    refused.push((run, true));
                }
            }
            _ => refused.push((run, known_cwd)),
        }
    }
    out.retain(|run| {
        !refused.iter().any(|(other, known_cwd)| {
            if *known_cwd {
                other.same_command(run)
            } else {
                other.patterns == run.patterns
                    && other.preloads == run.preloads
                    && other.global_setup == run.global_setup
            }
        })
    });
    out
}

/// The name of the variable a word `NAME=value` assigns.
fn assignment(word: &str) -> Option<&str> {
    let (name, _) = word.split_once('=')?;
    let ok = !name.is_empty()
        && !name.as_bytes()[0].is_ascii_digit()
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    ok.then_some(name)
}

/// Variables that change which modules the runner resolves or loads, without
/// an option on the command line. A run with one of them assigned (in front of
/// the command, through `cross-env` or `env`, or in an env file) is not
/// mapped. (`NODE_OPTIONS` in front of the command is read; in an env file it
/// is not.)
const RESOLUTION_VARIABLES: &[&str] = &[
    "NODE_PATH",
    "TSX_TSCONFIG_PATH",
    "ESBK_TSCONFIG_PATH",
    "TS_NODE_PROJECT",
    "TS_NODE_COMPILER_OPTIONS",
    "NODE_PRESERVE_SYMLINKS",
];

/// The env files that a wrapper among the words in front of the runner loads
/// (`dotenv -e .env.test --`, `dotenvx run -f x --`, `env-cmd -f x`): inputs of
/// the run's tests, like the file of `--env-file`. `None` when the wrapper's
/// files are not known (an option that is not read, `env-cmd` without a
/// file).
fn wrapper_env_files(before: &[(String, bool)]) -> Option<Vec<EnvFile>> {
    let Some(at) = before.iter().position(|(w, _)| {
        matches!(
            base_name(w),
            "dotenv" | "dotenv-cli" | "dotenvx" | "env-cmd"
        )
    }) else {
        return Some(Vec::new());
    };
    let wrapper = base_name(&before[at].0);
    let mut files = Vec::new();
    let mut words = before[at + 1..].iter().map(|(w, _)| w.as_str());
    while let Some(w) = words.next() {
        match w {
            "--" => break,
            "run" if wrapper == "dotenvx" => {}
            "-e" | "-f" | "--file" | "--env-file" => files.push(words.next()?.to_owned()),
            w if !w.starts_with('-') => break,
            w => match w.split_once('=') {
                Some(("--file" | "--env-file" | "-e" | "-f", f)) => files.push(f.to_owned()),
                _ => return None,
            },
        }
    }
    if files.is_empty() {
        if wrapper == "env-cmd" {
            return None;
        }
        files.push(".env".to_owned());
    }
    Some(
        files
            .into_iter()
            .map(|path| EnvFile {
                path,
                optional: true,
            })
            .collect(),
    )
}

/// The most a file read for the runs of a manifest may hold: the manifest, an
/// env file, an `.npmrc`, a preload file.
const READ_LIMIT: u64 = 4 << 20;

/// The most paths the walk from a run's preload files takes (a file a preload
/// names by a relative path, with each extension it may have), and the most
/// bytes it reads.
const MAX_PRELOAD_PATHS: usize = 2048;
const MAX_PRELOAD_BYTES: u64 = 16 << 20;
/// What a relative path of a preload may lack: the file is looked for with
/// each.
const PRELOAD_EXTENSIONS: &[&str] = &["", ".js", ".mjs", ".cjs", ".ts", ".mts", ".cts"];

/// A file read without a record ([`read_unrecorded`]).
enum Unrecorded {
    /// No file.
    Absent,
    /// The file.
    Bytes(Vec<u8>),
    /// Something that is not read: not a regular file (a device, a pipe, a
    /// directory, also behind a link), or more than [`READ_LIMIT`] bytes.
    Refused,
}

impl Unrecorded {
    fn text(&self) -> Option<std::borrow::Cow<'_, str>> {
        match self {
            Unrecorded::Bytes(b) => Some(String::from_utf8_lossy(b)),
            _ => None,
        }
    }
}

/// Reads a file without a record: what is computed from it is keyed by the
/// runs ([`manifest_scripts`]). Only a regular file of at most [`READ_LIMIT`]
/// bytes is read: a script names these paths, and a device or a pipe has no
/// end.
fn read_unrecorded(path: &Path) -> Unrecorded {
    turborepo_trace_core::record::pause(|| {
        if let Ok(meta) = std::fs::metadata(path)
            && (!meta.is_file() || meta.len() > READ_LIMIT)
        {
            return Unrecorded::Refused;
        }
        match fsx::read(path) {
            Ok(bytes) if bytes.len() as u64 > READ_LIMIT => Unrecorded::Refused,
            Ok(bytes) => Unrecorded::Bytes(bytes),
            Err(_) => Unrecorded::Absent,
        }
    })
}

/// Whether the run loads nothing that its command line does not say: no env
/// file of it sets `NODE_OPTIONS` or a [`RESOLUTION_VARIABLES`] entry, and no
/// preload file (or a file it imports by a relative path) moves the snapshot
/// files (`snapshot.setResolveSnapshotPath`). A file that is not read
/// ([`Unrecorded::Refused`]) may do either.
fn reads_only_what_is_modelled(run: &Run) -> bool {
    for e in &run.env_files {
        let file = read_unrecorded(&run.cwd.join(&e.path));
        let text = match &file {
            Unrecorded::Absent => continue,
            Unrecorded::Refused => return false,
            Unrecorded::Bytes(b) => String::from_utf8_lossy(b),
        };
        let sets = text.lines().any(|l| {
            let l = l.trim_start();
            let l = l.strip_prefix("export ").unwrap_or(l).trim_start();
            l.split_once(['=', ':'])
                .map(|(n, _)| n.trim())
                .is_some_and(|n| n == "NODE_OPTIONS" || RESOLUTION_VARIABLES.contains(&n))
        });
        if sets {
            return false;
        }
    }
    let mut todo: Vec<PathBuf> = run
        .preloads
        .iter()
        .chain(&run.global_setup)
        .filter(|p| p.starts_with('.'))
        .flat_map(|p| {
            // (`--require ./setup` loads `setup.js`.)
            let to = fsx::normalize(&run.cwd.join(p));
            PRELOAD_EXTENSIONS.iter().map(move |ext| {
                let mut name = to.clone().into_os_string();
                name.push(ext);
                PathBuf::from(name)
            })
        })
        .collect();
    // Every path is taken once, and the walk has a budget: the files a preload
    // names, and the bytes read for them. Beyond it the run is not mapped.
    let mut queued: std::collections::HashSet<PathBuf> = todo.iter().cloned().collect();
    let mut bytes_read = 0u64;
    while let Some(file) = todo.pop() {
        let read = read_unrecorded(&file);
        let text = match &read {
            Unrecorded::Absent => continue,
            Unrecorded::Refused => return false,
            Unrecorded::Bytes(b) => {
                bytes_read += b.len() as u64;
                if bytes_read > MAX_PRELOAD_BYTES {
                    return false;
                }
                String::from_utf8_lossy(b)
            }
        };
        if text.contains("setResolveSnapshotPath") {
            return false;
        }
        // The files it names by a relative path.
        let dir = file.parent().unwrap_or(Path::new("/"));
        for quote in ['\'', '"'] {
            for part in text.split(quote).skip(1).step_by(2) {
                // (A path of a file is short: a longer string is no path.)
                if !(part.starts_with("./") || part.starts_with("../")) || part.len() > 4096 {
                    continue;
                }
                let to = fsx::normalize(&dir.join(part));
                if queued.contains(&to) {
                    continue;
                }
                for ext in PRELOAD_EXTENSIONS {
                    let mut name = to.clone().into_os_string();
                    name.push(ext);
                    let name = PathBuf::from(name);
                    if queued.len() >= MAX_PRELOAD_PATHS {
                        return false;
                    }
                    if queued.insert(name.clone()) {
                        todo.push(name);
                    }
                }
            }
        }
    }
    true
}

/// Whether an `.npmrc` that governs the package at `dir` (in the directory or
/// above it, up to the repository root) sets `node-options`: npm and pnpm pass
/// it to every script as `NODE_OPTIONS`, which is not read.
fn npmrc_sets_node_options(dir: &Path) -> bool {
    for d in dir.ancestors() {
        let npmrc = read_unrecorded(&d.join(".npmrc"));
        if matches!(npmrc, Unrecorded::Refused)
            || npmrc.text().is_some_and(|text| {
                text.lines()
                    .any(|l| l.trim_start().starts_with("node-options"))
            })
        {
            return true;
        }
        if turborepo_trace_core::record::pause(|| fsx::exists(&d.join(".git"))) {
            break;
        }
    }
    false
}

/// The mapped runs of each script of the `package.json` at `manifest`, by
/// script name.
pub fn manifest_scripts(manifest: &Path) -> Vec<(String, Vec<Run>)> {
    // Not recorded as a read: a manifest changes with every dependency entry, and
    // only its runs matter here. Whoever keeps what is computed from them keys it
    // by the runs themselves (the graph store: the first-party key of the packages
    // the manifest governs).
    let Unrecorded::Bytes(bytes) = read_unrecorded(manifest) else {
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
    if npmrc_sets_node_options(dir) {
        return Vec::new();
    }
    scripts
        .iter()
        .filter_map(|(name, text)| {
            let runs = script_runs(dir, text.as_str()?);
            (!runs.is_empty()).then(|| (name.clone(), runs))
        })
        .collect()
}

/// The runs of the scripts of the `package.json` at `manifest`, each with the
/// name of its script. Runs that are equal (a `test` and a `test:watch` script)
/// are listed once.
pub fn manifest_runs(manifest: &Path) -> Vec<(String, Run)> {
    let mut out: Vec<(String, Run)> = Vec::new();
    for (name, runs) in manifest_scripts(manifest) {
        for run in runs {
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
            // What the runner loads is changed from outside its command line.
            "NODE_PATH=lib node --test",
            "cross-env TSX_TSCONFIG_PATH=tsconfig.alt.json tsx --test",
            "env TS_NODE_PROJECT=x node --loader ts-node/esm --test",
            "export NODE_OPTIONS='--import ./pre.mjs' && node --test",
            ". ./test-env.sh && node --test",
            "FLAG=1; node --test",
            "env-cmd node --test",
            "dotenv -c -- node --test",
            "node --test --test-isolation=none",
            "node --experimental-test-isolation none --test",
            "node --test --test-reporter ./reporter.mjs",
            // Another runner started as a file: a script of node, no run of its runner.
            "node ./node_modules/.bin/jest --ci",
            "node ./node_modules/.bin/jest --test",
            "node --experimental-vm-modules node_modules/.bin/jest",
            "jest",
            "# node --test",
        ] {
            assert!(runs(text).is_empty(), "{text}");
        }
        // The env files of a wrapper are the run's.
        assert_eq!(
            one("dotenv -e .env.test -- node --test --test-isolation=process").env_files,
            [EnvFile {
                path: ".env.test".into(),
                optional: true
            }]
        );
        assert_eq!(one("env-cmd -f .env.ci node --test").env_files.len(), 1);
        assert_eq!(one("dotenv -- node --test").env_files[0].path, ".env");
        assert!(one("FLAG=1 node --test").env_files.is_empty());
        // Two runs in one script.
        assert_eq!(
            runs("node --test test/unit/*.test.js && node --test test/e2e/*.test.js").len(),
            2
        );
    }

    /// A new directory below the temporary directory.
    #[allow(clippy::expect_used)]
    fn scratch(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "trace-node-test-{name}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir(&dir).expect("a new directory");
        dir.canonicalize().expect("the directory")
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn a_run_that_is_not_mapped_takes_the_same_command_with_it() {
        let dir = scratch("same-command");
        std::fs::write(dir.join(".env.plain"), "API_URL=http://localhost\n").expect("written");
        std::fs::write(
            dir.join(".env.preload"),
            "NODE_OPTIONS=--import ./pre.mjs\n",
        )
        .expect("written");
        for text in [
            // The second run gets a preload from the environment.
            "node --test a.test.js && export NODE_OPTIONS='--import ./pre.mjs' && node --test \
             a.test.js",
            "node --test a.test.js; . ./env.sh; node --test a.test.js",
            "node --test a.test.js && NODE_PATH=lib node --test a.test.js",
            "NODE_PATH=lib node --test a.test.js && node --test a.test.js",
            // From an env file.
            "node --test a.test.js && node --env-file=.env.preload --test a.test.js",
            "node --test a.test.js && dotenv -e .env.preload -- node --test a.test.js",
            "node --test a.test.js && env-cmd node --test a.test.js",
            // In a group, a branch or a subshell.
            "node --test a.test.js && { export NODE_OPTIONS=--import=./pre.mjs; node --test \
             a.test.js; }",
            "node --test a.test.js; if true; then export NODE_OPTIONS=--import=./pre.mjs; fi; \
             node --test a.test.js",
            "node --test a.test.js && (export NODE_OPTIONS=--import=./pre.mjs; node --test \
             a.test.js)",
            "node --test a.test.js && ( export NODE_OPTIONS=--import=./pre.mjs\nnode --test \
             a.test.js\n)",
            // Somewhere that is not followed: it may be the same directory.
            "node --test a.test.js; cd \"$DIR\" && node --test a.test.js",
            "node --test a.test.js && pnpm --dir \"$PWD\" exec node --test a.test.js",
        ] {
            assert!(script_runs(&dir, text).is_empty(), "{text}");
        }
        // Another command line is another run: the mapped one stays.
        for text in [
            "node --test a.test.js && NODE_PATH=lib node --test b.test.js",
            "node --test a.test.js && export CI=1 && node --test b.test.js",
            "node --test a.test.js && node --env-file=.env.preload --test b.test.js",
            "node --env-file=.env.plain --test a.test.js",
        ] {
            let runs = script_runs(&dir, text);
            assert_eq!(runs.len(), 1, "{text}");
            assert_eq!(runs[0].patterns, ["a.test.js"], "{text}");
        }
        std::fs::remove_dir_all(&dir).expect("removed");
    }

    #[cfg(unix)]
    #[test]
    #[allow(clippy::expect_used)]
    fn an_env_file_that_is_no_regular_file_or_too_large_is_not_read() {
        let dir = scratch("env-file");
        // A device behind a link has no end; a directory is no file.
        std::os::unix::fs::symlink("/dev/zero", dir.join(".env.stream")).expect("linked");
        std::fs::create_dir(dir.join(".env.dir")).expect("created");
        let large = std::fs::File::create(dir.join(".env.large")).expect("created");
        large.set_len(READ_LIMIT + 1).expect("sized");
        std::fs::write(dir.join(".env.plain"), "API_URL=http://localhost\n").expect("written");
        for file in [".env.stream", ".env.dir", ".env.large"] {
            for text in [
                format!("node --env-file={file} --test a.test.js"),
                format!("node --env-file-if-exists={file} --test a.test.js"),
                format!("dotenv -e {file} -- node --test a.test.js"),
            ] {
                assert!(script_runs(&dir, &text).is_empty(), "{text}");
            }
        }
        // Neither is a preload file, an `.npmrc` or the manifest itself.
        std::os::unix::fs::symlink("/dev/zero", dir.join("pre.mjs")).expect("linked");
        assert!(script_runs(&dir, "node --import ./pre.mjs --test a.test.js").is_empty());
        assert_eq!(
            script_runs(&dir, "node --env-file=.env.plain --test a.test.js").len(),
            1
        );
        let manifest = dir.join("package.json");
        std::fs::write(&manifest, r#"{"scripts":{"test":"node --test a.test.js"}}"#)
            .expect("written");
        assert_eq!(manifest_scripts(&manifest).len(), 1);
        std::os::unix::fs::symlink("/dev/zero", dir.join(".npmrc")).expect("linked");
        assert!(manifest_scripts(&manifest).is_empty());
        std::fs::remove_file(dir.join(".npmrc")).expect("removed");
        std::fs::remove_file(&manifest).expect("removed");
        std::os::unix::fs::symlink("/dev/zero", &manifest).expect("linked");
        assert!(manifest_scripts(&manifest).is_empty());
        std::fs::remove_dir_all(&dir).expect("removed");
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn a_preload_that_names_more_files_than_are_followed_is_not_mapped() {
        let dir = scratch("preload-walk");
        let script = "node --import ./pre.mjs --test a.test.js";
        // One file named many times is one file.
        std::fs::write(dir.join("pre.mjs"), "import './hook.mjs';\n".repeat(50_000))
            .expect("written");
        assert_eq!(script_runs(&dir, script).len(), 1);
        // More files than the walk takes: one of them may move the snapshots.
        let many: String = (0..MAX_PRELOAD_PATHS / 7 + 1)
            .map(|i| format!("import './hooks/h{i}.mjs';\n"))
            .collect();
        std::fs::write(dir.join("pre.mjs"), many).expect("written");
        assert!(script_runs(&dir, script).is_empty());
        // A preload named without its extension is read too.
        std::fs::write(
            dir.join("setup.js"),
            "require('node:test').snapshot.setResolveSnapshotPath((p) => p + '.snap');\n",
        )
        .expect("written");
        assert!(script_runs(&dir, "node --require ./setup --test a.test.js").is_empty());
        // More bytes than are read.
        std::fs::write(
            dir.join("pre.mjs"),
            "import './a.mjs';\nimport './b.mjs';\n",
        )
        .expect("written");
        for name in ["a.mjs", "b.mjs"] {
            let file = std::fs::File::create(dir.join(name)).expect("created");
            file.set_len(READ_LIMIT).expect("sized");
        }
        assert_eq!(script_runs(&dir, script).len(), 1);
        let more: String = (0..MAX_PRELOAD_BYTES / READ_LIMIT + 1)
            .map(|i| format!("import './big{i}.mjs';\n"))
            .collect();
        std::fs::write(dir.join("pre.mjs"), more).expect("written");
        for i in 0..MAX_PRELOAD_BYTES / READ_LIMIT + 1 {
            let file = std::fs::File::create(dir.join(format!("big{i}.mjs"))).expect("created");
            file.set_len(READ_LIMIT).expect("sized");
        }
        assert!(script_runs(&dir, script).is_empty());
        std::fs::remove_dir_all(&dir).expect("removed");
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn a_file_argument_nested_or_long_beyond_reading_is_not_mapped() {
        // On a small stack: the translation does not go as deep as the glob nests.
        let nested = format!("{}a{}.test.js", "@(".repeat(20_000), ")".repeat(20_000));
        let long = format!("{}.test.js", "a".repeat(MAX_FILE_ARGUMENT));
        let deep = format!("{}a{}.test.js", "@(".repeat(40), ")".repeat(40));
        let handle = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || {
                for file in [nested, long, deep] {
                    assert!(
                        matches!(run_of("node", &["--test", &file]), Parsed::Unmapped(_)),
                        "{} bytes",
                        file.len()
                    );
                    // A long name is exact; a glob nested deeper than is translated is not.
                    assert_eq!(
                        glob_is_exact(&file),
                        !file.contains('('),
                        "{} bytes",
                        file.len()
                    );
                }
            })
            .expect("a thread");
        handle.join().expect("no file argument ends the reader");
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
