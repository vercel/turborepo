//! Helpers shared by recognizers (also usable by other modules' recognizers).

use std::path::{Path, PathBuf};

use turborepo_trace_frameworks::{fsx, jsconfig};
use turborepo_trace_lang_js::extract::config::ConfigVal;

use crate::spec::{Command, EntryLang, Membership, TaskSpec};

/// Simple argv option scanner: knows which options take a value.
pub struct Args<'a> {
    words: Vec<&'a str>,
    i: usize,
    with_value: &'a [&'a str],
}

/// One scanned item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Arg<'a> {
    /// `--name value`, `--name=value`, `-n value`, `-nvalue` is not split. The
    /// name keeps its dashes.
    Opt(&'a str, Option<&'a str>),
    /// A positional argument (everything after `--` is positional too).
    Pos(&'a str),
}

impl<'a> Args<'a> {
    /// Scans the arguments of `cmd` from index `skip` (0 = first argument after
    /// the program). `with_value` lists options that take a separate value
    /// (`-p`, `--config`).
    pub fn new(cmd: &'a Command, skip: usize, with_value: &'a [&'a str]) -> Args<'a> {
        Args {
            words: cmd.args().skip(skip).collect(),
            i: 0,
            with_value,
        }
    }

    /// All items.
    pub fn collect_all(mut self) -> Vec<Arg<'a>> {
        let mut out = Vec::new();
        let mut rest = false;
        while let Some(w) = self.words.get(self.i).copied() {
            self.i += 1;
            if rest {
                out.push(Arg::Pos(w));
                continue;
            }
            if w == "--" {
                rest = true;
                continue;
            }
            if w.starts_with('-') && w.len() > 1 {
                if let Some((n, v)) = w.split_once('=') {
                    out.push(Arg::Opt(n, Some(v)));
                } else if self.with_value.contains(&w) {
                    let v = self.words.get(self.i).copied();
                    self.i += 1;
                    out.push(Arg::Opt(w, v));
                } else {
                    out.push(Arg::Opt(w, None));
                }
            } else {
                out.push(Arg::Pos(w));
            }
        }
        out
    }
}

/// The value of option `names` (first occurrence) among `args`.
pub fn opt<'a>(args: &[Arg<'a>], names: &[&str]) -> Option<&'a str> {
    args.iter().find_map(|a| match a {
        Arg::Opt(n, v) if names.contains(n) => *v,
        _ => None,
    })
}

/// Every value of option `names`.
pub fn opts<'a>(args: &[Arg<'a>], names: &[&str]) -> Vec<&'a str> {
    args.iter()
        .filter_map(|a| match a {
            Arg::Opt(n, v) if names.contains(n) => *v,
            _ => None,
        })
        .collect()
}

/// Whether flag `names` is present.
pub fn flag(args: &[Arg<'_>], names: &[&str]) -> bool {
    args.iter()
        .any(|a| matches!(a, Arg::Opt(n, _) if names.contains(n)))
}

/// Positional arguments.
pub fn positionals<'a>(args: &[Arg<'a>]) -> Vec<&'a str> {
    args.iter()
        .filter_map(|a| match a {
            Arg::Pos(p) => Some(*p),
            _ => None,
        })
        .collect()
}

/// Adds a path argument as an input: a glob becomes a membership of the cwd, a
/// directory a `**` membership, anything else (existing or not) an input file.
pub fn add_path_input(cmd: &Command, arg: &str, reason: &str, out: &mut TaskSpec) {
    if arg.contains('$') || arg.contains('`') {
        // A computed path: the registry adds the package directory for dynamic words.
        return;
    }
    if crate::glob::is_glob(arg) {
        add_glob(&cmd.cwd, arg, reason, out);
        return;
    }
    let p = cmd.path(arg);
    if out.written.contains(&p) {
        // An absolute path the command writes out: not looked up where it lands on this
        // machine. A directory that it may name inside the repository is a membership,
        // like a directory inside; the files it may name are the input's readings.
        for (dir, _) in fsx::readings(&p) {
            if fsx::is_dir(&dir) {
                out.membership(dir, "**", reason);
            }
        }
        out.input(p);
    } else if fsx::is_dir(&p) {
        out.membership(p, "**", reason);
    } else {
        out.input(p);
    }
}

/// Adds a glob (relative to `base`, or absolute) as a membership rooted at its
/// static prefix. A leading `!` adds an exclude to the last membership with the
/// same base instead.
pub fn add_glob(base: &Path, pattern: &str, reason: &str, out: &mut TaskSpec) {
    if let Some(neg) = pattern.strip_prefix('!') {
        let (prefix, rest) = crate::glob::split_static_prefix(neg);
        let dir = fsx::join(base, prefix);
        // Exclude from every membership whose dir contains it.
        for m in out
            .memberships
            .iter_mut()
            .filter(|m| dir.starts_with(&m.dir))
        {
            let rel = dir.strip_prefix(&m.dir).unwrap_or(Path::new(""));
            let rel = rel.to_string_lossy();
            let pat = match (rel.is_empty(), rest.is_empty()) {
                (true, _) => rest.to_string(),
                (false, true) => rel.to_string(),
                (false, false) => format!("{rel}/{rest}"),
            };
            if !pat.is_empty() {
                m.exclude.push(pat);
            }
        }
        return;
    }
    let (prefix, rest) = crate::glob::split_static_prefix(pattern);
    let dir = fsx::join(base, prefix);
    if rest.is_empty() {
        if fsx::is_dir(&dir) {
            out.membership(dir, "**", reason);
        } else {
            out.input(dir);
        }
    } else {
        out.memberships.push(Membership::new(dir, rest, reason));
    }
}

/// Adds existing-file positional arguments as inputs (for programs whose
/// arguments are mostly paths), skipping words that look like options or
/// values.
pub fn add_existing_files(cmd: &Command, words: &[&str], out: &mut TaskSpec) {
    for w in words {
        if w.starts_with('-') || w.is_empty() {
            continue;
        }
        let p = cmd.path(w);
        if fsx::is_file(&p) {
            out.input(p);
        }
    }
}

/// Finds the first existing config among `names` in `dir`.
pub fn find_config(dir: &Path, names: &[&str]) -> Option<PathBuf> {
    fsx::first_existing(dir, names)
}

/// The nearest ancestor of `dir` (inclusive) containing `name`, stopping at
/// `stop` (inclusive) when given.
pub fn find_up(dir: &Path, name: &str, stop: Option<&Path>) -> Option<PathBuf> {
    let mut d = Some(dir);
    while let Some(x) = d {
        let p = x.join(name);
        if fsx::is_file(&p) {
            return Some(p);
        }
        if stop.is_some_and(|s| s == x) {
            return None;
        }
        d = x.parent();
    }
    None
}

/// Adds a code config file: it is a config, and a JS entry (so what it imports
/// counts).
pub fn add_code_config(p: &Path, out: &mut TaskSpec) {
    out.config(p.to_path_buf());
    if EntryLang::of(p) == EntryLang::Js {
        out.entry(p.to_path_buf(), EntryLang::Js);
    }
}

/// Loads a JS/TS/JSON config with the lang-js config evaluator: the value plus
/// every file read (added as configs; code files also as entries).
pub fn load_js_config(p: &Path, out: &mut TaskSpec) -> Option<ConfigVal> {
    let l = jsconfig::load(p, jsconfig::LoadOptions::default());
    for f in &l.files {
        out.config(f.clone());
    }
    for f in &l.code_files {
        out.entry(f.clone(), EntryLang::Js);
    }
    l.value
}

/// The string values of a config value (a string, an array of strings, the
/// values of an object, alternatives); `complete` is false when some part was
/// not a string.
pub fn strings(v: &ConfigVal) -> (Vec<String>, bool) {
    let mut out = Vec::new();
    let mut complete = true;
    collect_strings(v, &mut out, &mut complete, 0);
    (out, complete)
}

fn collect_strings(v: &ConfigVal, out: &mut Vec<String>, complete: &mut bool, depth: usize) {
    if depth > 8 {
        *complete = false;
        return;
    }
    match v {
        ConfigVal::Null => {}
        ConfigVal::Arr(a) => a
            .iter()
            .for_each(|x| collect_strings(x, out, complete, depth + 1)),
        ConfigVal::Alts(a) => a
            .iter()
            .for_each(|x| collect_strings(x, out, complete, depth + 1)),
        ConfigVal::Obj(o) => {
            for (_, x) in &o.entries {
                collect_strings(x, out, complete, depth + 1);
            }
        }
        ConfigVal::CwdPath(v) => match v.as_str() {
            Some(s) => out.push(s.to_string()),
            None => *complete = false,
        },
        other => match other.as_str() {
            Some(s) => out.push(s.to_string()),
            None => *complete = false,
        },
    }
}

/// Resolves a module-like entry (`./src/index.ts`, `src/index`) against `dir`:
/// exact, with a JS/TS extension, or an `index` file. Returns the plain join
/// when nothing exists (a probe).
pub fn resolve_js_file(dir: &Path, spec: &str) -> PathBuf {
    let base = fsx::join(dir, spec);
    if fsx::is_file(&base) {
        return base;
    }
    let s = base.to_string_lossy().into_owned();
    for e in ["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"] {
        let p = PathBuf::from(format!("{s}.{e}"));
        if fsx::is_file(&p) {
            return p;
        }
    }
    for e in ["ts", "tsx", "js", "jsx", "mjs", "cjs"] {
        let p = base.join(format!("index.{e}"));
        if fsx::is_file(&p) {
            return p;
        }
    }
    base
}

/// Whether `s` looks like a file path rather than a bare package or script
/// name.
pub fn is_pathish(s: &str) -> bool {
    s.starts_with("./")
        || s.starts_with("../")
        || s.starts_with('/')
        || s.contains('/')
        || [
            ".js", ".mjs", ".cjs", ".ts", ".mts", ".cts", ".tsx", ".jsx", ".py", ".go", ".sh",
        ]
        .iter()
        .any(|e| s.ends_with(e))
}
