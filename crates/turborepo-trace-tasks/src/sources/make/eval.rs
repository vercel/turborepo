//! Make variable expansion: `$(VAR)`, `${VAR}`, `$X`, substitution references
//! (`$(VAR:.c=.o)`), automatic variables (`$@ $< $^ $* $(@D) $(@F)` …) and the
//! static functions.
//!
//! Nothing is executed. A part that cannot be known statically (`$(shell …)`,
//! an undefined variable that the environment or the command line may set,
//! `$(MAKECMDGOALS)`) expands to an *unknown marker* (see [`unknown`]) that
//! consumers detect with [`has_unknown`]: prerequisites containing one become
//! uncertainties, recipe words become dynamic shell words. `$(wildcard …)`
//! lists the filesystem through `fsx` and records the glob (a file
//! added or removed changes the result) in [`Exp::globs`]. Constructs that may
//! change the makefile itself (`$(eval …)`) are recorded in [`Exp::issues`].

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::fsx;

/// Start of an unknown marker.
pub const UNK_OPEN: char = '\u{1}';
/// End of an unknown marker.
pub const UNK_CLOSE: char = '\u{2}';

/// Maximum nesting of expansions (recursive variables, function arguments).
const MAX_DEPTH: u32 = 48;
/// Maximum length of one expansion result.
const MAX_LEN: usize = 256 * 1024;
/// Maximum total work (characters produced) per expander.
const MAX_WORK: usize = 4 * 1024 * 1024;

/// An unknown value named `name` (sanitized to `[A-Za-z0-9_]`).
pub fn unknown(name: &str) -> String {
    let mut s = String::with_capacity(name.len() + 2);
    s.push(UNK_OPEN);
    for c in name.chars().take(48) {
        s.push(if c.is_ascii_alphanumeric() || c == '_' {
            c
        } else {
            '_'
        });
    }
    s.push(UNK_CLOSE);
    s
}

/// Whether `s` contains an unknown part.
pub fn has_unknown(s: &str) -> bool {
    s.contains(UNK_OPEN)
}

/// Replaces unknown markers with `${__make_<name>}` (an unset shell variable,
/// so the shell parser marks the word dynamic).
pub fn unknown_to_shell(s: &str) -> String {
    if !has_unknown(s) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 16);
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == UNK_OPEN {
            out.push_str("${__make_");
            for d in it.by_ref() {
                if d == UNK_CLOSE {
                    break;
                }
                out.push(d);
            }
            out.push('}');
        } else if c != UNK_CLOSE {
            out.push(c);
        }
    }
    out
}

/// A directory listing filtered by a glob (`$(wildcard src/*.c)` → `src`,
/// `*.c`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Glob {
    /// Directory (absolute).
    pub dir: PathBuf,
    /// Glob relative to `dir`.
    pub pattern: String,
}

/// Something that could not be modelled.
pub type Issue = (UncertaintyKind, String);

/// A make variable.
#[derive(Clone, Debug, Default)]
pub struct Var {
    /// Raw value (recursive) or expanded value (simple).
    pub value: String,
    /// Recursively expanded (`=`, `?=`, `define`).
    pub recursive: bool,
    /// Issues attached to the value (assigned in an unknown conditional branch,
    /// or its immediate expansion had issues).
    pub issues: Vec<Issue>,
    /// Globs its immediate expansion listed.
    pub globs: Vec<Glob>,
    /// `export`ed to recipes.
    pub exported: bool,
    /// Defined with `override` (or by GNU make's defaults).
    pub origin: Origin,
}

/// Where a variable comes from.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Origin {
    /// The makefile.
    #[default]
    File,
    /// GNU make's built-in defaults (`CC`, `RM`, ...).
    Default,
    /// Automatic or function-local (`$@`, `$(1)`, foreach variables).
    Automatic,
}

impl Var {
    /// A simple (already expanded) variable.
    pub fn simple(value: impl Into<String>) -> Var {
        Var {
            value: value.into(),
            ..Default::default()
        }
    }
    /// A recursive variable.
    pub fn recursive(value: impl Into<String>) -> Var {
        Var {
            value: value.into(),
            recursive: true,
            ..Default::default()
        }
    }
}

/// Variables by name.
pub type Vars = HashMap<String, Var>;

/// GNU make's default variables (`make -p`), the subset whose value is known.
pub fn default_var(name: &str) -> Option<&'static str> {
    Some(match name {
        "MAKE" => "make",
        "SHELL" => "/bin/sh",
        ".SHELLFLAGS" => "-c",
        "CC" => "cc",
        "CXX" => "g++",
        "CPP" => "$(CC) -E",
        "AR" => "ar",
        "ARFLAGS" => "rv",
        "AS" => "as",
        "LD" => "ld",
        "RM" => "rm -f",
        "LEX" => "lex",
        "YACC" => "yacc",
        "FC" => "f77",
        "PC" => "pc",
        "CO" => "co",
        "GET" => "get",
        "MAKEINFO" => "makeinfo",
        "TEX" => "tex",
        "OUTPUT_OPTION" => "-o $@",
        "COMPILE.c" => "$(CC) $(CFLAGS) $(CPPFLAGS) $(TARGET_ARCH) -c",
        "COMPILE.cc" | "COMPILE.cpp" | "COMPILE.C" => {
            "$(CXX) $(CXXFLAGS) $(CPPFLAGS) $(TARGET_ARCH) -c"
        }
        "LINK.o" => "$(CC) $(LDFLAGS) $(TARGET_ARCH)",
        "LINK.c" => "$(CC) $(CFLAGS) $(CPPFLAGS) $(LDFLAGS) $(TARGET_ARCH)",
        "LINK.cc" | "LINK.cpp" => "$(CXX) $(CXXFLAGS) $(CPPFLAGS) $(LDFLAGS) $(TARGET_ARCH)",
        // Undefined by default (empty); flags, not inputs.
        "CFLAGS" | "CXXFLAGS" | "CPPFLAGS" | "LDFLAGS" | "LDLIBS" | "LOADLIBES" | "ASFLAGS"
        | "LFLAGS" | "YFLAGS" | "FFLAGS" | "TARGET_ARCH" | "MAKEFLAGS" | "MFLAGS"
        | "GNUMAKEFLAGS" | "MAKEOVERRIDES" | "SUFFIXES" | "VPATH" => "",
        "MAKELEVEL" => "0",
        "MAKE_VERSION" => "4.4",
        _ => return None,
    })
}

/// Names of the make functions.
const FUNCS: &[&str] = &[
    "subst",
    "patsubst",
    "strip",
    "findstring",
    "filter",
    "filter-out",
    "sort",
    "word",
    "wordlist",
    "words",
    "firstword",
    "lastword",
    "dir",
    "notdir",
    "suffix",
    "basename",
    "addsuffix",
    "addprefix",
    "join",
    "wildcard",
    "realpath",
    "abspath",
    "if",
    "or",
    "and",
    "intcmp",
    "foreach",
    "file",
    "call",
    "value",
    "eval",
    "origin",
    "flavor",
    "let",
    "shell",
    "guile",
    "error",
    "warning",
    "info",
];

/// One expansion context: global variables, local frames (automatic variables,
/// target variables, `call` arguments, `foreach` variables), and what was
/// observed.
pub struct Exp<'a> {
    vars: &'a Vars,
    dir: &'a Path,
    locals: Vec<Vars>,
    /// Issues (taints of used variables, `$(eval)`, …).
    pub issues: Vec<Issue>,
    /// Globs listed by `$(wildcard)`.
    pub globs: Vec<Glob>,
    depth: u32,
    work: usize,
    /// Variable names being expanded (self-reference guard).
    active: Vec<String>,
}

impl<'a> Exp<'a> {
    /// An expander over `vars`, with the makefile directory `dir` (make's cwd).
    pub fn new(vars: &'a Vars, dir: &'a Path) -> Exp<'a> {
        Exp {
            vars,
            dir,
            locals: Vec::new(),
            issues: Vec::new(),
            globs: Vec::new(),
            depth: 0,
            work: 0,
            active: Vec::new(),
        }
    }

    /// Pushes a frame of local variables (innermost wins).
    pub fn push_locals(&mut self, v: Vars) {
        self.locals.push(v);
    }

    fn issue(&mut self, kind: UncertaintyKind, d: impl Into<String>) {
        let d = d.into();
        if !self.issues.iter().any(|(k, x)| *k == kind && *x == d) {
            self.issues.push((kind, d));
        }
    }

    /// Expands `s`.
    pub fn expand(&mut self, s: &str) -> String {
        if !s.contains('$') {
            return s.to_string();
        }
        if self.depth >= MAX_DEPTH || self.work > MAX_WORK {
            self.issue(UncertaintyKind::ParseIncomplete, "make expansion too deep");
            return unknown("deep");
        }
        self.depth += 1;
        let b = s.as_bytes();
        let mut out = String::new();
        let mut i = 0;
        while i < b.len() {
            if b[i] != b'$' {
                let j = s[i..].find('$').map_or(b.len(), |k| i + k);
                out.push_str(&s[i..j]);
                i = j;
                continue;
            }
            if i + 1 >= b.len() {
                out.push('$');
                break;
            }
            match b[i + 1] {
                b'$' => {
                    out.push('$');
                    i += 2;
                }
                open @ (b'(' | b'{') => {
                    let close = if open == b'(' { b')' } else { b'}' };
                    match find_close(b, i + 2, open, close) {
                        Some(j) => {
                            let v = self.reference(&s[i + 2..j]);
                            out.push_str(&v);
                            i = j + 1;
                        }
                        None => {
                            self.issue(
                                UncertaintyKind::ParseIncomplete,
                                "unterminated make variable reference",
                            );
                            out.push_str(&s[i..]);
                            break;
                        }
                    }
                }
                _ => {
                    let ch = s[i + 1..].chars().next().unwrap_or('$');
                    let v = self.var(&ch.to_string());
                    out.push_str(&v);
                    i += 1 + ch.len_utf8();
                }
            }
            if out.len() > MAX_LEN {
                self.issue(UncertaintyKind::ParseIncomplete, "make expansion too long");
                out.clear();
                out.push_str(&unknown("long"));
                break;
            }
        }
        self.work += out.len();
        self.depth -= 1;
        out
    }

    /// The contents of `$( … )`.
    fn reference(&mut self, inner: &str) -> String {
        // Function call: `name args`.
        if let Some(ws) = inner.find([' ', '\t']) {
            let name = &inner[..ws];
            if FUNCS.contains(&name) {
                let args = inner[ws..].trim_start_matches([' ', '\t']);
                return self.function(name, args);
            }
        }
        // Substitution reference: `var:a=b`.
        if let Some(colon) = find_top(inner, b':')
            && let Some(eq) = find_top(&inner[colon + 1..], b'=')
        {
            let name = self.expand(&inner[..colon]);
            let from = self.expand(&inner[colon + 1..colon + 1 + eq]);
            let to = self.expand(&inner[colon + 2 + eq..]);
            let val = self.var(name.trim());
            let (pf, pt) = if from.contains('%') {
                (from, to)
            } else {
                (format!("%{from}"), format!("%{to}"))
            };
            return words(&val)
                .map(|w| patsubst_word(&pf, &pt, w))
                .collect::<Vec<_>>()
                .join(" ");
        }
        let name = self.expand(inner);
        self.var(&name)
    }

    /// The value of variable `name`.
    pub fn var(&mut self, name: &str) -> String {
        if has_unknown(name) {
            return unknown("computed");
        }
        // Locals, innermost first.
        let mut found: Option<Var> = None;
        for frame in self.locals.iter().rev() {
            if let Some(v) = frame.get(name) {
                found = Some(v.clone());
                break;
            }
        }
        if found.is_none()
            && let Some(v) = self.vars.get(name)
        {
            found = Some(v.clone());
        }
        if let Some(v) = found {
            return self.value_of(name, &v);
        }
        // `$(@D)`, `$(<F)`, …
        let b = name.as_bytes();
        if b.len() == 2 && b"@<^+*?|%".contains(&b[0]) && (b[1] == b'D' || b[1] == b'F') {
            let base = self.var(&name[..1]);
            return words(&base)
                .map(|w| {
                    if b[1] == b'D' {
                        match w.rfind('/') {
                            Some(0) => "/".to_string(),
                            Some(i) => w[..i].to_string(),
                            None => ".".to_string(),
                        }
                    } else {
                        w.rsplit('/').next().unwrap_or(w).to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
        }
        // Automatic variables outside a recipe are empty.
        if b.len() == 1 && b"@<^+*?|%".contains(&b[0]) {
            return String::new();
        }
        if name == "CURDIR" {
            return self.dir.to_string_lossy().into_owned();
        }
        if let Some(d) = default_var(name) {
            let d = d.to_string();
            return self.expand(&d);
        }
        unknown(name)
    }

    fn value_of(&mut self, name: &str, v: &Var) -> String {
        for i in &v.issues {
            self.issue(i.0, i.1.clone());
        }
        for g in &v.globs {
            if !self.globs.contains(g) {
                self.globs.push(g.clone());
            }
        }
        if !v.recursive {
            return v.value.clone();
        }
        if self.active.iter().any(|a| a == name) {
            self.issue(
                UncertaintyKind::ParseIncomplete,
                format!("recursive make variable `{name}` references itself"),
            );
            return unknown(name);
        }
        self.active.push(name.to_string());
        let r = self.expand(&v.value);
        self.active.pop();
        r
    }

    fn function(&mut self, name: &str, args: &str) -> String {
        let n = match name {
            "strip" | "words" | "firstword" | "lastword" | "dir" | "notdir" | "suffix"
            | "basename" | "wildcard" | "realpath" | "abspath" | "value" | "eval" | "origin"
            | "flavor" | "shell" | "guile" | "error" | "warning" | "info" | "sort" => 1,
            "findstring" | "filter" | "filter-out" | "word" | "addsuffix" | "addprefix"
            | "join" => 2,
            "subst" | "patsubst" | "wordlist" | "foreach" | "let" => 3,
            "if" => 3,
            _ => usize::MAX,
        };
        let raw = split_args(args, n);
        let arg = |e: &mut Self, i: usize| -> String {
            raw.get(i).map(|a| e.expand(a)).unwrap_or_default()
        };
        match name {
            "subst" => {
                let (from, to, text) = (arg(self, 0), arg(self, 1), arg(self, 2));
                if from.is_empty() {
                    text
                } else {
                    text.replace(&from, &to)
                }
            }
            "patsubst" => {
                let (p, r, text) = (arg(self, 0), arg(self, 1), arg(self, 2));
                words(&text)
                    .map(|w| patsubst_word(p.trim(), r.trim(), w))
                    .collect::<Vec<_>>()
                    .join(" ")
            }
            "strip" => words(&arg(self, 0)).collect::<Vec<_>>().join(" "),
            "findstring" => {
                let (f, t) = (arg(self, 0), arg(self, 1));
                if has_unknown(&t) || has_unknown(&f) {
                    unknown("findstring")
                } else if t.contains(&f) {
                    f
                } else {
                    String::new()
                }
            }
            "filter" | "filter-out" => {
                let (p, t) = (arg(self, 0), arg(self, 1));
                let pats: Vec<&str> = words(&p).collect();
                let keep = name == "filter";
                words(&t)
                    .filter(|w| {
                        has_unknown(w)
                            || pats.iter().any(|pp| pattern_match(pp, w).is_some()) == keep
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            }
            "sort" => {
                let t = arg(self, 0);
                let mut v: Vec<&str> = words(&t).collect();
                v.sort_unstable();
                v.dedup();
                v.join(" ")
            }
            "word" => {
                let (n, t) = (arg(self, 0), arg(self, 1));
                match n.trim().parse::<usize>() {
                    Ok(k) if k > 0 => words(&t).nth(k - 1).unwrap_or("").to_string(),
                    _ => unknown("word"),
                }
            }
            "wordlist" => {
                let (s, e, t) = (arg(self, 0), arg(self, 1), arg(self, 2));
                match (s.trim().parse::<usize>(), e.trim().parse::<usize>()) {
                    (Ok(s), Ok(e)) if s > 0 => words(&t)
                        .skip(s - 1)
                        .take((e + 1).saturating_sub(s))
                        .collect::<Vec<_>>()
                        .join(" "),
                    _ => unknown("wordlist"),
                }
            }
            "words" => {
                let t = arg(self, 0);
                if has_unknown(&t) {
                    unknown("words")
                } else {
                    words(&t).count().to_string()
                }
            }
            "firstword" => words(&arg(self, 0)).next().unwrap_or("").to_string(),
            "lastword" => words(&arg(self, 0)).last().unwrap_or("").to_string(),
            "dir" => map_words(&arg(self, 0), |w| match w.rfind('/') {
                Some(i) => w[..=i].to_string(),
                None => "./".to_string(),
            }),
            "notdir" => map_words(&arg(self, 0), |w| {
                w.rsplit('/').next().unwrap_or(w).to_string()
            }),
            "suffix" => {
                let t = arg(self, 0);
                words(&t)
                    .filter_map(|w| {
                        let base = w.rsplit('/').next().unwrap_or(w);
                        base.rfind('.').map(|i| base[i..].to_string())
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            }
            "basename" => map_words(&arg(self, 0), |w| {
                let slash = w.rfind('/').map_or(0, |i| i + 1);
                match w[slash..].rfind('.') {
                    Some(i) => w[..slash + i].to_string(),
                    None => w.to_string(),
                }
            }),
            "addsuffix" => {
                let (s, t) = (arg(self, 0), arg(self, 1));
                map_words(&t, |w| format!("{w}{s}"))
            }
            "addprefix" => {
                let (p, t) = (arg(self, 0), arg(self, 1));
                map_words(&t, |w| format!("{p}{w}"))
            }
            "join" => {
                let (a, b) = (arg(self, 0), arg(self, 1));
                let a: Vec<&str> = words(&a).collect();
                let b: Vec<&str> = words(&b).collect();
                (0..a.len().max(b.len()))
                    .map(|i| format!("{}{}", a.get(i).unwrap_or(&""), b.get(i).unwrap_or(&"")))
                    .collect::<Vec<_>>()
                    .join(" ")
            }
            "wildcard" => {
                let t = arg(self, 0);
                self.wildcard(&t)
            }
            "realpath" | "abspath" => {
                let t = arg(self, 0);
                let dir = self.dir.to_path_buf();
                map_words(&t, |w| {
                    if has_unknown(w) {
                        w.to_string()
                    } else {
                        fsx::join(&dir, w).to_string_lossy().into_owned()
                    }
                })
            }
            "if" => {
                let c = arg(self, 0);
                let c = c.trim();
                if has_unknown(c) {
                    self.issue(
                        UncertaintyKind::ConditionalResolution,
                        "$(if …) on an unknown value",
                    );
                    let (t, e) = (arg(self, 1), arg(self, 2));
                    format!("{t} {e}")
                } else if c.is_empty() {
                    arg(self, 2)
                } else {
                    arg(self, 1)
                }
            }
            "or" => {
                let mut acc = Vec::new();
                for i in 0..raw.len() {
                    let v = arg(self, i);
                    let t = v.trim();
                    if has_unknown(t) {
                        self.issue(
                            UncertaintyKind::ConditionalResolution,
                            "$(or …) on an unknown value",
                        );
                        acc.push(t.to_string());
                    } else if !t.is_empty() {
                        acc.push(t.to_string());
                        break;
                    }
                }
                acc.join(" ")
            }
            "and" => {
                let mut last = String::new();
                for i in 0..raw.len() {
                    let v = arg(self, i);
                    let t = v.trim().to_string();
                    if t.is_empty() {
                        return String::new();
                    }
                    if has_unknown(&t) {
                        self.issue(
                            UncertaintyKind::ConditionalResolution,
                            "$(and …) on an unknown value",
                        );
                    }
                    last = t;
                }
                last
            }
            "foreach" => {
                let var = arg(self, 0).trim().to_string();
                let list = arg(self, 1);
                let body = raw.get(2).cloned().unwrap_or_default();
                let mut out = Vec::new();
                for w in words(&list).map(str::to_string).collect::<Vec<_>>() {
                    let mut f = Vars::new();
                    f.insert(
                        var.clone(),
                        Var {
                            value: w,
                            origin: Origin::Automatic,
                            ..Default::default()
                        },
                    );
                    self.locals.push(f);
                    let v = self.expand(&body);
                    self.locals.pop();
                    if !v.is_empty() {
                        out.push(v);
                    }
                    if self.work > MAX_WORK {
                        self.issue(UncertaintyKind::ParseIncomplete, "make expansion too long");
                        break;
                    }
                }
                out.join(" ")
            }
            "call" => {
                let fname = arg(self, 0).trim().to_string();
                let mut f = Vars::new();
                f.insert(
                    "0".into(),
                    Var {
                        value: fname.clone(),
                        origin: Origin::Automatic,
                        ..Default::default()
                    },
                );
                for i in 1..raw.len().max(1) {
                    let v = arg(self, i);
                    f.insert(
                        i.to_string(),
                        Var {
                            value: v,
                            origin: Origin::Automatic,
                            ..Default::default()
                        },
                    );
                }
                // Unset higher-numbered parameters of an enclosing call.
                for i in raw.len().max(1)..10 {
                    f.insert(
                        i.to_string(),
                        Var {
                            origin: Origin::Automatic,
                            ..Default::default()
                        },
                    );
                }
                if FUNCS.contains(&fname.as_str()) {
                    let rest: Vec<String> = (1..raw.len())
                        .map(|i| {
                            f.get(&i.to_string())
                                .map(|v| v.value.clone())
                                .unwrap_or_default()
                        })
                        .collect();
                    return self.function(&fname, &rest.join(","));
                }
                let def = self
                    .locals
                    .iter()
                    .rev()
                    .find_map(|l| l.get(&fname))
                    .or_else(|| self.vars.get(&fname))
                    .cloned();
                match def {
                    Some(d) => {
                        self.locals.push(f);
                        let r = if d.recursive {
                            self.value_of(&fname, &d)
                        } else {
                            let v = d.value.clone();
                            for i in &d.issues {
                                self.issue(i.0, i.1.clone());
                            }
                            v
                        };
                        self.locals.pop();
                        r
                    }
                    None => unknown(&fname),
                }
            }
            "value" => {
                let n = arg(self, 0).trim().to_string();
                match self.vars.get(&n) {
                    Some(v) => v.value.clone(),
                    None => unknown(&n),
                }
            }
            "eval" => {
                let _ = arg(self, 0);
                self.issue(UncertaintyKind::EvalLike, "$(eval …)");
                String::new()
            }
            "origin" | "flavor" => {
                let n = arg(self, 0).trim().to_string();
                let local = self.locals.iter().rev().find_map(|l| l.get(&n)).cloned();
                match local.or_else(|| self.vars.get(&n).cloned()) {
                    Some(v) if name == "flavor" => {
                        if v.recursive { "recursive" } else { "simple" }.to_string()
                    }
                    Some(v) => match v.origin {
                        Origin::File => "file",
                        Origin::Default => "default",
                        Origin::Automatic => "automatic",
                    }
                    .to_string(),
                    None if default_var(&n).is_some() => if name == "flavor" {
                        "recursive"
                    } else {
                        "default"
                    }
                    .to_string(),
                    None => unknown(&n),
                }
            }
            "shell" => {
                // The marker makes every use unknown where it matters (prerequisites,
                // conditionals, recipe words); no blanket issue.
                let _ = arg(self, 0);
                unknown("shell")
            }
            "info" | "warning" | "error" => {
                let _ = arg(self, 0);
                String::new()
            }
            _ => {
                // file, guile, let, intcmp: not modelled.
                self.issue(UncertaintyKind::UnsupportedSyntax, format!("$({name} …)"));
                unknown(name)
            }
        }
    }

    /// `$(wildcard …)`: existing paths matching each word, relative as written.
    pub fn wildcard(&mut self, text: &str) -> String {
        let mut out = Vec::new();
        for w in words(text) {
            if has_unknown(w) {
                out.push(w.to_string());
                continue;
            }
            let (results, glob) = wildcard_word(self.dir, w);
            if !self.globs.contains(&glob) {
                self.globs.push(glob);
            }
            out.extend(results);
        }
        out.join(" ")
    }
}

/// Expands one wildcard word relative to `dir`: the matches (as written:
/// relative words stay relative) and the glob to watch.
pub fn wildcard_word(dir: &Path, w: &str) -> (Vec<String>, Glob) {
    let abs = w.starts_with('/');
    let (prefix, rest) = crate::glob::split_static_prefix(w);
    let base = fsx::join(dir, if prefix.is_empty() { "." } else { prefix });
    if rest.is_empty() {
        // A literal path: it exists or not.
        let p = fsx::join(dir, w);
        let parent = p
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| p.clone());
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let r = if fsx::exists(&p) {
            vec![w.to_string()]
        } else {
            Vec::new()
        };
        return (
            r,
            Glob {
                dir: parent,
                pattern: escape_glob(&name),
            },
        );
    }
    let segs: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
    let mut found = Vec::new();
    walk_glob(&base, &segs, &mut found, 0);
    found.sort();
    let shown = found
        .into_iter()
        .map(|p| {
            if abs {
                return p.to_string_lossy().into_owned();
            }
            // Keep a leading `./` or `../` as written.
            match p.strip_prefix(dir) {
                Ok(r) if w.starts_with("./") => format!("./{}", r.to_string_lossy()),
                Ok(r) => r.to_string_lossy().into_owned(),
                Err(_) => pathdiff(dir, &p),
            }
        })
        .collect();
    (
        shown,
        Glob {
            dir: base,
            pattern: rest.to_string(),
        },
    )
}

fn escape_glob(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '{' | '}') {
            o.push('[');
            o.push(c);
            o.push(']');
        } else {
            o.push(c);
        }
    }
    o
}

/// `p` relative to `base` with `..` segments.
fn pathdiff(base: &Path, p: &Path) -> String {
    let b: Vec<_> = base.components().collect();
    let q: Vec<_> = p.components().collect();
    let common = b.iter().zip(&q).take_while(|(x, y)| x == y).count();
    let mut parts: Vec<String> = Vec::new();
    for _ in common..b.len() {
        parts.push("..".into());
    }
    for c in &q[common..] {
        parts.push(c.as_os_str().to_string_lossy().into_owned());
    }
    parts.join("/")
}

fn walk_glob(dir: &Path, segs: &[&str], out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 32 || out.len() > 10_000 {
        return;
    }
    let Some((seg, rest)) = segs.split_first() else {
        return;
    };
    if !crate::glob::is_glob(seg) {
        let p = dir.join(seg);
        if rest.is_empty() {
            if fsx::exists(&p) {
                out.push(p);
            }
        } else if fsx::is_dir(&p) {
            walk_glob(&p, rest, out, depth + 1);
        }
        return;
    }
    let Some(entries) = fsx::read_dir(dir) else {
        return;
    };
    for (p, is_dir) in entries {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with('.') && !seg.starts_with('.') {
            continue;
        }
        if !crate::glob::matches(seg, name) {
            continue;
        }
        if rest.is_empty() {
            out.push(p);
        } else if is_dir {
            walk_glob(&p, rest, out, depth + 1);
        }
    }
}

/// Whitespace-separated words.
pub fn words(s: &str) -> impl Iterator<Item = &str> {
    s.split([' ', '\t', '\n', '\r']).filter(|w| !w.is_empty())
}

fn map_words(s: &str, f: impl Fn(&str) -> String) -> String {
    words(s).map(f).collect::<Vec<_>>().join(" ")
}

/// Matches a `%` pattern against `word`: the stem, or `Some("")` for an exact
/// match of a pattern without `%`.
pub fn pattern_match<'w>(pattern: &str, word: &'w str) -> Option<&'w str> {
    match pattern.find('%') {
        None => (pattern == word).then_some(""),
        Some(i) => {
            let (pre, suf) = (&pattern[..i], &pattern[i + 1..]);
            (word.len() >= pre.len() + suf.len() && word.starts_with(pre) && word.ends_with(suf))
                .then(|| &word[pre.len()..word.len() - suf.len()])
        }
    }
}

/// Replaces the first `%` of `pattern` with `stem` (a pattern without `%` is
/// returned as is).
pub fn subst_stem(pattern: &str, stem: &str) -> String {
    match pattern.find('%') {
        Some(i) => format!("{}{stem}{}", &pattern[..i], &pattern[i + 1..]),
        None => pattern.to_string(),
    }
}

fn patsubst_word(pattern: &str, repl: &str, w: &str) -> String {
    if has_unknown(w) {
        return w.to_string();
    }
    match pattern_match(pattern, w) {
        Some(stem) if pattern.contains('%') => subst_stem(repl, stem),
        Some(_) => repl.to_string(),
        None => w.to_string(),
    }
}

/// The index of the `close` byte matching an already-consumed `open` at
/// `start`.
pub fn find_close(b: &[u8], start: usize, open: u8, close: u8) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = start;
    while i < b.len() {
        if b[i] == open {
            depth += 1;
        } else if b[i] == close {
            if depth == 0 {
                return Some(i);
            }
            depth -= 1;
        }
        i += 1;
    }
    None
}

/// The first `c` outside `$( )`/`${ }` and parentheses.
pub fn find_top(s: &str, c: u8) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth = 0usize;
    for (i, &x) in b.iter().enumerate() {
        match x {
            b'(' | b'{' => depth += 1,
            b')' | b'}' => depth = depth.saturating_sub(1),
            _ if x == c && depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

/// Splits function arguments at top-level commas, at most `n` parts (the last
/// keeps the rest).
pub fn split_args(s: &str, n: usize) -> Vec<String> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (i, &x) in b.iter().enumerate() {
        match x {
            b'(' | b'{' => depth += 1,
            b')' | b'}' => depth = depth.saturating_sub(1),
            b',' if depth == 0 && out.len() + 1 < n => {
                out.push(s[start..i].to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(s[start..].to_string());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ex(vars: &[(&str, &str, bool)], s: &str) -> String {
        let mut v = Vars::new();
        for (k, val, rec) in vars {
            v.insert(
                k.to_string(),
                if *rec {
                    Var::recursive(*val)
                } else {
                    Var::simple(*val)
                },
            );
        }
        let dir = PathBuf::from("/nonexistent-make-dir");
        let mut e = Exp::new(&v, &dir);
        e.expand(s)
    }

    #[test]
    fn variables_and_functions() {
        let v = [
            ("SRCS", "a.c b.c sub/c.c", false),
            ("OBJS", "$(SRCS:.c=.o)", true),
            ("X", "${SRCS}", true),
        ];
        assert_eq!(ex(&v, "$(OBJS)"), "a.o b.o sub/c.o");
        assert_eq!(ex(&v, "$X|$(X)"), "a.c b.c sub/c.c|a.c b.c sub/c.c");
        assert_eq!(
            ex(&v, "$(patsubst %.c,build/%.o,$(SRCS))"),
            "build/a.o build/b.o build/sub/c.o"
        );
        assert_eq!(ex(&v, "$(SRCS:%.c=%.d)"), "a.d b.d sub/c.d");
        assert_eq!(ex(&v, "$(subst .c,.h,$(SRCS))"), "a.h b.h sub/c.h");
        assert_eq!(ex(&v, "$(addprefix x/,a b)"), "x/a x/b");
        assert_eq!(ex(&v, "$(addsuffix .go,a b)"), "a.go b.go");
        assert_eq!(ex(&v, "$(notdir $(SRCS))"), "a.c b.c c.c");
        assert_eq!(ex(&v, "$(dir sub/c.c a.c)"), "sub/ ./");
        assert_eq!(ex(&v, "$(basename sub/c.c x.tar.gz)"), "sub/c x.tar");
        assert_eq!(ex(&v, "$(filter %.c,a.c b.h c.c)"), "a.c c.c");
        assert_eq!(ex(&v, "$(filter-out b.c,$(SRCS))"), "a.c sub/c.c");
        assert_eq!(ex(&v, "$(sort c b a b)"), "a b c");
        assert_eq!(
            ex(&v, "$(foreach d,x y,$(d)/main.go)"),
            "x/main.go y/main.go"
        );
        assert_eq!(
            ex(
                &v,
                "$(words $(SRCS)) $(firstword $(SRCS)) $(lastword $(SRCS))"
            ),
            "3 a.c sub/c.c"
        );
        assert_eq!(ex(&v, "$(if $(SRCS),yes,no) $(if ,yes,no)"), "yes no");
        assert_eq!(ex(&v, "$$HOME"), "$HOME");
        assert_eq!(ex(&[("f", "<$(1)-$(2)>", true)], "$(call f,a,b)"), "<a-b>");
        assert_eq!(ex(&v, "$(strip  a   b )"), "a b");
    }

    #[test]
    fn unknowns() {
        assert!(has_unknown(&ex(&[], "$(shell git rev-parse HEAD)")));
        assert!(has_unknown(&ex(&[], "$(UNDEFINED_THING)")));
        assert!(!has_unknown(&ex(&[], "$(CC) $(CFLAGS) $(RM)")));
        assert_eq!(ex(&[], "$(CC) $(CFLAGS)x"), "cc x");
        assert_eq!(
            unknown_to_shell(&ex(&[], "go build $(GOFLAGS)")),
            "go build ${__make_GOFLAGS}"
        );
        // Self-reference does not loop.
        assert!(has_unknown(&ex(&[("A", "$(A) x", true)], "$(A)")));
    }

    #[test]
    fn automatic_variables() {
        let v = Vars::new();
        let dir = PathBuf::from("/d");
        let mut e = Exp::new(&v, &dir);
        let mut l = Vars::new();
        l.insert("@".into(), Var::simple("out/bin/app"));
        l.insert("<".into(), Var::simple("cmd/app/main.go"));
        l.insert("^".into(), Var::simple("cmd/app/main.go go.mod"));
        l.insert("*".into(), Var::simple("app"));
        e.push_locals(l);
        assert_eq!(
            e.expand("$@ $< $^ $* $(@D) $(@F) $(<D) $(^F)"),
            "out/bin/app cmd/app/main.go cmd/app/main.go go.mod app out/bin app cmd/app main.go \
             go.mod"
        );
    }

    #[test]
    fn args_split() {
        assert_eq!(split_args("a,b,c,d", 3), ["a", "b", "c,d"]);
        assert_eq!(split_args("$(x a,b),c", 2), ["$(x a,b)", "c"]);
    }
}
