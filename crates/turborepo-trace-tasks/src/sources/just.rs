//! `justfile`: recipes, dependencies, parameters, settings, variables with
//! `{{…}}` interpolation, shebang recipes, `import` and `mod`.
//!
//! Every recipe is `just:<dir>:<name>` (`<dir>`: the justfile's directory,
//! repo-relative; module recipes are `<module>::<name>`).
//!
//! | Construct | Becomes |
//! |---|---|
//! | recipe lines | command lines (each its own shell, in the justfile's directory or the `working-directory`) |
//! | `a: b (c "x") && d` | task dependencies on `b`, `c` and `d` |
//! | `{{expr}}` | static values (strings, `+`, `/`, variables, `justfile_directory()`, `join()`, ...) are substituted; parameters, backticks, `env()` and other unknown values make the word computed (an uncertainty) |
//! | `export x := …`, `set export` | environment for the lines |
//! | `set shell` to a non-POSIX shell, `[no-cd]` | uncertainties |
//! | `set dotenv-load` / `dotenv-filename` / `dotenv-path` | the dotenv file is an input |
//! | shebang recipe (`#!/usr/bin/env bash` / `node` / `python`), `[script]` | bash: the whole body is one command line; node/python: string-literal relative paths are entries or inputs plus the directory's files of that language; other interpreters: an uncertainty |
//! | `import 'x.just'`, `mod name` | merged recipes / module recipes |
//!
//! The [`JustCli`] recognizer turns `just build` in other tasks into
//! dependencies.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::fsx;

use crate::{
    recognizers::util::{Arg, Args},
    registry::{Recognition, RecognizeCx, Recognizer},
    sources::{
        DiscoveredTask, SourceCx, TaskSource, gha::script::shebang_interpreter, taskfile::repo_rel,
    },
    spec::{Command, EntryLang, TaskId, TaskRef, TaskSpec},
};

/// Task sources of this module.
pub fn sources() -> Vec<&'static dyn TaskSource> {
    vec![&Justfile]
}

/// Recognizers of this module.
pub fn recognizers() -> Vec<&'static dyn Recognizer> {
    vec![&JustCli]
}

/// File names `just` looks for.
pub const JUSTFILE_NAMES: &[&str] = &["justfile", "Justfile", ".justfile", "JUSTFILE"];

/// The justfile in `dir`, if any.
pub fn find_justfile(dir: &Path) -> Option<PathBuf> {
    fsx::first_existing(dir, JUSTFILE_NAMES)
}

/// One parsed recipe.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recipe {
    /// Name.
    pub name: String,
    /// Parameter names.
    pub params: Vec<String>,
    /// Dependencies (before and after `&&`).
    pub deps: Vec<String>,
    /// Attributes (`private`, `no-cd`, `working-directory('x')`,
    /// `script('python3')`).
    pub attrs: Vec<String>,
    /// Body lines (with leading indentation removed, continuation lines
    /// joined).
    pub body: Vec<String>,
}

/// One item of a justfile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    /// `[export] name := expr`.
    Assign {
        /// Name.
        name: String,
        /// Expression source.
        expr: String,
        /// `export`ed.
        export: bool,
    },
    /// `set name [:= expr]`.
    Set {
        /// Setting name.
        name: String,
        /// Expression source (`true` when absent).
        expr: String,
    },
    /// `alias a := b`.
    Alias {
        /// Alias.
        name: String,
        /// Target recipe.
        target: String,
    },
    /// `import 'path'` / `import? 'path'`.
    Import {
        /// Path expression.
        path: String,
        /// `import?`.
        optional: bool,
    },
    /// `mod name ['path']`.
    Mod {
        /// Module name.
        name: String,
        /// Optional path expression.
        path: Option<String>,
    },
    /// A recipe.
    Recipe(Recipe),
}

/// Parses justfile source into items. Never panics; unknown lines are skipped.
pub fn parse(src: &str) -> Vec<Item> {
    let mut items = Vec::new();
    let mut attrs: Vec<String> = Vec::new();
    let mut cur: Option<Recipe> = None;
    let lines: Vec<&str> = src.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let raw = lines[i].trim_end_matches('\r');
        i += 1;
        let indented = raw.starts_with(' ') || raw.starts_with('\t');
        if let Some(r) = cur.as_mut() {
            if indented || raw.trim().is_empty() {
                if raw.trim().is_empty() {
                    r.body.push(String::new());
                    continue;
                }
                let mut line = strip_indent(raw).to_string();
                // `\` continuation.
                while line.ends_with('\\') && i < lines.len() {
                    line.pop();
                    line.push(' ');
                    line.push_str(lines[i].trim());
                    i += 1;
                }
                r.body.push(line);
                continue;
            }
            let mut done = cur.take().unwrap_or_default();
            while done.body.last().is_some_and(String::is_empty) {
                done.body.pop();
            }
            items.push(Item::Recipe(done));
        }
        let t = raw.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if t.starts_with('[') && t.ends_with(']') {
            for a in split_top(&t[1..t.len() - 1], ',') {
                let a = a.trim();
                if !a.is_empty() {
                    attrs.push(a.to_string());
                }
            }
            continue;
        }
        if let Some(rest) = t.strip_prefix("set ") {
            let (name, expr) = match rest.split_once(":=") {
                Some((n, e)) => (n.trim(), e.trim()),
                None => (rest.trim(), "true"),
            };
            items.push(Item::Set {
                name: name.to_string(),
                expr: expr.to_string(),
            });
            attrs.clear();
            continue;
        }
        if let Some(rest) = t.strip_prefix("alias ") {
            if let Some((n, tg)) = rest.split_once(":=") {
                items.push(Item::Alias {
                    name: n.trim().to_string(),
                    target: tg.trim().to_string(),
                });
            }
            attrs.clear();
            continue;
        }
        if let Some(rest) = t
            .strip_prefix("import? ")
            .map(|r| (r, true))
            .or_else(|| t.strip_prefix("import ").map(|r| (r, false)))
        {
            items.push(Item::Import {
                path: rest.0.trim().to_string(),
                optional: rest.1,
            });
            attrs.clear();
            continue;
        }
        if let Some(rest) = t.strip_prefix("mod? ").or_else(|| t.strip_prefix("mod ")) {
            let rest = rest.trim();
            let (name, path) = match rest.split_once(char::is_whitespace) {
                Some((n, p)) => (n, Some(p.trim().to_string())),
                None => (rest, None),
            };
            items.push(Item::Mod {
                name: name.to_string(),
                path,
            });
            attrs.clear();
            continue;
        }
        if t.starts_with("unexport ") {
            continue;
        }
        // Assignment: `[export] name := expr` (possibly continued with an open paren).
        let (export, body) = match t.strip_prefix("export ") {
            Some(r) => (true, r.trim_start()),
            None => (false, t),
        };
        if let Some((n, e)) = body.split_once(":=") {
            let n = n.trim();
            if is_ident(n) {
                let mut expr = e.trim().to_string();
                // Multi-line expressions: keep reading while parens/brackets are open.
                while depth_of(&expr) > 0 && i < lines.len() {
                    expr.push(' ');
                    expr.push_str(lines[i].trim());
                    i += 1;
                }
                items.push(Item::Assign {
                    name: n.to_string(),
                    expr,
                    export,
                });
                attrs.clear();
                continue;
            }
        }
        // Recipe header: `@?name params...: deps && deps`.
        if let Some(colon) = header_colon(t) {
            let head = t[..colon].trim();
            let tail = t[colon + 1..].trim();
            let mut words = split_ws_top(head);
            if words.is_empty() {
                attrs.clear();
                continue;
            }
            let name = words.remove(0).trim_start_matches('@').to_string();
            if !is_ident(&name) {
                attrs.clear();
                continue;
            }
            let params = words
                .iter()
                .map(|w| {
                    let w = w.trim_start_matches(['+', '*', '$']);
                    w.split('=').next().unwrap_or(w).trim().to_string()
                })
                .filter(|w| !w.is_empty())
                .collect();
            let mut deps = Vec::new();
            for w in split_ws_top(tail) {
                if w == "&&" {
                    continue;
                }
                if w.starts_with('#') {
                    break;
                }
                let n = w
                    .trim_start_matches('(')
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim_end_matches(')');
                if is_ident(n) || n.contains("::") {
                    deps.push(n.to_string());
                }
            }
            cur = Some(Recipe {
                name,
                params,
                deps,
                attrs: std::mem::take(&mut attrs),
                body: Vec::new(),
            });
            continue;
        }
        attrs.clear();
    }
    if let Some(mut r) = cur {
        while r.body.last().is_some_and(String::is_empty) {
            r.body.pop();
        }
        items.push(Item::Recipe(r));
    }
    items
}

fn strip_indent(s: &str) -> &str {
    s.trim_start_matches([' ', '\t'])
}

fn is_ident(s: &str) -> bool {
    let mut b = s.bytes();
    b.next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        && b.all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

/// Open-paren/bracket depth at the end of `s` (outside quotes).
fn depth_of(s: &str) -> i32 {
    let mut d = 0;
    let mut q: Option<char> = None;
    for c in s.chars() {
        match q {
            Some(x) if c == x => q = None,
            Some(_) => {}
            None => match c {
                '\'' | '"' | '`' => q = Some(c),
                '(' | '[' => d += 1,
                ')' | ']' => d -= 1,
                _ => {}
            },
        }
    }
    d
}

/// The position of the recipe header's `:` (outside quotes and parens, not
/// `:=`, not `::`).
fn header_colon(t: &str) -> Option<usize> {
    let b = t.as_bytes();
    let mut q: Option<u8> = None;
    let mut d = 0i32;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match q {
            Some(x) if c == x => q = None,
            Some(_) => {}
            None => match c {
                b'\'' | b'"' | b'`' => q = Some(c),
                b'(' => d += 1,
                b')' => d -= 1,
                b':' if d == 0 => {
                    if b.get(i + 1) == Some(&b'=') {
                        return None;
                    }
                    if b.get(i + 1) == Some(&b':') {
                        i += 2;
                        continue;
                    }
                    return Some(i);
                }
                _ => {}
            },
        }
        i += 1;
    }
    None
}

/// Splits on `sep` outside quotes and parens.
fn split_top(s: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut q: Option<char> = None;
    let mut d = 0i32;
    for c in s.chars() {
        match q {
            Some(x) => {
                if c == x {
                    q = None;
                }
                cur.push(c);
            }
            None => {
                if c == sep && d == 0 {
                    out.push(std::mem::take(&mut cur));
                    continue;
                }
                match c {
                    '\'' | '"' | '`' => q = Some(c),
                    '(' => d += 1,
                    ')' => d -= 1,
                    _ => {}
                }
                cur.push(c);
            }
        }
    }
    out.push(cur);
    out
}

/// Splits on whitespace outside quotes and parens.
fn split_ws_top(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut q: Option<char> = None;
    let mut d = 0i32;
    for c in s.chars() {
        match q {
            Some(x) => {
                if c == x {
                    q = None;
                }
                cur.push(c);
            }
            None => {
                if c.is_whitespace() && d == 0 {
                    if !cur.is_empty() {
                        out.push(std::mem::take(&mut cur));
                    }
                    continue;
                }
                match c {
                    '\'' | '"' | '`' => q = Some(c),
                    '(' => d += 1,
                    ')' => d -= 1,
                    _ => {}
                }
                cur.push(c);
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Variables in scope: `None` for values that exist but are not statically
/// known.
pub type Vars = BTreeMap<String, Option<String>>;

/// What expressions can see besides variables.
struct EvalCx<'a> {
    vars: &'a Vars,
    justfile_dir: &'a Path,
    source_file: &'a Path,
}

/// Evaluates a just expression (static subset); `None` when not statically
/// known.
fn eval(expr: &str, cx: &EvalCx<'_>) -> Option<String> {
    let toks = tokenize(expr)?;
    let mut p = ExprParser { t: &toks, i: 0, cx };
    let v = p.expr(0)?;
    (p.i == toks.len()).then_some(v)
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Str(String),
    Ident(String),
    Op(char),
    Backtick,
}

fn tokenize(s: &str) -> Option<Vec<Tok>> {
    let c: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        if ch == '#' {
            break;
        }
        if ch == '\'' || ch == '"' {
            let triple = c.get(i + 1) == Some(&ch) && c.get(i + 2) == Some(&ch);
            let mut v = String::new();
            if triple {
                i += 3;
                while i < c.len()
                    && !(c[i] == ch && c.get(i + 1) == Some(&ch) && c.get(i + 2) == Some(&ch))
                {
                    v.push(c[i]);
                    i += 1;
                }
                i += 3;
            } else {
                i += 1;
                while i < c.len() && c[i] != ch {
                    if ch == '"' && c[i] == '\\' && i + 1 < c.len() {
                        i += 1;
                        v.push(match c[i] {
                            'n' => '\n',
                            't' => '\t',
                            x => x,
                        });
                    } else {
                        v.push(c[i]);
                    }
                    i += 1;
                }
                if i >= c.len() {
                    return None;
                }
                i += 1;
            }
            out.push(Tok::Str(v));
            continue;
        }
        if ch == '`' {
            out.push(Tok::Backtick);
            i += 1;
            while i < c.len() && c[i] != '`' {
                i += 1;
            }
            i += 1;
            continue;
        }
        if ch.is_ascii_alphabetic() || ch == '_' {
            let mut v = String::new();
            while i < c.len() && (c[i].is_ascii_alphanumeric() || c[i] == '_' || c[i] == '-') {
                v.push(c[i]);
                i += 1;
            }
            out.push(Tok::Ident(v));
            continue;
        }
        if matches!(
            ch,
            '+' | '/' | '(' | ')' | ',' | '{' | '}' | '=' | '!' | '&' | '|'
        ) {
            out.push(Tok::Op(ch));
            i += 1;
            continue;
        }
        return None;
    }
    Some(out)
}

struct ExprParser<'a> {
    t: &'a [Tok],
    i: usize,
    cx: &'a EvalCx<'a>,
}

impl ExprParser<'_> {
    fn expr(&mut self, depth: usize) -> Option<String> {
        if depth > 32 {
            return None;
        }
        if self.t.get(self.i) == Some(&Tok::Ident("if".into())) {
            return None;
        }
        let mut v = self.atom(depth)?;
        while let Some(Tok::Op(op @ ('+' | '/'))) = self.t.get(self.i) {
            let op = *op;
            self.i += 1;
            let r = self.atom(depth)?;
            if op == '+' {
                v.push_str(&r);
            } else {
                v = format!("{}/{}", v.trim_end_matches('/'), r);
            }
        }
        Some(v)
    }

    fn atom(&mut self, depth: usize) -> Option<String> {
        let t = self.t.get(self.i)?.clone();
        self.i += 1;
        match t {
            Tok::Str(s) => Some(s),
            Tok::Backtick => None,
            Tok::Op('(') => {
                let v = self.expr(depth + 1)?;
                (self.t.get(self.i) == Some(&Tok::Op(')'))).then(|| {
                    self.i += 1;
                    v
                })
            }
            Tok::Op('/') => {
                // `/ "x"`: an absolute path.
                let r = self.atom(depth)?;
                Some(format!("/{r}"))
            }
            Tok::Ident(name) => {
                if self.t.get(self.i) == Some(&Tok::Op('(')) {
                    self.i += 1;
                    let mut args = Vec::new();
                    let mut all_known = true;
                    while self.t.get(self.i) != Some(&Tok::Op(')')) {
                        let save = self.i;
                        match self.expr(depth + 1) {
                            Some(v) => args.push(v),
                            None => {
                                all_known = false;
                                // Skip to the next `,` or `)` at this level.
                                self.i = save;
                                let mut d = 0;
                                while let Some(t) = self.t.get(self.i) {
                                    match t {
                                        Tok::Op('(') => d += 1,
                                        Tok::Op(')') if d == 0 => break,
                                        Tok::Op(')') => d -= 1,
                                        Tok::Op(',') if d == 0 => break,
                                        _ => {}
                                    }
                                    self.i += 1;
                                }
                            }
                        }
                        if self.t.get(self.i) == Some(&Tok::Op(',')) {
                            self.i += 1;
                        } else if self.t.get(self.i) != Some(&Tok::Op(')')) {
                            return None;
                        }
                    }
                    self.i += 1;
                    if !all_known {
                        return None;
                    }
                    self.call(&name, &args)
                } else {
                    self.cx.vars.get(&name).cloned().flatten()
                }
            }
            Tok::Op(_) => None,
        }
    }

    fn call(&self, name: &str, args: &[String]) -> Option<String> {
        let dir_of = |p: &Path| p.parent().unwrap_or(p).to_string_lossy().into_owned();
        match (name, args) {
            ("justfile_directory" | "justfile_dir", []) => {
                Some(self.cx.justfile_dir.to_string_lossy().into_owned())
            }
            ("justfile", []) => self
                .cx
                .justfile_dir
                .join("justfile")
                .to_str()
                .map(str::to_string),
            ("source_directory" | "source_dir", []) => Some(dir_of(self.cx.source_file)),
            ("source_file", []) => Some(self.cx.source_file.to_string_lossy().into_owned()),
            ("join", [a, rest @ ..]) => {
                let mut p = PathBuf::from(a);
                for r in rest {
                    p.push(r);
                }
                Some(p.to_string_lossy().into_owned())
            }
            ("parent_directory", [a]) => Some(dir_of(Path::new(a))),
            ("clean", [a]) => Some(fsx::normalize(Path::new(a)).to_string_lossy().into_owned()),
            ("quote", [a]) => Some(format!("'{}'", a.replace('\'', "'\\''"))),
            ("uppercase", [a]) => Some(a.to_uppercase()),
            ("lowercase", [a]) => Some(a.to_lowercase()),
            ("trim", [a]) => Some(a.trim().to_string()),
            ("replace", [a, f, t]) => Some(a.replace(f.as_str(), t)),
            ("file_name", [a]) => Path::new(a)
                .file_name()
                .map(|f| f.to_string_lossy().into_owned()),
            ("without_extension", [a]) => Some(
                Path::new(a)
                    .with_extension("")
                    .to_string_lossy()
                    .into_owned(),
            ),
            _ => None,
        }
    }
}

/// Interpolates `{{expr}}` in a recipe line: unknown values become an unknown
/// shell variable (named after the parameter when the expression is one), so
/// the word is computed. `{{{{` is a literal `{{`.
fn interpolate(line: &str, cx: &EvalCx<'_>, params: &[String]) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(i) = rest.find("{{") {
        out.push_str(&rest[..i]);
        if rest[i..].starts_with("{{{{") {
            out.push_str("{{");
            rest = &rest[i + 4..];
            continue;
        }
        let after = &rest[i + 2..];
        let Some(j) = after.find("}}") else {
            out.push_str(&rest[i..]);
            return out;
        };
        let e = after[..j].trim();
        match eval(e, cx) {
            Some(v) => out.push_str(&v),
            None => {
                let var = if params.iter().any(|p| p == e) {
                    format!("${{just_param_{}}}", e.replace('-', "_"))
                } else {
                    "${__JUST_UNKNOWN}".to_string()
                };
                out.push_str(&var);
            }
        }
        rest = &after[j + 2..];
    }
    out.push_str(rest);
    out
}

/// `justfile` / `Justfile` / `.justfile`.
pub struct Justfile;

impl TaskSource for Justfile {
    fn id(&self) -> &'static str {
        "just"
    }

    fn config_file_names(&self) -> &'static [&'static str] {
        JUSTFILE_NAMES
    }

    fn discover(&self, cx: &SourceCx<'_>) -> Vec<DiscoveredTask> {
        let Some(file) = find_justfile(&cx.package.dir) else {
            return Vec::new();
        };
        let dir = cx.package.dir.clone();
        let mut l = Loader {
            rel: repo_rel(&cx.workspace.root, &dir),
            out: Vec::new(),
        };
        l.module(&file, &dir, "", &[], 0);
        crate::sources::gha::script::sanitize_lines(&mut l.out);
        l.out
    }
}

struct Loader {
    rel: String,
    out: Vec<DiscoveredTask>,
}

/// Settings that apply to a module's recipes.
#[derive(Default, Clone)]
struct Settings {
    export: bool,
    shell_ok: bool,
    shell: Option<String>,
    working_dir: Option<String>,
    dotenv: Vec<String>,
}

impl Loader {
    fn id(&self, name: &str) -> TaskId {
        TaskId::other("just", format!("{}:{name}", self.rel))
    }

    /// Collects the items of `file` and its imports (recursively) in order.
    fn collect(
        file: &Path,
        chain: &mut Vec<PathBuf>,
        items: &mut Vec<(PathBuf, Item)>,
        depth: usize,
    ) {
        if depth > 8 || chain.iter().any(|c| c == file) {
            return;
        }
        chain.push(file.to_path_buf());
        let src = fsx::read_to_string(file).unwrap_or_default();
        let dir = file.parent().unwrap_or(Path::new("/")).to_path_buf();
        for it in parse(&src) {
            if let Item::Import { path, .. } = &it {
                let vars = Vars::new();
                let ecx = EvalCx {
                    vars: &vars,
                    justfile_dir: &dir,
                    source_file: file,
                };
                if let Some(p) = eval(path, &ecx) {
                    let p = fsx::join(&dir, &p);
                    if fsx::is_file(&p) {
                        Self::collect(&p, chain, items, depth + 1);
                    }
                }
                continue;
            }
            items.push((file.to_path_buf(), it));
        }
    }

    fn module(
        &mut self,
        file: &Path,
        jdir: &Path,
        prefix: &str,
        parent_files: &[PathBuf],
        depth: usize,
    ) {
        let mut chain = Vec::new();
        let mut items = Vec::new();
        Self::collect(file, &mut chain, &mut items, 0);
        let mut files: Vec<PathBuf> = parent_files.to_vec();
        files.extend(chain);
        let mut vars = Vars::new();
        let mut exported: Vec<String> = Vec::new();
        let mut st = Settings {
            shell_ok: true,
            ..Default::default()
        };
        // Pass 1: settings and variables, in order (just allows forward references, but
        // static values are almost always defined before use).
        for (src_file, it) in &items {
            let ecx = EvalCx {
                vars: &vars,
                justfile_dir: jdir,
                source_file: src_file,
            };
            match it {
                Item::Assign { name, expr, export } => {
                    let v = eval(expr, &ecx);
                    vars.insert(name.clone(), v);
                    if *export {
                        exported.push(name.clone());
                    }
                }
                Item::Set { name, expr } => match name.as_str() {
                    "export" => st.export = expr != "false",
                    "shell" => {
                        let prog = expr
                            .trim_start_matches('[')
                            .split(',')
                            .next()
                            .unwrap_or("")
                            .trim()
                            .trim_matches(['\'', '"']);
                        let base = prog.rsplit('/').next().unwrap_or(prog);
                        st.shell_ok = matches!(base, "bash" | "sh" | "zsh" | "dash" | "ksh");
                        st.shell = Some(base.to_string());
                    }
                    "working-directory" => {
                        st.working_dir = eval(expr, &ecx).or(Some(String::new()))
                    }
                    "dotenv-load" if expr != "false" => st.dotenv.push(".env".into()),
                    "dotenv-filename" | "dotenv-path" => {
                        if let Some(v) = eval(expr, &ecx) {
                            st.dotenv.retain(|d| d != ".env");
                            st.dotenv.push(v);
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        let mut env: BTreeMap<String, String> = BTreeMap::new();
        for (k, v) in &vars {
            if (st.export || exported.contains(k))
                && let Some(v) = v
            {
                env.insert(k.clone(), v.clone());
            }
        }
        // Pass 2: recipes, aliases, modules.
        for (src_file, it) in &items {
            match it {
                Item::Alias { name, target } => {
                    let full = format!("{prefix}{name}");
                    let mut t = DiscoveredTask::new(self.id(&full), full, jdir.to_path_buf());
                    t.files = files.clone();
                    t.spec.dep(
                        TaskRef::Id {
                            id: self.id(&format!("{prefix}{target}")),
                        },
                        "just alias",
                    );
                    self.out.push(t);
                }
                Item::Mod { name, path } => {
                    if depth >= 4 {
                        continue;
                    }
                    let base = src_file.parent().unwrap_or(jdir).to_path_buf();
                    let found = match path {
                        Some(p) => {
                            let ecx = EvalCx {
                                vars: &vars,
                                justfile_dir: jdir,
                                source_file: src_file,
                            };
                            eval(p, &ecx)
                                .map(|p| fsx::join(&base, &p))
                                .filter(|p| fsx::is_file(p))
                        }
                        None => [
                            format!("{name}.just"),
                            format!("{name}/mod.just"),
                            format!("{name}/justfile"),
                            format!("{name}/Justfile"),
                            format!("{name}/.justfile"),
                        ]
                        .iter()
                        .map(|c| base.join(c))
                        .find(|p| fsx::is_file(p)),
                    };
                    if let Some(mf) = found {
                        let mdir = mf.parent().unwrap_or(&base).to_path_buf();
                        self.module(&mf, &mdir, &format!("{prefix}{name}::"), &files, depth + 1);
                    }
                }
                Item::Recipe(r) => {
                    let t = self.recipe(r, src_file, jdir, prefix, &vars, &env, &st, &files);
                    self.out.push(t);
                }
                _ => {}
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn recipe(
        &self,
        r: &Recipe,
        src_file: &Path,
        jdir: &Path,
        prefix: &str,
        vars: &Vars,
        env: &BTreeMap<String, String>,
        st: &Settings,
        files: &[PathBuf],
    ) -> DiscoveredTask {
        let full = format!("{prefix}{}", r.name);
        let mut vars = vars.clone();
        for p in &r.params {
            vars.insert(p.clone(), None);
        }
        let ecx = EvalCx {
            vars: &vars,
            justfile_dir: jdir,
            source_file: src_file,
        };
        let mut spec = TaskSpec::default();
        let mut cwd = match &st.working_dir {
            Some(w) => fsx::join(jdir, w),
            None => jdir.to_path_buf(),
        };
        let mut script_interp: Option<String> = None;
        for a in &r.attrs {
            let (an, arg) = match a.split_once('(') {
                Some((n, rest)) => (n.trim(), Some(rest.trim_end_matches(')').trim())),
                None => match a.split_once(':') {
                    Some((n, rest)) => (n.trim(), Some(rest.trim())),
                    None => (a.as_str(), None),
                },
            };
            match an {
                "no-cd" => spec.uncertain(
                    UncertaintyKind::CwdDependent,
                    format!("recipe `{full}` runs in the invocation directory (`[no-cd]`)"),
                ),
                "working-directory" => match arg.and_then(|x| eval(x, &ecx)) {
                    Some(w) => cwd = fsx::join(jdir, &w),
                    None => spec.uncertain(
                        UncertaintyKind::CwdDependent,
                        format!("computed working directory of recipe `{full}`"),
                    ),
                },
                "script" => {
                    script_interp =
                        Some(match arg.and_then(|x| eval(&split_top(x, ',')[0], &ecx)) {
                            Some(i) => i.split_whitespace().next().unwrap_or("sh").to_string(),
                            None => "sh".into(),
                        });
                }
                _ => {}
            }
        }
        for d in &st.dotenv {
            spec.input(fsx::join(jdir, d));
        }
        for d in &r.deps {
            let name = if d.contains("::") || prefix.is_empty() {
                d.clone()
            } else {
                format!("{prefix}{d}")
            };
            spec.dep(TaskRef::Id { id: self.id(&name) }, "just dependency");
        }
        let mut t = DiscoveredTask::new(self.id(&full), full.clone(), cwd.clone());
        let body: Vec<&String> = r.body.iter().collect();
        let shebang = body
            .first()
            .filter(|l| l.starts_with("#!"))
            .and_then(|l| shebang_interpreter(l));
        let interp = shebang.clone().or(script_interp);
        let code: Vec<String> = body
            .iter()
            .skip(usize::from(shebang.is_some()))
            .map(|l| interpolate(l, &ecx, &r.params))
            .collect();
        match interp.as_deref() {
            None if st.shell_ok => {
                for l in code {
                    let l = l
                        .trim_start_matches(['@', '-'])
                        .trim_start_matches(['@', '-']);
                    if !l.trim().is_empty() && !l.trim_start().starts_with('#') {
                        t.lines.push(l.to_string());
                    }
                }
            }
            None => {
                if !code.iter().all(|l| l.trim().is_empty()) {
                    spec.uncertain(
                        UncertaintyKind::UnknownCommand,
                        format!(
                            "recipe lines run by `{}`",
                            st.shell.as_deref().unwrap_or("?")
                        ),
                    );
                }
            }
            Some("bash" | "sh" | "zsh" | "dash" | "ksh") => {
                let whole = code.join("\n");
                if !whole.trim().is_empty() {
                    t.lines.push(whole);
                }
            }
            Some(i @ ("node" | "python" | "python3" | "tsx" | "bun" | "deno")) => {
                let lang = if i.starts_with("python") {
                    EntryLang::Python
                } else {
                    EntryLang::Js
                };
                inline_code(&code.join("\n"), &cwd, lang, &mut spec);
                if lang == EntryLang::Python {
                    spec.tool("pypi", "python");
                } else {
                    spec.tool("system", i);
                }
                if code
                    .iter()
                    .any(|l| l.contains("${__JUST_UNKNOWN}") || l.contains("${just_param_"))
                {
                    spec.uncertain(
                        UncertaintyKind::ComputedSpecifier,
                        format!("interpolated values in `{i}` recipe `{full}`"),
                    );
                }
            }
            Some(other) => spec.uncertain(
                UncertaintyKind::UnknownCommand,
                format!("recipe `{full}` run by `{other}`"),
            ),
        }
        t.env = env.clone();
        t.spec = spec;
        t.files = files.to_vec();
        t
    }
}

/// An inline node/python recipe body: relative string-literal paths that exist
/// are entries (code) or inputs (data), and the directory's files of that
/// language are a membership (local modules it may import).
fn inline_code(code: &str, cwd: &Path, lang: EntryLang, spec: &mut TaskSpec) {
    for s in string_literals(code) {
        if s.is_empty() || s.contains('\n') || s.contains("://") {
            continue;
        }
        if !(s.starts_with("./") || s.starts_with("../") || (s.contains('/') && s.contains('.'))) {
            continue;
        }
        let p = fsx::join(cwd, &s);
        if fsx::is_file(&p) {
            match EntryLang::of(&p) {
                EntryLang::Auto => spec.input(p),
                l => spec.entry(p, l),
            }
        } else if lang == EntryLang::Js {
            let r = crate::recognizers::util::resolve_js_file(cwd, &s);
            if fsx::is_file(&r) {
                spec.entry(r, EntryLang::Js);
            }
        }
    }
    let pat = match lang {
        EntryLang::Python => "**/*.py",
        _ => "**/*.{js,mjs,cjs,ts,mts,cts,json}",
    };
    spec.membership(cwd.to_path_buf(), pat, "inline recipe code");
}

/// Contents of `'…'` and `"…"` literals.
fn string_literals(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    let c: Vec<char> = code.chars().collect();
    let mut i = 0;
    while i < c.len() {
        if c[i] == '\'' || c[i] == '"' || c[i] == '`' {
            let q = c[i];
            let mut v = String::new();
            i += 1;
            while i < c.len() && c[i] != q && c[i] != '\n' {
                if c[i] == '\\' {
                    i += 1;
                }
                if let Some(&ch) = c.get(i) {
                    v.push(ch);
                }
                i += 1;
            }
            out.push(v);
        }
        i += 1;
    }
    out
}

/// `just [options] [recipe…]`: task dependencies.
pub struct JustCli;

impl Recognizer for JustCli {
    fn id(&self) -> &'static str {
        "just"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["just"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        out.tool("system", "just");
        let args = Args::new(
            cmd,
            0,
            &[
                "-f",
                "--justfile",
                "-d",
                "--working-directory",
                "--dotenv-filename",
                "--dotenv-path",
                "--shell",
                "--shell-arg",
                "--color",
                "--chooser",
                "--set",
                "--list-heading",
                "--list-prefix",
                "--command-color",
            ],
        )
        .collect_all();
        if args.iter().any(|a| {
            matches!(
                a,
                Arg::Opt(
                    "-l" | "--list"
                        | "--summary"
                        | "--dump"
                        | "--evaluate"
                        | "--fmt"
                        | "--show"
                        | "-s"
                        | "--variables"
                        | "--init"
                        | "--edit"
                        | "--choose"
                        | "--groups"
                        | "--version"
                        | "-h"
                        | "--help"
                        | "--completions",
                    _
                )
            )
        }) {
            return Recognition::Ignored;
        }
        let file = match crate::recognizers::util::opt(&args, &["-f", "--justfile"]) {
            Some(f) => Some(cmd.path(f)),
            None => {
                let mut found = None;
                for d in cmd.cwd.ancestors() {
                    if let Some(f) = find_justfile(d) {
                        found = Some(f);
                        break;
                    }
                    if d == cx.repo_root {
                        break;
                    }
                }
                found
            }
        };
        let Some(file) = file else {
            out.uncertain(UncertaintyKind::UnknownCommand, "just without a justfile");
            cx.whole_package(out, "just without a justfile");
            return Recognition::Partial;
        };
        out.config(file.clone());
        let dir = repo_rel(cx.repo_root, file.parent().unwrap_or(cx.repo_root));
        let id = |n: &str| TaskRef::Id {
            id: TaskId::other("just", format!("{dir}:{n}")),
        };
        // `--set k v` takes two values: drop the second.
        let mut pos: Vec<&str> = Vec::new();
        let mut skip = false;
        for a in &args {
            match a {
                Arg::Opt("--set", _) => skip = true,
                Arg::Pos(p) if skip => {
                    let _ = p;
                    skip = false;
                }
                Arg::Pos(p) => pos.push(p),
                _ => {}
            }
        }
        if pos.is_empty() {
            // The default recipe: `[default]`, else the first one.
            let items = parse(&fsx::read_to_string(&file).unwrap_or_default());
            let recipes: Vec<&Recipe> = items
                .iter()
                .filter_map(|i| match i {
                    Item::Recipe(r) => Some(r),
                    _ => None,
                })
                .collect();
            let def = recipes
                .iter()
                .find(|r| r.attrs.iter().any(|a| a == "default"))
                .or(recipes.first());
            if let Some(r) = def {
                out.dep(id(&r.name), "just");
            }
            return Recognition::Full;
        }
        // Arguments may be recipe names or parameters: every word that names a recipe
        // resolves (a sound over-approximation); `mod recipe` is also `mod::recipe`.
        for (i, p) in pos.iter().enumerate() {
            let p = p.split_once('=').map_or(*p, |_| "");
            if p.is_empty() {
                continue;
            }
            out.dep(id(p), "just");
            if let Some(next) = pos.get(i + 1) {
                out.dep(id(&format!("{p}::{next}")), "just");
            }
        }
        Recognition::Full
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_recipes() {
        let src = "set dotenv-load\nexport X := 'a' + \"b\"\n\n# c\n[private]\nbuild target='x' \
                   *args: (gen \"1\") lint && post\n    @echo {{target}}\n    tsc -p \\\n      \
                   tsconfig.json\n\nalias b := build\nmod tools\nimport? 'x.just'\nlint:\n  \
                   eslint .\n";
        let items = parse(src);
        let r = items
            .iter()
            .find_map(|i| match i {
                Item::Recipe(r) if r.name == "build" => Some(r),
                _ => None,
            })
            .unwrap();
        assert_eq!(r.params, ["target", "args"]);
        assert_eq!(r.deps, ["gen", "lint", "post"]);
        assert_eq!(r.attrs, ["private"]);
        assert_eq!(r.body, ["@echo {{target}}", "tsc -p  tsconfig.json"]);
        assert!(items.contains(&Item::Alias {
            name: "b".into(),
            target: "build".into()
        }));
        assert!(
            items
                .iter()
                .any(|i| matches!(i, Item::Mod { name, .. } if name == "tools"))
        );
    }

    #[test]
    fn evaluates() {
        let mut vars = Vars::new();
        vars.insert("a".into(), Some("x".into()));
        vars.insert("p".into(), None);
        let cx = EvalCx {
            vars: &vars,
            justfile_dir: Path::new("/r"),
            source_file: Path::new("/r/justfile"),
        };
        assert_eq!(eval("a + 'y'", &cx).as_deref(), Some("xy"));
        assert_eq!(
            eval("justfile_directory() / 'src'", &cx).as_deref(),
            Some("/r/src")
        );
        assert_eq!(eval("join(a, 'b', \"c\")", &cx).as_deref(), Some("x/b/c"));
        assert_eq!(eval("p", &cx), None);
        assert_eq!(eval("`git rev-parse HEAD`", &cx), None);
        assert_eq!(eval("env('X', 'd')", &cx), None);
        assert_eq!(eval("if a == 'x' { 'y' } else { 'z' }", &cx), None);
        assert_eq!(
            interpolate("echo {{a}} {{p}} {{{{x}}", &cx, &["p".into()]),
            "echo x ${just_param_p} {{x}}"
        );
    }

    #[test]
    fn parsers_never_panic() {
        let alphabet = b"ab:=+/'\"`()[]{}@*$#!\\\n \t&,set mod import alias export";
        let mut x: u64 = 0x853c49e6748fea9b;
        let vars = Vars::new();
        let cx = EvalCx {
            vars: &vars,
            justfile_dir: Path::new("/r"),
            source_file: Path::new("/r/justfile"),
        };
        for _ in 0..3000 {
            let mut s = String::new();
            for _ in 0..48 {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                s.push(alphabet[(x % alphabet.len() as u64) as usize] as char);
            }
            parse(&s);
            eval(&s, &cx);
            interpolate(&s, &cx, &[]);
        }
        for s in [
            "'", "'''", "\"\\", "(((", "f(", "f(a,", "{{", "a:", "[x", "mod", "@:",
        ] {
            parse(s);
            eval(s, &cx);
        }
    }
}
