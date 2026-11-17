//! A minimal GNU make parser: reads a makefile (and what it includes) into a
//! [`Db`] of variables and rules. Nothing is executed.
//!
//! - Logical lines (backslash continuation), comments, recipes (tab-prefixed or
//!   `.RECIPEPREFIX`), `define … endef`.
//! - Assignments `=`, `:=`, `::=`, `:::=`, `?=`, `+=`, `!=` (shell → unknown),
//!   with `override`/`export`/`private`, and target-specific variables (`t: VAR
//!   = x`).
//! - Conditionals (`ifeq`/`ifneq`/`ifdef`/`ifndef`/`else`/`endif`) on
//!   statically known values; an unknown condition takes both branches, and
//!   everything defined inside gets an issue ([`Rule::issues`],
//!   [`Var::issues`]).
//! - `include`/`-include`/`sinclude` (globs allowed) are followed; existing
//!   ones are recorded in [`Db::files`], missing ones in [`Db::missing`]
//!   (probes).
//! - Rules: explicit, multiple targets, order-only (`|`), pattern (`%.o: %.c`),
//!   static pattern (`objs: %.o: %.c`, split per target), old-style suffix
//!   rules (`.c.o:`), double-colon, `.PHONY`, `.ONESHELL`, `.DEFAULT_GOAL`.
//!
//! Never panics; malformed lines are skipped (with an issue when they matter).

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::fsx;

use super::eval::{
    Exp, Glob, Issue, Origin, Var, Vars, find_top, has_unknown, pattern_match, subst_stem, words,
};

/// Maximum include nesting.
const MAX_INCLUDE_DEPTH: usize = 16;
/// Maximum makefile size read.
const MAX_FILE: usize = 4 * 1024 * 1024;
/// Maximum number of rules kept.
const MAX_RULES: usize = 20_000;

/// One rule (a static pattern rule is split into one rule per target).
#[derive(Clone, Debug, Default)]
pub struct Rule {
    /// Targets (expanded). Patterns contain `%`.
    pub targets: Vec<String>,
    /// Normal prerequisites (expanded, wildcards applied).
    pub prereqs: Vec<String>,
    /// Order-only prerequisites (after `|`).
    pub order_only: Vec<String>,
    /// Raw recipe lines (prefix removed, continuations kept as `\` + newline).
    pub recipe: Vec<String>,
    /// `::` rule.
    pub double_colon: bool,
    /// A pattern rule (`%` in the targets).
    pub pattern: bool,
    /// The stem of a static pattern rule.
    pub stem: Option<String>,
    /// Issues: unknown conditionals around it, unknown parts of its
    /// targets/prerequisites.
    pub issues: Vec<Issue>,
    /// Globs listed while expanding targets and prerequisites.
    pub globs: Vec<Glob>,
}

/// A target-specific (or pattern-specific) variable assignment.
#[derive(Clone, Debug)]
pub struct TargetVar {
    /// Target name or `%` pattern.
    pub target: String,
    /// Variable name.
    pub name: String,
    /// Operator (`=`, `:=`, `+=`, `?=`, …).
    pub op: String,
    /// Raw right-hand side.
    pub rhs: String,
    /// Issues of the context.
    pub issues: Vec<Issue>,
}

/// A parsed makefile.
#[derive(Clone, Debug, Default)]
pub struct Db {
    /// The top makefile.
    pub path: PathBuf,
    /// Its directory (make's working directory).
    pub dir: PathBuf,
    /// The makefile and every existing included file, in read order.
    pub files: Vec<PathBuf>,
    /// Included files that do not exist (probes).
    pub missing: Vec<PathBuf>,
    /// Global variables after reading everything.
    pub vars: Vars,
    /// Rules in order.
    pub rules: Vec<Rule>,
    /// Target-specific variables.
    pub target_vars: Vec<TargetVar>,
    /// `.PHONY` targets.
    pub phony: BTreeSet<String>,
    /// The first target eligible as default goal.
    pub first_target: Option<String>,
    /// Issues that concern every target (`$(eval)` at top level, `vpath`,
    /// unknown includes).
    pub issues: Vec<Issue>,
    /// Globs that concern every target (wildcard includes).
    pub globs: Vec<Glob>,
    /// `.ONESHELL` is set.
    pub oneshell: bool,
    /// The known suffixes (`.SUFFIXES`), for old-style suffix rules.
    pub suffixes: Vec<String>,
    /// `.EXPORT_ALL_VARIABLES` or a bare `export`.
    pub export_all: bool,
}

impl Db {
    /// The default goal: `.DEFAULT_GOAL` if set, else the first target.
    pub fn default_goal(&self) -> Option<String> {
        if let Some(v) = self.vars.get(".DEFAULT_GOAL") {
            let mut e = Exp::new(&self.vars, &self.dir);
            let g = e.expand(&v.value);
            if let Some(w) = words(&g).next()
                && !has_unknown(w)
            {
                return Some(w.to_string());
            }
        }
        self.first_target.clone()
    }

    fn issue(&mut self, kind: UncertaintyKind, d: impl Into<String>) {
        push_issue(&mut self.issues, (kind, d.into()));
    }
}

/// Adds an issue unless already present.
pub fn push_issue(v: &mut Vec<Issue>, i: Issue) {
    if !v.contains(&i) {
        v.push(i);
    }
}

const DEFAULT_SUFFIXES: &[&str] = &[
    ".out", ".a", ".ln", ".o", ".c", ".cc", ".C", ".cpp", ".p", ".f", ".F", ".m", ".r", ".y", ".l",
    ".ym", ".yl", ".s", ".S", ".mod", ".sym", ".def", ".h", ".info", ".dvi", ".tex", ".texinfo",
    ".texi", ".txinfo", ".w", ".ch", ".web", ".sh", ".elc", ".el",
];

/// Special targets that are not tasks.
pub fn is_special_target(t: &str) -> bool {
    matches!(
        t,
        ".PHONY"
            | ".SUFFIXES"
            | ".DEFAULT"
            | ".PRECIOUS"
            | ".INTERMEDIATE"
            | ".NOTINTERMEDIATE"
            | ".SECONDARY"
            | ".SECONDEXPANSION"
            | ".DELETE_ON_ERROR"
            | ".IGNORE"
            | ".LOW_RESOLUTION_TIME"
            | ".SILENT"
            | ".EXPORT_ALL_VARIABLES"
            | ".NOTPARALLEL"
            | ".ONESHELL"
            | ".POSIX"
            | ".WAIT"
    )
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Branch {
    True,
    False,
    Unknown,
}

#[derive(Clone, Debug)]
struct Frame {
    state: Branch,
    any_true: bool,
    any_unknown: bool,
    /// The parent was not active (everything inside is skipped).
    dead: bool,
    desc: String,
}

/// Parses the makefile at `path` (read through `fsx`).
pub fn parse_file(path: &Path) -> Db {
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut db = Db {
        path: path.to_path_buf(),
        dir: dir.clone(),
        suffixes: DEFAULT_SUFFIXES.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    };
    let mut p = Parser {
        db: &mut db,
        cond: Vec::new(),
        prefix: '\t',
        cur_rules: Vec::new(),
        stack: Vec::new(),
    };
    p.include(path, 0, true);
    if !p.cond.is_empty() {
        p.db.issue(UncertaintyKind::ParseIncomplete, "missing `endif`");
    }
    db
}

/// Parses makefile text `src` as if read from `path` (for tests and fuzzing).
pub fn parse_str(path: &Path, src: &str) -> Db {
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut db = Db {
        path: path.to_path_buf(),
        dir,
        suffixes: DEFAULT_SUFFIXES.iter().map(|s| s.to_string()).collect(),
        files: vec![path.to_path_buf()],
        ..Default::default()
    };
    let mut p = Parser {
        db: &mut db,
        cond: Vec::new(),
        prefix: '\t',
        cur_rules: Vec::new(),
        stack: vec![path.to_path_buf()],
    };
    p.text(src, 0);
    db
}

struct Parser<'d> {
    db: &'d mut Db,
    cond: Vec<Frame>,
    prefix: char,
    /// Rules receiving recipe lines.
    cur_rules: Vec<usize>,
    /// Files being read (cycle guard).
    stack: Vec<PathBuf>,
}

impl Parser<'_> {
    /// Reads `path`. `top`: the main makefile (a missing one is not a probe).
    fn include(&mut self, path: &Path, depth: usize, top: bool) -> bool {
        if depth > MAX_INCLUDE_DEPTH || self.stack.iter().any(|p| p == path) {
            return true;
        }
        let Ok(bytes) = fsx::read(path) else {
            if !top && !self.db.missing.iter().any(|m| m == path) {
                self.db.missing.push(path.to_path_buf());
            }
            return false;
        };
        if !self.db.files.iter().any(|f| f == path) {
            self.db.files.push(path.to_path_buf());
        }
        let text = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_FILE)]).into_owned();
        self.stack.push(path.to_path_buf());
        // MAKEFILE_LIST grows with every file read.
        let list = self
            .db
            .files
            .iter()
            .map(|f| {
                f.strip_prefix(&self.db.dir)
                    .map(|r| r.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| f.to_string_lossy().into_owned())
            })
            .collect::<Vec<_>>()
            .join(" ");
        self.db
            .vars
            .insert("MAKEFILE_LIST".into(), Var::simple(list));
        let saved = std::mem::take(&mut self.cur_rules);
        self.text(&text, depth);
        self.cur_rules = saved;
        self.cur_rules.clear();
        self.stack.pop();
        true
    }

    /// The conditional context: `None` when skipped, else the descriptions of
    /// the unknown conditionals around.
    fn active(&self) -> Option<Vec<Issue>> {
        let mut out = Vec::new();
        for f in &self.cond {
            if f.dead || f.state == Branch::False {
                return None;
            }
            if f.state == Branch::Unknown {
                out.push((UncertaintyKind::ConditionalResolution, f.desc.clone()));
            }
        }
        Some(out)
    }

    fn text(&mut self, src: &str, depth: usize) {
        let lines: Vec<&str> = src
            .split('\n')
            .map(|l| l.strip_suffix('\r').unwrap_or(l))
            .collect();
        let mut i = 0;
        while i < lines.len() {
            let line = lines[i];
            i += 1;
            // Recipe lines.
            if !self.cur_rules.is_empty() && line.starts_with(self.prefix) {
                let mut body = line[self.prefix.len_utf8()..].to_string();
                while ends_with_odd_backslashes(&body) && i < lines.len() {
                    let next = lines[i];
                    i += 1;
                    body.push('\n');
                    body.push_str(next.strip_prefix(self.prefix).unwrap_or(next));
                }
                if let Some(ctx) = self.active() {
                    for &r in &self.cur_rules {
                        if let Some(rule) = self.db.rules.get_mut(r) {
                            rule.recipe.push(body.clone());
                            for c in &ctx {
                                push_issue(&mut rule.issues, c.clone());
                            }
                        }
                    }
                }
                continue;
            }
            // Logical line.
            let mut logical = line.to_string();
            while ends_with_odd_backslashes(&logical) && i < lines.len() {
                logical.pop();
                let t = logical.trim_end().len();
                logical.truncate(t);
                logical.push(' ');
                logical.push_str(lines[i].trim_start());
                i += 1;
            }
            let stripped = strip_comment(&logical);
            let t = stripped.trim();
            if t.is_empty() {
                continue;
            }
            let (word, rest) = split_word(t);
            // `define` (also when skipped, to find its `endef`).
            let (def_mods, def_word, def_rest) = strip_modifiers(t);
            if def_word == "define" {
                let mut body = Vec::new();
                let mut nest = 0;
                while i < lines.len() {
                    let l = lines[i];
                    i += 1;
                    let (w, _) = split_word(l.trim());
                    if w == "endef" {
                        if nest == 0 {
                            break;
                        }
                        nest -= 1;
                    } else if strip_modifiers(l.trim()).1 == "define" {
                        nest += 1;
                    }
                    body.push(l);
                }
                if let Some(ctx) = self.active() {
                    self.cur_rules.clear();
                    let header = def_rest.trim();
                    let (name, op) = match header.rfind(['=']) {
                        Some(_) => {
                            let (n, o) = split_define_op(header);
                            (n.to_string(), o.to_string())
                        }
                        None => (header.to_string(), "=".to_string()),
                    };
                    let value = body.join("\n");
                    self.assign(&name, &op, &value, &def_mods, ctx, true);
                }
                continue;
            }
            if matches!(
                word,
                "ifeq" | "ifneq" | "ifdef" | "ifndef" | "else" | "endif"
            ) {
                self.conditional(word, rest);
                continue;
            }
            let Some(ctx) = self.active() else { continue };
            match word {
                "include" | "-include" | "sinclude" => {
                    self.cur_rules.clear();
                    self.include_directive(word, rest, depth, &ctx);
                    continue;
                }
                "vpath" => {
                    self.cur_rules.clear();
                    self.db.issue(
                        UncertaintyKind::UnsupportedSyntax,
                        "vpath: prerequisites may be found in other directories",
                    );
                    continue;
                }
                "undefine" => {
                    self.cur_rules.clear();
                    let (_, w, r) = strip_modifiers(t);
                    let n = if w == "undefine" { r } else { rest };
                    let mut e = Exp::new(&self.db.vars, &self.db.dir);
                    let n = e.expand(n.trim());
                    self.db.vars.remove(n.trim());
                    continue;
                }
                "load" | "-load" => {
                    self.cur_rules.clear();
                    self.db
                        .issue(UncertaintyKind::UnsupportedSyntax, "load: make extension");
                    continue;
                }
                _ => {}
            }
            // `override`/`export`/`unexport`/`private` prefixes.
            let (mods, _, _) = strip_modifiers(t);
            let body = strip_modifier_words(t);
            if body.is_empty() {
                // Bare `export`/`unexport`.
                if mods.iter().any(|m| m == "export") {
                    self.db.export_all = true;
                }
                continue;
            }
            match classify(body) {
                LineKind::Assign {
                    name_end,
                    op_start,
                    op,
                } => {
                    self.cur_rules.clear();
                    let name = &body[..name_end];
                    let value = body[op_start + op.len()..].trim_start();
                    self.assign(name, op, value, &mods, ctx, false);
                }
                LineKind::Rule { colon, double } => {
                    self.rule(body, colon, double, ctx);
                }
                LineKind::Other => {
                    self.cur_rules.clear();
                    if mods.iter().any(|m| m == "export" || m == "unexport") {
                        // `export VAR1 VAR2`.
                        let mut e = Exp::new(&self.db.vars, &self.db.dir);
                        let names = e.expand(body);
                        let export = mods.iter().any(|m| m == "export");
                        for n in words(&names) {
                            // An exported variable that is not defined comes from the
                            // environment: nothing to record.
                            if let Some(v) = self.db.vars.get_mut(n) {
                                v.exported = export;
                            }
                        }
                        continue;
                    }
                    // A bare function call (`$(eval …)`, `$(info …)`) or garbage.
                    let mut e = Exp::new(&self.db.vars, &self.db.dir);
                    let out = e.expand(body);
                    let (issues, globs) =
                        (std::mem::take(&mut e.issues), std::mem::take(&mut e.globs));
                    drop(e);
                    for iss in issues {
                        push_issue(&mut self.db.issues, iss);
                    }
                    for g in globs {
                        if !self.db.globs.contains(&g) {
                            self.db.globs.push(g);
                        }
                    }
                    if !out.trim().is_empty() && !has_unknown(&out) {
                        self.db.issue(
                            UncertaintyKind::ParseIncomplete,
                            format!("unparsed makefile line `{}`", short(body)),
                        );
                    } else if has_unknown(&out) {
                        self.db.issue(
                            UncertaintyKind::EvalLike,
                            format!("computed makefile line `{}`", short(body)),
                        );
                    }
                }
            }
        }
    }

    fn conditional(&mut self, word: &str, rest: &str) {
        match word {
            "endif" => {
                self.cur_rules.clear();
                if self.cond.pop().is_none() {
                    self.db
                        .issue(UncertaintyKind::ParseIncomplete, "extraneous `endif`");
                }
            }
            "else" => {
                let Some(top) = self.cond.last().cloned() else {
                    self.db
                        .issue(UncertaintyKind::ParseIncomplete, "extraneous `else`");
                    return;
                };
                let (w, r) = split_word(rest.trim());
                let mut f = top.clone();
                if f.dead {
                    if let Some(last) = self.cond.last_mut() {
                        last.state = Branch::False;
                    }
                    return;
                }
                if top.any_true {
                    f.state = Branch::False;
                } else if matches!(w, "ifeq" | "ifneq" | "ifdef" | "ifndef") {
                    let (c, desc) = self.eval_cond(w, r);
                    f.state = match (c, top.any_unknown) {
                        (Branch::True, false) => Branch::True,
                        (Branch::False, _) => Branch::False,
                        _ => Branch::Unknown,
                    };
                    f.desc = desc;
                } else {
                    f.state = if top.any_unknown {
                        Branch::Unknown
                    } else {
                        Branch::True
                    };
                    f.desc = format!("else of {}", top.desc);
                }
                f.any_true |= f.state == Branch::True;
                f.any_unknown |= f.state == Branch::Unknown;
                if let Some(last) = self.cond.last_mut() {
                    *last = f;
                }
            }
            _ => {
                let dead = self.active().is_none();
                if dead {
                    self.cond.push(Frame {
                        state: Branch::False,
                        any_true: false,
                        any_unknown: false,
                        dead: true,
                        desc: String::new(),
                    });
                    return;
                }
                let (state, desc) = self.eval_cond(word, rest);
                self.cond.push(Frame {
                    state,
                    any_true: state == Branch::True,
                    any_unknown: state == Branch::Unknown,
                    dead: false,
                    desc,
                });
            }
        }
    }

    /// Evaluates a condition: its state and a description.
    fn eval_cond(&mut self, word: &str, rest: &str) -> (Branch, String) {
        let desc = format!("{word} {}", short(rest.trim()));
        let rest = rest.trim();
        let mut malformed = false;
        let mut e = Exp::new(&self.db.vars, &self.db.dir);
        let r = match word {
            "ifdef" | "ifndef" => {
                let name = e.expand(rest);
                let name = name.trim();
                let defined = if has_unknown(name) {
                    None
                } else if let Some(v) = self.db.vars.get(name) {
                    if !v.issues.is_empty() || has_unknown(&v.value) && !v.recursive {
                        None
                    } else {
                        Some(!v.value.is_empty())
                    }
                } else if super::eval::default_var(name).is_some() {
                    Some(!super::eval::default_var(name).unwrap_or("").is_empty())
                } else {
                    // May come from the environment or the command line.
                    None
                };
                match defined {
                    None => Branch::Unknown,
                    Some(d) if d == (word == "ifdef") => Branch::True,
                    Some(_) => Branch::False,
                }
            }
            _ => match cond_args(rest) {
                Some((a, b)) => {
                    let a = e.expand(&a);
                    let b = e.expand(&b);
                    let tainted = !e.issues.is_empty();
                    if has_unknown(&a) || has_unknown(&b) || tainted {
                        Branch::Unknown
                    } else if (a.trim() == b.trim()) == (word == "ifeq") {
                        Branch::True
                    } else {
                        Branch::False
                    }
                }
                None => {
                    malformed = true;
                    Branch::Unknown
                }
            },
        };
        // A `$(wildcard)` in a condition concerns every target.
        let globs = std::mem::take(&mut e.globs);
        drop(e);
        for g in globs {
            if !self.db.globs.contains(&g) {
                self.db.globs.push(g);
            }
        }
        if malformed {
            self.db.issue(
                UncertaintyKind::ParseIncomplete,
                format!("malformed conditional `{desc}`"),
            );
        }
        (r, desc)
    }

    fn include_directive(&mut self, word: &str, rest: &str, depth: usize, ctx: &[Issue]) {
        let mut e = Exp::new(&self.db.vars, &self.db.dir);
        let names = e.expand(rest);
        let issues = e.issues;
        let mut globs = e.globs;
        for i in issues {
            push_issue(&mut self.db.issues, i);
        }
        for c in ctx {
            // An include inside an unknown conditional: its rules carry the issue through
            // the variables and rules it defines; record it globally (it may define
            // anything).
            push_issue(&mut self.db.issues, c.clone());
        }
        for w in words(&names).map(str::to_string).collect::<Vec<_>>() {
            if has_unknown(&w) {
                self.db.issue(
                    UncertaintyKind::ComputedSpecifier,
                    format!("{word} of a computed path `{}`", short(rest.trim())),
                );
                continue;
            }
            let paths: Vec<PathBuf> = if crate::glob::is_glob(&w) {
                let (found, g) = super::eval::wildcard_word(&self.db.dir, &w);
                globs.push(g);
                found.iter().map(|f| fsx::join(&self.db.dir, f)).collect()
            } else {
                vec![fsx::join(&self.db.dir, &w)]
            };
            for p in paths {
                self.include(&p, depth + 1, false);
            }
        }
        for g in globs {
            if !self.db.globs.contains(&g) {
                self.db.globs.push(g);
            }
        }
    }

    fn assign(
        &mut self,
        name_raw: &str,
        op: &str,
        value: &str,
        mods: &[String],
        ctx: Vec<Issue>,
        is_define: bool,
    ) {
        let mut e = Exp::new(&self.db.vars, &self.db.dir);
        let name = e.expand(name_raw.trim());
        let name = name.trim().to_string();
        let mut issues = e.issues;
        if name.is_empty() || has_unknown(&name) {
            self.db.issue(
                UncertaintyKind::ComputedSpecifier,
                format!("assignment to a computed variable `{}`", short(name_raw)),
            );
            return;
        }
        let value = if is_define {
            value.to_string()
        } else {
            value.trim_end().to_string()
        };
        let exported = mods.iter().any(|m| m == "export")
            || self.db.vars.get(&name).is_some_and(|v| v.exported);
        issues.extend(ctx);
        let mut var = match op {
            ":=" | "::=" | ":::=" => {
                let mut e = Exp::new(&self.db.vars, &self.db.dir);
                let v = e.expand(&value);
                issues.extend(e.issues);
                Var {
                    value: v,
                    recursive: false,
                    issues: Vec::new(),
                    globs: e.globs,
                    exported,
                    origin: Origin::File,
                }
            }
            "?=" => {
                if self.db.vars.contains_key(&name) || super::eval::default_var(&name).is_some() {
                    if !issues.is_empty()
                        && let Some(v) = self.db.vars.get_mut(&name)
                    {
                        for i in issues {
                            push_issue(&mut v.issues, i);
                        }
                    }
                    return;
                }
                Var::recursive(value)
            }
            "+=" => match self.db.vars.get(&name).cloned() {
                Some(old) if !old.recursive => {
                    let mut e = Exp::new(&self.db.vars, &self.db.dir);
                    let v = e.expand(&value);
                    issues.extend(e.issues);
                    let mut globs = old.globs.clone();
                    globs.extend(e.globs);
                    Var {
                        value: join_space(&old.value, &v),
                        recursive: false,
                        issues: old.issues.clone(),
                        globs,
                        exported,
                        origin: old.origin,
                    }
                }
                Some(old) => Var {
                    value: join_space(&old.value, &value),
                    recursive: true,
                    issues: old.issues.clone(),
                    globs: old.globs.clone(),
                    exported,
                    origin: old.origin,
                },
                None => match super::eval::default_var(&name) {
                    Some(d) => Var::recursive(join_space(d, &value)),
                    None => Var::recursive(value),
                },
            },
            // A shell command's output: unknown wherever it is used.
            "!=" => Var::simple(super::eval::unknown("shell")),
            _ => Var::recursive(value),
        };
        var.exported = exported;
        for i in issues {
            push_issue(&mut var.issues, i);
        }
        if name == ".RECIPEPREFIX" {
            let mut e = Exp::new(&self.db.vars, &self.db.dir);
            let v = e.expand(&var.value);
            self.prefix = v.chars().next().unwrap_or('\t');
        }
        self.db.vars.insert(name, var);
    }

    fn rule(&mut self, line: &str, colon: usize, double: bool, ctx: Vec<Issue>) {
        let targets_raw = &line[..colon];
        let rest = &line[colon + if double { 2 } else { 1 }..];
        let mut issues = ctx;
        let mut e = Exp::new(&self.db.vars, &self.db.dir);
        let texp = e.expand(targets_raw);
        let mut globs = std::mem::take(&mut e.globs);
        issues.extend(std::mem::take(&mut e.issues));
        drop(e);
        let mut targets: Vec<String> = Vec::new();
        for w in words(&texp) {
            if has_unknown(w) {
                // An unknown target may be what a known prerequisite names: every target is
                // affected.
                self.db.issue(
                    UncertaintyKind::ComputedSpecifier,
                    format!("computed target `{}`", short(targets_raw.trim())),
                );
            } else {
                targets.push(w.to_string());
            }
        }
        // Target-specific variable: `targets: [override|export|private] VAR op value`.
        let rest_mods_stripped = strip_modifier_words(rest.trim_start());
        if let LineKind::Assign {
            name_end,
            op_start,
            op,
        } = classify(rest_mods_stripped)
        {
            let semi = find_top(rest_mods_stripped, b';');
            if semi.is_none_or(|s| op_start < s) {
                let name = rest_mods_stripped[..name_end].trim().to_string();
                let rhs = rest_mods_stripped[op_start + op.len()..].trim().to_string();
                for t in &targets {
                    self.db.target_vars.push(TargetVar {
                        target: t.clone(),
                        name: name.clone(),
                        op: op.to_string(),
                        rhs: rhs.clone(),
                        issues: issues.clone(),
                    });
                }
                // A target-specific variable line does not start a recipe context.
                self.cur_rules.clear();
                return;
            }
        }
        let (prq_raw, inline) = match find_top(rest, b';') {
            Some(s) => (&rest[..s], Some(rest[s + 1..].trim_start().to_string())),
            None => (rest, None),
        };
        // Static pattern rule: `targets: target-pattern: prereq-patterns`.
        let static_split = find_rule_colon(prq_raw);
        let mut e = Exp::new(&self.db.vars, &self.db.dir);
        let (tpat, prq_exp) = match static_split {
            Some(c) => {
                let tp = e.expand(&prq_raw[..c]);
                (Some(tp.trim().to_string()), e.expand(&prq_raw[c + 1..]))
            }
            None => (None, e.expand(prq_raw)),
        };
        globs.append(&mut e.globs);
        issues.append(&mut e.issues);
        drop(e);
        let mut prereqs = Vec::new();
        let mut order_only = Vec::new();
        let mut in_oo = false;
        for w in words(&prq_exp) {
            let mut w = w;
            if w == "|" {
                in_oo = true;
                continue;
            }
            if let Some(r) = w.strip_prefix('|') {
                in_oo = true;
                w = r;
            }
            if has_unknown(w) {
                push_issue(
                    &mut issues,
                    (
                        UncertaintyKind::ComputedSpecifier,
                        format!("computed prerequisite `{}`", short(prq_raw.trim())),
                    ),
                );
                continue;
            }
            // Make expands wildcards in prerequisites (not in pattern prerequisites).
            let expanded: Vec<String> = if crate::glob::is_glob(w) && !w.contains('%') {
                let (found, g) = super::eval::wildcard_word(&self.db.dir, w);
                globs.push(g);
                if found.is_empty() {
                    vec![w.to_string()]
                } else {
                    found
                }
            } else {
                vec![w.to_string()]
            };
            if in_oo {
                order_only.extend(expanded);
            } else {
                prereqs.extend(expanded);
            }
        }
        // Special targets.
        if targets.len() == 1 && is_special_target(&targets[0]) {
            match targets[0].as_str() {
                ".PHONY" => self.db.phony.extend(prereqs.iter().cloned()),
                ".ONESHELL" => self.db.oneshell = true,
                ".EXPORT_ALL_VARIABLES" => self.db.export_all = true,
                ".SECONDEXPANSION" => self.db.issue(
                    UncertaintyKind::UnsupportedSyntax,
                    ".SECONDEXPANSION: prerequisites are expanded twice",
                ),
                ".SUFFIXES" => {
                    if prereqs.is_empty() {
                        self.db.suffixes.clear();
                    } else {
                        self.db.suffixes.extend(prereqs.iter().cloned());
                    }
                }
                _ => {}
            }
            self.cur_rules.clear();
            return;
        }
        // Old-style suffix rule: `.c.o:` → `%.o: %.c`, `.c:` → `%: %.c`.
        let mut pattern = targets.iter().any(|t| t.contains('%'));
        if !pattern
            && targets.len() == 1
            && prereqs.is_empty()
            && let Some((src, dst)) = self.suffix_rule(&targets[0])
        {
            targets = vec![format!("%{dst}")];
            prereqs = vec![format!("%{src}")];
            pattern = true;
        }
        if self.db.rules.len() >= MAX_RULES {
            self.db
                .issue(UncertaintyKind::ParseIncomplete, "too many make rules");
            self.cur_rules.clear();
            return;
        }
        let recipe: Vec<String> = inline.into_iter().collect();
        self.cur_rules.clear();
        match tpat {
            Some(tp) if !pattern => {
                // Static pattern rule: one rule per target.
                for t in &targets {
                    let Some(stem) = pattern_match(&tp, t) else {
                        continue;
                    };
                    let stem = stem.to_string();
                    let rule = Rule {
                        targets: vec![t.clone()],
                        prereqs: prereqs.iter().map(|p| subst_stem(p, &stem)).collect(),
                        order_only: order_only.iter().map(|p| subst_stem(p, &stem)).collect(),
                        recipe: recipe.clone(),
                        double_colon: double,
                        pattern: false,
                        stem: Some(stem),
                        issues: issues.clone(),
                        globs: globs.clone(),
                    };
                    self.push_rule(rule);
                }
            }
            _ => {
                if self.db.first_target.is_none() && !pattern {
                    // The first target not starting with `.` (unless it has a `/`).
                    if let Some(t) = targets
                        .iter()
                        .find(|t| !t.starts_with('.') || t.contains('/'))
                    {
                        self.db.first_target = Some(t.clone());
                    }
                }
                let rule = Rule {
                    targets,
                    prereqs,
                    order_only,
                    recipe,
                    double_colon: double,
                    pattern,
                    stem: None,
                    issues,
                    globs,
                };
                self.push_rule(rule);
            }
        }
    }

    fn push_rule(&mut self, r: Rule) {
        self.db.rules.push(r);
        self.cur_rules.push(self.db.rules.len() - 1);
    }

    /// `.c.o` → (`.c`, `.o`); `.c` → (`.c`, ``), when the suffixes are known.
    fn suffix_rule(&self, t: &str) -> Option<(String, String)> {
        if !t.starts_with('.') || t.contains('/') {
            return None;
        }
        let known = |s: &str| self.db.suffixes.iter().any(|x| x == s);
        if known(t) {
            return Some((t.to_string(), String::new()));
        }
        for i in 1..t.len() {
            if t.as_bytes()[i] == b'.' && known(&t[..i]) && known(&t[i..]) {
                return Some((t[..i].to_string(), t[i..].to_string()));
            }
        }
        None
    }
}

fn join_space(a: &str, b: &str) -> String {
    match (a.is_empty(), b.is_empty()) {
        (true, _) => b.to_string(),
        (_, true) => a.to_string(),
        _ => format!("{a} {b}"),
    }
}

fn short(s: &str) -> String {
    let s: String = s.chars().take(80).collect();
    s.replace(super::eval::UNK_OPEN, "$(")
        .replace(super::eval::UNK_CLOSE, ")")
}

fn ends_with_odd_backslashes(s: &str) -> bool {
    let n = s.bytes().rev().take_while(|&b| b == b'\\').count();
    n % 2 == 1
}

/// Removes a `#` comment (`\#` is a literal `#`).
pub fn strip_comment(s: &str) -> String {
    if !s.contains('#') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\\' if it.peek() == Some(&'#') => {
                out.push('#');
                it.next();
            }
            '#' => break,
            _ => out.push(c),
        }
    }
    out
}

fn split_word(t: &str) -> (&str, &str) {
    match t.find([' ', '\t', '(']) {
        Some(i) if t.as_bytes()[i] == b'(' => (&t[..i], &t[i..]),
        Some(i) => (&t[..i], &t[i + 1..]),
        None => (t, ""),
    }
}

/// Leading `override`/`export`/`unexport`/`private` words, the next word and
/// the rest.
fn strip_modifiers(t: &str) -> (Vec<String>, &str, &str) {
    let mut mods = Vec::new();
    let mut rest = t;
    loop {
        let (w, r) = split_word(rest);
        if matches!(w, "override" | "export" | "unexport" | "private") {
            mods.push(w.to_string());
            rest = r.trim_start();
            continue;
        }
        return (mods, w, r);
    }
}

/// `t` without leading modifier words (only when followed by something that is
/// not an assignment operator: `export = x` assigns the variable `export`).
fn strip_modifier_words(t: &str) -> &str {
    let mut rest = t;
    loop {
        let (w, r) = split_word(rest);
        if matches!(w, "override" | "export" | "unexport" | "private") {
            let r2 = r.trim_start();
            if r2.starts_with('=')
                || r2.starts_with(":=")
                || r2.starts_with("::=")
                || r2.starts_with("?=")
                || r2.starts_with("+=")
                || r2.starts_with("!=")
            {
                return rest;
            }
            rest = r2;
            continue;
        }
        return rest;
    }
}

/// `NAME =`, `NAME :=` … in a define header.
fn split_define_op(h: &str) -> (&str, &str) {
    for op in [":::=", "::=", ":=", "?=", "+=", "!=", "="] {
        if let Some(n) = h.strip_suffix(op) {
            return (n.trim(), op);
        }
    }
    (h, "=")
}

enum LineKind {
    Assign {
        name_end: usize,
        op_start: usize,
        op: &'static str,
    },
    Rule {
        colon: usize,
        double: bool,
    },
    Other,
}

/// Whether a line is an assignment or a rule: the first top-level `=` or `:`
/// decides.
fn classify(s: &str) -> LineKind {
    let b = s.as_bytes();
    let mut depth = 0usize;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'$' && i + 1 < b.len() && (b[i + 1] == b'(' || b[i + 1] == b'{') {
            depth += 1;
            i += 2;
            continue;
        }
        if depth > 0 {
            match c {
                b'(' | b'{' => depth += 1,
                b')' | b'}' => depth -= 1,
                _ => {}
            }
            i += 1;
            continue;
        }
        match c {
            b'=' => {
                let (start, op): (usize, &'static str) = if i > 0 {
                    match b[i - 1] {
                        b'?' => (i - 1, "?="),
                        b'+' => (i - 1, "+="),
                        b'!' => (i - 1, "!="),
                        _ => (i, "="),
                    }
                } else {
                    (i, "=")
                };
                return LineKind::Assign {
                    name_end: start,
                    op_start: start,
                    op,
                };
            }
            b':' => {
                if s[i..].starts_with(":::=") {
                    return LineKind::Assign {
                        name_end: i,
                        op_start: i,
                        op: ":::=",
                    };
                }
                if s[i..].starts_with("::=") {
                    return LineKind::Assign {
                        name_end: i,
                        op_start: i,
                        op: "::=",
                    };
                }
                if s[i..].starts_with(":=") {
                    return LineKind::Assign {
                        name_end: i,
                        op_start: i,
                        op: ":=",
                    };
                }
                // A Windows drive letter (`C:\`).
                if i == 1
                    && b.get(2).is_some_and(|&x| x == b'\\' || x == b'/')
                    && b[0].is_ascii_alphabetic()
                {
                    i += 1;
                    continue;
                }
                let double = b.get(i + 1) == Some(&b':');
                return LineKind::Rule { colon: i, double };
            }
            _ => {}
        }
        i += 1;
    }
    LineKind::Other
}

/// The colon of a static pattern rule in a prerequisite list (not part of
/// `:=`).
fn find_rule_colon(s: &str) -> Option<usize> {
    match classify(s) {
        LineKind::Rule { colon, .. } => Some(colon),
        _ => None,
    }
}

/// The two arguments of `ifeq`: `(a,b)`, `"a" "b"`, `'a' 'b'`.
fn cond_args(s: &str) -> Option<(String, String)> {
    let s = s.trim();
    if let Some(inner) = s.strip_prefix('(') {
        let b = inner.as_bytes();
        let close = super::eval::find_close(b, 0, b'(', b')')?;
        let body = &inner[..close];
        let comma = find_top(body, b',')?;
        return Some((body[..comma].to_string(), body[comma + 1..].to_string()));
    }
    let mut parts = Vec::new();
    let mut rest = s;
    for _ in 0..2 {
        rest = rest.trim_start();
        let q = rest.chars().next()?;
        if q != '"' && q != '\'' {
            return None;
        }
        let end = rest[1..].find(q)? + 1;
        parts.push(rest[1..end].to_string());
        rest = &rest[end + 1..];
    }
    Some((parts[0].clone(), parts[1].clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db(src: &str) -> Db {
        parse_str(Path::new("/nonexistent-mk/Makefile"), src)
    }

    fn val(d: &Db, name: &str) -> String {
        let mut e = Exp::new(&d.vars, &d.dir);
        e.var(name)
    }

    #[test]
    fn flavors() {
        let d = db(
            "A = $(B)\nB = one\nC := $(B)\nB = two\nD ?= d\nD ?= e\nE += x\nE += y\nC += \
             z\noverride F = f\nexport G := g\nH ::= h\nI != date\n",
        );
        assert_eq!(val(&d, "A"), "two");
        assert_eq!(val(&d, "C"), "one z");
        assert_eq!(val(&d, "D"), "d");
        assert_eq!(val(&d, "E"), "x y");
        assert_eq!(val(&d, "F"), "f");
        assert!(d.vars["G"].exported);
        assert_eq!(val(&d, "H"), "h");
        assert!(has_unknown(&val(&d, "I")));
    }

    #[test]
    fn continuation_comments_define() {
        let d = db(
            "SRCS = a.c \\\n   b.c # comment\nX = a\\#b\ndefine RECIPE\n\techo one\n\techo \
             two\nendef\nall: ; @echo hi\n",
        );
        assert_eq!(val(&d, "SRCS"), "a.c b.c");
        assert_eq!(val(&d, "X"), "a#b");
        assert_eq!(val(&d, "RECIPE"), "\techo one\n\techo two");
        assert_eq!(d.rules[0].recipe, vec!["@echo hi".to_string()]);
    }

    #[test]
    fn conditionals() {
        let d = db(
            "OS = linux\nifeq ($(OS),linux)\nA = l\nelse ifeq ($(OS),darwin)\nA = d\nelse\nA = \
             o\nendif\nifdef OS\nB = yes\nendif\nifndef NOPE_UNSET\nC = maybe\nendif\nifneq \
             \"$(OS)\" \"linux\"\nD = no\nendif\n",
        );
        assert_eq!(val(&d, "A"), "l");
        assert!(d.vars["A"].issues.is_empty());
        assert_eq!(val(&d, "B"), "yes");
        // Unknown: taken, with an issue.
        assert_eq!(val(&d, "C"), "maybe");
        assert!(!d.vars["C"].issues.is_empty());
        assert!(!d.vars.contains_key("D"));
        // Nested in a false branch: skipped entirely.
        let d = db("ifeq (a,b)\nifeq ($(X),1)\nV = 1\nelse\nV = 2\nendif\nendif\n");
        assert!(!d.vars.contains_key("V"));
    }

    #[test]
    fn rules() {
        let d = db(
            ".PHONY: all clean\nall: app | out\napp: main.o util.o\n\tcc -o $@ $^\n%.o: \
             %.c\n\t$(CC) -c $< -o $@\nobjs := a.o b.o\n$(objs): %.o: src/%.c\n\tcc -c \
             $<\nclean::\n\trm -f app\nclean::\n\trm -f *.o\n.c.h:\n\tgen $<\napp: CFLAGS += -O2\n",
        );
        assert_eq!(d.first_target.as_deref(), Some("all"));
        assert!(d.phony.contains("clean"));
        let all = &d.rules[0];
        assert_eq!(all.prereqs, ["app"]);
        assert_eq!(all.order_only, ["out"]);
        let pat: Vec<&Rule> = d.rules.iter().filter(|r| r.pattern).collect();
        assert_eq!(pat.len(), 2);
        assert_eq!(pat[1].targets, ["%.h"]);
        assert_eq!(pat[1].prereqs, ["%.c"]);
        let sp: Vec<&Rule> = d.rules.iter().filter(|r| r.stem.is_some()).collect();
        assert_eq!(sp.len(), 2);
        assert_eq!(sp[0].prereqs, ["src/a.c"]);
        assert_eq!(d.rules.iter().filter(|r| r.double_colon).count(), 2);
        assert_eq!(d.target_vars.len(), 1);
        assert_eq!(d.target_vars[0].name, "CFLAGS");
    }

    #[test]
    fn recipe_prefix_and_default_goal() {
        let d = db(
            ".RECIPEPREFIX = >\n.DEFAULT_GOAL := build\ntest:\n> go test ./...\nbuild:\n> go \
             build\n",
        );
        assert_eq!(d.default_goal().as_deref(), Some("build"));
        assert_eq!(d.rules[0].recipe, vec![" go test ./...".to_string()]);
    }
}
